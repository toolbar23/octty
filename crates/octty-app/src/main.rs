use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use futures::{StreamExt, channel::mpsc};
use gpui::{
    Action, App, Application, Bounds, Context, FocusHandle, Font, FontFallbacks, FontFeatures,
    Hsla, IntoElement, KeyBinding, KeyDownEvent, Menu, MenuItem, MouseButton, Render, Rgba,
    ScrollDelta, ScrollWheelEvent, ShapedLine, SharedString, TextRun, Window, WindowBounds,
    WindowOptions, canvas, div, fill, font, point, prelude::*, px, rgb, size,
};
use gpui_component::Root;
use octty_core::{
    PanePayload, PaneState, PaneType, ProjectRootRecord, SessionSnapshot, SessionState,
    TerminalPanePayload, WorkspaceSnapshot, WorkspaceState, WorkspaceSummary, add_pane,
    create_default_snapshot, create_pane_state, has_recorded_workspace_path,
    layout::{LAYOUT_VERSION, now_ms},
    remove_pane, workspace_shortcut_targets,
};
use octty_jj::{discover_workspaces, read_workspace_status, resolve_repo_root};
use octty_store::{TursoStore, default_store_path};
use octty_term::{
    TerminalSessionSpec, capture_tmux_pane, ensure_tmux_session, kill_tmux_session,
    live::{
        LiveTerminalHandle, LiveTerminalKey, LiveTerminalKeyInput, LiveTerminalModifiers,
        LiveTerminalSnapshotNotifier, TerminalGridSnapshot, TerminalResize, TerminalRgb,
        spawn_live_terminal, spawn_live_terminal_with_notifier,
    },
    resize_tmux_session, send_tmux_keys, send_tmux_keys_to_session, send_tmux_text,
    send_tmux_text_to_session, stable_tmux_session_name,
};

mod gpui_tokio;

const TERMINAL_CELL_WIDTH: f32 = 8.0;
const TERMINAL_CELL_HEIGHT: f32 = 18.0;
const TERMINAL_FONT_SIZE: f32 = 14.0;
const TERMINAL_DEBUG_TIMER_FONT_SIZE: f32 = 10.0;
const TERMINAL_DEBUG_TIMER_LINE_HEIGHT: f32 = 12.0;
const TERMINAL_SURFACE_PADDING_Y: f32 = 16.0;
const TERMINAL_SURFACE_DEBUG_TIMER_MARGIN_BOTTOM: f32 = 4.0;
const TERMINAL_SURFACE_CHROME_HEIGHT: f32 = TERMINAL_SURFACE_PADDING_Y
    + TERMINAL_DEBUG_TIMER_LINE_HEIGHT
    + TERMINAL_SURFACE_DEBUG_TIMER_MARGIN_BOTTOM;
