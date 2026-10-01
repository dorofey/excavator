//! Workspace terminal actions. Never send browser navigation into a live shell.
use super::*;
use crate::terminal::Launch;
use std::ffi::OsString;

impl Workspace {
    pub(super) fn terminal_launch(&self) -> Result<(Launch, String), String> {
        if let Some(launch) = &self.panes[self.active].tab().terminal_launch {
            return Ok((
                launch.clone(),
                self.panes[self.active].tab().terminal_label.clone(),
            ));
        }
        match &self.panes[self.active].tab().path {
            Location::Local(path) => Ok((Launch::local(path.clone()), path.display().to_string())),
            Location::Sftp { connection, path } => {
                let record = self.connections.iter().find(|record| &record.id == connection)
                    .ok_or("The active SFTP connection is no longer available")?;
                record.validate()?;
                // SSH receives an argument vector, never a local shell command. The remote
                // shell receives only one single-quoted directory, including escaped quotes.
                let quoted = format!("'{}'", path.replace('\'', "'\\''"));
                let command = format!("cd {quoted} && exec \"${{SHELL:-/bin/sh}}\" -l");
                let args = vec![OsString::from("-tt"), OsString::from("-p"),
                    OsString::from(record.port.to_string()), OsString::from("-l"),
                    OsString::from(&record.username), OsString::from("--"),
                    OsString::from(&record.host), OsString::from(command)];
                Ok((Launch::command(PathBuf::from("/usr/bin/ssh"), args, None),
                    format!("{} · {} · system SSH authentication / known_hosts", record.name, path)))
            }
            Location::Ftps { .. } | Location::S3 { .. } =>
                Err("This provider has no shell. Choose a local folder or SFTP pane for a new terminal.".into()),
        }
    }

    pub(super) fn make_terminal_tab(
        &mut self,
        path: Location,
        launch: Launch,
        label: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Tab {
        let terminal = cx.new(|cx| TerminalView::new(launch.clone(), label.clone(), window, cx));
        terminal.update(cx, |view, cx| {
            view.set_theme(self.colors, self.preferences.appearance.font_size, cx)
        });
        let mut tab = Tab::new(self.next_id, path);
        self.next_id += 1;
        tab.terminal_subscription = Some(cx.observe(&terminal, |_, _, cx| cx.notify()));
        tab.terminal = Some(terminal);
        tab.terminal_label = label;
        tab.terminal_launch = Some(launch);
        tab
    }
    pub(super) fn show_terminal(&mut self, new: bool, window: &mut Window, cx: &mut Context<Self>) {
        let i = self.active;
        if !new {
            let pane = &self.panes[i];
            let existing = if pane.tab().terminal.is_some() {
                Some(pane.active)
            } else {
                pane.recent_terminal_tab
                    .and_then(|id| {
                        pane.tabs
                            .iter()
                            .position(|tab| tab.id == id && tab.terminal.is_some())
                    })
                    .or_else(|| pane.tabs.iter().rposition(|tab| tab.terminal.is_some()))
            };
            if let Some(index) = existing {
                self.panes[i].active = index;
                self.activate(i, window, cx);
                return;
            }
        }
        let (launch, label) = match self.terminal_launch() {
            Ok(result) => result,
            Err(error) => {
                self.notice = Some(error);
                cx.notify();
                return;
            }
        };
        let path = self.panes[i].tab().path.clone();
        let tab = self.make_terminal_tab(path, launch, label, window, cx);
        self.panes[i].tabs.push(tab);
        self.panes[i].active = self.panes[i].tabs.len() - 1;
        self.activate(i, window, cx);
        cx.notify();
    }
    pub(super) fn ensure_file_tab(
        &mut self,
        i: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane = &self.panes[i];
        let existing = if pane.tab().terminal.is_none() {
            Some(pane.active)
        } else {
            pane.recent_file_tab
                .and_then(|id| {
                    pane.tabs
                        .iter()
                        .position(|tab| tab.id == id && tab.terminal.is_none())
                })
                .or_else(|| pane.tabs.iter().rposition(|tab| tab.terminal.is_none()))
        };
        let index = existing.unwrap_or_else(|| {
            let path = self.panes[i].tab().path.clone();
            let tab = Tab::new(self.next_id, path);
            self.next_id += 1;
            self.panes[i].tabs.push(tab);
            self.panes[i].tabs.len() - 1
        });
        self.panes[i].active = index;
        self.sync_path(i, window, cx);
        if self.panes[i].tab().generation == 0 {
            self.request_listing(i, cx);
        }
    }
    pub(super) fn focus_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_file_tab(self.active, window, cx);
        self.activate(self.active, window, cx);
    }
    pub(super) fn end_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let i = self.active;
        let pane = &self.panes[i];
        let target = if pane.tab().terminal.is_some() {
            Some(pane.active)
        } else {
            pane.recent_terminal_tab
                .and_then(|id| {
                    pane.tabs
                        .iter()
                        .position(|tab| tab.id == id && tab.terminal.is_some())
                })
                .or_else(|| pane.tabs.iter().rposition(|tab| tab.terminal.is_some()))
        };
        let Some(index) = target else {
            self.notice = Some("This pane has no terminal session to end.".into());
            cx.notify();
            return;
        };
        let path = self.panes[i].tabs[index].path.clone();
        self.panes[i].tabs.remove(index);
        if self.panes[i].tabs.is_empty() {
            self.panes[i].tabs.push(Tab::new(self.next_id, path));
            self.next_id += 1;
        }
        self.panes[i].active = self.panes[i]
            .active
            .saturating_sub(usize::from(index <= self.panes[i].active))
            .min(self.panes[i].tabs.len() - 1);
        self.focus_files(window, cx);
        cx.notify();
    }
    pub(super) fn split_terminal(
        &mut self,
        axis: Axis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_pane_kind(axis, true, window, cx);
    }
}

