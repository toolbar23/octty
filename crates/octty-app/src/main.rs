use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap, VecDeque},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use futures::{StreamExt, channel::mpsc};
use gpui::{
    Action, AnyView, App, Application, Bounds, Context, Entity, FocusHandle, Font, FontFallbacks,
    FontFeatures, Hsla, IntoElement, KeyBinding, KeyDownEvent, Menu, MenuItem, MouseButton, Render,
    Rgba, ScrollDelta, ScrollWheelEvent, ShapedLine, SharedString, TextRun, Window, WindowBounds,
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
        LiveTerminalSnapshotNotifier, TerminalGridSnapshot, TerminalReplayStep, TerminalResize,
        TerminalRgb, replay_terminal_bytes, replay_terminal_steps, spawn_live_terminal,
        spawn_live_terminal_with_notifier,
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
    terminal_glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
    terminal_render_cache: Rc<RefCell<TerminalRenderCache>>,
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
    dirty_rows: VecDeque<u64>,
    dirty_cells: VecDeque<u64>,
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
            terminal_glyph_cache: Rc::new(RefCell::new(TerminalGlyphLayoutCache::default())),
            terminal_render_cache: Rc::new(RefCell::new(TerminalRenderCache::default())),
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
            if let Some(snapshot) = coalesce_terminal_snapshots(live.handle.drain_snapshots()) {
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
                live.latency.record_dirty_rows(snapshot.timing.dirty_rows);
                live.latency.record_dirty_cells(snapshot.timing.dirty_cells);
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

    fn record_dirty_rows(&mut self, rows: u32) {
        push_latency_sample(&mut self.dirty_rows, u64::from(rows));
    }

    fn record_dirty_cells(&mut self, cells: u32) {
        push_latency_sample(&mut self.dirty_cells, u64::from(cells));
    }

    fn summary_label(&self) -> Option<String> {
        let key = latency_summary(&self.key_to_snapshot_micros)?;
        let pty = latency_summary(&self.pty_to_snapshot_micros);
        let output_bytes = count_summary(&self.pty_output_bytes);
        let vt = latency_summary(&self.vt_write_micros);
        let update = latency_summary(&self.snapshot_update_micros);
        let extract = latency_summary(&self.snapshot_extract_micros);
        let build = latency_summary(&self.snapshot_build_micros);
        let dirty_rows = count_summary(&self.dirty_rows);
        let dirty_cells = count_summary(&self.dirty_cells);
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
        if let Some(dirty_rows) = dirty_rows {
            parts.push(format!("dirty rows {dirty_rows}"));
        }
        if let Some(dirty_cells) = dirty_cells {
            parts.push(format!("dirty cells {dirty_cells}"));
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

fn coalesce_terminal_snapshots(
    snapshots: Vec<TerminalGridSnapshot>,
) -> Option<TerminalGridSnapshot> {
    let mut snapshots = snapshots.into_iter();
    let mut latest = snapshots.next()?;
    let mut dirty_rows: BTreeSet<u16> = latest.damage.rows.iter().copied().collect();
    let mut force_full = latest.damage.full;
    let mut previous_session_id = latest.session_id.clone();
    let mut previous_size = (latest.cols, latest.rows);

    for snapshot in snapshots {
        if snapshot.damage.full
            || snapshot.session_id != previous_session_id
            || (snapshot.cols, snapshot.rows) != previous_size
        {
            force_full = true;
        }
        dirty_rows.extend(snapshot.damage.rows.iter().copied());
        previous_session_id = snapshot.session_id.clone();
        previous_size = (snapshot.cols, snapshot.rows);
        latest = snapshot;
    }

    if force_full {
        latest.damage.rows = (0..latest.rows).collect();
        latest.damage.full = true;
    } else {
        latest.damage.rows = dirty_rows
            .into_iter()
            .filter(|row| *row < latest.rows)
            .collect();
        latest.damage.full = latest.damage.rows.len() == usize::from(latest.rows);
    }

    latest.damage.cells = latest
        .damage
        .rows
        .len()
        .saturating_mul(usize::from(latest.cols)) as u32;
    latest.timing.dirty_rows = latest.damage.rows.len() as u32;
    latest.timing.dirty_cells = latest.damage.cells;
    Some(latest)
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
            self.terminal_glyph_cache.clone(),
            self.terminal_render_cache.clone(),
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
    if let Some((events_path, output_path)) = terminal_replay_events_args() {
        let summary =
            terminal_replay_events_check(events_path, output_path).expect("replay terminal events");
        println!("{summary}");
        return;
    }
    if let Some((path, cols, rows)) = terminal_replay_record_args() {
        let summary =
            terminal_replay_record_check(path, cols, rows).expect("replay terminal record");
        println!("{summary}");
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

fn terminal_replay_record_args() -> Option<(PathBuf, u16, u16)> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--terminal-replay-record" {
            let path = args.next().map(PathBuf::from).unwrap_or_else(|| {
                eprintln!("--terminal-replay-record requires a .pty path");
                std::process::exit(2);
            });
            let cols = args
                .next()
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(120);
            let rows = args
                .next()
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(40);
            return Some((path, cols, rows));
        }
    }
    None
}

fn terminal_replay_events_args() -> Option<(PathBuf, Option<PathBuf>)> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--terminal-replay-events" {
            let events_path = args.next().map(PathBuf::from).unwrap_or_else(|| {
                eprintln!("--terminal-replay-events requires a .events path");
                std::process::exit(2);
            });
            return Some((events_path, args.next().map(PathBuf::from)));
        }
    }
    None
}

fn terminal_replay_record_check(path: PathBuf, cols: u16, rows: u16) -> anyhow::Result<String> {
    let bytes = std::fs::read(&path)?;
    let snapshot = replay_terminal_bytes("terminal-replay", &bytes, cols, rows)?;
    Ok(terminal_replay_summary(
        "octty-rs terminal replay ok",
        &path,
        bytes.len(),
        &snapshot,
    ))
}

#[derive(Debug, PartialEq, Eq)]
struct TerminalReplayEventsPlan {
    output_path: PathBuf,
    initial_cols: u16,
    initial_rows: u16,
    steps: Vec<TerminalReplayEventsStep>,
}

#[derive(Debug, PartialEq, Eq)]
enum TerminalReplayEventsStep {
    Output { offset: usize, len: usize },
    Resize { cols: u16, rows: u16 },
}

fn terminal_replay_events_check(
    events_path: PathBuf,
    output_path_override: Option<PathBuf>,
) -> anyhow::Result<String> {
    let events = std::fs::read_to_string(&events_path)?;
    let mut plan = parse_terminal_replay_events(&events)?;
    if let Some(output_path) = output_path_override {
        plan.output_path = output_path;
    }
    let bytes = std::fs::read(&plan.output_path)?;
    let mut steps = Vec::with_capacity(plan.steps.len());
    for step in &plan.steps {
        match *step {
            TerminalReplayEventsStep::Resize { cols, rows } => {
                steps.push(TerminalReplayStep::Resize { cols, rows });
            }
            TerminalReplayEventsStep::Output { offset, len } => {
                let end = offset
                    .checked_add(len)
                    .ok_or_else(|| anyhow::anyhow!("output offset overflow at {offset}+{len}"))?;
                let chunk = bytes.get(offset..end).ok_or_else(|| {
                    anyhow::anyhow!(
                        "output chunk {offset}..{end} is outside {} bytes",
                        bytes.len()
                    )
                })?;
                steps.push(TerminalReplayStep::Output(chunk));
            }
        }
    }
    let snapshot = replay_terminal_steps(
        "terminal-replay-events",
        plan.initial_cols,
        plan.initial_rows,
        steps,
    )?;
    Ok(terminal_replay_summary(
        "octty-rs terminal event replay ok",
        &events_path,
        bytes.len(),
        &snapshot,
    ))
}

