//! Browsing state. Provider setup, futures, and sorting run on one detached worker.
use crate::{
    domain::{Entry, EntryKind, FsError, FsErrorKind, Location},
    providers::{CancellationToken, FileSystem, ListOptions, local::LocalFileSystem},
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    future::Future,
    process::{Command, Stdio},
    sync::{Arc, Condvar, Mutex},
    task::{Context, Poll, Wake, Waker},
    thread,
};

#[derive(Clone, Debug)]
pub enum ListingState {
    Loading,
    Loaded,
    Failed(FsError),
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Sort {
    Name,
    Size,
    Modified,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

struct Request {
    generation: u64,
    location: Location,
    hidden: bool,
    sort: Sort,
    direction: SortDirection,
    cancel: CancellationToken,
}

struct Response {
    generation: u64,
    result: Result<Vec<Entry>, FsError>,
    location: Location,
}

#[derive(Default)]
struct Mailbox {
    request: Option<Request>,
    response: Option<Response>,
    shutdown: bool,
}

#[derive(Default)]
struct OpenState {
    pending: bool,
    result: Option<Result<Location, FsError>>,
}

pub struct Browser {
    pub location: Location,
    pub entries: Vec<Entry>,
    pub state: ListingState,
    pub show_hidden: bool,
    pub cursor: Option<usize>,
    pub selected: BTreeSet<Location>,
    pub sort: Sort,
    pub sort_direction: SortDirection,
    pub search_query: String,
    mailbox: Arc<(Mutex<Mailbox>, Condvar)>,
    cancellation: CancellationToken,
    generation: u64,
    location_generation: u64,
    last_local: Location,
    open_state: Arc<Mutex<OpenState>>,
    history: Vec<Location>,
    history_index: usize,
    restore_cursor: Option<(Location, usize)>,
    selection_anchor: Option<Location>,
    worker_available: bool,
    children: BTreeMap<Location, Vec<Entry>>,
    expanded: BTreeSet<Location>,
    depths: Vec<usize>,
    parents: BTreeMap<Location, Location>,
    node_states: BTreeMap<Location, ListingState>,
    pending: VecDeque<Location>,
    in_flight: Option<Location>,
    requested: BTreeSet<Location>,
}

impl Browser {
    pub fn new(location: Location) -> Self {
        let mailbox = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
        let worker_mailbox = mailbox.clone();
        let worker = thread::Builder::new()
            .name("tui-listing".into())
            .spawn(move || worker(worker_mailbox));
        let worker_available = worker.is_ok();
        let state = match worker {
            Ok(_) => ListingState::Loading,
            Err(error) => ListingState::Failed(FsError::from_io(location.clone(), error)),
        };
        let mut browser = Self {
            last_local: if location.is_local() {
                location.clone()
            } else {
                Location::Local(
                    std::env::var_os("HOME")
                        .map(std::path::PathBuf::from)
                        .filter(|path| path.is_absolute())
                        .unwrap_or_else(std::env::temp_dir),
                )
            },
            open_state: Arc::new(Mutex::new(OpenState::default())),
            history: vec![location.clone()],
            location,
            entries: Vec::new(),
            state,
            show_hidden: false,
            cursor: None,
            selected: BTreeSet::new(),
            sort: Sort::Name,
            sort_direction: SortDirection::Ascending,
            search_query: String::new(),
            mailbox,
            cancellation: CancellationToken::new(),
            generation: 0,
            location_generation: 0,
            history_index: 0,
            restore_cursor: None,
            selection_anchor: None,
            worker_available,
            children: BTreeMap::new(),
            expanded: BTreeSet::new(),
            depths: Vec::new(),
            parents: BTreeMap::new(),
            node_states: BTreeMap::new(),
            pending: VecDeque::new(),
            in_flight: None,
            requested: BTreeSet::new(),
        };
        browser.refresh();
        browser
    }

    /// Consume at most one coalesced response; return whether visible state changed.
    pub fn poll(&mut self) -> bool {
        let response = self
            .mailbox
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .response
            .take();
        let Some(response) = response else {
            return self
                .open_state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .result
                .is_some();
        };
        if response.generation != self.generation {
            return false;
        }
        self.in_flight = None;
        let root = response.location == self.location;
        match response.result {
            Ok(entries) => {
                let expanded_children: Vec<_> = entries
                    .iter()
                    .filter(|entry| {
                        entry.kind == EntryKind::Directory
                            && self.expanded.contains(&entry.location)
                    })
                    .map(|entry| entry.location.clone())
                    .collect();
                self.children.insert(response.location.clone(), entries);
                self.node_states
                    .insert(response.location, ListingState::Loaded);
                if root {
                    self.state = ListingState::Loaded;
                }
                for location in expanded_children {
                    self.queue(location);
                }
                self.rebuild();
            }
            Err(error) => {
                let state = if error.kind == FsErrorKind::Cancelled {
                    ListingState::Cancelled
                } else {
                    ListingState::Failed(error)
                };
                if root {
                    self.state = state.clone();
                }
                self.node_states.insert(response.location, state);
            }
        }
        self.schedule_next();
        if !self.has_pending_listing() && matches!(self.state, ListingState::Loaded) {
            self.rebuild();
        }
        true
    }

    /// Navigate immediately; async responses never mutate history.
    pub fn navigate(&mut self, location: Location) {
        if location == self.location {
            self.refresh();
            return;
        }
        self.history.truncate(self.history_index + 1);
        self.history.push(location.clone());
        self.history_index += 1;
        self.change_location(location);
    }

    fn change_location(&mut self, location: Location) {
        if location.is_local() {
            self.last_local = location.clone();
        }
        self.location_generation += 1;
        self.location = location;
        self.entries.clear();
        self.children.clear();
        self.expanded.clear();
        self.parents.clear();
        self.depths.clear();
        self.node_states.clear();
        self.cursor = None;
        self.restore_cursor = None;
        self.clear_selection();
        self.refresh();
    }

    /// Leave remote browsing, cancelling its listing and restoring the latest local folder.
    /// Independent transfer jobs are unaffected.
    pub fn return_to_local(&mut self) -> bool {
        if self.location.is_local() {
            return false;
        }
        self.navigate(self.last_local.clone());
        true
    }

    /// Start one local regular-file open without blocking the UI or invoking a shell.
    pub fn open_file_cursor(&mut self) -> Result<bool, String> {
        if !matches!(self.state, ListingState::Loaded) {
            return Ok(false);
        }
        let Some(entry) = self.cursor.and_then(|index| self.entries.get(index)) else {
            return Ok(false);
        };
        if entry.kind != EntryKind::File || !entry.location.is_local() {
            return Ok(false);
        }
        let location = entry.location.clone();
        let mut state = self.open_state.lock().unwrap_or_else(|e| e.into_inner());
        if state.pending || state.result.is_some() {
            return Err("A file-open request is already pending".into());
        }
        state.pending = true;
        let result_state = self.open_state.clone();
        let spawn = thread::Builder::new()
            .name("tui-open-file".into())
            .spawn(move || {
                let result = open_local_file(&location).map(|()| location);
                let mut state = result_state.lock().unwrap_or_else(|e| e.into_inner());
                state.pending = false;
                state.result = Some(result);
            });
        if let Err(error) = spawn {
            state.pending = false;
            return Err(format!("Cannot start file-open worker: {error}"));
        }
        Ok(true)
    }

    /// Opening errors are separate from listing state, so the folder stays usable.
    pub fn take_open_result(&mut self) -> Option<Result<Location, FsError>> {
        self.open_state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .result
            .take()
    }

    /// Changes only on navigation, so peer targets survive listing refreshes.
    pub fn location_generation(&self) -> u64 {
        self.location_generation
    }

    pub fn back(&mut self) -> bool {
        if self.history_index == 0 {
            return false;
        }
        self.history_index -= 1;
        self.change_location(self.history[self.history_index].clone());
        true
    }

    pub fn forward(&mut self) -> bool {
        if self.history_index + 1 == self.history.len() {
            return false;
        }
        self.history_index += 1;
        self.change_location(self.history[self.history_index].clone());
        true
    }

    pub fn parent(&mut self) -> bool {
        let Some(parent) = self.location.parent() else {
            return false;
        };
        self.navigate(parent);
        true
    }

    pub fn refresh(&mut self) {
        if !self.worker_available {
            return;
        }
        if let Some((index, entry)) = self
            .cursor
            .and_then(|index| self.entries.get(index).map(|entry| (index, entry)))
        {
            self.restore_cursor = Some((entry.location.clone(), index));
        }
        self.cancellation.cancel();
        self.cancellation = CancellationToken::new();
        self.generation += 1;
        self.state = ListingState::Loading;
        self.children.clear();
        self.pending.clear();
        self.in_flight = None;
        self.requested.clear();
        self.node_states.clear();
        {
            let mut mailbox = self.mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
            mailbox.response = None;
            mailbox.request = None;
        }
        self.queue(self.location.clone());
        self.schedule_next();
    }

    fn queue(&mut self, location: Location) {
        if self.requested.insert(location.clone()) {
            self.node_states
                .insert(location.clone(), ListingState::Loading);
            self.pending.push_back(location);
        }
    }

    fn schedule_next(&mut self) {
        if self.in_flight.is_some() {
            return;
        }
        while let Some(location) = self.pending.pop_front() {
            if location != self.location
                && (!self.expanded.contains(&location)
                    || !self.entries.iter().any(|entry| entry.location == location))
            {
                self.requested.remove(&location);
                self.node_states.remove(&location);
                continue;
            }
            self.in_flight = Some(location.clone());
            let mut mailbox = self.mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
            mailbox.request = Some(Request {
                generation: self.generation,
                location,
                hidden: self.show_hidden,
                sort: self.sort,
                direction: self.sort_direction,
                cancel: self.cancellation.clone(),
            });
            self.mailbox.1.notify_one();
            break;
        }
    }

    fn rebuild(&mut self) {
        let cursor = self
            .cursor
            .and_then(|i| self.entries.get(i))
            .map(|entry| entry.location.clone());
        let old_index = self.cursor.unwrap_or(0);
        let mut reachable = BTreeSet::new();
        let mut parents = BTreeMap::new();
        let mut stack = vec![self.location.clone()];
        while let Some(parent) = stack.pop() {
            if !reachable.insert(parent.clone()) {
                continue;
            }
            if let Some(entries) = self.children.get(&parent) {
                for entry in entries {
                    parents.insert(entry.location.clone(), parent.clone());
                    if entry.kind == EntryKind::Directory {
                        stack.push(entry.location.clone());
                    } else {
                        reachable.insert(entry.location.clone());
                    }
                }
            }
        }
        self.children
            .retain(|location, _| reachable.contains(location));
        if self.pending.is_empty() && self.in_flight.is_none() {
            self.expanded
                .retain(|location| reachable.contains(location));
            self.selected
                .retain(|location| reachable.contains(location));
        }
        self.parents = parents;
        self.entries.clear();
        self.depths.clear();
        let mut stack: Vec<_> = self
            .children
            .get(&self.location)
            .into_iter()
            .flatten()
            .rev()
            .cloned()
            .map(|entry| (entry, 0))
            .collect();
        while let Some((entry, depth)) = stack.pop() {
            if entry.kind == EntryKind::Directory
                && self.expanded.contains(&entry.location)
                && depth < 128
            {
                stack.extend(
                    self.children
                        .get(&entry.location)
                        .into_iter()
                        .flatten()
                        .rev()
                        .cloned()
                        .map(|child| (child, depth + 1)),
                );
            }
            self.entries.push(entry);
            self.depths.push(depth);
        }
        let restored = self.restore_cursor.as_ref().and_then(|(location, _)| {
            self.entries
                .iter()
                .position(|entry| &entry.location == location)
        });
        let current = cursor.and_then(|location| {
            self.entries
                .iter()
                .position(|entry| entry.location == location)
        });
        self.cursor = if self.entries.is_empty() {
            None
        } else {
            Some(restored.or(current).unwrap_or_else(|| {
                self.restore_cursor
                    .as_ref()
                    .map_or(old_index, |(_, i)| *i)
                    .min(self.entries.len() - 1)
            }))
        };
        // Retain the refresh target until its expanded branch has finished loading.
        if self.pending.is_empty() {
            self.restore_cursor = None;
        }
        if self
            .selection_anchor
            .as_ref()
            .is_some_and(|location| !reachable.contains(location))
        {
            self.selection_anchor = None;
        }
    }

    pub fn has_pending_listing(&self) -> bool {
        self.in_flight.is_some() || !self.pending.is_empty()
    }

    pub fn is_empty_directory(&self, index: usize) -> bool {
        self.entries.get(index).is_some_and(|entry| {
            entry.kind == EntryKind::Directory
                && matches!(
                    self.node_states.get(&entry.location),
                    Some(ListingState::Loaded)
                )
                && self
                    .children
                    .get(&entry.location)
                    .is_some_and(Vec::is_empty)
        })
    }

    pub fn row_depth(&self, index: usize) -> usize {
        self.depths.get(index).copied().unwrap_or(0)
    }

    pub fn is_expanded(&self, index: usize) -> bool {
        self.entries
            .get(index)
            .is_some_and(|entry| self.expanded.contains(&entry.location))
    }

    pub fn row_listing_state(&self, index: usize) -> Option<&ListingState> {
        self.entries
            .get(index)
            .and_then(|entry| self.node_states.get(&entry.location))
    }

    pub fn expand_cursor(&mut self) -> bool {
        if !matches!(self.state, ListingState::Loaded) {
            return false;
        }
        let Some(index) = self.cursor else {
            return false;
        };
        let Some(entry) = self.entries.get(index) else {
            return false;
        };
        if entry.kind != EntryKind::Directory || self.row_depth(index) >= 128 {
            return false;
        }
        let location = entry.location.clone();
        self.restore_cursor = None;
        if self.expanded.insert(location.clone()) {
            if !self.requested.contains(&location) {
                self.queue(location);
            }
            self.rebuild();
            self.schedule_next();
        } else if matches!(
            self.node_states.get(&location),
            Some(ListingState::Failed(_) | ListingState::Cancelled)
        ) {
            self.requested.remove(&location);
            self.queue(location);
            self.schedule_next();
        } else if self
            .depths
            .get(index + 1)
            .is_some_and(|depth| *depth > self.depths[index])
        {
            self.cursor = Some(index + 1);
        }
        true
    }

    pub fn collapse_cursor(&mut self) -> bool {
        if !matches!(self.state, ListingState::Loaded) {
            return false;
        }
        let Some(entry) = self.cursor.and_then(|index| self.entries.get(index)) else {
            return false;
        };
        let location = entry.location.clone();
        if self.expanded.remove(&location) {
            self.requested.remove(&location);
            self.restore_cursor = None;
            self.rebuild();
            return true;
        }
        if let Some(parent) = self.parents.get(&location) {
            if let Some(index) = self
                .entries
                .iter()
                .position(|entry| &entry.location == parent)
            {
                self.cursor = Some(index);
                return true;
            }
        }
        false
    }

    /// Include selected cached children even when collapsed; a selected directory owns its descendants.
    pub fn selected_entries(&self) -> Vec<Entry> {
        self.children
            .values()
            .flatten()
            .filter(|entry| {
                if !self.selected.contains(&entry.location) {
                    return false;
                }
                let mut parent = self.parents.get(&entry.location);
                let mut seen = BTreeSet::new();
                while let Some(location) = parent {
                    if !seen.insert(location) {
                        return false;
                    }
                    if self.selected.contains(location) {
                        return false;
                    }
                    parent = self.parents.get(location);
                }
                true
            })
            .cloned()
            .collect()
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.refresh();
    }

    pub fn cancel(&mut self) {
        self.cancellation.cancel();
        self.generation += 1;
        self.pending.clear();
        self.in_flight = None;
        for state in self.node_states.values_mut() {
            if matches!(state, ListingState::Loading) {
                *state = ListingState::Cancelled;
            }
        }
        let mut mailbox = self.mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
        mailbox.request = None;
        mailbox.response = None;
        if matches!(self.state, ListingState::Loading) {
            self.state = ListingState::Cancelled;
        }
    }

    pub fn move_cursor(&mut self, delta: isize, extend: bool) {
        if !matches!(self.state, ListingState::Loaded) || self.entries.is_empty() {
            return;
        }
        self.restore_cursor = None;
        let old = self.cursor.unwrap_or(0).min(self.entries.len() - 1);
        let next = old.saturating_add_signed(delta).min(self.entries.len() - 1);
        if extend {
            let anchor = self
                .selection_anchor
                .get_or_insert_with(|| self.entries[old].location.clone());
            let anchor = self
                .entries
                .iter()
                .position(|entry| &entry.location == anchor)
                .unwrap_or(old);
            self.selected = self.entries[anchor.min(next)..=anchor.max(next)]
                .iter()
                .map(|entry| entry.location.clone())
                .collect();
        } else {
            self.selection_anchor = None;
        }
        self.cursor = Some(next);
    }

    /// Find a literal, case-insensitive name match in the visible tree, wrapping once.
    /// Return the one-based position of the match and the total number of matches.
    /// Search moves the cursor without changing the selected file locations.
    pub fn search_cursor(
        &mut self,
        query: &str,
        reverse: bool,
        include_current: bool,
    ) -> Option<(usize, usize)> {
        self.search_query = query.to_owned();
        if query.is_empty() || !matches!(self.state, ListingState::Loaded) {
            return None;
        }
        let query = query.to_lowercase();
        let matches: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                entry
                    .name
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&query)
                    .then_some(index)
            })
            .collect();
        let current = self.cursor.filter(|&index| index < self.entries.len());
        let ordinal = if reverse {
            matches
                .iter()
                .rposition(|&index| {
                    current.is_none_or(|current| {
                        index < current || (include_current && index == current)
                    })
                })
                .or_else(|| matches.len().checked_sub(1))
        } else {
            matches
                .iter()
                .position(|&index| {
                    current.is_none_or(|current| {
                        index > current || (include_current && index == current)
                    })
                })
                .or_else(|| (!matches.is_empty()).then_some(0))
        }?;
        self.restore_cursor = None;
        self.selection_anchor = None;
        self.cursor = Some(matches[ordinal]);
        Some((ordinal + 1, matches.len()))
    }

    pub fn toggle_selected(&mut self) {
        if !matches!(self.state, ListingState::Loaded) {
            return;
        }
        if let Some(entry) = self.cursor.and_then(|index| self.entries.get(index))
            && !self.selected.remove(&entry.location)
        {
            self.selected.insert(entry.location.clone());
        }
        self.selection_anchor = None;
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.selection_anchor = None;
    }

    pub fn cycle_sort(&mut self) {
        self.sort = match self.sort {
            Sort::Name => Sort::Size,
            Sort::Size => Sort::Modified,
            Sort::Modified => Sort::Name,
        };
        self.refresh();
    }

    pub fn set_sort_direction(&mut self, direction: SortDirection) {
        if self.sort_direction != direction {
            self.sort_direction = direction;
            self.refresh();
        }
    }

    pub fn toggle_sort_direction(&mut self) {
        self.set_sort_direction(match self.sort_direction {
            SortDirection::Ascending => SortDirection::Descending,
            SortDirection::Descending => SortDirection::Ascending,
        });
    }

    /// Return true only when a directory navigation was requested. Symlinks are not followed.
    pub fn open_cursor(&mut self) -> bool {
        if !matches!(self.state, ListingState::Loaded) {
            return false;
        }
        let Some(entry) = self.cursor.and_then(|index| self.entries.get(index)) else {
            return false;
        };
        if entry.kind != EntryKind::Directory {
            return false;
        }
        self.navigate(entry.location.clone());
        true
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.cancellation.cancel();
        let mut mailbox = self.mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
        mailbox.shutdown = true;
        mailbox.request = None;
        self.mailbox.1.notify_one();
        // No join: a filesystem syscall can take arbitrarily long to complete.
    }
}

