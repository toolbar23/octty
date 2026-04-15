use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    time::Duration,
};

use gpui::{
    Action, App, Application, Bounds, Context, FocusHandle, Font, FontFallbacks, FontFeatures,
    IntoElement, KeyBinding, KeyDownEvent, Menu, MenuItem, MouseButton, Render, Rgba, ScrollDelta,
    ScrollWheelEvent, SharedString, Window, WindowBounds, WindowOptions, div, font, prelude::*, px,
    rgb, size,
};
use gpui_component::Root;
use octty_core::{
    PanePayload, PaneState, PaneType, ProjectRootRecord, SessionSnapshot, SessionState,
    TerminalPanePayload, WorkspaceSnapshot, WorkspaceState, WorkspaceSummary, add_pane,
    create_default_snapshot, create_pane_state, has_recorded_workspace_path, layout::now_ms,
    workspace_shortcut_targets,
};
use octty_jj::{discover_workspaces, read_workspace_status, resolve_repo_root};
use octty_store::{TursoStore, default_store_path};
use octty_term::{
    TerminalSessionSpec, capture_tmux_pane, ensure_tmux_session, kill_tmux_session,
    live::{
        LiveTerminalHandle, LiveTerminalKey, LiveTerminalKeyInput, LiveTerminalModifiers,
        TerminalGridSnapshot, TerminalResize, TerminalRgb, spawn_live_terminal,
    },
    resize_tmux_session, send_tmux_keys, send_tmux_keys_to_session, send_tmux_text,
    send_tmux_text_to_session, stable_tmux_session_name,
};

mod gpui_tokio;

const TERMINAL_CELL_WIDTH: f32 = 8.0;
const TERMINAL_CELL_HEIGHT: f32 = 18.0;
const TERMINAL_FONT_SIZE: f32 = 14.0;
const TERMINAL_GRID_HEIGHT: f32 = 520.0;
const TERMINAL_PANE_CHROME_HEIGHT: f32 = 42.0;
const DEFAULT_TERMINAL_FONT_FAMILY: &str = "JetBrains Mono";

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
    terminal_poll_active: bool,
}

struct LiveTerminalPane {
    handle: LiveTerminalHandle,
    latest: Option<TerminalGridSnapshot>,
    last_resize: Option<(u16, u16)>,
}

impl OcttyApp {
    fn new(bootstrap: BootstrapState, focus_handle: FocusHandle) -> Self {
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
            terminal_poll_active: false,
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
        let workspace_display_name = workspace.display_name_or_workspace_name().to_owned();
        match load_workspace_snapshot_sync(&self.store_path, &workspace) {
            Ok(snapshot) => {
                self.active_snapshot = Some(snapshot);
                self.ensure_live_terminals_for_active_snapshot();
                self.schedule_terminal_poll(cx);
                self.status = format!("Opened {workspace_display_name}.").into();
            }
            Err(error) => {
                self.status =
                    format!("Failed to open {}: {error:#}", workspace.workspace_name).into();
            }
        }
        cx.notify();
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
        match save_workspace_snapshot_sync(&self.store_path, &snapshot) {
            Ok(()) => {
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
                self.active_snapshot = Some(snapshot);
                self.ensure_live_terminals_for_active_snapshot();
                self.schedule_terminal_poll(cx);
            }
            Err(error) => {
                self.status = format!("Failed to save taskspace: {error:#}").into();
                self.active_snapshot = Some(snapshot);
            }
        }
        cx.notify();
    }

    fn select_pane(&mut self, pane_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window);
        let snapshot_to_save = self.active_snapshot.as_mut().map(|snapshot| {
            snapshot.active_pane_id = Some(pane_id.to_owned());
            snapshot.updated_at = now_ms();
            snapshot.clone()
        });

