//! Floating command palette and the Zed-like title bar.
use super::*;
use gpui_kit::component::{
    TitleBar,
    input::{Escape as InputEscape, MoveDown, MoveUp},
};

/// Main-window options: a transparent title bar drawn by the workspace.
pub fn window_options(bounds: Bounds<Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(640.), px(420.))),
        titlebar: Some(TitlebarOptions {
            title: Some("Excavator".into()),
            ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
    }
}

impl Workspace {
    pub(super) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = true;
        self.palette_index = 0;
        self.palette_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }
    pub(super) fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = false;
        self.activate(self.active, window, cx);
    }
    pub(super) fn run_palette_command(
        &mut self,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.command(command, window, cx);
        if !matches!(command, Command::Shortcuts) {
            self.palette = false;
        }
        if self.operation_dialog.is_none()
            && self.connection_screen.is_none()
            && !self.settings_open
            && !matches!(
                command,
                Command::EditPath
                    | Command::Shortcuts
                    | Command::Terminal
                    | Command::FocusTerminal
                    | Command::NewTerminal
            )
        {
            self.activate(self.active, window, cx);
        }
        cx.notify();
    }
    pub(super) fn run_selected_palette_command(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let commands = self.filtered_commands(cx);
        let index = self.palette_index.min(commands.len().saturating_sub(1));
        if let Some((_, _, command)) = commands.get(index) {
            self.run_palette_command(*command, window, cx);
        }
    }
    fn step_palette(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.filtered_commands(cx).len();
        if count == 0 {
            return;
        }
        let index = self.palette_index.min(count - 1) as isize + delta;
        self.palette_index = index.rem_euclid(count as isize) as usize;
        self.palette_scroll.scroll_to_item(self.palette_index);
        cx.notify();
    }

    pub(super) fn render_palette(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.colors;
        let font_size = self.preferences.appearance.font_size;
        let viewport = window.viewport_size();
        let width = (viewport.width - px(32.)).min(px(620.));
        let list_height = (viewport.height * 0.55).max(px(120.));
        let commands = self.filtered_commands(cx);
        let selected = self.palette_index.min(commands.len().saturating_sub(1));
        let empty = commands.is_empty();
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .justify_center()
            .pt(px(font_size + 44.))
            .bg(rgba(0x00000022))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close_palette(window, cx)),
            )
            .child(
                div()
                    .id("command-palette")
                    .key_context("CommandPalette")
                    .aria_label("Command palette")
                    .w(width)
                    .h_auto()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(theme.border))
                    .bg(rgb(theme.dialog))
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                        cx.stop_propagation();
                        this.step_palette(1, cx);
                    }))
                    .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                        cx.stop_propagation();
                        this.step_palette(-1, cx);
                    }))
                    .capture_action(cx.listener(|this, _: &InputEscape, window, cx| {
                        cx.stop_propagation();
                        this.close_palette(window, cx);
                    }))
                    .child(
                        div()
                            .flex_none()
                            .px_2()
                            .py_1()
                            .border_b_1()
                            .border_color(rgb(theme.border))
                            .child(
                                Input::new(&self.palette_input).appearance(false).prefix(
                                    Icon::new(IconName::Search)
                                        .size(px(font_size))
                                        .text_color(rgb(theme.muted)),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .id("command-palette-list")
                            .flex_none()
                            .max_h(list_height)
                            .overflow_y_scroll()
                            .track_scroll(&self.palette_scroll)
                            .p_1()
                            .when(empty, |list| {
                                list.child(
                                    div()
                                        .p_2()
                                        .text_color(rgb(theme.muted))
                                        .child("No matching commands"),
                                )
                            })
                            .children(commands.into_iter().enumerate().map(
                                |(id, (label, shortcut, command))| {
                                    let active = id == selected;
                                    div()
                                        .id(("command", id))
                                        .h(px(font_size + 14.))
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .gap_3()
                                        .px_2()
                                        .rounded_md()
                                        .whitespace_nowrap()
                                        .cursor_pointer()
                                        .when(active, |row| row.bg(rgb(theme.selection)))
                                        .hover(|s| {
                                            s.bg(rgb(if active {
                                                theme.selection_hover
                                            } else {
                                                theme.hover
                                            }))
                                        })
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.run_palette_command(command, window, cx);
                                        }))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .child(label),
                                        )
                                        .child(
                                            div()
                                                .flex_none()
                                                .text_color(rgb(theme.muted))
                                                .child(shortcut),
                                        )
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .justify_end()
                            .gap_4()
                            .px_3()
                            .py_1()
                            .border_t_1()
                            .border_color(rgb(theme.border))
                            .text_size(px((font_size - 1.).max(10.)))
                            .text_color(rgb(theme.muted))
                            .child("Select ↑↓")
                            .child("Run ⏎")
                            .child("Close esc"),
                    ),
            )
            .into_any_element()
    }

    /// Compact title bar: window controls, with navigation kept on shortcuts.
    pub(super) fn render_title_bar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = self.colors;
        let sidebar_visible = self.preferences.sidebar_visible;
        let sidebar_width = if sidebar_visible {
            self.sidebar_split
                .read(cx)
                .sizes()
                .first()
                .copied()
                .filter(|width| *width > px(0.))
                .unwrap_or(px(super::sidebar::SIDEBAR_WIDTH))
        } else {
            px(0.)
        };
        // TitleBar already reserves the traffic-light inset. Only the remainder
        // of the sidebar belongs to its search field, keeping tabs over panes.
        let titlebar_inset = px(if cfg!(target_os = "macos") { 80. } else { 12. });
        let search_width = (sidebar_width - titlebar_inset).max(px(0.));
        let tabs_width = (window.viewport_size().width - sidebar_width).max(px(120.));
        let geometry = self.layout.geometry(tabs_width, cx);
        let top_leaves = self.layout.top_leaves();
        let tabs = div()
            .h_full()
            .min_w_0()
            .flex_1()
            .flex()
            .overflow_hidden()
            .children(geometry.into_iter().filter(|(leaf, _, _)| top_leaves.contains(leaf)).enumerate().map(|(index, (leaf, _, width))| {
                // With no sidebar, the window controls occupy the first pane's
                // leading space, rather than shifting every pane's tab strip.
                let width = if !sidebar_visible && index == 0 {
                    (width - titlebar_inset).max(px(0.))
                } else { width };
                div()
                    .w(width)
                    .min_w_0()
                    .flex_none()
                    .h_full()
                    .overflow_hidden()
                    .child(self.render_pane_tabs(leaf, cx))
            }));
        let title_content = div()
            .h_full()
            .min_w_0()
            .flex_1()
            .flex()
            .when(sidebar_visible, |row| {
                row.child(
                    div()
                        .id("titlebar-sidebar-search")
                        .h_full()
                        .w(search_width)
                        .flex_none()
                        .flex()
                        .items_center()
                        .px_2()
                        .border_r_1()
                        .border_color(rgb(theme.border))
                        .child(
                            Input::new(&self.sidebar_search).appearance(false).prefix(
                                Icon::new(IconName::Search)
                                    .size(px(self.preferences.appearance.font_size))
                                    .text_color(rgb(theme.muted)),
                            ),
                        ),
                )
            })
            .child(tabs);
        TitleBar::new()
            .bg(rgb(theme.surface))
            .border_color(rgb(theme.border))
            .child(title_content)
    }
}