fn parse_terminal_replay_events(events: &str) -> anyhow::Result<TerminalReplayEventsPlan> {
    let mut output_path = None;
    let mut initial_cols = None;
    let mut initial_rows = None;
    let mut steps = Vec::new();

    for line in events.lines() {
        match terminal_trace_value(line, "kind") {
            Some("start") => {
                output_path = Some(PathBuf::from(
                    terminal_trace_value(line, "output")
                        .ok_or_else(|| anyhow::anyhow!("trace start is missing output path"))?,
                ));
                initial_cols = Some(
                    terminal_trace_value(line, "cols")
                        .and_then(parse_u16)
                        .ok_or_else(|| anyhow::anyhow!("trace start is missing cols"))?,
                );
                initial_rows = Some(
                    terminal_trace_value(line, "rows")
                        .and_then(parse_u16)
                        .ok_or_else(|| anyhow::anyhow!("trace start is missing rows"))?,
                );
            }
            Some("resize") => {
                let cols = terminal_trace_value(line, "cols")
                    .and_then(parse_u16)
                    .ok_or_else(|| anyhow::anyhow!("trace resize is missing cols"))?;
                let rows = terminal_trace_value(line, "rows")
                    .and_then(parse_u16)
                    .ok_or_else(|| anyhow::anyhow!("trace resize is missing rows"))?;
                steps.push(TerminalReplayEventsStep::Resize { cols, rows });
            }
            Some("output") => {
                let offset = terminal_trace_value(line, "offset")
                    .and_then(parse_usize)
                    .ok_or_else(|| anyhow::anyhow!("trace output is missing offset"))?;
                let len = terminal_trace_value(line, "len")
                    .and_then(parse_usize)
                    .ok_or_else(|| anyhow::anyhow!("trace output is missing len"))?;
                steps.push(TerminalReplayEventsStep::Output { offset, len });
            }
            _ => {}
        }
    }

    Ok(TerminalReplayEventsPlan {
        output_path: output_path.ok_or_else(|| anyhow::anyhow!("trace is missing start event"))?,
        initial_cols: initial_cols.ok_or_else(|| anyhow::anyhow!("trace is missing start cols"))?,
        initial_rows: initial_rows.ok_or_else(|| anyhow::anyhow!("trace is missing start rows"))?,
        steps,
    })
}

fn terminal_trace_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    line.split_whitespace()
        .find_map(|part| part.strip_prefix(&prefix))
}

fn parse_u16(value: &str) -> Option<u16> {
    value.parse().ok()
}

fn parse_usize(value: &str) -> Option<usize> {
    value.parse().ok()
}

fn terminal_replay_summary(
    label: &str,
    path: &Path,
    bytes_len: usize,
    snapshot: &TerminalGridSnapshot,
) -> String {
    let cursor = snapshot
        .cursor
        .as_ref()
        .map(|cursor| format!("{},{}", cursor.col, cursor.row))
        .unwrap_or_else(|| "none".to_owned());
    format!(
        "{label}: path={} bytes={} grid={}x{} cursor={} dirty_rows={} dirty_cells={}\n{}\n{}",
        path.display(),
        bytes_len,
        snapshot.cols,
        snapshot.rows,
        cursor,
        snapshot.damage.rows.len(),
        snapshot.damage.cells,
        snapshot.plain_text,
        terminal_replay_style_summary(snapshot)
    )
}

fn terminal_replay_style_summary(snapshot: &TerminalGridSnapshot) -> String {
    let mut lines = Vec::new();
    for (row_index, row) in snapshot.rows_data.iter().enumerate() {
        let bg_runs = terminal_replay_bg_runs(row, snapshot.default_bg);
        if bg_runs.len() <= 1
            && bg_runs
                .first()
                .is_none_or(|run| run.color == snapshot.default_bg)
        {
            continue;
        }

        let text = row
            .cells
            .iter()
            .map(|cell| {
                if cell.text.is_empty() {
                    " "
                } else {
                    cell.text.as_str()
                }
            })
            .collect::<String>()
            .trim_end()
            .to_owned();
        lines.push(format!(
            "style row {:02}: bg={} text={}",
            row_index,
            bg_runs
                .iter()
                .map(|run| format!(
                    "{}:{}-{}",
                    terminal_rgb_hex(run.color),
                    run.start_col,
                    run.end_col
                ))
                .collect::<Vec<_>>()
                .join(","),
            text
        ));
    }

    if lines.is_empty() {
        "style rows: none".to_owned()
    } else {
        format!("style rows:\n{}", lines.join("\n"))
    }
}

#[derive(Debug, PartialEq, Eq)]
struct TerminalReplayBgRun {
    color: TerminalRgb,
    start_col: usize,
    end_col: usize,
}

fn terminal_replay_bg_runs(
    row: &octty_term::live::TerminalRowSnapshot,
    default_bg: TerminalRgb,
) -> Vec<TerminalReplayBgRun> {
    let mut runs = Vec::new();
    let mut active: Option<TerminalReplayBgRun> = None;
    for (col, cell) in row.cells.iter().enumerate() {
        let color = cell.bg.unwrap_or(default_bg);
        match active.as_mut() {
            Some(run) if run.color == color => run.end_col = col + 1,
            Some(_) => {
                runs.push(active.take().expect("checked"));
                active = Some(TerminalReplayBgRun {
                    color,
                    start_col: col,
                    end_col: col + 1,
                });
            }
            None => {
                active = Some(TerminalReplayBgRun {
                    color,
                    start_col: col,
                    end_col: col + 1,
                });
            }
        }
    }
    if let Some(run) = active {
        runs.push(run);
    }
    runs
}

