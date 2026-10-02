use super::*;
use crate::transfers::{ConflictPolicy, JobId, JobState, OperationPlan};
use std::time::Duration;
#[derive(Clone)]
pub(super) enum OperationDialog {
    Plan(OperationPlan),
    Conflict(JobId, Location, Location),
    Replace(JobId, Location, Location),
}
pub(super) fn operation_name(operation: Operation) -> &'static str {
    match operation {
        Operation::CreateDirectory => "Create folder",
        Operation::Rename => "Rename",
        Operation::Copy => "Copy",
        Operation::Move => "Move",
        Operation::Trash => "Move to Trash",
        Operation::Delete => "Permanently delete",
    }
}
impl Workspace {
    pub(super) fn drop_copy(
        &mut self,
        drag: super::PaneDrag,
        destination_pane: usize,
        destination_tab_id: u64,
        cx: &mut Context<Self>,
    ) {
        if !self.panes.contains(drag.source_pane) || !self.panes.contains(destination_pane) {
            self.notice = Some("A pane closed during the drag. Try the copy again.".into());
            cx.notify();
            return;
        }
        if drag.source_pane == destination_pane {
            return;
        }
        if self.panes[drag.source_pane].tab().id != drag.source_tab_id
            || self.panes[destination_pane].tab().id != destination_tab_id
            || self.panes[drag.source_pane].tab().terminal.is_some()
        {
            self.notice = Some("A pane changed during the drag. Try the copy again.".into());
            cx.notify();
            return;
        }
        self.queue_dropped_copy(drag.sources, destination_pane, destination_tab_id, cx);
    }

    pub(super) fn drop_external_copy(
        &mut self,
        paths: ExternalPaths,
        destination_pane: usize,
        destination_tab_id: u64,
        cx: &mut Context<Self>,
    ) {
        let sources = paths.paths().iter().cloned().map(Location::Local).collect();
        self.queue_dropped_copy(sources, destination_pane, destination_tab_id, cx);
    }

    fn queue_dropped_copy(
        &mut self,
        sources: Vec<Location>,
        destination_pane: usize,
        destination_tab_id: u64,
        cx: &mut Context<Self>,
    ) {
        if !self.panes.contains(destination_pane)
            || self.panes[destination_pane].tab().id != destination_tab_id
            || self.panes[destination_pane].tab().terminal.is_some()
        {
            self.notice = Some("The destination pane changed during the drag. Try again.".into());
            cx.notify();
            return;
        }
        let destination = self.panes[destination_pane].tab().path.clone();
        if sources.is_empty() || sources.iter().any(|source| source == &destination) {
            self.notice = Some("Choose items that are not the destination folder itself.".into());
            cx.notify();
            return;
        }

        let count = sources.len();
        self.transfers.enqueue(OperationPlan {
            operation: Operation::Copy,
            sources,
            destination: Some(destination.clone()),
            new_name: None,
            conflict_policy: ConflictPolicy::Ask,
        });
        self.notice = Some(format!(
            "Queued a copy of {count} item{} to {}. Dragging copies by default.",
            if count == 1 { "" } else { "s" },
            destination.display()
        ));
        self.transfer_drawer = true;
        self.run_transfers(cx);
    }

