mod file_icons;
mod layout;
mod palette;
mod shortcuts;
mod sidebar;
mod tree;
mod vim;
pub use file_icons::AppAssets;
pub use palette::window_options;
use tree::TreeState;
mod terminal;
mod terminal_commands;
use terminal::TerminalView;
mod settings;
mod usage;
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
        Disableable, Icon, IconName, Selectable, Sizable,
        button::{Button, ButtonVariants},
        input::{Input, InputEvent, InputState},
        resizable::{ResizableState, h_resizable, resizable_panel, v_resizable},
    },
    prelude::*,
    *,
};
use operations::OperationDialog;
use std::{
    collections::{BTreeSet, HashSet, VecDeque},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
actions!(excavator_settings, [Settings, CheckUpdates]);
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
        ExpandRow,
        CollapseRow,
        PaletteNext,
        PalettePrevious,
        SidebarUp,
        SidebarDown,
        SidebarExpand,
        SidebarCollapse,
        SidebarOpen,
        FocusSidebar,
        OpenSelection,
        SelectAll,
        EditPath,
        ToggleHidden,
        TogglePalette,
        ToggleShortcuts,
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
pub(super) enum Sort {
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
    /// Visible rows: the root listing plus loaded children of expanded folders.
    entries: Vec<Entry>,
    depths: Vec<usize>,
    root_entries: Vec<Entry>,
    tree: TreeState,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    selection_cursor: Option<usize>,
    loading: bool,
    cached_listing: bool,
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

#[derive(Clone)]
struct TabDrag {
    source_pane: usize,
    tab_id: u64,
    label: String,
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
            depths: vec![],
            root_entries: vec![],
            tree: TreeState::default(),
            selected: BTreeSet::new(),
            anchor: None,
            selection_cursor: None,
            loading: false,
            cached_listing: false,
            error: None,
            generation: 0,
            cancel: CancellationToken::new(),
            sort: Sort::Name,
            descending: false,
        }
    }
    fn sort_entries(&mut self) {
        let (sort, descending) = (self.sort, self.descending);
        tree::sort_list(&mut self.root_entries, sort, descending);
        for children in self.tree.children.values_mut() {
            if let tree::Children::Loaded(entries) = children {
                tree::sort_list(entries, sort, descending);
            }
        }
        self.rebuild_rows();
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
    CheckUpdates,
    Usage,
    Shortcuts,
    ToggleVim,
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
    MoveTabNextPane,
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
    SidebarUp,
    SidebarDown,
    SidebarExpand,
    SidebarCollapse,
    SidebarOpen,
    FocusSidebar,
    AppearanceMode(AppearanceMode),
    LightTheme(LightTheme),
    DarkTheme(DarkTheme),
    Density(RowDensity),
    ResetAppearance,
}
const INDENT: f32 = 16.;
const COMMANDS: &[(&str, &str, Command)] = &[
    ("Check for updates", "", Command::CheckUpdates),
    ("Show CPU and memory graphs", "", Command::Usage),
    ("Keyboard shortcuts", "⌘?", Command::Shortcuts),
    ("Toggle Vim mode", "", Command::ToggleVim),
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
    ("Focus sidebar", "⌘⌥S", Command::FocusSidebar),
    ("Select next sidebar item", "↓", Command::SidebarDown),
    ("Select previous sidebar item", "↑", Command::SidebarUp),
    ("Expand sidebar folder", "→", Command::SidebarExpand),
    ("Collapse sidebar folder", "←", Command::SidebarCollapse),
    ("Open sidebar item", "Enter", Command::SidebarOpen),
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
    ("Move tab to next pane", "", Command::MoveTabNextPane),
    ("Toggle hidden files", "⌘Shift.", Command::Hidden),
    ("Add current folder to favorites", "⌘D", Command::Favorite),
];
pub struct Workspace {
    vim: vim::VimState,
    usage: Entity<usage::UsageMonitor>,
    shortcuts_open: bool,
    shortcuts_focus: FocusHandle,
    shortcuts_previous_focus: Option<FocusHandle>,
    shortcuts_scroll: ScrollHandle,
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
    connection_groups: Vec<String>,
    connection_focus: FocusHandle,
    connection_screen: Option<ConnectionScreen>,
    connection_inputs: Vec<Entity<InputState>>,
    connection_busy: bool,
    connection_cancel: CancellationToken,
    connection_generation: u64,
    connection_scroll: ScrollHandle,
    sftp_listing_cache: VecDeque<(Location, bool, Vec<Entry>)>,
    listing_cache_epoch: u64,
    active: usize,
    preferences: Preferences,
    notice: Option<String>,
    palette: bool,
    palette_input: Entity<InputState>,
    palette_index: usize,
    palette_scroll: ScrollHandle,
    sidebar_tree: TreeState,
    sidebar_collapsed: HashSet<String>,
    sidebar_scroll: ScrollHandle,
    sidebar_search: Entity<InputState>,
    sidebar_cursor: usize,
    sidebar_focus: FocusHandle,
    sidebar_split: Entity<ResizableState>,
    _sidebar_subscription: std::rc::Rc<Subscription>,
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
            KeyBinding::new(
                "cmd-shift-/",
                ToggleShortcuts,
                Some("Workspace || Shortcuts"),
            ),
            KeyBinding::new("cmd-?", ToggleShortcuts, Some("Workspace || Shortcuts")),
            KeyBinding::new("?", ToggleShortcuts, Some("Listing && !Vim")),
            KeyBinding::new("shift-/", ToggleShortcuts, Some("Listing && !Vim")),
            KeyBinding::new("tab", SwitchPane, Some("Workspace && !Terminal && !Vim")),
            KeyBinding::new("shift-tab", PreviousPane, Some("Workspace && !Terminal && !VimInput")),
            KeyBinding::new("cmd-alt-right", SplitRight, Some("Workspace")),
            KeyBinding::new("cmd-alt-shift-right", SplitTerminalRight, Some("Workspace")),
            KeyBinding::new("cmd-alt-shift-down", SplitTerminalDown, Some("Workspace")),
            KeyBinding::new("cmd-alt-down", SplitDown, Some("Workspace")),
            KeyBinding::new("cmd-alt-w", ClosePane, Some("Workspace")),
            KeyBinding::new("cmd-alt-]", SwitchPane, Some("Workspace")),
            KeyBinding::new("cmd-alt-[", PreviousPane, Some("Workspace")),
            KeyBinding::new("cmd-b", ToggleSidebar, Some("Workspace")),
            KeyBinding::new("cmd-alt-s", FocusSidebar, Some("Workspace")),
            KeyBinding::new("up", SidebarUp, Some("Sidebar")),
            KeyBinding::new("down", SidebarDown, Some("Sidebar")),
            KeyBinding::new("right", SidebarExpand, Some("Sidebar")),
            KeyBinding::new("left", SidebarCollapse, Some("Sidebar")),
            KeyBinding::new("enter", SidebarOpen, Some("Sidebar")),
            KeyBinding::new("return", SidebarOpen, Some("Sidebar")),
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
            KeyBinding::new("down", SelectNext, Some("Listing && !VimInput")),
            KeyBinding::new("up", SelectPrevious, Some("Listing && !VimInput")),
            KeyBinding::new("right", ExpandRow, Some("Listing && !VimInput")),
            KeyBinding::new("left", CollapseRow, Some("Listing && !VimInput")),
            KeyBinding::new("enter", OpenSelection, Some("Listing && !Vim")),
            KeyBinding::new("cmd-a", SelectAll, Some("Listing && !VimInput")),
            KeyBinding::new("cmd-l", EditPath, Some("Workspace")),
            KeyBinding::new("cmd-o", ChooseFolder, Some("Workspace")),
            KeyBinding::new("cmd-shift-.", ToggleHidden, Some("Workspace")),
            KeyBinding::new("cmd-shift-p", TogglePalette, Some("Workspace")),
            KeyBinding::new("escape", Escape, Some("Workspace && !Terminal && !Vim && !TransferConflict")),
            KeyBinding::new("cmd-d", AddFavorite, Some("Workspace")),
            KeyBinding::new("cmd-shift-d", RemoveFavorite, Some("Workspace")),
            KeyBinding::new("shift-down", ExtendSelectionNext, Some("Listing && !VimInput")),
            KeyBinding::new("shift-up", ExtendSelectionPrevious, Some("Listing && !VimInput")),
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
        let connection_inputs = (0..16)
            .map(|i| {
                cx.new(|cx| InputState::new(window, cx).masked(matches!(i, 9 | 10 | 11 | 12 | 15)))
            })
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
        let sidebar_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search sidebar…"));
        let split_state = cx.new(|_| ResizableState::default());
        let split_subscription = Self::observe_split(&split_state, cx);
        let sidebar_split = cx.new(|_| ResizableState::default());
        let sidebar_subscription = Self::observe_split(&sidebar_split, cx);
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
            |this, _, event, window, cx| match event {
                InputEvent::PressEnter { .. } => this.run_selected_palette_command(window, cx),
                InputEvent::Change => {
                    this.palette_index = 0;
                    this.palette_scroll.scroll_to_item(0);
                    cx.notify();
                }
                _ => {}
            },
        ));
        subscriptions.push(
            cx.subscribe_in(&sidebar_search, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.sidebar_cursor = this
                        .visible_sidebar_nodes(cx)
                        .first()
                        .map(|(index, _)| *index)
                        .unwrap_or(0);
                    cx.notify();
                }
            }),
        );
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
                this.ensure_sidebar_disks_loaded(cx);
            }
        });
        let mut workspace = Self {
            vim: vim::VimState::default(),
            usage: cx.new(|cx| usage::UsageMonitor::new(colors, cx)),
            panes,
            colors,
            settings_open: false,
            settings_inputs,
            shortcuts_open: false,
            shortcuts_focus: cx.focus_handle(),
            shortcuts_previous_focus: None,
            shortcuts_scroll: ScrollHandle::new(),
            settings_scroll: ScrollHandle::new(),
            connections: vec![],
            connection_groups: vec![],
            connection_focus: cx.focus_handle(),
            connection_screen: None,
            connection_inputs,
            connection_busy: false,
            connection_cancel: CancellationToken::new(),
            connection_generation: 0,
            connection_scroll: ScrollHandle::new(),
            sftp_listing_cache: VecDeque::new(),
            listing_cache_epoch: 0,
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
            palette_index: 0,
            palette_scroll: ScrollHandle::new(),
            sidebar_tree: TreeState::default(),
            sidebar_collapsed: HashSet::new(),
            sidebar_scroll: ScrollHandle::new(),
            sidebar_search,
            sidebar_cursor: 0,
            sidebar_focus: cx.focus_handle(),
            sidebar_split,
            _sidebar_subscription: sidebar_subscription,
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
        workspace.start_transfer_poll(window, cx);
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
        self.reset_vim();
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
        let path = self.panes[i].tab().path.clone();
        let cached = self.sftp_listing_cache.iter()
            .find(|(location, hidden, _)| location == &path && *hidden == show_hidden)
            .map(|(_, _, entries)| entries.clone());
        let cache_epoch = self.listing_cache_epoch;
        let tab = self.panes[i].tab_mut();
        tab.cancel.cancel();
        tab.cancel = CancellationToken::new();
        tab.generation += 1;
        tab.loading = true;
        tab.error = None;
        tab.entries.clear();
        tab.depths.clear();
        tab.root_entries.clear();
        tab.tree.reset_children();
        tab.selected.clear();
        tab.anchor = None;
        tab.selection_cursor = None;
        tab.cached_listing = cached.is_some();
        if let Some(entries) = cached {
            tab.root_entries = entries;
            tab.sort_entries();
        }
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
                let Some(i) = this.panes.ids().into_iter().find(|pane| this.panes[*pane].tabs.iter().any(|tab| tab.id == id)) else { return; };
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
                let mut reexpand = vec![];
                match result {
                    Ok(entries) => {
                        let cache_entries = if matches!(path, Location::Sftp { .. }) && entries.len() <= 10_000 {
                            Some(entries.clone())
                        } else { None };
                        tab.root_entries = entries;
                        tab.sort_entries();
                        tab.cached_listing = false;
                        // Refresh keeps folders expanded; reload each of them.
                        reexpand = std::mem::take(&mut tab.tree.expanded).into_iter().collect();
                        if cache_epoch == this.listing_cache_epoch {
                            if let Some(entries) = cache_entries {
                                this.sftp_listing_cache.retain(|(location, hidden, _)| location != &path || *hidden != show_hidden);
                                this.sftp_listing_cache.push_back((path.clone(), show_hidden, entries));
                                while this.sftp_listing_cache.len() > 64 || this.sftp_listing_cache.iter().map(|(_, _, entries)| entries.len()).sum::<usize>() > 50_000 {
                                    this.sftp_listing_cache.pop_front();
                                }
                            }
                        }
                    }
                    Err(error) => tab.error = Some(error.to_string()),
                }
                for location in reexpand {
                    this.expand_location(i, id, location, cx);
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
        if tab.path != path {
            tab.tree.clear();
            self.vim = vim::VimState::default();
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
        if self.operation_dialog.is_some() { return; }
        if self.connection_screen.is_some()
            && !matches!(command,
                Command::SaveConnection | Command::TestDraftConnection | Command::ConfirmConnection
                | Command::ConnectionProtocol(_) | Command::SavedConnection(_, _)
                | Command::Connections | Command::NewConnection | Command::ImportForkLift
                | Command::TestConnection | Command::EditConnection | Command::RemoveConnection
                | Command::ResetHost)
        {
            return;
        }
        if matches!(command, Command::Shortcuts) {
            self.toggle_shortcuts(window, cx);
            return;
        }
        if self.shortcuts_open || !self.preferences_loaded {
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
            Command::ToggleVim => {
                self.preferences.vim_mode = !self.preferences.vim_mode;
                self.vim = vim::VimState::default();
                self.persist(cx);
            }
            Command::Shortcuts => unreachable!(),
            Command::Usage => {
                self.settings_open = false;
                self.toggle_usage(window, cx);
            },
            Command::CheckUpdates => {
                if let Err(error) = crate::updater::check_for_updates() { self.notice = Some(error); cx.notify(); }
            },
            Command::Settings => self.open_settings(window, cx),
            Command::SidebarUp => self.move_sidebar_cursor(-1, cx),
            Command::SidebarDown => self.move_sidebar_cursor(1, cx),
            Command::SidebarExpand => self.expand_sidebar_cursor(cx),
            Command::SidebarCollapse => self.collapse_sidebar_cursor(cx),
            Command::SidebarOpen => self.open_sidebar_cursor(window, cx),
            Command::FocusSidebar => {
                if !self.preferences.sidebar_visible {
                    self.preferences.sidebar_visible = true;
                    self.sidebar_cursor = 0;
                    self.persist(cx);
                }
                self.sidebar_focus.focus(window, cx);
                cx.notify();
            }
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
                self.connection_focus.focus(window, cx);
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
                if self.preferences.sidebar_visible {
                    self.sidebar_scroll.set_offset(point(px(0.), px(0.)));
                    self.sidebar_cursor = 0;
                }
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
                if self.panes[i].tabs.len() == 1 {
                    // The last tab closes its split; each original side keeps one pane.
                    self.close_pane(window, cx);
                } else {
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
            Command::MoveTabNextPane => {
                let leaves = self.layout.leaves();
                let position = leaves.iter().position(|pane| *pane == i).unwrap_or(0);
                let destination = leaves[(position + 1) % leaves.len()];
                let tab = self.panes[i].tab();
                let drag = TabDrag { source_pane: i, tab_id: tab.id, label: tab.path.label() };
                self.move_dragged_tab(&drag, destination, None, window, cx);
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
            } else if entry.kind == EntryKind::Symlink {
                self.open_linked_folder(self.active, entry.location.clone(), window, cx);
            } else {
                let location = entry.location.clone();
                self.open_file(location, cx);
            }
        }
    }
    fn open_linked_folder(&mut self, pane: usize, location: Location, window: &mut Window, cx: &mut Context<Self>) {
        let tab = self.panes[pane].tab();
        let id = tab.id;
        let generation = tab.generation;
        let cancel = tab.cancel.clone();
        let registry = self.registry.clone();
        let task = cx.background_executor().spawn(async move {
            registry.linked_directory(&location, &cancel)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.panes.contains(pane) || this.panes[pane].tab().id != id || this.panes[pane].tab().generation != generation { return; }
                match result {
                    Ok(target) => this.navigate(pane, target, true, window, cx),
                    Err(error) => { this.notice = Some(error.to_string()); cx.notify(); }
                }
            });
        }).detach();
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
    /// One pane's tab strip, rendered in the window's top tab row.
    fn move_dragged_tab(&mut self, drag: &TabDrag, destination: usize, before: Option<u64>, window: &mut Window, cx: &mut Context<Self>) {
        let source = drag.source_pane;
        if self.connection_screen.is_some() || !self.panes.contains(source) || !self.panes.contains(destination) { return; }
        let Some(index) = self.panes[source].tabs.iter().position(|tab| tab.id == drag.tab_id) else { return; };
        if before == Some(drag.tab_id) { return; }
        let path = self.panes[source].tabs[index].path.clone();
        let active_id = self.panes[source].tab().id;
        let tab = self.panes[source].tabs.remove(index);
        if source != destination {
            self.panes[source].recent_file_tab = self.panes[source].recent_file_tab.filter(|id| *id != drag.tab_id);
            self.panes[source].recent_terminal_tab = self.panes[source].recent_terminal_tab.filter(|id| *id != drag.tab_id);
        }
        let insertion = before.and_then(|id| self.panes[destination].tabs.iter().position(|tab| tab.id == id))
            .unwrap_or(self.panes[destination].tabs.len());
        self.panes[destination].tabs.insert(insertion, tab);
        self.panes[destination].active = insertion;
        if source != destination {
            if self.panes[source].tabs.is_empty() {
                if self.can_close_pane(source) {
                    self.active = source;
                    self.close_pane(window, cx);
                } else {
                    self.panes[source].tabs.push(Tab::new(self.next_id, path));
                    self.next_id += 1;
                    self.panes[source].active = 0;
                    self.request_listing(source, cx);
                }
            } else {
                self.panes[source].active = self.panes[source].tabs.iter().position(|tab| tab.id == active_id)
                    .unwrap_or(index.min(self.panes[source].tabs.len() - 1));
            }
            if self.panes.contains(source) { self.sync_path(source, window, cx); }
        }
        self.sync_path(destination, window, cx);
        self.activate(destination, window, cx);
        self.persist(cx);
        cx.notify();
    }
    fn render_pane_tabs(&self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.colors;
        let font_size = self.preferences.appearance.font_size;
        let scale = font_size / 13.;
        let pane = &self.panes[i];
        let active = self.active == i;
        let can_close_pane = self.can_close_pane(i);
        let pane_tab_id = pane.tab().id;
        div()
            .id(("pane-tabs", i))
            .key_context("TabStrip")
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .h(px(font_size + 19.))
            .line_height(relative(1.))
            .flex()
            .items_center()
            .bg(rgb(if active {
                theme.background
            } else {
                theme.surface
            }))
            .border_b_1()
            .border_color(rgb(if active { theme.accent } else { theme.border }))
            .can_drop(|payload, _, _| payload.is::<TabDrag>())
            .drag_over::<TabDrag>(move |style, _, _, _| style.bg(rgb(theme.hover)))
            .on_drop(cx.listener(move |this, drag: &TabDrag, window, cx| {
                this.move_dragged_tab(drag, i, None, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    if this.panes.contains(i) {
                        this.activate(i, window, cx);
                    }
                }),
            )
            .child(
                div()
                    .id(("pane-tabs-scroll", i))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .overflow_x_scroll()
                    .children(pane.tabs.iter().enumerate().map(|(t, tab)| {
                        let is_active = t == pane.active;
                        let path_label = tab.path.label();
                        let tab_icon = if tab.terminal.is_some() {
                            Icon::new(IconName::SquareTerminal)
                        } else if matches!(tab.path, Location::S3 { .. }) {
                            Icon::new(gpui_kit::assets::IconName::Cloud)
                        } else if !tab.path.is_local() {
                            Icon::new(gpui_kit::assets::IconName::Server)
                        } else {
                            Icon::new(gpui_kit::assets::IconName::FolderTree)
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
                                    .px(px(10.))
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .cursor_pointer()
                                    .on_drag(TabDrag { source_pane: i, tab_id, label: path_label.clone() }, |drag, _, _, cx| {
                                        cx.new(|_| PaneDragPreview { label: drag.label.clone() })
                                    })
                                    .can_drop(|payload, _, _| payload.is::<TabDrag>())
                                    .drag_over::<TabDrag>(move |style, _, _, _| style.border_l_2().border_color(rgb(theme.accent)))
                                    .on_drop(cx.listener(move |this, drag: &TabDrag, window, cx| {
                                        cx.stop_propagation();
                                        this.move_dragged_tab(drag, i, Some(tab_id), window, cx);
                                    }))
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
                                    .child(
                                        tab_icon
                                            .size(px(font_size + 1.))
                                            .flex_none()
                                            .text_color(rgb(theme.muted)),
                                    )
                                    .child(
                                        div().min_w_0().text_ellipsis().child(path_label.clone()),
                                    ),
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
                                    .tooltip(if pane.tabs.len() <= 1 {
                                        "Close tab and its split · ⌘W"
                                    } else if is_active {
                                        "Close tab · ⌘W"
                                    } else {
                                        "Select and close tab · ⌘W"
                                    })
                                    .disabled(pane.tabs.len() <= 1 && !can_close_pane)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if this.panes.contains(i)
                                            && this.panes[i]
                                                .tabs
                                                .get(t)
                                                .is_some_and(|tab| tab.id == tab_id)
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
                    })),
            )
            .child(
                div()
                    .flex_none()
                    .h_full()
                    .flex()
                    .items_center()
                    .child(
                        Button::new(("new-tab", i))
                            .ghost()
                            .compact()
                            .icon(Icon::new(IconName::Plus))
                            .accessibility_label("New tab; Option-click to split down; Shift-Option-click to split right")
                            .tooltip("New tab · ⌘T  ·  ⌥-click: split down  ·  ⇧⌥-click: split right")
                            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                if !this.panes.contains(i) || this.panes[i].tab().id != pane_tab_id
                                {
                                    return;
                                }
                                this.activate(i, window, cx);
                                this.command(
                                    if event.modifiers().alt && event.modifiers().shift {
                                        Command::Split(Axis::Right)
                                    } else if event.modifiers().alt {
                                        Command::Split(Axis::Down)
                                    } else {
                                        Command::NewTab
                                    },
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new(("new-terminal-tab", i))
                            .ghost()
                            .compact()
                            .icon(Icon::new(IconName::SquareTerminal))
                            .accessibility_label("New terminal tab; Option-click to split terminal down; Shift-Option-click to split terminal right")
                            .tooltip("New terminal tab · ⌘⌥T  ·  ⌥-click: split down  ·  ⇧⌥-click: split right")
                            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                if !this.panes.contains(i) || this.panes[i].tab().id != pane_tab_id
                                {
                                    return;
                                }
                                this.activate(i, window, cx);
                                this.command(
                                    if event.modifiers().alt && event.modifiers().shift {
                                        Command::SplitTerminal(Axis::Right)
                                    } else if event.modifiers().alt {
                                        Command::SplitTerminal(Axis::Down)
                                    } else {
                                        Command::NewTerminal
                                    },
                                    window, cx,
                                );
                            })),
                    ),
            )
            .into_any_element()
    }
    fn render_path_bar(&self, i: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let pane = &self.panes[i];
        let font_size = self.preferences.appearance.font_size;
        let loading_indicator = || div().flex_none().w(px(20.)).h_full().flex().items_center().justify_center()
            .when(pane.tab().loading, |slot| slot.child(
                gpui_kit::component::spinner::Spinner::new().with_size(px(14.)).color(rgb(self.colors.muted).into())
            ));
        if pane.path_input.read(cx).focus_handle(cx).is_focused(window) {
            return div().flex().items_center().h_full()
                .child(div().flex_1().min_w_0().child(Styled::h(Input::new(&pane.path_input), px(font_size + 16.))
                    .px(px(10.)).py(px(4.)).text_size(px(font_size))))
                .child(loading_indicator()).into_any_element();
        }
        let mut crumbs = vec![pane.tab().path.clone()];
        while let Some(parent) = crumbs.last().and_then(Location::parent) {
            if crumbs.contains(&parent) { break; }
            crumbs.push(parent);
        }
        crumbs.reverse();
        let tab_id = pane.tab().id;
        div().flex().items_center().h_full().gap(px(4.))
            .child(div().id(("path-breadcrumbs", i)).flex_1().min_w_0().h_full().flex().items_center().overflow_x_scroll()
                .children(crumbs.into_iter().enumerate().map(|(index, location)| {
                    let mut label = location.label();
                    if index == 0 {
                        if let Location::Sftp { connection, .. } | Location::Ftps { connection, .. } = &location {
                            label = self.connections.iter().find(|record| &record.id == connection)
                                .map(|record| record.name.clone()).unwrap_or_else(|| "Remote".into());
                        }
                    }
                    div().flex_none().flex().items_center().gap(px(4.))
                        .when(index > 0, |row| row.child(Icon::new(IconName::ChevronRight).size(px(12.)).text_color(rgb(self.colors.muted))))
                        .child(Button::new(("breadcrumb", i * 1000 + index)).ghost().compact().label(label.clone())
                            .accessibility_label(format!("Open folder {label}"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if !this.panes.contains(i) || this.panes[i].tab().id != tab_id { return; }
                                this.activate(i, window, cx);
                                this.navigate(i, location.clone(), true, window, cx);
                            })))
                })))
            .child(loading_indicator())
            .child(Button::new(("edit-pane-path", i)).ghost().compact().label("…")
                .accessibility_label("Edit path").tooltip("Edit path · ⌘L")
                .on_click(cx.listener(move |this, _, window, cx| {
                    if !this.panes.contains(i) { return; }
                    this.activate(i, window, cx);
                    this.command(Command::EditPath, window, cx);
                })))
            .into_any_element()
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
            - if self.preferences.sidebar_visible {
                self.sidebar_split
                    .read(cx)
                    .sizes()
                    .first()
                    .copied()
                    .filter(|width| *width > px(0.))
                    .unwrap_or(px(sidebar::SIDEBAR_WIDTH))
            } else {
                px(0.)
            }
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
        let mut content = div()
            .id(("listing", i))
            .track_focus(&pane.focus)
            .key_context(if self.preferences.vim_mode {
                if self.vim_input_active() { "Listing Vim VimInput" } else { "Listing Vim" }
            } else { "Listing" })
            .on_key_down(cx.listener(|this, event, window, cx| {
                if this.vim_key(event, window, cx) { cx.stop_propagation(); }
            }))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&pane.scroll)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| this.activate(i, window, cx)),
            );
        if tab.cached_listing && let Some(error) = &tab.error {
            content = content.child(div().px_3().py_1().text_color(rgb(theme.warning))
                .child(format!("Refresh failed: {error} · ⌘R Retry")));
        }
        if tab.loading && !tab.cached_listing {
            // Keep initial loading empty; progress lives in the fixed path bar.
        } else if let Some(error) = tab.error.as_ref().filter(|_| !tab.cached_listing) {
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
                let is_link = entry.kind == EntryKind::Symlink;
                let depth = tab.depth(row);
                let expanded = is_dir && tab.tree.is_expanded(&entry.location);
                let loading = expanded && tab.tree.is_loading(&entry.location);
                let (icon, icon_color) = file_icons::entry_icon(entry, expanded, &theme);
                let toggle_path = entry.location.clone();
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
                    } else if self.preferences.vim_mode && i == self.active && tab.selection_cursor == Some(row) {
                        theme.hover
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
                                } else if is_link {
                                    this.open_linked_folder(i, path.clone(), window, cx);
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
                    .when(depth > 0, |row| {
                        row.child(div().flex_none().h_full().flex().children((0..depth).map(
                            |_| {
                                div().flex_none().w(px(INDENT)).h_full().child(
                                    div()
                                        .ml(px(6.))
                                        .h_full()
                                        .border_l_1()
                                        .border_color(rgb(theme.border)),
                                )
                            },
                        )))
                    })
                    .child(
                        div()
                            .id(format!("disclosure-{i}-{row}"))
                            .flex_none()
                            .w(px(12.))
                            .h_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(if selected { theme.text } else { theme.muted }))
                            .when(is_dir, |chevron| {
                                chevron
                                    .cursor_pointer()
                                    .opacity(if loading { 0.4 } else { 1. })
                                    .child(
                                        Icon::new(if expanded {
                                            IconName::ChevronDown
                                        } else {
                                            IconName::ChevronRight
                                        })
                                        .size(px(12.)),
                                    )
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                            cx.stop_propagation();
                                            if this.panes.contains(i)
                                                && this.panes[i].tab().id == pane_tab_id
                                                && this.panes[i].tab().entries.get(row).is_some_and(
                                                    |entry| entry.location == toggle_path,
                                                )
                                            {
                                                this.activate(i, window, cx);
                                                this.toggle_row(i, row, cx);
                                            }
                                        }),
                                    )
                            }),
                    )
                    .child(
                        Icon::new(icon)
                            .size(px((font_size + 1.).max(12.)))
                            .text_color(rgb(icon_color)),
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
            .when(!self.layout.top_leaves().contains(&i), |pane| {
                pane.child(self.render_pane_tabs(i, cx))
            })
            .can_drop(move |payload, _, _| {
                payload.is::<TabDrag>() || file_tab
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
            .drag_over::<TabDrag>(move |style, _, _, _| {
                style.border_2().border_color(rgb(theme.accent))
            })
            .on_drop(cx.listener(move |this, drag: &PaneDrag, _, cx| {
                this.drop_copy(drag.clone(), i, pane_tab_id, cx);
            }))
            .on_drop(cx.listener(move |this, drag: &TabDrag, window, cx| {
                this.move_dragged_tab(drag, i, None, window, cx);
            }))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                this.drop_external_copy(paths.clone(), i, pane_tab_id, cx);
            }))
            .when(file_tab, |panel| {
                panel
                    .child(
                        div()
                            .h(px(font_size + 22.))
                            .flex_none()
                            .px_2()
                            .py_1()
                            .child(self.render_path_bar(i, window, cx)),
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
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(12. + 8. + (font_size + 1.).max(12.))),
                            )
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
                pane.child(div().flex_1().min_h_0().child(terminal).on_mouse_down(
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
        let mut root = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme.background))
            .text_color(rgb(theme.text))
            .font_family(self.preferences.appearance.font_family.clone())
            .text_size(px(font_size))
            .key_context(if self.shortcuts_open {
                "Shortcuts"
            } else {
                "Workspace"
            });
        macro_rules! action {
            ($ty:ty,$command:expr) => {
                root = root.on_action(
                    cx.listener(|this, _: &$ty, window, cx| this.command($command, window, cx)),
                );
            };
        }
        action!(ToggleShortcuts, Command::Shortcuts);
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
        action!(FocusSidebar, Command::FocusSidebar);
        action!(SidebarUp, Command::SidebarUp);
        action!(SidebarDown, Command::SidebarDown);
        action!(SidebarExpand, Command::SidebarExpand);
        action!(SidebarCollapse, Command::SidebarCollapse);
        action!(SidebarOpen, Command::SidebarOpen);
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
            .on_action(cx.listener(|this, _: &ExpandRow, _, cx| this.expand_selection(cx)))
            .on_action(cx.listener(|this, _: &CollapseRow, _, cx| this.collapse_selection(cx)))
            .on_action(
                cx.listener(|this, _: &OpenSelection, window, cx| this.open_selection(window, cx)),
            )
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                let tab = this.panes[this.active].tab_mut();
                tab.selected = (0..tab.entries.len()).collect();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &EditPath, window, cx| {
                this.command(Command::EditPath, window, cx);
            }))
            .on_action(cx.listener(|this, _: &TogglePalette, window, cx| {
                if this.connection_screen.is_some() || this.operation_dialog.is_some() {
                    return;
                }
                if this.palette {
                    this.close_palette(window, cx)
                } else {
                    this.open_palette(window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &CheckUpdates, window, cx| {
                this.command(Command::CheckUpdates, window, cx);
            }))
            .on_action(cx.listener(|this, _: &Escape, window, cx| {
                if matches!(this.operation_dialog, Some(OperationDialog::Conflict(..))) { return; }
                this.palette = false;
                if this.sidebar_focus.is_focused(window) {
                    this.activate(this.active, window, cx);
                    return;
                }
                this.settings_open = false;
                this.operation_dialog = None;
                this.close_connections(window, cx);
                this.notice = None;
                this.activate(this.active, window, cx);
            }));
        root = root.child(self.render_title_bar(window, cx));
        if self.settings_open {
            let mut root = root.child(self.render_settings(cx));
            if self.palette {
                root = root.child(self.render_palette(window, cx));
            }
            return self.with_shortcuts(root, window, cx);
        }
        let main = div().size_full().min_w_0().flex().flex_col().child(
            div()
                .flex_1()
                .min_h_0()
                .child(self.render_layout(&self.layout, window, cx)),
        );
        root = root.child(
            div().flex_1().min_h_0().flex().child(
                h_resizable("sidebar-split")
                    .with_state(&self.sidebar_split)
                    .child(
                        resizable_panel()
                            .visible(self.preferences.sidebar_visible)
                            .size(px(sidebar::SIDEBAR_WIDTH))
                            .size_range(px(120.)..px(480.))
                            .child(self.render_sidebar(window, cx)),
                    )
                    .child(resizable_panel().child(main)),
            ),
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
        let root = root.child(
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
                .child(self.status_usage(cx))
                .when(self.preferences.vim_mode && tab.terminal.is_none(), |bar| {
                    bar.child(div().text_color(rgb(theme.accent)).child(self.vim_status()))
                })
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
        );
        let root = if self.palette {
            root.child(self.render_palette(window, cx))
        } else {
            root
        };
        self.with_shortcuts(root, window, cx)
    }
}
