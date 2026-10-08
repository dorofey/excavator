//! Connection overlays. Blocking metadata, Keychain and network work stays off
//! the terminal event loop; at most one worker is active.
use super::{
    connection_service as service,
    modal::{Tone, input, panel},
    view::display_os,
};
use crate::{
    connections::{self, ConnectionRecord, Protocol},
    domain::Location,
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use std::{
    ffi::OsStr,
    sync::mpsc::{self, Receiver},
};

const LABELS: [&str; 17] = [
    "Name",
    "Protocol (sftp/ftps/s3)",
    "Host",
    "Port",
    "User",
    "Root",
    "Bucket",
    "Region",
    "Endpoint",
    "CA bundle",
    "Group",
    "SSH key path",
    "Password",
    "Access key",
    "Secret key",
    "Session token",
    "SSH passphrase",
];
const FIELDS: usize = 17;
struct Form {
    original: Option<ConnectionRecord>,
    id: String,
    values: Vec<String>,
    clear: [bool; 5],
    index: usize,
}
impl Form {
    fn new(record: Option<ConnectionRecord>) -> Self {
        let id = record
            .as_ref()
            .map(|r| r.id.clone())
            .unwrap_or_else(connections::new_id);
        let values = if let Some(r) = &record {
            vec![
                r.name.clone(),
                format!("{:?}", r.protocol).to_lowercase(),
                r.host.clone(),
                r.port.to_string(),
                r.username.clone(),
                r.root.clone(),
                r.bucket.clone(),
                r.region.clone(),
                r.endpoint.clone(),
                r.ca_bundle.clone(),
                r.group.clone(),
                r.ssh_key_path.clone(),
            ]
        } else {
            vec![
                String::new(),
                "sftp".into(),
                String::new(),
                "22".into(),
                String::new(),
                "/".into(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ]
        };
        let mut values = values;
        values.resize(FIELDS, String::new());
        Self {
            original: record,
            id,
            values,
            clear: [false; 5],
            index: 0,
        }
    }
    fn record(&self) -> Result<ConnectionRecord, String> {
        let v = &self.values;
        let protocol = match v[1].trim().to_ascii_lowercase().as_str() {
            "sftp" => Protocol::Sftp,
            "ftps" => Protocol::Ftps,
            "s3" => Protocol::S3,
            _ => return Err("Protocol must be sftp, ftps, or s3".into()),
        };
        let record = ConnectionRecord {
            id: self.id.clone(),
            name: v[0].clone(),
            protocol,
            host: v[2].clone(),
            port: v[3]
                .parse()
                .map_err(|_| "Port must be a number from 0 to 65535")?,
            username: v[4].clone(),
            root: v[5].clone(),
            bucket: v[6].clone(),
            region: v[7].clone(),
            endpoint: v[8].clone(),
            ca_bundle: v[9].clone(),
            group: v[10].clone(),
            ssh_key_path: v[11].clone(),
        };
        record.validate()?;
        Ok(record)
    }
}
enum Screen {
    Closed,
    Groups,
    GroupEdit { old: Option<String>, value: String },
    GroupRemove(String),
    Picker,
    Form(Form),
    Save(Form),
    Remove(ConnectionRecord),
    Trust(service::HostReview),
    Reset(service::HostReview),
    Busy,
    Error(String),
}
enum Reply {
    Records(Vec<ConnectionRecord>),
    Groups(Vec<String>, Vec<ConnectionRecord>, Option<String>),
    Navigate(Location),
    Host(Box<service::HostReview>),
    Notice(String),
}
pub struct ConnectionUi {
    screen: Screen,
    records: Vec<ConnectionRecord>,
    index: usize,
    groups: Vec<String>,
    selected_group: Option<String>,
    group_error: bool,
    worker: Option<Receiver<Result<Reply, String>>>,
    ignored: bool,
    approved: bool,
    dirty: bool,
    error_back: Option<Form>,
    scroll: u16,
}
impl Default for ConnectionUi {
    fn default() -> Self {
        Self::new()
    }
}
impl ConnectionUi {
    pub fn new() -> Self {
        Self {
            screen: Screen::Closed,
            records: vec![],
            index: 0,
            groups: vec![],
            selected_group: None,
            group_error: false,
            worker: None,
            ignored: false,
            approved: false,
            dirty: false,
            error_back: None,
            scroll: 0,
        }
    }
    pub fn active(&self) -> bool {
        !matches!(self.screen, Screen::Closed)
    }
    fn task(
        &mut self,
        approved: bool,
        job: impl FnOnce() -> Result<Reply, String> + Send + 'static,
    ) {
        if self.worker.is_some() {
            // Keep the existing task's dismissal/approval state. Reopening
            // must not allow a late cancelled result to navigate the browser.
            self.screen = Screen::Busy;
            self.dirty = true;
            return;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        self.worker = Some(rx);
        self.ignored = false;
        self.approved = approved;
        self.screen = Screen::Busy;
        self.dirty = true;
        let spawn = std::thread::Builder::new()
            .name("tui-connection".into())
            .spawn(move || {
                let result = job();
                let _ = tx.send(result);
            });
        if spawn.is_err() {
            self.worker = None;
            self.screen = Screen::Error("Cannot start connection worker".into());
        }
    }
    pub fn open(&mut self) {
        self.group_error = false;
        if self.worker.is_none() {
            self.task(false, || Ok(Reply::Records(service::load_saved()?)));
        } else {
            self.screen = Screen::Busy;
            self.dirty = true;
        }
    }
    pub fn open_groups(&mut self) {
        self.group_error = true;
        self.task(false, || {
            Ok(Reply::Groups(
                connections::load_groups()?,
                service::load_saved()?,
                None,
            ))
        });
    }
    pub fn probe(&mut self, id: String) {
        self.task(false, move || connect(&id));
    }
    pub fn poll(&mut self) -> (Option<Location>, Option<String>, bool) {
        let mut navigate = None;
        let mut notice = None;
        let result = self.worker.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("Connection worker stopped".into())),
            Err(mpsc::TryRecvError::Empty) => None,
        });
        if let Some(result) = result {
            self.worker = None;
            self.dirty = true;
            if !self.ignored || self.approved {
                match result {
                    Ok(Reply::Records(records)) => {
                        self.records = records;
                        self.index = self.index.min(self.records.len().saturating_sub(1));
                        self.screen = Screen::Picker;
                    }
                    Ok(Reply::Groups(groups, records, selected)) => {
                        self.groups = groups;
                        self.records = records;
                        self.selected_group = selected
                            .or_else(|| self.selected_group.clone())
                            .filter(|name| self.groups.contains(name))
                            .or_else(|| self.groups.first().cloned());
                        self.screen = Screen::Groups;
                    }
                    Ok(Reply::Navigate(location)) => {
                        navigate = Some(location);
                        self.screen = Screen::Closed;
                    }
                    Ok(Reply::Host(review)) => self.screen = Screen::Trust(*review),
                    Ok(Reply::Notice(text)) => {
                        notice = Some(text);
                        self.screen = Screen::Closed;
                    }
                    Err(error) => self.screen = Screen::Error(error),
                }
            } else {
                self.screen = Screen::Closed;
            }
            self.approved = false;
        }
        let redraw = std::mem::take(&mut self.dirty);
        (navigate, notice, redraw)
    }
    pub fn key(&mut self, key: KeyEvent) {
        if !self.active() || key.kind == KeyEventKind::Release {
            return;
        }
        let editing = matches!(self.screen, Screen::Form(_) | Screen::GroupEdit { .. });
        // A held Enter must not both open and confirm a destructive review.
        if key.kind != KeyEventKind::Press
            && (!editing
                || matches!(key.code, KeyCode::Enter | KeyCode::Esc)
                || (key.code == KeyCode::Char('d')
                    && key.modifiers.contains(KeyModifiers::CONTROL)))
        {
            return;
        }
        let allowed_control =
            key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL;
        if !(editing || key.modifiers.is_empty() || allowed_control) {
            return;
        }
        if editing && matches!(key.code, KeyCode::Enter | KeyCode::Esc) && !key.modifiers.is_empty()
        {
            return;
        }
        self.dirty = true;
        if key.code == KeyCode::PageDown {
            self.scroll = self.scroll.saturating_add(4);
            return;
        }
        if key.code == KeyCode::PageUp {
            self.scroll = self.scroll.saturating_sub(4);
            return;
        }
        if key.code == KeyCode::Esc
            || (key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL)
        {
            if matches!(self.screen, Screen::Busy) && self.approved {
                return;
            }
            self.ignored = true;
            self.screen = match self.screen {
                Screen::Form(_) | Screen::Save(_) | Screen::Remove(_) | Screen::Reset(_) => {
                    Screen::Picker
                }
                Screen::GroupEdit { .. } | Screen::GroupRemove(_) => Screen::Groups,
                Screen::Groups => Screen::Picker,
                Screen::Error(_) if self.group_error => Screen::Groups,
                Screen::Error(_) => self
                    .error_back
                    .take()
                    .map(Screen::Form)
                    .unwrap_or(Screen::Picker),
                _ => Screen::Closed,
            };
            return;
        }
        let screen = std::mem::replace(&mut self.screen, Screen::Closed);
        self.screen = match screen {
            Screen::Groups => {
                let index = self
                    .selected_group
                    .as_ref()
                    .and_then(|name| self.groups.iter().position(|g| g == name))
                    .unwrap_or(0);
                match key.code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.selected_group = self
                            .groups
                            .get((index + 1).min(self.groups.len().saturating_sub(1)))
                            .cloned()
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.selected_group = self.groups.get(index.saturating_sub(1)).cloned()
                    }
                    KeyCode::Char('n') => {
                        return self.set(Screen::GroupEdit {
                            old: None,
                            value: String::new(),
                        });
                    }
                    KeyCode::Char('e') | KeyCode::Enter => {
                        if let Some(name) = self.selected_group.clone() {
                            return self.set(Screen::GroupEdit {
                                old: Some(name.clone()),
                                value: name,
                            });
                        }
                    }
                    KeyCode::Char('d') => {
                        if let Some(name) = self.selected_group.clone() {
                            return self.set(Screen::GroupRemove(name));
                        }
                    }
                    _ => {}
                }
                Screen::Groups
            }
            Screen::GroupEdit { old, mut value } => {
                match key.code {
                    KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => value.clear(),
                    KeyCode::Backspace => {
                        value.pop();
                    }
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                            && !c.is_control() =>
                    {
                        if value.len() + c.len_utf8() <= 128 {
                            value.push(c);
                        }
                    }
                    KeyCode::Enter => {
                        self.group_error = true;
                        self.task(true, move || {
                            let (groups, records) = if let Some(old) = old {
                                connections::rename_group(&old, &value)?
                            } else {
                                (connections::save_group(&value)?, service::load_saved()?)
                            };
                            Ok(Reply::Groups(groups, records, Some(value)))
                        });
                        return;
                    }
                    _ => {}
                }
                Screen::GroupEdit { old, value }
            }
            Screen::GroupRemove(name) => {
                if key.code == KeyCode::Enter {
                    self.group_error = true;
                    self.task(true, move || {
                        let (groups, records) = connections::delete_group(&name)?;
                        Ok(Reply::Groups(groups, records, None))
                    });
                    return;
                }
                Screen::GroupRemove(name)
            }
            Screen::Picker => {
                self.group_error = false;
                match key.code {
                    KeyCode::Char('g') => {
                        self.open_groups();
                        return;
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.index = (self.index + 1).min(self.records.len().saturating_sub(1))
                    }
                    KeyCode::Up | KeyCode::Char('k') => self.index = self.index.saturating_sub(1),
                    KeyCode::Char('n') => return self.set(Screen::Form(Form::new(None))),
                    KeyCode::Char('e') => {
                        if let Some(record) = self.records.get(self.index) {
                            return self.set(Screen::Form(Form::new(Some(record.clone()))));
                        }
                    }
                    KeyCode::Char('d') => {
                        if let Some(record) = self.records.get(self.index) {
                            return self.set(Screen::Remove(record.clone()));
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(record) = self.records.get(self.index) {
                            let id = record.id.clone();
                            self.probe(id);
                            return;
                        }
                    }
                    _ => {}
                }
                Screen::Picker
            }
            Screen::Form(mut form) => {
                match key.code {
                    KeyCode::Tab | KeyCode::Down => form.index = (form.index + 1) % FIELDS,
                    KeyCode::BackTab | KeyCode::Up => {
                        form.index = (form.index + FIELDS - 1) % FIELDS
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        form.values[form.index].clear()
                    }
                    KeyCode::Char('d')
                        if key.modifiers.contains(KeyModifiers::CONTROL) && form.index >= 12 =>
                    {
                        form.clear[form.index - 12] = !form.clear[form.index - 12];
                        form.values[form.index].clear();
                    }
                    KeyCode::Backspace => {
                        form.values[form.index].pop();
                    }
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                            && !c.is_control() =>
                    {
                        if form.values[form.index].len() + c.len_utf8() <= 8192 {
                            form.values[form.index].push(c);
                            if form.index >= 12 {
                                form.clear[form.index - 12] = false;
                            }
                        }
                    }
                    KeyCode::Enter => {
                        if let Err(error) = form.record() {
                            self.screen = Screen::Form(form);
                            return self.set_error(error);
                        }
                        return self.set(Screen::Save(form));
                    }
                    _ => {}
                }
                Screen::Form(form)
            }
            Screen::Save(form) => {
                if key.code == KeyCode::Enter {
                    self.task(true, move || save_form(form));
                    return;
                }
                Screen::Save(form)
            }
            Screen::Remove(record) => {
                if key.code == KeyCode::Enter {
                    self.task(true, move || {
                        connections::remove_if_matches(&record)?;
                        Ok(Reply::Notice(
                            "Connection and stored credentials removed".into(),
                        ))
                    });
                    return;
                }
                Screen::Remove(record)
            }
            Screen::Trust(review) => {
                if review.previous.is_some()
                    && review.previous.as_ref() != Some(&review.fingerprint)
                {
                    if key.code == KeyCode::Char('f') {
                        return self.set(Screen::Reset(review));
                    }
                } else if key.code == KeyCode::Enter {
                    self.task(true, move || {
                        service::trust_review(&review)?;
                        connect(&review.record.id)
                    });
                    return;
                }
                Screen::Trust(review)
            }
            Screen::Reset(review) => {
                if key.code == KeyCode::Enter {
                    self.task(true, move || {
                        ensure_record(&review.record)?;
                        let previous = review
                            .previous
                            .as_deref()
                            .ok_or("No reviewed trusted key to reset")?;
                        connections::forget_host_if_matches(&review.record, previous)?;
                        Ok(Reply::Host(Box::new(service::probe(&review.record.id)?)))
                    });
                    return;
                }
                Screen::Reset(review)
            }
            other => other,
        };
    }
    fn set(&mut self, screen: Screen) {
        self.screen = screen;
        self.scroll = 0;
        self.dirty = true;
    }
    /// Paste never submits a form or invokes an action. Controls are excluded.
    pub fn paste(&mut self, text: &str) {
        if let Screen::GroupEdit { value, .. } = &mut self.screen {
            for c in text.chars().filter(|c| !c.is_control()) {
                if value.len() + c.len_utf8() <= 128 {
                    value.push(c);
                }
            }
            self.dirty = true;
            return;
        }
        let Screen::Form(form) = &mut self.screen else {
            return;
        };
        let field = &mut form.values[form.index];
        let before = field.len();
        for character in text.chars().filter(|character| !character.is_control()) {
            if field.len() + character.len_utf8() > 8192 {
                break;
            }
            field.push(character);
        }
        if field.len() != before {
            if form.index >= 12 {
                form.clear[form.index - 12] = false;
            }
            self.dirty = true;
        }
    }
    fn set_error(&mut self, error: String) {
        let previous = std::mem::replace(&mut self.screen, Screen::Error(error));
        if let Screen::Form(form) = previous {
            self.error_back = Some(form);
        }
    }
    pub fn render(&self, frame: &mut Frame) {
        if !self.active() {
            return;
        }
        let muted = Style::default().fg(Color::DarkGray);
        let accent = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        let danger = Style::default().fg(Color::Red);
        let mut lines = Vec::new();
        let mut selected = None;
        let mut active_input = None;
        let (title, footer, tone) = match &self.screen {
            Screen::Groups => {
                lines.push(Line::styled(
                    "Groups organize saved connections; credentials stay unchanged.",
                    muted,
                ));
                lines.push(Line::default());
                if self.groups.is_empty() {
                    lines.push(Line::raw("No groups yet. Press n to create one."));
                }
                for (i, name) in self.groups.iter().enumerate() {
                    let active = self.selected_group.as_ref() == Some(name);
                    let members = self.records.iter().filter(|r| r.group == *name).count();
                    lines.push(Line::styled(
                        format!(
                            "{} {} · {} connections",
                            if active { "›" } else { " " },
                            safe(name),
                            members
                        ),
                        if active {
                            Style::default()
                                .fg(Color::Black)
                                .bg(Color::Cyan)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        },
                    ));
                    if active {
                        selected = Some(i + 2);
                    }
                }
                (
                    "Connection groups",
                    "↑↓ choose · n new · e rename · d remove · Esc connections",
                    Tone::Normal,
                )
            }
            Screen::GroupEdit { old, value } => {
                lines.push(Line::styled(
                    if old.is_some() {
                        "Renaming also updates every connection in this group."
                    } else {
                        "Assign connections through their Group field in the connection editor."
                    },
                    muted,
                ));
                lines.push(Line::default());
                lines.push(Line::raw("Name: "));
                active_input = Some((2, 6, value.clone()));
                (
                    if old.is_some() {
                        "Rename group"
                    } else {
                        "New group"
                    },
                    "Enter save · Ctrl+U erase · Esc cancel",
                    Tone::Normal,
                )
            }
            Screen::GroupRemove(name) => {
                lines.push(Line::styled(safe(name), accent));
                lines.push(Line::default());
                lines.push(Line::raw(
                    "Remove this group? Its connections become Ungrouped.",
                ));
                lines.push(Line::raw(
                    "Saved connections and credentials are preserved.",
                ));
                (
                    "Remove group",
                    "Enter remove group · Esc cancel",
                    Tone::Danger,
                )
            }
            Screen::Picker => {
                lines.push(Line::styled(
                    format!("{} saved connections", self.records.len()),
                    muted,
                ));
                lines.push(Line::default());
                if self.records.is_empty() {
                    lines.push(Line::raw("No connections yet. Press n to create one."));
                }
                for (i, r) in self.records.iter().enumerate() {
                    let active = i == self.index;
                    let style = if active {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().add_modifier(Modifier::BOLD)
                    };
                    lines.push(Line::styled(
                        format!(
                            "{} {}  {:?} · {}",
                            if active { "›" } else { " " },
                            safe(&r.name),
                            r.protocol,
                            if r.group.is_empty() {
                                "Ungrouped".into()
                            } else {
                                safe(&r.group)
                            }
                        ),
                        style,
                    ));
                    lines.push(Line::styled(
                        format!("  {}", safe(&service::location_for(r).display())),
                        if active { style } else { muted },
                    ));
                }
                let selected_start = self.index * 2 + 2;
                let selected_width = usize::from(
                    frame
                        .area()
                        .width
                        .saturating_sub(if frame.area().width > 8 { 4 } else { 0 })
                        .min(86)
                        .saturating_sub(4),
                );
                for line in lines.iter_mut().skip(selected_start).take(2) {
                    line.spans.push(Span::raw(
                        " ".repeat(selected_width.saturating_sub(line.width())),
                    ));
                }
                selected = Some(self.index * 2 + 2);
                (
                    "Connections",
                    "↑↓ choose · Enter connect · n new · e edit · d remove · g groups · Esc close",
                    Tone::Normal,
                )
            }
            Screen::Form(form) => {
                lines.push(Line::styled(
                    "Blank secret fields preserve stored credentials.",
                    muted,
                ));
                lines.push(Line::default());
                for (i, label) in LABELS.iter().enumerate() {
                    let value = if i >= 12 {
                        if form.clear[i - 12] {
                            "[clear on save]".into()
                        } else if form.values[i].is_empty() {
                            "[keep stored value]".into()
                        } else {
                            "•".repeat(form.values[i].chars().count().min(40))
                        }
                    } else {
                        safe(&form.values[i])
                    };
                    let active = i == form.index;
                    let prefix_width = Line::raw(format!("› {label}: ")).width();
                    let available = frame.area().width.saturating_sub(8).min(82) as usize;
                    if active {
                        active_input = Some((
                            i + 2,
                            prefix_width,
                            if i >= 12 {
                                value.clone()
                            } else {
                                form.values[i].clone()
                            },
                        ));
                    }
                    let mut visible_value: String = value
                        .chars()
                        .take(available.saturating_sub(prefix_width).max(1))
                        .collect();
                    while Line::raw(&visible_value).width()
                        > available.saturating_sub(prefix_width).max(1)
                    {
                        visible_value.pop();
                    }
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("{} {}", if active { "›" } else { " " }, label),
                            if active { accent } else { muted },
                        ),
                        Span::raw(": "),
                        Span::styled(
                            if visible_value.is_empty() {
                                " ".into()
                            } else {
                                visible_value
                            },
                            if active {
                                Style::default().add_modifier(Modifier::REVERSED)
                            } else {
                                Style::default()
                            },
                        ),
                    ]));
                }
                selected = Some(form.index + 2);
                (
                    if form.original.is_some() {
                        "Edit connection"
                    } else {
                        "New connection"
                    },
                    "Tab/↑↓ field · Ctrl+U erase · Ctrl+D clear secret · Enter review · Esc discard",
                    Tone::Normal,
                )
            }
            Screen::Save(form) => {
                lines.push(Line::styled("Review settings before saving", accent));
                lines.push(Line::styled(
                    "Credentials are stored in macOS Keychain.",
                    muted,
                ));
                lines.push(Line::default());
                for (i, label) in LABELS.iter().enumerate().take(12) {
                    if !form.values[i].is_empty() {
                        lines.push(Line::from(vec![
                            Span::styled(format!("{label}: "), muted),
                            Span::raw(safe(&form.values[i])),
                        ]));
                    }
                }
                for (i, label) in LABELS.iter().enumerate().skip(12) {
                    if form.clear[i - 12] || !form.values[i].is_empty() {
                        lines.push(Line::styled(
                            format!(
                                "{}: {}",
                                label,
                                if form.clear[i - 12] {
                                    "CLEAR"
                                } else {
                                    "REPLACE"
                                }
                            ),
                            if form.clear[i - 12] { danger } else { accent },
                        ));
                    }
                }
                (
                    "Save connection",
                    "Enter save · Esc discard · PgUp/PgDn scroll",
                    Tone::Normal,
                )
            }
            Screen::Remove(r) => {
                lines.push(Line::styled(
                    safe(&r.name),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                lines.push(Line::styled(
                    safe(&service::location_for(r).display()),
                    muted,
                ));
                lines.push(Line::default());
                lines.push(Line::styled(
                    "Remove this connection and its stored credentials?",
                    danger,
                ));
                (
                    "Remove connection",
                    "Enter remove · Esc cancel",
                    Tone::Danger,
                )
            }
            Screen::Trust(r) | Screen::Reset(r) => {
                let changed = r.previous.is_some() && r.previous.as_ref() != Some(&r.fingerprint);
                lines.push(Line::styled(
                    format!(
                        "{}:{} · {}",
                        safe(&r.record.host),
                        r.record.port,
                        safe(&r.record.name)
                    ),
                    accent,
                ));
                lines.push(Line::default());
                if let Some(previous) = &r.previous {
                    lines.push(Line::styled("Previously trusted", muted));
                    lines.push(Line::raw(safe(previous)));
                    lines.push(Line::default());
                }
                lines.push(Line::styled("Observed fingerprint", muted));
                lines.push(Line::styled(
                    safe(&r.fingerprint),
                    if changed { danger } else { accent },
                ));
                lines.push(Line::default());
                lines.push(Line::raw(
                    "Verify this fingerprint through an independent trusted channel.",
                ));
                if matches!(&self.screen, Screen::Reset(_)) {
                    lines.push(Line::styled(
                        "Replacement key requires a separate new review.",
                        danger,
                    ));
                    (
                        "Reset SSH host trust",
                        "Enter forget old trust · Esc cancel · PgUp/PgDn scroll",
                        Tone::Danger,
                    )
                } else if changed {
                    lines.push(Line::styled(
                        "HOST KEY CHANGED — connection rejected",
                        danger,
                    ));
                    (
                        "SSH host key changed",
                        "f review trust reset · Esc cancel · PgUp/PgDn scroll",
                        Tone::Danger,
                    )
                } else {
                    (
                        "Trust SSH host key",
                        "Enter trust this key · Esc cancel · PgUp/PgDn scroll",
                        Tone::Normal,
                    )
                }
            }
            Screen::Busy => {
                lines.push(Line::styled(
                    if self.approved {
                        "Applying confirmed change…"
                    } else if self.ignored {
                        "Previous request is still finishing."
                    } else {
                        "Loading connection / checking SSH host…"
                    },
                    accent,
                ));
                if self.ignored {
                    lines.push(Line::styled("Its result has been dismissed.", muted));
                }
                (
                    "Connections",
                    if self.approved {
                        "Please wait for completion"
                    } else {
                        "Esc dismiss"
                    },
                    Tone::Normal,
                )
            }
            Screen::Error(error) => {
                lines.push(Line::styled(safe(error), danger));
                (
                    "Connection error",
                    if self.error_back.is_some() {
                        "Esc return to editor · PgUp/PgDn scroll"
                    } else {
                        "Esc return to connections · PgUp/PgDn scroll"
                    },
                    Tone::Danger,
                )
            }
            Screen::Closed => return,
        };
        let desired_width = 86_u16;
        let wrap_width = frame
            .area()
            .width
            .saturating_sub(8)
            .min(desired_width.saturating_sub(4))
            .max(1) as usize;
        let heights: Vec<usize> = lines
            .iter()
            .map(|line| line.width().max(1).div_ceil(wrap_width))
            .collect();
        let total: usize = heights.iter().sum();
        let body = panel(
            frame,
            title,
            desired_width,
            total.min(u16::MAX as usize) as u16,
            footer,
            tone,
        );
        let available = body.height as usize;
        let scroll = if let Some(index) = selected {
            let before: usize = heights.iter().take(index).sum();
            let after = before
                + heights.get(index).copied().unwrap_or(1)
                + if matches!(self.screen, Screen::Picker) {
                    heights.get(index + 1).copied().unwrap_or(0)
                } else {
                    0
                };
            after.saturating_sub(available).min(before)
        } else {
            (self.scroll as usize).min(total.saturating_sub(available))
        };
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((scroll.min(u16::MAX as usize) as u16, 0)),
            body,
        );
        if let Some((index, prefix_width, value)) = active_input {
            let before: usize = heights.iter().take(index).sum();
            if before >= scroll
                && before - scroll < body.height as usize
                && prefix_width < body.width as usize
            {
                input(
                    frame,
                    ratatui::layout::Rect::new(
                        body.x + prefix_width as u16,
                        body.y + (before - scroll) as u16,
                        body.width - prefix_width as u16,
                        1,
                    ),
                    &value,
                );
            }
        }
    }
}
fn safe(text: &str) -> String {
    display_os(OsStr::new(text))
}
fn ensure_record(expected: &ConnectionRecord) -> Result<(), String> {
    let current = service::load_saved()?
        .into_iter()
        .find(|r| r.id == expected.id)
        .ok_or("Connection was removed")?;
    if serde_json::to_vec(&current).map_err(|_| "Cannot compare connection settings")?
        != serde_json::to_vec(expected).map_err(|_| "Cannot compare connection settings")?
    {
        return Err("Connection settings changed during review; reopen the connection".into());
    }
    Ok(())
}
fn connect(id: &str) -> Result<Reply, String> {
    let mut record = service::load_saved()?
        .into_iter()
        .find(|r| r.id == id)
        .ok_or("Connection was removed")?;
    if record.protocol == Protocol::Sftp {
        let review = service::probe(id)?;
        if review.previous.as_ref() != Some(&review.fingerprint) {
            return Ok(Reply::Host(Box::new(review)));
        }
        record = review.record;
    }
    let location = service::location_for(&record);
    let _registry = service::prepare(&location)?;
    Ok(Reply::Navigate(location))
}
fn save_form(form: Form) -> Result<Reply, String> {
    let record = form.record()?;
    let patch = std::array::from_fn(|i| {
        if form.clear[i] {
            Some(String::new())
        } else if !form.values[i + 12].is_empty() {
            Some(form.values[i + 12].clone())
        } else {
            None
        }
    });
    connections::save_with_patch(&record, form.original.as_ref(), patch)?;
    Ok(Reply::Notice(
        "Connection saved; submitted credentials stored in macOS Keychain".into(),
    ))
}

