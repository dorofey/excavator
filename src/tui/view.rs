//! Terminal presentation only. Paths remain byte-safe in the browser model.
use super::{
    browser::{Browser, ListingState, Sort, SortDirection},
    modal::{self, Tone},
};
use crate::domain::{EntryKind, Location};
use crate::transfers::{JobSnapshot, JobState};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Cell, Gauge, Paragraph, Row, Table, TableState, Wrap},
};
use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

pub enum Overlay {
    None,
    Help { scroll: u16, horizontal: u16 },
    Path(String),
    Search(String),
    Commands { query: String, index: usize },
}

pub const COMMANDS: &[(&str, &str)] = &[
    ("Refresh", "r"),
    ("Parent", "Backspace"),
    ("Back", "Alt+Left"),
    ("Forward", "Alt+Right"),
    ("Hidden files", "."),
    ("Sort", "s"),
    ("Select all", "Ctrl+A"),
    ("Clear selection", "Esc"),
    ("Edit path", "Ctrl+L"),
    ("Help", "?"),
    ("Quit", "q"),
    ("Connections", "c"),
    ("Copy to pane", "F5"),
    ("Move to browser", "F6"),
    ("Copy to path", "Shift+F5"),
    ("Move to path", "Shift+F6"),
    ("Copy left", ":"),
    ("Copy right", ":"),
    ("Copy up", ":"),
    ("Copy down", ":"),
    ("Move left", ":"),
    ("Move right", ":"),
    ("Move up", ":"),
    ("Move down", ":"),
    ("Log", "L"),
    ("Cancel transfer", "X"),
    ("Expand folder", "Right"),
    ("Collapse folder / select parent", "Left"),
    ("Create folder", "F7"),
    ("Rename", "F2"),
    ("Trash local / delete remote", "F8"),
    ("Find filename", "/ / f"),
    ("Next filename match", "n"),
    ("Previous filename match", "N"),
    ("Return to local / disconnect browser", "D"),
    ("Browse favorites", "B"),
    ("Open folder / local file", "Enter"),
    ("Add current folder to favorites", "A"),
    ("Remove favorite", ":"),
    ("Reverse sort order", "S"),
    ("Sort ascending", ":"),
    ("Sort descending", ":"),
    ("Connection groups", ":"),
];

pub fn matching_commands(query: &str) -> Vec<usize> {
    let query = query.to_lowercase();
    COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(index, (name, _))| name.to_lowercase().contains(&query).then_some(index))
        .collect()
}

/// Escape literal backslashes, controls and invalid UTF-8 without losing bytes.
/// Rendering this value does not change the location used for provider I/O.
pub fn display_os(value: &OsStr) -> String {
    let mut bytes = value.as_bytes();
    let mut output = String::new();
    while !bytes.is_empty() {
        match std::str::from_utf8(bytes) {
            Ok(text) => {
                display_text(text, &mut output);
                break;
            }
            Err(error) => {
                let (valid, rest) = bytes.split_at(error.valid_up_to());
                // valid_up_to is guaranteed to end on a UTF-8 boundary.
                if let Ok(text) = std::str::from_utf8(valid) {
                    display_text(text, &mut output);
                }
                let count = error.error_len().unwrap_or(rest.len());
                for byte in &rest[..count] {
                    use std::fmt::Write;
                    let _ = write!(output, "\\x{byte:02X}");
                }
                bytes = &rest[count..];
            }
        }
    }
    output
}

