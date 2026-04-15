use super::*;

impl OcttyApp {
    pub(crate) fn add_shell_pane(
        &mut self,
        _: &AddShellPane,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_pane(PaneType::Shell, cx);
    }

    pub(crate) fn add_diff_pane(
        &mut self,
        _: &AddDiffPane,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_pane(PaneType::Diff, cx);
    }

    pub(crate) fn add_note_pane(
        &mut self,
        _: &AddNotePane,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_pane(PaneType::Note, cx);
    }

    pub(crate) fn paste_terminal_clipboard(
        &mut self,
        _: &PasteTerminalClipboard,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(clipboard) = cx.read_from_clipboard() {
            match terminal_clipboard_paste_text(&clipboard) {
                Ok(Some(text)) => {
                    self.send_bytes_to_active_terminal(terminal_paste_bytes(&text), cx);
                }
                Ok(None) => {}
                Err(error) => {
                    self.show_error(format!("Clipboard paste failed: {error:#}"), cx);
                }
            }
        }
        cx.stop_propagation();
    }

    pub(crate) fn copy_terminal_selection(
        &mut self,
        _: &CopyTerminalSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_terminal_selection_to_clipboard(false, cx);
        cx.stop_propagation();
    }

    pub(crate) fn cut_terminal_selection(
        &mut self,
        _: &CutTerminalSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_terminal_selection_to_clipboard(true, cx);
        cx.stop_propagation();
    }

    pub(crate) fn start_terminal_selection(
        &mut self,
        live_key: &str,
        point: TerminalGridPoint,
        mode: TerminalSelectionMode,
        cx: &mut Context<Self>,
    ) {
        self.activate_terminal_key(live_key, cx);
        let Some(live) = self.live_terminals.get_mut(live_key) else {
            return;
        };
        live.selection = None;
        live.selection_drag = Some(TerminalSelectionDrag { anchor: point });
        if mode.rectangular || mode.filter_indent {
            live.selection = Some(TerminalSelection {
                anchor: point,
                active: point,
                mode,
            });
        }
        cx.notify();
    }

    pub(crate) fn update_terminal_selection(
        &mut self,
        live_key: &str,
        point: TerminalGridPoint,
        mode: TerminalSelectionMode,
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.live_terminals.get_mut(live_key) else {
            return;
        };
        let Some(drag) = live.selection_drag.clone() else {
            return;
        };
        if drag.anchor == point {
            live.selection = None;
            cx.notify();
            return;
        }
        let selection = TerminalSelection {
            anchor: drag.anchor,
            active: point,
            mode,
        };
        let text = live
            .latest
            .as_ref()
            .map(|snapshot| terminal_selection_text(snapshot, &selection))
            .unwrap_or_default();
        live.selection = Some(selection);
        write_terminal_primary_text(text, cx);
        cx.notify();
    }

    pub(crate) fn finish_terminal_selection(&mut self, live_key: &str, cx: &mut Context<Self>) {
        let Some(live) = self.live_terminals.get_mut(live_key) else {
            return;
        };
        live.selection_drag = None;
        if let (Some(snapshot), Some(selection)) = (live.latest.as_ref(), live.selection.as_ref()) {
            write_terminal_primary_text(terminal_selection_text(snapshot, selection), cx);
        }
        cx.notify();
    }

    pub(crate) fn paste_terminal_primary_selection(
        &mut self,
        live_key: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = read_terminal_primary_text(cx) else {
            return;
        };
        self.send_bytes_to_terminal_key(live_key, terminal_paste_bytes(&text), cx);
        cx.stop_propagation();
    }

    pub(crate) fn copy_terminal_selection_to_clipboard(
        &mut self,
        clear_selection: bool,
        cx: &mut Context<Self>,
    ) {
        let Some((live_key, text)) = self.active_workspace_selection_text() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        write_terminal_primary_text(text, cx);
        if clear_selection && let Some(live) = self.live_terminals.get_mut(&live_key) {
            live.selection = None;
            live.selection_drag = None;
            cx.notify();
        }
    }

    pub(crate) fn active_workspace_selection_text(&self) -> Option<(String, String)> {
        let workspace = self.active_workspace()?;
        let snapshot = self.active_snapshot.as_ref()?;
        if let Some(pane_id) = active_terminal_pane_id(snapshot) {
            let live_key = live_terminal_key(&workspace.id, &pane_id);
            if let Some(text) = self.live_terminal_selection_text(&live_key) {
                return Some((live_key, text));
            }
        }

        self.live_terminals
            .keys()
            .filter(|live_key| {
                split_live_terminal_key(live_key)
                    .is_some_and(|(workspace_id, _)| workspace_id == workspace.id)
            })
            .find_map(|live_key| {
                self.live_terminal_selection_text(live_key)
                    .map(|text| (live_key.clone(), text))
            })
    }

