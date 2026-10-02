//! Inline folder expansion shared by pane listings and the sidebar.
use super::*;
use std::collections::HashMap;

pub(super) enum Children {
    Loading,
    Loaded(Vec<Entry>),
    Failed(String),
}

#[derive(Default)]
pub(super) struct TreeState {
    pub expanded: BTreeSet<Location>,
    pub children: HashMap<Location, Children>,
    cancels: HashMap<Location, CancellationToken>,
}
impl TreeState {
    pub fn is_expanded(&self, location: &Location) -> bool {
        self.expanded.contains(location)
    }
    pub fn is_loading(&self, location: &Location) -> bool {
        matches!(self.children.get(location), Some(Children::Loading))
    }
    /// Cancels pending loads and drops loaded children; expansion choices remain.
    pub fn reset_children(&mut self) {
        for (_, token) in self.cancels.drain() {
            token.cancel();
        }
        self.children.clear();
    }
    pub fn clear(&mut self) {
        self.reset_children();
        self.expanded.clear();
    }
    /// Marks `location` expanded and loading; returns the token for its request.
    pub fn begin(&mut self, location: &Location) -> CancellationToken {
        if let Some(old) = self.cancels.remove(location) {
            old.cancel();
        }
        let token = CancellationToken::new();
        self.expanded.insert(location.clone());
        self.children.insert(location.clone(), Children::Loading);
        self.cancels.insert(location.clone(), token.clone());
        token
    }
    /// Applies a finished load only if the folder is still expanded and waiting for it.
    pub fn finish(
        &mut self,
        location: &Location,
        token: &CancellationToken,
        result: Children,
    ) -> bool {
        // Re-expanding, collapsing, or resetting cancels the superseded token.
        if token.is_cancelled() || !self.expanded.contains(location) {
            return false;
        }
        self.cancels.remove(location);
        self.children.insert(location.clone(), result);
        true
    }
    /// Collapses a folder and every expanded descendant.
    pub fn collapse(&mut self, location: &Location) {
        let removed = self
            .expanded
            .iter()
            .filter(|candidate| *candidate == location || is_descendant(candidate, location))
            .cloned()
            .collect::<Vec<_>>();
        for item in removed {
            self.expanded.remove(&item);
            self.children.remove(&item);
            if let Some(token) = self.cancels.remove(&item) {
                token.cancel();
            }
        }
    }
}

pub(super) fn is_descendant(candidate: &Location, ancestor: &Location) -> bool {
    let mut current = candidate.parent();
    while let Some(parent) = current {
        if &parent == ancestor {
            return true;
        }
        current = parent.parent();
    }
    false
}

pub(super) fn sort_list(entries: &mut [Entry], sort: Sort, descending: bool) {
    entries.sort_by(|a, b| {
        (a.kind != EntryKind::Directory)
            .cmp(&(b.kind != EntryKind::Directory))
            .then_with(|| {
                let order = match sort {
                    Sort::Name => a.name.cmp(&b.name),
                    Sort::Kind => format!("{:?}", a.kind).cmp(&format!("{:?}", b.kind)),
                    Sort::Size => a.size.cmp(&b.size),
                    Sort::Modified => a.modified.cmp(&b.modified),
                };
                if descending { order.reverse() } else { order }
            })
            .then_with(|| a.name.cmp(&b.name))
    });
}

impl Tab {
    pub(super) fn depth(&self, row: usize) -> usize {
        self.depths.get(row).copied().unwrap_or(0)
    }
    /// Flattens the root listing and loaded expanded folders into visible rows,
    /// preserving selection, anchor, and cursor by location.
    pub(super) fn rebuild_rows(&mut self) {
        let location_at = |row: Option<usize>| {
            row.and_then(|row| self.entries.get(row))
                .map(|entry| entry.location.clone())
        };
        let selected = self
            .selected
            .iter()
            .filter_map(|row| self.entries.get(*row))
            .map(|entry| entry.location.clone())
            .collect::<BTreeSet<_>>();
        let anchor = location_at(self.anchor);
        let cursor = location_at(self.selection_cursor);
        fn push(
            entries: &[Entry],
            depth: usize,
            tree: &TreeState,
            rows: &mut Vec<Entry>,
            depths: &mut Vec<usize>,
        ) {
            for entry in entries {
                rows.push(entry.clone());
                depths.push(depth);
                if entry.kind == EntryKind::Directory
                    && tree.is_expanded(&entry.location)
                    && let Some(Children::Loaded(children)) = tree.children.get(&entry.location)
                {
                    push(children, depth + 1, tree, rows, depths);
                }
            }
        }
        let mut rows = Vec::with_capacity(self.root_entries.len());
        let mut depths = Vec::with_capacity(self.root_entries.len());
        push(&self.root_entries, 0, &self.tree, &mut rows, &mut depths);
        self.entries = rows;
        self.depths = depths;
        let find = |location: &Option<Location>| {
            location
                .as_ref()
                .and_then(|l| self.entries.iter().position(|e| &e.location == l))
        };
        self.anchor = find(&anchor);
        self.selection_cursor = find(&cursor);
        self.selected = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| selected.contains(&entry.location))
            .map(|(row, _)| row)
            .collect();
    }
}