impl Workspace {
    /// Isolated real PTY fixture covering pane/tab session ownership.
    #[allow(dead_code)]
    pub fn verify_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>, path: PathBuf) {
        let (launch, _) = self.terminal_launch().unwrap();
        assert_eq!(launch.cwd, Some(path.clone()));
        let browser = self.panes[0].tab().id;
        let history = self.panes[0].tab().history.clone();
        let launch = Launch::command(
            PathBuf::from("/bin/sh"),
            vec!["-i".into()],
            Some(path.clone()),
        );
        let tab = self.make_terminal_tab(
            Location::Local(path.clone()),
            launch,
            path.display().to_string(),
            window,
            cx,
        );
        let first = tab.terminal.as_ref().unwrap().entity_id();
        self.panes[0].tabs.push(tab);
        self.panes[0].active = 1;
        self.command(Command::FocusTerminal, window, cx);
        self.command(Command::Terminal, window, cx);
        assert_eq!(self.panes[0].tab().id, browser);
        assert_eq!(self.panes[0].tab().history, history);
        self.command(Command::FocusTerminal, window, cx);
        assert_eq!(
            self.panes[0].tab().terminal.as_ref().unwrap().entity_id(),
            first
        );
        self.command(Command::NewTerminal, window, cx);
        let extra = self.panes[0].tab().terminal.as_ref().unwrap().entity_id();
        assert_ne!(extra, first);
        self.command(Command::CloseTab, window, cx);
        assert_eq!(
            self.panes[0].tab().terminal.as_ref().unwrap().entity_id(),
            first
        );
        self.command(Command::Split(Axis::Right), window, cx);
        let second = self.active;
        let second_entity = self.panes[second]
            .tab()
            .terminal
            .as_ref()
            .unwrap()
            .entity_id();
        assert_ne!(second_entity, first);
        self.command(Command::SplitTerminal(Axis::Down), window, cx);
        let third = self.active;
        let third_entity = self.panes[third]
            .tab()
            .terminal
            .as_ref()
            .unwrap()
            .entity_id();
        assert_ne!(third_entity, second_entity);
        self.command(Command::Split(Axis::Down), window, cx);
        let disposable = self.active;
        self.command(Command::ClosePane, window, cx);
        assert!(!self.panes.contains(disposable));
        assert_eq!(
            self.panes[second]
                .tab()
                .terminal
                .as_ref()
                .unwrap()
                .entity_id(),
            second_entity
        );
        assert_eq!(
            self.panes[third]
                .tab()
                .terminal
                .as_ref()
                .unwrap()
                .entity_id(),
            third_entity
        );
        self.activate(0, window, cx);
        self.command(Command::FocusFiles, window, cx);
        assert_eq!(self.panes[0].tab().id, browser);
        assert_eq!(self.panes[0].tab().history, history);
        self.command(Command::EndTerminal, window, cx);
        assert!(self.panes[0].tabs.iter().all(|tab| tab.terminal.is_none()));
        assert_eq!(
            self.panes[second]
                .tab()
                .terminal
                .as_ref()
                .unwrap()
                .entity_id(),
            second_entity
        );
        let old_location = self.panes[0].tab().path.clone();
        self.panes[0].tab_mut().path = Location::S3 {
            connection: "fixture".into(),
            bucket: "fixture".into(),
            key: String::new(),
            prefix: true,
        };
        assert!(self.terminal_launch().is_err());
        self.command(Command::NewTerminal, window, cx);
        assert!(self.panes[0].tab().terminal.is_none());
        self.panes[0].tab_mut().path = old_location;
        self.notice = None;
        self.activate(second, window, cx);
        let terminal = self.panes[second].tab().terminal.as_ref().unwrap().clone();
        terminal.update(cx, |view, cx| {
            view.send_input(b"printf '\\033[32mREAL PTY READY\\033[0m\\n'; pwd\r", cx)
        });
        self.panes[third]
            .tab()
            .terminal
            .as_ref()
            .unwrap()
            .update(cx, |view, cx| {
                view.send_input(b"printf 'INDEPENDENT SPLIT SESSION\\n'; pwd\r", cx)
            });
        cx.spawn_in(window,async move |this,cx|{
            for _ in 0..100 {
                cx.background_executor().timer(std::time::Duration::from_millis(50)).await;
                let ready=this.update_in(cx,|this,_,cx|{
                    let text=this.panes[second].tab().terminal.as_ref().unwrap().read(cx).snapshot_text();
                    let other=this.panes[third].tab().terminal.as_ref().unwrap().read(cx).snapshot_text();
                    text.lines().any(|line|line=="REAL PTY READY")&&text.replace('\n', "").contains(path.to_string_lossy().as_ref())&&other.lines().any(|line|line=="INDEPENDENT SPLIT SESSION")&&other.replace('\n', "").contains(path.to_string_lossy().as_ref())&&!text.contains("INDEPENDENT SPLIT SESSION")&&!other.lines().any(|line|line=="REAL PTY READY")
                });
                match ready {Ok(true)=>{println!("Pane terminal assertions passed: independent PTYs, retained browser state, session-preserving tab switch, isolated close/end/split ownership.");return;},Err(_)=>return,_=>{}}
            }
            panic!("Pane terminal fixture did not receive independent real shell output");
        }).detach();
    }
}
