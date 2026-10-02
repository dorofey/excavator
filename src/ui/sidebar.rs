//! Scrollable, resizable sidebar with expandable favorites, locations and connections.
use super::{connections::record_location, tree::Children, *};
use gpui_kit::assets::IconName as AssetIcon;

pub(super) const SIDEBAR_WIDTH: f32 = 200.;

#[derive(Clone)]
pub(super) struct Node {
    key: String,
    location: Option<Location>,
    label: String,
    icon: AssetIcon,
    remove_favorite: Option<usize>,
}

impl Workspace {
    pub(super) fn toggle_sidebar_node(&mut self, location: Location, cx: &mut Context<Self>) {
        if self.sidebar_tree.is_expanded(&location) {
            self.sidebar_tree.collapse(&location);
            cx.notify();
            return;
        }
        let token = self.sidebar_tree.begin(&location);
        let request = self.registry.list(
            location.clone(),
            ListOptions {
                show_hidden: self.preferences.show_hidden,
            },
            token.clone(),
        );
        let task = cx.background_executor().spawn(request);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let children = match result {
                    Ok(entries) => {
                        let mut folders = entries
                            .into_iter()
                            .filter(|entry| entry.kind == EntryKind::Directory)
                            .collect::<Vec<_>>();
                        tree::sort_list(&mut folders, Sort::Name, false);
                        Children::Loaded(folders)
                    }
                    Err(error) => Children::Failed(error.to_string()),
                };
                if this.sidebar_tree.finish(&location, &token, children) {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn ensure_sidebar_disks_loaded(&mut self, cx: &mut Context<Self>) {
        let volumes = Location::Local(PathBuf::from("/Volumes"));
        if cfg!(target_os = "macos") && !self.sidebar_tree.children.contains_key(&volumes) {
            self.toggle_sidebar_node(volumes, cx);
        }
    }

    fn sidebar_nodes(&self, searching: bool) -> Vec<(Option<Node>, usize)> {
        let mut nodes = Vec::new();
        let section = |key: &str, label: &str, icon| Node {
            key: key.into(),
            location: None,
            label: label.into(),
            icon,
            remove_favorite: None,
        };
        nodes.push((
            Some(section("favorites", "Favorites", AssetIcon::Folder)),
            0,
        ));
        if searching || !self.sidebar_collapsed.contains("favorites") {
            for (index, path) in self.preferences.favorites.iter().enumerate() {
                nodes.push((
                    Some(Node {
                        key: format!("fav{index}"),
                        location: Some(Location::Local(path.clone())),
                        label: path
                            .file_name()
                            .unwrap_or(path.as_os_str())
                            .to_string_lossy()
                            .into_owned(),
                        icon: AssetIcon::Folder,
                        remove_favorite: Some(index),
                    }),
                    1,
                ));
            }
        }
        nodes.push((
            Some(section("disks", "Connected Disks", AssetIcon::HardDrive)),
            0,
        ));
        if searching || !self.sidebar_collapsed.contains("disks") {
            nodes.push((
                Some(Node {
                    key: "root".into(),
                    location: Some(Location::Local(PathBuf::from("/"))),
                    label: "Local disk".into(),
                    icon: AssetIcon::HardDrive,
                    remove_favorite: None,
                }),
                1,
            ));
            if cfg!(target_os = "macos") {
                let volumes = Location::Local(PathBuf::from("/Volumes"));
                if let Some(Children::Loaded(children)) = self.sidebar_tree.children.get(&volumes) {
                    for (index, volume) in children.iter().enumerate() {
                        nodes.push((
                            Some(Node {
                                key: format!("disk{index}"),
                                location: Some(volume.location.clone()),
                                label: volume.name.to_string_lossy().into_owned(),
                                icon: AssetIcon::HardDrive,
                                remove_favorite: None,
                            }),
                            1,
                        ));
                    }
                } else {
                    nodes.push((
                        Some(Node {
                            key: "volumes".into(),
                            location: Some(volumes),
                            label: "Mounted volumes".into(),
                            icon: AssetIcon::HardDrive,
                            remove_favorite: None,
                        }),
                        1,
                    ));
                }
            }
        }
        nodes.push((
            Some(section("connections", "Connections", AssetIcon::Server)),
            0,
        ));
        if searching || !self.sidebar_collapsed.contains("connections") {
            let mut groups = self.connection_groups.clone();
            groups.extend(
                self.connections
                    .iter()
                    .map(|record| record.group.clone())
                    .filter(|group| !group.is_empty()),
            );
            groups.sort();
            groups.dedup();
            if self
                .connections
                .iter()
                .any(|record| record.group.is_empty())
            {
                groups.push(String::new());
            }
            for group in groups {
                let key = format!("connection-group:{group}");
                nodes.push((
                    Some(section(
                        &key,
                        if group.is_empty() {
                            "Ungrouped"
                        } else {
                            &group
                        },
                        AssetIcon::Folder,
                    )),
                    1,
                ));
                if !searching && self.sidebar_collapsed.contains(&key) {
                    continue;
                }
                for (index, record) in self
                    .connections
                    .iter()
                    .enumerate()
                    .filter(|(_, record)| record.group == group)
                {
                    nodes.push((
                        Some(Node {
                            key: format!("conn{index}"),
                            location: Some(record_location(record)),
                            label: record.name.clone(),
                            icon: if record.protocol == Protocol::S3 {
                                AssetIcon::Cloud
                            } else {
                                AssetIcon::Server
                            },
                            remove_favorite: None,
                        }),
                        2,
                    ));
                }
            }
        }
        nodes
    }

    pub(super) fn visible_sidebar_nodes(&self, cx: &App) -> Vec<(usize, (Option<Node>, usize))> {
        let query = self
            .sidebar_search
            .read(cx)
            .value()
            .to_string()
            .trim()
            .to_lowercase();
        let nodes = self.sidebar_nodes(!query.is_empty());
        if query.is_empty() {
            return nodes.into_iter().enumerate().collect();
        }
        let mut included = std::collections::HashSet::new();
        for (index, (node, depth)) in nodes.iter().enumerate() {
            if node
                .as_ref()
                .is_some_and(|node| node.label.to_lowercase().contains(&query))
            {
                included.insert(index);
                let mut parent_depth = *depth;
                for parent in (0..index).rev() {
                    if nodes[parent].1 < parent_depth {
                        included.insert(parent);
                        parent_depth = nodes[parent].1;
                    }
                }
            }
        }
        nodes
            .into_iter()
            .enumerate()
            .filter(|(index, _)| included.contains(index))
            .collect()
    }

    fn toggle_sidebar_group(&mut self, key: &str, cx: &mut Context<Self>) {
        if key == "disks" {
            self.ensure_sidebar_disks_loaded(cx);
        }
        if !self.sidebar_collapsed.remove(key) {
            self.sidebar_collapsed.insert(key.into());
        }
        cx.notify();
    }

    fn sidebar_focus_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.activate(self.active, window, cx);
    }

    pub(super) fn move_sidebar_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        let nodes = self.visible_sidebar_nodes(cx);
        let count = nodes.len();
        if count == 0 {
            return;
        }
        let current = nodes
            .iter()
            .position(|(original, _)| *original == self.sidebar_cursor)
            .unwrap_or(0) as isize;
        let visible = (current + delta).rem_euclid(count as isize) as usize;
        self.sidebar_cursor = nodes[visible].0;
        self.sidebar_scroll.scroll_to_item(visible);
        cx.notify();
    }
    /// → expands the cursor node or steps into its first child.
    pub(super) fn expand_sidebar_cursor(&mut self, cx: &mut Context<Self>) {
        let Some((_, (Some(node), _))) = self
            .visible_sidebar_nodes(cx)
            .into_iter()
            .find(|(original, _)| *original == self.sidebar_cursor)
        else {
            return;
        };
        if node.location.is_none() {
            if self.sidebar_collapsed.contains(&node.key) {
                self.toggle_sidebar_group(&node.key, cx);
            } else {
                self.move_sidebar_cursor(1, cx);
            }
        }
    }
    /// ← collapses the cursor node, or steps to the parent row.
    pub(super) fn collapse_sidebar_cursor(&mut self, cx: &mut Context<Self>) {
        let nodes = self.visible_sidebar_nodes(cx);
        let Some(position) = nodes
            .iter()
            .position(|(original, _)| *original == self.sidebar_cursor)
        else {
            return;
        };
        let (Some(node), _) = nodes[position].1.clone() else {
            return;
        };
        if node.location.is_none() && !self.sidebar_collapsed.contains(&node.key) {
            self.toggle_sidebar_group(&node.key, cx);
            return;
        }
        if let Some(parent) = (0..position)
            .rev()
            .find(|row| nodes[*row].1.1 < nodes[position].1.1)
        {
            self.sidebar_cursor = nodes[parent].0;
            self.sidebar_scroll.scroll_to_item(parent);
            cx.notify();
        }
    }
    pub(super) fn open_sidebar_cursor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, (Some(node), _))) = self
            .visible_sidebar_nodes(cx)
            .into_iter()
            .find(|(original, _)| *original == self.sidebar_cursor)
        else {
            self.sidebar_focus_pane(window, cx);
            return;
        };
        let Some(location) = node.location else {
            self.toggle_sidebar_group(&node.key, cx);
            return;
        };
        self.navigate(self.active, location, true, window, cx);
        self.activate(self.active, window, cx);
    }
    pub(super) fn remove_sidebar_favorite(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.preferences_loaded || index >= self.preferences.favorites.len() {
            return;
        }
        self.preferences.favorites.remove(index);
        self.sidebar_cursor = self.sidebar_cursor.saturating_sub(1);
        self.persist(cx);
        cx.notify();
    }

    pub(super) fn render_sidebar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = self.colors;
        let font_size = self.preferences.appearance.font_size;
        let row_height = self
            .preferences
            .appearance
            .row_density
            .row_height(font_size);
        let focused = self.sidebar_focus.is_focused(window);
        let cursor = self.sidebar_cursor;
        let nodes = self.visible_sidebar_nodes(cx);
        let mut rows: Vec<AnyElement> = vec![];
        for (original_index, (node, depth)) in &nodes {
            let index = *original_index;
            let Some(node) = node else {
                continue;
            };
            let is_group = node.location.is_none();
            let expanded = is_group && !self.sidebar_collapsed.contains(&node.key);
            let current = self.panes.contains(self.active)
                && node
                    .location
                    .as_ref()
                    .is_some_and(|location| self.panes[self.active].tab().path == *location);
            let active = focused && index == cursor;
            let group = format!("sb-group-{}", node.key);
            let open = node.location.clone();
            let toggle = node.key.clone();
            let open_group = node.key.clone();
            let icon = if expanded && node.icon == AssetIcon::Folder {
                AssetIcon::FolderOpen
            } else {
                node.icon
            };
            rows.push(
                div()
                    .id(SharedString::from(format!("sb-{}", node.key)))
                    .group(group.clone())
                    .h(px(row_height))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .pl(px(8. + *depth as f32 * 18.))
                    .pr(px(8.))
                    .line_height(relative(1.))
                    .rounded_sm()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .cursor_pointer()
                    .role(gpui_kit::accesskit::Role::TreeItem)
                    .aria_label(node.label.clone())
                    .aria_selected(active)
                    .when(current || active, |row| {
                        row.bg(rgb(if current {
                            theme.selection
                        } else {
                            theme.hover
                        }))
                    })
                    .hover(|style| {
                        style.bg(rgb(if current {
                            theme.selection_hover
                        } else {
                            theme.hover
                        }))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.sidebar_focus.focus(window, cx);
                            this.sidebar_cursor = index;
                            cx.notify();
                        }),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(location) = open.clone() {
                            this.navigate(this.active, location, true, window, cx);
                            this.activate(this.active, window, cx);
                        } else {
                            this.toggle_sidebar_group(&open_group, cx);
                        }
                    }))
                    .when(is_group, |row| {
                        row.child(
                            div()
                                .id(SharedString::from(format!("sb-toggle-{}", node.key)))
                                .flex_none()
                                .w(px(12.))
                                .h_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(rgb(theme.muted))
                                .child(
                                    Icon::new(if expanded {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronRight
                                    })
                                    .size(px(12.)),
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.sidebar_focus.focus(window, cx);
                                        this.sidebar_cursor = index;
                                        this.toggle_sidebar_group(&toggle, cx);
                                    }),
                                )
                                .on_click(|_, _, cx| cx.stop_propagation()),
                        )
                    })
                    .child(
                        Icon::new(icon)
                            .size(px((font_size + 1.).max(12.)))
                            .flex_none()
                            .text_color(rgb(theme.muted)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(node.label.clone()),
                    )
                    .when_some(node.remove_favorite, |row, index| {
                        row.child(
                            div()
                                .id(SharedString::from(format!("sb-remove-{}", node.key)))
                                .flex_none()
                                .px_1()
                                .opacity(0.)
                                .group_hover(group, |style| style.opacity(1.))
                                .focus_visible(|style| style.opacity(1.))
                                .text_color(rgb(theme.muted))
                                .hover(|style| style.text_color(rgb(theme.text)))
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "Remove favorite · ⌘⇧D",
                                    )
                                    .build(window, cx)
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.remove_sidebar_favorite(index, cx);
                                }))
                                .child(Icon::new(IconName::Close).size(px(11.))),
                        )
                    })
                    .into_any_element(),
            );
        }
        rows.push(
            div()
                .id("manage-connections")
                .mt_2()
                .px_1()
                .flex_none()
                .whitespace_nowrap()
                .cursor_pointer()
                .text_color(rgb(theme.accent))
                .hover(|style| style.bg(rgb(theme.hover)))
                .rounded_sm()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.connection_screen = Some(ConnectionScreen::List);
                    cx.notify();
                }))
                .child("Manage connections…")
                .into_any_element(),
        );
        div()
            .id("sidebar")
            .role(gpui_kit::accesskit::Role::Tree)
            .aria_label("Sidebar: favorites, locations, connections")
            .key_context("Sidebar")
            .track_focus(&self.sidebar_focus)
            .size_full()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .track_scroll(&self.sidebar_scroll)
            .pb_3()
            .px_1()
            .bg(rgb(theme.sidebar))
            .when(focused, |sidebar| {
                sidebar.border_l_1().border_color(rgb(theme.accent))
            })
            .children(rows)
    }
}
