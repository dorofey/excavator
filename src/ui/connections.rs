use super::*;
use crate::{connections as store, domain::FsErrorKind};
#[derive(Clone)]
pub(super) enum ConnectionScreen {
    List,
    Import(crate::forklift::ImportPlan),
    Editor(ConnectionRecord),
    Trust {
        record: ConnectionRecord,
        fingerprint: String,
        previous: Option<String>,
    },
    Remove(ConnectionRecord),
    ResetHost(ConnectionRecord),
}
enum TestOutcome {
    Connected(String),
    Trust {
        record: ConnectionRecord,
        fingerprint: String,
        previous: Option<String>,
    },
    Failed(String),
}
const FIELDS: &[&str] = &[
    "Name",
    "Host",
    "Port",
    "Username",
    "Root path / S3 prefix",
    "Bucket",
    "Region",
    "S3 endpoint (optional)",
    "Trusted CA PEM path (optional)",
    "Password (blank keeps stored)",
    "Access key (blank keeps stored)",
    "Secret key (blank keeps stored)",
    "Session token (optional)",
];
impl Workspace {
    pub(super) fn load_connections(&mut self, cx: &mut Context<Self>) {
        let task = cx.background_executor().spawn(async { store::load() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(records) => this.connections = records,
                    Err(error) => this.notice = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn close_connections(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.connection_cancel.cancel();
        self.connection_generation += 1;
        self.connection_busy = false;
        self.connection_screen = None;
        self.clear_connection_secrets(window, cx);
    }
    fn clear_connection_secrets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in &self.connection_inputs[9..] {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }
    }
    pub(super) fn edit_connection(
        &mut self,
        record: Option<ConnectionRecord>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.connection_busy {
            return;
        }
        let record = record.unwrap_or_else(|| ConnectionRecord {
            id: store::new_id(),
            name: String::new(),
            protocol: Protocol::Sftp,
            host: String::new(),
            port: 22,
            username: String::new(),
            root: "/".into(),
            bucket: String::new(),
            region: String::new(),
            endpoint: String::new(),
            ca_bundle: String::new(),
        });
        let values = [
            record.name.clone(),
            record.host.clone(),
            record.port.to_string(),
            record.username.clone(),
            record.root.clone(),
            record.bucket.clone(),
            record.region.clone(),
            record.endpoint.clone(),
            record.ca_bundle.clone(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ];
        for (input, value) in self.connection_inputs.iter().zip(values) {
            input.update(cx, |state, cx| state.set_value(value, window, cx));
        }
        self.connection_screen = Some(ConnectionScreen::Editor(record));
        self.palette = false;
        self.notice = None;
        self.connection_inputs[0].update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
    fn draft_connection(&self, cx: &App) -> Result<ConnectionRecord, String> {
        let Some(ConnectionScreen::Editor(record)) = &self.connection_screen else {
            return Err("Open a connection editor first".into());
        };
        let value = |i: usize| self.connection_inputs[i].read(cx).value().to_string();
        let s3 = record.protocol == Protocol::S3;
        let draft = ConnectionRecord {
            id: record.id.clone(),
            name: value(0),
            protocol: record.protocol,
            host: if s3 { String::new() } else { value(1) },
            port: if s3 {
                443
            } else {
                value(2)
                    .parse()
                    .map_err(|_| "Port must be a number from 1 to 65535")?
            },
            username: if s3 { String::new() } else { value(3) },
            root: value(4),
            bucket: if s3 { value(5) } else { String::new() },
            region: if s3 { value(6) } else { String::new() },
            endpoint: if s3 { value(7) } else { String::new() },
            ca_bundle: if record.protocol == Protocol::Ftps {
                value(8)
            } else {
                String::new()
            },
        };
        draft.validate()?;
        Ok(draft)
    }
    fn entered_secrets(&self, protocol: Protocol, cx: &App) -> Option<ConnectionSecrets> {
        let value = |i: usize| self.connection_inputs[i].read(cx).value().to_string();
        let secret = if protocol == Protocol::S3 {
            ConnectionSecrets {
                password: String::new(),
                access_key: value(10),
                secret_key: value(11),
                session_token: value(12),
            }
        } else {
            ConnectionSecrets {
                password: value(9),
                access_key: String::new(),
                secret_key: String::new(),
                session_token: String::new(),
            }
        };
        if [
            &secret.password,
            &secret.access_key,
            &secret.secret_key,
            &secret.session_token,
        ]
        .iter()
        .all(|s| s.is_empty())
        {
            None
        } else {
            Some(secret)
        }
    }
    fn begin_connection_task(&mut self) -> (u64, CancellationToken) {
        self.connection_cancel.cancel();
        self.connection_cancel = CancellationToken::new();
        self.connection_generation += 1;
        self.connection_busy = true;
        (self.connection_generation, self.connection_cancel.clone())
    }
    pub(super) fn save_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connection_busy {
            return;
        }
        let record = match self.draft_connection(cx) {
            Ok(record) => record,
            Err(error) => {
                self.notice = Some(error);
                cx.notify();
                return;
            }
        };
        let secret = self.entered_secrets(record.protocol, cx);
        self.clear_connection_secrets(window, cx);
        let (generation, _) = self.begin_connection_task();
        let task = cx.background_executor().spawn(async move {
            store::save(&record, &secret)?;
            store::load()
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.connection_generation != generation {
                    return;
                }
                this.connection_busy = false;
                match result {
                    Ok(records) => {
                        this.connections = records;
                        this.connection_screen = Some(ConnectionScreen::List);
                        this.notice = Some(
                            "Connection metadata saved. Any submitted credentials were stored in macOS Keychain.".into(),
                        );
                    }
                    Err(error) => this.notice = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn test_record(
        &mut self,
        record: ConnectionRecord,
        secret: Option<ConnectionSecrets>,
        cx: &mut Context<Self>,
    ) {
        if self.connection_busy {
            return;
        }
        let (generation, cancel) = self.begin_connection_task();
        let task = cx.background_executor().spawn(async move {
            let result = (|| -> Result<TestOutcome, String> {
                record.validate()?;
                if record.protocol == Protocol::Sftp {
                    let fingerprint = crate::providers::probe_host(&record)?;
                    let previous = store::known_host(&record)?;
                    if previous.as_ref() != Some(&fingerprint) {
                        return Ok(TestOutcome::Trust {
                            record,
                            fingerprint,
                            previous,
                        });
                    }
                }
                if cancel.is_cancelled() {
                    return Err("Connection test cancelled".into());
                }
                let secret = match secret {
                    Some(secret) => secret,
                    None => store::secrets(&record.id)?,
                };
                match crate::providers::test_connection(&record, &secret, cancel) {
                    Ok(message) => Ok(TestOutcome::Connected(message)),
                    Err(error) => {
                        let hint = if matches!(
                            error.kind,
                            FsErrorKind::HostKeyChanged | FsErrorKind::HostKeyUnknown
                        ) {
                            " Test again to review the current host fingerprint."
                        } else {
                            ""
                        };
                        Ok(TestOutcome::Failed(format!("{error}{hint}")))
                    }
                }
            })();
            result.unwrap_or_else(TestOutcome::Failed)
        });
        cx.spawn(async move |this, cx| {
            let outcome = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.connection_generation != generation {
                    return;
                }
                this.connection_busy = false;
                match outcome {
                    TestOutcome::Connected(message) => this.notice = Some(format!("{message}. Test credentials were cleared; enter them again before saving if needed.")),
                    TestOutcome::Failed(error) => this.notice = Some(error),
                    TestOutcome::Trust {
                        record,
                        fingerprint,
                        previous,
                    } => {
                        this.connection_screen = Some(ConnectionScreen::Trust {
                            record,
                            fingerprint,
                            previous,
                        });
                        this.notice = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn test_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.draft_connection(cx) {
            Ok(record) => {
                let secret = self.entered_secrets(record.protocol, cx);
                self.clear_connection_secrets(window, cx);
                self.test_record(record, secret, cx)
            }
            Err(error) => {
                self.notice = Some(error);
                cx.notify();
            }
        }
    }
    pub(super) fn connect_record(
        &mut self,
        record: &ConnectionRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let location = match record.protocol {
            Protocol::Sftp => Location::Sftp {
                connection: record.id.clone(),
                path: record.root.clone(),
            },
            Protocol::Ftps => Location::Ftps {
                connection: record.id.clone(),
                path: record.root.clone(),
            },
            Protocol::S3 => Location::S3 {
                connection: record.id.clone(),
                bucket: record.bucket.clone(),
                key: record.root.clone(),
                prefix: true,
            },
        };
        self.close_connections(window, cx);
        self.navigate(self.active, location, true, window, cx);
        self.activate(self.active, window, cx);
        if record.protocol == Protocol::S3 {
            self.notice=Some("S3 folders are key prefixes. Deleting an object is permanent; versioned buckets may create delete markers.".into());
        }
    }
    pub(super) fn active_connection_command(
        &mut self,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let record = self.panes[self.active]
            .tab()
            .path
            .connection_id()
            .and_then(|id| self.connections.iter().find(|r| r.id == id))
            .cloned();
        let Some(record) = record else {
            self.connection_screen = Some(ConnectionScreen::List);
            self.notice = Some("Choose a saved connection first.".into());
            cx.notify();
            return;
        };
        self.palette = false;
        match command {
            Command::TestConnection => self.test_record(record, None, cx),
            Command::EditConnection => self.edit_connection(Some(record), window, cx),
            Command::RemoveConnection => {
                self.connection_screen = Some(ConnectionScreen::Remove(record))
            }
            Command::ResetHost => {
                self.connection_screen = Some(ConnectionScreen::ResetHost(record))
            }
            _ => {}
        }
        cx.notify();
    }
    pub(super) fn update_protocol(
        &mut self,
        protocol: Protocol,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ConnectionScreen::Editor(record)) = &mut self.connection_screen {
            record.protocol = protocol;
            let port = match protocol {
                Protocol::Sftp => "22",
                Protocol::Ftps => "21",
                Protocol::S3 => "443",
            };
            self.connection_inputs[2].update(cx, |input, cx| input.set_value(port, window, cx));
            self.connection_inputs[4].update(cx, |input, cx| {
                input.set_value(if protocol == Protocol::S3 { "" } else { "/" }, window, cx)
            });
            self.clear_connection_secrets(window, cx);
            cx.notify();
        }
    }
    pub(super) fn confirm_connection_action(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.connection_busy {
            return;
        }
        if matches!(self.connection_screen, Some(ConnectionScreen::Import(_))) {
            self.confirm_import(window, cx);
            return;
        }
        let Some(screen) = self.connection_screen.clone() else {
            return;
        };
        let (generation, _) = self.begin_connection_task();
        let task=cx.background_executor().spawn(async move{
            let result:Result<(String,Option<ConnectionRecord>),String>=match screen {
                ConnectionScreen::Trust{record,fingerprint,previous:None}=>{
                    let observed=crate::providers::probe_host(&record)?;
                    if observed!=fingerprint{return Err("Host identity changed while awaiting confirmation. Test again; no trust was saved.".into())}
                    store::trust_host(&record,&fingerprint)?;
                    Ok(("Host fingerprint trusted. Test again. Reenter new credentials if they were not saved yet.".into(),Some(record)))
                }
                ConnectionScreen::Trust{previous:Some(_),..}=>Err("Changed host identity cannot be trusted until you explicitly reset the previous trust.".into()),
                ConnectionScreen::Remove(record)=>{store::remove(&record.id)?;Ok(("Connection and its stored credentials removed.".into(),None))}
                ConnectionScreen::ResetHost(record)=>{store::forget_host(&record)?;Ok(("Previous host trust removed. Test again and independently verify the new fingerprint before trusting it.".into(),None))}
                _=>Err("No confirmation is pending".into()),
            };
            result.and_then(|(message,editor)|Ok((message,store::load()?,editor)))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.connection_generation != generation {
                    return;
                }
                this.connection_busy = false;
                match result {
                    Ok((message, records, editor)) => {
                        this.connections = records;
                        if let Some(record) = editor {
                            this.edit_connection(Some(record), window, cx)
                        } else {
                            this.connection_screen = Some(ConnectionScreen::List)
                        }
                        this.notice = Some(message);
                    }
                    Err(error) => this.notice = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn render_connections(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.colors;
        let screen = self.connection_screen.as_ref().unwrap();
        let mut panel = div()
            .id("connections-panel")
            .max_h(px(330.))
            .flex_none()
            .overflow_y_scroll()
            .track_scroll(&self.connection_scroll)
            .when(matches!(screen, ConnectionScreen::Editor(_)), |d| {
                d.key_context("ConnectionEditor")
            })
            .p_3()
            .bg(rgb(theme.surface))
            .border_t_1()
            .border_color(rgb(theme.accent))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_color(rgb(theme.accent))
                            .child("Connections · ⌘⇧C"),
                    )
                    .child(
                        div()
                            .id("close-connections")
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_connections(window, cx);
                                this.activate(this.active, window, cx);
                            }))
                            .child("Close · Esc"),
                    ),
            );
        if self.connection_busy {
            return panel
                .child("Working… Closing cancels the test; an in-flight save may still finish.");
        }
        match screen {
            ConnectionScreen::Import(plan) => {
                panel = panel.child(format!("{} connections ready · {} duplicates · {} non-connections ignored · {} unsupported or invalid", plan.candidates.len(), plan.duplicates, plan.ignored, plan.skipped.len()))
                    .child("Import adds connection settings only. Passwords, keys and SSH trust stay in ForkLift; edit imported connections to provide credentials before connecting.");
                for (index, record) in plan.candidates.iter().enumerate() {
                    let selected = plan.selected[index];
                    panel = panel.child(
                        Button::new(("import-select", index))
                            .ghost()
                            .selected(selected)
                            .toggled(selected)
                            .label(format!(
                                "{} {} · {}:{} · {} · {}",
                                if selected { "☑" } else { "☐" },
                                record.name,
                                record.host,
                                record.port,
                                record.username,
                                record.root
                            ))
                            .accessibility_label(format!(
                                "{} {} for import",
                                if selected { "Deselect" } else { "Select" },
                                record.name
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(ConnectionScreen::Import(plan)) =
                                    &mut this.connection_screen
                                {
                                    plan.selected[index] = !plan.selected[index];
                                }
                                cx.notify();
                            })),
                    );
                }
                for reason in plan.skipped.iter().take(20) {
                    panel = panel.child(reason.clone());
                }
                panel = panel
                    .child(
                        Button::new("confirm-import")
                            .label("Import selected connections")
                        .tooltip("Import selected connections · Command palette: Confirm pending connection action")
                            .disabled(!plan.selected.iter().any(|selected| *selected))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.confirm_import(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("browse-import-again")
                            .ghost()
                            .label("Choose another ForkLift database…")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.browse_import(window, cx)),
                            ),
                    );
            }
            ConnectionScreen::List => {
                panel = panel
                    .child(
                        Button::new("import-forklift")
                            .ghost()
                            .label("Import from ForkLift…")
                            .tooltip("Import ForkLift connection settings · ⌘⌥I")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.begin_import(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("browse-forklift")
                            .ghost()
                            .label("Choose ForkLift database…")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.browse_import(window, cx)),
                            ),
                    );
                panel =
                    panel.child(
                        div()
                            .id("add-connection")
                            .py_2()
                            .cursor_pointer()
                            .text_color(rgb(theme.accent))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.edit_connection(None, window, cx)
                            }))
                            .child("Add connection"),
                    );
                if self.connections.is_empty() {
                    panel = panel.child("No saved connections. Add SFTP, secure FTPS or S3.");
                }
                panel =
                    panel.children(self.connections.iter().enumerate().map(|(index, record)| {
                        let record = record.clone();
                        let mut row = div()
                            .py_2()
                            .border_b_1()
                            .border_color(rgb(theme.border))
                            .child(format!("{} · {:?}", record.name, record.protocol));
                        let actions = ["Connect", "Edit", "Test", "Remove", "Reset host trust"];
                        row = row.child(
                            div().flex().gap_3().children(
                                actions
                                    .into_iter()
                                    .enumerate()
                                    .filter(|(i, _)| *i != 4 || record.protocol == Protocol::Sftp)
                                    .map(|(action, label)| {
                                        let record = record.clone();
                                        div()
                                            .id(("connection-action", index * 10 + action))
                                            .cursor_pointer()
                                            .text_color(rgb(theme.accent))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                match action {
                                                    0 => this.connect_record(&record, window, cx),
                                                    1 => this.edit_connection(
                                                        Some(record.clone()),
                                                        window,
                                                        cx,
                                                    ),
                                                    2 => this.test_record(record.clone(), None, cx),
                                                    3 => {
                                                        this.connection_screen =
                                                            Some(ConnectionScreen::Remove(
                                                                record.clone(),
                                                            ))
                                                    }
                                                    _ => {
                                                        this.connection_screen =
                                                            Some(ConnectionScreen::ResetHost(
                                                                record.clone(),
                                                            ))
                                                    }
                                                }
                                                cx.notify();
                                            }))
                                            .child(label)
                                    }),
                            ),
                        );
                        row
                    }));
            }
            ConnectionScreen::Editor(record) => {
                let protocol = record.protocol;
                panel = panel.child(
                    div().py_2().flex().gap_4().children(
                        [
                            (Protocol::Sftp, "SFTP"),
                            (Protocol::Ftps, "FTPS · TLS required"),
                            (Protocol::S3, "S3"),
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(id, (value, label))| {
                            div()
                                .id(("connection-protocol", id))
                                .cursor_pointer()
                                .text_color(rgb(if protocol == value {
                                    theme.accent
                                } else {
                                    theme.muted
                                }))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.update_protocol(value, window, cx)
                                }))
                                .child(label)
                        }),
                    ),
                );
                let indices = if protocol == Protocol::S3 {
                    vec![0, 4, 5, 6, 7, 10, 11, 12]
                } else {
                    if protocol == Protocol::Ftps {
                        vec![0, 1, 2, 3, 4, 8, 9]
                    } else {
                        vec![0, 1, 2, 3, 4, 9]
                    }
                };
                panel=panel.children(indices.into_iter().map(|i|div().py_1().flex().items_center().gap_3().child(div().w(px(210.)).flex_none().child(FIELDS[i])).child(div().flex_1().child(Input::new(&self.connection_inputs[i]))))).child(div().py_2().text_color(rgb(theme.muted)).child("Blank credential fields preserve the existing Keychain entry. Remote pane state stays in this session."));
                panel = panel.child(
                    div()
                        .flex()
                        .gap_4()
                        .child(
                            div()
                                .id("save-connection")
                                .cursor_pointer()
                                .text_color(rgb(theme.accent))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_connection(window, cx)
                                }))
                                .child("Save connection"),
                        )
                        .child(
                            div()
                                .id("test-draft-connection")
                                .cursor_pointer()
                                .text_color(rgb(theme.accent))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.test_draft(window, cx)),
                                )
                                .child("Test connection"),
                        ),
                );
            }
            ConnectionScreen::Trust {
                record,
                fingerprint,
                previous,
            } => {
                panel = panel
                    .child(format!("SSH identity for {}:{}", record.host, record.port))
                    .child(format!("Presented: {fingerprint}"));
                if let Some(previous) = previous {
                    let record = record.clone();
                    panel=panel.child(format!("Previously trusted: {previous}")).child("Host identity changed. Authentication was stopped. Verify this change independently before resetting trust.").child(div().id("review-reset-host").py_2().cursor_pointer().text_color(rgb(theme.warning)).on_click(cx.listener(move|this,_,_,cx|{this.connection_screen=Some(ConnectionScreen::ResetHost(record.clone()));cx.notify();})).child("Review reset of previous trust…"));
                } else {
                    panel=panel.child("Compare the SHA256 fingerprint with the server administrator through a trusted channel before accepting.").child(div().id("accept-host").py_2().cursor_pointer().text_color(rgb(theme.accent)).on_click(cx.listener(|this,_,window,cx|this.confirm_connection_action(window,cx))).child("Trust this fingerprint"));
                }
            }
            ConnectionScreen::Remove(record) => {
                panel = panel
                    .child(format!(
                        "Remove {} and its Keychain credentials?",
                        record.name
                    ))
                    .child("Files on the remote server are unaffected.")
                    .child(
                        div()
                            .id("confirm-remove-connection")
                            .py_2()
                            .cursor_pointer()
                            .text_color(rgb(theme.warning))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_connection_action(window, cx)
                            }))
                            .child("Confirm removal"),
                    );
            }
            ConnectionScreen::ResetHost(record) => {
                panel=panel.child(format!("Forget the trusted SSH fingerprint for {}:{}?",record.host,record.port)).child("The next test will require a new fingerprint review. This does not accept a replacement key.").child(div().id("confirm-reset-host").py_2().cursor_pointer().text_color(rgb(theme.warning)).on_click(cx.listener(|this,_,window,cx|this.confirm_connection_action(window,cx))).child("Confirm forget trusted key"));
            }
        }
        panel
    }
}
impl Workspace {
    pub(super) fn saved_connection_command(
        &mut self,
        index: usize,
        action: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(record) = self.connections.get(index).cloned() else {
            return;
        };
        self.palette = false;
        match action {
            0 => self.connect_record(&record, window, cx),
            1 => self.edit_connection(Some(record), window, cx),
            2 => self.test_record(record, None, cx),
            3 => {
                self.clear_connection_secrets(window, cx);
                self.connection_screen = Some(ConnectionScreen::Remove(record))
            }
            _ => {
                self.clear_connection_secrets(window, cx);
                self.connection_screen = Some(ConnectionScreen::ResetHost(record))
            }
        }
        cx.notify();
    }
}
impl Workspace {
    pub(super) fn move_connection_focus(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ConnectionScreen::Editor(record)) = &self.connection_screen else {
            return;
        };
        let fields = if record.protocol == Protocol::S3 {
            vec![0, 4, 5, 6, 7, 10, 11, 12]
        } else if record.protocol == Protocol::Ftps {
            vec![0, 1, 2, 3, 4, 8, 9]
        } else {
            vec![0, 1, 2, 3, 4, 9]
        };
        let current = fields
            .iter()
            .position(|i| {
                self.connection_inputs[*i]
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            })
            .unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(fields.len() as isize) as usize;
        self.connection_inputs[fields[next]].update(cx, |input, cx| input.focus(window, cx));
        self.connection_scroll.scroll_to_item(next + 2);
        cx.notify();
    }
}