        if let Some(snapshot) = snapshot_to_save
            && let Err(error) = save_workspace_snapshot_sync(&self.store_path, &snapshot)
        {
            self.status = format!("Selected pane, but failed to save focus: {error:#}").into();
        }
        cx.notify();
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
        if let Some(live) = self.live_terminals.get(&live_key) {
            match &input {
                TerminalInput::LiveKey(key_input) => {
                    if let Err(error) = live.handle.send_key(key_input.clone()) {
                        self.status = format!("Terminal input failed: {error:#}").into();
                    }
                }
            }
            if let Some(snapshot) = self.active_snapshot.as_mut() {
                snapshot.active_pane_id = Some(pane_id);
            }
            self.schedule_terminal_poll(cx);
            cx.notify();
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
            match spawn_live_terminal(spec) {
                Ok(handle) => {
                    self.failed_live_terminals.remove(&key);
                    self.live_terminals.insert(
                        key,
                        LiveTerminalPane {
                            handle,
                            latest: None,
                            last_resize: None,
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

    fn schedule_terminal_poll(&mut self, cx: &mut Context<Self>) {
        if self.terminal_poll_active || self.live_terminals.is_empty() {
            return;
        }

        self.terminal_poll_active = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let keep_polling = this
                    .update(cx, |app, cx| {
                        let changed = app.drain_live_terminal_snapshots();
                        if changed {
                            cx.notify();
                        }
                        let keep_polling = !app.live_terminals.is_empty();
                        if !keep_polling {
                            app.terminal_poll_active = false;
                        }
                        keep_polling
                    })
                    .unwrap_or(false);
                if !keep_polling {
                    break;
                }
            }
        })
        .detach();
    }

    fn drain_live_terminal_snapshots(&mut self) -> bool {
        let mut changed = false;
        let Some(active_workspace) = self.active_workspace().cloned() else {
            return false;
        };
        let mut updates = Vec::new();
        for (key, live) in &mut self.live_terminals {
            for snapshot in live.handle.drain_snapshots() {
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
        self.schedule_terminal_poll(cx);
        cx.stop_propagation();
    }
}

impl Render for OcttyApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_live_terminals_for_active_snapshot();
        self.schedule_terminal_poll(cx);
        for (workspace_id, pane_id, cols, rows) in
            terminal_resize_requests(self.active_snapshot.as_ref())
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

        let taskspace = render_taskspace(self.active_snapshot.as_ref(), &self.live_terminals, cx);

        div()
            .id("octty-rs-root")
            .key_context("OcttyApp")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::open_workspace))
            .on_action(cx.listener(Self::add_shell_pane))
            .on_action(cx.listener(Self::add_diff_pane))
            .on_action(cx.listener(Self::add_note_pane))
            .on_key_down(cx.listener(Self::handle_key_down))
            .flex()
            .size_full()
            .bg(rgb(0x171717))
            .text_color(rgb(0xf2f2f2))
            .child(
                div()
                    .w(px(280.0))
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
    Ok(store
        .get_snapshot(&workspace.id)
        .await?
        .unwrap_or_else(|| create_default_snapshot(workspace.id.clone())))
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

fn load_workspace_snapshot_sync(
    store_path: &Path,
    workspace: &WorkspaceSummary,
) -> anyhow::Result<WorkspaceSnapshot> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let store = TursoStore::open(store_path).await?;
        load_workspace_snapshot(&store, workspace).await
    })
}

fn save_workspace_snapshot_sync(
    store_path: &Path,
    snapshot: &WorkspaceSnapshot,
) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let store = TursoStore::open(store_path).await?;
        store.save_snapshot(snapshot).await?;
        Ok(())
    })
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

fn workspace_key_bindings() -> [KeyBinding; 10] {
    [
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
    cx: &mut Context<OcttyApp>,
) -> gpui::Div {
    let mut taskspace = div().mt_4().flex().gap_3().flex_1().h_full();
    let Some(snapshot) = snapshot else {
        return taskspace.child(
            div()
                .text_color(rgb(0xa0a0a0))
                .child("Open a workspace to start."),
        );
    };
    if snapshot.panes.is_empty() {
        return taskspace.child(div().text_color(rgb(0xa0a0a0)).child("No panes are open."));
    }

    for column_id in &snapshot.center_column_ids {
        let Some(column) = snapshot.columns.get(column_id) else {
            continue;
        };
        let mut column_el = div()
            .flex()
            .flex_col()
            .gap_3()
            .h_full()
            .w(px(column.width_px as f32));
        for pane_id in &column.pane_ids {
            if let Some(pane) = snapshot.panes.get(pane_id) {
                let active = snapshot.active_pane_id.as_deref() == Some(pane.id.as_str());
                let terminal_snapshot = live_terminals
                    .get(&live_terminal_key(&snapshot.workspace_id, &pane.id))
                    .and_then(|live| live.latest.as_ref());
                column_el = column_el.child(render_pane(
                    &snapshot.workspace_id,
                    pane,
                    active,
                    terminal_snapshot,
                    cx,
                ));
            }
        }
        taskspace = taskspace.child(column_el);
    }
    taskspace
}

fn render_pane(
    workspace_id: &str,
    pane: &PaneState,
    active: bool,
    terminal_snapshot: Option<&TerminalGridSnapshot>,
    cx: &mut Context<OcttyApp>,
) -> gpui::Div {
    let pane_id = pane.id.clone();
    let scroll_workspace_id = workspace_id.to_owned();
    let scroll_pane_id = pane.id.clone();
    div()
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
        }))
        .child(
            div()
                .p_2()
                .border_b_1()
                .border_color(rgb(0x333333))
                .text_sm()
                .child(pane.title.clone()),
        )
        .child(render_pane_body(pane, active, terminal_snapshot))
}