fn display_text(text: &str, output: &mut String) {
    for character in text.chars() {
        if character == '\\' {
            output.push_str("\\\\");
        } else if character.is_control()
            || matches!(character, '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
        {
            use std::fmt::Write;
            let _ = write!(output, "\\u{{{:X}}}", character as u32);
        } else {
            output.push(character);
        }
    }
}

pub fn display_location(location: &Location) -> String {
    match location {
        Location::Local(path) => display_os(path.as_os_str()),
        _ => display_os(OsStr::new(&location.display())),
    }
}

// Nerd Fonts Font Awesome glyphs; use a Nerd Font Mono in the terminal.
fn entry_icon(kind: EntryKind, name: &OsStr, expanded: bool) -> &'static str {
    match kind {
        EntryKind::Directory => {
            if expanded {
                "\u{f07c}"
            } else {
                "\u{f07b}"
            }
        }
        EntryKind::Symlink => "\u{f0c1}",
        EntryKind::Other => "\u{f15b}",
        EntryKind::File => {
            let extension = std::path::Path::new(name)
                .extension()
                .and_then(OsStr::to_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            match extension.as_str() {
                "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "c" | "h" | "cpp" | "go" | "swift"
                | "sh" | "json" | "toml" | "yaml" | "yml" | "html" | "css" => "\u{f1c9}",
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "heic" => "\u{f1c5}",
                "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" => "\u{f1c7}",
                "mp4" | "mov" | "mkv" | "webm" => "\u{f1c8}",
                "pdf" => "\u{f1c1}",
                _ => "\u{f15b}",
            }
        }
    }
}

fn size_text(size: Option<u64>) -> String {
    let Some(size) = size else {
        return "—".into();
    };
    if size < 1024 {
        return format!("{size} B");
    }
    let mut scaled = size as f64;
    let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let mut unit = 0;
    while scaled >= 1024.0 && unit + 1 < units.len() {
        scaled /= 1024.0;
        unit += 1;
    }
    format!("{scaled:.1} {}", units[unit])
}

pub fn render(
    frame: &mut Frame,
    browser: &Browser,
    table_state: &mut TableState,
    overlay: &Overlay,
    notice: Option<&str>,
    progress: Option<&JobSnapshot>,
) {
    let area = frame.area();
    let [
        path_area,
        table_area,
        status_area,
        hints_area,
        progress_area,
    ] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(if progress.is_some() && area.height >= 7 {
            2
        } else {
            0
        }),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(display_location(&browser.location)).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        path_area,
    );
    let state_message = match &browser.state {
        ListingState::Loading => Some("Loading…  Ctrl+C cancels this request".into()),
        ListingState::Failed(error) => Some(format!(
            "{:?}: {}  ·  r retries; Backspace goes to parent",
            error.kind,
            display_os(OsStr::new(&error.message))
        )),
        ListingState::Cancelled => Some("Listing cancelled. Press r to retry.".into()),
        ListingState::Loaded if browser.entries.is_empty() => Some(if browser.show_hidden {
            "This directory is empty.".into()
        } else {
            "No visible entries. Press . to show hidden files.".into()
        }),
        ListingState::Loaded => None,
    };
    if let Some(message) = state_message {
        frame.render_widget(
            Paragraph::new(message).wrap(Wrap { trim: false }),
            table_area,
        );
    } else {
        let full = table_area.width >= 70;
        let size_visible = table_area.width >= 45;
        let mut widths = vec![Constraint::Min(1)];
        let mut headings = vec!["Name"];
        if size_visible {
            widths.push(Constraint::Length(10));
            headings.push("Size");
        }
        if full {
            widths.push(Constraint::Length(16));
            headings.push("Modified UTC");
        }
        let visible_rows = usize::from(table_area.height.saturating_sub(1));
        let mut offset = table_state
            .offset()
            .min(browser.entries.len().saturating_sub(visible_rows.max(1)));
        if let Some(cursor) = browser.cursor {
            if cursor < offset {
                offset = cursor;
            } else if cursor >= offset.saturating_add(visible_rows.max(1)) {
                offset = cursor.saturating_sub(visible_rows.saturating_sub(1));
            }
        }
        table_state.select(browser.cursor);
        *table_state.offset_mut() = offset;
        let end = offset
            .saturating_add(visible_rows)
            .min(browser.entries.len());
        let rows = browser.entries[offset..end]
            .iter()
            .enumerate()
            .map(|(row, entry)| {
                let index = offset + row;
                let marked = browser.selected.contains(&entry.location);
                let name = format!(
                    "{} {}{} {} {}",
                    if marked { "*" } else { " " },
                    "  ".repeat(browser.row_depth(index).min(64)),
                    if entry.kind == EntryKind::Directory {
                        if browser.is_expanded(index) {
                            "▾"
                        } else {
                            "▸"
                        }
                    } else {
                        " "
                    },
                    entry_icon(entry.kind, &entry.name, browser.is_expanded(index)),
                    display_os(&entry.name)
                );
                let suffix = match browser.row_listing_state(index) {
                    Some(ListingState::Loading) => "  loading…".to_owned(),
                    Some(ListingState::Failed(error)) => format!(
                        "  ! {} · Right retries",
                        display_os(OsStr::new(&error.message))
                    ),
                    Some(ListingState::Cancelled) => "  cancelled · Right retries".to_owned(),
                    Some(ListingState::Loaded)
                        if browser.is_expanded(index) && browser.is_empty_directory(index) =>
                    {
                        "  empty".to_owned()
                    }
                    _ => String::new(),
                };
                let mut cells = vec![Cell::from(format!("{name}{suffix}"))];
                if size_visible {
                    cells.push(Cell::from(size_text(entry.size)));
                }
                if full {
                    let modified = entry.modified.map_or_else(
                        || "—".into(),
                        |time| {
                            let time: chrono::DateTime<chrono::Utc> = time.into();
                            time.format("%Y-%m-%d %H:%M").to_string()
                        },
                    );
                    cells.push(Cell::from(modified));
                }
                let style = if entry.kind == EntryKind::Directory {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default()
                };
                Row::new(cells).style(style)
            });
        let mut visible_state =
            TableState::default().with_selected(browser.cursor.and_then(|cursor| {
                (cursor >= offset && cursor < end).then_some(cursor.saturating_sub(offset))
            }));
        frame.render_stateful_widget(
            Table::new(rows, widths)
                .header(Row::new(headings).style(Style::default().fg(Color::DarkGray)))
                .row_highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("› "),
            table_area,
            &mut visible_state,
        );
    }
    let status = notice
        .map(|text| display_os(OsStr::new(text)))
        .unwrap_or_else(|| browser_status(browser));
    frame.render_widget(Paragraph::new(status), status_area);
    frame.render_widget(
        Paragraph::new(
            "j/k move · h/l tree · v select · / find · : commands · F5 copy · F6 move · c connections",
        )
        .style(Style::default().fg(Color::DarkGray)),
        hints_area,
    );
    if let Some(job) = progress {
        render_progress(frame, progress_area, job);
    }
    render_overlay(frame, overlay, browser);
}

