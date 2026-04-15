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
    project_roots: Vec<ProjectRootRecord>,
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
    terminal_deferred_snapshot_timer_active: bool,
    terminal_window_active: bool,
    terminal_last_snapshot_notify_at: Option<Instant>,
    terminal_glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
    terminal_render_cache: Rc<RefCell<TerminalRenderCache>>,
    sidebar_menu: Option<SidebarMenuOverlay>,
    sidebar_rename_dialog: Option<SidebarRenameDialog>,
    toasts: VecDeque<AppToast>,
    next_toast_id: u64,
    pane_activity: HashMap<(String, String), PaneActivity>,
    pending_pane_activity_persistence: HashMap<(String, String), PaneActivity>,
    pane_activity_persist_active: bool,
    pane_activity_reconcile_active: bool,
}

struct LiveTerminalPane {
    handle: LiveTerminalHandle,
    latest: Option<TerminalGridSnapshot>,
    pending_snapshot: Option<TerminalGridSnapshot>,
    last_presented_snapshot_at: Option<Instant>,
    last_resize: Option<(u16, u16)>,
    last_input_at: Option<Instant>,
    latency: TerminalLatencyStats,
    selection: Option<TerminalSelection>,
    selection_drag: Option<TerminalSelectionDrag>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TerminalGridPoint {
    row: u16,
    col: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TerminalSelection {
    anchor: TerminalGridPoint,
    active: TerminalGridPoint,
    mode: TerminalSelectionMode,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct TerminalSelectionMode {
    rectangular: bool,
    filter_indent: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TerminalSelectionDrag {
    anchor: TerminalGridPoint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TerminalSelectionRun {
    row: u16,
    start_col: u16,
    end_col: u16,
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

#[derive(Default)]
struct TerminalSnapshotDrainResult {
    changed: bool,
    deferred_delay: Option<Duration>,
}

impl TerminalSnapshotDrainResult {
    fn defer_for(&mut self, delay: Duration) {
        self.deferred_delay = Some(
            self.deferred_delay
                .map_or(delay, |current| current.min(delay)),
        );
    }
}

fn take_presentable_terminal_snapshot(
    live: &mut LiveTerminalPane,
    focused: bool,
    now: Instant,
) -> Option<TerminalGridSnapshot> {
    if terminal_snapshot_presentation_delay(live, focused, now).is_some() {
        return None;
    }
    live.pending_snapshot.take()
}

fn terminal_snapshot_presentation_delay(
    live: &LiveTerminalPane,
    focused: bool,
    now: Instant,
) -> Option<Duration> {
    terminal_snapshot_presentation_delay_for_state(
        live.pending_snapshot.is_some(),
        live.last_presented_snapshot_at,
        focused,
        now,
    )
}

fn terminal_snapshot_presentation_delay_for_state(
    has_pending_snapshot: bool,
    last_presented_at: Option<Instant>,
    focused: bool,
    now: Instant,
) -> Option<Duration> {
    if !has_pending_snapshot || focused {
        return None;
    }
    let last_presented_at = last_presented_at?;
    let elapsed = now.saturating_duration_since(last_presented_at);
    if elapsed >= TERMINAL_BACKGROUND_FRAME_INTERVAL {
        None
    } else {
        Some(TERMINAL_BACKGROUND_FRAME_INTERVAL - elapsed)
    }
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

fn terminal_snapshot_coalesce_interval(
    window_active: bool,
    has_recent_input: bool,
    last_snapshot_notify_at: Option<Instant>,
    now: Instant,
) -> Duration {
    if window_active && has_recent_input {
        remaining_terminal_frame_delay(last_snapshot_notify_at, now)
    } else if window_active {
        TERMINAL_FOCUSED_FRAME_INTERVAL
    } else {
        TERMINAL_BACKGROUND_FRAME_INTERVAL
    }
}

fn remaining_terminal_frame_delay(
    last_snapshot_notify_at: Option<Instant>,
    now: Instant,
) -> Duration {
    let Some(last_snapshot_notify_at) = last_snapshot_notify_at else {
        return Duration::ZERO;
    };
    let elapsed = now.saturating_duration_since(last_snapshot_notify_at);
    TERMINAL_FOCUSED_FRAME_INTERVAL.saturating_sub(elapsed)
}
