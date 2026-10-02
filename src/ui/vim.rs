use super::*;

#[derive(Default)]
pub(super) struct VimState {
    pub visual: bool,
    marked_selection: bool,
    count: String,
    prefix: String,
    input: Option<(char, String)>,
    search: String,
    context: Option<(usize, u64)>,
}

impl Workspace {
    pub(super) fn vim_input_active(&self) -> bool { self.vim.input.is_some() }
    pub(super) fn vim_shortcuts() -> Vec<(String, String)> {
        [
            ("Move down / up", "j / k; counts e.g. 5j"),
            ("Parent / open", "h / l / Enter"),
            ("First / last row", "gg / G; 5gg or 5G"),
            ("Half page down / up", "Ctrl+D / Ctrl+U"),
            ("Toggle / expand / collapse tree", "za / zo / zc"),
            ("Range selection", "v"),
            ("Toggle item selection", "Space"),
            ("Leave visual / clear selection", "Esc"),
            ("Find name / next / previous", "/ / n / N"),
            ("History back / forward", "Ctrl+O / Ctrl+I"),
            ("Next / previous tab", "gt / gT"),
            (
                "Focus pane left / down / up / right",
                "Ctrl+W h / j / k / l",
            ),
            ("Split below / right", "Ctrl+W s / v"),
            ("Close split", "Ctrl+W c"),
            ("Review copy / Trash (remote: delete)", "yy / dd"),
            ("Rename / refresh / help", ":rename / :refresh / :help"),
            ("Shortcut help", "?"),
        ]
        .into_iter()
        .map(|(a, b)| (a.into(), b.into()))
        .collect()
    }
    pub(super) fn vim_status(&self) -> String {
        if let Some((kind, text)) = &self.vim.input {
            return format!("{kind}{text}▏");
        }
        format!(
            "{} {}{}",
            if self.vim.visual { "VISUAL" } else { "NORMAL" },
            self.vim.count,
            self.vim.prefix
        )
    }
    pub(super) fn reset_vim(&mut self) {
        self.vim = VimState::default();
    }
    pub(super) fn vim_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.preferences.vim_mode
            || self.palette
            || self.settings_open
            || self.shortcuts_open
            || self.connection_screen.is_some()
            || self.operation_dialog.is_some()
            || self.panes[self.active].tab().terminal.is_some()
            || !self.panes[self.active].focus.is_focused(window)
        {
            return false;
        }
        let stroke = &event.keystroke;
        let m = stroke.modifiers;
        if m.platform || m.alt {
            return false;
        }
        let context = (self.active, self.panes[self.active].tab().id);
        if self.vim.context != Some(context) {
            self.vim.visual = false;
            self.vim.count.clear();
            self.vim.prefix.clear();
            self.vim.input = None;
            self.vim.context = Some(context);
        }
        let mut key = stroke.key.clone();
        if m.shift && key.len() == 1 {
            key = match key.as_str() {
                "/" => "?".into(),
                ";" => ":".into(),
                _ => key.to_uppercase(),
            };
        }
        if key == "escape" {
            if self.vim.input.take().is_none() && !self.vim.visual {
                self.panes[self.active].tab_mut().selected.clear();
                self.vim.marked_selection = false;
            }
            self.vim.visual = false;
            self.vim.count.clear();
            self.vim.prefix.clear();
            cx.notify();
            return true;
        }
        if let Some((kind, mut text)) = self.vim.input.take() {
            if key == "enter" || key == "return" {
                if kind == '/' {
                    self.vim.search = text;
                    self.vim_search(false, true, cx);
                } else {
                    match text.trim() {
                        "rename" => self.command(Command::Operation(Operation::Rename), window, cx),
                        "refresh" => self.command(Command::Refresh, window, cx),
                        "help" => self.command(Command::Shortcuts, window, cx),
                        "" => {}
                        other => self.notice = Some(format!("Unknown Vim command: {other}")),
                    }
                }
            } else {
                if key == "backspace" {
                    text.pop();
                } else if !m.control {
                    if let Some(character) = &stroke.key_char {
                        text.push_str(character);
                    } else if key == "space" {
                        text.push(' ');
                    } else if key.chars().count() == 1 {
                        text.push_str(&key);
                    }
                }
                self.vim.input = Some((kind, text));
            }
            cx.notify();
            return true;
        }
        if m.control {
            if key == "w" {
                self.vim.prefix = "^w".into();
                cx.notify();
                return true;
            }
            let command = match key.as_str() {
                "o" => Some(Command::Back),
                "i" => Some(Command::Forward),
                _ => None,
            };
            if let Some(command) = command {
                self.vim.visual = false;
                self.vim.count.clear();
                self.vim.prefix.clear();
                self.command(command, window, cx);
                return true;
            }
            if key == "d" || key == "u" {
                let rows = (((f32::from(window.viewport_size().height) - 80.)
                    * self.vim_height_fraction(cx)
                    - 40.)
                    .max(1.)
                    / self
                        .preferences
                        .appearance
                        .row_density
                        .row_height(self.preferences.appearance.font_size)
                    / 2.)
                    .max(1.) as isize;
                let count = self.vim_count();
                self.vim_move(if key == "d" { rows } else { -rows } * count as isize, cx);
                return true;
            }
            return false;
        }
        if key.len() == 1 && key.as_bytes()[0].is_ascii_digit() && self.vim.prefix.is_empty() {
            if self.vim.count.len() < 6 {
                self.vim.count.push_str(&key);
            }
            cx.notify();
            return true;
        }
        let prefix = std::mem::take(&mut self.vim.prefix);
        if !prefix.is_empty() {
            let count = self.vim_count();
            match (prefix.as_str(), key.as_str()) {
                ("g", "g") => self.vim_row(count.saturating_sub(1), cx),
                ("g", "t") | ("g", "T") => {
                    for _ in 0..count {
                        self.command(
                            if key == "t" {
                                Command::NextTab
                            } else {
                                Command::PreviousTab
                            },
                            window,
                            cx,
                        );
                    }
                }
                ("z", "a" | "o" | "c") => {
                    let tab = self.panes[self.active].tab();
                    if let Some(row) = tab.selection_cursor {
                        if let Some(entry) = tab.entries.get(row) {
                            let expanded = tab.tree.is_expanded(&entry.location);
                            if entry.kind == EntryKind::Directory
                                && (key == "a"
                                    || (key == "o" && !expanded)
                                    || (key == "c" && expanded))
                            {
                                self.toggle_row(self.active, row, cx);
                            }
                        }
                    }
                }
                ("y", "y") => {
                    self.vim_operation_selection(count, cx);
                    self.command(Command::Operation(Operation::Copy), window, cx);
                }
                ("d", "d") => {
                    self.vim_operation_selection(count, cx);
                    self.command(Command::Operation(Operation::Trash), window, cx);
                }
                ("^w", "s") => self.command(Command::Split(Axis::Down), window, cx),
                ("^w", "v") => self.command(Command::Split(Axis::Right), window, cx),
                ("^w", "c") => self.command(Command::ClosePane, window, cx),
                ("^w", "h" | "j" | "k" | "l") => self.vim_focus(&key, window, cx),
                _ => {}
            }
            cx.notify();
            return true;
        }
        match key.as_str() {
            "g" | "z" | "y" | "d" => self.vim.prefix = key,
            "j" | "k" => {
                let count = self.vim_count() as isize;
                self.vim_move(if key == "j" { count } else { -count }, cx);
            }
            "G" => {
                let explicit = !self.vim.count.is_empty();
                let count = self.vim_count();
                let len = self.panes[self.active].tab().entries.len();
                self.vim_row(
                    if explicit {
                        count.saturating_sub(1)
                    } else {
                        len.saturating_sub(1)
                    },
                    cx,
                );
            }
            "h" => {
                self.vim.visual = false;
                self.command(Command::Parent, window, cx);
            }
            "l" | "enter" | "return" => {
                self.vim.visual = false;
                self.command(Command::Open, window, cx);
            }
            "v" => {
                if self.panes[self.active].tab().entries.is_empty() { return true; }
                self.vim.marked_selection = false;
                self.vim.visual = !self.vim.visual;
                if self.vim.visual {
                    let row = self.panes[self.active].tab().selection_cursor.unwrap_or(0);
                    self.select(self.active, row, false, false, cx);
                }
            }
            "space" => {
                let row = self.panes[self.active].tab().selection_cursor.unwrap_or(0);
                if row < self.panes[self.active].tab().entries.len() {
                    self.vim.marked_selection = true;
                    self.select(self.active, row, true, false, cx);
                }
            }
            "/" | ":" => {
                self.vim.input = Some((key.chars().next().unwrap(), String::new()));
                self.vim.count.clear();
            }
            "n" | "N" => {
                let count = self.vim_count();
                for _ in 0..count {
                    self.vim_search(key == "N", false, cx);
                }
            }
            "?" => self.command(Command::Shortcuts, window, cx),
            "tab" => self.command(if m.shift { Command::PreviousPane } else { Command::Switch }, window, cx),
            _ => {
                self.vim.count.clear();
                return false;
            }
        }
        if self.vim.prefix.is_empty() {
            self.vim.count.clear();
        }
        cx.notify();
        true
    }
    fn vim_operation_selection(&mut self, count: usize, cx: &mut Context<Self>) {
        let tab = self.panes[self.active].tab();
        let len = tab.entries.len();
        if len == 0 || self.vim.visual {
            return;
        }
        let row = tab.selection_cursor.unwrap_or(0);
        if count > 1 || tab.selected.is_empty() {
            self.select(self.active, row, false, false, cx);
            if count > 1 {
                self.select(
                    self.active,
                    row.saturating_add(count - 1).min(len - 1),
                    false,
                    true,
                    cx,
                );
            }
        }
    }
    fn vim_count(&mut self) -> usize {
        std::mem::take(&mut self.vim.count)
            .parse::<usize>()
            .unwrap_or(1)
            .clamp(1, 10000)
    }
    fn vim_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.vim.visual {
            self.extend_selection(delta, cx);
        } else if self.vim.marked_selection {
            let tab = self.panes[self.active].tab_mut();
            if tab.entries.is_empty() { return; }
            let row = (tab.selection_cursor.unwrap_or(0) as isize + delta).clamp(0, tab.entries.len() as isize - 1) as usize;
            tab.selection_cursor = Some(row);
            self.panes[self.active].scroll.scroll_to_item(row);
            cx.notify();
        } else {
            let tab = self.panes[self.active].tab();
            if tab.entries.is_empty() { return; }
            let row = (tab.selection_cursor.unwrap_or(0) as isize + delta)
                .clamp(0, tab.entries.len() as isize - 1) as usize;
            self.select(self.active, row, false, false, cx);
        }
    }
    fn vim_row(&mut self, row: usize, cx: &mut Context<Self>) {
        let len = self.panes[self.active].tab().entries.len();
        if len > 0 {
            let row = row.min(len - 1);
            if self.vim.marked_selection && !self.vim.visual {
                self.panes[self.active].tab_mut().selection_cursor = Some(row);
                self.panes[self.active].scroll.scroll_to_item(row);
                cx.notify();
            } else {
                self.select(self.active, row, false, self.vim.visual, cx);
            }
        }
    }
    fn vim_search(&mut self, backwards: bool, include_current: bool, cx: &mut Context<Self>) {
        let tab = self.panes[self.active].tab();
        let len = tab.entries.len();
        if len == 0 || self.vim.search.is_empty() {
            return;
        }
        let current = tab.selection_cursor.unwrap_or(0);
        let needle = self.vim.search.to_lowercase();
        let start = usize::from(!include_current);
        let found = (start..len + start)
            .map(|offset| {
                if backwards {
                    (current + len - offset % len) % len
                } else {
                    (current + offset) % len
                }
            })
            .find(|row| {
                tab.entries[*row]
                    .name
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&needle)
            });
        if let Some(row) = found {
            self.vim_row(row, cx);
        } else {
            self.notice = Some(format!("No names match /{}", self.vim.search));
        }
    }
    fn vim_height_fraction(&self, cx: &App) -> f32 {
        fn height(layout: &Layout, target: usize, cx: &App, available: f32) -> Option<f32> {
            match layout {
                Layout::Leaf(id) => (*id == target).then_some(available),
                Layout::Split {
                    axis,
                    state,
                    children,
                    ..
                } => {
                    let sizes = state.read(cx).sizes();
                    let ratio = if sizes.len() >= 2 && sizes[0] + sizes[1] > px(0.) {
                        f32::from(sizes[0]) / f32::from(sizes[0] + sizes[1])
                    } else {
                        0.5
                    };
                    let (a, b) = match axis {
                        Axis::Down => (available * ratio, available * (1. - ratio)),
                        Axis::Right => (available, available),
                    };
                    height(&children[0], target, cx, a)
                        .or_else(|| height(&children[1], target, cx, b))
                }
            }
        }
        height(&self.layout, self.active, cx, 1.).unwrap_or(1.)
    }
    fn vim_focus(&mut self, direction: &str, window: &mut Window, cx: &mut Context<Self>) {
        fn rectangles(
            layout: &Layout,
            cx: &App,
            rect: (f32, f32, f32, f32),
            out: &mut Vec<(usize, (f32, f32, f32, f32))>,
        ) {
            match layout {
                Layout::Leaf(id) => out.push((*id, rect)),
                Layout::Split {
                    axis,
                    children,
                    state,
                    ..
                } => {
                    let (x, y, w, h) = rect;
                    let sizes = state.read(cx).sizes();
                    let ratio = if sizes.len() >= 2 && sizes[0] + sizes[1] > px(0.) {
                        (f32::from(sizes[0]) / f32::from(sizes[0] + sizes[1])).clamp(0.01, 0.99)
                    } else {
                        0.5
                    };
                    let (a, b) = match axis {
                        Axis::Right => (
                            (x, y, w * ratio, h),
                            (x + w * ratio, y, w * (1. - ratio), h),
                        ),
                        Axis::Down => (
                            (x, y, w, h * ratio),
                            (x, y + h * ratio, w, h * (1. - ratio)),
                        ),
                    };
                    rectangles(&children[0], cx, a, out);
                    rectangles(&children[1], cx, b, out);
                }
            }
        }
        let mut rects = Vec::new();
        rectangles(
            &self.layout,
            cx,
            (
                0.,
                0.,
                f32::from(window.viewport_size().width),
                (f32::from(window.viewport_size().height) - 80.).max(1.),
            ),
            &mut rects,
        );
        let Some((_, (x, y, w, h))) = rects.iter().find(|(id, _)| *id == self.active).copied()
        else {
            return;
        };
        let (ax, ay) = (x + w / 2., y + h / 2.);
        let target = rects
            .into_iter()
            .filter(|(id, _)| *id != self.active)
            .filter_map(|(id, (x, y, w, h))| {
                let (dx, dy) = (x + w / 2. - ax, y + h / 2. - ay);
                let (forward, lateral) = match direction {
                    "h" => (-dx, dy.abs()),
                    "l" => (dx, dy.abs()),
                    "j" => (dy, dx.abs()),
                    _ => (-dy, dx.abs()),
                };
                (forward > 0.0001).then_some((id, forward + lateral * 3.))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((id, _)) = target {
            self.activate(id, window, cx);
        }
    }
}