const TERMINAL_TASKSPACE_VERTICAL_CHROME_HEIGHT: f32 = 176.0;
const WORKSPACE_SIDEBAR_WIDTH: f32 = 280.0;
const TASKSPACE_HORIZONTAL_PADDING: f32 = 48.0;
const TASKSPACE_PANEL_GAP: f32 = 12.0;
const COLUMN_WIDTH_STEP_PX: f64 = 80.0;
const MIN_COLUMN_WIDTH_PX: f64 = 240.0;
const MAX_COLUMN_WIDTH_PX: f64 = 1_600.0;
const DEFAULT_TERMINAL_FONT_FAMILY: &str = "JetBrains Mono";
const TERMINAL_FOCUSED_FRAME_INTERVAL: Duration = Duration::from_millis(8);
const TERMINAL_BACKGROUND_FRAME_INTERVAL: Duration = Duration::from_millis(100);
const TERMINAL_INTERACTIVE_SNAPSHOT_WINDOW: Duration = Duration::from_millis(150);
const TERMINAL_LATENCY_SAMPLE_LIMIT: usize = 256;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct OpenWorkspaceShortcut {
    index: usize,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct AddShellPane;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct AddDiffPane;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct AddNotePane;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct PasteTerminalClipboard;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct NavigatePane {
    direction: PaneNavigationDirection,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct CloseActivePane;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct ResizeFocusedColumn {
    direction: ColumnResizeDirection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaneNavigationDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColumnResizeDirection {
    Slimmer,
    Wider,
}

#[derive(Clone)]
struct BootstrapState {
    status: String,
    workspaces: Vec<WorkspaceSummary>,
    active_workspace_index: Option<usize>,
    active_snapshot: Option<WorkspaceSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TerminalInput {
    LiveKey(LiveTerminalKeyInput),
}

#[derive(Clone, Debug)]
struct PendingTerminalInput {
    workspace: WorkspaceSummary,
    snapshot: WorkspaceSnapshot,
    pane_id: String,
    payload: TerminalPanePayload,
    input: TerminalInput,
}

struct OcttyApp {
    status: SharedString,
    workspaces: Vec<WorkspaceSummary>,
    active_workspace_index: Option<usize>,
    active_snapshot: Option<WorkspaceSnapshot>,
    store_path: std::path::PathBuf,
    focus_handle: FocusHandle,
    pending_terminal_inputs: Vec<PendingTerminalInput>,
    terminal_flush_active: bool,
    live_terminals: HashMap<String, LiveTerminalPane>,
    failed_live_terminals: BTreeSet<String>,
    terminal_snapshot_tx: mpsc::UnboundedSender<()>,
    terminal_snapshot_rx: Option<mpsc::UnboundedReceiver<()>>,
    terminal_notifications_active: bool,
    terminal_window_active: bool,
}

struct LiveTerminalPane {
    handle: LiveTerminalHandle,
    latest: Option<TerminalGridSnapshot>,
    last_resize: Option<(u16, u16)>,
    last_input_at: Option<Instant>,
    latency: TerminalLatencyStats,
}

#[derive(Default)]
struct TerminalLatencyStats {
    key_to_snapshot_micros: VecDeque<u64>,
    pty_to_snapshot_micros: VecDeque<u64>,
    pty_output_bytes: VecDeque<u64>,
    vt_write_micros: VecDeque<u64>,
    snapshot_update_micros: VecDeque<u64>,
    snapshot_extract_micros: VecDeque<u64>,
    snapshot_build_micros: VecDeque<u64>,
}

impl OcttyApp {
    fn new(bootstrap: BootstrapState, focus_handle: FocusHandle) -> Self {
        let (terminal_snapshot_tx, terminal_snapshot_rx) = mpsc::unbounded();
        let mut app = Self {
            status: bootstrap.status.into(),
            workspaces: bootstrap.workspaces,
            active_workspace_index: bootstrap.active_workspace_index,
            active_snapshot: bootstrap.active_snapshot,
            store_path: default_store_path(),
            focus_handle,
            pending_terminal_inputs: Vec::new(),
            terminal_flush_active: false,
            live_terminals: HashMap::new(),
            failed_live_terminals: BTreeSet::new(),
            terminal_snapshot_tx,
            terminal_snapshot_rx: Some(terminal_snapshot_rx),
            terminal_notifications_active: false,
            terminal_window_active: true,
        };
        app.ensure_live_terminals_for_active_snapshot();
        app
    }

    fn open_workspace(
        &mut self,
        action: &OpenWorkspaceShortcut,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if action.index >= self.workspaces.len() {
            return;
        }

        self.active_workspace_index = Some(action.index);
        let workspace = self.workspaces[action.index].clone();
        let workspace_id = workspace.id.clone();
        let workspace_display_name = workspace.display_name_or_workspace_name().to_owned();
        self.active_snapshot = None;
        self.status = format!("Opening {workspace_display_name}...").into();
        cx.notify();

        let store_path = self.store_path.clone();
        cx.spawn(async move |this, cx| {
            let result = match gpui_tokio::Tokio::spawn_result(cx, async move {
                let store = TursoStore::open(store_path).await?;
                load_workspace_snapshot(&store, &workspace).await
            }) {
                Ok(task) => task.await,
                Err(error) => Err(error),
            };

            let _ = this.update(cx, |app, cx| {
                let still_active = app
                    .active_workspace()
                    .is_some_and(|workspace| workspace.id == workspace_id);
                if !still_active {
                    return;
                }
                match result {
                    Ok(snapshot) => {
                        app.active_snapshot = Some(snapshot);
                        app.ensure_live_terminals_for_active_snapshot();
                        app.schedule_terminal_snapshot_notifications(cx);
                        app.status = format!("Opened {workspace_display_name}.").into();
                    }
                    Err(error) => {
                        app.status =
                            format!("Failed to open {workspace_display_name}: {error:#}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn add_shell_pane(&mut self, _: &AddShellPane, _: &mut Window, cx: &mut Context<Self>) {
        self.add_pane(PaneType::Shell, cx);
    }

    fn add_diff_pane(&mut self, _: &AddDiffPane, _: &mut Window, cx: &mut Context<Self>) {
        self.add_pane(PaneType::Diff, cx);
    }

    fn add_note_pane(&mut self, _: &AddNotePane, _: &mut Window, cx: &mut Context<Self>) {
        self.add_pane(PaneType::Note, cx);
    }

    fn paste_terminal_clipboard(
        &mut self,
        _: &PasteTerminalClipboard,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(clipboard) = cx.read_from_clipboard()
            && let Some(text) = clipboard.text()
        {
            self.send_bytes_to_active_terminal(terminal_paste_bytes(&text), cx);
        }
        cx.stop_propagation();
    }

    fn navigate_pane(
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
        self.save_workspace_snapshot(
            snapshot_to_save,
            "Selected pane, but failed to save focus",
            cx,
        );
        cx.notify();
    }

    fn close_active_pane(&mut self, _: &CloseActivePane, _: &mut Window, cx: &mut Context<Self>) {
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
                self.ensure_live_terminals_for_active_snapshot();
                self.schedule_terminal_snapshot_notifications(cx);
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
                self.status = format!("Failed to close pane: {error:#}").into();
                cx.notify();
            }
        }
    }

    fn resize_focused_column(
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

    fn add_pane(&mut self, pane_type: PaneType, cx: &mut Context<Self>) {
        let Some(workspace) = self.active_workspace().cloned() else {
            self.status = "No active workspace.".into();
            cx.notify();
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
                    self.status =
                        format!("Added pane, but terminal metadata failed: {error:#}").into();
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
        self.ensure_live_terminals_for_active_snapshot();
        self.schedule_terminal_snapshot_notifications(cx);
        self.save_workspace_snapshot(snapshot, "Failed to save taskspace", cx);
        cx.notify();
    }

    fn select_pane(&mut self, pane_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window);
        let snapshot_to_save = self.active_snapshot.as_mut().map(|snapshot| {
            snapshot.active_pane_id = Some(pane_id.to_owned());
            snapshot.updated_at = now_ms();
            snapshot.clone()
        });

        if let Some(snapshot) = snapshot_to_save {
            self.save_workspace_snapshot(snapshot, "Selected pane, but failed to save focus", cx);
        }
        cx.notify();
    }

    fn save_workspace_snapshot(
        &self,
        snapshot: WorkspaceSnapshot,
        error_context: &'static str,
        cx: &mut Context<Self>,
    ) {
        let store_path = self.store_path.clone();
        cx.spawn(async move |this, cx| {
            let result = match gpui_tokio::Tokio::spawn_result(cx, async move {
                let store = TursoStore::open(store_path).await?;
                store.save_snapshot(&snapshot).await?;
                Ok(())
            }) {
                Ok(task) => task.await,
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                let _ = this.update(cx, |app, cx| {
                    app.status = format!("{error_context}: {error:#}").into();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn kill_terminal_session(&self, session_id: String, cx: &mut Context<Self>) {
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
                    app.status = format!(
                        "Closed pane, but failed to stop {session_id_for_error}: {error:#}"
                    )
                    .into();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(input) = terminal_input_from_key_event(event) else {
            return;
        };
        self.send_input_to_active_terminal(input, cx);
        cx.stop_propagation();
    }

    fn send_input_to_active_terminal(&mut self, input: TerminalInput, cx: &mut Context<Self>) {
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
        if let Some(live) = self.live_terminals.get_mut(&live_key) {
            match &input {
                TerminalInput::LiveKey(key_input) => {
                    if let Err(error) = live.handle.send_key(key_input.clone()) {
                        self.status = format!("Terminal input failed: {error:#}").into();
                    } else {
                        live.last_input_at = Some(Instant::now());
                    }
                }
            }
            if let Some(snapshot) = self.active_snapshot.as_mut() {
                snapshot.active_pane_id = Some(pane_id);
            }
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
        self.schedule_terminal_flush(cx);
        cx.notify();
    }

    fn send_bytes_to_active_terminal(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
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
        let Some(live) = self.live_terminals.get_mut(&live_key) else {
            return;
        };
        if let Err(error) = live.handle.send_bytes(bytes) {
            self.status = format!("Terminal paste failed: {error:#}").into();
            cx.notify();
            return;
        }
        live.last_input_at = Some(Instant::now());
        if let Some(snapshot) = self.active_snapshot.as_mut() {
            snapshot.active_pane_id = Some(pane_id);
        }
    }

    fn schedule_terminal_flush(&mut self, cx: &mut Context<Self>) {
        if self.terminal_flush_active {
            return;
        }

        self.terminal_flush_active = true;
        let timer = cx.background_executor().timer(Duration::from_millis(8));
        cx.spawn(async move |this, cx| {
            timer.await;
            loop {
                let Some((store_path, pending)) = this
                    .update(cx, |app, _cx| {
                        let pending = std::mem::take(&mut app.pending_terminal_inputs);
                        if pending.is_empty() {
                            app.terminal_flush_active = false;
                            None
                        } else {
                            Some((app.store_path.clone(), pending))
                        }
                    })
                    .ok()
                    .flatten()
                else {
                    break;
                };

                let result = match gpui_tokio::Tokio::spawn_result(
                    cx,
                    flush_terminal_inputs(store_path, pending),
                ) {
                    Ok(flush) => flush.await,
                    Err(error) => Err(error),
                };

                let _ = this.update(cx, |app, cx| {
                    match result {
                        Ok(snapshots) => app.apply_terminal_flush_snapshots(snapshots),
                        Err(error) => {
                            app.status = format!("Terminal input failed: {error:#}").into();
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

    fn apply_terminal_flush_snapshots(&mut self, snapshots: Vec<WorkspaceSnapshot>) {
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

    fn active_workspace(&self) -> Option<&WorkspaceSummary> {
        self.active_workspace_index
            .and_then(|index| self.workspaces.get(index))
    }

    fn ensure_live_terminals_for_active_snapshot(&mut self) {
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
                            last_resize: None,
                            last_input_at: None,
                            latency: TerminalLatencyStats::default(),
                        },
                    );
                }
                Err(error) => {
                    self.failed_live_terminals.insert(key);
                    self.status = format!("Failed to start live terminal: {error:#}").into();
                }
            }
        }
    }

    fn schedule_terminal_snapshot_notifications(&mut self, cx: &mut Context<Self>) {
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
                let delay = this
                    .update(cx, |app, _cx| app.terminal_snapshot_coalesce_interval())
                    .unwrap_or(TERMINAL_BACKGROUND_FRAME_INTERVAL);
                cx.background_executor().timer(delay).await;
                drain_pending_terminal_notifications(&mut notification_rx);
                let _ = this.update(cx, |app, cx| {
                    let changed = app.drain_live_terminal_snapshots();
                    if changed {
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

    fn terminal_snapshot_coalesce_interval(&self) -> Duration {
        terminal_snapshot_coalesce_interval(
            self.terminal_window_active,
            self.has_recent_terminal_input(),
        )
    }

    fn has_recent_terminal_input(&self) -> bool {
        self.live_terminals.values().any(|live| {
            live.last_input_at
                .is_some_and(|input_at| input_at.elapsed() <= TERMINAL_INTERACTIVE_SNAPSHOT_WINDOW)
        })
    }

    fn drain_live_terminal_snapshots(&mut self) -> bool {
        let mut changed = false;
        let Some(active_workspace) = self.active_workspace().cloned() else {
            return false;
        };
        let mut updates = Vec::new();
        for (key, live) in &mut self.live_terminals {
            if let Some(snapshot) = live.handle.drain_latest_snapshot() {
                if let Some(input_at) = live.last_input_at.take() {
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
                live.latest = Some(snapshot.clone());
                updates.push((key.clone(), snapshot));
            }
        }

        for (key, snapshot) in updates {
            let Some((workspace_id, pane_id)) = split_live_terminal_key(&key) else {
                continue;
            };
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
                active_snapshot.updated_at = now_ms();
                changed = true;
            }
        }
        changed
    }

    fn resize_live_terminal(&mut self, workspace_id: &str, pane_id: &str, cols: u16, rows: u16) {
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

    fn scroll_live_terminal(
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

impl TerminalLatencyStats {
    fn record_key_to_snapshot(&mut self, duration: Duration) {
        push_latency_sample(
            &mut self.key_to_snapshot_micros,
            duration.as_micros().min(u128::from(u64::MAX)) as u64,
        );
    }

    fn record_pty_to_snapshot(&mut self, micros: Option<u64>) {
        if let Some(micros) = micros {
            push_latency_sample(&mut self.pty_to_snapshot_micros, micros);
        }
    }

    fn record_pty_output_bytes(&mut self, bytes: u64) {
        push_latency_sample(&mut self.pty_output_bytes, bytes);
    }

    fn record_vt_write(&mut self, micros: u64) {
        push_latency_sample(&mut self.vt_write_micros, micros);
    }

    fn record_snapshot_update(&mut self, micros: u64) {
        push_latency_sample(&mut self.snapshot_update_micros, micros);
    }

    fn record_snapshot_extract(&mut self, micros: u64) {
        push_latency_sample(&mut self.snapshot_extract_micros, micros);
    }

    fn record_snapshot_build(&mut self, micros: u64) {
        push_latency_sample(&mut self.snapshot_build_micros, micros);
    }

    fn summary_label(&self) -> Option<String> {
        let key = latency_summary(&self.key_to_snapshot_micros)?;
        let pty = latency_summary(&self.pty_to_snapshot_micros);
        let output_bytes = count_summary(&self.pty_output_bytes);
        let vt = latency_summary(&self.vt_write_micros);
        let update = latency_summary(&self.snapshot_update_micros);
        let extract = latency_summary(&self.snapshot_extract_micros);
        let build = latency_summary(&self.snapshot_build_micros);
        let render = terminal_render_profile_summary();
        let mut parts = vec![format!("key {key}")];
        if let Some(pty) = pty {
            parts.push(format!("pty {pty}"));
        }
        if let Some(output_bytes) = output_bytes {
            parts.push(format!("out {output_bytes}b"));
        }
        if let Some(vt) = vt {
            parts.push(format!("vt {vt}"));
        }
        if let Some(update) = update {
            parts.push(format!("upd {update}"));
        }
        if let Some(extract) = extract {
            parts.push(format!("extract {extract}"));
        }
        if let Some(build) = build {
            parts.push(format!("snap {build}"));
        }
        if let Some(render) = render {
            parts.push(render);
        }
        Some(parts.join(" · "))
    }
}

fn drain_pending_terminal_notifications(notification_rx: &mut mpsc::UnboundedReceiver<()>) {
    while notification_rx.try_recv().is_ok() {}
}

fn terminal_snapshot_coalesce_interval(window_active: bool, has_recent_input: bool) -> Duration {
    if window_active && has_recent_input {
        Duration::ZERO
    } else if window_active {
        TERMINAL_FOCUSED_FRAME_INTERVAL
    } else {
        TERMINAL_BACKGROUND_FRAME_INTERVAL
    }
}

impl Render for OcttyApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_live_terminals_for_active_snapshot();
        self.terminal_window_active = window.is_window_active();
        self.schedule_terminal_snapshot_notifications(cx);
        let taskspace_height =
            taskspace_height_for_viewport(f32::from(window.viewport_size().height));
        let taskspace_width = taskspace_width_for_viewport(f32::from(window.viewport_size().width));
        for (workspace_id, pane_id, cols, rows) in
            terminal_resize_requests(self.active_snapshot.as_ref(), taskspace_height)
        {
            self.resize_live_terminal(&workspace_id, &pane_id, cols, rows);
        }

        let shortcuts = workspace_shortcut_targets(&self.workspaces);
        let mut workspace_list = div().mt_4().flex().flex_col().gap_2();

        if self.workspaces.is_empty() {
            workspace_list = workspace_list.child(
                div()
                    .text_sm()
                    .text_color(rgb(0xa0a0a0))
                    .child("No JJ workspaces discovered."),
            );
        }

        for (index, workspace) in self.workspaces.iter().enumerate() {
            let shortcut = shortcuts
                .get(index)
                .map(|target| format!(" <{}>", target.label))
                .unwrap_or_default();
            workspace_list = workspace_list.child(
                div()
                    .p_2()
                    .border_1()
                    .border_color(rgb(0x333333))
                    .rounded_md()
                    .bg(if self.active_workspace_index == Some(index) {
                        rgb(0x242424)
                    } else {
                        rgb(0x171717)
                    })
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.open_workspace(&OpenWorkspaceShortcut { index }, window, cx);
                        }),
                    )
                    .child(div().text_sm().child(format!(
                        "{}{}",
                        workspace.display_name_or_workspace_name(),
                        shortcut
                    )))
                    .child(
                        div()
                            .mt_1()
                            .text_xs()
                            .text_color(rgb(0xa0a0a0))
                            .child(format!(
                                "{} · {}",
                                workspace.project_display_name,
                                workspace_status_label(&workspace.status.workspace_state)
                            )),
                    ),
            );
        }

        let taskspace = render_taskspace(
            self.active_snapshot.as_ref(),
            &self.live_terminals,
            taskspace_width,
            cx,
        );

        div()
            .id("octty-rs-root")
            .key_context("OcttyApp")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::open_workspace))
            .on_action(cx.listener(Self::add_shell_pane))
            .on_action(cx.listener(Self::add_diff_pane))
            .on_action(cx.listener(Self::add_note_pane))
            .on_action(cx.listener(Self::paste_terminal_clipboard))
            .on_action(cx.listener(Self::navigate_pane))
            .on_action(cx.listener(Self::close_active_pane))
            .on_action(cx.listener(Self::resize_focused_column))
            .on_key_down(cx.listener(Self::handle_key_down))
            .flex()
            .size_full()
            .bg(rgb(0x171717))
            .text_color(rgb(0xf2f2f2))
            .child(
                div()
                    .w(px(WORKSPACE_SIDEBAR_WIDTH))
                    .h_full()
                    .border_r_1()
                    .border_color(rgb(0x3a3a3a))
                    .p_4()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("Octty"),
                    )
                    .child(
                        div()
                            .mt_4()
                            .text_xs()
                            .text_color(rgb(0x7f7f7f))
                            .child("Workspaces"),
                    )
                    .child(workspace_list),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .p_6()
                    .child(div().text_xl().child("Taskspace"))
                    .child(
                        div()
                            .mt_3()
                            .text_sm()
                            .text_color(rgb(0xb8b8b8))
                            .child(self.status.clone()),
                    )
                    .child(
                        div()
                            .mt_6()
                            .flex()
                            .gap_2()
                            .child(toolbar_button("Shell").on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.add_pane(PaneType::Shell, cx);
                                }),
                            ))
                            .child(toolbar_button("Diff").on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.add_pane(PaneType::Diff, cx);
                                }),
                            ))
                            .child(toolbar_button("Note").on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.add_pane(PaneType::Note, cx);
                                }),
                            )),
                    )
                    .child(taskspace),
            )
    }
}

fn main() {
    let runtime = tokio::runtime::Runtime::new().expect("create tokio runtime");
    if std::env::args().any(|arg| arg == "--headless-check") {
        runtime
            .block_on(load_bootstrap(false))
            .expect("load Rust Octty bootstrap");
        println!("octty-rs headless check ok");
        return;
    }
    if std::env::args().any(|arg| arg == "--bootstrap-check") {
        let bootstrap = runtime
            .block_on(load_bootstrap(true))
            .expect("load Rust Octty bootstrap");
        println!(
            "octty-rs bootstrap check ok: {} workspace(s)",
            bootstrap.workspaces.len()
        );
        return;
    }
    if std::env::args().any(|arg| arg == "--pane-check") {
        let count = runtime
            .block_on(pane_persistence_check())
            .expect("run pane persistence check");
        println!("octty-rs pane check ok: {count} pane(s)");
        return;
    }
    if std::env::args().any(|arg| arg == "--shell-check") {
        let session_id = runtime
            .block_on(shell_session_check())
            .expect("run shell session check");
        println!("octty-rs shell check ok: {session_id}");
        return;
    }
    if std::env::args().any(|arg| arg == "--terminal-io-check") {
        let marker = runtime
            .block_on(terminal_io_check())
            .expect("run terminal io check");
        println!("octty-rs terminal io check ok: {marker}");
        return;
    }
    if std::env::args().any(|arg| arg == "--live-terminal-check") {
        let marker = runtime
            .block_on(live_terminal_check())
            .expect("run live terminal check");
        println!("octty-rs live terminal check ok: {marker}");
        return;
    }

    let bootstrap = runtime
        .block_on(load_bootstrap(true))
        .unwrap_or_else(|error| BootstrapState {
            status: format!("Startup failed: {error:#}"),
            workspaces: Vec::new(),
            active_workspace_index: None,
            active_snapshot: None,
        });

    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);
        gpui_tokio::init_from_handle(cx, runtime.handle().clone());
        cx.bind_keys(workspace_key_bindings());
        set_workspace_menu(cx, &bootstrap.workspaces);

        let bounds = Bounds::centered(None, size(px(1200.0), px(760.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Octty".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                let focus_handle = cx.focus_handle();
                focus_handle.focus(window);
                let view = cx.new(|_| OcttyApp::new(bootstrap, focus_handle));
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("open Octty window");
        cx.activate(true);
    });
}

async fn load_bootstrap(auto_seed_current_repo: bool) -> anyhow::Result<BootstrapState> {
    let store = TursoStore::open(default_store_path()).await?;
    let mut roots = store.list_project_roots().await?;
    if roots.is_empty() && auto_seed_current_repo {
        if let Ok(root_path) = resolve_repo_root(std::env::current_dir()?).await {
            let root = project_root_from_path(&root_path);
            store.upsert_project_root(&root).await?;
            roots.push(root);
        }
    }

    let mut errors = Vec::new();
    let mut workspaces = Vec::new();
    for root in roots {
        match discover_workspaces(&root).await {
            Ok(discovered) => {
                for mut workspace in discovered {
                    let now = now_ms();
                    if workspace.created_at == 0 {
                        workspace.created_at = now;
                    }
                    workspace.updated_at = now;
                    if has_recorded_workspace_path(&workspace.workspace_path) {
                        match read_workspace_status(&workspace.workspace_path).await {
                            Ok(status) => workspace.status = status,
                            Err(error) => errors.push(format!(
                                "{}: failed to read status: {error}",
                                workspace.workspace_name
                            )),
                        }
                    }
                    store.upsert_workspace(&workspace).await?;
                    workspaces.push(workspace);
                }
            }
            Err(error) => errors.push(format!(
                "{}: failed to discover workspaces: {error}",
                root.root_path
            )),
        }
    }

    if workspaces.is_empty() {
        workspaces = store.list_workspaces().await?;
    }

    let active_workspace_index = if workspaces.is_empty() { None } else { Some(0) };
    let active_snapshot = match active_workspace_index {
        Some(index) => Some(load_workspace_snapshot(&store, &workspaces[index]).await?),
        None => None,
    };

    let status = if workspaces.is_empty() {
        "No project roots yet. Run from inside a JJ repo to auto-seed the first root.".to_owned()
    } else if errors.is_empty() {
        format!("Loaded {} JJ workspace(s).", workspaces.len())
    } else {
        format!(
            "Loaded {} JJ workspace(s), with {} refresh warning(s).",
            workspaces.len(),
            errors.len()
        )
    };

    Ok(BootstrapState {
        status,
        workspaces,
        active_workspace_index,
        active_snapshot,
    })
}

async fn load_workspace_snapshot(
    store: &TursoStore,
    workspace: &WorkspaceSummary,
) -> anyhow::Result<WorkspaceSnapshot> {
    if let Some(snapshot) = store.get_snapshot(&workspace.id).await?
        && snapshot.layout_version == LAYOUT_VERSION
    {
        return Ok(snapshot);
    }

    let snapshot = create_default_snapshot(workspace.id.clone());
    store.save_snapshot(&snapshot).await?;
    Ok(snapshot)
}

async fn pane_persistence_check() -> anyhow::Result<usize> {
    let bootstrap = load_bootstrap(true).await?;
    let Some(index) = bootstrap.active_workspace_index else {
        anyhow::bail!("no active workspace");
    };
    let workspace = &bootstrap.workspaces[index];
    let snapshot = bootstrap
        .active_snapshot
        .unwrap_or_else(|| create_default_snapshot(workspace.id.clone()));
    let snapshot = add_pane(
        snapshot,
        create_pane_state(PaneType::Shell, workspace.workspace_path.clone(), None),
    );

    let store = TursoStore::open(default_store_path()).await?;
    store.save_snapshot(&snapshot).await?;
    let saved = load_workspace_snapshot(&store, workspace).await?;
    Ok(saved.panes.len())
}

async fn shell_session_check() -> anyhow::Result<String> {
    let bootstrap = load_bootstrap(true).await?;
    let Some(index) = bootstrap.active_workspace_index else {
        anyhow::bail!("no active workspace");
    };
    let workspace = &bootstrap.workspaces[index];
    let snapshot = bootstrap
        .active_snapshot
        .unwrap_or_else(|| create_default_snapshot(workspace.id.clone()));
    let pane = create_pane_state(PaneType::Shell, workspace.workspace_path.clone(), None);
    let pane_id = pane.id.clone();
    let snapshot = add_pane(snapshot, pane);
    let snapshot = start_terminal_session(
        &TursoStore::open(default_store_path()).await?,
        workspace,
        snapshot,
        &pane_id,
    )
    .await?;
    Ok(snapshot
        .panes
        .get(&pane_id)
        .and_then(|pane| match &pane.payload {
            PanePayload::Terminal(payload) => payload.session_id.clone(),
            _ => None,
        })
        .unwrap_or_default())
}

async fn terminal_io_check() -> anyhow::Result<String> {
    let bootstrap = load_bootstrap(true).await?;
    let Some(index) = bootstrap.active_workspace_index else {
        anyhow::bail!("no active workspace");
    };
    let workspace = &bootstrap.workspaces[index];
    let snapshot = bootstrap
        .active_snapshot
        .unwrap_or_else(|| create_default_snapshot(workspace.id.clone()));
    let pane = create_pane_state(PaneType::Shell, workspace.workspace_path.clone(), None);
    let pane_id = pane.id.clone();
    let snapshot = add_pane(snapshot, pane);
    let store = TursoStore::open(default_store_path()).await?;
    let mut snapshot = start_terminal_session(&store, workspace, snapshot, &pane_id).await?;

    let payload = terminal_payload_for_pane(&snapshot, &pane_id)?.clone();
    let spec = terminal_spec_for_payload(workspace, &pane_id, &payload, 120, 40);
    resize_tmux_session(&spec, 120, 40).await?;

    let marker = format!("octty-terminal-io-{}", now_ms());
    let session_id = ensure_tmux_session(&spec).await?;
    send_tmux_text(&spec, &format!("clear; printf '{marker}\\n'")).await?;
    send_tmux_keys(&spec, &["Enter"]).await?;
    let screen = capture_tmux_until_contains(&spec, &marker, Duration::from_millis(1_000)).await?;
    snapshot =
        persist_terminal_screen(&store, workspace, snapshot, &pane_id, session_id, screen).await?;
    store.save_snapshot(&snapshot).await?;

    Ok(marker)
}

async fn live_terminal_check() -> anyhow::Result<String> {
    let marker = format!("octty-live-terminal-{}", now_ms());
    let pane_id = format!("pane-{}", now_ms());
    let spec = TerminalSessionSpec {
        workspace_id: "live-terminal-check".to_owned(),
        pane_id,
        kind: octty_core::TerminalKind::Shell,
        cwd: std::env::current_dir()?.to_string_lossy().to_string(),
        cols: 80,
        rows: 24,
    };
    let mut terminal = spawn_live_terminal(spec)?;
    terminal.send_bytes(format!("printf '{marker}\\n'\r").into_bytes())?;

    let deadline = tokio::time::Instant::now() + Duration::from_millis(2_000);
    loop {
        for snapshot in terminal.drain_snapshots() {
            if snapshot.plain_text.contains(&marker) {
                let session_id = terminal.session_id().to_owned();
                drop(terminal);
                let _ = kill_tmux_session(&session_id).await;
                return Ok(marker);
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let session_id = terminal.session_id().to_owned();
            drop(terminal);
            let _ = kill_tmux_session(&session_id).await;
            anyhow::bail!("live terminal snapshot did not contain marker `{marker}`");
        }
        tokio::time::sleep(Duration::from_millis(16)).await;
    }
}

async fn flush_terminal_inputs(
    store_path: PathBuf,
    pending: Vec<PendingTerminalInput>,
) -> anyhow::Result<Vec<WorkspaceSnapshot>> {
    let store = TursoStore::open(store_path).await?;
    let mut touched = Vec::<PendingTerminalInput>::new();

    for input in pending {
        let spec =
            terminal_spec_for_payload(&input.workspace, &input.pane_id, &input.payload, 120, 40);
        let session_id = input
            .payload
            .session_id
            .clone()
            .unwrap_or_else(|| stable_tmux_session_name(&spec));

        if send_terminal_input_to_session(&session_id, &input.input)
            .await
            .is_err()
        {
            let session_id = ensure_tmux_session(&spec).await?;
            send_terminal_input_to_session(&session_id, &input.input).await?;
        }
        touched.push(input);
    }

    let mut snapshots = Vec::new();
    let mut captured_panes = BTreeSet::<String>::new();
    for input in touched.into_iter().rev() {
        let capture_key = format!("{}:{}", input.workspace.id, input.pane_id);
        if !captured_panes.insert(capture_key) {
            continue;
        }

        let spec =
            terminal_spec_for_payload(&input.workspace, &input.pane_id, &input.payload, 120, 40);
        let session_id = input
            .payload
            .session_id
            .clone()
            .unwrap_or_else(|| stable_tmux_session_name(&spec));
        let screen = capture_tmux_pane(&spec).await.unwrap_or_default();
        let snapshot = persist_terminal_screen(
            &store,
            &input.workspace,
            input.snapshot,
            &input.pane_id,
            session_id,
            screen,
        )
        .await?;
        store.save_snapshot(&snapshot).await?;
        snapshots.push(snapshot);
    }

    Ok(snapshots)
}

async fn send_terminal_input_to_session(
    session_id: &str,
    input: &TerminalInput,
) -> anyhow::Result<()> {
    match input {
        TerminalInput::LiveKey(key_input) => {
            if let Some(text) = &key_input.text {
                send_tmux_text_to_session(session_id, text).await?;
            } else if let Some(key) = tmux_key_for_live_key(key_input) {
                send_tmux_keys_to_session(session_id, &[key.as_str()]).await?;
            }
        }
    }
    Ok(())
}

fn tmux_key_for_live_key(input: &LiveTerminalKeyInput) -> Option<String> {
    let key = match input.key {
        LiveTerminalKey::Enter => "Enter".to_owned(),
        LiveTerminalKey::Backspace => "BSpace".to_owned(),
        LiveTerminalKey::Delete => "Delete".to_owned(),
        LiveTerminalKey::Tab => "Tab".to_owned(),
        LiveTerminalKey::Escape => "Escape".to_owned(),
        LiveTerminalKey::ArrowLeft => "Left".to_owned(),
        LiveTerminalKey::ArrowRight => "Right".to_owned(),
        LiveTerminalKey::ArrowUp => "Up".to_owned(),
        LiveTerminalKey::ArrowDown => "Down".to_owned(),
        LiveTerminalKey::Home => "Home".to_owned(),
        LiveTerminalKey::End => "End".to_owned(),
        LiveTerminalKey::PageUp => "PageUp".to_owned(),
        LiveTerminalKey::PageDown => "PageDown".to_owned(),
        LiveTerminalKey::Insert => "Insert".to_owned(),
        LiveTerminalKey::Character(character) if input.modifiers.control => {
            format!("C-{}", character.to_ascii_lowercase())
        }
        LiveTerminalKey::F(number) if (1..=12).contains(&number) => format!("F{number}"),
        _ => return None,
    };
    Some(key)
}

async fn capture_tmux_until_contains(
    spec: &TerminalSessionSpec,
    needle: &str,
    timeout: Duration,
) -> anyhow::Result<String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let screen = capture_tmux_pane(spec).await?;
        if screen.contains(needle) {
            return Ok(screen);
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("terminal screen did not contain marker `{needle}`");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn prepare_live_terminal_snapshot(
    workspace: &WorkspaceSummary,
    mut snapshot: WorkspaceSnapshot,
    pane_id: &str,
) -> anyhow::Result<WorkspaceSnapshot> {
    let pane = snapshot
        .panes
        .get_mut(pane_id)
        .ok_or_else(|| anyhow::anyhow!("pane `{pane_id}` missing from snapshot"))?;
    let PanePayload::Terminal(payload) = &mut pane.payload else {
        anyhow::bail!("pane `{pane_id}` is not a terminal");
    };
    let (cols, rows) = default_terminal_grid_for_pane();
    let spec = TerminalSessionSpec {
        workspace_id: workspace.id.clone(),
        pane_id: pane_id.to_owned(),
        kind: payload.kind.clone(),
        cwd: payload.cwd.clone(),
        cols,
        rows,
    };
    payload.session_id = Some(stable_tmux_session_name(&spec));
    payload.session_state = SessionState::Live;
    snapshot.updated_at = now_ms();
    Ok(snapshot)
}

async fn start_terminal_session(
    store: &TursoStore,
    workspace: &WorkspaceSummary,
    snapshot: WorkspaceSnapshot,
    pane_id: &str,
) -> anyhow::Result<WorkspaceSnapshot> {
    let payload = terminal_payload_for_pane(&snapshot, pane_id)?.clone();
    let spec = terminal_spec_for_payload(workspace, pane_id, &payload, 120, 40);
    let session_id = ensure_tmux_session(&spec).await?;
    let screen = capture_tmux_pane(&spec).await.unwrap_or_default();

    persist_terminal_screen(store, workspace, snapshot, pane_id, session_id, screen).await
}

async fn persist_terminal_screen(
    store: &TursoStore,
    workspace: &WorkspaceSummary,
    mut snapshot: WorkspaceSnapshot,
    pane_id: &str,
    session_id: String,
    screen: String,
) -> anyhow::Result<WorkspaceSnapshot> {
    let pane = snapshot
        .panes
        .get_mut(pane_id)
        .ok_or_else(|| anyhow::anyhow!("pane `{pane_id}` missing from snapshot"))?;
    let PanePayload::Terminal(payload) = &mut pane.payload else {
        anyhow::bail!("pane `{pane_id}` is not a terminal");
    };

    payload.session_id = Some(session_id.clone());
    payload.session_state = SessionState::Live;
    payload.restored_buffer = screen.clone();

    store
        .upsert_session_state(&SessionSnapshot {
            id: session_id,
            workspace_id: workspace.id.clone(),
            pane_id: pane_id.to_owned(),
            kind: payload.kind.clone(),
            cwd: payload.cwd.clone(),
            command: payload.command.clone(),
            buffer: screen.clone(),
            screen: Some(screen),
            state: SessionState::Live,
            exit_code: None,
            embedded_session: payload.embedded_session.clone(),
            embedded_session_correlation_id: payload.embedded_session_correlation_id.clone(),
            agent_attention_state: payload.agent_attention_state.clone(),
        })
        .await?;

    snapshot.updated_at = now_ms();
    Ok(snapshot)
}

fn terminal_payload_for_pane<'a>(
    snapshot: &'a WorkspaceSnapshot,
    pane_id: &str,
) -> anyhow::Result<&'a TerminalPanePayload> {
    let pane = snapshot
        .panes
        .get(pane_id)
        .ok_or_else(|| anyhow::anyhow!("pane `{pane_id}` missing from snapshot"))?;
    let PanePayload::Terminal(payload) = &pane.payload else {
        anyhow::bail!("pane `{pane_id}` is not a terminal");
    };
    Ok(payload)
}

fn terminal_spec_for_payload(
    workspace: &WorkspaceSummary,
    pane_id: &str,
    payload: &TerminalPanePayload,
    cols: u16,
    rows: u16,
) -> TerminalSessionSpec {
    TerminalSessionSpec {
        workspace_id: workspace.id.clone(),
        pane_id: pane_id.to_owned(),
        kind: payload.kind.clone(),
        cwd: payload.cwd.clone(),
        cols,
        rows,
    }
}

fn project_root_from_path(root_path: &Path) -> ProjectRootRecord {
    let root_path_string = root_path.to_string_lossy().to_string();
    let now = now_ms();
    ProjectRootRecord {
        id: stable_project_root_id(&root_path_string),
        root_path: root_path_string,
        display_name: root_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("repo")
            .to_owned(),
        created_at: now,
        updated_at: now,
    }
}

fn stable_project_root_id(root_path: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in root_path.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("root-{hash:016x}")
}

fn set_workspace_menu(cx: &mut App, workspaces: &[WorkspaceSummary]) {
    cx.set_menus(vec![Menu {
        name: "Workspaces".into(),
        items: workspace_menu_items(workspaces),
    }]);
}

fn workspace_menu_items(workspaces: &[WorkspaceSummary]) -> Vec<MenuItem> {
    workspace_shortcut_targets(workspaces)
        .into_iter()
        .enumerate()
        .map(|(index, target)| {
            let workspace = &workspaces[index];
            let name = format!(
                "{} <{}>",
                workspace.display_name_or_workspace_name(),
                target.label
            );
            MenuItem::action(name, OpenWorkspaceShortcut { index })
        })
        .collect()
}

fn workspace_key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("ctrl-shift-1", OpenWorkspaceShortcut { index: 0 }, None),
        KeyBinding::new("ctrl-shift-2", OpenWorkspaceShortcut { index: 1 }, None),
        KeyBinding::new("ctrl-shift-3", OpenWorkspaceShortcut { index: 2 }, None),
        KeyBinding::new("ctrl-shift-4", OpenWorkspaceShortcut { index: 3 }, None),
        KeyBinding::new("ctrl-shift-5", OpenWorkspaceShortcut { index: 4 }, None),
        KeyBinding::new("ctrl-shift-6", OpenWorkspaceShortcut { index: 5 }, None),
        KeyBinding::new("ctrl-shift-7", OpenWorkspaceShortcut { index: 6 }, None),
        KeyBinding::new("ctrl-shift-8", OpenWorkspaceShortcut { index: 7 }, None),
        KeyBinding::new("ctrl-shift-9", OpenWorkspaceShortcut { index: 8 }, None),
        KeyBinding::new("ctrl-shift-0", OpenWorkspaceShortcut { index: 9 }, None),
        KeyBinding::new("ctrl-shift-v", PasteTerminalClipboard, None),
        KeyBinding::new("cmd-v", PasteTerminalClipboard, None),
        KeyBinding::new(
            "ctrl-shift-left",
            NavigatePane {
                direction: PaneNavigationDirection::Left,
            },
            None,
        ),
        KeyBinding::new(
            "ctrl-shift-right",
            NavigatePane {
                direction: PaneNavigationDirection::Right,
            },
            None,
        ),
        KeyBinding::new(
            "ctrl-shift-up",
            NavigatePane {
                direction: PaneNavigationDirection::Up,
            },
            None,
        ),
        KeyBinding::new(
            "ctrl-shift-down",
            NavigatePane {
                direction: PaneNavigationDirection::Down,
            },
            None,
        ),
        KeyBinding::new("ctrl-shift-w", CloseActivePane, None),
        KeyBinding::new(
            "ctrl-alt-left",
            ResizeFocusedColumn {
                direction: ColumnResizeDirection::Slimmer,
            },
            None,
        ),
        KeyBinding::new(
            "ctrl-alt-right",
            ResizeFocusedColumn {
                direction: ColumnResizeDirection::Wider,
            },
            None,
        ),
    ]
}

fn toolbar_button(label: &'static str) -> gpui::Div {
    div()
        .px_3()
        .py_2()
        .border_1()
        .border_color(rgb(0x444444))
        .rounded_md()
        .text_sm()
        .child(label)
}

fn terminal_input_from_key_event(event: &KeyDownEvent) -> Option<TerminalInput> {
    live_terminal_input_from_key_parts(
        &event.keystroke.key,
        event.keystroke.key_char.as_deref(),
        event.keystroke.modifiers.control,
        event.keystroke.modifiers.alt,
        event.keystroke.modifiers.shift,
        event.keystroke.modifiers.platform,
        event.keystroke.modifiers.function,
    )
    .map(TerminalInput::LiveKey)
}

fn live_terminal_input_from_key_parts(
    key: &str,
    key_char: Option<&str>,
    control: bool,
    alt: bool,
    shift: bool,
    platform: bool,
    function: bool,
) -> Option<LiveTerminalKeyInput> {
    if function {
        return None;
    }
    if control
        && shift
        && matches!(
            key,
            "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
        )
    {
        return None;
    }
    if control && shift && key.eq_ignore_ascii_case("v") {
        return None;
    }
    if control && shift && is_pane_action_key(key) {
        return None;
    }
    if control && alt && is_column_resize_key(key) {
        return None;
    }

    let normalized_key = key.to_ascii_lowercase();
    let live_key = match normalized_key.as_str() {
        "enter" => LiveTerminalKey::Enter,
        "backspace" => LiveTerminalKey::Backspace,
        "delete" => LiveTerminalKey::Delete,
        "tab" => LiveTerminalKey::Tab,
        "escape" => LiveTerminalKey::Escape,
        "left" => LiveTerminalKey::ArrowLeft,
        "right" => LiveTerminalKey::ArrowRight,
        "up" => LiveTerminalKey::ArrowUp,
        "down" => LiveTerminalKey::ArrowDown,
        "home" => LiveTerminalKey::Home,
        "end" => LiveTerminalKey::End,
        "pageup" => LiveTerminalKey::PageUp,
        "pagedown" => LiveTerminalKey::PageDown,
        "insert" => LiveTerminalKey::Insert,
        "space" => LiveTerminalKey::Space,
        "f1" => LiveTerminalKey::F(1),
        "f2" => LiveTerminalKey::F(2),
        "f3" => LiveTerminalKey::F(3),
        "f4" => LiveTerminalKey::F(4),
        "f5" => LiveTerminalKey::F(5),
        "f6" => LiveTerminalKey::F(6),
        "f7" => LiveTerminalKey::F(7),
        "f8" => LiveTerminalKey::F(8),
        "f9" => LiveTerminalKey::F(9),
        "f10" => LiveTerminalKey::F(10),
        "f11" => LiveTerminalKey::F(11),
        "f12" => LiveTerminalKey::F(12),
        _ => {
            let key_char =
                key_char.filter(|text| !text.is_empty() && *text != "\r" && *text != "\n");
            if let Some(text) = key_char
                && let Some(character) = text.chars().next()
            {
                LiveTerminalKey::Character(unshifted_character(character))
            } else if normalized_key.len() == 1 {
                LiveTerminalKey::Character(
                    normalized_key
                        .chars()
                        .next()
                        .map(unshifted_character)
                        .unwrap_or('\0'),
                )
            } else {
                return None;
            }
        }
    };

    let text = key_char
        .filter(|text| !control && !platform && !text.is_empty() && *text != "\r" && *text != "\n")
        .map(str::to_owned);
    let unshifted = match live_key {
        LiveTerminalKey::Character(character) => character,
        LiveTerminalKey::Space => ' ',
        _ => '\0',
    };

    Some(LiveTerminalKeyInput {
        key: live_key,
        text,
        modifiers: LiveTerminalModifiers {
            shift,
            alt,
            control,
            platform,
        },
        unshifted,
    })
}

fn is_pane_action_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "left"
            | "right"
            | "up"
            | "down"
            | "arrowleft"
            | "arrowright"
            | "arrowup"
            | "arrowdown"
            | "w"
    )
}

fn is_column_resize_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "left" | "right" | "arrowleft" | "arrowright"
    )
}

fn unshifted_character(character: char) -> char {
    match character {
        'A'..='Z' => character.to_ascii_lowercase(),
        ')' => '0',
        '!' => '1',
        '@' => '2',
        '#' => '3',
        '$' => '4',
        '%' => '5',
        '^' => '6',
        '&' => '7',
        '*' => '8',
        '(' => '9',
        '_' => '-',
        '+' => '=',
        '{' => '[',
        '}' => ']',
        '|' => '\\',
        ':' => ';',
        '"' => '\'',
        '<' => ',',
        '>' => '.',
        '?' => '/',
        '~' => '`',
        other => other,
    }
}

fn active_terminal_pane_id(snapshot: &WorkspaceSnapshot) -> Option<String> {
    snapshot
        .active_pane_id
        .as_deref()
        .and_then(|pane_id| {
            snapshot
                .panes
                .get(pane_id)
                .filter(|pane| matches!(pane.payload, PanePayload::Terminal(_)))
                .map(|pane| pane.id.clone())
        })
        .or_else(|| {
            snapshot
                .panes
                .values()
                .find(|pane| matches!(pane.payload, PanePayload::Terminal(_)))
                .map(|pane| pane.id.clone())
        })
}

fn pane_navigation_target(
    snapshot: &WorkspaceSnapshot,
    direction: PaneNavigationDirection,
) -> Option<String> {
    let active_pane_id = snapshot
        .active_pane_id
        .as_deref()
        .or_else(|| first_center_pane_id(snapshot))?;

    let (column_index, pane_index) = pane_layout_position(snapshot, active_pane_id)?;
    let target = match direction {
        PaneNavigationDirection::Up => {
            let column = center_column(snapshot, column_index)?;
            pane_index
                .checked_sub(1)
                .and_then(|index| column.pane_ids.get(index))
        }
        PaneNavigationDirection::Down => {
            let column = center_column(snapshot, column_index)?;
            column.pane_ids.get(pane_index + 1)
        }
        PaneNavigationDirection::Left => column_index
            .checked_sub(1)
            .and_then(|index| pane_in_neighbor_column(snapshot, index, pane_index)),
        PaneNavigationDirection::Right => {
            pane_in_neighbor_column(snapshot, column_index + 1, pane_index)
        }
    };

    target.cloned()
}

fn first_center_pane_id(snapshot: &WorkspaceSnapshot) -> Option<&str> {
    snapshot
        .center_column_ids
        .iter()
        .filter_map(|column_id| snapshot.columns.get(column_id))
        .flat_map(|column| column.pane_ids.iter())
        .next()
        .map(String::as_str)
}

fn pane_layout_position(snapshot: &WorkspaceSnapshot, pane_id: &str) -> Option<(usize, usize)> {
    for (column_index, column_id) in snapshot.center_column_ids.iter().enumerate() {
        let column = snapshot.columns.get(column_id)?;
        if let Some(pane_index) = column.pane_ids.iter().position(|id| id == pane_id) {
            return Some((column_index, pane_index));
        }
    }
    None
}

fn center_column(
    snapshot: &WorkspaceSnapshot,
    column_index: usize,
) -> Option<&octty_core::WorkspaceColumn> {
    snapshot
        .center_column_ids
        .get(column_index)
        .and_then(|column_id| snapshot.columns.get(column_id))
}

fn pane_in_neighbor_column(
    snapshot: &WorkspaceSnapshot,
    column_index: usize,
    source_pane_index: usize,
) -> Option<&String> {
    let column = center_column(snapshot, column_index)?;
    let target_index = source_pane_index.min(column.pane_ids.len().saturating_sub(1));
    column.pane_ids.get(target_index)
}

fn resize_focused_column_in_snapshot(
    snapshot: &mut WorkspaceSnapshot,
    direction: ColumnResizeDirection,
) -> Option<f64> {
    let column_id = active_column_id(snapshot)?;
    let column = snapshot.columns.get_mut(&column_id)?;
    let delta = match direction {
        ColumnResizeDirection::Slimmer => -COLUMN_WIDTH_STEP_PX,
        ColumnResizeDirection::Wider => COLUMN_WIDTH_STEP_PX,
    };
    let next_width = (column.width_px + delta).clamp(MIN_COLUMN_WIDTH_PX, MAX_COLUMN_WIDTH_PX);
    if (next_width - column.width_px).abs() < f64::EPSILON {
        return None;
    }
    column.width_px = next_width;
    snapshot.updated_at = now_ms();
    Some(next_width)
}

fn active_column_id(snapshot: &WorkspaceSnapshot) -> Option<String> {
    let active_pane_id = snapshot
        .active_pane_id
        .as_deref()
        .or_else(|| first_center_pane_id(snapshot))?;
    snapshot
        .center_column_ids
        .iter()
        .find(|column_id| {
            snapshot.columns.get(*column_id).is_some_and(|column| {
                column
                    .pane_ids
                    .iter()
                    .any(|pane_id| pane_id == active_pane_id)
            })
        })
        .cloned()
}

fn preview_terminal_input(snapshot: &mut WorkspaceSnapshot, pane_id: &str, input: &TerminalInput) {
    let Some(pane) = snapshot.panes.get_mut(pane_id) else {
        return;
    };
    let PanePayload::Terminal(payload) = &mut pane.payload else {
        return;
    };

    match input {
        TerminalInput::LiveKey(key_input) if key_input.text.is_some() => {
            payload
                .restored_buffer
                .push_str(key_input.text.as_deref().unwrap_or_default());
        }
        TerminalInput::LiveKey(key_input) if key_input.key == LiveTerminalKey::Enter => {
            payload.restored_buffer.push('\n');
        }
        TerminalInput::LiveKey(key_input) if key_input.key == LiveTerminalKey::Backspace => {
            payload.restored_buffer.pop();
        }
        TerminalInput::LiveKey(key_input) if key_input.key == LiveTerminalKey::Tab => {
            payload.restored_buffer.push('\t');
        }
        TerminalInput::LiveKey(_) => {}
    }
    snapshot.updated_at = now_ms();
}

fn render_taskspace(
    snapshot: Option<&WorkspaceSnapshot>,
    live_terminals: &HashMap<String, LiveTerminalPane>,
    viewport_width: f32,
    cx: &mut Context<OcttyApp>,
) -> gpui::Div {
    let taskspace = div().mt_4().flex_1().h_full().overflow_hidden();
    let Some(snapshot) = snapshot else {
        return taskspace.flex().child(
            div()
                .text_color(rgb(0xa0a0a0))
                .child("Open a workspace to start."),
        );
    };
    if snapshot.panes.is_empty() {
        return taskspace
            .flex()
            .child(div().text_color(rgb(0xa0a0a0)).child("No panes are open."));
    }

    let viewport_offset = taskspace_viewport_offset(snapshot, viewport_width);
    let mut panel_strip = div()
        .flex()
        .gap_3()
        .h_full()
        .ml(px(-viewport_offset))
        .flex_none();

    for column_id in &snapshot.center_column_ids {
        let Some(column) = snapshot.columns.get(column_id) else {
            continue;
        };
        let mut column_el = div()
            .flex()
            .flex_col()
            .gap_3()
            .h_full()
            .overflow_hidden()
            .flex_none()
            .w(px(column.width_px as f32));
        for pane_id in &column.pane_ids {
            if let Some(pane) = snapshot.panes.get(pane_id) {
                let active = snapshot.active_pane_id.as_deref() == Some(pane.id.as_str());
                let terminal_live =
                    live_terminals.get(&live_terminal_key(&snapshot.workspace_id, &pane.id));
                column_el = column_el.child(render_pane(
                    &snapshot.workspace_id,
                    pane,
                    active,
                    terminal_live,
                    cx,
                ));
            }
        }
        panel_strip = panel_strip.child(column_el);
    }
    taskspace.child(panel_strip)
}

fn render_pane(
    workspace_id: &str,
    pane: &PaneState,
    active: bool,
    terminal_live: Option<&LiveTerminalPane>,
    cx: &mut Context<OcttyApp>,
) -> gpui::Div {
    let pane_id = pane.id.clone();
    let scroll_workspace_id = workspace_id.to_owned();
    let scroll_pane_id = pane.id.clone();
    let mut pane_el = div()
        .flex()
        .flex_col()
        .flex_1()
        .overflow_hidden()
        .border_1()
        .border_color(if active { rgb(0x6aa36f) } else { rgb(0x444444) })
        .rounded_md()
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.select_pane(&pane_id, window, cx);
            }),
        )
        .on_scroll_wheel(cx.listener(move |this, event, _window, cx| {
            this.scroll_live_terminal(&scroll_workspace_id, &scroll_pane_id, event, cx);
        }));

    if !matches!(pane.payload, PanePayload::Terminal(_)) {
        pane_el = pane_el.child(
            div()
                .p_2()
                .border_b_1()
                .border_color(rgb(0x333333))
                .text_sm()
                .child(pane.title.clone()),
        );
    }

    pane_el.child(render_pane_body(pane, active, terminal_live))
}

fn render_pane_body(
    pane: &PaneState,
    active: bool,
    terminal_live: Option<&LiveTerminalPane>,
) -> gpui::Div {
    match &pane.payload {
        PanePayload::Terminal(payload) => render_terminal_surface(payload, active, terminal_live),
        _ => div()
            .flex_1()
            .overflow_hidden()
            .p_3()
            .text_sm()
            .text_color(rgb(0xb8b8b8))
            .child(pane_body_label(pane)),
    }
}

fn render_terminal_surface(
    payload: &TerminalPanePayload,
    active: bool,
    terminal_live: Option<&LiveTerminalPane>,
) -> gpui::Div {
    let terminal_snapshot = terminal_live.and_then(|live| live.latest.as_ref());
    let Some(snapshot) = terminal_snapshot else {
        return div()
            .flex_1()
            .overflow_hidden()
            .p_3()
            .bg(rgb(0x080a0d))
            .font(terminal_font())
            .text_size(px(TERMINAL_FONT_SIZE))
            .line_height(px(TERMINAL_CELL_HEIGHT))
            .text_color(rgb(0xc8d0d8))
            .child(terminal_screen_excerpt(&payload.restored_buffer));
    };

    let default_fg = terminal_rgb_to_rgba(snapshot.default_fg);
    let default_bg = terminal_rgb_to_rgba(snapshot.default_bg);
    let debug_timer_label = terminal_live.and_then(|live| live.latency.summary_label());
    let mut surface = div()
        .flex_1()
        .overflow_hidden()
        .p_2()
        .bg(default_bg)
        .font(terminal_font())
        .text_size(px(TERMINAL_FONT_SIZE))
        .line_height(px(TERMINAL_CELL_HEIGHT));

    if let Some(label) = debug_timer_label {
        surface = surface.child(
            div()
                .text_size(px(TERMINAL_DEBUG_TIMER_FONT_SIZE))
                .line_height(px(TERMINAL_DEBUG_TIMER_LINE_HEIGHT))
                .mb(px(TERMINAL_SURFACE_DEBUG_TIMER_MARGIN_BOTTOM))
                .text_color(rgb(if active { 0x6fae74 } else { 0x4f5d68 }))
                .truncate()
                .child(label),
        );
    }

    surface.child(render_terminal_grid(snapshot, default_fg, default_bg))
}

#[derive(Clone, Debug, PartialEq)]
struct TerminalCellRun {
    text: String,
    cell_count: usize,
    fg: Option<Rgba>,
    bg: Option<Rgba>,
    bold: bool,
    italic: bool,
    underline: bool,
}

struct TerminalGridPaintInput {
    cols: u16,
    rows: u16,
    default_bg: Rgba,
    rows_data: Vec<TerminalPaintRowInput>,
    text_rows: Vec<TerminalPaintTextRow>,
}

struct TerminalPaintRowInput {
    background_runs: Vec<TerminalPaintBackgroundRun>,
}

struct TerminalPaintTextRow {
    row_index: usize,
    start_col: usize,
    text: SharedString,
    shape_style: TextRun,
    foreground_runs: Vec<TerminalPaintForegroundRun>,
    style_run_count: usize,
}

struct TerminalPaintForegroundRun {
    start_byte: usize,
    end_byte: usize,
    color: Hsla,
}

struct TerminalPaintBackgroundRun {
    start_col: usize,
    cell_count: usize,
    color: Rgba,
}

struct TerminalPaintSurface {
    input: TerminalGridPaintInput,
    shaped_text_rows: Vec<TerminalShapedTextRow>,
    shape_micros: u64,
}

struct TerminalShapedTextRow {
    input_row_index: usize,
    row_index: usize,
    start_col: usize,
    line: ShapedLine,
}

struct TerminalPaintFonts {
    normal: Font,
}

#[derive(Clone, Copy, Debug, Default)]
struct TerminalRenderProfileSample {
    build_micros: u64,
    shape_micros: u64,
    paint_micros: u64,
    rows: u16,
    cols: u16,
    text_rows: usize,
    text_runs: usize,
    background_runs: usize,
    text_bytes: usize,
}

#[derive(Default)]
struct TerminalRenderProfiler {
    build_micros: VecDeque<u64>,
    shape_micros: VecDeque<u64>,
    paint_micros: VecDeque<u64>,
    text_rows: VecDeque<u64>,
    text_runs: VecDeque<u64>,
    background_runs: VecDeque<u64>,
    text_bytes: VecDeque<u64>,
    last_report_at: Option<Instant>,
}

fn render_terminal_grid(
    snapshot: &TerminalGridSnapshot,
    default_fg: Rgba,
    default_bg: Rgba,
) -> impl IntoElement {
    let build_started_at = Instant::now();
    let input = terminal_paint_input(snapshot, default_fg, default_bg);
    let build_micros = duration_micros(build_started_at.elapsed());
    let width = TERMINAL_CELL_WIDTH * input.cols as f32;
    let height = TERMINAL_CELL_HEIGHT * input.rows as f32;

    canvas(
        move |_bounds, window, _cx| {
            let shape_started_at = Instant::now();
            let shaped_text_rows = input
                .text_rows
                .iter()
                .enumerate()
                .map(|(input_row_index, row)| {
                    let line = window.text_system().shape_line(
                        row.text.clone(),
                        px(TERMINAL_FONT_SIZE),
                        std::slice::from_ref(&row.shape_style),
                        Some(px(TERMINAL_CELL_WIDTH)),
                    );
                    TerminalShapedTextRow {
                        input_row_index,
                        row_index: row.row_index,
                        start_col: row.start_col,
                        line,
                    }
                })
                .collect();
            let shape_micros = duration_micros(shape_started_at.elapsed());
            TerminalPaintSurface {
                input,
                shaped_text_rows,
                shape_micros,
            }
        },
        move |bounds, surface, window, cx| {
            let mut sample = terminal_render_profile_sample(&surface, build_micros);
            let paint_started_at = Instant::now();
            paint_terminal_surface(bounds, surface, window, cx);
            sample.paint_micros = duration_micros(paint_started_at.elapsed());
            record_terminal_render_profile(sample);
        },
    )
    .w(px(width))
    .h(px(height))
    .overflow_hidden()
}

fn terminal_paint_input(
    snapshot: &TerminalGridSnapshot,
    default_fg: Rgba,
    default_bg: Rgba,
) -> TerminalGridPaintInput {
    let normal_font = terminal_font();
    let fonts = TerminalPaintFonts {
        normal: normal_font,
    };
    let mut rows_data = Vec::with_capacity(snapshot.rows_data.len());
    let mut text_rows = Vec::new();

    for (row_index, row) in snapshot.rows_data.iter().enumerate() {
        let mut background_runs = Vec::new();
        let mut start_col = 0usize;
        let runs = terminal_cell_runs(row_index as u16, row, snapshot);

        if let Some(text_row) = terminal_text_row(row_index, &runs, &fonts, default_fg) {
            text_rows.push(text_row);
        }

        for run in runs {
            if let Some(bg) = run.bg
                && bg != default_bg
            {
                background_runs.push(TerminalPaintBackgroundRun {
                    start_col,
                    cell_count: run.cell_count,
                    color: bg,
                });
            }
            start_col += run.cell_count;
        }

        rows_data.push(TerminalPaintRowInput { background_runs });
    }

    TerminalGridPaintInput {
        cols: snapshot.cols,
        rows: snapshot.rows,
        default_bg,
        rows_data,
        text_rows,
    }
}

fn terminal_text_row(
    row_index: usize,
    runs: &[TerminalCellRun],
    fonts: &TerminalPaintFonts,
    default_fg: Rgba,
) -> Option<TerminalPaintTextRow> {
    let full_text: String = runs.iter().map(|run| run.text.as_str()).collect();
    let (visible_start_cell, visible_end_cell) = terminal_visible_text_cell_range(&full_text)?;
    let visible_text: String = full_text
        .chars()
        .skip(visible_start_cell)
        .take(visible_end_cell - visible_start_cell)
        .collect();
    let mut foreground_runs: Vec<TerminalPaintForegroundRun> = Vec::new();
    let mut style_run_count = 0usize;
    let mut run_start_cell = 0usize;

    for run in runs {
        let run_end_cell = run_start_cell + run.text.chars().count();
        let overlap_start = run_start_cell.max(visible_start_cell);
        let overlap_end = run_end_cell.min(visible_end_cell);
        if overlap_start < overlap_end {
            let style_text: String = run
                .text
                .chars()
                .skip(overlap_start - run_start_cell)
                .take(overlap_end - overlap_start)
                .collect();
            if !style_text.is_empty() {
                style_run_count += 1;
                let skipped_visible_cells = overlap_start - visible_start_cell;
                let start_byte = terminal_byte_index_for_cell(&visible_text, skipped_visible_cells);
                let end_byte = start_byte + style_text.len();
                let color = Hsla::from(run.fg.unwrap_or(default_fg));
                if let Some(last) = foreground_runs.last_mut()
                    && last.end_byte == start_byte
                    && last.color == color
                {
                    last.end_byte = end_byte;
                } else {
                    foreground_runs.push(TerminalPaintForegroundRun {
                        start_byte,
                        end_byte,
                        color,
                    });
                }
            }
        }
        run_start_cell = run_end_cell;
    }

    if visible_text.is_empty() || foreground_runs.is_empty() {
        return None;
    }
    let visible_text_len = visible_text.len();

    Some(TerminalPaintTextRow {
        row_index,
        start_col: visible_start_cell,
        text: SharedString::from(visible_text),
        shape_style: terminal_text_style(visible_text_len, fonts, default_fg),
        foreground_runs,
        style_run_count,
    })
}

fn terminal_byte_index_for_cell(text: &str, cell_index: usize) -> usize {
    text.char_indices()
        .nth(cell_index)
        .map(|(byte_index, _)| byte_index)
        .unwrap_or(text.len())
}

fn terminal_visible_text_cell_range(text: &str) -> Option<(usize, usize)> {
    let start = text.chars().position(|ch| ch != ' ')?;
    let mut end = start + 1;
    for (index, ch) in text.chars().enumerate().skip(start + 1) {
        if ch != ' ' {
            end = index + 1;
        }
    }
    Some((start, end))
}

fn terminal_text_style(len: usize, fonts: &TerminalPaintFonts, default_fg: Rgba) -> TextRun {
    TextRun {
        len,
        font: fonts.normal.clone(),
        color: Hsla::from(default_fg),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

fn paint_terminal_surface(
    bounds: Bounds<gpui::Pixels>,
    surface: TerminalPaintSurface,
    window: &mut Window,
    cx: &mut App,
) {
    window.paint_quad(fill(bounds, surface.input.default_bg));
    for (row_index, row) in surface.input.rows_data.iter().enumerate() {
        let row_top = bounds.origin.y + px(row_index as f32 * TERMINAL_CELL_HEIGHT);
        for run in &row.background_runs {
            let origin = point(
                bounds.origin.x + px(run.start_col as f32 * TERMINAL_CELL_WIDTH),
                row_top,
            );
            let size = size(
                px(run.cell_count as f32 * TERMINAL_CELL_WIDTH),
                px(TERMINAL_CELL_HEIGHT),
            );
            window.paint_quad(fill(Bounds { origin, size }, run.color));
        }
    }

    for run in surface.shaped_text_rows.iter() {
        let origin = point(
            bounds.origin.x + px(run.start_col as f32 * TERMINAL_CELL_WIDTH),
            bounds.origin.y + px(run.row_index as f32 * TERMINAL_CELL_HEIGHT),
        );
        let input_row = &surface.input.text_rows[run.input_row_index];
        let _ = paint_terminal_text_row(origin, input_row, &run.line, window, cx);
    }
}

fn paint_terminal_text_row(
    origin: gpui::Point<gpui::Pixels>,
    row: &TerminalPaintTextRow,
    line: &ShapedLine,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Result<()> {
    let padding_top = (px(TERMINAL_CELL_HEIGHT) - line.ascent - line.descent) / 2.0;
    let baseline_offset = point(px(0.0), padding_top + line.ascent);
    let text_system = cx.text_system().clone();
    let mut glyph_origin = origin;
    let mut prev_glyph_position = gpui::Point::default();
    let mut foreground_run_index = 0usize;

    for run in &line.runs {
        let max_glyph_size = text_system.bounding_box(run.font_id, line.font_size).size;
        for glyph in &run.glyphs {
            glyph_origin.x += glyph.position.x - prev_glyph_position.x;
            prev_glyph_position = glyph.position;

            while row
                .foreground_runs
                .get(foreground_run_index)
                .is_some_and(|run| glyph.index >= run.end_byte)
            {
                foreground_run_index += 1;
            }
            let color = row
                .foreground_runs
                .get(foreground_run_index)
                .filter(|run| glyph.index >= run.start_byte)
                .map(|run| run.color)
                .unwrap_or(row.shape_style.color);

            let max_glyph_bounds = Bounds {
                origin: glyph_origin,
                size: max_glyph_size,
            };
            let content_mask = window.content_mask();
            if !max_glyph_bounds.intersects(&content_mask.bounds) {
                continue;
            }

            if glyph.is_emoji {
                window.paint_emoji(
                    glyph_origin + baseline_offset,
                    run.font_id,
                    glyph.id,
                    line.font_size,
                )?;
            } else {
                window.paint_glyph(
                    glyph_origin + baseline_offset,
                    run.font_id,
                    glyph.id,
                    line.font_size,
                    color,
                )?;
            }
        }
    }
    Ok(())
}

fn terminal_render_profile_sample(
    surface: &TerminalPaintSurface,
    build_micros: u64,
) -> TerminalRenderProfileSample {
    TerminalRenderProfileSample {
        build_micros,
        shape_micros: surface.shape_micros,
        paint_micros: 0,
        rows: surface.input.rows,
        cols: surface.input.cols,
        text_rows: surface.input.text_rows.len(),
        text_runs: surface
            .input
            .text_rows
            .iter()
            .map(|row| row.style_run_count)
            .sum(),
        background_runs: surface
            .input
            .rows_data
            .iter()
            .map(|row| row.background_runs.len())
            .sum(),
        text_bytes: surface
            .input
            .text_rows
            .iter()
            .map(|row| row.text.len())
            .sum(),
    }
}

fn record_terminal_render_profile(sample: TerminalRenderProfileSample) {
    if !terminal_render_profile_enabled() {
        return;
    }

    let profiler =
        TERMINAL_RENDER_PROFILER.get_or_init(|| Mutex::new(TerminalRenderProfiler::default()));
    let Ok(mut profiler) = profiler.lock() else {
        return;
    };
    profiler.record(sample);
    profiler.maybe_report(sample);
}

fn terminal_render_profile_summary() -> Option<String> {
    if !terminal_render_profile_enabled() {
        return None;
    }

    let profiler = TERMINAL_RENDER_PROFILER.get()?;
    let profiler = profiler.lock().ok()?;
    profiler.summary()
}

fn terminal_render_profile_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("OCTTY_TERMINAL_PROFILE")
            .ok()
            .is_some_and(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
    })
}

static TERMINAL_RENDER_PROFILER: OnceLock<Mutex<TerminalRenderProfiler>> = OnceLock::new();

impl TerminalRenderProfiler {
    fn record(&mut self, sample: TerminalRenderProfileSample) {
        push_latency_sample(&mut self.build_micros, sample.build_micros);
        push_latency_sample(&mut self.shape_micros, sample.shape_micros);
        push_latency_sample(&mut self.paint_micros, sample.paint_micros);
        push_latency_sample(&mut self.text_rows, sample.text_rows as u64);
        push_latency_sample(&mut self.text_runs, sample.text_runs as u64);
        push_latency_sample(&mut self.background_runs, sample.background_runs as u64);
        push_latency_sample(&mut self.text_bytes, sample.text_bytes as u64);
    }

    fn summary(&self) -> Option<String> {
        let build = latency_summary(&self.build_micros)?;
        let shape = latency_summary(&self.shape_micros)?;
        let paint = latency_summary(&self.paint_micros)?;
        Some(format!(
            "render build {build} · shape {shape} · paint {paint}"
        ))
    }

    fn maybe_report(&mut self, sample: TerminalRenderProfileSample) {
        let now = Instant::now();
        if self
            .last_report_at
            .is_some_and(|reported_at| now.duration_since(reported_at) < Duration::from_secs(1))
        {
            return;
        }
        self.last_report_at = Some(now);

        let Some(summary) = self.summary() else {
            return;
        };
        eprintln!(
            "octty terminal render profile: {summary} · grid {}x{} · text rows {} · text runs {} · bg runs {} · text bytes {}",
            sample.cols,
            sample.rows,
            count_summary(&self.text_rows).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.text_runs).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.background_runs).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.text_bytes).unwrap_or_else(|| "n/a".to_owned())
        );
    }
}

