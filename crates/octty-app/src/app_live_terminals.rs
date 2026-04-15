use super::*;

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
                    .unwrap_or_else(|| stable_tmux_session_name(&spec));
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
        for (key, live) in &mut self.live_terminals {
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

            let focused = focused_live_key.as_deref() == Some(key.as_str());
            if let Some(snapshot) = take_presentable_terminal_snapshot(live, focused, now) {
                live.latest = Some(snapshot.clone());
                live.last_presented_snapshot_at = Some(now);
                updates.push((key.clone(), snapshot));
            } else if live.pending_snapshot.is_some()
                && let Some(delay) = terminal_snapshot_presentation_delay(live, focused, now)
            {
                result.defer_for(delay);
            }
        }

        for (key, snapshot) in updates {
            let Some((workspace_id, pane_id)) = split_live_terminal_key(&key) else {
                continue;
            };
            if workspace_id != active_workspace.id {
                continue;
            }
            self.record_pane_activity(
                workspace_id,
                pane_id,
                now_ms(),
                None,
                Some(&snapshot.plain_text),
                cx,
            );
            if let Some(active_snapshot) = self.active_snapshot.as_mut()
                && let Some(pane) = active_snapshot.panes.get_mut(pane_id)
                && let PanePayload::Terminal(payload) = &mut pane.payload
            {
                payload.session_id = Some(snapshot.session_id.clone());
                payload.session_state = SessionState::Live;
                payload.restored_buffer = snapshot.plain_text.clone();
                active_snapshot.updated_at = now_ms();
                result.changed = true;
            }
        }
        result
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
        let key = live_terminal_key(workspace_id, pane_id);
        let Some(live) = self.live_terminals.get(&key) else {
            return;
        };
        let lines = match event.delta {
            ScrollDelta::Lines(point) => point.y.round() as isize,
            ScrollDelta::Pixels(point) => {
                (f32::from(point.y) / TERMINAL_CELL_HEIGHT).round() as isize
            }
        };
        if lines == 0 {
            return;
        }
        let _ = live.handle.scroll(lines);
        cx.stop_propagation();
    }
}