fn open_local_file(location: &Location) -> Result<(), FsError> {
    let path = location.local_path();
    // Recheck on the worker: the listed file may have changed since selection.
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| FsError::from_io(location.clone(), error))?;
    if !metadata.file_type().is_file() {
        return Err(FsError::from_io(
            location.clone(),
            std::io::Error::other("The selected item is no longer a regular file"),
        ));
    }
    let status = Command::new("/usr/bin/open")
        .arg("--")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| FsError::from_io(location.clone(), error))?;
    if status.success() {
        Ok(())
    } else {
        Err(FsError::from_io(
            location.clone(),
            std::io::Error::other(format!("macOS could not open the selected file ({status})")),
        ))
    }
}

fn compare_entries(
    a: &Entry,
    b: &Entry,
    sort: Sort,
    direction: SortDirection,
) -> std::cmp::Ordering {
    (a.kind != EntryKind::Directory)
        .cmp(&(b.kind != EntryKind::Directory))
        .then_with(|| {
            let order = match sort {
                Sort::Name => a.name.cmp(&b.name),
                Sort::Size => a.size.cmp(&b.size),
                Sort::Modified => a.modified.cmp(&b.modified),
            }
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.location.cmp(&b.location));
            match direction {
                SortDirection::Ascending => order,
                SortDirection::Descending => order.reverse(),
            }
        })
}