fn terminal_cell_runs(
    row_index: u16,
    row: &octty_term::live::TerminalRowSnapshot,
    snapshot: &TerminalGridSnapshot,
) -> Vec<TerminalCellRun> {
    let mut runs: Vec<TerminalCellRun> = Vec::new();
    for (col, cell) in row.cells.iter().enumerate() {
        let is_cursor = snapshot.cursor.as_ref().is_some_and(|cursor| {
            cursor.visible && cursor.row == row_index && cursor.col == col as u16
        });
        let mut fg = cell.fg.map(terminal_rgb_to_rgba);
        let mut bg = cell.bg.map(terminal_rgb_to_rgba);
        if cell.inverse {
            std::mem::swap(&mut fg, &mut bg);
        }
        if is_cursor {
            fg = Some(terminal_rgb_to_rgba(snapshot.default_bg));
            bg = Some(terminal_rgb_to_rgba(snapshot.default_fg));
        }
        let text = if cell.text.is_empty() {
            " ".to_owned()
        } else {
            cell.text.clone()
        };
        let can_extend = runs.last().is_some_and(|run| {
            run.fg == fg
                && run.bg == bg
                && run.bold == cell.bold
                && run.italic == cell.italic
                && run.underline == cell.underline
        });
        if can_extend {
            let run = runs.last_mut().expect("checked above");
            run.text.push_str(&text);
            run.cell_count += 1;
        } else {
            runs.push(TerminalCellRun {
                text,
                cell_count: 1,
                fg,
                bg,
                bold: cell.bold,
                italic: cell.italic,
                underline: cell.underline,
            });
        }
    }
    runs
}