fn terminal_rgb_hex(color: TerminalRgb) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
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
    let mut text = None;
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
            let key_text = terminal_printable_key_text(key_char, control, platform);
            if let Some(key_text) = key_text
                && let Some(character) = key_text.chars().next()
            {
                text = Some(key_text);
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

fn terminal_printable_key_text(
    key_char: Option<&str>,
    control: bool,
    platform: bool,
) -> Option<String> {
    if control || platform {
        return None;
    }
    let text = key_char?;
    if text.is_empty()
        || text == "\r"
        || text == "\n"
        || text.chars().any(|character| character.is_control())
    {
        return None;
    }
    Some(text.to_owned())
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
    terminal_glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
    terminal_render_cache: Rc<RefCell<TerminalRenderCache>>,
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
                    terminal_glyph_cache.clone(),
                    terminal_render_cache.clone(),
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
    terminal_glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
    terminal_render_cache: Rc<RefCell<TerminalRenderCache>>,
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

    pane_el.child(render_pane_body(
        pane,
        active,
        terminal_live,
        terminal_glyph_cache,
        terminal_render_cache,
        cx,
    ))
}

fn render_pane_body(
    pane: &PaneState,
    active: bool,
    terminal_live: Option<&LiveTerminalPane>,
    terminal_glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
    terminal_render_cache: Rc<RefCell<TerminalRenderCache>>,
    cx: &mut Context<OcttyApp>,
) -> gpui::Div {
    match &pane.payload {
        PanePayload::Terminal(payload) => render_terminal_surface(
            payload,
            active,
            terminal_live,
            terminal_glyph_cache,
            terminal_render_cache,
            cx,
        ),
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
    terminal_glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
    terminal_render_cache: Rc<RefCell<TerminalRenderCache>>,
    cx: &mut Context<OcttyApp>,
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

    surface.child(render_terminal_grid(
        snapshot,
        default_fg,
        default_bg,
        terminal_glyph_cache,
        terminal_render_cache,
        cx,
    ))
}

struct TerminalGridPaintInput {
    session_id: String,
    cols: u16,
    rows: u16,
    default_bg: Rgba,
    rows_data: Vec<TerminalPaintRowInput>,
    glyph_cells: Vec<TerminalPaintGlyphCell>,
    dirty_rows: usize,
    dirty_cells: usize,
    rebuilt_rows: usize,
    reused_rows: usize,
    rebuilt_row_flags: Vec<bool>,
}

#[derive(Clone)]
struct TerminalPaintRowInput {
    default_bg: Rgba,
    background_runs: Vec<TerminalPaintBackgroundRun>,
}

#[derive(Clone)]
struct TerminalPaintGlyphCell {
    row_index: usize,
    col_index: usize,
    text: SharedString,
    cell_width: u8,
    color: Hsla,
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    overline: bool,
}

#[derive(Clone)]
struct TerminalPaintBackgroundRun {
    start_col: usize,
    cell_count: usize,
    color: Rgba,
}

struct TerminalRowPaintSurface {
    row_input: TerminalPaintRowInput,
    glyph_cells: Vec<TerminalPaintGlyphCell>,
    shaped_glyph_cells: Vec<TerminalShapedGlyphCell>,
}

struct TerminalFullPaintSurface {
    input: TerminalGridPaintInput,
    shaped_glyph_cells: Vec<TerminalShapedGlyphCell>,
    glyph_cache_hits: usize,
    glyph_cache_misses: usize,
}

struct TerminalShapedGlyphCell {
    input_cell_index: usize,
    line: ShapedLine,
}

#[derive(Default)]
struct TerminalGlyphLayoutCache {
    glyphs: HashMap<TerminalGlyphCacheKey, ShapedLine>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TerminalGlyphCacheKey {
    text: String,
    bold: bool,
    italic: bool,
    strikethrough: bool,
}

#[derive(Default)]
struct TerminalRenderCache {
    sessions: HashMap<String, TerminalRenderGridCache>,
}

struct TerminalRenderGridCache {
    cols: u16,
    rows: u16,
    default_fg: Rgba,
    default_bg: Rgba,
    rows_data: Vec<Option<TerminalCachedPaintRow>>,
    row_views: Vec<Option<Entity<TerminalRowView>>>,
}

#[derive(Clone)]
struct TerminalCachedPaintRow {
    row_input: TerminalPaintRowInput,
    glyph_cells: Vec<TerminalPaintGlyphCell>,
}

struct TerminalRowView {
    cols: u16,
    row_input: TerminalPaintRowInput,
    glyph_cells: Vec<TerminalPaintGlyphCell>,
    glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
}

#[derive(Clone, Copy, Debug, Default)]
struct TerminalRenderProfileSample {
    build_micros: u64,
    shape_micros: u64,
    paint_micros: u64,
    rows: u16,
    cols: u16,
    glyph_cells: usize,
    glyph_cache_hits: usize,
    glyph_cache_misses: usize,
    background_runs: usize,
    text_bytes: usize,
    dirty_rows: usize,
    dirty_cells: usize,
    rebuilt_rows: usize,
    reused_rows: usize,
}

#[derive(Default)]
struct TerminalRenderProfiler {
    build_micros: VecDeque<u64>,
    shape_micros: VecDeque<u64>,
    paint_micros: VecDeque<u64>,
    glyph_cells: VecDeque<u64>,
    glyph_cache_hits: VecDeque<u64>,
    glyph_cache_misses: VecDeque<u64>,
    background_runs: VecDeque<u64>,
    text_bytes: VecDeque<u64>,
    dirty_rows: VecDeque<u64>,
    dirty_cells: VecDeque<u64>,
    rebuilt_rows: VecDeque<u64>,
    reused_rows: VecDeque<u64>,
    last_report_at: Option<Instant>,
}

fn render_terminal_grid(
    snapshot: &TerminalGridSnapshot,
    default_fg: Rgba,
    default_bg: Rgba,
    terminal_glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
    terminal_render_cache: Rc<RefCell<TerminalRenderCache>>,
    cx: &mut Context<OcttyApp>,
) -> gpui::Div {
    let build_started_at = Instant::now();
    let input = terminal_paint_input(
        snapshot,
        default_fg,
        default_bg,
        &mut terminal_render_cache.borrow_mut(),
    );
    let build_micros = duration_micros(build_started_at.elapsed());
    let width = TERMINAL_CELL_WIDTH * input.cols as f32;
    let height = TERMINAL_CELL_HEIGHT * input.rows as f32;

    if terminal_prefers_full_canvas(&input) {
        clear_terminal_row_views(&input.session_id, &mut terminal_render_cache.borrow_mut());
        return render_terminal_full_canvas(
            input,
            terminal_glyph_cache,
            build_micros,
            width,
            height,
        );
    }

    let row_views = {
        let mut render_cache = terminal_render_cache.borrow_mut();
        terminal_row_views_for_input(&input, terminal_glyph_cache, &mut render_cache, cx)
    };
    record_terminal_render_build_profile(&input, build_micros);

    div()
        .flex()
        .flex_col()
        .w(px(width))
        .h(px(height))
        .overflow_hidden()
        .children(row_views)
}

fn terminal_paint_input(
    snapshot: &TerminalGridSnapshot,
    default_fg: Rgba,
    default_bg: Rgba,
    render_cache: &mut TerminalRenderCache,
) -> TerminalGridPaintInput {
    let mut rows_data = Vec::with_capacity(snapshot.rows_data.len());
    let mut glyph_cells = Vec::new();
    let cache = render_cache
        .sessions
        .entry(snapshot.session_id.clone())
        .or_insert_with(|| TerminalRenderGridCache {
            cols: snapshot.cols,
            rows: snapshot.rows,
            default_fg,
            default_bg,
            rows_data: Vec::new(),
            row_views: Vec::new(),
        });
    let cache_invalid = cache.cols != snapshot.cols
        || cache.rows != snapshot.rows
        || cache.default_fg != default_fg
        || cache.default_bg != default_bg
        || cache.rows_data.len() != snapshot.rows_data.len();
    if cache_invalid {
        cache.cols = snapshot.cols;
        cache.rows = snapshot.rows;
        cache.default_fg = default_fg;
        cache.default_bg = default_bg;
        cache.rows_data = vec![None; snapshot.rows_data.len()];
        cache.row_views = vec![None; snapshot.rows_data.len()];
    }

    let mut dirty_row_flags = vec![cache_invalid || snapshot.damage.full; snapshot.rows_data.len()];
    for row in snapshot.damage.rows.iter().copied() {
        if let Some(flag) = dirty_row_flags.get_mut(row as usize) {
            *flag = true;
        }
    }

    let mut rebuilt_rows = 0usize;
    let mut reused_rows = 0usize;

    for (row_index, row) in snapshot.rows_data.iter().enumerate() {
        let rebuild_row = dirty_row_flags[row_index] || cache.rows_data[row_index].is_none();
        let cached_row = if rebuild_row {
            rebuilt_rows += 1;
            let cached_row = terminal_cached_paint_row(row_index, row, snapshot, default_bg);
            cache.rows_data[row_index] = Some(cached_row.clone());
            cached_row
        } else {
            reused_rows += 1;
            cache.rows_data[row_index]
                .as_ref()
                .expect("checked above")
                .clone()
        };

        glyph_cells.extend(cached_row.glyph_cells.iter().cloned());
        rows_data.push(cached_row.row_input);
    }

    TerminalGridPaintInput {
        session_id: snapshot.session_id.clone(),
        cols: snapshot.cols,
        rows: snapshot.rows,
        default_bg,
        rows_data,
        glyph_cells,
        dirty_rows: snapshot.damage.rows.len(),
        dirty_cells: snapshot.damage.cells as usize,
        rebuilt_rows,
        reused_rows,
        rebuilt_row_flags: dirty_row_flags,
    }
}

fn terminal_prefers_full_canvas(input: &TerminalGridPaintInput) -> bool {
    // Keep one stable GPUI tree for the terminal. Switching between row views and
    // a monolithic canvas during dense TUI redraws caused stale pixels to be
    // composited into unrelated rows.
    let _ = input;
    false
}

fn clear_terminal_row_views(session_id: &str, render_cache: &mut TerminalRenderCache) {
    if let Some(cache) = render_cache.sessions.get_mut(session_id) {
        cache.row_views.fill_with(|| None);
    }
}

fn render_terminal_full_canvas(
    input: TerminalGridPaintInput,
    terminal_glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
    build_micros: u64,
    width: f32,
    height: f32,
) -> gpui::Div {
    div().w(px(width)).h(px(height)).overflow_hidden().child(
        canvas(
            move |_bounds, window, _cx| {
                let mut glyph_cache = terminal_glyph_cache.borrow_mut();
                let mut glyph_cache_hits = 0usize;
                let mut glyph_cache_misses = 0usize;
                let shaped_glyph_cells = shape_terminal_glyph_cells(
                    &input.glyph_cells,
                    &mut glyph_cache,
                    &mut glyph_cache_hits,
                    &mut glyph_cache_misses,
                    window,
                );
                TerminalFullPaintSurface {
                    input,
                    shaped_glyph_cells,
                    glyph_cache_hits,
                    glyph_cache_misses,
                }
            },
            move |bounds, surface, window, cx| {
                let sample = terminal_full_render_profile_sample(&surface, build_micros);
                let paint_started_at = Instant::now();
                paint_terminal_full_surface(bounds, surface, window, cx);
                let mut sample = sample;
                sample.paint_micros = duration_micros(paint_started_at.elapsed());
                record_terminal_render_profile(sample);
            },
        )
        .w(px(width))
        .h(px(height))
        .overflow_hidden(),
    )
}

fn shape_terminal_glyph_cells(
    glyph_cells: &[TerminalPaintGlyphCell],
    glyph_cache: &mut TerminalGlyphLayoutCache,
    glyph_cache_hits: &mut usize,
    glyph_cache_misses: &mut usize,
    window: &mut Window,
) -> Vec<TerminalShapedGlyphCell> {
    glyph_cells
        .iter()
        .enumerate()
        .map(|(input_cell_index, cell)| {
            let key = TerminalGlyphCacheKey {
                text: cell.text.to_string(),
                bold: cell.bold,
                italic: cell.italic,
                strikethrough: cell.strikethrough,
            };
            if let Some(line) = glyph_cache.glyphs.get(&key) {
                *glyph_cache_hits += 1;
                return TerminalShapedGlyphCell {
                    input_cell_index,
                    line: line.clone(),
                };
            }

            let text = cell.text.clone();
            let style =
                terminal_glyph_shape_style(text.len(), cell.bold, cell.italic, cell.strikethrough);
            let line = window.text_system().shape_line(
                text,
                px(TERMINAL_FONT_SIZE),
                std::slice::from_ref(&style),
                Some(px(TERMINAL_CELL_WIDTH)),
            );
            glyph_cache.glyphs.insert(key, line.clone());
            *glyph_cache_misses += 1;
            TerminalShapedGlyphCell {
                input_cell_index,
                line,
            }
        })
        .collect()
}

fn terminal_cached_paint_row(
    row_index: usize,
    row: &octty_term::live::TerminalRowSnapshot,
    snapshot: &TerminalGridSnapshot,
    default_bg: Rgba,
) -> TerminalCachedPaintRow {
    let background_runs = terminal_background_runs(row_index as u16, row, snapshot, default_bg);
    let mut glyph_cells = Vec::new();

    for (col_index, cell) in row.cells.iter().enumerate() {
        if cell.width > 0 && !cell.invisible && !cell.text.is_empty() && cell.text != " " {
            let (fg, _) =
                terminal_effective_cell_colors(row_index as u16, col_index as u16, cell, snapshot);
            glyph_cells.push(TerminalPaintGlyphCell {
                row_index,
                col_index,
                text: SharedString::from(cell.text.clone()),
                color: Hsla::from(fg),
                bold: cell.bold,
                italic: cell.italic,
                underline: cell.underline,
                strikethrough: cell.strikethrough,
                overline: cell.overline,
                cell_width: cell.width,
            });
        }
    }

    TerminalCachedPaintRow {
        row_input: TerminalPaintRowInput {
            default_bg,
            background_runs,
        },
        glyph_cells,
    }
}

fn terminal_row_views_for_input(
    input: &TerminalGridPaintInput,
    glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
    render_cache: &mut TerminalRenderCache,
    cx: &mut Context<OcttyApp>,
) -> Vec<AnyView> {
    let Some(cache) = render_cache.sessions.get_mut(&input.session_id) else {
        return Vec::new();
    };
    if cache.row_views.len() != input.rows_data.len() {
        cache.row_views = vec![None; input.rows_data.len()];
    }

    let mut glyph_cells_by_row = vec![Vec::new(); input.rows_data.len()];
    for cell in input.glyph_cells.iter().cloned() {
        if let Some(row_cells) = glyph_cells_by_row.get_mut(cell.row_index) {
            row_cells.push(cell);
        }
    }

    let mut views = Vec::with_capacity(input.rows_data.len());
    for (row_index, row_input) in input.rows_data.iter().cloned().enumerate() {
        let glyph_cells = glyph_cells_by_row[row_index].clone();
        let view = if let Some(view) = cache.row_views[row_index].as_ref() {
            if input
                .rebuilt_row_flags
                .get(row_index)
                .copied()
                .unwrap_or(true)
            {
                let _ = view.update(cx, |view, cx| {
                    view.cols = input.cols;
                    view.row_input = row_input;
                    view.glyph_cells = glyph_cells;
                    view.glyph_cache = glyph_cache.clone();
                    cx.notify();
                });
            }
            view.clone()
        } else {
            let view = cx.new(|_| TerminalRowView {
                cols: input.cols,
                row_input,
                glyph_cells,
                glyph_cache: glyph_cache.clone(),
            });
            cache.row_views[row_index] = Some(view.clone());
            view
        };

        let any_view: AnyView = view.into();
        views.push(any_view);
    }

    views
}

impl Render for TerminalRowView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let cols = self.cols;
        let row_input = self.row_input.clone();
        let glyph_cells = self.glyph_cells.clone();
        let glyph_cache = self.glyph_cache.clone();
        let width = TERMINAL_CELL_WIDTH * cols as f32;

        canvas(
            move |_bounds, window, _cx| {
                let mut cache = glyph_cache.borrow_mut();
                let mut glyph_cache_hits = 0usize;
                let mut glyph_cache_misses = 0usize;
                let shaped_glyph_cells = shape_terminal_glyph_cells(
                    &glyph_cells,
                    &mut cache,
                    &mut glyph_cache_hits,
                    &mut glyph_cache_misses,
                    window,
                );
                TerminalRowPaintSurface {
                    row_input,
                    glyph_cells,
                    shaped_glyph_cells,
                }
            },
            move |bounds, surface, window, cx| {
                let paint_started_at = Instant::now();
                paint_terminal_row_surface(bounds, surface, window, cx);
                let _paint_micros = duration_micros(paint_started_at.elapsed());
            },
        )
        .w(px(width))
        .h(px(TERMINAL_CELL_HEIGHT))
        .overflow_hidden()
    }
}

fn terminal_glyph_shape_style(
    len: usize,
    bold: bool,
    italic: bool,
    strikethrough: bool,
) -> TextRun {
    let mut font = terminal_font();
    if bold {
        font = font.bold();
    }
    if italic {
        font = font.italic();
    }
    TextRun {
        len,
        font,
        color: Hsla::from(rgb(0xffffff)),
        background_color: None,
        underline: None,
        strikethrough: strikethrough.then_some(Default::default()),
    }
}

fn paint_terminal_row_surface(
    bounds: Bounds<gpui::Pixels>,
    surface: TerminalRowPaintSurface,
    window: &mut Window,
    cx: &mut App,
) {
    window.paint_quad(fill(bounds, surface.row_input.default_bg));

    for run in &surface.row_input.background_runs {
        let origin = point(
            bounds.origin.x + px(run.start_col as f32 * TERMINAL_CELL_WIDTH),
            bounds.origin.y,
        );
        let size = size(
            px(run.cell_count as f32 * TERMINAL_CELL_WIDTH),
            px(TERMINAL_CELL_HEIGHT),
        );
        window.paint_quad(fill(Bounds { origin, size }, run.color));
    }

    for shaped_cell in surface.shaped_glyph_cells.iter() {
        let cell = &surface.glyph_cells[shaped_cell.input_cell_index];
        let origin = point(
            bounds.origin.x + px(cell.col_index as f32 * TERMINAL_CELL_WIDTH),
            bounds.origin.y,
        );
        let _ = paint_terminal_glyph_cell(origin, cell, &shaped_cell.line, window, cx);
    }
}

fn paint_terminal_full_surface(
    bounds: Bounds<gpui::Pixels>,
    surface: TerminalFullPaintSurface,
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

    for shaped_cell in surface.shaped_glyph_cells.iter() {
        let cell = &surface.input.glyph_cells[shaped_cell.input_cell_index];
        let origin = point(
            bounds.origin.x + px(cell.col_index as f32 * TERMINAL_CELL_WIDTH),
            bounds.origin.y + px(cell.row_index as f32 * TERMINAL_CELL_HEIGHT),
        );
        let _ = paint_terminal_glyph_cell(origin, cell, &shaped_cell.line, window, cx);
    }
}

fn paint_terminal_glyph_cell(
    origin: gpui::Point<gpui::Pixels>,
    cell: &TerminalPaintGlyphCell,
    line: &ShapedLine,
    window: &mut Window,
    _cx: &mut App,
) -> gpui::Result<()> {
    let cell_bounds = terminal_glyph_cell_bounds(origin, cell);
    window.with_content_mask(
        Some(gpui::ContentMask {
            bounds: cell_bounds,
        }),
        |window| {
            let padding_top = (px(TERMINAL_CELL_HEIGHT) - line.ascent - line.descent) / 2.0;
            let baseline_offset = point(px(0.0), padding_top + line.ascent);
            let mut glyph_origin = origin;
            let mut prev_glyph_position = gpui::Point::default();

            for run in &line.runs {
                for glyph in &run.glyphs {
                    glyph_origin.x += glyph.position.x - prev_glyph_position.x;
                    prev_glyph_position = glyph.position;

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
                            cell.color,
                        )?;
                    }
                }
            }
            paint_terminal_cell_decorations(origin, cell, window);
            Ok(())
        },
    )
}