fn render_progress(frame: &mut Frame, area: Rect, job: &JobSnapshot) {
    if area.height < 2 {
        return;
    }
    let state = match job.state {
        JobState::Queued => "Queued",
        JobState::Running => "Transferring",
        JobState::AwaitingConflict { .. } => "Awaiting conflict decision",
        JobState::Completed => "Complete",
        JobState::Cancelled => "Cancelled",
        JobState::Failed => "Failed",
    };
    let color = match job.state {
        JobState::Failed => Color::Red,
        JobState::Cancelled | JobState::AwaitingConflict { .. } => Color::Yellow,
        _ => Color::Cyan,
    };
    let destination = job
        .plan
        .destination
        .as_ref()
        .map(|location| format!(" → {}", display_location(location)))
        .unwrap_or_default();
    frame.render_widget(
        Paragraph::new(format!(
            "{state} · {:?}{destination} · {} bytes",
            job.plan.operation, job.bytes_copied
        ))
        .style(Style::default().fg(color)),
        Rect::new(area.x, area.y, area.width, 1),
    );
    // Provider snapshots expose completed items, not a reliable total byte count.
    let ratio = if job.state == JobState::Completed {
        1.0
    } else if job.total_items == 0 {
        0.0
    } else {
        (job.completed_items as f64 / job.total_items as f64).clamp(0.0, 1.0)
    };
    frame.render_widget(
        Gauge::default()
            .ratio(ratio)
            .gauge_style(Style::default().fg(color))
            .label(format!(
                "{} / {} items · {state}",
                job.completed_items, job.total_items
            )),
        Rect::new(area.x, area.y + 1, area.width, 1),
    );
}

pub fn browser_status(browser: &Browser) -> String {
    let sort = match browser.sort {
        Sort::Name => "name",
        Sort::Size => "size",
        Sort::Modified => "modified",
    };
    format!(
        "{} entries · {} selected · sort: {} {} · hidden: {}",
        browser.entries.len(),
        browser.selected.len(),
        sort,
        match browser.sort_direction {
            SortDirection::Ascending => "↑",
            SortDirection::Descending => "↓",
        },
        if browser.show_hidden {
            "shown"
        } else {
            "hidden"
        }
    )
}

