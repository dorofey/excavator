use super::*;
impl Workspace {
    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = true;
        self.palette = false;
        self.sync_appearance_inputs(window, cx);
        self.settings_inputs[0].update(cx, |input, cx| input.focus(window, cx));
        self.settings_scroll.set_offset(point(px(0.), px(0.)));
        if self.preferences_writable {
            self.notice = None;
        }
        cx.notify();
    }
    pub(super) fn sync_appearance_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let family = self.preferences.appearance.font_family.clone();
        let size = self.preferences.appearance.font_size.to_string();
        self.settings_inputs[2].update(cx, |input, cx| input.set_value(family, window, cx));
        self.settings_inputs[3].update(cx, |input, cx| input.set_value(size, window, cx));
    }
    pub(super) fn refresh_system_appearance(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.preferences.appearance.mode == AppearanceMode::System
            && self.colors
                != crate::appearance::resolve(&self.preferences.appearance, cx.window_appearance())
        {
            self.colors = crate::appearance::refresh(&self.preferences.appearance, window, cx);
            cx.notify();
        }
    }
    pub(super) fn preview_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.persist(cx);
        cx.defer_in(window, |this, window, cx| {
            this.colors = crate::appearance::apply(&this.preferences.appearance, window, cx);
            cx.notify();
            window.refresh();
        });
    }
    pub(super) fn update_font_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.settings_open || !self.preferences_loaded {
            return;
        }
        let mut appearance = self.preferences.appearance.clone();
        appearance.font_family = self.settings_inputs[2].read(cx).value().to_string();
        let size = self.settings_inputs[3].read(cx).value().to_string();
        appearance.font_size = match size.parse() {
            Ok(size) => size,
            Err(_) => {
                self.notice = Some("Font size must be a number from 10 to 20.".into());
                return;
            }
        };
        if let Err(error) = appearance.validate() {
            self.notice = Some(error);
            return;
        }
        if appearance != self.preferences.appearance {
            self.preferences.appearance = appearance;
            if self.preferences_writable {
                self.notice = None;
            }
            self.preview_appearance(window, cx);
        }
    }
    pub(super) fn settings_input_event(
        &mut self,
        index: usize,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index < 2 {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let query = self.settings_inputs[index].read(cx).value().to_lowercase();
                let command = if index == 0 {
                    [
                        ("Paper", Command::LightTheme(LightTheme::Paper)),
                        ("Frost", Command::LightTheme(LightTheme::Frost)),
                    ]
                    .into_iter()
                    .find(|(name, _)| name.to_lowercase().contains(&query))
                } else {
                    [
                        ("Graphite", Command::DarkTheme(DarkTheme::Graphite)),
                        ("Midnight", Command::DarkTheme(DarkTheme::Midnight)),
                    ]
                    .into_iter()
                    .find(|(name, _)| name.to_lowercase().contains(&query))
                };
                if let Some((_, command)) = command {
                    self.command(command, window, cx);
                }
            }
        } else if matches!(event, InputEvent::Change | InputEvent::PressEnter { .. }) {
            self.update_font_preview(window, cx);
        }
        cx.notify();
    }
    pub(super) fn move_settings_focus(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self
            .settings_inputs
            .iter()
            .position(|input| input.read(cx).focus_handle(cx).is_focused(window))
            .unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(4) as usize;
        self.settings_inputs[next].update(cx, |input, cx| input.focus(window, cx));
        self.settings_scroll.scroll_to_item([2, 3, 5, 6][next]);
        cx.notify();
    }
    fn setting_choice(
        &self,
        id: &'static str,
        label: &'static str,
        selected: bool,
        command: Command,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = self.colors;
        div()
            .id(id)
            .role(gpui_kit::accesskit::Role::Button)
            .aria_label(label)
            .flex_none()
            .whitespace_nowrap()
            .px_3()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(rgb(if selected { theme.accent } else { theme.border }))
            .bg(rgb(if selected {
                theme.selection
            } else {
                theme.surface
            }))
            .text_color(rgb(theme.text))
            .cursor_pointer()
            .hover(|style| {
                style.bg(rgb(if selected {
                    theme.selection_hover
                } else {
                    theme.hover
                }))
            })
            .on_click(cx.listener(move |this, _, window, cx| this.command(command, window, cx)))
            .child(label)
    }
    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.colors;
        let font = self.preferences.appearance.font_size;
        let appearance = &self.preferences.appearance;
        let label = |title: &'static str, description: &'static str| {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(title)
                .child(
                    div()
                        .text_size(px((font - 1.).max(10.)))
                        .text_color(rgb(theme.muted))
                        .child(description),
                )
        };
        let row = || {
            div()
                .w_full()
                .min_h(px(font * 3. + 25.))
                .flex()
                .items_center()
                .gap_4()
                .py_4()
                .border_b_1()
                .border_color(rgb(theme.border))
        };
        let section = |title: &'static str| {
            div()
                .pt_5()
                .pb_2()
                .border_b_1()
                .border_color(rgb(theme.border))
                .text_size(px((font - 1.).max(10.)))
                .text_color(rgb(theme.muted))
                .child(title)
        };
        let light_query = self.settings_inputs[0].read(cx).value().to_lowercase();
        let dark_query = self.settings_inputs[1].read(cx).value().to_lowercase();
        let light_choices = [
            ("Paper", "settings-paper", LightTheme::Paper),
            ("Frost", "settings-frost", LightTheme::Frost),
        ]
        .into_iter()
        .filter(|(name, _, _)| name.to_lowercase().contains(&light_query))
        .collect::<Vec<_>>();
        let dark_choices = [
            ("Graphite", "settings-graphite", DarkTheme::Graphite),
            ("Midnight", "settings-midnight", DarkTheme::Midnight),
        ]
        .into_iter()
        .filter(|(name, _, _)| name.to_lowercase().contains(&dark_query))
        .collect::<Vec<_>>();
        let body=div().id("settings-scroll").flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.settings_scroll).px_5().pb_5()
            .child(section("THEME"))
            .child(row().child(label("Appearance mode","Light and Dark keep a fixed appearance. System follows your OS."))
                .child(div().w(px(260.)).flex_none().flex().flex_wrap().gap_2().child(self.setting_choice("settings-light","Light",appearance.mode==AppearanceMode::Light,Command::AppearanceMode(AppearanceMode::Light),cx)).child(self.setting_choice("settings-dark","Dark",appearance.mode==AppearanceMode::Dark,Command::AppearanceMode(AppearanceMode::Dark),cx)).child(self.setting_choice("settings-system","System",appearance.mode==AppearanceMode::System,Command::AppearanceMode(AppearanceMode::System),cx))))
            .child(row().child(label("Light theme","Used in Light mode and when System appearance is light."))
                .child(div().w(px(260.)).flex_none().flex().flex_col().gap_2().child(Input::new(&self.settings_inputs[0]).aria_label("Search light themes")).child(div().flex().flex_wrap().gap_2().children(light_choices.iter().map(|(name,id,value)|self.setting_choice(id,name,appearance.light_theme==*value,Command::LightTheme(*value),cx)))).when(light_choices.is_empty(),|d|d.child(div().text_color(rgb(theme.muted)).child("No matching light themes")))))
            .child(row().child(label("Dark theme","Used in Dark mode and when System appearance is dark."))
                .child(div().w(px(260.)).flex_none().flex().flex_col().gap_2().child(Input::new(&self.settings_inputs[1]).aria_label("Search dark themes")).child(div().flex().flex_wrap().gap_2().children(dark_choices.iter().map(|(name,id,value)|self.setting_choice(id,name,appearance.dark_theme==*value,Command::DarkTheme(*value),cx)))).when(dark_choices.is_empty(),|d|d.child(div().text_color(rgb(theme.muted)).child("No matching dark themes")))))
            .child(section("UI FONT"))
            .child(row().child(label("Font family","Choose an installed font family. The default follows your system font."))
                .child(div().w(px(260.)).flex_none().child(Input::new(&self.settings_inputs[2]).aria_label("UI font family"))))
            .child(row().child(label("Font size","10–20 pixels. Valid changes preview immediately throughout the app."))
                .child(div().w(px(260.)).flex_none().child(Input::new(&self.settings_inputs[3]).aria_label("UI font size, 10 to 20 pixels"))))
            .child(section("FILE LIST"))
            .child(row().child(label("Row density","Adjust the space around filenames without changing their order or selection."))
                .child(div().w(px(260.)).flex_none().flex().flex_col().gap_2().children([("Compact","density-compact",RowDensity::Compact),("Comfortable","density-comfortable",RowDensity::Comfortable),("Spacious","density-spacious",RowDensity::Spacious)].into_iter().map(|(name,id,density)|self.setting_choice(id,name,appearance.row_density==density,Command::Density(density),cx)))))
            .child(section("INTERACTION"))
            .child(row().child(label("Vim mode","Vim keys apply to the file listing. Inputs, terminals, and dialogs keep their usual keys."))
                .child(div().w(px(260.)).flex_none().flex().flex_col().gap_2()
                    .child(self.setting_choice("settings-vim-mode",if self.preferences.vim_mode { "Enabled" } else { "Disabled" },self.preferences.vim_mode,Command::ToggleVim,cx))
                    .child(div().text_size(px((font - 1.).max(10.))).text_color(rgb(theme.muted)).child("Press ? in the file listing for keyboard help."))))
            .child(div().pt_5().flex().items_center().gap_4().child(self.setting_choice("reset-appearance","Reset appearance",false,Command::ResetAppearance,cx)).child(div().flex_1().text_color(rgb(theme.muted)).child("Changes save automatically. Pane locations and transfers stay intact.")))
            .when(!self.preferences_writable,|d|d.child(div().pt_3().text_color(rgb(theme.warning)).child("Settings could not be loaded safely. Changes preview for this session; the original JSON file is preserved.")))
            .when_some(self.notice.as_ref(),|d,notice|d.child(div().pt_3().text_color(rgb(theme.warning)).child(notice.clone())));
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(rgb(theme.background))
            .key_context("AppearanceSettings")
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "tab" {
                    this.move_settings_focus(
                        if event.keystroke.modifiers.shift {
                            -1
                        } else {
                            1
                        },
                        window,
                        cx,
                    );
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex_none()
                    .h(px(font + 52.))
                    .px_5()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(theme.border))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_size(px(font + 5.)).child("Settings"))
                            .child(
                                div()
                                    .text_size(px((font - 1.).max(10.)))
                                    .text_color(rgb(theme.muted))
                                    .child("Settings · live preview · ⌘,"),
                            ),
                    )
                    .child(
                        div()
                            .id("close-settings")
                            .role(gpui_kit::accesskit::Role::Button)
                            .aria_label("Close settings")
                            .aria_keyshortcuts("Escape")
                            .px_3()
                            .py_1()
                            .cursor_pointer()
                            .text_color(rgb(theme.accent))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.settings_open = false;
                                this.activate(this.active, window, cx);
                            }))
                            .child("Close · Esc"),
                    ),
            )
            .child(body)
    }
}
