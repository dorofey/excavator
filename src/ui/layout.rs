use super::*;

#[derive(Clone, Copy)]
pub(super) enum Axis {
    Right,
    Down,
}
#[derive(Clone)]
pub(super) enum Layout {
    Leaf(usize),
    Split {
        id: usize,
        axis: Axis,
        state: Entity<ResizableState>,
        _subscription: std::rc::Rc<Subscription>,
        children: [Box<Layout>; 2],
    },
}
impl Layout {
    /// Panes that touch the workspace's top edge and use the title-bar tabs.
    pub fn top_leaves(&self) -> Vec<usize> {
        match self {
            Self::Leaf(id) => vec![*id],
            Self::Split { axis: Axis::Down, children, .. } => children[0].top_leaves(),
            Self::Split { children, .. } => children.iter().flat_map(|c| c.top_leaves()).collect(),
        }
    }
    pub fn leaves(&self) -> Vec<usize> {
        match self {
            Self::Leaf(id) => vec![*id],
            Self::Split { children, .. } => children.iter().flat_map(|c| c.leaves()).collect(),
        }
    }
    pub fn replace_leaf(&mut self, target: usize, replacement: Layout) -> bool {
        match self {
            Self::Leaf(id) if *id == target => {
                *self = replacement;
                true
            }
            Self::Split { children, .. } => children
                .iter_mut()
                .any(|c| c.replace_leaf(target, replacement.clone())),
            _ => false,
        }
    }
    pub fn remove(&mut self, target: usize) -> bool {
        let Self::Split { children, .. } = self else {
            return false;
        };
        for index in 0..2 {
            if matches!(*children[index], Self::Leaf(id) if id == target) {
                *self = (*children[1 - index]).clone();
                return true;
            }
            if children[index].remove(target) {
                return true;
            }
        }
        false
    }
    pub fn nearest(&self, target: usize) -> Option<Entity<ResizableState>> {
        match self {
            Self::Leaf(_) => None,
            Self::Split {
                state, children, ..
            } => {
                let child = children.iter().find(|c| c.leaves().contains(&target))?;
                child.nearest(target).or_else(|| Some(state.clone()))
            }
        }
    }
    /// Left offset and width of every leaf in layout order, so the window's top
    /// tab row can align each pane's strip above its pane.
    pub fn geometry(&self, available: Pixels, cx: &App) -> Vec<(usize, Pixels, Pixels)> {
        let mut leaves = Vec::new();
        self.collect_geometry(px(0.), available, cx, &mut leaves);
        leaves
    }
    fn collect_geometry(
        &self,
        left: Pixels,
        available: Pixels,
        cx: &App,
        out: &mut Vec<(usize, Pixels, Pixels)>,
    ) {
        match self {
            Self::Leaf(id) => out.push((*id, left, available)),
            Self::Split {
                axis,
                state,
                children,
                ..
            } => {
                let width = |index: usize| match axis {
                    Axis::Down => available,
                    Axis::Right => state
                        .read(cx)
                        .sizes()
                        .get(index)
                        .copied()
                        .filter(|width| *width > px(0.))
                        .unwrap_or(available / 2.),
                };
                let first = width(0);
                children[0].collect_geometry(left, first, cx, out);
                let second_left = match axis {
                    Axis::Right => left + first,
                    Axis::Down => left,
                };
                children[1].collect_geometry(second_left, width(1), cx, out);
            }
        }
    }
    pub fn width(&self, target: usize, available: Pixels, cx: &App) -> Pixels {
        match self {
            Self::Leaf(_) => available,
            Self::Split {
                axis,
                state,
                children,
                ..
            } => {
                let index = usize::from(!children[0].leaves().contains(&target));
                let width = match axis {
                    Axis::Down => available,
                    Axis::Right => state
                        .read(cx)
                        .sizes()
                        .get(index)
                        .copied()
                        .filter(|w| *w > px(0.))
                        .unwrap_or(available / 2.),
                };
                children[index].width(target, width, cx)
            }
        }
    }
}
pub(super) struct PaneStore(Vec<Option<Pane>>);
impl PaneStore {
    pub fn new(panes: Vec<Pane>) -> Self {
        Self(panes.into_iter().map(Some).collect())
    }
    pub fn ids(&self) -> Vec<usize> {
        self.0
            .iter()
            .enumerate()
            .filter_map(|(id, p)| p.as_ref().map(|_| id))
            .collect()
    }
    pub fn contains(&self, id: usize) -> bool {
        self.0.get(id).is_some_and(Option::is_some)
    }
    pub fn get_mut(&mut self, id: usize) -> Option<&mut Pane> {
        self.0.get_mut(id)?.as_mut()
    }
    pub fn push(&mut self, pane: Pane) -> usize {
        let id = self.0.len();
        self.0.push(Some(pane));
        id
    }
    pub fn remove(&mut self, id: usize) {
        if let Some(pane) = self.0[id].take() {
            for tab in pane.tabs {
                tab.cancel.cancel();
            }
        }
    }
}
impl std::ops::Index<usize> for PaneStore {
    type Output = Pane;
    fn index(&self, id: usize) -> &Pane {
        self.0[id].as_ref().expect("live pane")
    }
}
impl std::ops::IndexMut<usize> for PaneStore {
    fn index_mut(&mut self, id: usize) -> &mut Pane {
        self.0[id].as_mut().expect("live pane")
    }
}