fn render_overlay(frame: &mut Frame, overlay: &Overlay, browser: &Browser) {
    match overlay {
        Overlay::None => {}
        Overlay::Help { scroll, horizontal } => {
            let area = modal::panel(
                frame,
                "Keyboard shortcuts",
                78,
                25,
                "↑↓ Scroll   ←→ Pan   Esc Close",
                Tone::Normal,
            );
            frame.render_widget(
                Paragraph::new(
                    "↑↓ / j k        Move cursor; counts: 10j\ngg / G          First / last; 5G selects row 5\nv / V / Esc     Visual range / leave visual mode\nh / l            Collapse / expand tree\nCtrl+D / Ctrl+U  Half-page down / up\nCtrl+O / Ctrl+I  Back / forward history\nza / zo / zc     Toggle / expand / collapse tree\nyy / dd          Review copy / Trash or remote delete\nR                Rename selected item\nEnter            Open folder / regular local file\nD                Return to local / disconnect browser\nB / A            Browse / add local favorite\nB, then d        Review removal of a favorite\nRight / Left     Expand / collapse; select parent\nBackspace        Parent directory\nAlt+Left/Right   Back / forward\nSpace            Toggle selection\nShift+↑↓         Extend selection\nCtrl+A           Select all\nEsc              Clear selection / close overlay\nCtrl+L           Edit path (Enter accepts)\nr                Refresh\n.                Toggle hidden files\ns / S            Sort field / reverse order\n: / Ctrl+P       Search commands\n?                Shortcut help\nCtrl+C           Cancel loading; otherwise exit\nq                Quit\nc / then g       Connections / manage groups\nF5 / F6          Copy to pane / move to browser\nShift+F5 / F6    Copy / move to path\n:                Directional copy / move commands\nL / X            Log / cancel transfer\nF7 / F2 / F8     Create folder / rename / Trash or delete\n/ or f / n / N   Find filename / next / previous match\n\n* marks selected entries. Links remain links.\nDisplay escapes do not change filesystem paths."
                )

                .scroll(((*scroll).min(42), (*horizontal).min(55))),
                area,
            );
        }
        Overlay::Search(input) => {
            let body = modal::panel(
                frame,
                "Find filename",
                72,
                4,
                "Enter Find   Ctrl+U Clear   Esc Cancel",
                Tone::Normal,
            );
            frame.render_widget(
                Paragraph::new("Visible tree rows · literal, case-insensitive")
                    .style(Style::default().fg(Color::DarkGray)),
                Rect::new(body.x, body.y, body.width, body.height.min(1)),
            );
            if body.height > 1 {
                render_input(frame, Rect::new(body.x, body.y + 1, body.width, 1), input);
            }
            if body.height > 2 {
                frame.render_widget(
                    Paragraph::new("n / N  Next / previous match")
                        .style(Style::default().fg(Color::DarkGray)),
                    Rect::new(body.x, body.y + 2, body.width, 1),
                );
            }
        }
        Overlay::Path(input) => {
            let body = modal::panel(
                frame,
                "Open location",
                80,
                4,
                "Enter Open   Ctrl+U Clear   Esc Cancel",
                Tone::Normal,
            );
            let hint = match &browser.location {
                Location::Local(_) => "Local directory · absolute path or ~/…",
                Location::S3 { .. } => "Object prefix in this bucket",
                _ => "Absolute path on this connection",
            };
            frame.render_widget(
                Paragraph::new(hint).style(Style::default().fg(Color::DarkGray)),
                Rect::new(body.x, body.y, body.width, body.height.min(1)),
            );
            if body.height > 1 {
                render_input(frame, Rect::new(body.x, body.y + 1, body.width, 1), input);
            }
        }
        Overlay::Commands { query, index } => {
            let count = matching_commands(query).len();
            let inner = modal::panel(
                frame,
                "Commands",
                68,
                count.min(10) as u16 + 2,
                "↑↓ Choose   Enter Run   Esc Close",
                Tone::Normal,
            );
            render_input(frame, inner, query);
            let list_area = Rect::new(
                inner.x,
                inner.y.saturating_add(2),
                inner.width,
                inner.height.saturating_sub(2),
            );
            let matching = matching_commands(query);
            if matching.is_empty() {
                frame.render_widget(Paragraph::new("No matching commands"), list_area);
            } else {
                let rows = matching.iter().map(|command| {
                    let (name, shortcut) = COMMANDS[*command];
                    Row::new([name, shortcut])
                });
                let mut state =
                    TableState::default().with_selected(Some((*index).min(matching.len() - 1)));
                frame.render_stateful_widget(
                    Table::new(rows, [Constraint::Min(1), Constraint::Length(14)])
                        .row_highlight_style(
                            Style::default()
                                .bg(Color::Cyan)
                                .fg(Color::Black)
                                .add_modifier(Modifier::BOLD),
                        )
                        .highlight_symbol("› "),
                    list_area,
                    &mut state,
                );
            }
        }
    }
}

