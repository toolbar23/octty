use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TerminalInput {
    LiveKey(LiveTerminalKeyInput),
}

#[derive(Clone, Debug)]
pub(crate) struct PendingTerminalInput {
    pub(crate) workspace: WorkspaceSummary,
    pub(crate) snapshot: WorkspaceSnapshot,
    pub(crate) pane_id: String,
    pub(crate) payload: TerminalPanePayload,
    pub(crate) input: TerminalInput,
}

pub(crate) struct OcttyApp {
    pub(crate) status: SharedString,
    pub(crate) project_roots: Vec<ProjectRootRecord>,
    pub(crate) workspaces: Vec<WorkspaceSummary>,
    pub(crate) active_workspace_index: Option<usize>,
    pub(crate) active_snapshot: Option<WorkspaceSnapshot>,
    pub(crate) store_path: std::path::PathBuf,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) pending_terminal_inputs: Vec<PendingTerminalInput>,
    pub(crate) terminal_flush_active: bool,
    pub(crate) live_terminals: HashMap<String, LiveTerminalPane>,
    pub(crate) failed_live_terminals: BTreeSet<String>,
    pub(crate) terminal_snapshot_tx: mpsc::UnboundedSender<()>,
    pub(crate) terminal_snapshot_rx: Option<mpsc::UnboundedReceiver<()>>,
    pub(crate) terminal_notifications_active: bool,
    pub(crate) terminal_deferred_snapshot_timer_active: bool,
    pub(crate) terminal_window_active: bool,
    pub(crate) terminal_last_snapshot_notify_at: Option<Instant>,
    pub(crate) terminal_glyph_cache: Rc<RefCell<TerminalGlyphLayoutCache>>,
    pub(crate) terminal_render_cache: Rc<RefCell<TerminalRenderCache>>,
    pub(crate) sidebar_menu: Option<SidebarMenuOverlay>,
    pub(crate) sidebar_rename_dialog: Option<SidebarRenameDialog>,
    pub(crate) toasts: VecDeque<AppToast>,
    pub(crate) next_toast_id: u64,
    pub(crate) pane_activity: HashMap<(String, String), PaneActivity>,
    pub(crate) pending_pane_activity_persistence: HashMap<(String, String), PaneActivity>,
    pub(crate) pane_activity_persist_active: bool,
    pub(crate) pane_activity_reconcile_active: bool,
}

pub(crate) struct LiveTerminalPane {
    pub(crate) handle: LiveTerminalHandle,
    pub(crate) latest: Option<TerminalGridSnapshot>,
    pub(crate) pending_snapshot: Option<TerminalGridSnapshot>,
    pub(crate) last_presented_snapshot_at: Option<Instant>,
    pub(crate) last_resize: Option<(u16, u16)>,
    pub(crate) last_input_at: Option<Instant>,
    pub(crate) latency: TerminalLatencyStats,
    pub(crate) selection: Option<TerminalSelection>,
    pub(crate) selection_drag: Option<TerminalSelectionDrag>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerminalGridPoint {
    pub(crate) row: u16,
    pub(crate) col: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TerminalSelection {
    pub(crate) anchor: TerminalGridPoint,
    pub(crate) active: TerminalGridPoint,
    pub(crate) mode: TerminalSelectionMode,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TerminalSelectionMode {
    pub(crate) rectangular: bool,
    pub(crate) filter_indent: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TerminalSelectionDrag {
    pub(crate) anchor: TerminalGridPoint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerminalSelectionRun {
    pub(crate) row: u16,
    pub(crate) start_col: u16,
    pub(crate) end_col: u16,
}

#[derive(Default)]
pub(crate) struct TerminalLatencyStats {
    pub(crate) key_to_snapshot_micros: VecDeque<u64>,
    pub(crate) pty_to_snapshot_micros: VecDeque<u64>,
    pub(crate) pty_output_bytes: VecDeque<u64>,
    pub(crate) vt_write_micros: VecDeque<u64>,
    pub(crate) snapshot_update_micros: VecDeque<u64>,
    pub(crate) snapshot_extract_micros: VecDeque<u64>,
    pub(crate) snapshot_build_micros: VecDeque<u64>,
    pub(crate) dirty_rows: VecDeque<u64>,
    pub(crate) dirty_cells: VecDeque<u64>,
}

impl TerminalLatencyStats {
    pub(crate) fn record_key_to_snapshot(&mut self, duration: Duration) {
        push_latency_sample(
            &mut self.key_to_snapshot_micros,
            duration.as_micros().min(u128::from(u64::MAX)) as u64,
        );
    }

    pub(crate) fn record_pty_to_snapshot(&mut self, micros: Option<u64>) {
        if let Some(micros) = micros {
            push_latency_sample(&mut self.pty_to_snapshot_micros, micros);
        }
    }

    pub(crate) fn record_pty_output_bytes(&mut self, bytes: u64) {
        push_latency_sample(&mut self.pty_output_bytes, bytes);
    }

    pub(crate) fn record_vt_write(&mut self, micros: u64) {
        push_latency_sample(&mut self.vt_write_micros, micros);
    }

    pub(crate) fn record_snapshot_update(&mut self, micros: u64) {
        push_latency_sample(&mut self.snapshot_update_micros, micros);
    }

    pub(crate) fn record_snapshot_extract(&mut self, micros: u64) {
        push_latency_sample(&mut self.snapshot_extract_micros, micros);
    }

    pub(crate) fn record_snapshot_build(&mut self, micros: u64) {
        push_latency_sample(&mut self.snapshot_build_micros, micros);
    }

    pub(crate) fn record_dirty_rows(&mut self, rows: u32) {
        push_latency_sample(&mut self.dirty_rows, u64::from(rows));
    }

    pub(crate) fn record_dirty_cells(&mut self, cells: u32) {
        push_latency_sample(&mut self.dirty_cells, u64::from(cells));
    }

    pub(crate) fn summary_label(&self) -> Option<String> {
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

pub(crate) fn drain_pending_terminal_notifications(
    notification_rx: &mut mpsc::UnboundedReceiver<()>,
) {
    while notification_rx.try_recv().is_ok() {}
}

#[derive(Default)]
pub(crate) struct TerminalSnapshotDrainResult {
    pub(crate) changed: bool,
    pub(crate) deferred_delay: Option<Duration>,
}

impl TerminalSnapshotDrainResult {
    pub(crate) fn defer_for(&mut self, delay: Duration) {
        self.deferred_delay = Some(
            self.deferred_delay
                .map_or(delay, |current| current.min(delay)),
        );
    }
}

pub(crate) fn take_presentable_terminal_snapshot(
    live: &mut LiveTerminalPane,
    focused: bool,
    now: Instant,
) -> Option<TerminalGridSnapshot> {
    if terminal_snapshot_presentation_delay(live, focused, now).is_some() {
        return None;
    }
    live.pending_snapshot.take()
}

pub(crate) fn terminal_snapshot_presentation_delay(
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

pub(crate) fn terminal_snapshot_presentation_delay_for_state(
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

pub(crate) fn coalesce_terminal_snapshots(
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

pub(crate) fn terminal_snapshot_coalesce_interval(
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

pub(crate) fn remaining_terminal_frame_delay(
    last_snapshot_notify_at: Option<Instant>,
    now: Instant,
) -> Duration {
    let Some(last_snapshot_notify_at) = last_snapshot_notify_at else {
        return Duration::ZERO;
    };
    let elapsed = now.saturating_duration_since(last_snapshot_notify_at);
    TERMINAL_FOCUSED_FRAME_INTERVAL.saturating_sub(elapsed)
}