impl Workspace {
    pub(super) fn observe_split(
        state: &Entity<ResizableState>,
        cx: &mut Context<Self>,
    ) -> std::rc::Rc<Subscription> {
        let mut previous = Vec::new();
        std::rc::Rc::new(cx.observe(state, move |_, state, cx| {
            let sizes = state.read(cx).sizes().to_vec();
            if sizes != previous {
                previous = sizes;
                cx.notify();
            }
        }))
    }
    pub(super) fn root_panes(&self) -> [usize; 2] {
        let Layout::Split { children, .. } = &self.layout else {
            unreachable!("root split")
        };
        [children[0].leaves()[0], children[1].leaves()[0]]
    }
    pub(super) fn transfer_target(&self) -> usize {
        self.recent_pane
            .filter(|id| {
                *id != self.active
                    && self.panes.contains(*id)
                    && self.panes[*id].tab().terminal.is_none()
            })
            .or_else(|| {
                self.layout
                    .leaves()
                    .into_iter()
                    .find(|id| *id != self.active && self.panes[*id].tab().terminal.is_none())
            })
            .unwrap_or_else(|| {
                self.layout
                    .leaves()
                    .into_iter()
                    .find(|id| *id != self.active)
                    .expect("two root groups")
            })
    }
    pub(super) fn focus_pane(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let leaves = self.layout.leaves();
        let position = leaves.iter().position(|id| *id == self.active).unwrap_or(0);
        self.activate(
            leaves[(position as isize + delta).rem_euclid(leaves.len() as isize) as usize],
            window,
            cx,
        );
    }
    pub(super) fn split_pane(&mut self, axis: Axis, window: &mut Window, cx: &mut Context<Self>) {
        self.split_pane_kind(axis, false, window, cx);
    }
    pub(super) fn split_pane_kind(
        &mut self,
        axis: Axis,
        force_terminal: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.preferences_loaded {
            return;
        }
        let path = self.panes[self.active].tab().path.clone();
        let tab = if force_terminal || self.panes[self.active].tab().terminal.is_some() {
            let (launch, label) = match self.terminal_launch() {
                Ok(result) => result,
                Err(error) => {
                    self.notice = Some(error);
                    cx.notify();
                    return;
                }
            };
            self.make_terminal_tab(path.clone(), launch, label, window, cx)
        } else {
            let tab = Tab::new(self.next_id, path.clone());
            self.next_id += 1;
            tab
        };
        let recent_file_tab = tab.terminal.is_none().then_some(tab.id);
        let recent_terminal_tab = tab.terminal.is_some().then_some(tab.id);
        let pane = Pane {
            tabs: vec![tab],
            active: 0,
            recent_file_tab,
            recent_terminal_tab,
            focus: cx.focus_handle(),
            path_input: cx.new(|cx| InputState::new(window, cx).default_value(path.display())),
            scroll: ScrollHandle::new(),
            subscription: None,
        };
        let id = self.panes.push(pane);
        self.subscribe_pane(id, window, cx);
        let state = cx.new(|_| ResizableState::default());
        let subscription = Self::observe_split(&state, cx);
        let split = Layout::Split {
            id: self.next_split_id,
            axis,
            state,
            _subscription: subscription,
            children: [
                Box::new(Layout::Leaf(self.active)),
                Box::new(Layout::Leaf(id)),
            ],
        };
        self.next_split_id += 1;
        self.layout.replace_leaf(self.active, split);
        self.request_listing(id, cx);
        self.activate(id, window, cx);
    }
    /// Whether `id` is a split that can close without emptying an original side.
    pub(super) fn can_close_pane(&self, id: usize) -> bool {
        let Layout::Split { children, .. } = &self.layout else {
            return false;
        };
        children
            .iter()
            .find(|group| group.leaves().contains(&id))
            .is_some_and(|group| group.leaves().len() > 1)
    }
    pub(super) fn close_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Layout::Split { children, .. } = &mut self.layout else {
            return;
        };
        let Some(group) = children
            .iter_mut()
            .find(|group| group.leaves().contains(&self.active))
        else {
            return;
        };
        if group.leaves().len() == 1 {
            self.notice = Some("Keep at least one pane on each side.".into());
            cx.notify();
            return;
        }
        let old = self.active;
        let leaves = group.leaves();
        let position = leaves
            .iter()
            .position(|id| *id == old)
            .expect("active leaf");
        let next = if position > 0 {
            leaves[position - 1]
        } else {
            leaves[1]
        };
        group.remove(old);
        self.panes.remove(old);
        self.recent_pane = self.recent_pane.filter(|id| *id != old);
        self.activate(next, window, cx);
        self.persist(cx);
    }
    pub(super) fn subscribe_pane(
        &mut self,
        id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.panes[id].path_input.clone();
        let subscription =
            cx.subscribe_in(&input, window, move |this, input, event, window, cx| {
                if !this.panes.contains(id) {
                    return;
                }
                match event {
                    InputEvent::Focus => {
                        if this.active != id {
                            this.recent_pane = Some(this.active);
                        }
                        this.active = id;
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => {
                        let value = input.read(cx).value().to_string();
                        match this.panes[id].tab().path.parse_path(&value) {
                            Ok(path) => {
                                this.navigate(id, path, true, window, cx);
                                this.activate(id, window, cx);
                            }
                            Err(error) => {
                                this.notice = Some(error.to_string());
                                cx.notify();
                            }
                        }
                    }
                    _ => {}
                }
            });
        self.panes[id].subscription = Some(subscription);
    }
    pub(super) fn render_layout(
        &self,
        layout: &Layout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match layout {
            Layout::Leaf(id) => self.render_pane(*id, window, cx).into_any_element(),
            Layout::Split {
                id,
                axis,
                state,
                children,
                ..
            } => {
                let group = match axis {
                    Axis::Right => h_resizable(("pane-split", *id)),
                    Axis::Down => v_resizable(("pane-split", *id)),
                };
                // Keep listings and terminal grids inside the size assigned by
                // the splitter. Their intrinsic height must not grow a vertical
                // panel (and push its sibling below the viewport).
                group
                    .with_state(state)
                    .child(
                        resizable_panel()
                            .min_w_0()
                            .min_h_0()
                            .size_range(px(140.)..px(10000.))
                            .child(
                                div().absolute().inset_0().overflow_hidden()
                                    .child(self.render_layout(&children[0], window, cx)),
                            ),
                    )
                    .child(
                        resizable_panel()
                            .min_w_0()
                            .min_h_0()
                            .size_range(px(140.)..px(10000.))
                            .child(
                                div().absolute().inset_0().overflow_hidden()
                                    .child(self.render_layout(&children[1], window, cx)),
                            ),
                    )
                    .into_any_element()
            }
        }
    }
}