fn render_input(frame: &mut Frame, area: Rect, input: &str) {
    modal::input(frame, area, input);
}

#[cfg(test)]
mod modal_tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use std::path::PathBuf;
    #[test]
    fn transfer_gauge_uses_items_and_reserves_bottom_rows() {
        use crate::transfers::{ConflictPolicy, Operation, OperationPlan};
        let browser = Browser::new(Location::Local("/private/tmp".into()));
        let mut job = JobSnapshot {
            id: 1,
            plan: OperationPlan {
                operation: Operation::Copy,
                sources: vec![Location::Local("/source".into())],
                destination: Some(Location::Local("/destination".into())),
                new_name: None,
                conflict_policy: ConflictPolicy::Ask,
            },
            state: JobState::Running,
            completed_items: 2,
            total_items: 7,
            bytes_copied: 12345,
            current: None,
            journal: vec![],
            error: None,
        };
        for (name, state) in [
            ("transfer-running", JobState::Running),
            ("transfer-complete", JobState::Completed),
            ("transfer-failed", JobState::Failed),
        ] {
            job.state = state;
            let mut terminal = Terminal::new(TestBackend::new(110, 14)).unwrap();
            terminal
                .draw(|frame| {
                    render(
                        frame,
                        &browser,
                        &mut TableState::default(),
                        &Overlay::None,
                        None,
                        Some(&job),
                    )
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            let row = |y| {
                (0..110)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            };
            assert!(row(12).contains("12345 bytes"));
            assert!(row(13).contains("2 / 7 items"));
            assert!(row(11).contains("j/k move"));
            modal::capture(name, buffer);
        }
        let mut terminal = Terminal::new(TestBackend::new(24, 6)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    &browser,
                    &mut TableState::default(),
                    &Overlay::None,
                    None,
                    Some(&job),
                )
            })
            .unwrap();
        assert!(
            !terminal
                .backend()
                .buffer()
                .content
                .iter()
                .any(|cell| cell.symbol() == "█")
        );
    }
    #[test]
    fn overlays_fit_and_inputs_keep_cursor_visible() {
        let browser = Browser::new(Location::Local(PathBuf::from(
            "/private/tmp/excavator-dialog-preview",
        )));
        for (name, overlay, width, height) in [
            (
                "search",
                Overlay::Search("long filename query with unicode café".into()),
                110,
                28,
            ),
            (
                "path",
                Overlay::Path("/Users/example/Personal/project".into()),
                110,
                28,
            ),
            (
                "commands",
                Overlay::Commands {
                    query: "copy".into(),
                    index: 0,
                },
                110,
                28,
            ),
            (
                "help",
                Overlay::Help {
                    scroll: 0,
                    horizontal: 0,
                },
                110,
                28,
            ),
            (
                "path-narrow",
                Overlay::Path("/a/very/long/directory/with/a-visible-tail".into()),
                32,
                10,
            ),
        ] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| render_overlay(frame, &overlay, &browser))
                .unwrap();
            let cursor = terminal.get_cursor_position().unwrap();
            let buffer = terminal.backend().buffer();
            modal::capture(name, buffer);
            if matches!(overlay, Overlay::Path(_) | Overlay::Search(_)) {
                assert!(cursor.x < width && cursor.y < height);
                assert!(
                    buffer
                        .content
                        .iter()
                        .any(|cell| cell.modifier.contains(Modifier::UNDERLINED))
                );
            }
        }
    }
}