fn terminal_glyph_cell_bounds(
    origin: gpui::Point<gpui::Pixels>,
    cell: &TerminalPaintGlyphCell,
) -> Bounds<gpui::Pixels> {
    Bounds {
        origin,
        size: size(
            px(TERMINAL_CELL_WIDTH * f32::from(cell.cell_width.max(1))),
            px(TERMINAL_CELL_HEIGHT),
        ),
    }
}

fn paint_terminal_cell_decorations(
    origin: gpui::Point<gpui::Pixels>,
    cell: &TerminalPaintGlyphCell,
    window: &mut Window,
) {
    let width = px(TERMINAL_CELL_WIDTH * f32::from(cell.cell_width.max(1)));
    let thickness = px(1.0);
    if cell.overline {
        window.paint_quad(fill(
            Bounds {
                origin,
                size: size(width, thickness),
            },
            cell.color,
        ));
    }
    if cell.strikethrough {
        window.paint_quad(fill(
            Bounds {
                origin: point(origin.x, origin.y + px(TERMINAL_CELL_HEIGHT * 0.5)),
                size: size(width, thickness),
            },
            cell.color,
        ));
    }
    if cell.underline {
        window.paint_quad(fill(
            Bounds {
                origin: point(origin.x, origin.y + px(TERMINAL_CELL_HEIGHT - 2.0)),
                size: size(width, thickness),
            },
            cell.color,
        ));
    }
}