fn render_pane_body(
    pane: &PaneState,
    active: bool,
    terminal_snapshot: Option<&TerminalGridSnapshot>,
) -> gpui::Div {
    match &pane.payload {
        PanePayload::Terminal(payload) => {
            render_terminal_surface(payload, active, terminal_snapshot)
        }
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
    terminal_snapshot: Option<&TerminalGridSnapshot>,
) -> gpui::Div {
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
    let mut surface = div()
        .flex_1()
        .overflow_hidden()
        .p_2()
        .bg(default_bg)
        .font(terminal_font())
        .text_size(px(TERMINAL_FONT_SIZE))
        .line_height(px(TERMINAL_CELL_HEIGHT))
        .child(
            div()
                .text_xs()
                .mb_1()
                .text_color(rgb(if active { 0x8fd694 } else { 0x5f6b78 }))
                .child(format!(
                    "{:?} · {}x{} · {}",
                    payload.kind, snapshot.cols, snapshot.rows, payload.cwd
                )),
        );

    for (row_index, row) in snapshot.rows_data.iter().enumerate() {
        surface = surface.child(render_terminal_row(
            row_index as u16,
            row,
            snapshot,
            default_fg,
            default_bg,
        ));
    }
    surface
}

fn render_terminal_row(
    row_index: u16,
    row: &octty_term::live::TerminalRowSnapshot,
    snapshot: &TerminalGridSnapshot,
    default_fg: Rgba,
    default_bg: Rgba,
) -> gpui::Div {
    let mut row_el = div()
        .flex()
        .h(px(TERMINAL_CELL_HEIGHT))
        .line_height(px(TERMINAL_CELL_HEIGHT));

    for run in terminal_cell_runs(row_index, row, snapshot) {
        let mut run_el = div()
            .h(px(TERMINAL_CELL_HEIGHT))
            .w(px(TERMINAL_CELL_WIDTH * run.cell_count as f32))
            .overflow_hidden()
            .text_color(run.fg.unwrap_or(default_fg))
            .bg(run.bg.unwrap_or(default_bg))
            .child(run.text);
        if run.bold {
            run_el = run_el.font_weight(gpui::FontWeight::BOLD);
        }
        if run.italic {
            run_el = run_el.italic();
        }
        if run.underline {
            run_el = run_el.underline();
        }
        row_el = row_el.child(run_el);
    }
    row_el
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
                format!("Terminal · kind={:?} · cwd={}", payload.kind, payload.cwd)
            } else {
                format!("Terminal · {:?} · {}\n{screen}", payload.kind, payload.cwd)
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

fn terminal_resize_requests(
    snapshot: Option<&WorkspaceSnapshot>,
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
        let pane_height = (TERMINAL_GRID_HEIGHT - (pane_count.saturating_sub(1) as f32 * 12.0))
            / pane_count as f32;
        let terminal_height = (pane_height - TERMINAL_PANE_CHROME_HEIGHT).max(TERMINAL_CELL_HEIGHT);
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
    fn control_letters_keep_control_modifier_for_encoder() {
        let input = live_terminal_input_from_key_parts("c", None, true, false, false, false, false)
            .expect("control input");
        assert_eq!(input.key, LiveTerminalKey::Character('c'));
        assert!(input.modifiers.control);
    }
}