    pub(crate) fn live_terminal_selection_text(&self, live_key: &str) -> Option<String> {
        let live = self.live_terminals.get(live_key)?;
        let snapshot = live.latest.as_ref()?;
        let selection = live.selection.as_ref()?;
        let text = terminal_selection_text(snapshot, selection);
        (!text.is_empty()).then_some(text)
    }

    pub(crate) fn activate_terminal_key(&mut self, live_key: &str, cx: &mut Context<Self>) {
        let Some((workspace_id, pane_id)) = split_live_terminal_key(live_key) else {
            return;
        };
        let Some(workspace) = self.active_workspace() else {
            return;
        };
        if workspace.id != workspace_id {
            return;
        }
        if let Some(snapshot) = self.active_snapshot.as_mut()
            && snapshot.panes.contains_key(pane_id)
        {
            snapshot.active_pane_id = Some(pane_id.to_owned());
            self.record_active_pane_seen(cx);
        }
    }

    pub(crate) fn navigate_pane(
        &mut self,
        action: &NavigatePane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window);
        let Some(snapshot) = self.active_snapshot.as_mut() else {
            return;
        };
        let Some(target_pane_id) = pane_navigation_target(snapshot, action.direction) else {
            return;
        };
        if snapshot.active_pane_id.as_deref() == Some(target_pane_id.as_str()) {
            return;
        }