fn terminal_background_runs(
    row_index: u16,
    row: &octty_term::live::TerminalRowSnapshot,
    snapshot: &TerminalGridSnapshot,
    default_bg: Rgba,
) -> Vec<TerminalPaintBackgroundRun> {
    let mut runs = Vec::new();
    let mut active: Option<TerminalPaintBackgroundRun> = None;

    for (col, cell) in row.cells.iter().enumerate() {
        let (_, bg) = terminal_effective_cell_colors(row_index, col as u16, cell, snapshot);
        let bg = (bg != default_bg).then_some(bg);

        match (&mut active, bg) {
            (Some(run), Some(bg)) if run.color == bg && run.start_col + run.cell_count == col => {
                run.cell_count += 1;
            }
            (Some(_), Some(bg)) => {
                runs.push(active.take().expect("checked above"));
                active = Some(TerminalPaintBackgroundRun {
                    start_col: col,
                    cell_count: 1,
                    color: bg,
                });
            }
            (None, Some(bg)) => {
                active = Some(TerminalPaintBackgroundRun {
                    start_col: col,
                    cell_count: 1,
                    color: bg,
                });
            }
            (Some(_), None) => {
                runs.push(active.take().expect("checked above"));
            }
            (None, None) => {}
        }
    }

    if let Some(run) = active {
        runs.push(run);
    }

    runs
}

fn terminal_effective_cell_colors(
    row_index: u16,
    col: u16,
    cell: &octty_term::live::TerminalCellSnapshot,
    snapshot: &TerminalGridSnapshot,
) -> (Rgba, Rgba) {
    let is_cursor = snapshot
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.visible && cursor.row == row_index && cursor.col == col);
    let mut fg = cell
        .fg
        .map(terminal_rgb_to_rgba)
        .unwrap_or_else(|| terminal_rgb_to_rgba(snapshot.default_fg));
    let mut bg = cell
        .bg
        .map(terminal_rgb_to_rgba)
        .unwrap_or_else(|| terminal_rgb_to_rgba(snapshot.default_bg));
    if cell.inverse {
        std::mem::swap(&mut fg, &mut bg);
    }
    if is_cursor {
        fg = terminal_rgb_to_rgba(snapshot.default_bg);
        bg = terminal_rgb_to_rgba(snapshot.default_fg);
    } else if cell.faint {
        fg = terminal_dim_color(fg, bg);
    }
    (fg, bg)
}

fn record_terminal_render_build_profile(input: &TerminalGridPaintInput, build_micros: u64) {
    let sample = TerminalRenderProfileSample {
        build_micros,
        rows: input.rows,
        cols: input.cols,
        glyph_cells: input.glyph_cells.len(),
        background_runs: input
            .rows_data
            .iter()
            .map(|row| row.background_runs.len())
            .sum(),
        text_bytes: input.glyph_cells.iter().map(|cell| cell.text.len()).sum(),
        dirty_rows: input.dirty_rows,
        dirty_cells: input.dirty_cells,
        rebuilt_rows: input.rebuilt_rows,
        reused_rows: input.reused_rows,
        ..TerminalRenderProfileSample::default()
    };
    record_terminal_render_profile(sample);
}