    pub(super) fn start_transfer_poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let manager = self.transfers.clone();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(180))
                    .await;
                let snapshot_manager = manager.clone();
                let jobs = cx
                    .background_executor()
                    .spawn(async move { snapshot_manager.snapshots() })
                    .await;
                if this
                    .update_in(cx, |this, window, cx| {
                        let changed = jobs.iter().any(|job| {
                            this.jobs
                                .iter()
                                .find(|old| old.id == job.id)
                                .is_none_or(|old| {
                                    old.state != job.state
                                        || old.completed_items != job.completed_items
                                        || old.bytes_copied != job.bytes_copied
                                })
                        });
                        let finished = jobs.iter().any(|job| {
                            matches!(
                                job.state,
                                JobState::Completed | JobState::Failed | JobState::Cancelled
                            ) && this
                                .jobs
                                .iter()
                                .find(|old| old.id == job.id)
                                .is_none_or(|old| old.state != job.state)
                        });
                        this.jobs = jobs;
                        if let Some(OperationDialog::Conflict(id, ..) | OperationDialog::Replace(id, ..)) = &this.operation_dialog {
                            if !this.jobs.iter().any(|job| job.id == *id && matches!(job.state, JobState::AwaitingConflict { .. })) {
                                this.operation_dialog = None;
                                this.activate(this.active, window, cx);
                            }
                        }

                        if this.operation_dialog.is_none() && this.connection_screen.is_none()
                            && !this.settings_open && !this.palette && !this.shortcuts_open {
                            if let Some((id, source, destination)) = this.jobs.iter().find_map(|job| {
                                if let JobState::AwaitingConflict { source, destination } = &job.state {
                                    Some((job.id, source.clone(), destination.clone()))
                                } else { None }
                            }) {
                                this.operation_dialog = Some(OperationDialog::Conflict(id, source, destination));
                                this.operation_focus.focus(window, cx);
                                cx.notify();
                            }
                        }
                        if finished {
                            this.sftp_listing_cache.clear();
                            this.listing_cache_epoch += 1;
                            for i in this.panes.ids() {
                                let active = this.panes[i].active;
                                for tab in 0..this.panes[i].tabs.len() {
                                    this.panes[i].active = tab;
                                    this.request_listing(i, cx);
                                }
                                this.panes[i].active = active;
                            }
                        }
                        if changed {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }
    pub(super) fn begin_operation(
        &mut self,
        operation: Operation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.panes[self.active].tab().terminal.is_some()
            || (matches!(operation, Operation::Copy | Operation::Move)
                && self.panes[self.transfer_target()].tab().terminal.is_some())
        {
            self.notice = Some(
                "Select a file tab in the source and destination panes before a file operation."
                    .into(),
            );
            cx.notify();
            return;
        }
        let operation =
            if operation == Operation::Trash && !self.panes[self.active].tab().path.is_local() {
                Operation::Delete
            } else {
                operation
            };
        if operation == Operation::Move
            && (!self.panes[self.active].tab().path.is_local()
                || !self.panes[self.transfer_target()].tab().path.is_local())
        {
            self.notice=Some("Remote moves are unsupported. Copy first, verify the destination, then explicitly delete the source.".into());
            cx.notify();
            return;
        }
        if matches!(self.panes[self.active].tab().path, Location::S3 { .. })
            && operation == Operation::CreateDirectory
        {
            self.notice = Some(
                "S3 has object keys and prefixes; creating a filesystem directory is unsupported."
                    .into(),
            );
            cx.notify();
            return;
        }
        let tab = self.panes[self.active].tab();
        let selected = tab
            .selected
            .iter()
            .filter_map(|row| tab.entries.get(*row).map(|e| e.location.clone()))
            .collect::<Vec<_>>();
        // Expanded tree rows can select a folder and its contents; act on the folder once.
        let sources = selected
            .iter()
            .filter(|location| {
                !selected
                    .iter()
                    .any(|other| super::tree::is_descendant(location, other))
            })
            .cloned()
            .collect::<Vec<_>>();
        if operation != Operation::CreateDirectory && sources.is_empty() {
            self.notice = Some("Select the items to operate on first.".into());
            cx.notify();
            return;
        }
        if operation == Operation::Rename && sources.len() != 1 {
            self.notice = Some("Select exactly one item to rename.".into());
            cx.notify();
            return;
        }
        let name = if operation == Operation::Rename {
            tab.entries[*tab.selected.first().unwrap()]
                .name
                .to_string_lossy()
                .into_owned()
        } else {
            String::new()
        };
        let destination = match operation {
            Operation::CreateDirectory => Some(tab.path.clone()),
            // Nested tree rows rename within their own folder, not the pane root.
            Operation::Rename => Some(sources[0].parent().unwrap_or_else(|| tab.path.clone())),
            Operation::Copy | Operation::Move => {
                Some(self.panes[self.transfer_target()].tab().path.clone())
            }
            Operation::Trash | Operation::Delete => None,
        };
        self.notice = None;
        self.operation_dialog = Some(OperationDialog::Plan(OperationPlan {
            operation,
            sources,
            destination,
            new_name: None,
            conflict_policy: ConflictPolicy::Ask,
        }));
        self.palette = false;
        self.operation_input.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            if matches!(operation, Operation::CreateDirectory | Operation::Rename) {
                input.focus(window, cx);
            }
        });
        if !matches!(operation, Operation::CreateDirectory | Operation::Rename) {
            self.operation_focus.focus(window, cx);
        }
        cx.notify();
    }
    pub(super) fn submit_operation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.operation_dialog.clone() else {
            return;
        };
        match dialog {
            OperationDialog::Plan(mut plan) => {
                if matches!(
                    plan.operation,
                    Operation::CreateDirectory | Operation::Rename
                ) {
                    let name = self.operation_input.read(cx).value().to_string();
                    if name.is_empty()
                        || name == "."
                        || name == ".."
                        || name.contains('/')
                        || name.contains('\0')
                    {
                        self.notice = Some(
                            "Enter a single non-empty name, without / or a null character.".into(),
                        );
                        cx.notify();
                        return;
                    }
                    let original = plan.sources.first().and_then(|source| {
                        if let Location::Local(path) = source {
                            path.file_name()
                        } else {
                            None
                        }
                    });
                    plan.new_name = Some(
                        if plan.operation == Operation::Rename
                            && original.is_some_and(|original| original.to_string_lossy() == name)
                        {
                            original.unwrap().to_os_string()
                        } else {
                            name.into()
                        },
                    );
                }
                self.notice = None;
                self.transfers.enqueue(plan);
                self.run_transfers(cx);
            }
            OperationDialog::Conflict(..) => return,
            OperationDialog::Replace(id, _, destination) => {
                if !destination.is_local() {
                    self.notice = Some(
                        "Remote replacement is unsupported. Choose Keep both or Skip instead."
                            .into(),
                    );
                    cx.notify();
                    return;
                }
                self.transfers.resolve_conflict(id, ConflictPolicy::Replace);
                self.run_transfers(cx);
            }
        }
        self.operation_dialog = None;
        self.transfer_drawer = true;
        self.activate(self.active, window, cx);
    }
    fn run_transfers(&mut self, cx: &mut Context<Self>) {
        let manager = self.transfers.clone();
        cx.background_executor()
            .spawn(async move { manager.run_pending() })
            .detach();
        cx.notify();
    }
    fn resolve(&mut self, id: JobId, policy: ConflictPolicy, cx: &mut Context<Self>) {
        self.transfers.resolve_conflict(id, policy);
        self.run_transfers(cx);
    }
    pub(super) fn render_operation_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.colors;
        let dialog = self.operation_dialog.as_ref().unwrap();
        if let OperationDialog::Conflict(id, source, destination) = dialog {
            return self.render_conflict_modal(*id, source, destination, cx);
        }
        let (title, paths, input, label) = match dialog {
            OperationDialog::Conflict(..) => unreachable!(),
            OperationDialog::Plan(plan) => {
                let mut paths = plan.sources.iter().map(|source| format!("Source: {}",source.display())).collect::<Vec<_>>();
                if let Some(destination) = &plan.destination {
                    paths.push(format!("Destination folder: {}", destination.display()));
                }
                if plan.operation==Operation::Delete{paths.push("PERMANENT remote deletion. Regular files or empty directories only; no Trash or automatic undo. S3 deletes the current object and can create a delete marker in a versioned bucket.".into());}
                if plan.operation == Operation::Trash {
                    paths.push("Local destination: macOS Trash. Remote deletion is permanent and requires confirmation.".into());
                }
                if plan.operation == Operation::Move {
                    paths.push("Sources are removed only after the destination is verified.".into());
                }
                let input = matches!(plan.operation, Operation::CreateDirectory | Operation::Rename);
                let label = match plan.operation { Operation::Copy => "Confirm copy", Operation::Move => "Confirm move", Operation::Delete => "Confirm delete", Operation::Trash => "Move to Trash", Operation::Rename => "Rename", Operation::CreateDirectory => "Create folder" };
                (operation_name(plan.operation).to_string(), paths, input, label)
            }
            OperationDialog::Replace(_, source, destination) => (
                "Replace existing file?".into(),
                vec![format!("Source: {}", source.display()), format!("Replace: {}", destination.display()), "Applies to remaining regular-file conflicts in this job. Directory replacement is unsupported.".into()],
                false,
                "Confirm replace",
            ),
        };
        let panel = div()
            .track_focus(&self.operation_focus)
            .key_context("OperationConfirm")
            .on_action(cx.listener(|this, _: &ConfirmOperation, window, cx| {
                this.submit_operation(window, cx);
            }))
            .flex_none()
            .p_3()
            .bg(rgb(theme.dialog))
            .border_t_1()
            .border_color(rgb(theme.accent))
            .child(div().text_color(rgb(theme.accent)).child(title))
            .child(
                div()
                    .id("operation-paths")
                    .max_h(px(150.))
                    .overflow_y_scroll()
                    .children(paths.into_iter().map(|path| div().py_1().child(path))),
            )
            .when(input, |d| d.child(Input::new(&self.operation_input)))
            .child(
                div()
                    .mt_2()
                    .flex()
                    .gap_4()
                    .child(
                        Button::new("confirm-operation")
                            .label(format!("{label} · Enter"))
                            .accessibility_label(label)
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.submit_operation(window, cx)
                                }),
                            ),
                    )
                    .child(
                        Button::new("cancel-operation")
                            .ghost()
                            .label("Cancel · Esc")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.operation_dialog = None;
                                this.activate(this.active, window, cx);
                            })),
                    ),
            )
            .when_some(self.notice.as_ref(), |panel, notice| {
                panel.child(div().mt_2().text_color(rgb(theme.warning)).child(notice.clone()))
            });
        let weak = cx.entity().downgrade();
        gpui_kit::base::Dialog::new(cx)
            .focus_handle(self.operation_focus.clone())
            .on_ok(|_, _, _| false)
            .close_on_backdrop_press(false)
            .popup(gpui_kit::base::DialogPopup::new()
                .w(px(720.)).max_w(gpui::relative(0.95)).rounded_lg().child(panel))
            .backdrop(div().size_full().bg(rgba(0x00000080)))
            .on_open_change(move |open, _, window, cx| {
                if !open {
                    let _ = weak.update(cx, |this, cx| {
                        this.operation_dialog = None;
                        this.activate(this.active, window, cx);
                        cx.notify();
                    });
                }
            })
            .into_any_element()
    }
    fn render_conflict_modal(&self, id: JobId, source: &Location, destination: &Location, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.colors;
        let can_replace = self.jobs.iter().find(|job| job.id == id).is_some_and(|job|
            source.is_local() && destination.is_local() && job.plan.sources.iter().all(Location::is_local)
                && job.plan.destination.as_ref().is_none_or(Location::is_local));
        let source = source.clone();
        let destination = destination.clone();
        let panel = div().track_focus(&self.operation_focus).key_context("TransferConflict")
            .p_3().bg(rgb(theme.dialog))
            .child(div().text_color(rgb(theme.accent)).child("Destination already exists"))
            .child(div().py_2().child(format!("Source: {}", source.display())))
            .child(div().py_2().child(format!("Destination: {}", destination.display())))
            .child(div().text_color(rgb(theme.muted)).child("Choice applies to remaining conflicts in this job. Replacement requires another confirmation."))
            .child(div().mt_3().flex().gap_2().children([
                (ConflictPolicy::KeepBoth, "Keep both"), (ConflictPolicy::Skip, "Skip"), (ConflictPolicy::Cancel, "Cancel job")
            ].into_iter().enumerate().map(|(n, (policy, label))| {
                Button::new(("modal-conflict-choice", n)).label(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.resolve(id, policy, cx);
                        this.operation_dialog = None;
                        this.activate(this.active, window, cx);
                    }))
            })).child(Button::new("modal-review-replace").label("Review replace…").disabled(!can_replace)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.operation_dialog = Some(OperationDialog::Replace(id, source.clone(), destination.clone()));
                    this.operation_focus.focus(window, cx);
                    cx.notify();
                }))));
        gpui_kit::base::Dialog::new(cx).focus_handle(self.operation_focus.clone())
            .on_ok(|_, _, _| false).close_on_escape(false).close_on_backdrop_press(false)
            .popup(gpui_kit::base::DialogPopup::new().w(px(720.)).max_w(gpui::relative(0.95)).rounded_lg().child(panel))
            .backdrop(div().size_full().bg(rgba(0x00000080))).into_any_element()
    }
    pub(super) fn render_transfers(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.colors;
        let font_size = self.preferences.appearance.font_size;
        div()
            .id("transfer-drawer")
            .h(px(180.))
            .flex_none()
            .overflow_y_scroll()
            .p_3()
            .bg(rgb(theme.surface))
            .border_t_1()
            .border_color(rgb(theme.border))
            .child(
                div()
                    .text_color(rgb(theme.accent))
                    .child("Transfers · ⌘J Hide"),
            )
            .when(self.jobs.is_empty(), |d| {
                d.child("No jobs queued. F5 Copy · F6 Move · ⌘⇧N New folder")
            })
            .children(self.jobs.iter().rev().map(|job| {
                let id = job.id;
                let status = match &job.state {
                    JobState::Queued => "Queued",
                    JobState::Running => "Running",
                    JobState::AwaitingConflict { .. } => "Conflict needs a decision",
                    JobState::Completed => "Completed",
                    JobState::Cancelled => "Cancelled",
                    JobState::Failed => "Failed",
                };
                let mut row = div()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(theme.border))
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(format!(
                                "#{} {} · {} · {}/{} items · {} copied",
                                id,
                                operation_name(job.plan.operation),
                                status,
                                job.completed_items,
                                job.total_items,
                                bytes(Some(job.bytes_copied))
                            ))
                            .when(
                                matches!(
                                    job.state,
                                    JobState::Queued
                                        | JobState::Running
                                        | JobState::AwaitingConflict { .. }
                                ),
                                |d| {
                                    d.child(
                                        div()
                                            .id(("cancel-job", id))
                                            .cursor_pointer()
                                            .text_color(rgb(theme.accent))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.transfers.cancel(id);
                                                cx.notify();
                                            }))
                                            .child("Cancel"),
                                    )
                                },
                            ),
                    );
                if let Some(current) = &job.current {
                    row = row.child(div().text_color(rgb(theme.muted)).child(current.display()));
                }
                if let Some(error) = job.error.as_ref().filter(|_| !matches!(job.state, JobState::AwaitingConflict { .. })) {
                    row = row.child(div().text_color(rgb(theme.error)).child(error.to_string()));
                }
                if let JobState::AwaitingConflict {
                    source,
                    destination,
                } = &job.state
                {
                    let source = source.clone();
                    let destination = destination.clone();
                    row = row.child(Button::new(("review-conflict", id)).label("Review conflict…")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.operation_dialog = Some(OperationDialog::Conflict(id, source.clone(), destination.clone()));
                            this.operation_focus.focus(window, cx);
                            cx.notify();
                        })));

                }
                if !job.journal.is_empty() {
                    row = row.children(job.journal.iter().map(|item| {
                        div()
                            .text_size(px((font_size - 1.).max(10.)))
                            .text_color(rgb(theme.muted))
                            .child(format!(
                                "{} · {} → {}",
                                item.description,
                                item.source
                                    .as_ref()
                                    .map(|p| p.display())
                                    .unwrap_or_default(),
                                item.destination
                                    .as_ref()
                                    .map(|p| p.display())
                                    .unwrap_or_default()
                            ))
                    }));
                }
                row
            }))
    }
}
impl Workspace {
    pub(super) fn decide_conflict(
        &mut self,
        policy: ConflictPolicy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let conflict = self.jobs.iter().find_map(|job| {
            if let JobState::AwaitingConflict {
                source,
                destination,
            } = &job.state
            {
                Some((job.id, source.clone(), destination.clone()))
            } else {
                None
            }
        });
        if let Some((id, source, destination)) = conflict {
            if policy == ConflictPolicy::Replace {
                self.operation_dialog = Some(OperationDialog::Replace(id, source, destination));
                self.operation_focus.focus(window, cx);
            } else {
                self.resolve(id, policy, cx);
            }
        } else {
            self.notice = Some("There is no pending conflict.".into());
        }
        cx.notify();
    }
    pub(super) fn cancel_transfer(&mut self, cx: &mut Context<Self>) {
        if let Some(job) = self
            .jobs
            .iter()
            .find(|job| {
                matches!(
                    job.state,
                    JobState::Running | JobState::AwaitingConflict { .. }
                )
            })
            .or_else(|| self.jobs.iter().find(|job| job.state == JobState::Queued))
        {
            self.transfers.cancel(job.id);
        } else {
            self.notice = Some("There is no active transfer.".into());
        }
        cx.notify();
    }
}