impl Workspace {
    pub(super) fn toggle_row(&mut self, i: usize, row: usize, cx: &mut Context<Self>) {
        if !self.panes.contains(i) {
            return;
        }
        let tab = self.panes[i].tab_mut();
        let Some(entry) = tab.entries.get(row) else {
            return;
        };
        if entry.kind != EntryKind::Directory {
            return;
        }
        let location = entry.location.clone();
        let tab_id = tab.id;
        if tab.tree.is_expanded(&location) {
            tab.tree.collapse(&location);
            tab.rebuild_rows();
            cx.notify();
        } else {
            self.expand_location(i, tab_id, location, cx);
        }
    }
    pub(super) fn expand_location(
        &mut self,
        i: usize,
        tab_id: u64,
        location: Location,
        cx: &mut Context<Self>,
    ) {
        let show_hidden = self.preferences.show_hidden;
        let registry = self.registry.clone();
        let Some(tab) = self
            .panes
            .get_mut(i)
            .and_then(|pane| pane.tabs.iter_mut().find(|tab| tab.id == tab_id))
        else {
            return;
        };
        let token = tab.tree.begin(&location);
        let generation = tab.generation;
        let request = registry.list(location.clone(), ListOptions { show_hidden }, token.clone());
        let task = cx.background_executor().spawn(request);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let Some(i) = this.panes.ids().into_iter().find(|pane| this.panes[*pane].tabs.iter().any(|tab| tab.id == tab_id)) else { return; };
                let Some(pane) = this.panes.get_mut(i) else {
                    return;
                };
                let Some(tab) = pane
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id == tab_id && tab.generation == generation)
                else {
                    return;
                };
                let (sort, descending) = (tab.sort, tab.descending);
                let failure = result.as_ref().err().map(|error| error.to_string());
                let children = match result {
                    Ok(mut entries) => {
                        sort_list(&mut entries, sort, descending);
                        Children::Loaded(entries)
                    }
                    Err(error) => Children::Failed(error.to_string()),
                };
                if !tab.tree.finish(&location, &token, children) {
                    return;
                }
                if let Some(error) = failure {
                    tab.tree.collapse(&location);
                    this.notice = Some(format!("Could not expand {}: {error}", location.label()));
                }
                let tab = this.panes[i]
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id == tab_id)
                    .expect("tab checked above");
                tab.rebuild_rows();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    /// → expands the cursor folder or steps into its first child.
    pub(super) fn expand_selection(&mut self, cx: &mut Context<Self>) {
        let i = self.active;
        let tab = self.panes[i].tab();
        let Some(row) = tab.selection_cursor else {
            return;
        };
        let Some(entry) = tab.entries.get(row) else {
            return;
        };
        if entry.kind != EntryKind::Directory {
            return;
        }
        if !tab.tree.is_expanded(&entry.location) {
            self.toggle_row(i, row, cx);
        } else if tab.depth(row + 1) > tab.depth(row) && row + 1 < tab.entries.len() {
            self.select(i, row + 1, false, false, cx);
        }
    }
    /// ← collapses the cursor folder or moves to its parent row.
    pub(super) fn collapse_selection(&mut self, cx: &mut Context<Self>) {
        let i = self.active;
        let tab = self.panes[i].tab();
        let Some(row) = tab.selection_cursor else {
            return;
        };
        let Some(entry) = tab.entries.get(row) else {
            return;
        };
        if entry.kind == EntryKind::Directory && tab.tree.is_expanded(&entry.location) {
            self.toggle_row(i, row, cx);
            return;
        }
        let depth = tab.depth(row);
        if depth == 0 {
            return;
        }
        if let Some(parent) = (0..row).rev().find(|r| tab.depth(*r) < depth) {
            self.select(i, parent, false, false, cx);
        }
    }
}