impl Workspace {
    /// Runs real workspace command paths in a persistence-free acceptance fixture.
    #[allow(dead_code)]
    pub fn verify_splits(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        assert!(!self.preferences_writable && self.preferences_loaded);
        let original = self.panes[0].tab().id;
        self.panes[0]
            .tab_mut()
            .history
            .push(Location::Local(PathBuf::from("/fixture-history")));
        self.panes[0].tab_mut().selected.insert(7);
        self.command(Command::Split(Axis::Right), window, cx);
        let second = self.active;
        self.command(Command::Split(Axis::Down), window, cx);
        let third = self.active;
        assert_eq!(self.layout.leaves(), vec![0, second, third, 1]);
        assert_eq!(self.panes[0].tab().id, original);
        assert_eq!(self.panes[0].tab().history.len(), 2);
        assert!(self.panes[0].tab().selected.contains(&7));
        assert_eq!(self.panes[third].tab().history.len(), 1);
        assert!(self.panes[third].tab().selected.is_empty());
        assert_eq!(self.transfer_target(), second);
        let destination = Location::Local(PathBuf::from("/fixture-transfer-target"));
        self.panes[second].tab_mut().path = destination.clone();
        let source = self.panes[third].tab().path.clone();
        self.panes[third].tab_mut().entries.push(Entry {
            id: crate::domain::EntryId(source.clone()),
            location: source,
            name: "fixture-item".into(),
            kind: EntryKind::File,
            size: Some(0),
            modified: None,
        });
        self.panes[third].tab_mut().selected.insert(0);
        self.command(Command::Operation(Operation::Copy), window, cx);
        let Some(OperationDialog::Plan(plan)) = &self.operation_dialog else {
            panic!("copy preview");
        };
        assert_eq!(plan.destination, Some(destination));
        self.operation_dialog = None;
        self.panes[second].tab_mut().path = self.panes[0].tab().path.clone();

        self.command(Command::PreviousPane, window, cx);
        assert_eq!(self.active, second);
        self.command(Command::Switch, window, cx);
        assert_eq!(self.active, third);
        let token = self.panes[third].tab().cancel.clone();
        self.command(Command::ClosePane, window, cx);
        assert!(!self.panes.contains(third));
        assert!(token.is_cancelled());
        assert_eq!(self.layout.leaves(), vec![0, second, 1]);
        assert_eq!(
            self.active, second,
            "closing nested leaf focuses adjacent survivor"
        );
        self.activate(0, window, cx);
        self.command(Command::Split(Axis::Down), window, cx);
        let replacement = self.active;
        assert!(replacement > third, "pane IDs never reused");
        self.drop_copy(
            PaneDrag {
                source_pane: third,
                source_tab_id: 0,
                sources: vec![],
                external_paths: None,
            },
            replacement,
            self.panes[replacement].tab().id,
            cx,
        );
        assert!(self.jobs.is_empty());
        assert!(
            self.notice
                .as_deref()
                .is_some_and(|notice| notice.contains("closed during"))
        );
        self.activate(0, window, cx);
        self.command(Command::ClosePane, window, cx);
        assert!(!self.panes.contains(0));
        assert_eq!(self.root_panes()[0], replacement);
        assert_eq!(
            self.active, replacement,
            "closing first leaf focuses next survivor"
        );
        self.activate(second, window, cx);
        self.command(Command::ClosePane, window, cx);
        self.command(Command::ClosePane, window, cx);
        assert!(self.panes.contains(replacement));
        assert_eq!(self.layout.leaves(), vec![replacement, 1]);
        self.command(Command::Split(Axis::Right), window, cx);
        self.command(Command::Split(Axis::Down), window, cx);
        let last_tab_pane = self.active;
        assert_eq!(self.panes[last_tab_pane].tabs.len(), 1);
        self.command(Command::CloseTab, window, cx);
        assert!(
            !self.panes.contains(last_tab_pane),
            "closing a split's last tab closes the split"
        );
        let sole = self.root_panes()[1];
        assert_eq!(self.layout.leaves().last(), Some(&sole));
        self.activate(sole, window, cx);
        let tabs = self.panes[sole].tabs.len();
        self.command(Command::CloseTab, window, cx);
        if tabs == 1 {
            assert!(
                self.panes.contains(sole),
                "sole pane on a side keeps its tab"
            );
        }

        let total = px(900.);
        let geometry = self.layout.geometry(total, cx);
        assert_eq!(
            geometry.iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
            self.layout.leaves(),
            "top tab row covers every pane in layout order"
        );
        assert_eq!(geometry[0].1, px(0.), "first strip starts at the row edge");
        assert!(
            geometry.windows(2).all(|pair| pair[0].1 <= pair[1].1),
            "strips advance left to right: {geometry:?}"
        );
        let last = geometry.last().expect("at least one strip");
        assert!(
            last.1 + last.2 <= total && last.2 > px(0.),
            "strips stay inside the row: {geometry:?}"
        );
        assert!(
            geometry.iter().all(|(id, _, _)| self.panes.contains(*id)),
            "strips only reference live panes"
        );

        let tab = self.panes[sole].tab_mut();
        let root = tab.path.clone();
        let entry = |location: Location, name: &str, kind: EntryKind| Entry {
            id: crate::domain::EntryId(location.clone()),
            location,
            name: name.into(),
            kind,
            size: None,
            modified: None,
        };
        let folder = root.join(std::ffi::OsStr::new("folder")).unwrap();
        let nested = folder.join(std::ffi::OsStr::new("nested.txt")).unwrap();
        tab.root_entries = vec![
            entry(
                root.join(std::ffi::OsStr::new("z.txt")).unwrap(),
                "z.txt",
                EntryKind::File,
            ),
            entry(folder.clone(), "folder", EntryKind::Directory),
        ];
        tab.sort_entries();
        assert_eq!(tab.entries[0].location, folder, "folders sort first");
        tab.selected.insert(1);
        tab.selection_cursor = Some(1);
        let token = tab.tree.begin(&folder);
        assert!(tab.tree.finish(
            &folder,
            &token,
            super::tree::Children::Loaded(vec![entry(
                nested.clone(),
                "nested.txt",
                EntryKind::File
            )]),
        ));
        tab.rebuild_rows();
        assert_eq!(tab.entries.len(), 3);
        assert_eq!((tab.entries[1].location.clone(), tab.depth(1)), (nested, 1));
        assert!(tab.selected.contains(&2), "selection follows its location");
        let stale = tab.tree.begin(&folder);
        tab.tree.collapse(&folder);
        assert!(
            !tab.tree
                .finish(&folder, &stale, super::tree::Children::Loaded(vec![]))
        );
        tab.rebuild_rows();
        assert_eq!(tab.entries.len(), 2);
        assert_eq!(tab.selection_cursor, Some(1));
        println!(
            "PASS: real split/close/focus commands; independent state; cancelled pending pane; stable IDs; collapsed roots; stale drag rejected; recent transfer destination beyond pane 1; last-tab split close; tree rows/selection/stale expansion; top tab-row geometry"
        );
    }
}
