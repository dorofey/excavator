mod layout;
mod terminal;
mod terminal_commands;
use terminal::TerminalView;
mod settings;
use crate::appearance::Tokens;
use crate::persistence::{AppearanceMode, AppearanceSettings, DarkTheme, LightTheme, RowDensity};
use layout::{Axis, Layout, PaneStore};
mod connections;
use crate::connections::{ConnectionRecord, ConnectionSecrets, Protocol};
use connections::ConnectionScreen;
mod operations;
use crate::transfers::{JobSnapshot, Operation, TransferManager};
use crate::{
    domain::{Entry, EntryKind, Location},
    persistence::{self, Preferences},
    providers::{CancellationToken, FileSystem, ListOptions, ProviderRegistry},
};
use gpui_kit::{
    component::{
        Disableable, Icon, IconName, Selectable,
        button::{Button, ButtonVariants},
        input::{Input, InputEvent, InputState},
        resizable::{ResizableState, h_resizable, resizable_panel, v_resizable},
    },
    prelude::*,
    *,
};
use operations::OperationDialog;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
actions!(excavator_settings, [Settings]);
actions!(
    excavator_workspace,
    [
        SwitchPane,
        PreviousPane,
        SplitRight,
        SplitDown,
        SplitTerminalRight,
        SplitTerminalDown,
        ClosePane,
        ToggleSidebar,
        Back,
        Forward,
        Parent,
        Refresh,
        NewTab,
        CloseTab,
        NextTab,
        PreviousTab,
        MoveTabLeft,
        MoveTabRight,
        SelectNext,
        SelectPrevious,
        OpenSelection,
        SelectAll,
        EditPath,
        ToggleHidden,
        TogglePalette,
        Escape,
        AddFavorite,
        CreateFolder,
        RenameItem,
        CopyItems,
        MoveItems,
        TrashItems,
        ToggleTransfers,
        ToggleTerminal,
        FocusTerminal,
        NewTerminal,
        EndTerminal,
        FocusFiles,
        ConfirmOperation,
        RemoveFavorite,
        ExtendSelectionNext,
        ExtendSelectionPrevious,
        GrowLeftPane,
        ShrinkLeftPane,
        ChooseFolder,
        ManageConnections,
        NewConnection,
        ImportForkLift,
        NextSettingsField,
        PreviousSettingsField,
        NextConnectionField,
        PreviousConnectionField
    ]
);
#[derive(Clone, Copy)]
enum Sort {
    Name,
    Kind,
    Size,
    Modified,
}
struct Tab {
    id: u64,
    terminal: Option<Entity<TerminalView>>,
    terminal_subscription: Option<Subscription>,
    terminal_label: String,
    terminal_launch: Option<crate::terminal::Launch>,
    path: Location,
    history: Vec<Location>,
    cursor: usize,
    entries: Vec<Entry>,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    selection_cursor: Option<usize>,
    loading: bool,
    error: Option<String>,
    generation: u64,
    cancel: CancellationToken,
    sort: Sort,
    descending: bool,
}
#[derive(Clone)]
struct PaneDrag {
    source_pane: usize,
    source_tab_id: u64,
    sources: Vec<Location>,
    external_paths: Option<Vec<(PathBuf, bool)>>,
}

struct PaneDragPreview {
    label: String,
}

impl Render for PaneDragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(0x252a34))
            .text_color(rgb(0xffffff))
            .child(self.label.clone())
    }
}