fn worker(mailbox: Arc<(Mutex<Mailbox>, Condvar)>) {
    let provider = LocalFileSystem;
    loop {
        let request = {
            let mut slot = mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
            while slot.request.is_none() && !slot.shutdown {
                slot = mailbox.1.wait(slot).unwrap_or_else(|e| e.into_inner());
            }
            if slot.shutdown {
                return;
            }
            slot.request.take().expect("request checked above")
        };
        if request.cancel.is_cancelled() {
            continue;
        }
        let options = ListOptions {
            show_hidden: request.hidden,
        };
        let mut result = if request.location.is_local() {
            block_on(provider.list(request.location.clone(), options, request.cancel.clone()))
        } else {
            match super::connection_service::prepare(&request.location) {
                Ok(registry) => {
                    if request.cancel.is_cancelled() {
                        continue;
                    }
                    block_on(registry.list(
                        request.location.clone(),
                        options,
                        request.cancel.clone(),
                    ))
                }
                Err(message) => Err(FsError {
                    kind: FsErrorKind::Authentication,
                    location: request.location.clone(),
                    message,
                }),
            }
        };
        if request.cancel.is_cancelled() {
            continue;
        }
        if let Ok(entries) = &mut result {
            entries.sort_by(|a, b| compare_entries(a, b, request.sort, request.direction));
        }
        if request.cancel.is_cancelled() {
            continue;
        }
        let mut slot = mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
        if slot.shutdown {
            return;
        }
        if !request.cancel.is_cancelled() && slot.request.is_none() {
            slot.response = Some(Response {
                generation: request.generation,
                result,
                location: request.location,
            });
        }
    }
}

