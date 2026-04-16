use super::*;

impl LiveTerminalPane {
    pub(crate) fn scroll_viewport(&self, lines: isize) {
        let _ = self.handle.scroll(lines);
    }
}

impl OcttyApp {
    pub(crate) fn ensure_live_terminals_for_active_snapshot(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.active_workspace().cloned() else {
            return;
        };
        let Some(snapshot) = self.active_snapshot.as_mut() else {
            return;
        };

        let pane_specs: Vec<_> = snapshot
            .panes
            .iter_mut()
            .filter_map(|(pane_id, pane)| {
                let PanePayload::Terminal(payload) = &mut pane.payload else {
                    return None;
                };
                let (cols, rows) = default_terminal_grid_for_pane();
                let spec = terminal_spec_for_payload(&workspace, pane_id, payload, cols, rows);
                let session_id = payload
                    .session_id
                    .clone()
                    .unwrap_or_else(|| stable_retach_session_name(&spec));
                payload.session_id = Some(session_id);
                payload.session_state = SessionState::Live;
                Some((pane_id.clone(), spec))
            })
            .collect();

        for (pane_id, spec) in pane_specs {
            let key = live_terminal_key(&workspace.id, &pane_id);
            if self.live_terminals.contains_key(&key) || self.failed_live_terminals.contains(&key) {
                continue;
            }
            let notify_tx = Arc::new(Mutex::new(self.terminal_snapshot_tx.clone()));
            let notifier = LiveTerminalSnapshotNotifier::new(move || {
                if let Ok(tx) = notify_tx.lock() {
                    let _ = tx.unbounded_send(());
                }
            });
            match spawn_live_terminal_with_notifier(spec, notifier) {
                Ok(handle) => {
                    self.failed_live_terminals.remove(&key);
                    self.live_terminals.insert(
                        key,
                        LiveTerminalPane {
                            handle,
                            latest: None,
                            pending_snapshot: None,
                            last_presented_snapshot_at: None,
                            last_resize: None,
                            last_input_at: None,
                            latency: TerminalLatencyStats::default(),
                            selection: None,
                            selection_drag: None,
                        },
                    );
                }
                Err(error) => {
                    self.failed_live_terminals.insert(key);
                    self.show_error(format!("Failed to start live terminal: {error:#}"), cx);
                }
            }
        }
    }

    pub(crate) fn schedule_terminal_snapshot_notifications(&mut self, cx: &mut Context<Self>) {
        if self.terminal_notifications_active {
            return;
        }
        let Some(mut notification_rx) = self.terminal_snapshot_rx.take() else {
            return;
        };

        self.terminal_notifications_active = true;
        cx.spawn(async move |this, cx| {
            while notification_rx.next().await.is_some() {
                drain_pending_terminal_notifications(&mut notification_rx);
                let now = Instant::now();
                let delay = this
                    .update(cx, |app, _cx| app.terminal_snapshot_coalesce_interval(now))
                    .unwrap_or(TERMINAL_BACKGROUND_FRAME_INTERVAL);
                cx.background_executor().timer(delay).await;
                drain_pending_terminal_notifications(&mut notification_rx);
                let _ = this.update(cx, |app, cx| {
                    let now = Instant::now();
                    let result = app.drain_live_terminal_snapshots(now, cx);
                    if let Some(delay) = result.deferred_delay {
                        app.schedule_deferred_terminal_snapshot(delay, cx);
                    }
                    if result.changed {
                        app.terminal_last_snapshot_notify_at = Some(now);
                        cx.notify();
                    }
                });
            }

            let _ = this.update(cx, |app, _cx| {
                app.terminal_notifications_active = false;
            });
        })
        .detach();
    }