fn terminal_full_render_profile_sample(
    surface: &TerminalFullPaintSurface,
    build_micros: u64,
) -> TerminalRenderProfileSample {
    TerminalRenderProfileSample {
        build_micros,
        rows: surface.input.rows,
        cols: surface.input.cols,
        glyph_cells: surface.input.glyph_cells.len(),
        glyph_cache_hits: surface.glyph_cache_hits,
        glyph_cache_misses: surface.glyph_cache_misses,
        background_runs: surface
            .input
            .rows_data
            .iter()
            .map(|row| row.background_runs.len())
            .sum(),
        text_bytes: surface
            .input
            .glyph_cells
            .iter()
            .map(|cell| cell.text.len())
            .sum(),
        dirty_rows: surface.input.dirty_rows,
        dirty_cells: surface.input.dirty_cells,
        rebuilt_rows: surface.input.rebuilt_rows,
        reused_rows: surface.input.reused_rows,
        ..TerminalRenderProfileSample::default()
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
        if sample.shape_micros > 0 {
            push_latency_sample(&mut self.shape_micros, sample.shape_micros);
        }
        if sample.paint_micros > 0 {
            push_latency_sample(&mut self.paint_micros, sample.paint_micros);
        }
        push_latency_sample(&mut self.glyph_cells, sample.glyph_cells as u64);
        push_latency_sample(&mut self.glyph_cache_hits, sample.glyph_cache_hits as u64);
        push_latency_sample(
            &mut self.glyph_cache_misses,
            sample.glyph_cache_misses as u64,
        );
        push_latency_sample(&mut self.background_runs, sample.background_runs as u64);
        push_latency_sample(&mut self.text_bytes, sample.text_bytes as u64);
        push_latency_sample(&mut self.dirty_rows, sample.dirty_rows as u64);
        push_latency_sample(&mut self.dirty_cells, sample.dirty_cells as u64);
        push_latency_sample(&mut self.rebuilt_rows, sample.rebuilt_rows as u64);
        push_latency_sample(&mut self.reused_rows, sample.reused_rows as u64);
    }

    fn summary(&self) -> Option<String> {
        let build = latency_summary(&self.build_micros)?;
        let mut parts = vec![format!("render build {build}")];
        if let Some(shape) = latency_summary(&self.shape_micros) {
            parts.push(format!("shape {shape}"));
        }
        if let Some(paint) = latency_summary(&self.paint_micros) {
            parts.push(format!("paint {paint}"));
        }
        Some(parts.join(" · "))
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
            "octty terminal render profile: {summary} · grid {}x{} · dirty rows {} · dirty cells {} · rebuilt rows {} · reused rows {} · glyph cells {} · glyph hits {} · glyph misses {} · bg runs {} · text bytes {}",
            sample.cols,
            sample.rows,
            count_summary(&self.dirty_rows).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.dirty_cells).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.rebuilt_rows).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.reused_rows).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.glyph_cells).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.glyph_cache_hits).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.glyph_cache_misses).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.background_runs).unwrap_or_else(|| "n/a".to_owned()),
            count_summary(&self.text_bytes).unwrap_or_else(|| "n/a".to_owned())
        );
    }
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

