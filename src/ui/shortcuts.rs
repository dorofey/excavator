//! Grouped app shortcuts. Palette entries supply the shared action labels/bindings.
use super::*;

impl Workspace {
    pub(super) fn toggle_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shortcuts_open {
            self.close_shortcuts(window, cx);
        } else {
            self.shortcuts_previous_focus = window.focused(cx);
            self.shortcuts_open = true;
            self.shortcuts_focus.focus(window, cx);
            cx.notify();
        }
    }
    fn close_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.shortcuts_open = false;
        if let Some(focus) = self.shortcuts_previous_focus.take() {
            focus.focus(window, cx);
        }
        cx.notify();
    }
    pub(super) fn render_shortcuts(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors;
        let font_size = self.preferences.appearance.font_size;
        let viewport = window.viewport_size();
        let width = viewport.width.min(px(680.)).max(px(260.)) - px(32.);
        let height = (viewport.height - px(40.)).max(px(160.));
        let mut groups: Vec<(&str, Vec<(String, String)>)> = vec![
            (
                "General",
                vec![
                    ("Keyboard shortcuts".into(), "⌘? / ? in file list".into()),
                    ("Command palette".into(), "⌘⇧P".into()),
                    ("Dismiss dialog or palette".into(), "Esc".into()),
                    ("Quit Excavator".into(), "⌘Q".into()),
                ],
            ),
            (
                "File navigation",
                vec![
                    ("Next / previous item".into(), "↓ / ↑".into()),
                    ("Extend selection".into(), "⇧↓ / ⇧↑".into()),
                    ("Expand folder / step into it".into(), "→".into()),
                    ("Collapse folder / go to parent row".into(), "←".into()),
                ],
            ),
            (
                "Sidebar",
                vec![
                    ("Return to the active pane".into(), "Esc".into()),
                    ("Remove the focused favorite".into(), "⌘⇧D".into()),
                ],
            ),
            ("Panes", vec![]),
            (
                "Tabs",
                vec![(
                    "New terminal tab from the + button".into(),
                    "⇧-click +".into(),
                )],
            ),
            (
                "File operations",
                vec![("Confirm reviewed operation".into(), "Enter".into())],
            ),
            ("Connections", vec![]),
            ("Terminal", vec![]),
            (
                "Text fields and dialogs",
                vec![
                    (
                        "Next / previous settings or connection field".into(),
                        "Tab / ⇧Tab".into(),
                    ),
                    ("Move palette selection".into(), "↑ / ↓".into()),
                    ("Run selected palette command".into(), "Enter".into()),
                    ("Copy / cut / paste text".into(), "⌘C / ⌘X / ⌘V".into()),
                    ("Select field text".into(), "⌘A".into()),
                ],
            ),
        ];
        for (label, binding, command) in COMMANDS {
            if binding.is_empty() {
                continue;
            }
            let group = match command {
                Command::Switch
                | Command::PreviousPane
                | Command::Split(_)
                | Command::ClosePane
                | Command::ResizeLeft(_) => 3,
                Command::NewTab
                | Command::CloseTab
                | Command::NextTab
                | Command::PreviousTab
                | Command::MoveTabLeft
                | Command::MoveTabRight => 4,
                Command::Operation(_) | Command::Transfers => 5,
                Command::Connections | Command::ImportForkLift => 6,
                Command::Terminal
                | Command::FocusTerminal
                | Command::NewTerminal
                | Command::SplitTerminal(_)
                | Command::EndTerminal
                | Command::FocusFiles => 7,
                Command::Settings | Command::Sidebar | Command::Hidden => 0,
                Command::FocusSidebar
                | Command::SidebarUp
                | Command::SidebarDown
                | Command::SidebarExpand
                | Command::SidebarCollapse
                | Command::SidebarOpen => 5,
                Command::Shortcuts => continue,
                _ => 1,
            };
            groups[group].1.push((
                label.to_string(),
                binding
                    .replace("Shift", "⇧")
                    .replace("Ctrl", "⌃")
                    .replace("Alt", "⌥"),
            ));
        }
        if self.preferences.vim_mode {
            groups.push(("Vim mode · file listings", Self::vim_shortcuts()));
        }
        groups[7].1.extend([
            (
                "Interrupt / EOF / suspend foreground command".into(),
                "⌃C / ⌃D / ⌃Z".into(),
            ),
            ("Shell control keys".into(), "⌃A–Z / ⌃Space".into()),
            ("Shell completion / cancel".into(), "Tab / Esc".into()),
            (
                "Cursor / navigation keys".into(),
                "Arrows / Home / End / PgUp / PgDn".into(),
            ),
            ("Function keys sent to terminal".into(), "F1–F12".into()),
            (
                "Alt prefixes terminal control sequences".into(),
                "⌥ + control/navigation key".into(),
            ),
            ("Paste into terminal".into(), "⌘V".into()),
            (
                "Enter / erase / delete".into(),
                "Enter / Backspace / Delete".into(),
            ),
            ("Reverse terminal completion".into(), "⇧Tab".into()),
        ]);
        let weak = cx.entity().downgrade();
        let close = weak.clone();
        let popup=div().id("shortcut-dialog-content").w(width).h(height).max_h(height).aria_label("Keyboard shortcuts").flex().flex_col().p_3().gap_2().rounded_lg().bg(rgb(colors.surface)).border_1().border_color(rgb(colors.border)).text_color(rgb(colors.text)).font_family(self.preferences.appearance.font_family.clone()).text_size(px(font_size))
            .child(div().flex().items_center().gap_2().child(div().flex_1().text_size(px(font_size+3.)).child("Keyboard shortcuts")).child(Button::new("close-shortcuts").ghost().compact().icon(Icon::new(IconName::Close)).accessibility_label("Close keyboard shortcuts").tooltip("Close keyboard shortcuts · Esc / ⌘?").on_click(move|_,window,cx|{let _=close.update(cx,|this,cx|this.close_shortcuts(window,cx));})))
            .child(div().text_color(rgb(colors.muted)).child("? opens help from a file list; text fields and shells keep normal typing. Terminal Tab, Esc and Control keys go to the shell; use ⌘⌥[ / ] to switch panes."))
            .child(div().id("shortcut-dialog-scroll").flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.shortcuts_scroll).children(groups.into_iter().map(|(title,rows)| {
                div().py_2().child(div().mb_1().text_color(rgb(colors.accent)).child(title)).children(rows.into_iter().map(|(label,binding)|div().flex().items_start().gap_3().py_1().child(div().flex_1().min_w_0().child(label)).child(div().w(px(190.)).flex_none().text_color(rgb(colors.muted)).child(binding))))
            })))
            .child(div().text_color(rgb(colors.muted)).child("Scroll / ↑↓ / PgUp / PgDn for more · Esc or ⌘? to close"));
        gpui_kit::base::Dialog::new(cx)
            .focus_handle(self.shortcuts_focus.clone())
            .popup(popup)
            .backdrop(div().size_full().bg(rgba(0x00000080)))
            .on_open_change(move |open, _, window, cx| {
                if !open {
                    let _ = weak.update(cx, |this, cx| this.close_shortcuts(window, cx));
                }
            })
            .into_any_element()
    }
    pub(super) fn with_shortcuts(
        &self,
        root: impl IntoElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.shortcuts_open {
            let scroll = self.shortcuts_scroll.clone();
            let mut wrapper = div()
                .size_full()
                .key_context("Shortcuts")
                .on_key_down(move |event, window, cx| {
                    let delta = match event.keystroke.key.as_str() {
                        "down" => Some(-32.),
                        "up" => Some(32.),
                        "pagedown" => Some(-250.),
                        "pageup" => Some(250.),
                        "home" => {
                            scroll.set_offset(point(px(0.), px(0.)));
                            Some(0.)
                        }
                        "end" => {
                            scroll.scroll_to_bottom();
                            Some(0.)
                        }
                        _ => None,
                    };
                    if let Some(delta) = delta {
                        let offset = scroll.offset();
                        scroll.set_offset(point(offset.x, offset.y + px(delta)));
                        cx.stop_propagation();
                        window.prevent_default();
                        window.refresh();
                    }
                })
                .on_action(cx.listener(|this, _: &ToggleShortcuts, window, cx| {
                    this.close_shortcuts(window, cx)
                }));
            macro_rules! block {
                ($($action:ty),* $(,)?) => { $(wrapper = wrapper.capture_action(|_: &$action, _, cx| cx.stop_propagation());)* };
            }
            block!(
                Settings,
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
                OpenSelection,
                SelectAll,
                EditPath,
                ToggleHidden,
                TogglePalette,
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
            );
            wrapper
                .child(root)
                .child(self.render_shortcuts(window, cx))
                .into_any_element()
        } else {
            root.into_any_element()
        }
    }
}