#[cfg(test)]
mod modal_tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    fn record(name: &str) -> ConnectionRecord {
        let mut form = Form::new(None);
        form.values[0] = name.into();
        form.values[2] = "example.test".into();
        form.values[4] = "developer".into();
        form.record().unwrap()
    }
    fn draw(ui: &ConnectionUi, width: u16, height: u16, name: &str) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| ui.render(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        super::super::modal::capture(name, buffer);
        buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }
    #[test]
    fn group_management_keyboard_and_review() {
        let mut ui = ConnectionUi::new();
        ui.groups = vec!["Production".into(), "Staging".into()];
        ui.selected_group = Some("Production".into());
        ui.screen = Screen::Groups;
        assert!(draw(&ui, 90, 22, "connection-groups").contains("Production"));
        ui.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(ui.selected_group.as_deref(), Some("Staging"));
        ui.key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        ui.paste(" backup\n");
        let Screen::GroupEdit { old, value } = &ui.screen else {
            panic!("editor missing")
        };
        assert_eq!(old.as_deref(), Some("Staging"));
        assert_eq!(value, "Staging backup");
        assert!(draw(&ui, 90, 15, "connection-group-edit").contains("Renaming"));
        ui.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        ui.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        let text = draw(&ui, 90, 15, "connection-group-remove");
        assert!(text.contains("Ungrouped"));
        assert!(text.contains("credentials are preserved"));
        ui.key(KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Repeat,
            state: crossterm::event::KeyEventState::NONE,
        });
        assert!(matches!(ui.screen, Screen::GroupRemove(_)));
        ui.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(ui.screen, Screen::Groups));
    }
    #[test]
    fn connection_picker_and_editor_snapshots() {
        let mut ui = ConnectionUi::new();
        ui.records = vec![record("Staging server"), record("Production server")];
        ui.screen = Screen::Picker;
        let text = draw(&ui, 110, 35, "connection-picker");
        assert!(text.contains("Staging server"));
        assert!(text.contains("Production server"));
        ui.index = 1;
        assert!(draw(&ui, 36, 12, "connection-picker-narrow").contains("Production server"));
        let mut form = Form::new(Some(record("Staging server")));
        form.index = 12;
        form.values[12] = "SECRET-MUST-NOT-APPEAR".into();
        ui.screen = Screen::Form(form);
        let text = draw(&ui, 90, 26, "connection-editor-secret");
        assert!(!text.contains("SECRET-MUST-NOT-APPEAR"));
        assert!(text.contains("Password"));
        let Screen::Form(form) = &mut ui.screen else {
            unreachable!()
        };
        form.index = 2;
        form.values[2] = format!("{}-visible-tail", "long-host-".repeat(30));
        assert!(draw(&ui, 90, 26, "connection-editor-long").contains("visible-tail"));
    }
    #[test]
    fn host_key_and_remove_review_snapshots() {
        let mut ui = ConnectionUi::new();
        ui.screen = Screen::Trust(service::HostReview {
            record: record("Production server"),
            fingerprint: "SHA256:observed-key".into(),
            previous: Some("SHA256:previous-key".into()),
        });
        let text = draw(&ui, 100, 30, "connection-host-key-changed");
        assert!(text.contains("connection rejected"));
        assert!(terminal_has_red(&ui));
        ui.screen = Screen::Remove(record("Production server"));
        assert!(draw(&ui, 100, 30, "connection-remove").contains("stored credentials"));
        assert!(terminal_has_red(&ui));
    }
    #[test]
    fn remaining_connection_states_render() {
        let mut ui = ConnectionUi::new();
        let mut form = Form::new(Some(record("Staging server")));
        form.values[12] = "secret-never-rendered".into();
        ui.screen = Screen::Save(form);
        let text = draw(&ui, 100, 32, "connection-save");
        assert!(text.contains("REPLACE"));
        assert!(!text.contains("secret-never-rendered"));
        ui.screen = Screen::Trust(service::HostReview {
            record: record("Staging server"),
            fingerprint: "SHA256:first-key".into(),
            previous: None,
        });
        assert!(draw(&ui, 100, 32, "connection-trust-first").contains("Enter trust"));
        ui.screen = Screen::Reset(service::HostReview {
            record: record("Staging server"),
            fingerprint: "SHA256:replacement-key".into(),
            previous: Some("SHA256:old-key".into()),
        });
        assert!(draw(&ui, 100, 32, "connection-reset-trust").contains("separate new review"));
        ui.screen = Screen::Busy;
        assert!(draw(&ui, 100, 32, "connection-busy").contains("checking SSH"));
        ui.screen = Screen::Error("Connection failed: authentication rejected".into());
        assert!(draw(&ui, 100, 32, "connection-error").contains("authentication rejected"));
    }
    fn terminal_has_red(ui: &ConnectionUi) -> bool {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| ui.render(frame)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|cell| cell.fg == Color::Red)
    }
}