        snapshot.active_pane_id = Some(target_pane_id);
        snapshot.updated_at = now_ms();
        let snapshot_to_save = snapshot.clone();
        self.record_active_pane_seen(cx);
        self.save_workspace_snapshot(
            snapshot_to_save,
            "Selected pane, but failed to save focus",
            cx,
        );
        cx.notify();
    }

    pub(crate) fn close_active_pane(
        &mut self,
        _: &CloseActivePane,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.active_snapshot.clone() else {
            return;
        };
        let Some(pane_id) = snapshot.active_pane_id.clone() else {
            return;
        };

        let terminal_session_id =
            snapshot
                .panes
                .get(&pane_id)
                .and_then(|pane| match &pane.payload {
                    PanePayload::Terminal(payload) => payload.session_id.clone(),
                    _ => None,
                });

        let workspace_id = snapshot.workspace_id.clone();

        match remove_pane(snapshot, &pane_id) {
            Ok(snapshot) => {
                let live_key = live_terminal_key(&workspace_id, &pane_id);
                let live_session_id = self
                    .live_terminals
                    .remove(&live_key)
                    .map(|live| live.handle.session_id().to_owned());
                self.status = format!("Closed pane {pane_id}.").into();
                self.active_snapshot = Some(snapshot.clone());
                self.ensure_live_terminals_for_active_snapshot(cx);
                self.schedule_terminal_snapshot_notifications(cx);
                self.delete_pane_activity(&workspace_id, &pane_id, cx);
                self.save_workspace_snapshot(
                    snapshot,
                    "Closed pane, but failed to save taskspace",
                    cx,
                );
                if let Some(session_id) = live_session_id.or(terminal_session_id) {
                    self.kill_terminal_session(session_id, cx);
                }
                cx.notify();
            }
            Err(error) => {
                self.show_error(format!("Failed to close pane: {error:#}"), cx);
            }
        }
    }

    pub(crate) fn resize_focused_column(
        &mut self,
        action: &ResizeFocusedColumn,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.active_snapshot.as_mut() else {
            return;
        };
        let Some(width_px) = resize_focused_column_in_snapshot(snapshot, action.direction) else {
            return;
        };

        let snapshot_to_save = snapshot.clone();
        self.status = format!("Column width: {}px.", width_px.round() as u32).into();
        self.save_workspace_snapshot(
            snapshot_to_save,
            "Resized column, but failed to save taskspace",
            cx,
        );
        cx.notify();
    }

    pub(crate) fn add_pane(&mut self, pane_type: PaneType, cx: &mut Context<Self>) {
        let Some(workspace) = self.active_workspace().cloned() else {
            self.show_error("No active workspace.", cx);
            return;
        };

        let snapshot = self
            .active_snapshot
            .take()
            .unwrap_or_else(|| create_default_snapshot(workspace.id.clone()));
        let pane = create_pane_state(pane_type, workspace.workspace_path.clone(), None);
        let pane_id = pane.id.clone();
        let snapshot = add_pane(snapshot, pane);
        let is_terminal = matches!(
            snapshot.panes.get(&pane_id).map(|pane| &pane.payload),
            Some(PanePayload::Terminal(_))
        );
        let (snapshot, terminal_started) = if is_terminal {
            match prepare_live_terminal_snapshot(&workspace, snapshot.clone(), &pane_id) {
                Ok(snapshot) => (snapshot, true),
                Err(error) => {
                    self.show_error(
                        format!("Added pane, but terminal metadata failed: {error:#}"),
                        cx,
                    );
                    (snapshot, false)
                }
            }
        } else {
            (snapshot, false)
        };
        if terminal_started {
            self.status = format!(
                "Started shell and saved {} pane(s) for {}.",
                snapshot.panes.len(),
                workspace.display_name_or_workspace_name()
            )
            .into();
        } else {
            self.status = format!(
                "Saved {} pane(s) for {}.",
                snapshot.panes.len(),
                workspace.display_name_or_workspace_name()
            )
            .into();
        }
        self.active_snapshot = Some(snapshot.clone());
        self.ensure_live_terminals_for_active_snapshot(cx);
        self.schedule_terminal_snapshot_notifications(cx);
        self.record_active_pane_seen(cx);
        self.save_workspace_snapshot(snapshot, "Failed to save taskspace", cx);
        cx.notify();
    }

    pub(crate) fn select_pane(
        &mut self,
        pane_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window);
        let snapshot_to_save = self.active_snapshot.as_mut().map(|snapshot| {
            snapshot.active_pane_id = Some(pane_id.to_owned());
            snapshot.updated_at = now_ms();
            snapshot.clone()
        });

        if let Some(snapshot) = snapshot_to_save {
            self.record_active_pane_seen(cx);
            self.save_workspace_snapshot(snapshot, "Selected pane, but failed to save focus", cx);
        }
        cx.notify();
    }

    pub(crate) fn save_workspace_snapshot(
        &self,
        snapshot: WorkspaceSnapshot,
        error_context: &'static str,
        cx: &mut Context<Self>,
    ) {
        let store = self.store.clone();
        cx.spawn(async move |this, cx| {
            let result = match gpui_tokio::Tokio::spawn_result(cx, async move {
                store.save_snapshot(&snapshot).await?;
                Ok(())
            }) {
                Ok(task) => task.await,
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                let _ = this.update(cx, |app, cx| {
                    app.show_error(format!("{error_context}: {error:#}"), cx);
                });
            }
        })
        .detach();
    }

    pub(crate) fn kill_terminal_session(&self, session_id: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let session_id_for_error = session_id.clone();
            let result = match gpui_tokio::Tokio::spawn_result(cx, async move {
                kill_tmux_session(&session_id).await?;
                Ok(())
            }) {
                Ok(task) => task.await,
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                let _ = this.update(cx, |app, cx| {
                    app.show_error(
                        format!(
                            "Closed pane, but failed to stop {session_id_for_error}: {error:#}"
                        ),
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    pub(crate) fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sidebar_rename_dialog.is_some() {
            match event.keystroke.key.to_ascii_lowercase().as_str() {
                "enter" => self.confirm_sidebar_rename_dialog(cx),
                "escape" => self.cancel_sidebar_rename_dialog(cx),
                _ => {}
            }
            cx.stop_propagation();
            return;
        }

        if let Some(index) = workspace_shortcut_index_from_key_event(event) {
            self.open_workspace(&OpenWorkspaceShortcut { index }, window, cx);
            cx.stop_propagation();
            return;
        }

        let Some(input) = terminal_input_from_key_event(event) else {
            return;
        };
        self.send_input_to_active_terminal(input, cx);
        cx.stop_propagation();
    }

    pub(crate) fn forward_terminal_tab(
        &mut self,
        action: &ForwardTerminalTab,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sidebar_rename_dialog.is_some() {
            return;
        }

        self.send_input_to_active_terminal(
            TerminalInput::LiveKey(terminal_tab_input(action.shift)),
            cx,
        );
    }

    pub(crate) fn send_input_to_active_terminal(
        &mut self,
        input: TerminalInput,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.active_workspace().cloned() else {
            return;
        };
        let Some(snapshot) = self.active_snapshot.clone() else {
            return;
        };
        let Some(pane_id) = active_terminal_pane_id(&snapshot) else {
            return;
        };
        let Ok(payload) = terminal_payload_for_pane(&snapshot, &pane_id).cloned() else {
            return;
        };

        let live_key = live_terminal_key(&workspace.id, &pane_id);
        let mut send_error = None;
        if let Some(live) = self.live_terminals.get_mut(&live_key) {
            match &input {
                TerminalInput::LiveKey(key_input) => {
                    if let Err(error) = live.handle.send_key(key_input.clone()) {
                        send_error = Some(format!("Terminal input failed: {error:#}"));
                    } else {
                        live.last_input_at = Some(Instant::now());
                    }
                }
            }
            if let Some(snapshot) = self.active_snapshot.as_mut() {
                snapshot.active_pane_id = Some(pane_id);
            }
            if let Some(send_error) = send_error {
                self.show_error(send_error, cx);
            }
            self.record_active_pane_seen(cx);
            return;
        }

        if let Some(snapshot) = self.active_snapshot.as_mut() {
            snapshot.active_pane_id = Some(pane_id.clone());
            preview_terminal_input(snapshot, &pane_id, &input);
        }
        let snapshot = self
            .active_snapshot
            .clone()
            .unwrap_or_else(|| snapshot.clone());

        self.pending_terminal_inputs.push(PendingTerminalInput {
            workspace,
            snapshot,
            pane_id,
            payload,
            input,
        });
        self.record_active_pane_seen(cx);
        self.schedule_terminal_flush(cx);
        cx.notify();
    }

    pub(crate) fn send_bytes_to_active_terminal(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if bytes.is_empty() {
            return;
        }
        let Some(workspace) = self.active_workspace().cloned() else {
            return;
        };
        let Some(snapshot) = self.active_snapshot.clone() else {
            return;
        };
        let Some(pane_id) = active_terminal_pane_id(&snapshot) else {
            return;
        };

        let live_key = live_terminal_key(&workspace.id, &pane_id);
        self.send_bytes_to_terminal_key(&live_key, bytes, cx);
        if let Some(snapshot) = self.active_snapshot.as_mut() {
            snapshot.active_pane_id = Some(pane_id);
        }
        self.record_active_pane_seen(cx);
    }

    pub(crate) fn send_bytes_to_terminal_key(
        &mut self,
        live_key: &str,
        bytes: Vec<u8>,
        cx: &mut Context<Self>,
    ) {
        if bytes.is_empty() {
            return;
        }
        let Some(live) = self.live_terminals.get_mut(live_key) else {
            return;
        };
        if let Err(error) = live.handle.send_bytes(bytes) {
            self.show_error(format!("Terminal paste failed: {error:#}"), cx);
            return;
        }
        live.last_input_at = Some(Instant::now());
    }

    pub(crate) fn schedule_terminal_flush(&mut self, cx: &mut Context<Self>) {
        if self.terminal_flush_active {
            return;
        }

        self.terminal_flush_active = true;
        let timer = cx.background_executor().timer(Duration::from_millis(8));
        cx.spawn(async move |this, cx| {
            timer.await;
            loop {
                let Some((store, pending)) = this
                    .update(cx, |app, _cx| {
                        let pending = std::mem::take(&mut app.pending_terminal_inputs);
                        if pending.is_empty() {
                            app.terminal_flush_active = false;
                            None
                        } else {
                            Some((app.store.clone(), pending))
                        }
                    })
                    .ok()
                    .flatten()
                else {
                    break;
                };

                let result = match gpui_tokio::Tokio::spawn_result(
                    cx,
                    flush_terminal_inputs(store, pending),
                ) {
                    Ok(flush) => flush.await,
                    Err(error) => Err(error),
                };

                let _ = this.update(cx, |app, cx| {
                    match result {
                        Ok(snapshots) => app.apply_terminal_flush_snapshots(snapshots),
                        Err(error) => {
                            app.show_error(format!("Terminal input failed: {error:#}"), cx);
                        }
                    }
                    cx.notify();
                });

                cx.background_executor()
                    .timer(Duration::from_millis(8))
                    .await;
            }
        })
        .detach();
    }

    pub(crate) fn apply_terminal_flush_snapshots(&mut self, snapshots: Vec<WorkspaceSnapshot>) {
        let pending_panes: BTreeSet<_> = self
            .pending_terminal_inputs
            .iter()
            .map(|pending| pending.pane_id.as_str())
            .collect();
        let Some(active_snapshot) = self.active_snapshot.as_mut() else {
            return;
        };

        for snapshot in snapshots {
            if snapshot.workspace_id != active_snapshot.workspace_id {
                continue;
            }

            for (pane_id, pane) in snapshot.panes {
                if pending_panes.contains(pane_id.as_str()) {
                    continue;
                }
                let Some(current_pane) = active_snapshot.panes.get_mut(&pane_id) else {
                    continue;
                };
                let PanePayload::Terminal(updated_payload) = pane.payload else {
                    continue;
                };
                let PanePayload::Terminal(current_payload) = &mut current_pane.payload else {
                    continue;
                };
                current_payload.session_id = updated_payload.session_id;
                current_payload.session_state = updated_payload.session_state;
                current_payload.exit_code = updated_payload.exit_code;
                current_payload.restored_buffer = updated_payload.restored_buffer;
            }
            active_snapshot.updated_at = now_ms();
        }
    }
}