fn pane_body_label(pane: &PaneState) -> String {
    match &pane.payload {
        PanePayload::Terminal(payload) => {
            let screen = terminal_screen_excerpt(&payload.restored_buffer);
            if screen.is_empty() {
                String::new()
            } else {
                screen
            }
        }
        PanePayload::Note(payload) => format!(
            "Note placeholder · {}",
            payload.note_path.as_deref().unwrap_or("no note selected")
        ),
        PanePayload::Diff(_) => "Diff placeholder · JJ diff will render here.".to_owned(),
        PanePayload::Browser(payload) => format!("Browser deferred · {}", payload.url),
    }
}

fn terminal_screen_excerpt(screen: &str) -> String {
    let lines: Vec<_> = screen
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.len().saturating_sub(8);
    lines[start..].join("\n")
}

fn terminal_paste_bytes(text: &str) -> Vec<u8> {
    text.replace("\r\n", "\n").replace('\n', "\r").into_bytes()
}

fn push_latency_sample(samples: &mut VecDeque<u64>, micros: u64) {
    if samples.len() == TERMINAL_LATENCY_SAMPLE_LIMIT {
        samples.pop_front();
    }
    samples.push_back(micros);
}

fn duration_micros(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

fn latency_summary(samples: &VecDeque<u64>) -> Option<String> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted: Vec<_> = samples.iter().copied().collect();
    sorted.sort_unstable();
    let p50 = latency_percentile(&sorted, 50);
    let p95 = latency_percentile(&sorted, 95);
    let max = *sorted.last().unwrap_or(&p95);
    Some(format!(
        "p50 {} p95 {} max {}",
        format_latency_micros(p50),
        format_latency_micros(p95),
        format_latency_micros(max)
    ))
}

