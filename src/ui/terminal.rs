//! Interactive PTY view. Terminal process and screen ownership stay in the backend.
use crate::{
    appearance::Tokens,
    terminal::{Color, Launch, Session, Snapshot, Status},
};
use gpui_kit::{prelude::*, *};
use std::{ops::Range, time::Duration};

pub(super) struct TerminalView {
    session: Option<Session>,
    snapshot: Option<Snapshot>,
    failure: Option<String>,
    focus: FocusHandle,
    colors: Tokens,
    font_size: f32,
    pub(super) label: String,
    marked: String,
    bounds: Bounds<Pixels>,
    requested_size: (u16, u16),
    font: Font,
}
/// Menlo with installed Nerd Font families as glyph fallbacks, so Powerline and
/// icon code points in prompts render instead of missing-glyph boxes.
fn terminal_font(window: &Window) -> Font {
    static FALLBACKS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let fallbacks = FALLBACKS.get_or_init(|| {
        let names = window.text_system().all_font_names();
        let mut nerd = names
            .into_iter()
            .filter(|name| name.contains("Nerd Font"))
            .collect::<Vec<_>>();
        // Prefer the symbols-only and monospaced variants, which keep cell widths.
        nerd.sort_by_key(|name| {
            (
                !name.starts_with("Symbols Nerd Font"),
                !name.ends_with("Nerd Font Mono"),
                name.clone(),
            )
        });
        nerd.truncate(3);
        nerd
    });
    let mut font = font("Menlo");
    if !fallbacks.is_empty() {
        font.fallbacks = Some(FontFallbacks::from_fonts(fallbacks.clone()));
    }
    font
}
impl TerminalView {
    pub(super) fn new(
        launch: Launch,
        label: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Workspace sets semantic colors before displaying the view.
        let colors = crate::appearance::resolve(&Default::default(), window.appearance());
        let (session, failure) = match Session::spawn(launch, 14, 80) {
            Ok(session) => (Some(session), None),
            Err(error) => (None, Some(error)),
        };
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(30))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        if let Some(snapshot) = this.session.as_ref().and_then(Session::snapshot) {
                            if this
                                .snapshot
                                .as_ref()
                                .is_none_or(|old| old.revision != snapshot.revision)
                            {
                                this.snapshot = Some(snapshot);
                                cx.notify();
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Self {
            session,
            snapshot: None,
            failure,
            focus: cx.focus_handle(),
            colors,
            font_size: 13.,
            label,
            marked: String::new(),
            bounds: Bounds::default(),
            requested_size: (14, 80),
            font: terminal_font(window),
        }
    }
    pub(super) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
    }
    pub(super) fn set_theme(&mut self, colors: Tokens, font_size: f32, cx: &mut Context<Self>) {
        if self.colors != colors || self.font_size != font_size {
            self.colors = colors;
            self.font_size = font_size;
            cx.notify();
        }
    }
    pub(super) fn status(&self) -> String {
        if let Some(error) = &self.failure {
            return format!("Failed: {error}");
        }
        match self.snapshot.as_ref().map(|s| &s.status) {
            Some(Status::Running) => "Running".into(),
            Some(Status::Exited(code)) => format!("Exited ({code})"),
            Some(Status::Failed(error)) => format!("Failed: {error}"),
            _ => "Starting".into(),
        }
    }
    pub(super) fn snapshot_text(&self) -> String {
        self.snapshot
            .as_ref()
            .map(Snapshot::text)
            .unwrap_or_default()
    }
    pub(super) fn send_input(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        if let Some(session) = &self.session {
            if let Err(error) = session.input(bytes) {
                self.failure = Some(error);
                cx.notify();
            }
        }
    }
    fn paste_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let text = text
            .chars()
            .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
            .collect::<String>();
        let bytes = if self.snapshot.as_ref().is_some_and(|s| s.bracketed_paste) {
            format!("\x1b[200~{text}\x1b[201~").into_bytes()
        } else {
            text.into_bytes()
        };
        self.send_input(&bytes, cx);
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        let m = key.modifiers;
        if m.platform {
            if key.key == "v" && !m.alt {
                if let Some(item) = cx.read_from_clipboard() {
                    if let Some(text) = item.text() {
                        self.paste_text(&text, cx);
                    }
                }
                cx.stop_propagation();
                window.prevent_default();
            }
            return;
        }
        if m.control && key.key == "`" {
            return;
        }
        let app = self.snapshot.as_ref().is_some_and(|s| s.application_cursor);
        let arrows = if app { "\x1bO" } else { "\x1b[" };
        let bytes = match key.key.as_str() {
            "enter" => Some(vec![b'\r']),
            "backspace" => Some(vec![127]),
            "tab" => Some(if m.shift {
                b"\x1b[Z".to_vec()
            } else {
                vec![b'\t']
            }),
            "escape" => Some(vec![27]),
            "up" => Some(format!("{arrows}A").into_bytes()),
            "down" => Some(format!("{arrows}B").into_bytes()),
            "right" => Some(format!("{arrows}C").into_bytes()),
            "left" => Some(format!("{arrows}D").into_bytes()),
            "home" => Some(b"\x1b[H".to_vec()),
            "end" => Some(b"\x1b[F".to_vec()),
            "delete" => Some(b"\x1b[3~".to_vec()),
            "pageup" => Some(b"\x1b[5~".to_vec()),
            "pagedown" => Some(b"\x1b[6~".to_vec()),
            "f1" => Some(b"\x1bOP".to_vec()),
            "f2" => Some(b"\x1bOQ".to_vec()),
            "f3" => Some(b"\x1bOR".to_vec()),
            "f4" => Some(b"\x1bOS".to_vec()),
            "f5" => Some(b"\x1b[15~".to_vec()),
            "f6" => Some(b"\x1b[17~".to_vec()),
            "f7" => Some(b"\x1b[18~".to_vec()),
            "f8" => Some(b"\x1b[19~".to_vec()),
            "f9" => Some(b"\x1b[20~".to_vec()),
            "f10" => Some(b"\x1b[21~".to_vec()),
            "f11" => Some(b"\x1b[23~".to_vec()),
            "f12" => Some(b"\x1b[24~".to_vec()),
            "space" if m.control => Some(vec![0]),
            _ if m.control => {
                let b = key.key.as_bytes();
                if b.len() == 1 && (b[0].is_ascii_alphabetic() || (b'@'..=b'_').contains(&b[0])) {
                    Some(vec![b[0].to_ascii_uppercase() & 31])
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(mut bytes) = bytes {
            if m.alt {
                bytes.insert(0, 27);
            }
            self.send_input(&bytes, cx);
            cx.stop_propagation();
            window.prevent_default();
        }
        // Printable text reaches EntityInputHandler, including Unicode and IME commits.
    }
}
impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let paint_entity = entity.clone();
        let colors = self.colors;
        let font_size = self.font_size;
        let base_font = self.font.clone();
        let paint_font = self.font.clone();
        div()
            .id("terminal-screen")
            .role(gpui_kit::accesskit::Role::Terminal)
            .aria_label(format!(
                "Integrated terminal · {} · {}",
                self.label,
                self.status()
            ))
            .key_context("Terminal")
            .track_focus(&self.focus)
            .size_full()
            .overflow_hidden()
            .bg(rgb(colors.background))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.focus(window, cx)),
            )
            .on_key_down(cx.listener(Self::key))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                let delta: f32 = event.delta.pixel_delta(px(this.font_size * 1.4)).y.into();
                if let Some(session) = &this.session {
                    if let Err(error) =
                        session.scroll((delta / (this.font_size * 1.4)).round() as i32)
                    {
                        this.failure = Some(error);
                        cx.notify();
                    }
                }
                cx.stop_propagation();
            }))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let run = TextRun {
                            len: 1,
                            font: base_font.clone(),
                            color: rgb(colors.text).into(),
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        let line = window.text_system().shape_line(
                            "M".into(),
                            px(font_size),
                            &[run],
                            None,
                        );
                        let cell_width = line.x_for_index(1).max(px(1.));
                        let line_height = px(font_size * 1.4);
                        let width: f32 = bounds.size.width.into();
                        let height: f32 = bounds.size.height.into();
                        let cw: f32 = cell_width.into();
                        let lh: f32 = line_height.into();
                        let rows = (height / lh).floor().clamp(2., 160.) as u16;
                        let cols = (width / cw).floor().clamp(2., 400.) as u16;
                        entity.update(cx, |this, cx| {
                            this.bounds = bounds;
                            if this.requested_size != (rows, cols) {
                                if let Some(session) = &this.session {
                                    match session.resize(rows, cols) {
                                        Ok(()) => this.requested_size = (rows, cols),
                                        Err(error) => {
                                            this.failure = Some(error);
                                            cx.notify();
                                        }
                                    }
                                }
                            }
                        });
                        (cell_width, line_height)
                    },
                    move |bounds, (cw, lh), window, cx| {
                        let focus = paint_entity.read(cx).focus.clone();
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, paint_entity.clone()),
                            cx,
                        );
                        let snapshot = paint_entity.read(cx).snapshot.clone();
                        let marked = paint_entity.read(cx).marked.clone();
                        if let Some(snapshot) = &snapshot {
                            for row in 0..snapshot.rows {
                                for col in 0..snapshot.cols {
                                    let Some(cell) = snapshot
                                        .cells
                                        .get(row as usize * snapshot.cols as usize + col as usize)
                                    else {
                                        continue;
                                    };
                                    if cell.continuation {
                                        continue;
                                    }
                                    let origin =
                                        bounds.origin + point(cw * col as f32, lh * row as f32);
                                    let mut fg =
                                        terminal_color(cell.foreground.clone(), colors.text);
                                    let mut bg =
                                        terminal_color(cell.background.clone(), colors.background);
                                    if cell.inverse {
                                        std::mem::swap(&mut fg, &mut bg);
                                    }
                                    if cell.dim {
                                        fg = ((fg & 0xfefefe) >> 1) + ((bg & 0xfefefe) >> 1);
                                    }
                                    let is_cursor = !snapshot.hide_cursor
                                        && snapshot.scrollback == 0
                                        && snapshot.cursor == (row, col)
                                        && focus.is_focused(window);
                                    if is_cursor {
                                        std::mem::swap(&mut fg, &mut bg);
                                        if fg == bg {
                                            bg = colors.accent;
                                            fg = colors.background;
                                        }
                                    }
                                    let width = if cell.wide { cw * 2. } else { cw };
                                    window.paint_quad(fill(
                                        Bounds::new(origin, size(width, lh)),
                                        rgb(bg),
                                    ));
                                    if !cell.text.is_empty() {
                                        let mut f = paint_font.clone();
                                        if cell.bold {
                                            f.weight = FontWeight::BOLD;
                                        }
                                        if cell.italic {
                                            f.style = FontStyle::Italic;
                                        }
                                        let run = TextRun {
                                            len: cell.text.len(),
                                            font: f,
                                            color: rgb(fg).into(),
                                            background_color: None,
                                            underline: cell.underline.then_some(UnderlineStyle {
                                                color: Some(rgb(fg).into()),
                                                thickness: px(1.),
                                                wavy: false,
                                            }),
                                            strikethrough: None,
                                        };
                                        let line = window.text_system().shape_line(
                                            cell.text.clone().into(),
                                            px(font_size),
                                            &[run],
                                            None,
                                        );
                                        let _ = line.paint(
                                            origin,
                                            lh,
                                            TextAlign::Left,
                                            None,
                                            window,
                                            cx,
                                        );
                                    }
                                }
                            }
                            if !marked.is_empty() && snapshot.scrollback == 0 {
                                let origin = bounds.origin
                                    + point(
                                        cw * snapshot.cursor.1 as f32,
                                        lh * snapshot.cursor.0 as f32,
                                    );
                                let run = TextRun {
                                    len: marked.len(),
                                    font: paint_font.clone(),
                                    color: rgb(colors.text).into(),
                                    background_color: Some(rgb(colors.surface).into()),
                                    underline: Some(UnderlineStyle {
                                        color: Some(rgb(colors.accent).into()),
                                        thickness: px(1.),
                                        wavy: false,
                                    }),
                                    strikethrough: None,
                                };
                                let line = window.text_system().shape_line(
                                    marked.into(),
                                    px(font_size),
                                    &[run],
                                    None,
                                );
                                let _ = line.paint(origin, lh, TextAlign::Left, None, window, cx);
                            }
                        }
                    },
                )
                .size_full(),
            )
    }
}
fn terminal_color(color: Color, default: u32) -> u32 {
    match color {
        Color::Default => default,
        Color::Rgb(r, g, b) => (r as u32) << 16 | (g as u32) << 8 | b as u32,
        Color::Indexed(i) => {
            const BASE: [u32; 16] = [
                0x202020, 0xc44e4e, 0x6aab73, 0xc9a85c, 0x6699cc, 0xb17db8, 0x6bb5b3, 0xd4d4d4,
                0x707070, 0xed7878, 0x91ce97, 0xebd18a, 0x93baf0, 0xd5a8db, 0x99dcda, 0xffffff,
            ];
            if i < 16 {
                BASE[i as usize]
            } else if i < 232 {
                let n = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v as u32 * 40 };
                level(n / 36) << 16 | level((n / 6) % 6) << 8 | level(n % 6)
            } else {
                let n = 8 + (i as u32 - 232) * 10;
                n << 16 | n << 8 | n
            }
        }
    }
}
impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        Some(self.marked.clone())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.marked.is_empty()).then_some(0..self.marked.encode_utf16().count())
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked.clear();
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked.clear();
        self.send_input(text.as_bytes(), cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = text.into();
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(self.bounds)
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }
    fn paste(&mut self, item: ClipboardItem, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = item.text() {
            self.paste_text(&text, cx);
        }
    }
}