fn terminal_dim_color(color: Rgba, target: Rgba) -> Rgba {
    Rgba {
        r: (color.r + target.r) * 0.5,
        g: (color.g + target.g) * 0.5,
        b: (color.b + target.b) * 0.5,
        a: color.a,
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
    fn named_keys_do_not_forward_key_char_as_text() {
        let escape = live_terminal_input_from_key_parts(
            "escape",
            Some("\\x1b"),
            false,
            false,
            false,
            false,
            false,
        )
        .expect("escape input");
        assert_eq!(escape.key, LiveTerminalKey::Escape);
        assert_eq!(escape.text, None);

        let up = live_terminal_input_from_key_parts(
            "up",
            Some("\\x1b[A"),
            false,
            false,
            false,
            false,
            false,
        )
        .expect("up input");
        assert_eq!(up.key, LiveTerminalKey::ArrowUp);
        assert_eq!(up.text, None);
    }

    #[test]
    fn control_characters_are_not_forwarded_as_text() {
        let escape = live_terminal_input_from_key_parts(
            "escape",
            Some("\x1b"),
            false,
            false,
            false,
            false,
            false,
        )
        .expect("escape input");
        assert_eq!(escape.text, None);
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
    fn terminal_snapshot_coalesce_keeps_dirty_rows_from_skipped_snapshots() {
        let first = test_terminal_snapshot("session", 4, 3, vec![0, 1], false);
        let second = test_terminal_snapshot("session", 4, 3, vec![2], false);

        let snapshot =
            coalesce_terminal_snapshots(vec![first, second]).expect("coalesced snapshot");

        assert_eq!(snapshot.damage.rows, vec![0, 1, 2]);
        assert!(snapshot.damage.full);
        assert_eq!(snapshot.damage.cells, 12);
        assert_eq!(snapshot.timing.dirty_rows, 3);
        assert_eq!(snapshot.timing.dirty_cells, 12);
    }

    #[test]
    fn terminal_snapshot_coalesce_forces_full_damage_on_resize() {
        let first = test_terminal_snapshot("session", 4, 3, vec![1], false);
        let second = test_terminal_snapshot("session", 4, 2, vec![0], false);

        let snapshot =
            coalesce_terminal_snapshots(vec![first, second]).expect("coalesced snapshot");

        assert_eq!(snapshot.rows, 2);
        assert_eq!(snapshot.damage.rows, vec![0, 1]);
        assert!(snapshot.damage.full);
        assert_eq!(snapshot.damage.cells, 8);
    }

    #[test]
    fn terminal_replay_event_parser_keeps_resizes_and_output_order() {
        let events = "\
3 kind=start session=s cols=90 rows=20 output=/tmp/octty-record/session.pty
9 kind=resize cols=87 rows=52 pixel_width=696 pixel_height=936
10 kind=output offset=0 len=258 hex=1b5b
11 kind=input source=key len=1 hex=6e
12 kind=resize cols=87 rows=18 pixel_width=696 pixel_height=324
13 kind=output offset=258 len=224 hex=1b5b
";

        let plan = parse_terminal_replay_events(events).expect("parsed trace");

        assert_eq!(
            plan.output_path,
            PathBuf::from("/tmp/octty-record/session.pty")
        );
        assert_eq!(plan.initial_cols, 90);
        assert_eq!(plan.initial_rows, 20);
        assert_eq!(
            plan.steps,
            vec![
                TerminalReplayEventsStep::Resize { cols: 87, rows: 52 },
                TerminalReplayEventsStep::Output {
                    offset: 0,
                    len: 258
                },
                TerminalReplayEventsStep::Resize { cols: 87, rows: 18 },
                TerminalReplayEventsStep::Output {
                    offset: 258,
                    len: 224
                },
            ]
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
            damage: octty_term::live::TerminalDamageSnapshot::default(),
            rows_data: vec![octty_term::live::TerminalRowSnapshot {
                cells: vec![
                    octty_term::live::TerminalCellSnapshot {
                        text: String::new(),
                        width: 1,
                        fg: None,
                        bg: None,
                        bold: false,
                        italic: false,
                        faint: false,
                        blink: false,
                        underline: false,
                        inverse: false,
                        invisible: false,
                        strikethrough: false,
                        overline: false,
                    },
                    octty_term::live::TerminalCellSnapshot {
                        text: "a".to_owned(),
                        width: 1,
                        fg: None,
                        bg: None,
                        bold: false,
                        italic: false,
                        faint: false,
                        blink: false,
                        underline: false,
                        inverse: false,
                        invisible: false,
                        strikethrough: false,
                        overline: false,
                    },
                    octty_term::live::TerminalCellSnapshot {
                        text: String::new(),
                        width: 1,
                        fg: None,
                        bg: None,
                        bold: false,
                        italic: false,
                        faint: false,
                        blink: false,
                        underline: false,
                        inverse: false,
                        invisible: false,
                        strikethrough: false,
                        overline: false,
                    },
                ],
            }],
            plain_text: " a\n".to_owned(),
            timing: octty_term::live::TerminalSnapshotTiming::default(),
        };

        let mut render_cache = TerminalRenderCache::default();
        let input = terminal_paint_input(
            &snapshot,
            terminal_rgb_to_rgba(default_fg),
            terminal_rgb_to_rgba(default_bg),
            &mut render_cache,
        );

        assert_eq!(input.glyph_cells.len(), 1);
        assert_eq!(input.glyph_cells[0].col_index, 1);
        assert_eq!(input.glyph_cells[0].text.as_ref(), "a");
        assert!(input.rows_data[0].background_runs.is_empty());
    }

    #[test]
    fn terminal_background_runs_ignore_foreground_style_splits() {
        let default_fg = TerminalRgb {
            r: 200,
            g: 200,
            b: 200,
        };
        let default_bg = TerminalRgb { r: 0, g: 0, b: 0 };
        let highlighted_bg = TerminalRgb {
            r: 20,
            g: 60,
            b: 80,
        };
        let snapshot = TerminalGridSnapshot {
            session_id: "session-1".to_owned(),
            cols: 4,
            rows: 1,
            default_fg,
            default_bg,
            cursor: None,
            damage: octty_term::live::TerminalDamageSnapshot::default(),
            rows_data: vec![octty_term::live::TerminalRowSnapshot {
                cells: vec![
                    picker_cell(
                        "a",
                        Some(TerminalRgb { r: 255, g: 0, b: 0 }),
                        Some(highlighted_bg),
                        false,
                        false,
                    ),
                    picker_cell(
                        "b",
                        Some(TerminalRgb { r: 0, g: 255, b: 0 }),
                        Some(highlighted_bg),
                        true,
                        false,
                    ),
                    picker_cell(
                        "c",
                        Some(TerminalRgb { r: 0, g: 0, b: 255 }),
                        Some(highlighted_bg),
                        false,
                        true,
                    ),
                    picker_cell("d", None, Some(highlighted_bg), false, false),
                ],
            }],
            plain_text: "abcd\n".to_owned(),
            timing: octty_term::live::TerminalSnapshotTiming::default(),
        };

        let mut render_cache = TerminalRenderCache::default();
        let input = terminal_paint_input(
            &snapshot,
            terminal_rgb_to_rgba(default_fg),
            terminal_rgb_to_rgba(default_bg),
            &mut render_cache,
        );

        assert_eq!(input.rows_data[0].background_runs.len(), 1);
        assert_eq!(input.rows_data[0].background_runs[0].start_col, 0);
        assert_eq!(input.rows_data[0].background_runs[0].cell_count, 4);
    }

    #[test]
    fn terminal_background_runs_render_inverse_default_colors() {
        let default_fg = TerminalRgb {
            r: 200,
            g: 200,
            b: 200,
        };
        let default_bg = TerminalRgb { r: 0, g: 0, b: 0 };
        let mut inverse_cell = picker_cell("a", None, None, false, false);
        inverse_cell.inverse = true;
        let snapshot = TerminalGridSnapshot {
            session_id: "session-1".to_owned(),
            cols: 2,
            rows: 1,
            default_fg,
            default_bg,
            cursor: None,
            damage: octty_term::live::TerminalDamageSnapshot::default(),
            rows_data: vec![octty_term::live::TerminalRowSnapshot {
                cells: vec![inverse_cell, picker_cell("b", None, None, false, false)],
            }],
            plain_text: "ab\n".to_owned(),
            timing: octty_term::live::TerminalSnapshotTiming::default(),
        };

        let mut render_cache = TerminalRenderCache::default();
        let input = terminal_paint_input(
            &snapshot,
            terminal_rgb_to_rgba(default_fg),
            terminal_rgb_to_rgba(default_bg),
            &mut render_cache,
        );

        assert_eq!(input.rows_data[0].background_runs.len(), 1);
        assert_eq!(input.rows_data[0].background_runs[0].start_col, 0);
        assert_eq!(input.rows_data[0].background_runs[0].cell_count, 1);
        assert_eq!(
            input.rows_data[0].background_runs[0].color,
            terminal_rgb_to_rgba(default_fg)
        );
        assert_eq!(
            input.glyph_cells[0].color,
            Hsla::from(terminal_rgb_to_rgba(default_bg))
        );
    }

    #[test]
    fn terminal_paint_input_rebuilds_only_dirty_rows() {
        let default_fg = TerminalRgb {
            r: 200,
            g: 200,
            b: 200,
        };
        let default_bg = TerminalRgb { r: 0, g: 0, b: 0 };
        let mut snapshot = TerminalGridSnapshot {
            session_id: "session-1".to_owned(),
            cols: 2,
            rows: 2,
            default_fg,
            default_bg,
            cursor: None,
            damage: octty_term::live::TerminalDamageSnapshot {
                full: true,
                rows: vec![0, 1],
                cells: 4,
            },
            rows_data: vec![
                octty_term::live::TerminalRowSnapshot {
                    cells: vec![
                        picker_cell("a", None, None, false, false),
                        picker_cell("", None, None, false, false),
                    ],
                },
                octty_term::live::TerminalRowSnapshot {
                    cells: vec![
                        picker_cell("x", None, None, false, false),
                        picker_cell("", None, None, false, false),
                    ],
                },
            ],
            plain_text: "a\nx\n".to_owned(),
            timing: octty_term::live::TerminalSnapshotTiming::default(),
        };
        let mut render_cache = TerminalRenderCache::default();

        let first = terminal_paint_input(
            &snapshot,
            terminal_rgb_to_rgba(default_fg),
            terminal_rgb_to_rgba(default_bg),
            &mut render_cache,
        );
        assert_eq!(first.rebuilt_rows, 2);
        assert_eq!(first.reused_rows, 0);

        snapshot.damage = octty_term::live::TerminalDamageSnapshot {
            full: false,
            rows: vec![1],
            cells: 2,
        };
        snapshot.rows_data[1].cells[0].text = "b".to_owned();
        let second = terminal_paint_input(
            &snapshot,
            terminal_rgb_to_rgba(default_fg),
            terminal_rgb_to_rgba(default_bg),
            &mut render_cache,
        );

        assert_eq!(second.rebuilt_rows, 1);
        assert_eq!(second.reused_rows, 1);
        assert!(
            second
                .glyph_cells
                .iter()
                .any(|cell| cell.row_index == 0 && cell.text.as_ref() == "a")
        );
        assert!(
            second
                .glyph_cells
                .iter()
                .any(|cell| cell.row_index == 1 && cell.text.as_ref() == "b")
        );
    }

    #[test]
    fn terminal_paint_input_keeps_glyphs_on_original_cell_columns() {
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
            damage: octty_term::live::TerminalDamageSnapshot::default(),
            rows_data: vec![octty_term::live::TerminalRowSnapshot {
                cells: vec![
                    picker_cell("\u{65}\u{301}", None, None, false, false),
                    picker_cell("", None, None, false, false),
                    picker_cell("x", None, None, false, false),
                ],
            }],
            plain_text: "\u{65}\u{301} x\n".to_owned(),
            timing: octty_term::live::TerminalSnapshotTiming::default(),
        };

        let mut render_cache = TerminalRenderCache::default();
        let input = terminal_paint_input(
            &snapshot,
            terminal_rgb_to_rgba(default_fg),
            terminal_rgb_to_rgba(default_bg),
            &mut render_cache,
        );

        let glyph_columns: Vec<_> = input
            .glyph_cells
            .iter()
            .map(|cell| (cell.col_index, cell.text.to_string()))
            .collect();
        assert_eq!(
            glyph_columns,
            vec![(0, "\u{65}\u{301}".to_owned()), (2, "x".to_owned())]
        );
    }

    #[test]
    fn terminal_paint_input_preserves_wide_cell_widths() {
        let default_fg = TerminalRgb {
            r: 200,
            g: 200,
            b: 200,
        };
        let default_bg = TerminalRgb { r: 0, g: 0, b: 0 };
        let mut wide = picker_cell("表", None, None, false, false);
        wide.width = 2;
        let mut spacer = picker_cell("", None, None, false, false);
        spacer.width = 0;
        let snapshot = TerminalGridSnapshot {
            session_id: "session-1".to_owned(),
            cols: 3,
            rows: 1,
            default_fg,
            default_bg,
            cursor: None,
            damage: octty_term::live::TerminalDamageSnapshot::default(),
            rows_data: vec![octty_term::live::TerminalRowSnapshot {
                cells: vec![wide, spacer, picker_cell("x", None, None, false, false)],
            }],
            plain_text: "表 x\n".to_owned(),
            timing: octty_term::live::TerminalSnapshotTiming::default(),
        };

        let mut render_cache = TerminalRenderCache::default();
        let input = terminal_paint_input(
            &snapshot,
            terminal_rgb_to_rgba(default_fg),
            terminal_rgb_to_rgba(default_bg),
            &mut render_cache,
        );

        let glyph_cells: Vec<_> = input
            .glyph_cells
            .iter()
            .map(|cell| (cell.col_index, cell.cell_width, cell.text.to_string()))
            .collect();
        assert_eq!(
            glyph_cells,
            vec![(0, 2, "表".to_owned()), (2, 1, "x".to_owned())]
        );
    }

    #[test]
    fn terminal_paint_input_moves_highlight_for_dirty_rows() {
        let default_fg = TerminalRgb {
            r: 200,
            g: 200,
            b: 200,
        };
        let default_bg = TerminalRgb { r: 0, g: 0, b: 0 };
        let marker_bg = TerminalRgb {
            r: 30,
            g: 90,
            b: 120,
        };
        let mut snapshot = TerminalGridSnapshot {
            session_id: "session-1".to_owned(),
            cols: 4,
            rows: 2,
            default_fg,
            default_bg,
            cursor: None,
            damage: octty_term::live::TerminalDamageSnapshot {
                full: true,
                rows: vec![0, 1],
                cells: 8,
            },
            rows_data: vec![
                octty_term::live::TerminalRowSnapshot {
                    cells: vec![
                        picker_cell("a", None, Some(marker_bg), false, false),
                        picker_cell("b", None, Some(marker_bg), false, false),
                        picker_cell("c", None, Some(marker_bg), false, false),
                        picker_cell("d", None, Some(marker_bg), false, false),
                    ],
                },
                octty_term::live::TerminalRowSnapshot {
                    cells: vec![
                        picker_cell("w", None, None, false, false),
                        picker_cell("x", None, None, false, false),
                        picker_cell("y", None, None, false, false),
                        picker_cell("z", None, None, false, false),
                    ],
                },
            ],
            plain_text: "abcd\nwxyz\n".to_owned(),
            timing: octty_term::live::TerminalSnapshotTiming::default(),
        };
        let mut render_cache = TerminalRenderCache::default();

        let first = terminal_paint_input(
            &snapshot,
            terminal_rgb_to_rgba(default_fg),
            terminal_rgb_to_rgba(default_bg),
            &mut render_cache,
        );
        assert_eq!(first.rows_data[0].background_runs.len(), 1);
        assert!(first.rows_data[1].background_runs.is_empty());

        snapshot.damage = octty_term::live::TerminalDamageSnapshot {
            full: false,
            rows: vec![0, 1],
            cells: 8,
        };
        for cell in &mut snapshot.rows_data[0].cells {
            cell.bg = None;
        }
        for cell in &mut snapshot.rows_data[1].cells {
            cell.bg = Some(marker_bg);
        }
        let second = terminal_paint_input(
            &snapshot,
            terminal_rgb_to_rgba(default_fg),
            terminal_rgb_to_rgba(default_bg),
            &mut render_cache,
        );

        assert_eq!(second.rebuilt_rows, 2);
        assert!(second.rows_data[0].background_runs.is_empty());
        assert_eq!(second.rows_data[1].background_runs.len(), 1);
        assert_eq!(second.rows_data[1].background_runs[0].start_col, 0);
        assert_eq!(second.rows_data[1].background_runs[0].cell_count, 4);
    }

    #[test]
    fn terminal_picker_preview_workload_has_dense_runs_and_backgrounds() {
        let snapshot = picker_preview_snapshot(7, 120, 40);
        let mut render_cache = TerminalRenderCache::default();
        let input = terminal_paint_input(
            &snapshot,
            terminal_rgb_to_rgba(snapshot.default_fg),
            terminal_rgb_to_rgba(snapshot.default_bg),
            &mut render_cache,
        );
        let background_runs: usize = input
            .rows_data
            .iter()
            .map(|row| row.background_runs.len())
            .sum();

        assert_eq!(input.cols, 120);
        assert_eq!(input.rows, 40);
        assert!(input.glyph_cells.len() > 1_000);
        assert!(background_runs > 40);
    }

    #[test]
    #[ignore = "profiling workload; run with --ignored --nocapture"]
    fn terminal_picker_preview_paint_input_profile() {
        let mut samples = VecDeque::new();
        let mut glyph_cells = VecDeque::new();
        let mut background_runs = VecDeque::new();
        let mut rebuilt_rows = VecDeque::new();
        let mut reused_rows = VecDeque::new();
        let mut render_cache = TerminalRenderCache::default();

        for frame in 0..240 {
            let snapshot = picker_preview_snapshot(frame, 120, 40);
            let started_at = Instant::now();
            let input = terminal_paint_input(
                &snapshot,
                terminal_rgb_to_rgba(snapshot.default_fg),
                terminal_rgb_to_rgba(snapshot.default_bg),
                &mut render_cache,
            );
            push_latency_sample(&mut samples, duration_micros(started_at.elapsed()));
            push_latency_sample(&mut glyph_cells, input.glyph_cells.len() as u64);
            push_latency_sample(&mut rebuilt_rows, input.rebuilt_rows as u64);
            push_latency_sample(&mut reused_rows, input.reused_rows as u64);
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
            "picker preview paint-input: {} · glyph cells {} · rebuilt rows {} · reused rows {} · background runs {}",
            latency_summary(&samples).unwrap(),
            count_summary(&glyph_cells).unwrap(),
            count_summary(&rebuilt_rows).unwrap(),
            count_summary(&reused_rows).unwrap(),
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
            damage: octty_term::live::TerminalDamageSnapshot {
                full: true,
                rows: (0..rows).collect(),
                cells: u32::from(cols) * u32::from(rows),
            },
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
            width: 1,
            fg,
            bg,
            bold,
            italic,
            faint: false,
            blink: false,
            underline: false,
            inverse: false,
            invisible: false,
            strikethrough: false,
            overline: false,
        }
    }

    fn test_terminal_snapshot(
        session_id: &str,
        cols: u16,
        rows: u16,
        dirty_rows: Vec<u16>,
        full_damage: bool,
    ) -> TerminalGridSnapshot {
        let default_fg = TerminalRgb {
            r: 210,
            g: 216,
            b: 222,
        };
        let default_bg = TerminalRgb {
            r: 30,
            g: 34,
            b: 48,
        };
        let rows_data = (0..rows)
            .map(|_| octty_term::live::TerminalRowSnapshot {
                cells: (0..cols)
                    .map(|_| picker_cell("", None, None, false, false))
                    .collect(),
            })
            .collect();
        let damage_cells = dirty_rows.len().saturating_mul(usize::from(cols)) as u32;
        TerminalGridSnapshot {
            session_id: session_id.to_owned(),
            cols,
            rows,
            default_fg,
            default_bg,
            cursor: None,
            damage: octty_term::live::TerminalDamageSnapshot {
                full: full_damage,
                rows: dirty_rows,
                cells: damage_cells,
            },
            rows_data,
            plain_text: String::new(),
            timing: octty_term::live::TerminalSnapshotTiming {
                dirty_rows: damage_cells / u32::from(cols),
                dirty_cells: damage_cells,
                ..Default::default()
            },
        }
    }
}