fn count_summary(samples: &VecDeque<u64>) -> Option<String> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted: Vec<_> = samples.iter().copied().collect();
    sorted.sort_unstable();
    let p50 = latency_percentile(&sorted, 50);
    let p95 = latency_percentile(&sorted, 95);
    let max = *sorted.last().unwrap_or(&p95);
    Some(format!("p50 {p50} p95 {p95} max {max}"))
}

fn latency_percentile(sorted_micros: &[u64], percentile: usize) -> u64 {
    let index = ((sorted_micros.len().saturating_sub(1) * percentile) / 100)
        .min(sorted_micros.len().saturating_sub(1));
    sorted_micros[index]
}

fn format_latency_micros(micros: u64) -> String {
    if micros >= 1_000 {
        format!("{:.1}ms", micros as f64 / 1_000.0)
    } else {
        format!("{micros}us")
    }
}

fn terminal_font() -> Font {
    let mut terminal_font = font(terminal_font_family());
    terminal_font.features = FontFeatures::disable_ligatures();
    terminal_font.fallbacks = Some(FontFallbacks::from_fonts(vec![
        "DejaVu Sans Mono".to_owned(),
        "Liberation Mono".to_owned(),
        "Noto Sans Mono".to_owned(),
        "Cascadia Mono".to_owned(),
        "Menlo".to_owned(),
        "Consolas".to_owned(),
        "monospace".to_owned(),
    ]));
    terminal_font
}

