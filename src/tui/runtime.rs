//! Source-owned transfer orchestration. Coordination and credential I/O stay on workers.
use super::{
    browser::{Browser, ListingState},
    connection_service,
    coordination::{Coordination, Direction, Peer},
    modal::{self, Tone},
    view,
};
use crate::{
    domain::Location,
    transfers::{ConflictPolicy, JobSnapshot, JobState, Operation, OperationPlan, TransferManager},
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph},
};
use std::{
    cell::Cell,
    collections::VecDeque,
    ffi::OsString,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone)]
enum Target {
    Peer(Peer),
    Manual(Location),
}
impl Target {
    fn location(&self) -> &Location {
        match self {
            Self::Peer(peer) => &peer.location,
            Self::Manual(location) => location,
        }
    }
}
#[derive(Clone)]
struct Review {
    operation: Operation,
    sources: Vec<Location>,
    target: Target,
    metadata: String,
    new_name: Option<OsString>,
}
#[derive(Clone)]
struct CopyChoice {
    label: String,
    target: Option<Target>,
    provider_path: bool,
    metadata: String,
}
enum Modal {
    CopyDestination {
        sources: Vec<Location>,
        base: Location,
        choices: Vec<CopyChoice>,
        index: usize,
        input: String,
        editing: bool,
        alternate_input: String,
    },
    Busy {
        text: String,
        approved: bool,
    },
    Peers {
        operation: Operation,
        sources: Vec<Location>,
        peers: Vec<Peer>,
        index: usize,
    },
    Destination {
        operation: Operation,
        sources: Vec<Location>,
        base: Location,
        input: String,
    },
    Name {
        operation: Operation,
        sources: Vec<Location>,
        base: Location,
        input: String,
    },
    Review(Review),
    Conflict {
        id: u64,
        source: Location,
        destination: Location,
        replace: bool,
    },
    Log,
}
enum Task {
    Discover {
        operation: Operation,
        sources: Vec<Location>,
        direction: Option<Direction>,
    },
    Review {
        operation: Operation,
        sources: Vec<Location>,
        target: Target,
        reuse: bool,
    },
    OperationReview {
        operation: Operation,
        sources: Vec<Location>,
        base: Location,
        new_name: Option<OsString>,
    },
    Queue(Review, Option<Direction>),
    Notify(Vec<Location>),
}
enum Reply {
    Ready(Arc<Coordination>),
    Peers(Operation, Vec<Location>, Vec<Peer>, bool),
    CopyChoices(Vec<Location>, Vec<CopyChoice>),
    Review(Box<Review>),
    Started(TransferManager),
    Error(String),
}
struct TaskEnvelope {
    generation: u64,
    task: Task,
}
struct ReplyEnvelope {
    generation: u64,
    reply: Reply,
}
pub struct Runtime {
    tasks: SyncSender<TaskEnvelope>,
    replies: Receiver<ReplyEnvelope>,
    request_generation: Arc<AtomicU64>,
    generation: u64,
    pending_queue: bool,
    direction: Option<Direction>,
    worker_manager: Arc<Mutex<Vec<TransferManager>>>,
    worker_coordination: Arc<Mutex<Option<Arc<Coordination>>>>,
    scroll: Cell<u16>,
    page_height: Cell<u16>,
    copy_columns: Cell<usize>,
    follow_peer: Cell<bool>,
    coordination: Option<Arc<Coordination>>,
    modal: Option<Modal>,
    active: Option<TransferManager>,
    queued: VecDeque<(u64, TransferManager)>,
    active_label: u64,
    next_label: u64,
    logs: Vec<JobSnapshot>,
    last_target: Option<Peer>,
    last_poll: Instant,
    location_generation: u64,
    completed: bool,
    pub summary: String,
}
impl Runtime {
    pub fn new(location: Location) -> Self {
        let (tasks, receive) = mpsc::sync_channel(4);
        let (send, replies) = mpsc::sync_channel(4);
        let request_generation = Arc::new(AtomicU64::new(0));
        let worker_generation = request_generation.clone();
        let worker_manager = Arc::new(Mutex::new(Vec::<TransferManager>::new()));
        let running_manager = worker_manager.clone();
        let worker_coordination = Arc::new(Mutex::new(None));
        let shared_coordination = worker_coordination.clone();
        thread::spawn(move || {
            // Exit waits for this bounded startup section so owned sockets cannot
            // be published after the frontend has already cleaned up.
            let mut startup = shared_coordination
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if worker_generation.load(Ordering::Acquire) == u64::MAX {
                return;
            }
            let coordination = match Coordination::start(location) {
                Ok(value) => Arc::new(value),
                Err(error) => {
                    let _ = send.send(ReplyEnvelope {
                        generation: 0,
                        reply: Reply::Error(error),
                    });
                    return;
                }
            };
            if worker_generation.load(Ordering::Acquire) == u64::MAX {
                coordination.shutdown();
                return;
            }
            *startup = Some(coordination.clone());
            drop(startup);
            if send
                .send(ReplyEnvelope {
                    generation: 0,
                    reply: Reply::Ready(coordination.clone()),
                })
                .is_err()
            {
                return;
            }
            while let Ok(TaskEnvelope { generation, task }) = receive.recv() {
                let cancellable = matches!(
                    task,
                    Task::Discover { .. } | Task::Review { .. } | Task::OperationReview { .. }
                );
                if cancellable && worker_generation.load(Ordering::Acquire) != generation {
                    continue;
                }
                let response: Result<Option<Reply>, String> = (|| match task {
                    Task::Discover {
                        operation,
                        sources,
                        direction,
                    } => {
                        let peers = eligible_peers(&coordination, direction, operation)?;
                        if operation == Operation::Copy {
                            let (preferences, warning) = crate::persistence::load();
                            if let Some(warning) = warning {
                                return Err(warning);
                            }
                            let mut choices = copy_choices(peers, preferences.favorites);
                            let local = Location::Local("/".into());
                            for choice in &mut choices {
                                let destination =
                                    choice.target.as_ref().map(Target::location).unwrap_or_else(
                                        || {
                                            if choice.provider_path {
                                                sources.first().unwrap_or(&local)
                                            } else {
                                                &local
                                            }
                                        },
                                    );
                                choice.metadata = metadata_revision(&sources, destination)?;
                            }
                            Ok(Some(Reply::CopyChoices(sources, choices)))
                        } else {
                            Ok(Some(Reply::Peers(
                                operation,
                                sources,
                                peers,
                                direction.is_some(),
                            )))
                        }
                    }
                    Task::Review {
                        operation,
                        sources,
                        target,
                        reuse,
                    } => {
                        if operation == Operation::Move
                            && matches!(&target, Target::Peer(peer) if peer.is_shell)
                        {
                            return Err(
                                "Shell destinations support Copy only. Choose a browser for Move."
                                    .into(),
                            );
                        }
                        if let Target::Peer(peer) = &target
                            && let Err(error) = coordination.validate(peer)
                        {
                            if reuse {
                                return Ok(Some(Reply::Peers(
                                    operation,
                                    sources,
                                    eligible_peers(&coordination, None, operation)?,
                                    false,
                                )));
                            }
                            return Err(error);
                        }
                        let metadata = metadata_revision(&sources, target.location())?;
                        Ok(Some(Reply::Review(Box::new(Review {
                            operation,
                            sources,
                            target,
                            metadata,
                            new_name: None,
                        }))))
                    }
                    Task::OperationReview {
                        operation,
                        sources,
                        base,
                        new_name,
                    } => {
                        let metadata = metadata_revision(&sources, &base)?;
                        Ok(Some(Reply::Review(Box::new(Review {
                            operation,
                            sources,
                            target: Target::Manual(base),
                            metadata,
                            new_name,
                        }))))
                    }
                    Task::Queue(review, direction) => {
                        validate_target(&coordination, &review, direction)?;
                        if metadata_revision(&review.sources, review.target.location())?
                            != review.metadata
                        {
                            return Err("Connection metadata changed. Choose and review the destination again.".into());
                        }
                        let mut locations = review.sources.clone();
                        locations.push(review.target.location().clone());
                        let registry = connection_service::prepare_locations(&locations)?;
                        // Recheck after potentially slow Keychain work and immediately before enqueue.
                        validate_target(&coordination, &review, direction)?;
                        if metadata_revision(&review.sources, review.target.location())?
                            != review.metadata
                        {
                            return Err(
                                "Connection metadata changed during preparation. Review again."
                                    .into(),
                            );
                        }
                        let mut running = running_manager.lock().unwrap_or_else(|p| p.into_inner());
                        if worker_generation.load(Ordering::Acquire) != generation {
                            return Err("Source browser exited during transfer preparation.".into());
                        }
                        let manager = TransferManager::with_registry(registry);
                        manager.enqueue(OperationPlan {
                            operation: review.operation,
                            sources: review.sources,
                            destination: Some(review.target.location().clone()),
                            new_name: review.new_name,
                            conflict_policy: ConflictPolicy::Ask,
                        });
                        running.retain(|m| m.snapshots().iter().any(|j| !terminal(&j.state)));
                        running.push(manager.clone());
                        drop(running);
                        Ok(Some(Reply::Started(manager)))
                    }
                    Task::Notify(locations) => {
                        coordination.notify_locations(&locations);
                        Ok(None)
                    }
                })();
                if cancellable && worker_generation.load(Ordering::Acquire) != generation {
                    continue;
                }
                match response {
                    Ok(Some(reply)) => {
                        if send.send(ReplyEnvelope { generation, reply }).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        if send
                            .send(ReplyEnvelope {
                                generation,
                                reply: Reply::Error(error),
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        });
        Self {
            tasks,
            replies,
            request_generation,
            generation: 0,
            pending_queue: false,
            direction: None,
            worker_manager,
            worker_coordination,
            scroll: Cell::new(0),
            page_height: Cell::new(8),
            copy_columns: Cell::new(1),
            follow_peer: Cell::new(true),
            coordination: None,
            modal: None,
            active: None,
            queued: VecDeque::new(),
            active_label: 0,
            next_label: 1,
            logs: Vec::new(),
            last_target: None,
            last_poll: Instant::now(),
            location_generation: 0,
            completed: false,
            summary: String::new(),
        }
    }
    fn task(&mut self, task: Task, label: &str, notice: &mut Option<String>) {
        if self.pending_queue {
            *notice = Some("Wait for the approved transfer to finish preparation.".into());
            return;
        }
        let approved = matches!(task, Task::Queue(..));
        self.generation = self.generation.wrapping_add(1).max(1);
        self.request_generation
            .store(self.generation, Ordering::Release);
        match self.tasks.try_send(TaskEnvelope {
            generation: self.generation,
            task,
        }) {
            Ok(()) => {
                self.pending_queue = approved;
                self.scroll.set(0);
                self.modal = Some(Modal::Busy {
                    text: label.into(),
                    approved,
                });
            }
            Err(_) => {
                self.modal = None;
                *notice = Some("The background action queue is unavailable or busy. Retry.".into());
            }
        }
    }
    pub fn modal_active(&self) -> bool {
        self.modal.is_some()
    }
    pub fn poll(&mut self, browser: &mut Browser, notice: &mut Option<String>) -> bool {
        let mut changed = false;
        if let Some(coordination) = &self.coordination {
            if self.location_generation != browser.location_generation() {
                self.location_generation = browser.location_generation();
                coordination.publish(browser.location.clone(), self.location_generation);
            }
            if coordination.take_invalidation() {
                browser.refresh();
                changed = true;
            }
        }
        while let Ok(ReplyEnvelope { generation, reply }) = self.replies.try_recv() {
            if generation != 0 && generation != self.generation {
                continue;
            }
            changed = true;
            self.scroll.set(0);
            self.follow_peer.set(true);
            match reply {
                Reply::CopyChoices(sources, choices) => {
                    self.modal = Some(Modal::CopyDestination {
                        sources,
                        base: browser.location.clone(),
                        choices,
                        index: 0,
                        input: String::new(),
                        editing: false,
                        alternate_input: String::new(),
                    });
                }
                Reply::Ready(coordination) => {
                    coordination.publish(browser.location.clone(), browser.location_generation());
                    self.coordination = Some(coordination);
                }
                Reply::Peers(operation, sources, peers, directional) => {
                    self.last_target = None;
                    if directional && peers.len() == 1 {
                        self.task(
                            Task::Review {
                                operation,
                                sources,
                                target: Target::Peer(peers[0].clone()),
                                reuse: false,
                            },
                            "Reading destination…",
                            notice,
                        );
                    } else if peers.is_empty() {
                        self.modal = Some(Modal::Destination {
                            operation,
                            sources,
                            base: browser.location.clone(),
                            input: String::new(),
                        });
                        *notice=Some("No eligible destination. Copy supports browser and local shell panes; Move requires a browser. Enter a provider path, or Esc to cancel.".into());
                    } else {
                        self.modal = Some(Modal::Peers {
                            operation,
                            sources,
                            peers,
                            index: 0,
                        });
                    }
                }
                Reply::Review(review) => self.modal = Some(Modal::Review(*review)),
                Reply::Started(manager) => {
                    self.pending_queue = false;
                    self.modal = None;
                    self.queued.push_back((self.next_label, manager));
                    self.next_label = self.next_label.wrapping_add(1).max(1);
                    self.start_next();
                    self.summary = format!("Job queued · {} waiting", self.queued.len());
                }
                Reply::Error(error) => {
                    self.pending_queue = false;
                    self.last_target = None;
                    self.modal = None;
                    *notice = Some(error);
                }
            }
        }
        if self.last_poll.elapsed() >= Duration::from_millis(150) {
            self.last_poll = Instant::now();
            if let Some(manager) = &self.active
                && let Some(mut job) = manager.snapshots().into_iter().last()
            {
                let summary = format!(
                    "{:?}: {} / {} items, {} bytes · {} queued",
                    job.state,
                    job.completed_items,
                    job.total_items,
                    job.bytes_copied,
                    self.queued.len()
                );
                if self.summary != summary {
                    self.summary = summary;
                    changed = true;
                }
                if let JobState::AwaitingConflict {
                    source,
                    destination,
                } = &job.state
                    && self.modal.is_none()
                {
                    self.modal = Some(Modal::Conflict {
                        id: job.id,
                        source: source.clone(),
                        destination: destination.clone(),
                        replace: false,
                    });
                    changed = true;
                }
                if matches!(
                    job.state,
                    JobState::Completed | JobState::Failed | JobState::Cancelled
                ) && !self.completed
                {
                    self.completed = true;
                    if matches!(self.modal, Some(Modal::Conflict { .. })) {
                        self.modal = None;
                    }
                    changed = true;
                    let mut affected: Vec<Location> = job
                        .plan
                        .sources
                        .iter()
                        .filter_map(Location::parent)
                        .collect();
                    if let Some(destination) = &job.plan.destination {
                        affected.push(destination.clone());
                    }
                    let _ = self.tasks.try_send(TaskEnvelope {
                        generation: 0,
                        task: Task::Notify(affected),
                    });
                    browser.refresh();
                    if let Some(error) = &job.error {
                        *notice = Some(error.to_string());
                    }
                    job.id = self.active_label;
                    self.logs.push(job);
                    if self.logs.len() > 100 {
                        self.logs.remove(0);
                    }
                    self.start_next();
                }
            }
        }
        changed
    }
    pub fn begin(
        &mut self,
        operation: Operation,
        direction: Option<Direction>,
        manual: bool,
        browser: &Browser,
        notice: &mut Option<String>,
    ) {
        if self.pending_queue || self.modal.is_some() {
            *notice = Some("Finish or close the current review first.".into());
            return;
        }
        self.scroll.set(0);
        if self.queued.len() + usize::from(self.active.is_some() && !self.completed) >= 16 {
            *notice = Some("The job queue is full (16). Wait for a job to finish.".into());
            return;
        }
        if !matches!(browser.state, ListingState::Loaded) || browser.has_pending_listing() {
            *notice = Some("Wait for the directory listing before choosing sources.".into());
            return;
        }
        let sources = if browser.selected.is_empty() {
            browser
                .cursor
                .and_then(|i| browser.entries.get(i))
                .map(|entry| vec![entry.location.clone()])
                .unwrap_or_default()
        } else {
            browser
                .selected_entries()
                .into_iter()
                .map(|entry| entry.location.clone())
                .collect()
        };
        if sources.is_empty() {
            *notice = Some("Select a file or directory first.".into());
            return;
        }
        self.direction = if manual { None } else { direction };
        if manual {
            self.modal = Some(Modal::Destination {
                operation,
                sources,
                base: browser.location.clone(),
                input: String::new(),
            });
        } else if operation != Operation::Copy
            && direction.is_none()
            && self
                .last_target
                .as_ref()
                .is_some_and(|peer| operation == Operation::Copy || !peer.is_shell)
        {
            self.task(
                Task::Review {
                    operation,
                    sources,
                    target: Target::Peer(self.last_target.clone().unwrap()),
                    reuse: true,
                },
                "Rechecking previous destination…",
                notice,
            );
        } else {
            self.task(
                Task::Discover {
                    operation,
                    sources,
                    direction,
                },
                "Finding browser and shell destinations…",
                notice,
            );
        }
    }
    fn start_next(&mut self) {
        if self.active.is_some() && !self.completed {
            return;
        }
        if let Some((label, manager)) = self.queued.pop_front() {
            self.active_label = label;
            self.completed = false;
            let runner = manager.clone();
            self.active = Some(manager);
            thread::spawn(move || runner.run_pending());
        }
    }
    pub fn begin_operation(
        &mut self,
        mut operation: Operation,
        browser: &Browser,
        notice: &mut Option<String>,
    ) {
        if self.pending_queue || self.modal.is_some() {
            *notice = Some("Finish or close the current review first.".into());
            return;
        }
        if self.queued.len() + usize::from(self.active.is_some() && !self.completed) >= 16 {
            *notice = Some("The job queue is full (16).".into());
            return;
        }
        if !matches!(browser.state, ListingState::Loaded) || browser.has_pending_listing() {
            *notice = Some("Wait for the directory listing.".into());
            return;
        }
        let sources: Vec<Location> = if operation == Operation::CreateDirectory {
            Vec::new()
        } else if browser.selected.is_empty() {
            browser
                .cursor
                .and_then(|i| browser.entries.get(i))
                .map(|e| vec![e.location.clone()])
                .unwrap_or_default()
        } else {
            browser
                .selected_entries()
                .into_iter()
                .map(|e| e.location.clone())
                .collect()
        };
        if operation != Operation::CreateDirectory && sources.is_empty() {
            *notice = Some("Select a file or directory first.".into());
            return;
        }
        if operation == Operation::Rename && sources.len() != 1 {
            *notice = Some("Rename requires exactly one selected item.".into());
            return;
        }
        if operation == Operation::Trash {
            let local = sources.iter().filter(|l| l.is_local()).count();
            if local != 0 && local != sources.len() {
                *notice = Some("Choose local or remote items separately for deletion.".into());
                return;
            }
            if local == 0 {
                operation = Operation::Delete;
            }
        }
        if operation == Operation::CreateDirectory
            && matches!(browser.location, Location::S3 { .. })
        {
            *notice = Some(
                "S3 prefixes are created by uploading objects; empty folders are unsupported."
                    .into(),
            );
            return;
        }
        if operation == Operation::Rename
            && matches!(sources[0], Location::Ftps { .. } | Location::S3 { .. })
        {
            *notice = Some("This provider does not support safe native rename.".into());
            return;
        }
        self.direction = None;
        let base = if operation == Operation::Rename {
            sources[0]
                .parent()
                .unwrap_or_else(|| browser.location.clone())
        } else {
            browser.location.clone()
        };
        if matches!(operation, Operation::CreateDirectory | Operation::Rename) {
            self.modal = Some(Modal::Name {
                operation,
                sources,
                base,
                input: String::new(),
            });
        } else if matches!(operation, Operation::Trash | Operation::Delete) {
            self.task(
                Task::OperationReview {
                    operation,
                    sources,
                    base,
                    new_name: None,
                },
                "Preparing operation review…",
                notice,
            );
        }
    }
    pub fn open_log(&mut self) {
        if self.pending_queue || self.modal.is_some() {
            return;
        }
        self.scroll.set(0);
        self.modal = Some(Modal::Log);
    }
    pub fn cancel_transfer(&mut self) {
        if let Some(manager) = &self.active {
            for job in manager.snapshots() {
                manager.cancel(job.id);
            }
        }
    }
    pub fn key(&mut self, key: KeyEvent, notice: &mut Option<String>) -> bool {
        if self.modal.is_none() {
            return false;
        }
        if key.kind != KeyEventKind::Press {
            return true;
        }
        let plain = key.modifiers.is_empty();
        if plain
            && matches!(
                key.code,
                KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End
            )
        {
            let page = self.page_height.get().max(1);
            self.scroll.set(match key.code {
                KeyCode::PageUp => self.scroll.get().saturating_sub(page),
                KeyCode::PageDown => self.scroll.get().saturating_add(page),
                KeyCode::Home => 0,
                _ => u16::MAX,
            });
            return true;
        }
        let Some(modal) = self.modal.take() else {
            return false;
        };
        if (plain && key.code == KeyCode::Esc)
            || (key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL)
        {
            if matches!(modal, Modal::Busy { approved: true, .. }) {
                self.modal = Some(modal);
                *notice = Some("This transfer is approved. Wait for preparation, then cancel the active job with x.".into());
                return true;
            }
            if let Modal::Conflict { id, .. } = modal
                && let Some(manager) = &self.active
            {
                manager.cancel(id);
            }
            self.generation = self.generation.wrapping_add(1).max(1);
            self.request_generation
                .store(self.generation, Ordering::Release);
            self.scroll.set(0);
            return true;
        }
        match modal {
            Modal::CopyDestination {
                sources,
                base,
                choices,
                mut index,
                mut input,
                mut editing,
                mut alternate_input,
            } => {
                let previous_index = index;
                match key.code {
                    KeyCode::Tab if plain => {
                        editing = !editing && choices[index].target.is_none();
                    }
                    KeyCode::Up | KeyCode::Left if plain && !editing => {
                        let step = if key.code == KeyCode::Up {
                            self.copy_columns.get()
                        } else {
                            1
                        };
                        index = index.saturating_sub(step);
                    }
                    KeyCode::Down | KeyCode::Right if plain && !editing => {
                        let step = if key.code == KeyCode::Down {
                            self.copy_columns.get()
                        } else {
                            1
                        };
                        index = (index + step).min(choices.len().saturating_sub(1));
                    }
                    KeyCode::Enter if plain => {
                        let choice = &choices[index];
                        let target = if let Some(target) = &choice.target {
                            Ok(target.clone())
                        } else {
                            let local = Location::Local(std::path::PathBuf::from("/"));
                            (if choice.provider_path { &base } else { &local })
                                .parse_path(&input)
                                .map(Target::Manual)
                                .map_err(|e| e.message)
                        };
                        match target {
                            Ok(target) => {
                                // Local shortcuts are not constrained to pane geometry.
                                if !matches!(target, Target::Peer(_)) {
                                    self.direction = None;
                                }
                                self.task(
                                    Task::Queue(
                                        Review {
                                            operation: Operation::Copy,
                                            sources,
                                            target,
                                            metadata: choice.metadata.clone(),
                                            new_name: None,
                                        },
                                        self.direction,
                                    ),
                                    "Preparing approved copy…",
                                    notice,
                                );
                                return true;
                            }
                            Err(error) => {
                                *notice = Some(error);
                                editing = true;
                            }
                        }
                    }
                    KeyCode::Backspace if choices[index].target.is_none() => {
                        input.pop();
                        editing = true;
                    }
                    KeyCode::Char('u')
                        if key.modifiers == KeyModifiers::CONTROL
                            && choices[index].target.is_none() =>
                    {
                        input.clear();
                        editing = true;
                    }
                    KeyCode::Char(c)
                        if choices[index].target.is_none()
                            && !key.modifiers.intersects(
                                KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                            )
                            && !c.is_control()
                            && input.len() + c.len_utf8() <= 8192 =>
                    {
                        input.push(c);
                        editing = true;
                    }
                    _ => {}
                }
                if choices[previous_index].provider_path != choices[index].provider_path {
                    std::mem::swap(&mut input, &mut alternate_input);
                }
                self.modal = Some(Modal::CopyDestination {
                    sources,
                    base,
                    choices,
                    index,
                    input,
                    editing,
                    alternate_input,
                });
            }
            Modal::Busy { text, approved } => self.modal = Some(Modal::Busy { text, approved }),
            Modal::Log => {
                if plain && key.code == KeyCode::Char('x') {
                    self.cancel_transfer();
                }
                if key.code == KeyCode::Char('X')
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
                {
                    for (label, manager) in self.queued.drain(..) {
                        for mut job in manager.snapshots() {
                            manager.cancel(job.id);
                            job.state = JobState::Cancelled;
                            job.id = label;
                            self.logs.push(job);
                        }
                    }
                    if self.logs.len() > 100 {
                        self.logs.drain(..self.logs.len() - 100);
                    }
                    *notice = Some("Cancelled waiting jobs; the active job continues.".into());
                }
                if !plain || key.code != KeyCode::Char('q') {
                    self.modal = Some(Modal::Log);
                }
            }
            Modal::Peers {
                operation,
                sources,
                peers,
                mut index,
            } => {
                match key.code {
                    KeyCode::Up if plain => {
                        index = index.saturating_sub(1);
                        self.follow_peer.set(true);
                    }
                    KeyCode::Down if plain => {
                        index = (index + 1).min(peers.len() - 1);
                        self.follow_peer.set(true);
                    }
                    KeyCode::Enter if plain => {
                        self.task(
                            Task::Review {
                                operation,
                                sources,
                                target: Target::Peer(peers[index].clone()),
                                reuse: false,
                            },
                            "Checking destination…",
                            notice,
                        );
                        return true;
                    }
                    _ => {}
                }
                self.modal = Some(Modal::Peers {
                    operation,
                    sources,
                    peers,
                    index,
                });
            }
            Modal::Destination {
                operation,
                sources,
                base,
                mut input,
            } => {
                match key.code {
                    KeyCode::Enter if plain => match base.parse_path(&input) {
                        Ok(location) => {
                            self.task(
                                Task::Review {
                                    operation,
                                    sources,
                                    target: Target::Manual(location),
                                    reuse: false,
                                },
                                "Reviewing destination…",
                                notice,
                            );
                            return true;
                        }
                        Err(error) => *notice = Some(error.message),
                    },
                    KeyCode::Backspace => {
                        input.pop();
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        input.clear()
                    }
                    KeyCode::Char(c)
                        if !key.modifiers.intersects(
                            KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                        ) && !c.is_control()
                            && input.len() + c.len_utf8() <= 8192 =>
                    {
                        input.push(c)
                    }
                    _ => {}
                }
                self.modal = Some(Modal::Destination {
                    operation,
                    sources,
                    base,
                    input,
                });
            }
            Modal::Name {
                operation,
                sources,
                base,
                mut input,
            } => {
                match key.code {
                    KeyCode::Enter if plain => {
                        if input.is_empty()
                            || matches!(input.as_str(), "." | "..")
                            || input.contains('/')
                            || input.chars().any(char::is_control)
                        {
                            *notice = Some(
                                "Enter one name, without / or control characters; . and .. are invalid.".into(),
                            );
                        } else {
                            self.task(
                                Task::OperationReview {
                                    operation,
                                    sources,
                                    base,
                                    new_name: Some(OsString::from(input)),
                                },
                                "Preparing operation review…",
                                notice,
                            );
                            return true;
                        }
                    }
                    KeyCode::Backspace if plain => {
                        input.pop();
                    }
                    KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => input.clear(),
                    KeyCode::Char(c)
                        if !key.modifiers.intersects(
                            KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                        ) && !c.is_control()
                            && input.len() + c.len_utf8() <= 8192 =>
                    {
                        input.push(c)
                    }
                    _ => {}
                }
                self.modal = Some(Modal::Name {
                    operation,
                    sources,
                    base,
                    input,
                });
            }
            Modal::Review(review) => {
                if plain && key.code == KeyCode::Enter {
                    if let Target::Peer(peer) = &review.target {
                        self.last_target = Some(peer.clone());
                    }
                    self.task(
                        Task::Queue(review, self.direction),
                        "Rechecking and queuing transfer…",
                        notice,
                    );
                } else {
                    self.modal = Some(Modal::Review(review));
                }
            }
            Modal::Conflict {
                id,
                source,
                destination,
                mut replace,
            } => {
                let policy = if replace {
                    (plain && key.code == KeyCode::Enter).then_some(ConflictPolicy::Replace)
                } else {
                    match key.code {
                        KeyCode::Char('k') if plain => Some(ConflictPolicy::KeepBoth),
                        KeyCode::Char('s') if plain => Some(ConflictPolicy::Skip),
                        KeyCode::Char('x') if plain => Some(ConflictPolicy::Cancel),
                        KeyCode::Char('r') if plain => {
                            replace = true;
                            self.scroll.set(0);
                            None
                        }
                        _ => None,
                    }
                };
                if let Some(policy) = policy {
                    if let Some(manager) = &self.active {
                        manager.resolve_conflict(id, policy);
                        let runner = manager.clone();
                        thread::spawn(move || runner.run_pending());
                    }
                } else {
                    self.modal = Some(Modal::Conflict {
                        id,
                        source,
                        destination,
                        replace,
                    });
                }
            }
        }
        true
    }
    pub fn paste(&mut self, text: &str, notice: &mut Option<String>) {
        if text.chars().any(char::is_control) {
            *notice = Some("Paste rejected: control characters are not allowed.".into());
            return;
        }
        if let Some(Modal::CopyDestination {
            choices,
            index,
            input,
            editing,
            ..
        }) = &mut self.modal
        {
            if choices[*index].target.is_none() {
                for c in text.chars() {
                    if input.len() + c.len_utf8() > 8192 {
                        break;
                    }
                    input.push(c);
                }
                *editing = true;
            }
            return;
        }
        if let Some(Modal::Destination { input, .. } | Modal::Name { input, .. }) = &mut self.modal
        {
            for c in text.chars() {
                if input.len() + c.len_utf8() > 8192 {
                    break;
                }
                input.push(c);
            }
        }
    }
    pub fn progress(&self) -> Option<JobSnapshot> {
        self.active
            .as_ref()
            .and_then(|manager| manager.snapshots().into_iter().last())
            .or_else(|| self.logs.last().cloned())
            .filter(|job| matches!(job.plan.operation, Operation::Copy | Operation::Move))
    }
    pub fn render(&self, frame: &mut Frame) {
        let Some(modal) = &self.modal else {
            return;
        };
        match modal {
            Modal::CopyDestination {
                sources,
                base,
                choices,
                index,
                input,
                editing,
                ..
            } => {
                self.copy_columns
                    .set(if frame.area().width >= 76 { 2 } else { 1 });
                render_copy_destination(
                    frame,
                    sources,
                    base,
                    choices,
                    *index,
                    input,
                    *editing,
                    &self.scroll,
                    &self.page_height,
                );
                return;
            }
            Modal::Destination {
                operation,
                input,
                base,
                ..
            }
            | Modal::Name {
                operation,
                input,
                base,
                ..
            } => {
                let destination = matches!(modal, Modal::Destination { .. });
                let title = if destination { "Destination" } else { "Name" };
                let label = if destination {
                    if matches!(base, Location::S3 { .. }) {
                        "Object prefix in this bucket"
                    } else {
                        "Absolute path in the current provider"
                    }
                } else {
                    "New name"
                };
                let body = modal::panel(
                    frame,
                    title,
                    82,
                    6,
                    "Enter review · Ctrl+U clear · Esc cancel",
                    Tone::Normal,
                );
                let context = wrapped_lines(&view::display_location(base), body.width);
                let mut lines = vec![
                    Line::styled(format!("{:?}", operation), Style::default().fg(Color::Cyan)),
                    Line::styled(
                        context.first().cloned().unwrap_or_default(),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Line::raw(""),
                    Line::styled(label, Style::default().add_modifier(Modifier::BOLD)),
                ];
                let input_row = 4.min(body.height.saturating_sub(1));
                lines.truncate(input_row as usize);
                frame.render_widget(Paragraph::new(lines), body);
                if body.height > 0 {
                    modal::input(
                        frame,
                        Rect::new(body.x, body.y + input_row, body.width, 1),
                        input,
                    );
                }
                self.page_height.set(body.height.max(1));
                return;
            }
            Modal::Peers {
                operation,
                peers,
                index,
                ..
            } => {
                let desired = (peers.len().min(20) * 3 + 2).min(u16::MAX as usize) as u16;
                let body = modal::panel(
                    frame,
                    "Choose destination",
                    82,
                    desired,
                    "↑↓ choose · Enter review · Esc cancel",
                    Tone::Normal,
                );
                frame.render_widget(
                    Paragraph::new(Line::styled(
                        format!("{:?} · {} available panes", operation, peers.len()),
                        Style::default().fg(Color::DarkGray),
                    )),
                    body,
                );
                let header_height = if body.height >= 4 { 2 } else { 0 };
                let cards = Rect::new(
                    body.x,
                    body.y.saturating_add(header_height),
                    body.width,
                    body.height.saturating_sub(header_height),
                );
                self.page_height.set(cards.height.max(1));
                let maximum = peers
                    .len()
                    .saturating_mul(3)
                    .saturating_sub(cards.height as usize)
                    .min(u16::MAX as usize) as u16;
                if self.follow_peer.replace(false) {
                    let start = index.saturating_mul(3).min(u16::MAX as usize) as u16;
                    let end = start.saturating_add(2);
                    if start < self.scroll.get() {
                        self.scroll.set(start);
                    } else if end > self.scroll.get().saturating_add(cards.height) {
                        self.scroll.set(end.saturating_sub(cards.height));
                    }
                }
                self.scroll.set(self.scroll.get().min(maximum));
                let mut lines = Vec::new();
                for (i, peer) in peers.iter().enumerate() {
                    let selected = i == *index;
                    let style = if selected {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    let identity = escaped(
                        peer.identity
                            .as_ref()
                            .map(|id| id.pane_id.as_str())
                            .unwrap_or(&peer.instance_id),
                    );
                    lines.push(
                        Line::from(vec![
                            Span::styled(
                                format!(
                                    "{} {}",
                                    if selected { "›" } else { " " },
                                    if peer.is_shell { "Shell" } else { "Browser" }
                                ),
                                style,
                            ),
                            Span::styled(
                                format!("  {identity}"),
                                if selected {
                                    style
                                } else {
                                    Style::default().fg(Color::DarkGray)
                                },
                            ),
                        ])
                        .style(style),
                    );
                    lines.push(Line::styled(
                        format!("  {}", view::display_location(&peer.location)),
                        if selected {
                            style
                        } else {
                            Style::default().fg(Color::DarkGray)
                        },
                    ));
                    if selected {
                        for line in lines.iter_mut().rev().take(2) {
                            line.spans.push(Span::raw(
                                " ".repeat(usize::from(cards.width).saturating_sub(line.width())),
                            ));
                        }
                    }
                    lines.push(Line::raw(""));
                }
                frame.render_widget(Paragraph::new(lines).scroll((self.scroll.get(), 0)), cards);
                return;
            }
            _ => {}
        }
        let (title, text) = match modal {
            Modal::Busy { text, approved } => (
                "Working",
                format!(
                    "{text}\n\n{}",
                    if *approved {
                        "Approved transfer preparation cannot be dismissed. Wait, then x cancels the active job."
                    } else {
                        "Esc cancels this read and discards its result."
                    }
                ),
            ),
            Modal::CopyDestination { .. }
            | Modal::Destination { .. }
            | Modal::Name { .. }
            | Modal::Peers { .. } => {
                unreachable!("input and picker dialogs rendered above")
            }
            Modal::Review(review) => (
                if review.operation == Operation::Delete {
                    "Review permanent remote deletion"
                } else if review.operation == Operation::Trash {
                    "Review Move to Trash"
                } else if matches!(review.operation, Operation::Copy | Operation::Move) {
                    "Review transfer"
                } else {
                    "Review operation"
                },
                format!(
                    "{:?} · {} selected sources\n{}{}\n\n{}: {}{}{}\n\nEnter confirms · Esc cancels\nClosing Excavator cancels its jobs.",
                    review.operation,
                    review.sources.len(),
                    review
                        .sources
                        .iter()
                        .take(128)
                        .map(view::display_location)
                        .collect::<Vec<_>>()
                        .join("\n"),
                    if review.sources.len() > 128 {
                        format!(
                            "\n… {} additional selected sources are included.",
                            review.sources.len() - 128
                        )
                    } else {
                        String::new()
                    },
                    if matches!(review.operation, Operation::Copy | Operation::Move) {
                        "Destination"
                    } else {
                        "Parent directory"
                    },
                    view::display_location(review.target.location()),
                    review
                        .new_name
                        .as_ref()
                        .map(|n| format!("\nNew name: {}", view::display_os(n)))
                        .unwrap_or_default(),
                    if review.operation == Operation::Delete {
                        "\nPERMANENT deletion: remote items cannot be recovered from Trash. Files and empty directories only; deletion is nonrecursive."
                    } else if matches!(&review.target, Target::Peer(peer) if peer.is_shell) {
                        "\nLocal shell working directory · rechecked before copying"
                    } else {
                        ""
                    }
                ),
            ),
            Modal::Conflict {
                source,
                destination,
                replace,
                ..
            } => (
                if *replace {
                    "Confirm replacement"
                } else {
                    "File exists"
                },
                format!(
                    "Source: {}\nDestination: {}\n\n{}",
                    view::display_location(source),
                    view::display_location(destination),
                    if *replace {
                        "Enter replaces ALL existing destinations in this job · Esc cancels job"
                    } else {
                        "Policies apply to ALL conflicts in this job.\nk keep both · s skip (copy only) · r review replacement · x cancel job"
                    }
                ),
            ),
            Modal::Log => (
                "Log",
                self.active
                    .as_ref()
                    .map(|m| {
                        m.snapshots()
                            .into_iter()
                            .map(|mut j| {
                                j.id = self.active_label;
                                j
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|job| {
                        !matches!(
                            job.state,
                            JobState::Completed | JobState::Cancelled | JobState::Failed
                        )
                    })
                    .chain(self.queued.iter().flat_map(|(label, m)| {
                        m.snapshots().into_iter().map(move |mut j| {
                            j.id = *label;
                            j
                        })
                    }))
                    .chain(self.logs.clone().into_iter().rev())
                    .map(|j| {
                        format!(
                            "Job {} {:?} {:?}: {} / {} items, {} bytes{}{}",
                            j.id,
                            j.plan.operation,
                            j.state,
                            j.completed_items,
                            j.total_items,
                            j.bytes_copied,
                            j.error
                                .map(|e| format!("\n{}", escaped(&e.to_string())))
                                .unwrap_or_default(),
                            j.journal
                                .iter()
                                .rev()
                                .take(8)
                                .map(|entry| format!("\n  {}", escaped(&entry.description)))
                                .collect::<String>()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
                    + if self.active.is_none() && self.queued.is_empty() && self.logs.is_empty() {
                        "No operations yet."
                    } else {
                        ""
                    },
            ),
        };
        let footer = match modal {
            Modal::Busy { approved: true, .. } => "Preparing approved job",
            Modal::Busy { .. } => "Esc cancel",
            Modal::Review(_) => "Enter confirm · Esc cancel · PgUp/PgDn scroll",
            Modal::Conflict { replace: true, .. } => "Enter replace all · Esc cancel job",
            Modal::Conflict { .. } => "k keep both · s skip · r replace · x cancel",
            Modal::Log => "x cancel active · X cancel waiting · Esc close · PgUp/PgDn scroll",
            _ => "Esc cancel",
        };
        let text = text
            .lines()
            .filter(|line| {
                !line.contains("Enter confirms · Esc cancels")
                    && !line.contains("x cancels active job · X cancels waiting jobs")
                    && !line.contains("Enter replaces ALL existing destinations")
                    && !line.contains("k keep both · s skip (copy only)")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let width = frame
            .area()
            .width
            .saturating_sub(if frame.area().width > 8 { 4 } else { 0 })
            .min(82)
            .saturating_sub(4)
            .max(1);
        let lines = wrapped_lines(text.trim_end(), width);
        let danger = matches!(modal, Modal::Review(r) if r.operation == Operation::Delete)
            || matches!(modal, Modal::Conflict { replace: true, .. });
        let body = modal::panel(
            frame,
            title,
            82,
            lines.len().min(u16::MAX as usize) as u16,
            footer,
            if danger { Tone::Danger } else { Tone::Normal },
        );
        self.page_height.set(body.height.max(1));
        let maximum = lines
            .len()
            .saturating_sub(body.height as usize)
            .min(u16::MAX as usize) as u16;
        self.scroll.set(self.scroll.get().min(maximum));
        let lines = lines
            .into_iter()
            .map(|line| {
                let emphasis = line.starts_with("Destination:")
                    || line.starts_with("Parent directory:")
                    || line.starts_with("New name:")
                    || line.starts_with("Source:")
                    || line.starts_with("Job ");
                let warning =
                    line.contains("PERMANENT deletion") || line.contains("Policies apply to ALL");
                Line::styled(
                    line,
                    if warning {
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
                    } else if emphasis {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                )
            })
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines).scroll((self.scroll.get(), 0)), body);
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.request_generation.store(u64::MAX, Ordering::Release);
        if let Some(coordination) = self
            .worker_coordination
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            coordination.shutdown();
        }
        for manager in self
            .worker_manager
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
        {
            for job in manager.snapshots() {
                manager.cancel(job.id);
            }
        }
    }
}
// Explicit character wrapping keeps scrolling deterministic without Ratatui's
// unstable rendered-line-info API. Display width preserves wide/combining glyphs.
fn wrapped_lines(text: &str, columns: u16) -> Vec<String> {
    let columns = usize::from(columns.max(1));
    let mut lines = Vec::new();
    for line in text.split('\n') {
        let mut current = String::new();
        let mut width = 0;
        for character in line.chars() {
            let character_width = ratatui::text::Span::raw(character.to_string()).width();
            if width + character_width > columns && !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                width = 0;
                if lines.len() >= u16::MAX as usize {
                    return lines;
                }
            }
            current.push(character);
            width += character_width;
        }
        lines.push(current);
        if lines.len() >= u16::MAX as usize {
            break;
        }
    }
    lines
}
fn provider_name(location: &Location) -> &'static str {
    match location {
        Location::Local(_) => "LOCAL",
        Location::Sftp { .. } => "SFTP",
        Location::Ftps { .. } => "FTPS",
        Location::S3 { .. } => "S3",
    }
}
fn copy_choices(peers: Vec<Peer>, favorites: Vec<std::path::PathBuf>) -> Vec<CopyChoice> {
    let mut choices = peers
        .into_iter()
        .map(|peer| CopyChoice {
            label: format!(
                "{} {}",
                if peer.is_shell { "Shell" } else { "Browser" },
                peer.identity
                    .as_ref()
                    .map(|id| id.pane_id.as_str())
                    .unwrap_or(&peer.instance_id)
            ),
            target: Some(Target::Peer(peer)),
            provider_path: false,
            metadata: String::new(),
        })
        .collect::<Vec<_>>();
    if let Some(home) = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        for (label, path) in [
            ("Home", home.clone()),
            ("Downloads", home.join("Downloads")),
            ("Desktop", home.join("Desktop")),
        ] {
            choices.push(CopyChoice {
                label: label.into(),
                target: Some(Target::Manual(Location::Local(path))),
                provider_path: false,
                metadata: String::new(),
            });
        }
    }
    for path in favorites.into_iter().filter(|p| p.is_absolute()) {
        if choices.iter().any(|choice| {
            choice
                .target
                .as_ref()
                .is_some_and(|target| target.location() == &Location::Local(path.clone()))
        }) {
            continue;
        }
        let label = format!(
            "Favorite · {}",
            path.file_name()
                .map(view::display_os)
                .unwrap_or_else(|| "/".into())
        );
        choices.push(CopyChoice {
            label,
            target: Some(Target::Manual(Location::Local(path))),
            provider_path: false,
            metadata: String::new(),
        });
    }
    choices.push(CopyChoice {
        label: "Custom local path".into(),
        target: None,
        provider_path: false,
        metadata: String::new(),
    });
    choices.push(CopyChoice {
        label: "Current provider path".into(),
        target: None,
        provider_path: true,
        metadata: String::new(),
    });
    choices
}
fn render_copy_destination(
    frame: &mut Frame,
    sources: &[Location],
    base: &Location,
    choices: &[CopyChoice],
    index: usize,
    input: &str,
    editing: bool,
    scroll: &Cell<u16>,
    page_height: &Cell<u16>,
) {
    let columns = if frame.area().width >= 76 { 2 } else { 1 };
    let choice_rows = choices.len().div_ceil(columns);
    let body = modal::panel(
        frame,
        "Review copy",
        100,
        (11 + choice_rows.min(5) * 3) as u16,
        "↑↓←→ choose · Tab edit · PgUp/PgDn sources · Enter confirm copy · Esc cancel",
        Tone::Normal,
    );
    if body.height == 0 {
        return;
    }
    let source_height = body.height.saturating_sub(8).min(5).max(1);
    let source_area = Rect::new(body.x, body.y.saturating_add(1), body.width, source_height);
    frame.render_widget(
        Paragraph::new(Line::styled(
            format!(
                "COPY · {} → {} · {} selected source{}",
                sources.first().map(provider_name).unwrap_or("LOCAL"),
                choices[index]
                    .target
                    .as_ref()
                    .map(|target| provider_name(target.location()))
                    .unwrap_or_else(|| if choices[index].provider_path {
                        provider_name(base)
                    } else {
                        "LOCAL"
                    }),
                sources.len(),
                if sources.len() == 1 { "" } else { "s" }
            ),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Rect::new(body.x, body.y, body.width, 1),
    );
    // Common parent plus filenames preserves context; mixed-parent selections show full paths.
    let common_parent = sources.first().and_then(Location::parent).filter(|parent| {
        sources
            .iter()
            .all(|source| source.parent().as_ref() == Some(parent))
    });
    let mut source_lines = Vec::new();
    if let Some(parent) = &common_parent {
        for line in wrapped_lines(&view::display_location(parent), body.width) {
            source_lines.push(Line::styled(line, Style::default().fg(Color::DarkGray)));
        }
    }
    for source in sources {
        let display = view::display_location(source);
        let name = common_parent
            .as_ref()
            .map(|parent| {
                let prefix = format!("{}/", view::display_location(parent).trim_end_matches('/'));
                display.strip_prefix(&prefix).unwrap_or(&display).to_owned()
            })
            .unwrap_or(display);
        source_lines.extend(
            wrapped_lines(&format!("› {name}"), body.width)
                .into_iter()
                .map(Line::raw),
        );
    }
    page_height.set(source_height);
    scroll.set(
        scroll.get().min(
            source_lines
                .len()
                .saturating_sub(source_height as usize)
                .min(u16::MAX as usize) as u16,
        ),
    );
    frame.render_widget(
        Paragraph::new(source_lines).scroll((scroll.get(), 0)),
        source_area,
    );
    let label_y = source_area.bottom().saturating_add(1);
    if label_y >= body.bottom() {
        return;
    }
    frame.render_widget(
        Paragraph::new("CHOOSE DESTINATION · local folders, favorites and panes").style(
            Style::default()
                .fg(Color::Rgb(211, 231, 232))
                .bg(Color::Rgb(40, 66, 70)),
        ),
        Rect::new(body.x, label_y, body.width, 1),
    );
    let card_y = label_y + 1;
    let available = body.bottom().saturating_sub(card_y).saturating_sub(3);
    let visible_rows = usize::from(available / 3).max(1);
    let selected_row = index / columns;
    let first_row = selected_row.saturating_sub(visible_rows.saturating_sub(1));
    for row in first_row..(first_row + visible_rows).min(choice_rows) {
        for col in 0..columns {
            let i = row * columns + col;
            let Some(choice) = choices.get(i) else {
                continue;
            };
            let y = card_y + ((row - first_row) * 3) as u16;
            if y.saturating_add(1) >= body.bottom() {
                continue;
            }
            let width = body.width / columns as u16;
            let x = body.x + col as u16 * width;
            let selected = i == index;
            let style = if selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let path = choice
                .target
                .as_ref()
                .map(|target| view::display_location(target.location()))
                .unwrap_or_else(|| {
                    if choice.provider_path {
                        format!("Path in {}", view::display_location(base))
                    } else {
                        "Enter an absolute local folder path".into()
                    }
                });
            let card = Rect::new(
                x,
                y,
                width.saturating_sub(1),
                3.min(body.bottom().saturating_sub(y)),
            );
            let block = Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(if selected {
                    style
                } else {
                    Style::default().fg(Color::DarkGray)
                })
                .title(Line::styled(
                    format!(
                        " {} {} ",
                        if selected { "◉" } else { "○" },
                        escaped(&choice.label)
                    ),
                    style,
                ));
            let inner = block.inner(card);
            frame.render_widget(block, card);
            frame.render_widget(
                Paragraph::new(path).style(if selected {
                    style
                } else {
                    Style::default().fg(Color::DarkGray)
                }),
                inner,
            );
        }
    }
    let input_y = body.bottom().saturating_sub(2);
    if input_y > card_y {
        let custom = choices[index].target.is_none();
        let label = if custom && choices[index].provider_path {
            "CURRENT PROVIDER DESTINATION PATH"
        } else {
            "CUSTOM LOCAL DESTINATION PATH"
        };
        frame.render_widget(
            Paragraph::new(if custom {
                label
            } else {
                "Select a destination · PgUp/PgDn scroll sources"
            })
            .style(Style::default().fg(Color::DarkGray)),
            Rect::new(body.x, input_y, body.width, 1),
        );
        if custom {
            let area = Rect::new(body.x, input_y + 1, body.width, 1);
            if editing {
                modal::input(frame, area, input);
            } else {
                frame.render_widget(
                    Paragraph::new(if input.is_empty() {
                        "/path/to/folder · Tab to edit"
                    } else {
                        input
                    }),
                    area,
                );
            }
        }
    }
}

fn escaped(text: &str) -> String {
    view::display_os(std::ffi::OsStr::new(text))
}
fn metadata_revision(sources: &[Location], destination: &Location) -> Result<String, String> {
    let ids: std::collections::BTreeSet<_> = sources
        .iter()
        .chain(std::iter::once(destination))
        .filter_map(Location::connection_id)
        .collect();
    if ids.is_empty() {
        return Ok(String::new());
    }
    let records = connection_service::load_saved()?;
    let mut relevant = Vec::new();
    for id in ids {
        relevant.push(
            records
                .iter()
                .find(|record| record.id == id)
                .ok_or("A connection was removed. Review again.")?,
        );
    }
    serde_json::to_string(&relevant).map_err(|_| "Cannot compare connection metadata.".into())
}

fn validate_target(
    coordination: &Coordination,
    review: &Review,
    direction: Option<Direction>,
) -> Result<(), String> {
    if let Target::Peer(peer) = &review.target {
        if review.operation == Operation::Move && peer.is_shell {
            return Err("Shell destinations support Copy only.".into());
        }
        coordination.validate(peer)?;
        if let Some(direction) = direction {
            let eligible = coordination.discover(Some(direction))?;
            if !eligible.iter().any(|candidate| candidate == peer) {
                return Err("Destination is no longer eligible in that direction. Choose and review it again.".into());
            }
        }
    }
    Ok(())
}

fn eligible_peers(
    coordination: &Coordination,
    direction: Option<Direction>,
    operation: Operation,
) -> Result<Vec<Peer>, String> {
    Ok(coordination
        .discover(direction)?
        .into_iter()
        .filter(|peer| operation == Operation::Copy || !peer.is_shell)
        .collect())
}

fn terminal(state: &JobState) -> bool {
    matches!(
        state,
        JobState::Completed | JobState::Failed | JobState::Cancelled
    )
}

#[cfg(test)]
mod modal_tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    // No worker, provider, credential store, or live coordination is created.
    fn runtime(modal: Modal) -> Runtime {
        let (tasks, _) = mpsc::sync_channel(1);
        let (_, replies) = mpsc::sync_channel(1);
        Runtime {
            tasks,
            replies,
            request_generation: Arc::new(AtomicU64::new(0)),
            generation: 0,
            pending_queue: false,
            direction: None,
            worker_manager: Arc::new(Mutex::new(Vec::new())),
            worker_coordination: Arc::new(Mutex::new(None)),
            scroll: Cell::new(0),
            page_height: Cell::new(8),
            copy_columns: Cell::new(1),
            follow_peer: Cell::new(true),
            coordination: None,
            modal: Some(modal),
            active: None,
            queued: VecDeque::new(),
            active_label: 0,
            next_label: 1,
            logs: Vec::new(),
            last_target: None,
            last_poll: Instant::now(),
            location_generation: 0,
            completed: false,
            summary: String::new(),
        }
    }
    fn draw(name: &str, runtime: &Runtime, width: u16, height: u16) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| runtime.render(frame)).unwrap();
        super::super::modal::capture(name, terminal.backend().buffer());
        terminal
    }
    fn text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }
    fn peer(name: &str, path: &str) -> Peer {
        Peer {
            is_shell: true,
            instance_id: name.into(),
            pid: 0,
            location: Location::Local(path.into()),
            generation: 0,
            identity: None,
        }
    }
    fn copy_fixture() -> Modal {
        Modal::CopyDestination {
            sources: vec![
                Location::Local("/source/one.csv".into()),
                Location::Local("/other/two.csv".into()),
            ],
            base: Location::Local("/source".into()),
            choices: vec![CopyChoice {
                label: "Custom local path".into(),
                target: None,
                provider_path: false,
                metadata: "snapshot".into(),
            }],
            index: 0,
            input: String::new(),
            editing: false,
            alternate_input: String::new(),
        }
    }
    #[test]
    fn copy_form_renders_sources_and_local_favorites_without_io() {
        let mut form = copy_fixture();
        if let Modal::CopyDestination { choices, .. } = &mut form {
            *choices = copy_choices(
                vec![peer("test-pane", "/test/shell")],
                vec!["/test/favorite".into()],
            );
        }
        let runtime = runtime(form);
        let rendered = draw("runtime-copy-form", &runtime, 110, 32);
        let output = text(&rendered);
        assert!(output.contains("2 selected sources"));
        assert!(output.contains("/source/one.csv"));
        assert!(output.contains("Favorite"));
        assert!(output.contains("confirm copy"));
        for (width, height) in [(40, 12), (18, 7), (5, 3)] {
            let _ = draw("runtime-copy-form-small", &runtime, width, height);
        }
    }
    #[test]
    fn copy_custom_local_confirmation_preserves_sources_and_snapshot() {
        let mut form = copy_fixture();
        if let Modal::CopyDestination { sources, base, .. } = &mut form {
            sources[0] = Location::Sftp {
                connection: "fixture".into(),
                path: "/source/one.csv".into(),
            };
            *base = Location::Sftp {
                connection: "fixture".into(),
                path: "/source".into(),
            };
        }
        let mut instance = runtime(form);
        let (sender, receiver) = mpsc::sync_channel(1);
        instance.tasks = sender;
        let mut notice = None;
        instance.paste("/test/destination", &mut notice);
        instance.key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut notice,
        );
        let request = receiver.try_recv().unwrap();
        match request.task {
            Task::Queue(review, direction) => {
                assert_eq!(review.sources.len(), 2);
                assert_eq!(review.metadata, "snapshot");
                assert_eq!(
                    review.target.location(),
                    &Location::Local("/test/destination".into())
                );
                assert!(direction.is_none());
            }
            _ => panic!("Copy confirmation must queue the reviewed choice"),
        }
        assert!(instance.pending_queue);
        let mut canceled = runtime(copy_fixture());
        let (sender, receiver) = mpsc::sync_channel(1);
        canceled.tasks = sender;
        canceled.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &mut notice);
        assert!(receiver.try_recv().is_err());
        assert!(canceled.modal.is_none());
    }
    #[test]
    fn copy_grid_navigation_retains_local_and_provider_drafts() {
        let mut form = copy_fixture();
        if let Modal::CopyDestination {
            choices,
            sources,
            base,
            ..
        } = &mut form
        {
            *sources = vec![Location::Sftp {
                connection: "fixture".into(),
                path: "/source/one.csv".into(),
            }];
            *base = Location::Sftp {
                connection: "fixture".into(),
                path: "/source".into(),
            };
            *choices = copy_choices(vec![], vec!["/test/favorite".into()]);
            // Only four fixed fixtures keep navigation deterministic on every machine.
            *choices = vec![
                choices.iter().find(|c| c.target.is_some()).unwrap().clone(),
                CopyChoice {
                    label: "Second local".into(),
                    target: Some(Target::Manual(Location::Local("/second".into()))),
                    provider_path: false,
                    metadata: String::new(),
                },
                CopyChoice {
                    label: "Custom local path".into(),
                    target: None,
                    provider_path: false,
                    metadata: String::new(),
                },
                CopyChoice {
                    label: "Current provider path".into(),
                    target: None,
                    provider_path: true,
                    metadata: String::new(),
                },
            ];
        }
        let mut instance = runtime(form);
        let rendered = draw("runtime-copy-sftp-local", &instance, 110, 32);
        assert!(text(&rendered).contains("SFTP → LOCAL"));
        let mut notice = None;
        instance.key(
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            &mut notice,
        );
        assert!(matches!(
            instance.modal,
            Some(Modal::CopyDestination { index: 2, .. })
        ));
        instance.paste("/local/draft", &mut notice);
        instance.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &mut notice);
        instance.key(
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            &mut notice,
        );
        instance.paste("/remote/draft", &mut notice);
        instance.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &mut notice);
        instance.key(
            KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            &mut notice,
        );
        assert!(
            matches!(&instance.modal,Some(Modal::CopyDestination { index:2,input,.. }) if input=="/local/draft")
        );
        instance.key(
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            &mut notice,
        );
        assert!(
            matches!(&instance.modal,Some(Modal::CopyDestination { index:3,input,.. }) if input=="/remote/draft")
        );
        let _ = draw("runtime-copy-custom-provider", &instance, 110, 32);
    }
    #[test]
    fn destination_cards_fit_content_and_follow_selection() {
        let runtime = runtime(Modal::Peers {
            operation: Operation::Copy,
            sources: Vec::new(),
            peers: vec![
                peer("w1T:pP", "/Users/dorofeev/Personal/excavator"),
                peer("w1T:pV", "/Users/dorofeev"),
            ],
            index: 0,
        });
        let terminal = draw("runtime-destinations", &runtime, 110, 28);
        assert!(text(&terminal).contains("/Users/dorofeev/Personal/excavator"));
        let buffer = terminal.backend().buffer();
        let border_rows = (0..28)
            .filter(|y| (0..110).any(|x| matches!(buffer[(x, *y)].symbol(), "╭" | "╰")))
            .collect::<Vec<_>>();
        assert_eq!(border_rows.len(), 2);
        assert!(border_rows[1] - border_rows[0] < 14);
        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| cell.bg == Color::Cyan && cell.fg == Color::Black)
        );
        let runtime = runtime_many_peers();
        let terminal = draw("runtime-destinations-narrow", &runtime, 40, 12);
        assert!(text(&terminal).contains("w1T:p9"));
    }
    fn runtime_many_peers() -> Runtime {
        runtime(Modal::Peers {
            operation: Operation::Copy,
            sources: Vec::new(),
            peers: (0..10)
                .map(|i| peer(&format!("w1T:p{i}"), "/temporary/destination"))
                .collect(),
            index: 9,
        })
    }
    #[test]
    fn input_and_destructive_review_render_on_small_terminals() {
        let runtime = runtime(Modal::Destination {
            operation: Operation::Copy,
            sources: Vec::new(),
            base: Location::Local("/tmp".into()),
            input: "/some/very/long/path/with/世界/destination".into(),
        });
        let terminal = draw("runtime-input-narrow", &runtime, 40, 12);
        assert!(text(&terminal).contains("destination"));
        let review = runtime_for_review();
        let terminal = draw("runtime-delete-review", &review, 110, 28);
        assert!(text(&terminal).contains("PERMANENT deletion"));
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .any(|cell| cell.fg == Color::Red)
        );
        let conflict = super::modal_tests::runtime(Modal::Conflict {
            id: 1,
            source: Location::Local("/tmp/source".into()),
            destination: Location::Local("/tmp/destination".into()),
            replace: true,
        });
        assert!(
            text(&draw("runtime-replacement", &conflict, 110, 28)).contains("Confirm replacement")
        );
        let log = super::modal_tests::runtime(Modal::Log);
        draw("runtime-log-empty", &log, 110, 28);
    }
    fn runtime_for_review() -> Runtime {
        runtime(Modal::Review(Review {
            operation: Operation::Delete,
            sources: vec![Location::Sftp {
                connection: "fixture".into(),
                path: "/archive/report.csv".into(),
            }],
            target: Target::Manual(Location::Sftp {
                connection: "fixture".into(),
                path: "/archive".into(),
            }),
            metadata: String::new(),
            new_name: None,
        }))
    }
}