impl Workspace {
    pub(super) fn begin_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connection_busy {
            return;
        }
        self.connection_screen = Some(ConnectionScreen::List);
        let paths = crate::forklift::default_paths();
        if let Some(path) = paths.into_iter().next() {
            self.read_import(path, cx);
        } else {
            self.browse_import(window, cx);
        }
    }
    fn browse_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connection_busy {
            return;
        }
        let generation = self.connection_generation;
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose ForkLift Favorites.sqlite".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = picker.await;
            let _ = this.update_in(cx, |this, _, cx| {
                if generation != this.connection_generation { return; }
                match result {
                    Ok(Ok(Some(paths))) => { if let Some(path) = paths.into_iter().next() { this.read_import(path, cx); } },
                    Ok(Ok(None)) => {},
                    _ => { this.notice=Some("Unable to choose ForkLift database. Retry using Choose ForkLift database.".into()); cx.notify(); },
                }
            });
        }).detach();
    }
    fn read_import(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let existing = self.connections.clone();
        let (generation, _) = self.begin_connection_task();
        let task = cx
            .background_executor()
            .spawn(async move { crate::forklift::read(&path, &existing) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.connection_generation != generation {
                    return;
                }
                this.connection_busy = false;
                match result {
                    Ok(plan) => {
                        this.connection_screen = Some(ConnectionScreen::Import(plan));
                        this.notice = None;
                    }
                    Err(error) => {
                        this.connection_screen = Some(ConnectionScreen::List);
                        this.notice = Some(format!(
                            "{error}. Use Choose ForkLift database to select an accessible copy."
                        ));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn confirm_import(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.connection_busy {
            return;
        }
        let Some(ConnectionScreen::Import(plan)) = &self.connection_screen else {
            return;
        };
        let records = plan
            .candidates
            .iter()
            .zip(&plan.selected)
            .filter(|(_, selected)| **selected)
            .map(|(record, _)| record.clone())
            .collect::<Vec<_>>();
        if records.is_empty() {
            return;
        }
        let (generation, _) = self.begin_connection_task();
        let task = cx
            .background_executor()
            .spawn(async move { store::import_metadata(&records) });
        cx.spawn(async move |this,cx| {
            let result=task.await;
            let _=this.update(cx,|this,cx| {
                if this.connection_generation != generation {return;}
                this.connection_busy=false;
                match result { Ok(outcome)=>{this.connections=outcome.records;this.connection_screen=Some(ConnectionScreen::List);this.notice=Some(format!("Imported {} connection settings; skipped {} duplicates. Edit imported connections to provide credentials. No passwords or SSH trust were imported.",outcome.added,outcome.duplicates));},Err(error)=>this.notice=Some(error) }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
}