fn terminal_font_family() -> String {
    std::env::var("OCTTY_RS_TERMINAL_FONT_FAMILY")
        .or_else(|_| std::env::var("OCTTY_TERMINAL_FONT_FAMILY"))
        .ok()
        .and_then(|family| first_font_family(&family))
        .unwrap_or_else(|| DEFAULT_TERMINAL_FONT_FAMILY.to_owned())
}

fn first_font_family(input: &str) -> Option<String> {
    input
        .split(',')
        .map(|family| family.trim().trim_matches('"').trim_matches('\'').trim())
        .find(|family| !family.is_empty() && !family.eq_ignore_ascii_case("monospace"))
        .map(str::to_owned)
}

fn default_terminal_grid_for_pane() -> (u16, u16) {
    (
        (720.0_f32 / TERMINAL_CELL_WIDTH).floor() as u16,
        (360.0_f32 / TERMINAL_CELL_HEIGHT).floor() as u16,
    )
}

fn taskspace_height_for_viewport(viewport_height: f32) -> f32 {
    (viewport_height - TERMINAL_TASKSPACE_VERTICAL_CHROME_HEIGHT).max(160.0)
}

fn taskspace_width_for_viewport(viewport_width: f32) -> f32 {
    (viewport_width - WORKSPACE_SIDEBAR_WIDTH - TASKSPACE_HORIZONTAL_PADDING).max(240.0)
}