impl Tab {
    fn new(id: u64, path: Location) -> Self {
        Self {
            id,
            terminal: None,
            terminal_subscription: None,
            terminal_label: String::new(),
            terminal_launch: None,
            path: path.clone(),
            history: vec![path],
            cursor: 0,
            entries: vec![],
            selected: BTreeSet::new(),
            anchor: None,
            selection_cursor: None,
            loading: false,
            error: None,
            generation: 0,
            cancel: CancellationToken::new(),
            sort: Sort::Name,
            descending: false,
        }
    }
    fn sort_entries(&mut self) {
        let sort = self.sort;
        let desc = self.descending;
        self.entries.sort_by(|a, b| {
            (a.kind != EntryKind::Directory)
                .cmp(&(b.kind != EntryKind::Directory))
                .then_with(|| {
                    let order = match sort {
                        Sort::Name => a.name.cmp(&b.name),
                        Sort::Kind => format!("{:?}", a.kind).cmp(&format!("{:?}", b.kind)),
                        Sort::Size => a.size.cmp(&b.size),
                        Sort::Modified => a.modified.cmp(&b.modified),
                    };
                    if desc { order.reverse() } else { order }
                })
                .then_with(|| a.name.cmp(&b.name))
        });
        self.selected.clear();
        self.anchor = None;
        self.selection_cursor = None;
    }
}
struct Pane {
    tabs: Vec<Tab>,
    active: usize,
    recent_file_tab: Option<u64>,
    recent_terminal_tab: Option<u64>,
    focus: FocusHandle,
    path_input: Entity<InputState>,
    scroll: ScrollHandle,
    subscription: Option<Subscription>,
}
impl Pane {
    fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }
    fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }
    fn file_tab(&self) -> Option<&Tab> {
        if self.tab().terminal.is_none() {
            return Some(self.tab());
        }
        self.recent_file_tab
            .and_then(|id| {
                self.tabs
                    .iter()
                    .find(|tab| tab.id == id && tab.terminal.is_none())
            })
            .or_else(|| self.tabs.iter().find(|tab| tab.terminal.is_none()))
    }
}
#[derive(Clone, Copy)]
enum Command {
    Switch,
    PreviousPane,
    Split(Axis),
    SplitTerminal(Axis),
    ClosePane,
    Sidebar,
    Back,
    Forward,
    Parent,
    Refresh,
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
    MoveTabLeft,
    MoveTabRight,
    Hidden,
    Favorite,
    RemoveFavorite,
    EditPath,
    ChooseFolder,
    Open,
    SelectAll,
    Operation(Operation),
    Transfers,
    Terminal,
    FocusTerminal,
    NewTerminal,
    EndTerminal,
    FocusFiles,
    Conflict(crate::transfers::ConflictPolicy),
    CancelTransfer,
    ResizeLeft(f32),
    Connections,
    NewConnection,
    ImportForkLift,
    TestConnection,
    EditConnection,
    RemoveConnection,
    ResetHost,
    SaveConnection,
    TestDraftConnection,
    ConfirmConnection,
    ConnectionProtocol(Protocol),
    SavedConnection(usize, u8),
    Settings,
    AppearanceMode(AppearanceMode),
    LightTheme(LightTheme),
    DarkTheme(DarkTheme),
    Density(RowDensity),
    ResetAppearance,
}
const COMMANDS: &[(&str, &str, Command)] = &[
    ("Settings: Appearance", "⌘,", Command::Settings),
    ("Choose folder for active pane", "⌘O", Command::ChooseFolder),
    (
        "Appearance mode: Light",
        "",
        Command::AppearanceMode(AppearanceMode::Light),
    ),
    (
        "Appearance mode: Dark",
        "",
        Command::AppearanceMode(AppearanceMode::Dark),
    ),
    (
        "Appearance mode: System",
        "",
        Command::AppearanceMode(AppearanceMode::System),
    ),
    (
        "Light theme: Paper",
        "",
        Command::LightTheme(LightTheme::Paper),
    ),
    (
        "Light theme: Frost",
        "",
        Command::LightTheme(LightTheme::Frost),
    ),
    (
        "Dark theme: Graphite",
        "",
        Command::DarkTheme(DarkTheme::Graphite),
    ),
    (
        "Dark theme: Midnight",
        "",
        Command::DarkTheme(DarkTheme::Midnight),
    ),
    (
        "Row density: Compact",
        "",
        Command::Density(RowDensity::Compact),
    ),
    (
        "Row density: Comfortable",
        "",
        Command::Density(RowDensity::Comfortable),
    ),
    (
        "Row density: Spacious",
        "",
        Command::Density(RowDensity::Spacious),
    ),
    ("Reset appearance to defaults", "", Command::ResetAppearance),
    ("Save connection editor", "", Command::SaveConnection),
    ("Test connection editor", "", Command::TestDraftConnection),
    (
        "Confirm pending connection action",
        "",
        Command::ConfirmConnection,
    ),
    (
        "Use SFTP in connection editor",
        "",
        Command::ConnectionProtocol(Protocol::Sftp),
    ),
    (
        "Use FTPS in connection editor",
        "",
        Command::ConnectionProtocol(Protocol::Ftps),
    ),
    (
        "Use S3 in connection editor",
        "",
        Command::ConnectionProtocol(Protocol::S3),
    ),
    ("Manage connections", "⌘ShiftC", Command::Connections),
    ("Add connection", "", Command::NewConnection),
    ("Test active connection", "", Command::TestConnection),
    ("Edit active connection", "", Command::EditConnection),
    ("Remove active connection", "", Command::RemoveConnection),
    (
        "Reset trusted host for active connection",
        "",
        Command::ResetHost,
    ),
    (
        "Grow nearest pane split",
        "CtrlAlt→",
        Command::ResizeLeft(40.),
    ),
    (
        "Shrink nearest pane split",
        "CtrlAlt←",
        Command::ResizeLeft(-40.),
    ),
    (
        "Remove current folder from favorites",
        "⌘ShiftD",
        Command::RemoveFavorite,
    ),
    (
        "Keep both for pending conflict",
        "",
        Command::Conflict(crate::transfers::ConflictPolicy::KeepBoth),
    ),
    (
        "Skip pending conflict",
        "",
        Command::Conflict(crate::transfers::ConflictPolicy::Skip),
    ),
    (
        "Review replacing pending conflict",
        "",
        Command::Conflict(crate::transfers::ConflictPolicy::Replace),
    ),
    ("Cancel current transfer", "", Command::CancelTransfer),
    (
        "Create folder",
        "⌘ShiftN",
        Command::Operation(Operation::CreateDirectory),
    ),
    (
        "Rename selected item",
        "F2",
        Command::Operation(Operation::Rename),
    ),
    (
        "Copy selected to other pane",
        "F5",
        Command::Operation(Operation::Copy),
    ),
    (
        "Move selected to other pane",
        "F6",
        Command::Operation(Operation::Move),
    ),
    (
        "Move selected to Trash",
        "⌘Backspace",
        Command::Operation(Operation::Trash),
    ),
    ("Toggle transfer queue", "⌘J", Command::Transfers),
    ("Toggle terminal", "⌃`", Command::Terminal),
    ("Focus terminal", "⌘⌥J", Command::FocusTerminal),
    (
        "Import connections from ForkLift…",
        "⌘⌥I",
        Command::ImportForkLift,
    ),
    ("New terminal tab", "⌘⌥T", Command::NewTerminal),
    (
        "Split terminal right",
        "⌘⌥⇧→",
        Command::SplitTerminal(Axis::Right),
    ),
    (
        "Split terminal down",
        "⌘⌥⇧↓",
        Command::SplitTerminal(Axis::Down),
    ),
    (
        "End terminal session (stops running commands)",
        "⌘⌥K",
        Command::EndTerminal,
    ),
    ("Focus active file pane", "⌘⌥F", Command::FocusFiles),
    ("Edit current path", "⌘L", Command::EditPath),
    ("Open selected item", "Enter", Command::Open),
    ("Select all items", "⌘A", Command::SelectAll),
    ("Focus next pane", "Tab / ⌘⌥]", Command::Switch),
    ("Focus previous pane", "⇧Tab / ⌘⌥[", Command::PreviousPane),
    ("Split pane right", "⌘⌥→", Command::Split(Axis::Right)),
    ("Split pane down", "⌘⌥↓", Command::Split(Axis::Down)),
    ("Close pane split", "⌘⌥W", Command::ClosePane),
    ("Toggle sidebar", "⌘B", Command::Sidebar),
    ("Back", "⌘[", Command::Back),
    ("Forward", "⌘]", Command::Forward),
    ("Parent folder", "⌘↑", Command::Parent),
    ("Refresh", "⌘R", Command::Refresh),
    ("New tab", "⌘T", Command::NewTab),
    ("Close tab", "⌘W", Command::CloseTab),
    ("Next tab", "Ctrl Tab", Command::NextTab),
    ("Previous tab", "Ctrl Shift Tab", Command::PreviousTab),
    ("Move tab left", "⌘Shift[", Command::MoveTabLeft),
    ("Move tab right", "⌘Shift]", Command::MoveTabRight),
    ("Toggle hidden files", "⌘Shift.", Command::Hidden),
    ("Add current folder to favorites", "⌘D", Command::Favorite),
];
pub struct Workspace {
    panes: PaneStore,
    layout: Layout,
    next_split_id: usize,
    recent_pane: Option<usize>,
    registry: ProviderRegistry,
    colors: Tokens,
    settings_open: bool,
    settings_inputs: Vec<Entity<InputState>>,
    settings_scroll: ScrollHandle,
    connections: Vec<ConnectionRecord>,
    connection_screen: Option<ConnectionScreen>,
    connection_inputs: Vec<Entity<InputState>>,
    connection_busy: bool,
    connection_cancel: CancellationToken,
    connection_generation: u64,
    connection_scroll: ScrollHandle,
    active: usize,
    preferences: Preferences,
    notice: Option<String>,
    palette: bool,
    palette_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
    next_id: u64,
    saving: bool,
    preferences_writable: bool,
    preferences_loaded: bool,
    save_pending: bool,
    transfers: TransferManager,
    jobs: Vec<JobSnapshot>,
    transfer_drawer: bool,
    operation_dialog: Option<OperationDialog>,
    operation_input: Entity<InputState>,
    operation_focus: FocusHandle,
}
impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_registry(window, cx, ProviderRegistry::default())
    }
    /// Explicit fixture dependency injection; the production constructor uses Keychain.
    pub fn with_registry(
        window: &mut Window,
        cx: &mut Context<Self>,
        registry: ProviderRegistry,
    ) -> Self {
        Self::build(window, cx, registry, None)
    }
    /// Isolated acceptance window: no persisted preferences, connections, or secrets.
    #[allow(dead_code)]
    pub fn fixture(window: &mut Window, cx: &mut Context<Self>, path: PathBuf) -> Self {
        Self::build(window, cx, ProviderRegistry::default(), Some(path))
    }
    fn build(
        window: &mut Window,
        cx: &mut Context<Self>,
        registry: ProviderRegistry,
        fixture: Option<PathBuf>,
    ) -> Self {
        cx.bind_keys([
            KeyBinding::new("tab", SwitchPane, Some("Workspace && !Terminal")),
            KeyBinding::new("shift-tab", PreviousPane, Some("Workspace && !Terminal")),
            KeyBinding::new("cmd-alt-right", SplitRight, Some("Workspace")),
            KeyBinding::new("cmd-alt-shift-right", SplitTerminalRight, Some("Workspace")),
            KeyBinding::new("cmd-alt-shift-down", SplitTerminalDown, Some("Workspace")),
            KeyBinding::new("cmd-alt-down", SplitDown, Some("Workspace")),
            KeyBinding::new("cmd-alt-w", ClosePane, Some("Workspace")),
            KeyBinding::new("cmd-alt-]", SwitchPane, Some("Workspace")),
            KeyBinding::new("cmd-alt-[", PreviousPane, Some("Workspace")),
            KeyBinding::new("cmd-b", ToggleSidebar, Some("Workspace")),
            KeyBinding::new("cmd-[", Back, Some("Workspace")),
            KeyBinding::new("cmd-]", Forward, Some("Workspace")),
            KeyBinding::new("cmd-up", Parent, Some("Workspace")),
            KeyBinding::new("cmd-r", Refresh, Some("Workspace")),
            KeyBinding::new("cmd-t", NewTab, Some("Workspace")),
            KeyBinding::new("cmd-w", CloseTab, Some("Workspace")),
            KeyBinding::new("ctrl-tab", NextTab, Some("Workspace && !Terminal")),
            KeyBinding::new(
                "ctrl-shift-tab",
                PreviousTab,
                Some("Workspace && !Terminal"),
            ),
            KeyBinding::new("cmd-shift-[", MoveTabLeft, Some("Workspace")),
            KeyBinding::new("cmd-shift-]", MoveTabRight, Some("Workspace")),
            KeyBinding::new("down", SelectNext, Some("Listing")),
            KeyBinding::new("up", SelectPrevious, Some("Listing")),
            KeyBinding::new("enter", OpenSelection, Some("Listing")),
            KeyBinding::new("cmd-a", SelectAll, Some("Listing")),
            KeyBinding::new("cmd-l", EditPath, Some("Workspace")),
            KeyBinding::new("cmd-o", ChooseFolder, Some("Workspace")),
            KeyBinding::new("cmd-shift-.", ToggleHidden, Some("Workspace")),
            KeyBinding::new("cmd-shift-p", TogglePalette, Some("Workspace")),
            KeyBinding::new("escape", Escape, Some("Workspace && !Terminal")),
            KeyBinding::new("cmd-d", AddFavorite, Some("Workspace")),
            KeyBinding::new("cmd-shift-d", RemoveFavorite, Some("Workspace")),
            KeyBinding::new("shift-down", ExtendSelectionNext, Some("Listing")),
            KeyBinding::new("shift-up", ExtendSelectionPrevious, Some("Listing")),
            KeyBinding::new(
                "ctrl-alt-right",
                GrowLeftPane,
                Some("Workspace && !Terminal"),
            ),
            KeyBinding::new(
                "ctrl-alt-left",
                ShrinkLeftPane,
                Some("Workspace && !Terminal"),
            ),
        ]);
        cx.bind_keys([
            KeyBinding::new("cmd-shift-n", CreateFolder, Some("Workspace")),
            KeyBinding::new("f2", RenameItem, Some("Listing")),
            KeyBinding::new("f5", CopyItems, Some("Listing")),
            KeyBinding::new("f6", MoveItems, Some("Listing")),
            KeyBinding::new("cmd-backspace", TrashItems, Some("Listing")),
            KeyBinding::new("cmd-j", ToggleTransfers, Some("Workspace")),
            KeyBinding::new("ctrl-`", ToggleTerminal, Some("Workspace")),
            KeyBinding::new("cmd-alt-j", FocusTerminal, Some("Workspace")),
            KeyBinding::new("cmd-alt-t", NewTerminal, Some("Workspace")),
            KeyBinding::new("cmd-alt-i", ImportForkLift, Some("Workspace")),
            KeyBinding::new("cmd-alt-k", EndTerminal, Some("Workspace")),
            KeyBinding::new("cmd-alt-f", FocusFiles, Some("Workspace")),
            KeyBinding::new("enter", ConfirmOperation, Some("OperationConfirm")),
        ]);
        cx.bind_keys([KeyBinding::new(
            "cmd-shift-c",
            ManageConnections,
            Some("Workspace"),
        )]);
        cx.bind_keys([
            KeyBinding::new(
                "tab",
                NextConnectionField,
                Some("ConnectionEditor && Input"),
            ),
            KeyBinding::new(
                "shift-tab",
                PreviousConnectionField,
                Some("ConnectionEditor && Input"),
            ),
        ]);
        let connection_inputs = (0..13)
            .map(|i| cx.new(|cx| InputState::new(window, cx).masked(i >= 9)))
            .collect::<Vec<_>>();
        let operation_input = cx.new(|cx| InputState::new(window, cx).placeholder("Name"));
        cx.bind_keys([
            KeyBinding::new("cmd-,", Settings, Some("Workspace")),
            KeyBinding::new("tab", NextSettingsField, Some("AppearanceSettings")),
            KeyBinding::new(
                "shift-tab",
                PreviousSettingsField,
                Some("AppearanceSettings"),
            ),
            KeyBinding::new("escape", Escape, Some("AppearanceSettings")),
        ]);
        let isolated = fixture.is_some();
        let mut prefs = Preferences::default();
        if let Some(path) = fixture {
            prefs.left = path.clone();
            prefs.right = path;
        }
        let colors = crate::appearance::apply(&prefs.appearance, window, cx);
        let settings_inputs = [
            "",
            "",
            &prefs.appearance.font_family,
            &prefs.appearance.font_size.to_string(),
        ]
        .into_iter()
        .map(|value| cx.new(|cx| InputState::new(window, cx).default_value(value.to_string())))
        .collect::<Vec<_>>();
        let panes = [prefs.left.clone(), prefs.right.clone()]
            .into_iter()
            .enumerate()
            .map(|(i, path)| Pane {
                tabs: vec![{
                    let mut tab = Tab::new(i as u64, Location::Local(path.clone()));
                    tab.loading = true;
                    tab
                }],
                active: 0,
                recent_file_tab: Some(i as u64),
                recent_terminal_tab: None,
                focus: cx.focus_handle(),
                subscription: None,
                scroll: ScrollHandle::new(),
                path_input: cx.new(|cx| {
                    InputState::new(window, cx).default_value(path.to_string_lossy().into_owned())
                }),
            })
            .collect::<Vec<_>>();
        let panes = PaneStore::new(panes);
        panes[0].focus.focus(window, cx);
        let palette_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search commands…"));
        let split_state = cx.new(|_| ResizableState::default());
        let split_subscription = Self::observe_split(&split_state, cx);
        let mut subscriptions = vec![];
        for (index, input) in settings_inputs.iter().enumerate() {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                move |this, _, event, window, cx| {
                    this.settings_input_event(index, event, window, cx)
                },
            ));
        }
        subscriptions.push(cx.subscribe_in(
            &palette_input,
            window,
            |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    if let Some((_, _, command)) = this.filtered_commands(cx).first() {
                        this.command(*command, window, cx);
                        this.palette = false;
                        if this.operation_dialog.is_none()
                            && this.connection_screen.is_none()
                            && !this.settings_open
                            && !matches!(
                                command,
                                Command::EditPath
                                    | Command::Terminal
                                    | Command::FocusTerminal
                                    | Command::NewTerminal
                            )
                        {
                            this.activate(this.active, window, cx);
                        }
                    }
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &operation_input,
            window,
            |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.submit_operation(window, cx);
                }
            },
        ));
        cx.defer_in(window, move |this, window, cx| {
            this._subscriptions
                .push(cx.observe_window_appearance(window, |this, window, cx| {
                    this.refresh_system_appearance(window, cx);
                }));
            cx.spawn_in(window, async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(250))
                        .await;
                    if this
                        .update_in(cx, |this, window, cx| {
                            this.refresh_system_appearance(window, cx)
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
            if isolated {
                for id in this.panes.ids() {
                    this.request_listing(id, cx);
                }
            } else {
                this.load_preferences(window, cx);
                this.load_connections(cx);
            }
        });
        let mut workspace = Self {
            panes,
            colors,
            settings_open: false,
            settings_inputs,
            settings_scroll: ScrollHandle::new(),
            connections: vec![],
            connection_screen: None,
            connection_inputs,
            connection_busy: false,
            connection_cancel: CancellationToken::new(),
            connection_generation: 0,
            connection_scroll: ScrollHandle::new(),
            layout: Layout::Split {
                id: 0,
                axis: Axis::Right,
                state: split_state,
                _subscription: split_subscription,
                children: [Box::new(Layout::Leaf(0)), Box::new(Layout::Leaf(1))],
            },
            next_split_id: 1,
            recent_pane: Some(1),
            active: 0,
            preferences: prefs,
            notice: None,
            palette: false,
            palette_input,
            _subscriptions: subscriptions,
            next_id: 2,
            saving: false,
            preferences_writable: false,
            preferences_loaded: isolated,
            save_pending: false,
            transfers: TransferManager::with_registry(registry.clone()),
            registry,
            jobs: vec![],
            transfer_drawer: false,
            operation_dialog: None,
            operation_input,
            operation_focus: cx.focus_handle(),
        };
        for id in 0..2 {
            workspace.subscribe_pane(id, window, cx);
        }
        workspace.start_transfer_poll(cx);
        workspace
    }
    fn load_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let load = cx
            .background_executor()
            .spawn(async { persistence::load() });
        cx.spawn_in(window, async move |this, cx| {
            let (prefs, notice) = load.await;
            if let Err(error) = this.update_in(cx, |this, window, cx| {
                this.preferences = prefs;
                this.colors = crate::appearance::apply(&this.preferences.appearance, window, cx);
                this.sync_appearance_inputs(window, cx);
                this.preferences_loaded = true;
                this.preferences_writable = notice.is_none();
                this.notice = notice;
                for (side, i) in this.root_panes().into_iter().enumerate() {
                    let path = if side == 0 {
                        this.preferences.left.clone()
                    } else {
                        this.preferences.right.clone()
                    };
                    this.panes[i].tabs[0] = Tab::new(i as u64, Location::Local(path));
                    this.sync_path(i, window, cx);
                    this.request_listing(i, cx);
                }
                cx.notify();
            }) {
                eprintln!("Unable to apply workspace startup preferences: {error}");
            }
        })
        .detach();
    }
    fn activate(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.panes.contains(i) {
            return;
        }
        if self.active != i && self.panes.contains(self.active) {
            self.recent_pane = Some(self.active);
        }
        self.active = i;
        let tab = self.panes[i].tab();
        let tab_id = tab.id;
        if let Some(terminal) = tab.terminal.clone() {
            self.panes[i].recent_terminal_tab = Some(tab_id);
            terminal.read(cx).focus_handle(cx).focus(window, cx);
        } else {
            self.panes[i].recent_file_tab = Some(tab_id);
            self.panes[i].focus.focus(window, cx);
        }
        cx.notify();
    }
    fn sync_path(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.panes[i].tab().path.display();
        self.panes[i]
            .path_input
            .update(cx, |input, cx| input.set_value(path, window, cx));
    }
    fn request_listing(&mut self, i: usize, cx: &mut Context<Self>) {
        if !self.panes.contains(i) {
            return;
        }
        if self.panes[i].tab().terminal.is_some() {
            return;
        }
        let registry = self.registry.clone();
        let show_hidden = self.preferences.show_hidden;
        let tab = self.panes[i].tab_mut();
        tab.cancel.cancel();
        tab.cancel = CancellationToken::new();
        tab.generation += 1;
        tab.loading = true;
        tab.error = None;
        tab.entries.clear();
        tab.selected.clear();
        tab.anchor = None;
        tab.selection_cursor = None;
        let id = tab.id;
        let generation = tab.generation;
        let request = registry.list(
            tab.path.clone(),
            ListOptions { show_hidden },
            tab.cancel.clone(),
        );
        let task = cx.background_executor().spawn(request);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let Some(pane) = this.panes.get_mut(i) else {
                    return;
                };
                let Some(tab) = pane
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id == id && tab.generation == generation)
                else {
                    return;
                };
                tab.loading = false;
                match result {
                    Ok(entries) => {
                        tab.entries = entries;
                        tab.sort_entries()
                    }
                    Err(error) => tab.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn navigate(
        &mut self,
        i: usize,
        path: Location,
        record: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.preferences_loaded || !self.panes.contains(i) {
            return;
        }
        if self.panes[i].tab().terminal.is_some() {
            self.ensure_file_tab(i, window, cx);
        }
        let tab = self.panes[i].tab_mut();
        if record && tab.path != path {
            tab.history.truncate(tab.cursor + 1);
            tab.history.push(path.clone());
            tab.cursor += 1;
        }
        tab.path = path;
        self.sync_path(i, window, cx);
        self.request_listing(i, cx);
        self.persist(cx);
    }
    fn history(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let i = self.active;
        if self.panes[i].tab().terminal.is_some() {
            return;
        }
        let tab = self.panes[i].tab_mut();
        let cursor = tab.cursor as isize + delta;
        if cursor < 0 || cursor >= tab.history.len() as isize {
            return;
        }
        tab.cursor = cursor as usize;
        let path = tab.history[tab.cursor].clone();
        self.navigate(i, path, false, window, cx);
    }
    fn persist(&mut self, cx: &mut Context<Self>) {
        if !self.preferences_writable {
            return;
        }
        let roots = self.root_panes();
        if let Some(Location::Local(path)) = self.panes[roots[0]].file_tab().map(|tab| &tab.path) {
            self.preferences.left = path.clone();
        }
        if let Some(Location::Local(path)) = self.panes[roots[1]].file_tab().map(|tab| &tab.path) {
            self.preferences.right = path.clone();
        }
        if self.saving {
            self.save_pending = true;
            return;
        }
        self.saving = true;
        let prefs = self.preferences.clone();
        let task = cx
            .background_executor()
            .spawn(async move { persistence::save(&prefs) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(()) => {
                        if this.preferences_writable {
                            this.notice = None;
                        }
                    }
                    Err(error) => this.notice = Some(error),
                }
                if this.save_pending {
                    this.save_pending = false;
                    this.persist(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn filtered_commands(&self, cx: &App) -> Vec<(String, &'static str, Command)> {
        let query = self.palette_input.read(cx).value().to_lowercase();
        let mut commands = COMMANDS
            .iter()
            .map(|(label, shortcut, command)| (label.to_string(), *shortcut, *command))
            .collect::<Vec<_>>();
        for (index, record) in self.connections.iter().enumerate() {
            for (action, label) in [
                (0, "Connect"),
                (1, "Edit"),
                (2, "Test"),
                (3, "Remove"),
                (4, "Reset host trust for"),
            ] {
                if action == 4 && record.protocol != Protocol::Sftp {
                    continue;
                }
                commands.push((
                    format!("{label} {}", record.name),
                    "",
                    Command::SavedConnection(index, action),
                ));
            }
        }
        commands
            .into_iter()
            .filter(|(name, _, _)| name.to_lowercase().contains(&query))
            .collect()
    }
    fn command(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        if !self.preferences_loaded {
            return;
        }
        let i = self.active;
        if self.panes[i].tab().terminal.is_some()
            && matches!(
                command,
                Command::Back
                    | Command::Forward
                    | Command::Parent
                    | Command::Refresh
                    | Command::EditPath
                    | Command::ChooseFolder
                    | Command::Open
                    | Command::SelectAll
                    | Command::Operation(_)
                    | Command::Favorite
                    | Command::RemoveFavorite
            )
        {
            self.notice = Some("Select a file tab to use file actions. Focus files · ⌘⌥F".into());
            cx.notify();
            return;
        }
        match command {
            Command::Settings => self.open_settings(window, cx),
            Command::AppearanceMode(mode) => {
                self.preferences.appearance.mode = mode;
                self.preview_appearance(window, cx);
            }
            Command::LightTheme(theme) => {
                self.preferences.appearance.light_theme = theme;
                self.preview_appearance(window, cx);
            }
            Command::DarkTheme(theme) => {
                self.preferences.appearance.dark_theme = theme;
                self.preview_appearance(window, cx);
            }
            Command::Density(density) => {
                self.preferences.appearance.row_density = density;
                self.preview_appearance(window, cx);
            }
            Command::ResetAppearance => {
                self.preferences.appearance = AppearanceSettings::default();
                self.sync_appearance_inputs(window, cx);
                self.preview_appearance(window, cx);
            }
            Command::SaveConnection => self.save_connection(window, cx),
            Command::TestDraftConnection => self.test_draft(window, cx),
            Command::ConfirmConnection => self.confirm_connection_action(window, cx),
            Command::ConnectionProtocol(protocol) => self.update_protocol(protocol, window, cx),
            Command::SavedConnection(index, action) => {
                self.saved_connection_command(index, action, window, cx)
            }
            Command::Connections => {
                self.connection_screen = Some(ConnectionScreen::List);
                self.palette = false;
            }
            Command::NewConnection => self.edit_connection(None, window, cx),
            Command::ImportForkLift => self.begin_import(window, cx),
            Command::TestConnection
            | Command::EditConnection
            | Command::RemoveConnection
            | Command::ResetHost => self.active_connection_command(command, window, cx),
            Command::ResizeLeft(delta) => {
                if let Some(width) = self
                    .layout
                    .nearest(i)
                    .and_then(|state| state.read(cx).sizes().first().copied())
                {
                    self.layout.nearest(i).unwrap().update(cx, |state, cx| {
                        state.resize_panel(0, width + px(delta), window, cx)
                    });
                }
            }
            Command::Conflict(policy) => self.decide_conflict(policy, window, cx),
            Command::CancelTransfer => self.cancel_transfer(cx),
            Command::Operation(operation) => self.begin_operation(operation, window, cx),
            Command::Transfers => self.transfer_drawer = !self.transfer_drawer,
            Command::Terminal => {
                if self.panes[i].tab().terminal.is_some() {
                    self.focus_files(window, cx);
                } else {
                    self.show_terminal(false, window, cx);
                }
            }
            Command::FocusTerminal => self.show_terminal(false, window, cx),
            Command::NewTerminal => self.show_terminal(true, window, cx),
            Command::EndTerminal => self.end_terminal(window, cx),
            Command::FocusFiles => self.focus_files(window, cx),
            Command::EditPath => self.panes[i]
                .path_input
                .update(cx, |input, cx| input.focus(window, cx)),
            Command::ChooseFolder => self.choose_folder(window, cx),
            Command::Open => self.open_selection(window, cx),
            Command::SelectAll => {
                let tab = self.panes[i].tab_mut();
                tab.selected = (0..tab.entries.len()).collect();
            }
            Command::Switch => self.focus_pane(1, window, cx),
            Command::PreviousPane => self.focus_pane(-1, window, cx),
            Command::Split(axis) => self.split_pane(axis, window, cx),
            Command::SplitTerminal(axis) => self.split_terminal(axis, window, cx),
            Command::ClosePane => self.close_pane(window, cx),
            Command::Sidebar => {
                self.preferences.sidebar_visible = !self.preferences.sidebar_visible;
                self.persist(cx)
            }
            Command::Back => self.history(-1, window, cx),
            Command::Forward => self.history(1, window, cx),
            Command::Parent => {
                if let Some(path) = self.panes[i].tab().path.parent() {
                    self.navigate(i, path, true, window, cx)
                }
            }
            Command::Refresh => self.request_listing(i, cx),
            Command::NewTab => {
                let path = self.panes[i].tab().path.clone();
                self.panes[i].tabs.push(Tab::new(self.next_id, path));
                self.next_id += 1;
                self.panes[i].active = self.panes[i].tabs.len() - 1;
                self.sync_path(i, window, cx);
                self.activate(i, window, cx);
                self.request_listing(i, cx)
            }
            Command::CloseTab => {
                if self.panes[i].tabs.len() > 1 {
                    let active = self.panes[i].active;
                    self.panes[i].tabs[active].cancel.cancel();
                    self.panes[i].tabs.remove(active);
                    self.panes[i].active = active.min(self.panes[i].tabs.len() - 1);
                    self.sync_path(i, window, cx);
                    self.activate(i, window, cx);
                    self.persist(cx)
                }
            }
            Command::NextTab | Command::PreviousTab => {
                let n = self.panes[i].tabs.len();
                self.panes[i].active = (self.panes[i].active
                    + if matches!(command, Command::NextTab) {
                        1
                    } else {
                        n - 1
                    })
                    % n;
                self.sync_path(i, window, cx);
                self.activate(i, window, cx);
                self.persist(cx)
            }
            Command::MoveTabLeft | Command::MoveTabRight => {
                let old = self.panes[i].active;
                let next = if matches!(command, Command::MoveTabLeft) {
                    old.checked_sub(1)
                } else {
                    (old + 1 < self.panes[i].tabs.len()).then_some(old + 1)
                };
                if let Some(next) = next {
                    self.panes[i].tabs.swap(old, next);
                    self.panes[i].active = next;
                }
            }
            Command::Hidden => {
                self.preferences.show_hidden = !self.preferences.show_hidden;
                for p in self.panes.ids() {
                    let active = self.panes[p].active;
                    for t in 0..self.panes[p].tabs.len() {
                        self.panes[p].active = t;
                        self.request_listing(p, cx);
                    }
                    self.panes[p].active = active;
                }
                self.persist(cx)
            }
            Command::RemoveFavorite => {
                let Location::Local(path) = &self.panes[i].tab().path else {
                    self.notice=Some("Remote favorites are not persisted yet. Use saved connections in the sidebar.".into());
                    return;
                };
                let path = path.clone();
                self.preferences
                    .favorites
                    .retain(|favorite| favorite != &path);
                self.persist(cx);
            }
            Command::Favorite => {
                let Location::Local(path) = &self.panes[i].tab().path else {
                    self.notice=Some("Remote favorites are not persisted yet. Use saved connections in the sidebar.".into());
                    return;
                };
                let path = path.clone();
                if !self.preferences.favorites.contains(&path) {
                    self.preferences.favorites.push(path);
                    self.persist(cx)
                }
            }
        }
        cx.notify();
    }
    fn select(&mut self, i: usize, row: usize, command: bool, shift: bool, cx: &mut Context<Self>) {
        if !self.panes.contains(i) {
            return;
        }
        let tab = self.panes[i].tab_mut();
        if shift {
            let anchor = tab.anchor.unwrap_or(row);
            tab.selected = (anchor.min(row)..=anchor.max(row)).collect()
        } else if command {
            if !tab.selected.insert(row) {
                tab.selected.remove(&row);
            }
            tab.anchor = Some(row)
        } else {
            tab.selected.clear();
            tab.selected.insert(row);
            tab.anchor = Some(row)
        }
        tab.selection_cursor = Some(row);
        self.panes[i].scroll.scroll_to_item(row);
        cx.notify();
    }
    fn step_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let tab = self.panes[self.active].tab();
        if tab.entries.is_empty() {
            return;
        }
        let current = tab
            .selection_cursor
            .unwrap_or(if delta > 0 { usize::MAX } else { 0 });
        let row = if current == usize::MAX {
            0
        } else {
            (current as isize + delta).clamp(0, tab.entries.len() as isize - 1) as usize
        };
        self.select(self.active, row, false, false, cx);
    }
    fn extend_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let i = self.active;
        let tab = self.panes[i].tab_mut();
        if tab.entries.is_empty() {
            return;
        }
        let anchor = tab.anchor.unwrap_or(0);
        let edge = tab.selection_cursor.unwrap_or(anchor);
        let row = (edge as isize + delta).clamp(0, tab.entries.len() as isize - 1) as usize;
        tab.anchor = Some(anchor);
        tab.selection_cursor = Some(row);
        tab.selected = (anchor.min(row)..=anchor.max(row)).collect();
        self.panes[i].scroll.scroll_to_item(row);
        cx.notify();
    }
    fn choose_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pane_index = self.active;
        let tab_id = self.panes[pane_index].tab().id;
        if !matches!(self.panes[pane_index].tab().path, Location::Local(_)) {
            self.notice = Some("Choose Folder is available only for local panes.".into());
            cx.notify();
            return;
        }

        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose Folder".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = paths.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            let same_tab = this.panes.contains(pane_index) && this.panes[pane_index].tab().id == tab_id;
                            if same_tab
                                && matches!(this.panes[pane_index].tab().path, Location::Local(_))
                            {
                                this.navigate(
                                    pane_index,
                                    Location::Local(path),
                                    true,
                                    window,
                                    cx,
                                );
                            } else {
                                this.notice = Some(
                                    "The pane changed while the folder picker was open. Choose the folder again.".into(),
                                );
                            }
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => {
                        this.notice = Some(format!("Could not open folder picker: {error}"));
                    }
                    Err(_) => {
                        this.notice = Some("The folder picker closed without returning a result.".into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab = self.panes[self.active].tab();
        if let Some(entry) = tab.selection_cursor.and_then(|row| tab.entries.get(row)) {
            if entry.kind == EntryKind::Directory {
                self.navigate(self.active, entry.location.clone(), true, window, cx)
            } else {
                let location = entry.location.clone();
                self.open_file(location, cx);
            }
        }
    }
    fn open_file(&mut self, location: Location, cx: &mut Context<Self>) {
        if !location.is_local() {
            self.notice = Some(
                "Copy this remote file to a local pane with F5, then open the local copy.".into(),
            );
            cx.notify();
            return;
        }
        let task = cx
            .background_executor()
            .spawn(async move { crate::platform::open_in_default_app(&location) });
        cx.spawn(async move |this, cx| {
            if let Err(error) = task.await {
                let _ = this.update(cx, |this, cx| {
                    this.notice = Some(error);
                    cx.notify();
                });
            }
        })
        .detach();
    }
    fn render_pane(&self, i: usize, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.colors;
        let font_size = self.preferences.appearance.font_size;
        let row_height = self
            .preferences
            .appearance
            .row_density
            .row_height(font_size);
        let pane = &self.panes[i];
        let fallback = (window.viewport_size().width
            - px(if self.preferences.sidebar_visible {
                160.
            } else {
                0.
            })
            - px(1.))
            / 2.;
        let width = self.layout.width(i, fallback * 2., cx);
        let scale = font_size / 13.;
        let show_kind = width >= px(320. * scale);
        let show_size = width >= px(230. * scale);
        let show_modified = width >= px(430. * scale);
        let tab = pane.tab();
        let pane_tab_id = tab.id;
        let file_tab = tab.terminal.is_none();
        let active = self.active == i;
        let mut content = div()
            .id(("listing", i))
            .track_focus(&pane.focus)
            .key_context("Listing")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&pane.scroll)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| this.activate(i, window, cx)),
            );
        if tab.loading {
            content = content.child(
                div()
                    .p_4()
                    .text_color(rgb(theme.muted))
                    .child("Loading directory…"),
            )
        } else if let Some(error) = &tab.error {
            content = content.child(
                div()
                    .p_4()
                    .text_color(rgb(theme.error))
                    .child(error.clone())
                    .child(" · ⌘R Retry"),
            )
        } else if tab.entries.is_empty() {
            content = content.child(div().p_4().text_color(rgb(theme.muted)).child(
                if matches!(tab.path, Location::S3 { .. }) {
                    "This prefix has no objects"
                } else {
                    "This folder is empty"
                },
            ))
        } else {
            content = content.children(tab.entries.iter().enumerate().map(|(row, entry)| {
                let name = entry.name.to_string_lossy().into_owned();
                let selected = tab.selected.contains(&row);
                let path = entry.location.clone();
                let dragged_entries: Vec<&Entry> = if selected {
                    tab.selected
                        .iter()
                        .filter_map(|selected_row| tab.entries.get(*selected_row))
                        .collect()
                } else {
                    vec![entry]
                };
                let sources = dragged_entries
                    .iter()
                    .map(|entry| entry.location.clone())
                    .collect();
                let external_paths = dragged_entries
                    .iter()
                    .map(|entry| match &entry.location {
                        Location::Local(path) => {
                            Some((path.clone(), entry.kind == EntryKind::Directory))
                        }
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>();
                let drag = PaneDrag {
                    source_pane: i,
                    source_tab_id: tab.id,
                    sources,
                    external_paths,
                };
                let is_dir = entry.kind == EntryKind::Directory;
                div()
                    .id(format!("entry-{i}-{row}"))
                    .h(px(row_height))
                    .whitespace_nowrap()
                    .flex()
                    .items_center()
                    .px_3()
                    .gap_2()
                    .bg(rgb(if selected {
                        theme.selection
                    } else {
                        theme.background
                    }))
                    .hover(|style| {
                        style.bg(rgb(if selected {
                            theme.selection_hover
                        } else {
                            theme.hover
                        }))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            if !this.panes.contains(i)
                                || this.panes[i].tab().id != pane_tab_id
                                || this.panes[i]
                                    .tab()
                                    .entries
                                    .get(row)
                                    .is_none_or(|entry| entry.location != path)
                            {
                                return;
                            }
                            this.activate(i, window, cx);
                            if !selected || event.modifiers.platform || event.modifiers.shift {
                                this.select(
                                    i,
                                    row,
                                    event.modifiers.platform,
                                    event.modifiers.shift,
                                    cx,
                                );
                            } else {
                                if !this.panes.contains(i) {
                                    return;
                                }
                                this.panes[i].tab_mut().selection_cursor = Some(row);
                            }
                            if event.click_count == 2 {
                                if is_dir {
                                    this.navigate(i, path.clone(), true, window, cx);
                                } else {
                                    this.open_file(path.clone(), cx);
                                }
                            }
                        }),
                    )
                    .on_drag(drag, |drag, _, _, cx| {
                        let label = if drag.sources.len() == 1 {
                            drag.sources[0].label()
                        } else {
                            format!("{} items", drag.sources.len())
                        };
                        cx.new(|_| PaneDragPreview { label })
                    })
                    .external_drag_payload(|drag: &PaneDrag, _, _| {
                        let paths = drag.external_paths.as_ref()?;
                        (!paths.is_empty())
                            .then(|| ExternalDragPayload::Files(FileDragPaths::new(paths.clone())))
                    })
                    .child(
                        div()
                            .w(px(12.))
                            .text_color(rgb(if selected { theme.text } else { theme.muted }))
                            .child(if is_dir { "▸" } else { "·" }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(name),
                    )
                    .when(show_kind, |row| {
                        row.child(
                            div()
                                .flex_none()
                                .w(px(44. * scale))
                                .text_color(rgb(if selected { theme.text } else { theme.muted }))
                                .child(
                                    if matches!(entry.location, Location::S3 { prefix: true, .. }) {
                                        "Prefix"
                                    } else {
                                        kind(entry.kind)
                                    },
                                ),
                        )
                    })
                    .when(show_size, |row| {
                        row.child(
                            div()
                                .flex_none()
                                .w(px(55. * scale))
                                .text_color(rgb(if selected { theme.text } else { theme.muted }))
                                .child(bytes(entry.size)),
                        )
                    })
                    .when(show_modified, |row| {
                        row.child(
                            div()
                                .flex_none()
                                .w(px(85. * scale))
                                .text_color(rgb(if selected { theme.text } else { theme.muted }))
                                .child(modified(entry.modified)),
                        )
                    })
            }));
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .bg(rgb(theme.background))
            .can_drop(move |payload, _, _| {
                file_tab
                    && (payload
                        .downcast_ref::<PaneDrag>()
                        .is_some_and(|drag| drag.source_pane != i && !drag.sources.is_empty())
                        || payload
                            .downcast_ref::<ExternalPaths>()
                            .is_some_and(|paths| !paths.paths().is_empty()))
            })
            .drag_over::<PaneDrag>(move |style, _, _, _| {
                style.border_2().border_color(rgb(theme.accent))
            })
            .drag_over::<ExternalPaths>(move |style, _, _, _| {
                style.border_2().border_color(rgb(theme.accent))
            })
            .on_drop(cx.listener(move |this, drag: &PaneDrag, _, cx| {
                this.drop_copy(drag.clone(), i, pane_tab_id, cx);
            }))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                this.drop_external_copy(paths.clone(), i, pane_tab_id, cx);
            }))
            .child(
                div()
                    .id(("pane-tabs", i))
                    .overflow_x_scroll()
                    .h(px(font_size + 19.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .bg(rgb(theme.surface))
                    .border_b_1()
                    .border_color(rgb(if active { theme.accent } else { theme.border }))
                    .children(pane.tabs.iter().enumerate().map(|(t, tab)| {
                        let is_active = t == pane.active;
                        let path_label = if tab.terminal.is_some() {
                            format!("Terminal · {}", tab.path.label())
                        } else {
                            tab.path.label()
                        };
                        let tab_id = tab.id;
                        let tab_group = format!("pane-{i}-tab-{t}");
                        div()
                            .group(tab_group.clone())
                            .flex_none()
                            .h_full()
                            .flex()
                            .items_center()
                            .bg(rgb(if is_active {
                                theme.background
                            } else {
                                theme.surface
                            }))
                            .child(
                                div()
                                    .id(format!("tab-{i}-{t}"))
                                    .max_w(px(160. * scale))
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .px_3()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgb(theme.hover)))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if !this.panes.contains(i)
                                            || this.panes[i]
                                                .tabs
                                                .get(t)
                                                .is_none_or(|tab| tab.id != tab_id)
                                        {
                                            return;
                                        }
                                        this.panes[i].active = t;
                                        this.activate(i, window, cx);
                                        this.sync_path(i, window, cx);
                                        this.persist(cx);
                                    }))
                                    .child(path_label.clone()),
                            )
                            .child(
                                Button::new(format!("close-tab-{i}-{t}"))
                                    .ghost()
                                    .compact()
                                    .w(px(22.))
                                    .opacity(0.)
                                    .group_hover(tab_group, |style| style.opacity(1.))
                                    .focus_visible(|style| style.opacity(1.))
                                    .icon(Icon::new(IconName::Close))
                                    .accessibility_label(format!("Close tab {path_label}"))
                                    .tooltip(if is_active {
                                        "Close tab · ⌘W"
                                    } else {
                                        "Select and close tab · ⌘W"
                                    })
                                    .disabled(pane.tabs.len() <= 1)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if this.panes.contains(i)
                                            && this.panes[i]
                                                .tabs
                                                .get(t)
                                                .is_some_and(|tab| tab.id == tab_id)
                                            && this.panes[i].tabs.len() > 1
                                        {
                                            if !this.panes.contains(i)
                                                || this.panes[i]
                                                    .tabs
                                                    .get(t)
                                                    .is_none_or(|tab| tab.id != tab_id)
                                            {
                                                return;
                                            }
                                            this.panes[i].active = t;
                                            this.activate(i, window, cx);
                                            this.command(Command::CloseTab, window, cx);
                                        }
                                    })),
                            )
                    }))
                    .child(
                        Button::new(("new-tab", i))
                            .secondary()
                            .compact()
                            .label("+")
                            .accessibility_label("New tab")
                            .tooltip("New tab · ⌘T")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if !this.panes.contains(i) || this.panes[i].tab().id != pane_tab_id
                                {
                                    return;
                                }
                                this.activate(i, window, cx);
                                this.command(Command::NewTab, window, cx);
                            })),
                    ),
            )
            .when(file_tab, |panel| {
                panel
                    .child(
                        div()
                            .h(px(font_size + 22.))
                            .flex_none()
                            .px_2()
                            .py_1()
                            .child(Input::new(&pane.path_input)),
                    )
                    .child(
                        div()
                            .h(px(font_size + 14.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .px_3()
                            .gap_2()
                            .text_color(rgb(theme.muted))
                            .border_b_1()
                            .border_color(rgb(theme.border))
                            .child(div().w(px(12.)))
                            .children(
                                [
                                    (Sort::Name, "Name", None),
                                    (Sort::Kind, "Kind", Some(44. * scale)),
                                    (Sort::Size, "Size", Some(55. * scale)),
                                    (Sort::Modified, "Modified", Some(85. * scale)),
                                ]
                                .into_iter()
                                .enumerate()
                                .filter(|(_, (sort, _, _))| match sort {
                                    Sort::Name => true,
                                    Sort::Kind => show_kind,
                                    Sort::Size => show_size,
                                    Sort::Modified => show_modified,
                                })
                                .map(
                                    |(col, (sort, label, width))| {
                                        div()
                                            .id(format!("sort-{i}-{col}"))
                                            .when_some(width, |d, w| d.flex_none().w(px(w)))
                                            .when(width.is_none(), |d| d.flex_1())
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if !this.panes.contains(i)
                                                    || this.panes[i].tab().id != pane_tab_id
                                                {
                                                    return;
                                                }
                                                let tab = this.panes[i].tab_mut();
                                                if std::mem::discriminant(&tab.sort)
                                                    == std::mem::discriminant(&sort)
                                                {
                                                    tab.descending = !tab.descending
                                                } else {
                                                    tab.sort = sort;
                                                    tab.descending = false
                                                }
                                                tab.sort_entries();
                                                cx.notify();
                                            }))
                                            .child(label)
                                    },
                                ),
                            ),
                    )
            })
            .when_some(tab.terminal.clone(), |pane, terminal| {
                terminal.update(cx, |view, cx| view.set_theme(theme, font_size, cx));
                pane.child(
                    div()
                        .flex_none()
                        .px_2()
                        .py_1()
                        .flex()
                        .items_center()
                        .gap_2()
                        .border_b_1()
                        .border_color(rgb(theme.border))
                        .bg(rgb(theme.surface))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .text_color(rgb(theme.muted))
                                .child(format!(
                                    "{} · {}",
                                    tab.terminal_label,
                                    terminal.read(cx).status()
                                )),
                        )
                        .child(
                            Button::new(format!("terminal-files-{i}"))
                                .ghost()
                                .compact()
                                .label("Files")
                                .tooltip("Focus file tab · ⌘⌥F")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if this.panes.contains(i)
                                        && this.panes[i].tab().id == pane_tab_id
                                    {
                                        this.activate(i, window, cx);
                                        this.focus_files(window, cx);
                                    }
                                })),
                        )
                        .child(
                            Button::new(format!("terminal-end-{i}"))
                                .ghost()
                                .compact()
                                .label("End")
                                .tooltip("End terminal tab; stops running commands · ⌘⌥K")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if this.panes.contains(i)
                                        && this.panes[i].tab().id == pane_tab_id
                                    {
                                        this.activate(i, window, cx);
                                        this.end_terminal(window, cx);
                                    }
                                })),
                        ),
                )
                .child(div().flex_1().min_h_0().child(terminal).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        if this.panes.contains(i) && this.panes[i].tab().id == pane_tab_id {
                            this.activate(i, window, cx);
                        }
                    }),
                ))
            })
            .when(file_tab, |pane| pane.child(content))
    }
}
fn kind(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Directory => "Folder",
        EntryKind::File => "File",
        EntryKind::Symlink => "Link",
        EntryKind::Other => "Other",
    }
}
fn bytes(size: Option<u64>) -> String {
    match size {
        None => "—".into(),
        Some(n) if n >= 1_000_000_000 => format!("{:.1} GB", n as f64 / 1e9),
        Some(n) if n >= 1_000_000 => format!("{:.1} MB", n as f64 / 1e6),
        Some(n) if n >= 1_000 => format!("{:.1} KB", n as f64 / 1e3),
        Some(n) => format!("{n} B"),
    }
}
fn modified(time: Option<SystemTime>) -> String {
    let Some(seconds) = time
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
    else {
        return "—".into();
    };
    let days = seconds / 86400;
    let z = days as i64 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    format!(
        "{:04}-{:02}-{:02}",
        y + if month <= 2 { 1 } else { 0 },
        month,
        day
    )
}
impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.colors;
        let font_size = self.preferences.appearance.font_size;
        let compact_toolbar = window.viewport_size().width < px(900. * font_size / 13.);
        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme.background))
            .text_color(rgb(theme.text))
            .font_family(self.preferences.appearance.font_family.clone())
            .text_size(px(font_size))
            .key_context("Workspace");
        macro_rules! action {
            ($ty:ty,$command:expr) => {
                root = root.on_action(
                    cx.listener(|this, _: &$ty, window, cx| this.command($command, window, cx)),
                );
            };
        }
        action!(Settings, Command::Settings);
        action!(ChooseFolder, Command::ChooseFolder);
        root = root
            .on_action(cx.listener(|this, _: &NextSettingsField, window, cx| {
                this.move_settings_focus(1, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PreviousSettingsField, window, cx| {
                this.move_settings_focus(-1, window, cx)
            }));
        action!(ManageConnections, Command::Connections);
        action!(NewConnection, Command::NewConnection);
        root = root
            .on_action(cx.listener(|this, _: &NextConnectionField, window, cx| {
                this.move_connection_focus(1, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &PreviousConnectionField, window, cx| {
                    this.move_connection_focus(-1, window, cx)
                }),
            );
        action!(SwitchPane, Command::Switch);
        action!(PreviousPane, Command::PreviousPane);
        action!(SplitRight, Command::Split(Axis::Right));
        action!(SplitDown, Command::Split(Axis::Down));
        action!(SplitTerminalRight, Command::SplitTerminal(Axis::Right));
        action!(SplitTerminalDown, Command::SplitTerminal(Axis::Down));
        action!(ClosePane, Command::ClosePane);
        action!(ToggleSidebar, Command::Sidebar);
        action!(Back, Command::Back);
        action!(Forward, Command::Forward);
        action!(Parent, Command::Parent);
        action!(Refresh, Command::Refresh);
        action!(NewTab, Command::NewTab);
        action!(CloseTab, Command::CloseTab);
        action!(NextTab, Command::NextTab);
        action!(PreviousTab, Command::PreviousTab);
        action!(MoveTabLeft, Command::MoveTabLeft);
        action!(MoveTabRight, Command::MoveTabRight);
        action!(ToggleHidden, Command::Hidden);
        action!(AddFavorite, Command::Favorite);
        action!(RemoveFavorite, Command::RemoveFavorite);
        action!(GrowLeftPane, Command::ResizeLeft(40.));
        action!(ShrinkLeftPane, Command::ResizeLeft(-40.));
        root = root
            .on_action(
                cx.listener(|this, _: &ExtendSelectionNext, _, cx| this.extend_selection(1, cx)),
            )
            .on_action(cx.listener(|this, _: &ExtendSelectionPrevious, _, cx| {
                this.extend_selection(-1, cx)
            }));
        action!(CreateFolder, Command::Operation(Operation::CreateDirectory));
        action!(RenameItem, Command::Operation(Operation::Rename));
        action!(CopyItems, Command::Operation(Operation::Copy));
        action!(MoveItems, Command::Operation(Operation::Move));
        action!(TrashItems, Command::Operation(Operation::Trash));
        action!(ToggleTransfers, Command::Transfers);
        action!(ToggleTerminal, Command::Terminal);
        action!(FocusTerminal, Command::FocusTerminal);
        action!(NewTerminal, Command::NewTerminal);
        action!(ImportForkLift, Command::ImportForkLift);
        action!(EndTerminal, Command::EndTerminal);
        action!(FocusFiles, Command::FocusFiles);
        root =
            root.on_action(cx.listener(|this, _: &ConfirmOperation, window, cx| {
                this.submit_operation(window, cx)
            }));
        root = root
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.step_selection(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.step_selection(-1, cx)))
            .on_action(
                cx.listener(|this, _: &OpenSelection, window, cx| this.open_selection(window, cx)),
            )
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                let tab = this.panes[this.active].tab_mut();
                tab.selected = (0..tab.entries.len()).collect();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &EditPath, window, cx| {
                this.panes[this.active]
                    .path_input
                    .update(cx, |input, cx| input.focus(window, cx));
            }))
            .on_action(cx.listener(|this, _: &TogglePalette, window, cx| {
                this.palette = !this.palette;
                if this.palette {
                    this.palette_input.update(cx, |input, cx| {
                        input.set_value("", window, cx);
                        input.focus(window, cx);
                    });
                } else {
                    this.activate(this.active, window, cx)
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Escape, window, cx| {
                this.palette = false;
                this.settings_open = false;
                this.operation_dialog = None;
                this.close_connections(window, cx);
                this.notice = None;
                this.activate(this.active, window, cx);
            }));
        root = root.child(
            div()
                .h(px(font_size + 25.))
                .flex_none()
                .flex()
                .items_center()
                .px_3()
                .gap_4()
                .bg(rgb(theme.surface))
                .border_b_1()
                .border_color(rgb(theme.border))
                .children(
                    [
                        ("← ⌘[", Command::Back),
                        ("→ ⌘]", Command::Forward),
                        ("↑ ⌘↑", Command::Parent),
                        ("Refresh ⌘R", Command::Refresh),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(id, (label, command))| {
                        div()
                            .id(("toolbar", id))
                            .role(gpui_kit::accesskit::Role::Button)
                            .aria_label(label)
                            .focusable()
                            .px_1()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(|s| s.text_color(rgb(theme.accent)))
                            .active(|s| s.bg(rgb(theme.selection_hover)))
                            .focus_visible(|s| {
                                s.bg(rgb(theme.selection)).text_color(rgb(theme.accent))
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.command(command, window, cx)
                            }))
                            .child(if compact_toolbar {
                                match command {
                                    Command::Sidebar => "Sidebar",
                                    Command::Back => "←",
                                    Command::Forward => "→",
                                    Command::Parent => "↑",
                                    Command::Refresh => "Refresh",
                                    _ => label,
                                }
                            } else {
                                label
                            })
                    }),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("palette-button")
                        .role(gpui_kit::accesskit::Role::Button)
                        .aria_label("Open command palette")
                        .focusable()
                        .px_1()
                        .py_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .active(|s| s.bg(rgb(theme.selection_hover)))
                        .focus_visible(|s| s.bg(rgb(theme.selection)).text_color(rgb(theme.accent)))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.palette = true;
                            this.palette_input
                                .update(cx, |input, cx| input.focus(window, cx));
                            cx.notify();
                        }))
                        .child(if compact_toolbar {
                            "Commands"
                        } else {
                            "Commands ⌘⇧P"
                        }),
                ),
        );
        if self.palette {
            root = root.child(
                div()
                    .flex_none()
                    .p_3()
                    .bg(rgb(theme.surface))
                    .border_b_1()
                    .border_color(rgb(theme.border))
                    .child(Input::new(&self.palette_input))
                    .children(self.filtered_commands(cx).into_iter().enumerate().map(
                        |(id, (label, shortcut, command))| {
                            div()
                                .id(("command", id))
                                .h(px(font_size + 16.))
                                .flex()
                                .items_center()
                                .px_2()
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(theme.selection)))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.command(command, window, cx);
                                    this.palette = false;
                                    if this.operation_dialog.is_none()
                                        && this.connection_screen.is_none()
                                        && !this.settings_open
                                        && !matches!(
                                            command,
                                            Command::EditPath
                                                | Command::Terminal
                                                | Command::FocusTerminal
                                                | Command::NewTerminal
                                        )
                                    {
                                        this.activate(this.active, window, cx);
                                    }
                                }))
                                .child(div().flex_1().child(label))
                                .child(div().text_color(rgb(theme.text)).child(shortcut))
                        },
                    )),
            );
        }
        if self.settings_open {
            return root.child(self.render_settings(cx));
        }
        root = root.child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .when(self.preferences.sidebar_visible, |body| {
                    body.child(
                        div()
                            .w(px(160.))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .py_3()
                            .px_2()
                            .gap_2()
                            .bg(rgb(theme.sidebar))
                            .border_r_1()
                            .border_color(rgb(theme.border))
                            .child(
                                div()
                                    .px_1()
                                    .text_size(px((font_size - 1.).max(10.)))
                                    .text_color(rgb(theme.muted))
                                    .child("FAVORITES · ⌘D Add"),
                            )
                            .children(self.preferences.favorites.iter().enumerate().map(
                                |(id, path)| {
                                    let path = path.clone();
                                    let label = path
                                        .file_name()
                                        .unwrap_or(path.as_os_str())
                                        .to_string_lossy()
                                        .into_owned();
                                    div()
                                        .flex()
                                        .items_center()
                                        .child(
                                            div()
                                                .id(("favorite", id))
                                                .flex_1()
                                                .px_1()
                                                .cursor_pointer()
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.navigate(
                                                            this.active,
                                                            Location::Local(path.clone()),
                                                            true,
                                                            window,
                                                            cx,
                                                        )
                                                    },
                                                ))
                                                .child(label),
                                        )
                                        .child(
                                            div()
                                                .id(("remove-favorite", id))
                                                .px_1()
                                                .cursor_pointer()
                                                .text_color(rgb(theme.muted))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    if !this.preferences_loaded {
                                                        return;
                                                    }
                                                    this.preferences.favorites.remove(id);
                                                    this.persist(cx);
                                                    cx.notify();
                                                }))
                                                .child("×"),
                                        )
                                },
                            ))
                            .child(
                                div()
                                    .mt_4()
                                    .px_1()
                                    .text_size(px((font_size - 1.).max(10.)))
                                    .text_color(rgb(theme.muted))
                                    .child("LOCATIONS"),
                            )
                            .child(
                                div()
                                    .id("local-root")
                                    .px_1()
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.navigate(
                                            this.active,
                                            Location::Local(PathBuf::from("/")),
                                            true,
                                            window,
                                            cx,
                                        )
                                    }))
                                    .child("Local disk"),
                            )
                            .when(cfg!(target_os = "macos"), |sidebar| {
                                sidebar.child(
                                    Button::new("mounted-volumes")
                                        .secondary()
                                        .compact()
                                        .label("Mounted volumes")
                                        .accessibility_label("Browse mounted volume roots")
                                        .tooltip("Browse mounted macOS volumes at /Volumes")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.navigate(
                                                this.active,
                                                Location::Local(PathBuf::from("/Volumes")),
                                                true,
                                                window,
                                                cx,
                                            )
                                        })),
                                )
                            })
                            .child(
                                div()
                                    .mt_4()
                                    .text_size(px((font_size - 1.).max(10.)))
                                    .text_color(rgb(theme.muted))
                                    .child("CONNECTIONS"),
                            )
                            .children(self.connections.iter().enumerate().map(|(id, record)| {
                                let record = record.clone();
                                let label = record.name.clone();
                                div()
                                    .id(("connection-sidebar", id))
                                    .px_1()
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.connect_record(&record, window, cx)
                                    }))
                                    .child(label)
                            }))
                            .child(
                                div()
                                    .id("manage-connections")
                                    .px_1()
                                    .cursor_pointer()
                                    .text_color(rgb(theme.accent))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.connection_screen = Some(ConnectionScreen::List);
                                        cx.notify();
                                    }))
                                    .child("Manage · ⌘⇧C"),
                            ),
                    )
                })
                .child(div().flex_1().min_w_0().child(self.render_layout(
                    &self.layout,
                    window,
                    cx,
                ))),
        );
        if self.connection_screen.is_some() {
            root = root.child(self.render_connections(cx));
        }
        if self.transfer_drawer {
            root = root.child(self.render_transfers(cx));
        }
        if self.operation_dialog.is_some() {
            root = root.child(self.render_operation_dialog(cx));
        }
        if let Some(notice) = &self.notice {
            root = root.child(
                div()
                    .flex_none()
                    .px_3()
                    .py_1()
                    .text_color(rgb(theme.warning))
                    .child(notice.clone()),
            );
        }
        let tab = self.panes[self.active].tab();
        let selected_bytes = tab
            .selected
            .iter()
            .filter_map(|i| tab.entries.get(*i).and_then(|e| e.size))
            .sum();
        root.child(
            div()
                .h(px(font_size + 14.))
                .flex_none()
                .flex()
                .items_center()
                .px_3()
                .gap_3()
                .border_t_1()
                .border_color(rgb(theme.border))
                .text_color(rgb(theme.muted))
                .child(div().flex_1().overflow_hidden().child(format!(
                    "{} · {}",
                    if self.root_panes()[0] == self.active {
                        "Left"
                    } else if self.root_panes()[1] == self.active {
                        "Right"
                    } else {
                        "Split"
                    },
                    tab.path.display()
                )))
                .child(if let Some(terminal) = &tab.terminal {
                    terminal.read(cx).status()
                } else {
                    format!(
                        "{} items · {} selected · {}",
                        tab.entries.len(),
                        tab.selected.len(),
                        bytes(Some(selected_bytes))
                    )
                })
                .child(if self.preferences.show_hidden {
                    "Hidden shown"
                } else {
                    "Hidden off"
                })
                .child(format!("{} jobs", self.jobs.len()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            Button::new("status-sidebar")
                                .ghost()
                                .compact()
                                .icon(Icon::new(IconName::PanelLeft))
                                .selected(self.preferences.sidebar_visible)
                                .toggled(self.preferences.sidebar_visible)
                                .accessibility_label(if self.preferences.sidebar_visible {
                                    "Hide sidebar"
                                } else {
                                    "Show sidebar"
                                })
                                .tooltip(if self.preferences.sidebar_visible {
                                    "Hide sidebar · ⌘B"
                                } else {
                                    "Show sidebar · ⌘B"
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.command(Command::Sidebar, window, cx)
                                })),
                        )
                        .child(
                            Button::new("status-terminal")
                                .ghost()
                                .compact()
                                .icon(Icon::new(IconName::SquareTerminal))
                                .selected(tab.terminal.is_some())
                                .toggled(tab.terminal.is_some())
                                .accessibility_label(if tab.terminal.is_some() {
                                    "Switch to files; keep terminal running"
                                } else {
                                    "Focus terminal tab"
                                })
                                .tooltip("Toggle terminal · ⌃` · Focus terminal ⌘⌥J")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.command(Command::Terminal, window, cx)
                                })),
                        )
                        .child(
                            Button::new("status-transfers")
                                .ghost()
                                .compact()
                                .icon(Icon::new(IconName::PanelBottom))
                                .selected(self.transfer_drawer)
                                .toggled(self.transfer_drawer)
                                .accessibility_label(format!(
                                    "{} transfer queue, {} jobs",
                                    if self.transfer_drawer { "Hide" } else { "Show" },
                                    self.jobs.len()
                                ))
                                .tooltip(if self.transfer_drawer {
                                    "Hide transfer queue · ⌘J"
                                } else {
                                    "Show transfer queue · ⌘J"
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.command(Command::Transfers, window, cx)
                                })),
                        ),
                ),
        )
    }
}