    pub(crate) fn schedule_deferred_terminal_snapshot(
        &mut self,
        delay: Duration,
        cx: &mut Context<Self>,
    ) {
        if self.terminal_deferred_snapshot_timer_active {
            return;
        }
        self.terminal_deferred_snapshot_timer_active = true;
        let notify_tx = self.terminal_snapshot_tx.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = notify_tx.unbounded_send(());
            let _ = this.update(cx, |app, _cx| {
                app.terminal_deferred_snapshot_timer_active = false;
            });
        })
        .detach();
    }

    pub(crate) fn terminal_snapshot_coalesce_interval(&self, now: Instant) -> Duration {
        terminal_snapshot_coalesce_interval(
            self.terminal_window_active,
            self.has_recent_terminal_input(),
            self.terminal_last_snapshot_notify_at,
            now,
        )
    }

    pub(crate) fn has_recent_terminal_input(&self) -> bool {
        self.live_terminals.values().any(|live| {
            live.last_input_at
                .is_some_and(|input_at| input_at.elapsed() <= TERMINAL_INTERACTIVE_SNAPSHOT_WINDOW)
        })
    }

    pub(crate) fn drain_live_terminal_snapshots(
        &mut self,
        now: Instant,
        cx: &mut Context<Self>,
    ) -> TerminalSnapshotDrainResult {
        let mut result = TerminalSnapshotDrainResult::default();
        let Some(active_workspace) = self.active_workspace().cloned() else {
            return result;
        };
        let focused_live_key = self
            .active_snapshot
            .as_ref()
            .and_then(active_terminal_pane_id)
            .map(|pane_id| live_terminal_key(&active_workspace.id, &pane_id));
        let mut updates = Vec::new();
        let mut notifications = Vec::new();
        let mut exits = Vec::new();
        for (key, live) in &mut self.live_terminals {
            let selected = focused_live_key.as_deref() == Some(key.as_str());
            let panel_focused = self.terminal_window_active && selected;
            for exit in live.handle.drain_exits() {
                exits.push((key.clone(), exit));
            }
            for notification in live.handle.drain_notifications() {
                notifications.push((key.clone(), notification, panel_focused));
            }
            if let Some(snapshot) = coalesce_terminal_snapshots(live.handle.drain_snapshots()) {
                let input_at = live.last_input_at.take();
                if terminal_performance_data_enabled() {
                    if let Some(input_at) = input_at {
                        live.latency.record_key_to_snapshot(input_at.elapsed());
                    }
                    live.latency
                        .record_pty_to_snapshot(snapshot.timing.pty_to_snapshot_micros);
                    live.latency
                        .record_pty_output_bytes(snapshot.timing.pty_output_bytes);
                    live.latency
                        .record_vt_write(snapshot.timing.vt_write_micros);
                    live.latency
                        .record_snapshot_update(snapshot.timing.snapshot_update_micros);
                    live.latency
                        .record_snapshot_extract(snapshot.timing.snapshot_extract_micros);
                    live.latency
                        .record_snapshot_build(snapshot.timing.snapshot_build_micros);
                    live.latency.record_dirty_rows(snapshot.timing.dirty_rows);
                    live.latency.record_dirty_cells(snapshot.timing.dirty_cells);
                }
                live.pending_snapshot = Some(snapshot);
            }

            if let Some(mut snapshot) = take_presentable_terminal_snapshot(live, selected, now) {
                if split_live_terminal_key(key)
                    .is_none_or(|(workspace_id, _)| workspace_id != active_workspace.id)
                {
                    mark_terminal_snapshot_full_damage(&mut snapshot);
                }
                live.latest = Some(snapshot.clone());
                live.last_presented_snapshot_at = Some(now);
                updates.push((key.clone(), snapshot, panel_focused));
            } else if live.pending_snapshot.is_some()
                && let Some(delay) = terminal_snapshot_presentation_delay(live, selected, now)
            {
                result.defer_for(delay);
            }
        }

        let now_ms = now_ms();
        for (key, notification, panel_focused) in notifications {
            if let Some((workspace_id, pane_id)) = split_live_terminal_key(&key) {
                if panel_focused {
                    self.record_pane_seen(workspace_id, pane_id, now_ms, cx);
                    result.changed = true;
                    continue;
                }
                self.record_pane_attention(workspace_id, pane_id, now_ms, cx);
                result.changed = true;
            }
            show_desktop_notification(&notification);
        }

        for (key, snapshot, panel_focused) in updates {
            let Some((workspace_id, pane_id)) = split_live_terminal_key(&key) else {
                continue;
            };
            self.record_pane_activity(workspace_id, pane_id, now_ms, None, None, cx);
            result.changed = true;
            if panel_focused {
                self.record_pane_seen(workspace_id, pane_id, now_ms, cx);
            }
            if workspace_id != active_workspace.id {
                continue;
            }
            if let Some(active_snapshot) = self.active_snapshot.as_mut()
                && let Some(pane) = active_snapshot.panes.get_mut(pane_id)
                && let PanePayload::Terminal(payload) = &mut pane.payload
            {
                payload.session_id = Some(snapshot.session_id.clone());
                payload.session_state = SessionState::Live;
                payload.restored_buffer = snapshot.plain_text.clone();
                active_snapshot.updated_at = now_ms;
                result.changed = true;
            }
        }
        self.close_exited_live_terminal_panes(exits, cx, &mut result);
        result
    }

    pub(crate) fn sync_active_workspace_terminal_snapshots(
        &mut self,
        now: Instant,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(active_workspace) = self.active_workspace().cloned() else {
            return false;
        };

        let mut updates = Vec::new();
        for (key, live) in &mut self.live_terminals {
            let Some((workspace_id, _pane_id)) = split_live_terminal_key(key) else {
                continue;
            };
            if workspace_id != active_workspace.id {
                continue;
            }

            if let Some(mut snapshot) = coalesce_terminal_snapshots(live.handle.drain_snapshots()) {
                mark_terminal_snapshot_full_damage(&mut snapshot);
                live.pending_snapshot = Some(snapshot);
            }

            if let Some(mut snapshot) = live.pending_snapshot.take() {
                mark_terminal_snapshot_full_damage(&mut snapshot);
                live.latest = Some(snapshot.clone());
                live.last_presented_snapshot_at = Some(now);
                updates.push((key.clone(), snapshot));
            } else if let Some(snapshot) = live.latest.as_mut() {
                mark_terminal_snapshot_full_damage(snapshot);
                updates.push((key.clone(), snapshot.clone()));
            }
        }

        let mut changed = false;
        for (key, snapshot) in updates {
            let Some((workspace_id, pane_id)) = split_live_terminal_key(&key) else {
                continue;
            };
            self.record_pane_seen(workspace_id, pane_id, now_ms(), cx);
            if let Some(active_snapshot) = self.active_snapshot.as_mut()
                && let Some(pane) = active_snapshot.panes.get_mut(pane_id)
                && let PanePayload::Terminal(payload) = &mut pane.payload
            {
                payload.session_id = Some(snapshot.session_id.clone());
                payload.session_state = SessionState::Live;
                payload.restored_buffer = snapshot.plain_text.clone();
                active_snapshot.updated_at = now_ms();
                changed = true;
            }
        }
        changed
    }

    pub(crate) fn close_exited_live_terminal_panes(
        &mut self,
        exits: Vec<(String, LiveTerminalExit)>,
        cx: &mut Context<Self>,
        result: &mut TerminalSnapshotDrainResult,
    ) {
        for (key, exit) in exits {
            let Some((workspace_id, pane_id)) = split_live_terminal_key(&key)
                .map(|(workspace_id, pane_id)| (workspace_id.to_owned(), pane_id.to_owned()))
            else {
                continue;
            };
            self.live_terminals.remove(&key);
            self.failed_live_terminals.remove(&key);
            if self.close_exited_active_terminal_pane(&workspace_id, &pane_id, &exit, cx, result) {
                continue;
            }
            self.close_exited_stored_terminal_pane(workspace_id, pane_id, exit, cx);
        }
    }

    fn close_exited_active_terminal_pane(
        &mut self,
        workspace_id: &str,
        pane_id: &str,
        exit: &LiveTerminalExit,
        cx: &mut Context<Self>,
        result: &mut TerminalSnapshotDrainResult,
    ) -> bool {
        let Some(snapshot) = self.active_snapshot.take() else {
            return false;
        };
        if snapshot.workspace_id != workspace_id || !snapshot.panes.contains_key(pane_id) {
            self.active_snapshot = Some(snapshot);
            return false;
        }

        match remove_pane(snapshot.clone(), pane_id) {
            Ok(updated_snapshot) => {
                let exit_label = terminal_exit_status_label(exit.exit_code);
                self.status = format!("Closed pane {pane_id} after terminal {exit_label}.").into();
                self.active_snapshot = Some(updated_snapshot.clone());
                self.delete_pane_activity(workspace_id, pane_id, cx);
                self.save_workspace_snapshot(
                    updated_snapshot,
                    "Terminal exited, but failed to save taskspace",
                    cx,
                );
                result.changed = true;
                true
            }
            Err(error) => {
                self.active_snapshot = Some(snapshot);
                self.show_error(
                    format!("Terminal exited, but pane close failed: {error:#}"),
                    cx,
                );
                true
            }
        }
    }

    fn close_exited_stored_terminal_pane(
        &self,
        workspace_id: String,
        pane_id: String,
        exit: LiveTerminalExit,
        cx: &mut Context<Self>,
    ) {
        let store = self.store.clone();
        let workspace_id_for_store = workspace_id.clone();
        let pane_id_for_store = pane_id.clone();
        cx.spawn(async move |this, cx| {
            let result = match gpui_tokio::Tokio::spawn_result(cx, async move {
                let Some(snapshot) = store.get_snapshot(&workspace_id_for_store).await? else {
                    return Ok(false);
                };
                if !snapshot.panes.contains_key(&pane_id_for_store) {
                    return Ok(false);
                }
                let snapshot = remove_pane(snapshot, &pane_id_for_store)?;
                store.save_snapshot(&snapshot).await?;
                store
                    .delete_pane_activity(&workspace_id_for_store, &pane_id_for_store)
                    .await?;
                Ok::<bool, anyhow::Error>(true)
            }) {
                Ok(task) => task.await,
                Err(error) => Err(error),
            };

            let _ = this.update(cx, |app, cx| match result {
                Ok(true) => {
                    let exit_label = terminal_exit_status_label(exit.exit_code);
                    app.status =
                        format!("Closed pane {pane_id} after terminal {exit_label}.").into();
                    cx.notify();
                }
                Ok(false) => {}
                Err(error) => {
                    app.show_error(
                        format!(
                            "Terminal {} exited, but failed to save pane close: {error:#}",
                            exit.session_id
                        ),
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    pub(crate) fn resize_live_terminal(
        &mut self,
        workspace_id: &str,
        pane_id: &str,
        cols: u16,
        rows: u16,
    ) {
        let key = live_terminal_key(workspace_id, pane_id);
        let Some(live) = self.live_terminals.get_mut(&key) else {
            return;
        };
        if live.last_resize == Some((cols, rows)) {
            return;
        }
        live.last_resize = Some((cols, rows));
        let _ = live.handle.resize(TerminalResize {
            cols,
            rows,
            pixel_width: cols.saturating_mul(TERMINAL_CELL_WIDTH as u16),
            pixel_height: rows.saturating_mul(TERMINAL_CELL_HEIGHT as u16),
        });
    }

    pub(crate) fn scroll_live_terminal(
        &mut self,
        workspace_id: &str,
        pane_id: &str,
        event: &ScrollWheelEvent,
        cx: &mut Context<Self>,
    ) {
        let lines = match event.delta {
            ScrollDelta::Lines(point) => -(point.y.round() as isize),
            ScrollDelta::Pixels(point) => {
                -((f32::from(point.y) / TERMINAL_CELL_HEIGHT).round() as isize)
            }
        };
        self.scroll_live_terminal_lines(workspace_id, pane_id, lines, cx);
    }

    pub(crate) fn scroll_live_terminal_lines(
        &mut self,
        workspace_id: &str,
        pane_id: &str,
        lines: isize,
        cx: &mut Context<Self>,
    ) {
        if lines == 0 {
            return;
        }
        let key = live_terminal_key(workspace_id, pane_id);
        let Some(live) = self.live_terminals.get(&key) else {
            return;
        };
        live.scroll_viewport(lines);
        cx.stop_propagation();
    }
}

pub(crate) fn terminal_exit_status_label(exit_code: Option<i64>) -> String {
    match exit_code {
        Some(0) => "exited".to_owned(),
        Some(code) => format!("exited with status {code}"),
        None => "exited".to_owned(),
    }
}

pub(crate) fn terminal_page_scroll_lines(live: &LiveTerminalPane, direction: isize) -> isize {
    let rows = live
        .latest
        .as_ref()
        .or(live.pending_snapshot.as_ref())
        .map(|snapshot| snapshot.rows)
        .unwrap_or(24);
    direction * rows.saturating_sub(1).max(1) as isize
}

pub(crate) fn terminal_page_scroll_direction(input: &LiveTerminalKeyInput) -> Option<isize> {
    if input.text.is_some()
        || input.modifiers.shift
        || input.modifiers.alt
        || input.modifiers.control
        || input.modifiers.platform
    {
        return None;
    }
    match input.key {
        LiveTerminalKey::PageUp => Some(-1),
        LiveTerminalKey::PageDown => Some(1),
        _ => None,
    }
}