fn taskspace_viewport_offset(snapshot: &WorkspaceSnapshot, viewport_width: f32) -> f32 {
    let Some((active_left, active_width, total_width)) = active_column_metrics(snapshot) else {
        return 0.0;
    };
    let max_offset = (total_width - viewport_width).max(0.0);
    let centered_offset = active_left + (active_width / 2.0) - (viewport_width / 2.0);
    centered_offset.clamp(0.0, max_offset)
}

fn active_column_metrics(snapshot: &WorkspaceSnapshot) -> Option<(f32, f32, f32)> {
    let active_pane_id = snapshot
        .active_pane_id
        .as_deref()
        .or_else(|| first_center_pane_id(snapshot))?;

    let mut total_width = 0.0;
    let mut active_left = None;
    let mut active_width = None;
    let mut visible_column_count = 0usize;

    for column_id in &snapshot.center_column_ids {
        let Some(column) = snapshot.columns.get(column_id) else {
            continue;
        };
        if visible_column_count > 0 {
            total_width += TASKSPACE_PANEL_GAP;
        }
        if column
            .pane_ids
            .iter()
            .any(|pane_id| pane_id == active_pane_id)
        {
            active_left = Some(total_width);
            active_width = Some(column.width_px as f32);
        }
        total_width += column.width_px as f32;
        visible_column_count += 1;
    }

    Some((active_left?, active_width?, total_width))
}

fn terminal_resize_requests(
    snapshot: Option<&WorkspaceSnapshot>,
    taskspace_height: f32,
) -> Vec<(String, String, u16, u16)> {
    let Some(snapshot) = snapshot else {
        return Vec::new();
    };
    let mut requests = Vec::new();
    for column_id in &snapshot.center_column_ids {
        let Some(column) = snapshot.columns.get(column_id) else {
            continue;
        };
        let pane_count = column.pane_ids.len().max(1);
        let pane_height =
            (taskspace_height - (pane_count.saturating_sub(1) as f32 * 12.0)) / pane_count as f32;
        let terminal_height =
            (pane_height - TERMINAL_SURFACE_CHROME_HEIGHT).max(TERMINAL_CELL_HEIGHT);
        let cols = ((column.width_px as f32 - 24.0) / TERMINAL_CELL_WIDTH)
            .floor()
            .max(20.0) as u16;
        let rows = (terminal_height / TERMINAL_CELL_HEIGHT).floor().max(4.0) as u16;
        for pane_id in &column.pane_ids {
            let Some(pane) = snapshot.panes.get(pane_id) else {
                continue;
            };
            if matches!(pane.payload, PanePayload::Terminal(_)) {
                requests.push((snapshot.workspace_id.clone(), pane_id.clone(), cols, rows));
            }
        }
    }
    requests
}

fn live_terminal_key(workspace_id: &str, pane_id: &str) -> String {
    format!("{workspace_id}:{pane_id}")
}

fn split_live_terminal_key(key: &str) -> Option<(&str, &str)> {
    key.split_once(':')
}

fn terminal_rgb_to_rgba(color: TerminalRgb) -> Rgba {
    Rgba {
        r: color.r as f32 / 255.0,
        g: color.g as f32 / 255.0,
        b: color.b as f32 / 255.0,
        a: 1.0,
    }
}

fn workspace_status_label(state: &WorkspaceState) -> &'static str {
    match state {
        WorkspaceState::Published => "published",
        WorkspaceState::MergedLocal => "merged local",
        WorkspaceState::Draft => "draft",
        WorkspaceState::Conflicted => "conflicted",
        WorkspaceState::Unknown => "unknown",
    }
}

trait WorkspaceDisplayName {
    fn display_name_or_workspace_name(&self) -> &str;
}

