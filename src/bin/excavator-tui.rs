//! Single-pane local file browser. Provider work runs on its bounded worker.
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use excavator::domain::Location;
use excavator::transfers::Operation;
use excavator::tui::{
    browser::{Browser, SortDirection},
    connection_ui::ConnectionUi,
    coordination::Direction,
    favorites::Favorites,
    runtime::Runtime,
    view::{self, Overlay},
};
use ratatui::widgets::TableState;
use signal_hook::{
    SigId,
    consts::{SIGINT, SIGTERM},
    low_level::unregister,
};
use std::{
    io::{self, IsTerminal},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
struct VimState {
    count: String,
    prefix: Option<char>,
    anchor: Option<Location>,
    generation: u64,
}
impl VimState {
    fn reset(&mut self) {
        self.count.clear();
        self.prefix = None;
        self.anchor = None;
    }
    fn take_count(&mut self) -> Option<usize> {
        let value = self.count.parse().ok();
        self.count.clear();
        value
    }
    fn status(&self) -> String {
        format!(
            "{} {}{}",
            if self.anchor.is_some() {
                "VISUAL"
            } else {
                "NORMAL"
            },
            self.count,
            self.prefix.map(String::from).unwrap_or_default()
        )
    }
    fn select_range(&mut self, browser: &mut Browser) {
        let Some(anchor) = self.anchor.as_ref() else {
            return;
        };
        let Some(start) = browser
            .entries
            .iter()
            .position(|entry| &entry.location == anchor)
        else {
            self.anchor = None;
            return;
        };
        let Some(end) = browser.cursor else {
            return;
        };
        browser.selected = browser.entries[start.min(end)..=start.max(end)]
            .iter()
            .map(|entry| entry.location.clone())
            .collect();
    }
}

struct TerminalCleanup;
impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        if let Err(error) = crossterm::execute!(io::stdout(), event::DisableBracketedPaste) {
            eprintln!("Could not disable bracketed paste: {error}");
        }
        ratatui::restore();
    }
}
struct Signals(Vec<SigId>);
impl Drop for Signals {
    fn drop(&mut self) {
        for id in self.0.drain(..) {
            unregister(id);
        }
    }
}
fn run() -> io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.as_deref() == Some(std::ffi::OsStr::new("--help")) {
        println!(
            "Usage: excavator-tui [local-directory]\nSingle-pane file tree with saved connections and Herdr targeting.
Right/Left: expand/collapse; Enter: open folder; Backspace: parent.
q: exit; ?: shortcuts; Ctrl+L: path; / or f: filename search; n/N: next/previous.
:: commands; c: connections; F5/F6: copy/move; F7: new folder; F2: rename; F8: Trash/delete.
Ctrl+C cancels a loading request; otherwise it exits."
        );
        return Ok(());
    }
    if first.as_deref() == Some(std::ffi::OsStr::new("--version")) {
        println!("excavator-tui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Expected at most one local directory; see --help.",
        ));
    }
    let path = match first {
        Some(path) => std::path::PathBuf::from(path),
        None => std::env::current_dir()?,
    };
    let location = Location::Local(std::path::absolute(path)?);
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "An interactive terminal is required.",
        ));
    }
    let stop = Arc::new(AtomicBool::new(false));
    let mut signals = Signals(Vec::new());
    for signal in [SIGINT, SIGTERM] {
        signals
            .0
            .push(signal_hook::flag::register(signal, stop.clone())?);
    }
    // Create the guard before initialization to also clean up partial startup failures.
    let _cleanup = TerminalCleanup;
    let mut terminal = ratatui::try_init()?;
    crossterm::execute!(io::stdout(), event::EnableBracketedPaste)?;
    let mut runtime = Runtime::new(location.clone());
    let mut connections = ConnectionUi::new();
    let mut favorites = Favorites::new();
    let mut browser = Browser::new(location);
    let mut table_state = TableState::default();
    let mut overlay = Overlay::None;
    let mut notice = None;
    let mut vim = VimState::default();
    let mut redraw = true;
    while !stop.load(Ordering::Relaxed) {
        redraw |= browser.poll();
        if let Some(result) = browser.take_open_result() {
            notice = Some(match result {
                Ok(location) => format!("Opened {}", view::display_location(&location)),
                Err(error) => error.to_string(),
            });
        }
        redraw |= runtime.poll(&mut browser, &mut notice);
        let (destination, message, changed) = connections.poll();
        redraw |= changed;
        if let Some(destination) = destination {
            browser.navigate(destination);
        }
        if let Some(message) = message {
            notice = Some(message);
        }
        let (destination, message, changed) = favorites.poll();
        redraw |= changed;
        if let Some(destination) = destination {
            browser.navigate(destination);
        }
        if let Some(message) = message {
            notice = Some(message);
        }
        if vim.generation != browser.location_generation() {
            vim.reset();
            vim.generation = browser.location_generation();
        }
        if redraw {
            let progress = runtime.progress();
            let browser_status = view::browser_status(&browser);
            let detail = notice.as_deref().filter(|text| !text.is_empty()).unwrap_or(
                if progress.is_some() {
                    ""
                } else {
                    &runtime.summary
                },
            );
            let status = if detail.is_empty() {
                format!("{} · {}", vim.status(), browser_status)
            } else {
                format!("{} · {} · {}", vim.status(), browser_status, detail)
            };
            terminal.draw(|frame| {
                view::render(
                    frame,
                    &browser,
                    &mut table_state,
                    &overlay,
                    Some(&status),
                    progress.as_ref(),
                );
                connections.render(frame);
                favorites.render(frame);
                runtime.render(frame);
            })?;
            redraw = false;
        }
        if event::poll(Duration::from_millis(30))? {
            match event::read()? {
                Event::Resize(..) => redraw = true,
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    if runtime.modal_active() {
                        vim.reset();
                        runtime.key(key, &mut notice);
                    } else if connections.active() {
                        vim.reset();
                        connections.key(key);
                    } else if favorites.active() {
                        vim.reset();
                        favorites.key(key);
                    } else if handle_key(
                        key,
                        &mut browser,
                        &mut overlay,
                        &mut notice,
                        &mut runtime,
                        &mut connections,
                        &mut favorites,
                        &mut vim,
                    ) {
                        break;
                    }
                    redraw = true;
                }
                Event::Paste(text) => {
                    if text.chars().any(char::is_control) {
                        notice =
                            Some("Paste rejected: the text contains control characters.".into());
                        redraw = true;
                        continue;
                    }
                    if runtime.modal_active() {
                        runtime.paste(&text, &mut notice);
                    } else if connections.active() {
                        connections.paste(&text);
                    } else if !favorites.active() {
                        match &mut overlay {
                            Overlay::Path(value) | Overlay::Search(value) => {
                                append_input(value, &text)
                            }
                            Overlay::Commands { query, index } => {
                                append_input(query, &text);
                                *index = 0;
                            }
                            _ => {}
                        }
                    }
                    redraw = true;
                }
                _ => {}
            }
        }
    }
    runtime.cancel_transfer();
    browser.cancel();
    ratatui::try_restore()?;
    Ok(())
}
fn append_input(value: &mut String, text: &str) {
    for character in text.chars() {
        if value.len() + character.len_utf8() > 8192 {
            break;
        }
        value.push(character);
    }
}
fn edit_path(browser: &Browser, overlay: &mut Overlay, notice: &mut Option<String>) {
    if let Location::Local(path) = &browser.location {
        let value = path.to_str().map(str::to_owned).unwrap_or_else(|| {
            *notice = Some("The current path contains non-UTF-8 bytes. Enter an absolute UTF-8 path, or navigate with the listing.".into());
            String::new()
        });
        *overlay = Overlay::Path(value);
    } else {
        *overlay = Overlay::Path(match &browser.location {
            Location::Sftp { path, .. } | Location::Ftps { path, .. } => path.clone(),
            Location::S3 { key, .. } => key.clone(),
            Location::Local(_) => unreachable!(),
        });
    }
}
fn command(
    index: usize,
    browser: &mut Browser,
    overlay: &mut Overlay,
    notice: &mut Option<String>,
    runtime: &mut Runtime,
    connections: &mut ConnectionUi,
    favorites: &mut Favorites,
) -> bool {
    match index {
        0 => browser.refresh(),
        1 => {
            browser.parent();
        }
        2 => {
            browser.back();
        }
        3 => {
            browser.forward();
        }
        4 => browser.toggle_hidden(),
        5 => browser.cycle_sort(),
        6 => {
            browser.selected = browser
                .entries
                .iter()
                .map(|entry| entry.location.clone())
                .collect();
        }
        7 => browser.clear_selection(),
        8 => edit_path(browser, overlay, notice),
        9 => {
            *overlay = Overlay::Help {
                scroll: 0,
                horizontal: 0,
            }
        }
        10 => return true,
        11 => connections.open(),
        12 => runtime.begin(Operation::Copy, None, false, browser, notice),
        13 => runtime.begin(Operation::Move, None, false, browser, notice),
        14 => runtime.begin(Operation::Copy, None, true, browser, notice),
        15 => runtime.begin(Operation::Move, None, true, browser, notice),
        16..=23 => {
            let direction = [
                Direction::Left,
                Direction::Right,
                Direction::Up,
                Direction::Down,
            ][(index - 16) % 4];
            runtime.begin(
                if index < 20 {
                    Operation::Copy
                } else {
                    Operation::Move
                },
                Some(direction),
                false,
                browser,
                notice,
            );
        }
        24 => runtime.open_log(),
        25 => runtime.cancel_transfer(),
        26 => {
            browser.expand_cursor();
        }
        27 => {
            browser.collapse_cursor();
        }
        28 => runtime.begin_operation(Operation::CreateDirectory, browser, notice),
        29 => runtime.begin_operation(Operation::Rename, browser, notice),
        30 => runtime.begin_operation(Operation::Trash, browser, notice),
        31 => *overlay = Overlay::Search(browser.search_query.clone()),
        32 => find_filename(browser, false, false, notice),
        33 => find_filename(browser, true, false, notice),
        34 => return_local(browser, notice),
        35 => favorites.open(),
        36 => open_item(browser, notice),
        37 => favorites.add_current(&browser.location),
        38 => favorites.request_remove(),
        39 => browser.toggle_sort_direction(),
        40 => browser.set_sort_direction(SortDirection::Ascending),
        41 => browser.set_sort_direction(SortDirection::Descending),
        42 => connections.open_groups(),
        _ => {}
    }
    false
}
fn return_local(browser: &mut Browser, notice: &mut Option<String>) {
    *notice = Some(if browser.return_to_local() {
        "Returned to local files. Approved transfers continue; X cancels an active transfer.".into()
    } else {
        "Already browsing local files.".into()
    });
}
fn open_item(browser: &mut Browser, notice: &mut Option<String>) {
    if browser.open_cursor() {
        return;
    }
    *notice = Some(match browser.open_file_cursor() {
        Ok(true) => "Opening file in its default app…".into(),
        Ok(false) => "Enter opens folders and regular local files. Remote files and symbolic links are not opened.".into(),
        Err(error) => error,
    });
}
fn find_filename(
    browser: &mut Browser,
    reverse: bool,
    include_current: bool,
    notice: &mut Option<String>,
) {
    let query = browser.search_query.clone();
    if query.is_empty() {
        *notice = Some("Press / or f to enter a filename search.".into());
        return;
    }
    *notice = Some(
        match browser.search_cursor(&query, reverse, include_current) {
            Some((index, total)) => {
                format!("Filename match {index}/{total}: {query} · n/N repeats")
            }
            None => format!("No visible filename matches: {query}"),
        },
    );
}
fn handle_key(
    key: KeyEvent,
    browser: &mut Browser,
    overlay: &mut Overlay,
    notice: &mut Option<String>,
    runtime: &mut Runtime,
    connections: &mut ConnectionUi,
    favorites: &mut Favorites,
    vim: &mut VimState,
) -> bool {
    if key.modifiers.contains(KeyModifiers::SUPER) {
        return false;
    }
    if key.kind == KeyEventKind::Repeat
        && (!matches!(
            key.code,
            KeyCode::Up
                | KeyCode::Down
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Char('j')
                | KeyCode::Char('k')
        ) || key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER))
    {
        return false;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        vim.reset();
        if !matches!(overlay, Overlay::None) {
            *overlay = Overlay::None;
        } else if browser.has_pending_listing() {
            browser.cancel();
        } else {
            return true;
        }
        return false;
    }
    match overlay {
        Overlay::Help { scroll, horizontal } => {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => *scroll = scroll.saturating_add(1).min(38),
                KeyCode::PageUp => *scroll = scroll.saturating_sub(8),
                KeyCode::PageDown => *scroll = scroll.saturating_add(8).min(38),
                KeyCode::Left => *horizontal = horizontal.saturating_sub(4),
                KeyCode::Right => *horizontal = horizontal.saturating_add(4).min(55),
                _ => {}
            }
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
            ) {
                *overlay = Overlay::None;
            }
            return false;
        }
        Overlay::Search(value) => {
            match key.code {
                KeyCode::Esc => *overlay = Overlay::None,
                KeyCode::Enter => {
                    browser.search_query = value.clone();
                    *overlay = Overlay::None;
                    find_filename(browser, false, true, notice);
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    value.clear()
                }
                KeyCode::Backspace => {
                    value.pop();
                }
                KeyCode::Char(c)
                    if !key.modifiers.intersects(
                        KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                    ) && !c.is_control() =>
                {
                    append_input(value, &c.to_string())
                }
                _ => {}
            }
            return false;
        }
        Overlay::Path(value) => {
            match key.code {
                KeyCode::Esc => *overlay = Overlay::None,
                KeyCode::Enter => match browser.location.parse_path(value) {
                    Ok(location) => {
                        browser.navigate(location);
                        *overlay = Overlay::None;
                        *notice = None;
                    }
                    Err(error) => *notice = Some(error.message),
                },
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    value.clear()
                }
                KeyCode::Backspace => {
                    value.pop();
                }
                KeyCode::Char(c)
                    if !key.modifiers.intersects(
                        KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                    ) && !c.is_control() =>
                {
                    append_input(value, &c.to_string())
                }
                _ => {}
            }
            return false;
        }
        Overlay::Commands { query, index } => {
            match key.code {
                KeyCode::Esc => *overlay = Overlay::None,
                KeyCode::Up => *index = index.saturating_sub(1),
                KeyCode::Down => {
                    let count = view::matching_commands(query).len();
                    *index = (*index + 1).min(count.saturating_sub(1));
                }
                KeyCode::Enter => {
                    let chosen = view::matching_commands(query).get(*index).copied();
                    *overlay = Overlay::None;
                    if let Some(chosen) = chosen {
                        return command(
                            chosen,
                            browser,
                            overlay,
                            notice,
                            runtime,
                            connections,
                            favorites,
                        );
                    }
                }
                KeyCode::Backspace => {
                    query.pop();
                    *index = 0;
                }
                KeyCode::Char(c)
                    if !key.modifiers.intersects(
                        KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                    ) && !c.is_control() =>
                {
                    append_input(query, &c.to_string());
                    *index = 0;
                }
                _ => {}
            }
            return false;
        }
        Overlay::None => {}
    }
    if key.code == KeyCode::Esc
        && (vim.anchor.is_some() || vim.prefix.is_some() || !vim.count.is_empty())
    {
        vim.reset();
        *notice = None;
        return false;
    }
    *notice = None;
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    if key.modifiers.contains(KeyModifiers::SUPER)
        || (control
            && !matches!(
                key.code,
                KeyCode::Char('a' | 'p' | 'l' | 'd' | 'u' | 'o' | 'i') | KeyCode::Tab
            ))
    {
        return false;
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        vim.reset();
        match key.code {
            KeyCode::Left => {
                browser.back();
            }
            KeyCode::Right => {
                browser.forward();
            }
            _ => {}
        };
        return false;
    }
    if !control {
        if let KeyCode::Char(digit @ '0'..='9') = key.code {
            if digit != '0' || !vim.count.is_empty() {
                if vim.count.len() < 4 {
                    vim.count.push(digit);
                }
                return false;
            }
        }
        if let Some(prefix) = vim.prefix.take() {
            let count = vim.take_count().unwrap_or(1);
            match (prefix, key.code) {
                ('g', KeyCode::Char('g')) => {
                    let target = count
                        .saturating_sub(1)
                        .min(browser.entries.len().saturating_sub(1));
                    browser.move_cursor(
                        target as isize - browser.cursor.unwrap_or(0) as isize,
                        false,
                    );
                    vim.select_range(browser);
                    return false;
                }
                ('y', KeyCode::Char('y')) | ('d', KeyCode::Char('d')) => {
                    if browser.selected.is_empty() {
                        if let Some(start) = browser.cursor {
                            browser.selected = browser
                                .entries
                                .iter()
                                .skip(start)
                                .take(count)
                                .map(|entry| entry.location.clone())
                                .collect();
                        }
                    }
                    vim.reset();
                    if prefix == 'y' {
                        runtime.begin(Operation::Copy, None, false, browser, notice);
                    } else {
                        runtime.begin_operation(Operation::Trash, browser, notice);
                    }
                    return false;
                }
                ('z', KeyCode::Char('a')) => {
                    if browser
                        .cursor
                        .is_some_and(|index| browser.is_expanded(index))
                    {
                        browser.collapse_cursor();
                    } else {
                        browser.expand_cursor();
                    }
                    vim.select_range(browser);
                    return false;
                }
                ('z', KeyCode::Char('o')) => {
                    if browser
                        .cursor
                        .is_some_and(|index| !browser.is_expanded(index))
                    {
                        browser.expand_cursor();
                    }
                    vim.select_range(browser);
                    return false;
                }
                ('z', KeyCode::Char('c')) => {
                    if browser
                        .cursor
                        .is_some_and(|index| browser.is_expanded(index))
                    {
                        browser.collapse_cursor();
                    }
                    vim.select_range(browser);
                    return false;
                }
                _ => {}
            }
        }
        if matches!(key.code, KeyCode::Char('g' | 'y' | 'd' | 'z')) {
            if let KeyCode::Char(prefix) = key.code {
                vim.prefix = Some(prefix);
            }
            return false;
        }
    }
    vim.prefix = None;
    let explicit_count = vim.take_count();
    let count = explicit_count.unwrap_or(1) as isize;
    let half_page = crossterm::terminal::size()
        .map(|(_, height)| height.saturating_sub(4) as isize / 2)
        .unwrap_or(10)
        .max(1);
    match key.code {
        KeyCode::Char('l') if control => edit_path(browser, overlay, notice),
        KeyCode::Char('o') if control => {
            browser.back();
        }
        KeyCode::Char('i') if control => {
            browser.forward();
        }
        KeyCode::Tab => {
            browser.forward();
        }
        KeyCode::Char('d') if control => browser.move_cursor(half_page * count, shift),
        KeyCode::Char('u') if control => browser.move_cursor(-half_page * count, shift),
        KeyCode::Char('v' | 'V') => {
            if vim.anchor.is_some() {
                vim.anchor = None;
            } else {
                vim.anchor = browser
                    .cursor
                    .and_then(|index| browser.entries.get(index))
                    .map(|entry| entry.location.clone());
                vim.select_range(browser);
            }
        }
        KeyCode::Char('R') => runtime.begin_operation(Operation::Rename, browser, notice),
        KeyCode::Char('c') => connections.open(),
        KeyCode::F(5) | KeyCode::Char('C') => runtime.begin(
            Operation::Copy,
            None,
            key.code == KeyCode::F(5) && shift,
            browser,
            notice,
        ),
        KeyCode::F(6) | KeyCode::Char('M') => runtime.begin(
            Operation::Move,
            None,
            key.code == KeyCode::F(6) && shift,
            browser,
            notice,
        ),
        KeyCode::F(7) => runtime.begin_operation(Operation::CreateDirectory, browser, notice),
        KeyCode::F(2) => runtime.begin_operation(Operation::Rename, browser, notice),
        KeyCode::F(8) => runtime.begin_operation(Operation::Trash, browser, notice),
        KeyCode::Char('f' | '/') => *overlay = Overlay::Search(browser.search_query.clone()),
        KeyCode::Char('n') => {
            for _ in 0..count {
                find_filename(browser, false, false, notice);
            }
        }
        KeyCode::Char('N') => {
            for _ in 0..count {
                find_filename(browser, true, false, notice);
            }
        }
        KeyCode::Char('L') => runtime.open_log(),
        KeyCode::Char('X') => runtime.cancel_transfer(),
        KeyCode::Char('q') => return true,
        KeyCode::Char('p') if control => {
            *overlay = Overlay::Commands {
                query: String::new(),
                index: 0,
            }
        }
        KeyCode::Char('a') if control => {
            browser.selected = browser
                .entries
                .iter()
                .map(|entry| entry.location.clone())
                .collect();
        }
        KeyCode::Up | KeyCode::Char('k') => browser.move_cursor(-count, shift),
        KeyCode::Down | KeyCode::Char('j') => browser.move_cursor(count, shift),
        KeyCode::PageUp => browser.move_cursor(-10 * count, shift),
        KeyCode::PageDown => browser.move_cursor(10 * count, shift),
        KeyCode::Home => browser.move_cursor(-(browser.entries.len() as isize), shift),
        KeyCode::End | KeyCode::Char('G') => {
            let target = explicit_count
                .map(|n| n.saturating_sub(1))
                .unwrap_or(browser.entries.len().saturating_sub(1))
                .min(browser.entries.len().saturating_sub(1));
            browser.move_cursor(
                target as isize - browser.cursor.unwrap_or(0) as isize,
                shift,
            )
        }
        KeyCode::Enter => open_item(browser, notice),
        KeyCode::Char('D') => return_local(browser, notice),
        KeyCode::Char('B') => favorites.open(),
        KeyCode::Char('A') => favorites.add_current(&browser.location),
        KeyCode::Char('S') => browser.toggle_sort_direction(),
        KeyCode::Right | KeyCode::Char('l') => {
            browser.expand_cursor();
        }
        KeyCode::Left | KeyCode::Char('h') => {
            browser.collapse_cursor();
        }
        KeyCode::Backspace => {
            browser.parent();
        }
        KeyCode::Char(' ') => {
            vim.anchor = None;
            browser.toggle_selected();
        }
        KeyCode::Esc => {
            let pending = vim.anchor.is_some() || vim.prefix.is_some() || explicit_count.is_some();
            vim.reset();
            if !pending {
                browser.clear_selection();
            }
        }
        KeyCode::Char('.') => browser.toggle_hidden(),
        KeyCode::Char('r') => browser.refresh(),
        KeyCode::Char('s') => browser.cycle_sort(),
        KeyCode::Char('?') => {
            *overlay = Overlay::Help {
                scroll: 0,
                horizontal: 0,
            }
        }
        KeyCode::Char(':') => {
            *overlay = Overlay::Commands {
                query: String::new(),
                index: 0,
            }
        }
        _ => {}
    }
    if !matches!(overlay, Overlay::None) || runtime.modal_active() || connections.active() {
        vim.reset();
    } else {
        vim.select_range(browser);
    }
    false
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("excavator-tui: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