struct ThreadWake(thread::Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park(),
        }
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;
    use crate::domain::EntryId;

    fn browser() -> Browser {
        let mut browser = Browser::new(Location::Local(std::env::temp_dir()));
        browser.cancel();
        browser.state = ListingState::Loaded;
        browser.entries = ["Alpha.csv", "notes.txt", "ALPINE.csv"]
            .into_iter()
            .map(|name| {
                let location = Location::Local(std::env::temp_dir().join(name));
                Entry {
                    id: EntryId(location.clone()),
                    location,
                    name: name.into(),
                    kind: EntryKind::File,
                    size: None,
                    modified: None,
                }
            })
            .collect();
        browser.cursor = Some(0);
        browser
    }

    #[test]
    fn sort_directions_cover_every_field_and_keep_directories_first() {
        use std::time::{Duration, UNIX_EPOCH};
        let make = |name: &str, kind, size, seconds| {
            let location = Location::Local(std::env::temp_dir().join(name));
            Entry {
                id: EntryId(location.clone()),
                location,
                name: name.into(),
                kind,
                size: Some(size),
                modified: Some(UNIX_EPOCH + Duration::from_secs(seconds)),
            }
        };
        let entries = vec![
            make("a", EntryKind::File, 3, 20),
            make("b", EntryKind::File, 9, 10),
            make("folder", EntryKind::Directory, 0, 0),
        ];
        for (sort, ascending) in [
            (Sort::Name, ["a", "b"]),
            (Sort::Size, ["a", "b"]),
            (Sort::Modified, ["b", "a"]),
        ] {
            for direction in [SortDirection::Ascending, SortDirection::Descending] {
                let mut sorted = entries.clone();
                sorted.sort_by(|a, b| compare_entries(a, b, sort, direction));
                assert_eq!(sorted[0].kind, EntryKind::Directory);
                let expected = if direction == SortDirection::Ascending {
                    ascending
                } else {
                    [ascending[1], ascending[0]]
                };
                assert_eq!(sorted[1].name, std::ffi::OsStr::new(expected[0]));
                assert_eq!(sorted[2].name, std::ffi::OsStr::new(expected[1]));
            }
        }
    }

    #[test]
    fn reversing_sort_restores_selected_expanded_child_by_location() {
        use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
        struct Temp(std::path::PathBuf);
        impl Drop for Temp {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let temp = Temp(
            std::env::temp_dir().join(format!(
                "excavator-sort-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )),
        );
        std::fs::create_dir_all(temp.0.join("folder")).unwrap();
        for path in ["a.txt", "z.txt", "folder/a-child", "folder/z-child"] {
            std::fs::write(temp.0.join(path), path).unwrap();
        }
        let drain = |browser: &mut Browser| {
            let deadline = Instant::now() + Duration::from_secs(3);
            while browser.has_pending_listing() && Instant::now() < deadline {
                browser.poll();
                thread::sleep(Duration::from_millis(5));
            }
            assert!(!browser.has_pending_listing());
            assert!(matches!(browser.state, ListingState::Loaded));
        };
        let mut browser = Browser::new(Location::Local(temp.0.clone()));
        drain(&mut browser);
        browser.cursor = Some(0);
        browser.expand_cursor();
        drain(&mut browser);
        let selected = Location::Local(temp.0.join("folder/a-child"));
        browser.cursor = browser
            .entries
            .iter()
            .position(|entry| entry.location == selected);
        browser.toggle_selected();
        browser.toggle_sort_direction();
        drain(&mut browser);
        assert_eq!(browser.sort_direction, SortDirection::Descending);
        assert_eq!(browser.entries[browser.cursor.unwrap()].location, selected);
        assert!(browser.selected.contains(&selected));
        assert!(browser.is_expanded(0));
        assert_eq!(browser.entries[1].name, std::ffi::OsStr::new("z-child"));
    }

    #[test]
    fn return_local_restores_latest_folder_and_keeps_history() {
        let mut browser = browser();
        let local = Location::Local(std::env::temp_dir().join("local-browser-test"));
        browser.navigate(local.clone());
        let remote = Location::Sftp {
            connection: "test".into(),
            path: "/".into(),
        };
        browser.navigate(remote.clone());
        assert!(browser.return_to_local());
        assert_eq!(browser.location, local);
        assert!(!browser.return_to_local());
        assert!(browser.back());
        assert_eq!(browser.location, remote);
        assert!(browser.return_to_local());
        assert_eq!(browser.location, local);
    }

    #[test]
    fn opening_rejects_symlinks_and_remote_files_without_worker() {
        let mut browser = browser();
        browser.entries[0].kind = EntryKind::Symlink;
        assert_eq!(browser.open_file_cursor(), Ok(false));
        browser.entries[0].kind = EntryKind::File;
        browser.entries[0].location = Location::Sftp {
            connection: "test".into(),
            path: "/file".into(),
        };
        assert_eq!(browser.open_file_cursor(), Ok(false));
        assert!(matches!(browser.state, ListingState::Loaded));
    }

    #[test]
    fn failed_open_preserves_listing_and_returns_typed_error() {
        let mut browser = browser();
        let missing = Location::Local(
            std::env::temp_dir().join(format!("excavator-missing-open-{}", std::process::id())),
        );
        browser.entries[0].location = missing.clone();
        assert_eq!(browser.open_file_cursor(), Ok(true));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let result = loop {
            if let Some(result) = browser.take_open_result() {
                break result;
            }
            assert!(std::time::Instant::now() < deadline, "open worker timeout");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(result.unwrap_err().location, missing);
        assert!(matches!(browser.state, ListingState::Loaded));
        assert_eq!(browser.entries.len(), 3);
        assert_eq!(browser.cursor, Some(0));
    }

    #[test]
    fn search_wraps_both_directions_and_preserves_selection() {
        let mut browser = browser();
        browser.selected.insert(browser.entries[1].location.clone());
        let selected = browser.selected.clone();
        assert_eq!(browser.search_cursor("alp", false, true), Some((1, 2)));
        assert_eq!(browser.cursor, Some(0));
        assert_eq!(browser.search_cursor("ALP", false, false), Some((2, 2)));
        assert_eq!(browser.cursor, Some(2));
        assert_eq!(browser.search_cursor("alp", false, false), Some((1, 2)));
        assert_eq!(browser.search_cursor("alp", true, false), Some((2, 2)));
        assert_eq!(browser.search_cursor("alp", true, false), Some((1, 2)));
        assert_eq!(browser.selected, selected);
    }

    #[test]
    fn unmatched_and_empty_search_keep_cursor_and_store_query() {
        let mut browser = browser();
        assert_eq!(browser.search_cursor(".*", false, true), None);
        assert_eq!(browser.search_query, ".*");
        assert_eq!(browser.cursor, Some(0));
        assert_eq!(browser.search_cursor("", false, true), None);
        assert!(browser.search_query.is_empty());
        assert_eq!(browser.cursor, Some(0));
    }
}