impl WorkspaceDisplayName for WorkspaceSummary {
    fn display_name_or_workspace_name(&self) -> &str {
        if self.display_name.is_empty() {
            &self.workspace_name
        } else {
            &self.display_name
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printable_keys_become_terminal_text() {
        let input =
            live_terminal_input_from_key_parts("a", Some("a"), false, false, false, false, false)
                .expect("printable input");
        assert_eq!(input.key, LiveTerminalKey::Character('a'));
        assert_eq!(input.text.as_deref(), Some("a"));

        let shifted =
            live_terminal_input_from_key_parts("a", Some("A"), false, false, true, false, false)
                .expect("shifted input");
        assert_eq!(shifted.key, LiveTerminalKey::Character('a'));
        assert_eq!(shifted.text.as_deref(), Some("A"));
        assert!(shifted.modifiers.shift);
    }

    #[test]
    fn named_keys_become_terminal_keys() {
        let enter =
            live_terminal_input_from_key_parts("enter", None, false, false, false, false, false)
                .expect("enter input");
        assert_eq!(enter.key, LiveTerminalKey::Enter);

        let backspace = live_terminal_input_from_key_parts(
            "backspace",
            None,
            false,
            false,
            false,
            false,
            false,
        )
        .expect("backspace input");
        assert_eq!(backspace.key, LiveTerminalKey::Backspace);
    }

    #[test]
    fn terminal_input_preserves_workspace_shortcuts() {
        assert_eq!(
            live_terminal_input_from_key_parts("1", Some("!"), true, false, true, false, false),
            None
        );
    }

    #[test]
    fn terminal_input_preserves_paste_shortcut() {
        assert_eq!(
            live_terminal_input_from_key_parts("v", Some("V"), true, false, true, false, false),
            None
        );
    }

    #[test]
    fn terminal_input_preserves_pane_action_shortcuts() {
        assert_eq!(
            live_terminal_input_from_key_parts("left", None, true, false, true, false, false),
            None
        );
        assert_eq!(
            live_terminal_input_from_key_parts("w", Some("W"), true, false, true, false, false),
            None
        );
    }

    #[test]
    fn terminal_input_preserves_column_resize_shortcuts() {
        assert_eq!(
            live_terminal_input_from_key_parts("left", None, true, true, false, false, false),
            None
        );
        assert_eq!(
            live_terminal_input_from_key_parts("right", None, true, true, false, false, false),
            None
        );
    }

    #[test]
    fn control_letters_keep_control_modifier_for_encoder() {
        let input = live_terminal_input_from_key_parts("c", None, true, false, false, false, false)
            .expect("control input");
        assert_eq!(input.key, LiveTerminalKey::Character('c'));
        assert!(input.modifiers.control);
    }

    #[test]
    fn css_font_stack_prefers_first_real_family() {
        assert_eq!(
            first_font_family("\"Iosevka Term\", monospace").as_deref(),
            Some("Iosevka Term")
        );
        assert_eq!(
            first_font_family("monospace, \"JetBrains Mono\"").as_deref(),
            Some("JetBrains Mono")
        );
    }

    #[test]
    fn terminal_paste_normalizes_newlines_to_carriage_returns() {
        assert_eq!(
            terminal_paste_bytes("one\ntwo\r\nthree"),
            b"one\rtwo\rthree"
        );
    }

    #[test]
    fn latency_summary_reports_millisecond_percentiles() {
        let samples = VecDeque::from([500, 1_500, 8_000]);
        let summary = latency_summary(&samples).expect("latency summary");
        assert!(summary.contains("p50 1.5ms"));
        assert!(summary.contains("max 8.0ms"));
    }

    #[test]
    fn terminal_notification_drain_coalesces_queued_wakeups() {
        let (tx, mut rx) = mpsc::unbounded();
        tx.unbounded_send(()).expect("first wakeup");
        tx.unbounded_send(()).expect("second wakeup");

        drain_pending_terminal_notifications(&mut rx);

        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn terminal_snapshot_coalesce_skips_delay_for_recent_focused_input() {
        assert_eq!(
            terminal_snapshot_coalesce_interval(true, true),
            Duration::ZERO
        );
        assert_eq!(
            terminal_snapshot_coalesce_interval(true, false),
            TERMINAL_FOCUSED_FRAME_INTERVAL
        );
        assert_eq!(
            terminal_snapshot_coalesce_interval(false, true),
            TERMINAL_BACKGROUND_FRAME_INTERVAL
        );
    }

    #[test]
    fn terminal_paint_input_shapes_only_visible_text_cells() {
        let default_fg = TerminalRgb {
            r: 200,
            g: 200,
            b: 200,
        };
        let default_bg = TerminalRgb { r: 0, g: 0, b: 0 };
        let snapshot = TerminalGridSnapshot {
            session_id: "session-1".to_owned(),
            cols: 3,
            rows: 1,
            default_fg,
            default_bg,
            cursor: None,
            rows_data: vec![octty_term::live::TerminalRowSnapshot {
                cells: vec![
                    octty_term::live::TerminalCellSnapshot {
                        text: String::new(),
                        fg: None,
                        bg: None,
                        bold: false,
                        italic: false,
                        underline: false,
                        inverse: false,
                    },
                    octty_term::live::TerminalCellSnapshot {
                        text: "a".to_owned(),
                        fg: None,
                        bg: None,
                        bold: false,
                        italic: false,
                        underline: false,
                        inverse: false,
                    },
                    octty_term::live::TerminalCellSnapshot {
                        text: String::new(),
                        fg: None,
                        bg: None,
                        bold: false,
                        italic: false,
                        underline: false,
                        inverse: false,
                    },
                ],
            }],
            plain_text: " a\n".to_owned(),
            timing: octty_term::live::TerminalSnapshotTiming::default(),
        };

        let input = terminal_paint_input(
            &snapshot,
            terminal_rgb_to_rgba(default_fg),
            terminal_rgb_to_rgba(default_bg),
        );

        assert_eq!(input.text_rows.len(), 1);
        assert_eq!(input.text_rows[0].start_col, 1);
        assert_eq!(input.text_rows[0].text.as_ref(), "a");
        assert_eq!(input.text_rows[0].shape_style.len, 1);
        assert_eq!(input.text_rows[0].foreground_runs.len(), 1);
        assert!(input.rows_data[0].background_runs.is_empty());
    }

    #[test]
    fn terminal_picker_preview_workload_has_dense_runs_and_backgrounds() {
        let snapshot = picker_preview_snapshot(7, 120, 40);
        let input = terminal_paint_input(
            &snapshot,
            terminal_rgb_to_rgba(snapshot.default_fg),
            terminal_rgb_to_rgba(snapshot.default_bg),
        );
        let background_runs: usize = input
            .rows_data
            .iter()
            .map(|row| row.background_runs.len())
            .sum();
        let style_runs: usize = input.text_rows.iter().map(|row| row.style_run_count).sum();

        assert_eq!(input.cols, 120);
        assert_eq!(input.rows, 40);
        assert!(input.text_rows.len() <= 40);
        assert!(input.text_rows.len() > 30);
        assert!(style_runs > 180);
        assert!(background_runs > 40);
    }

    #[test]
    #[ignore = "profiling workload; run with --ignored --nocapture"]
    fn terminal_picker_preview_paint_input_profile() {
        let mut samples = VecDeque::new();
        let mut text_rows = VecDeque::new();
        let mut text_runs = VecDeque::new();
        let mut background_runs = VecDeque::new();

        for frame in 0..240 {
            let snapshot = picker_preview_snapshot(frame, 120, 40);
            let started_at = Instant::now();
            let input = terminal_paint_input(
                &snapshot,
                terminal_rgb_to_rgba(snapshot.default_fg),
                terminal_rgb_to_rgba(snapshot.default_bg),
            );
            push_latency_sample(&mut samples, duration_micros(started_at.elapsed()));
            push_latency_sample(&mut text_rows, input.text_rows.len() as u64);
            push_latency_sample(
                &mut text_runs,
                input
                    .text_rows
                    .iter()
                    .map(|row| row.style_run_count)
                    .sum::<usize>() as u64,
            );
            push_latency_sample(
                &mut background_runs,
                input
                    .rows_data
                    .iter()
                    .map(|row| row.background_runs.len())
                    .sum::<usize>() as u64,
            );
            std::hint::black_box(input);
        }

        println!(
            "picker preview paint-input: {} · text rows {} · text runs {} · background runs {}",
            latency_summary(&samples).unwrap(),
            count_summary(&text_rows).unwrap(),
            count_summary(&text_runs).unwrap(),
            count_summary(&background_runs).unwrap()
        );
    }

    #[test]
    fn pane_navigation_moves_between_columns() {
        let mut snapshot = create_default_snapshot("workspace-1");
        snapshot = add_pane(snapshot, create_pane_state(PaneType::Note, "/tmp", None));
        let first = snapshot.active_pane_id.clone().expect("first pane");
        snapshot = add_pane(snapshot, create_pane_state(PaneType::Diff, "/tmp", None));
        let second = snapshot.active_pane_id.clone().expect("second pane");

        snapshot.active_pane_id = Some(first.clone());
        assert_eq!(
            pane_navigation_target(&snapshot, PaneNavigationDirection::Right).as_deref(),
            Some(second.as_str())
        );

        snapshot.active_pane_id = Some(second);
        assert_eq!(
            pane_navigation_target(&snapshot, PaneNavigationDirection::Left).as_deref(),
            Some(first.as_str())
        );
    }

    #[test]
    fn taskspace_viewport_offset_keeps_focused_column_visible() {
        let mut snapshot = create_default_snapshot("workspace-1");
        snapshot = add_pane(snapshot, create_pane_state(PaneType::Note, "/tmp", None));
        let first = snapshot.active_pane_id.clone().expect("first pane");
        snapshot = add_pane(snapshot, create_pane_state(PaneType::Diff, "/tmp", None));
        let second = snapshot.active_pane_id.clone().expect("second pane");

        snapshot.active_pane_id = Some(first);
        assert_eq!(taskspace_viewport_offset(&snapshot, 560.0), 0.0);

        snapshot.active_pane_id = Some(second);
        assert_eq!(taskspace_viewport_offset(&snapshot, 560.0), 602.0);
    }

    #[test]
    fn taskspace_viewport_offset_stays_zero_when_columns_fit() {
        let mut snapshot = create_default_snapshot("workspace-1");
        snapshot = add_pane(snapshot, create_pane_state(PaneType::Note, "/tmp", None));
        snapshot = add_pane(snapshot, create_pane_state(PaneType::Diff, "/tmp", None));

        assert_eq!(taskspace_viewport_offset(&snapshot, 1_400.0), 0.0);
    }

    #[test]
    fn terminal_resize_rows_subtract_all_visible_chrome() {
        let snapshot = add_pane(
            create_default_snapshot("workspace-1"),
            create_pane_state(PaneType::Shell, "/tmp", None),
        );

        let requests = terminal_resize_requests(Some(&snapshot), 1_000.0);

        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].2, 87);
        assert_eq!(requests[0].3, 53);
    }

    #[test]
    fn resize_focused_column_only_changes_active_column_width() {
        let mut snapshot = create_default_snapshot("workspace-1");
        snapshot = add_pane(snapshot, create_pane_state(PaneType::Note, "/tmp", None));
        let first_column_id = snapshot.center_column_ids[0].clone();
        snapshot = add_pane(snapshot, create_pane_state(PaneType::Diff, "/tmp", None));
        let second_column_id = snapshot.center_column_ids[1].clone();
        let first_width = snapshot.columns[&first_column_id].width_px;
        let second_width = snapshot.columns[&second_column_id].width_px;

        let resized =
            resize_focused_column_in_snapshot(&mut snapshot, ColumnResizeDirection::Wider)
                .expect("resized focused column");

        assert_eq!(snapshot.columns[&first_column_id].width_px, first_width);
        assert_eq!(resized, second_width + COLUMN_WIDTH_STEP_PX);
        assert_eq!(snapshot.columns[&second_column_id].width_px, resized);
    }

    #[test]
    fn resize_focused_column_clamps_to_minimum_width() {
        let mut snapshot = create_default_snapshot("workspace-1");
        snapshot = add_pane(snapshot, create_pane_state(PaneType::Note, "/tmp", None));
        let column_id = snapshot.center_column_ids[0].clone();
        snapshot.columns.get_mut(&column_id).unwrap().width_px = MIN_COLUMN_WIDTH_PX;

        assert_eq!(
            resize_focused_column_in_snapshot(&mut snapshot, ColumnResizeDirection::Slimmer),
            None
        );
        assert_eq!(snapshot.columns[&column_id].width_px, MIN_COLUMN_WIDTH_PX);
    }

    #[test]
    fn pane_navigation_moves_within_column() {
        let mut snapshot = create_default_snapshot("workspace-1");
        snapshot = add_pane(snapshot, create_pane_state(PaneType::Note, "/tmp", None));
        let first = snapshot.active_pane_id.clone().expect("first pane");
        let second_pane = create_pane_state(PaneType::Diff, "/tmp", None);
        let second = second_pane.id.clone();
        let column_id = snapshot.center_column_ids[0].clone();
        snapshot.panes.insert(second.clone(), second_pane);
        let column = snapshot.columns.get_mut(&column_id).expect("center column");
        column.pane_ids.push(second.clone());
        column.height_fractions = vec![0.5, 0.5];

        snapshot.active_pane_id = Some(first.clone());
        assert_eq!(
            pane_navigation_target(&snapshot, PaneNavigationDirection::Down).as_deref(),
            Some(second.as_str())
        );

        snapshot.active_pane_id = Some(second);
        assert_eq!(
            pane_navigation_target(&snapshot, PaneNavigationDirection::Up).as_deref(),
            Some(first.as_str())
        );
    }

    fn picker_preview_snapshot(frame: usize, cols: u16, rows: u16) -> TerminalGridSnapshot {
        let default_fg = TerminalRgb {
            r: 210,
            g: 216,
            b: 222,
        };
        let default_bg = TerminalRgb {
            r: 18,
            g: 20,
            b: 22,
        };
        let mut rows_data = Vec::with_capacity(rows as usize);
        for row_index in 0..rows as usize {
            let mut cells = vec![picker_cell("", None, None, false, false); cols as usize];
            if row_index == 0 {
                write_picker_text(
                    &mut cells,
                    0,
                    "  Find files                                      Preview",
                    Some(TerminalRgb {
                        r: 240,
                        g: 240,
                        b: 240,
                    }),
                    Some(TerminalRgb {
                        r: 42,
                        g: 48,
                        b: 56,
                    }),
                    true,
                    false,
                );
            } else {
                let selected = row_index == (frame % (rows as usize - 2)) + 1;
                let file_name = format!(
                    " crates/octty-app/src/{:03}_picker_case.rs ",
                    (frame + row_index) % 173
                );
                write_picker_text(
                    &mut cells,
                    0,
                    &format!("{file_name:40}"),
                    Some(if selected {
                        TerminalRgb {
                            r: 245,
                            g: 250,
                            b: 255,
                        }
                    } else {
                        TerminalRgb {
                            r: 170,
                            g: 184,
                            b: 194,
                        }
                    }),
                    selected.then_some(TerminalRgb {
                        r: 28,
                        g: 92,
                        b: 72,
                    }),
                    selected,
                    false,
                );
                write_picker_preview_line(&mut cells, row_index, frame, 43);
            }
            rows_data.push(octty_term::live::TerminalRowSnapshot { cells });
        }

        TerminalGridSnapshot {
            session_id: "picker-preview-profile".to_owned(),
            cols,
            rows,
            default_fg,
            default_bg,
            cursor: Some(octty_term::live::TerminalCursorSnapshot {
                col: 2,
                row: ((frame % (rows as usize - 2)) + 1) as u16,
                visible: true,
            }),
            rows_data,
            plain_text: String::new(),
            timing: octty_term::live::TerminalSnapshotTiming::default(),
        }
    }

    fn write_picker_preview_line(
        cells: &mut [octty_term::live::TerminalCellSnapshot],
        row_index: usize,
        frame: usize,
        start_col: usize,
    ) {
        let line_no = (row_index + frame) % 97;
        write_picker_text(
            cells,
            start_col,
            &format!("{line_no:>3} "),
            Some(TerminalRgb {
                r: 105,
                g: 116,
                b: 126,
            }),
            Some(TerminalRgb {
                r: 30,
                g: 34,
                b: 38,
            }),
            false,
            false,
        );
        let segments = [
            (
                "let ".to_owned(),
                TerminalRgb {
                    r: 235,
                    g: 118,
                    b: 135,
                },
                false,
            ),
            (
                format!("preview_{line_no}"),
                TerminalRgb {
                    r: 132,
                    g: 204,
                    b: 244,
                },
                false,
            ),
            (
                " = ".to_owned(),
                TerminalRgb {
                    r: 210,
                    g: 216,
                    b: 222,
                },
                false,
            ),
            (
                format!("render_case({frame}, {row_index});"),
                TerminalRgb {
                    r: 166,
                    g: 218,
                    b: 149,
                },
                false,
            ),
        ];
        let mut col = start_col + 4;
        for (text, color, bold) in segments {
            write_picker_text(cells, col, &text, Some(color), None, bold, false);
            col += text.chars().count();
        }
        if row_index % 5 == 0 {
            write_picker_text(
                cells,
                start_col + 5,
                " changed ",
                Some(TerminalRgb {
                    r: 18,
                    g: 20,
                    b: 22,
                }),
                Some(TerminalRgb {
                    r: 238,
                    g: 212,
                    b: 132,
                }),
                true,
                false,
            );
        }
    }

    fn write_picker_text(
        cells: &mut [octty_term::live::TerminalCellSnapshot],
        start_col: usize,
        text: &str,
        fg: Option<TerminalRgb>,
        bg: Option<TerminalRgb>,
        bold: bool,
        italic: bool,
    ) {
        for (offset, ch) in text.chars().enumerate() {
            let Some(cell) = cells.get_mut(start_col + offset) else {
                break;
            };
            *cell = picker_cell(&ch.to_string(), fg, bg, bold, italic);
        }
    }

    fn picker_cell(
        text: &str,
        fg: Option<TerminalRgb>,
        bg: Option<TerminalRgb>,
        bold: bool,
        italic: bool,
    ) -> octty_term::live::TerminalCellSnapshot {
        octty_term::live::TerminalCellSnapshot {
            text: text.to_owned(),
            fg,
            bg,
            bold,
            italic,
            underline: false,
            inverse: false,
        }
    }
}
