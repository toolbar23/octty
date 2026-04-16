use super::*;

pub(crate) fn push_latency_sample(samples: &mut VecDeque<u64>, micros: u64) {
    if samples.len() == TERMINAL_LATENCY_SAMPLE_LIMIT {
        samples.pop_front();
    }
    samples.push_back(micros);
}

pub(crate) fn duration_micros(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

pub(crate) fn latency_summary(samples: &VecDeque<u64>) -> Option<String> {
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

pub(crate) fn count_summary(samples: &VecDeque<u64>) -> Option<String> {
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

pub(crate) fn latency_percentile(sorted_micros: &[u64], percentile: usize) -> u64 {
    let index = ((sorted_micros.len().saturating_sub(1) * percentile) / 100)
        .min(sorted_micros.len().saturating_sub(1));
    sorted_micros[index]
}

pub(crate) fn format_latency_micros(micros: u64) -> String {
    if micros >= 1_000 {
        format!("{:.1}ms", micros as f64 / 1_000.0)
    } else {
        format!("{micros}us")
    }
}

pub(crate) fn terminal_font() -> Font {
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

pub(crate) fn terminal_font_family() -> String {
    std::env::var("OCTTY_RS_TERMINAL_FONT_FAMILY")
        .or_else(|_| std::env::var("OCTTY_TERMINAL_FONT_FAMILY"))
        .ok()
        .and_then(|family| first_font_family(&family))
        .unwrap_or_else(|| DEFAULT_TERMINAL_FONT_FAMILY.to_owned())
}

pub(crate) fn first_font_family(input: &str) -> Option<String> {
    input
        .split(',')
        .map(|family| family.trim().trim_matches('"').trim_matches('\'').trim())
        .find(|family| !family.is_empty() && !family.eq_ignore_ascii_case("monospace"))
        .map(str::to_owned)
}

pub(crate) fn default_terminal_grid_for_pane() -> (u16, u16) {
    (
        (720.0_f32 / TERMINAL_CELL_WIDTH).floor() as u16,
        (360.0_f32 / TERMINAL_CELL_HEIGHT).floor() as u16,
    )
}

pub(crate) fn taskspace_height_for_viewport(viewport_height: f32) -> f32 {
    (viewport_height - TERMINAL_TASKSPACE_VERTICAL_CHROME_HEIGHT).max(160.0)
}

pub(crate) fn taskspace_width_for_viewport(viewport_width: f32) -> f32 {
    (viewport_width - WORKSPACE_SIDEBAR_WIDTH - TASKSPACE_HORIZONTAL_PADDING).max(240.0)
}

pub(crate) fn terminal_surface_chrome_height() -> f32 {
    let debug_height = if terminal_performance_data_enabled() {
        TERMINAL_DEBUG_TIMER_LINE_HEIGHT + TERMINAL_SURFACE_DEBUG_TIMER_MARGIN_BOTTOM
    } else {
        0.0
    };
    terminal_pane_border_chrome() + TERMINAL_SURFACE_PADDING_Y + debug_height
}

pub(crate) fn terminal_surface_chrome_width() -> f32 {
    terminal_pane_border_chrome() + TERMINAL_SURFACE_PADDING_X + TERMINAL_SCROLLBAR_WIDTH
}

fn terminal_pane_border_chrome() -> f32 {
    TERMINAL_PANE_BORDER_WIDTH * 2.0
}

pub(crate) fn taskspace_viewport_offset(snapshot: &WorkspaceSnapshot, viewport_width: f32) -> f32 {
    let Some((active_left, active_width, total_width)) = active_column_metrics(snapshot) else {
        return 0.0;
    };
    let max_offset = (total_width - viewport_width).max(0.0);
    let centered_offset = active_left + (active_width / 2.0) - (viewport_width / 2.0);
    centered_offset.clamp(0.0, max_offset)
}

pub(crate) fn active_column_metrics(snapshot: &WorkspaceSnapshot) -> Option<(f32, f32, f32)> {
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

pub(crate) fn terminal_resize_requests(
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
            (pane_height - terminal_surface_chrome_height()).max(TERMINAL_CELL_HEIGHT);
        let cols = ((column.width_px as f32 - terminal_surface_chrome_width())
            / TERMINAL_CELL_WIDTH)
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

pub(crate) fn live_terminal_key(workspace_id: &str, pane_id: &str) -> String {
    format!("{workspace_id}:{pane_id}")
}

pub(crate) fn rekey_live_terminal_key(
    key: &str,
    previous_workspace_id: &str,
    next_workspace_id: &str,
) -> Option<(String, String)> {
    let (workspace_id, pane_id) = split_live_terminal_key(key)?;
    (workspace_id == previous_workspace_id).then(|| {
        (
            key.to_owned(),
            live_terminal_key(next_workspace_id, pane_id),
        )
    })
}

pub(crate) fn pane_activity_map(
    activities: Vec<PaneActivity>,
) -> HashMap<(String, String), PaneActivity> {
    activities
        .into_iter()
        .map(|activity| {
            (
                (activity.workspace_id.clone(), activity.pane_id.clone()),
                activity,
            )
        })
        .collect()
}

pub(crate) fn rekey_pane_activity_map(
    activities: &mut HashMap<(String, String), PaneActivity>,
    previous_workspace_id: &str,
    next_workspace_id: &str,
) {
    let rekeys = activities
        .keys()
        .filter(|(workspace_id, _pane_id)| workspace_id == previous_workspace_id)
        .cloned()
        .collect::<Vec<_>>();
    for previous_key in rekeys {
        let Some(mut activity) = activities.remove(&previous_key) else {
            continue;
        };
        activity.workspace_id = next_workspace_id.to_owned();
        activities.insert((next_workspace_id.to_owned(), previous_key.1), activity);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceActivityIndicator {
    pub(crate) activity_state: ActivityState,
    pub(crate) needs_attention: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PaneActivityIndicator {
    pub(crate) activity_state: ActivityState,
    pub(crate) needs_attention: bool,
    pub(crate) show_attention: bool,
}

pub(crate) fn pane_activity_indicator(
    workspace_id: &str,
    pane_id: &str,
    pane_activity: &HashMap<(String, String), PaneActivity>,
) -> PaneActivityIndicator {
    let now = now_ms();
    pane_activity
        .get(&(workspace_id.to_owned(), pane_id.to_owned()))
        .map(|activity| PaneActivityIndicator {
            activity_state: activity.state_at(now, PANE_ACTIVITY_ACTIVE_WINDOW_MS),
            needs_attention: activity.needs_attention,
            show_attention: pane_attention_visible(activity, now),
        })
        .unwrap_or(PaneActivityIndicator {
            activity_state: ActivityState::IdleSeen,
            needs_attention: false,
            show_attention: false,
        })
}

pub(crate) fn pane_attention_visible(activity: &PaneActivity, now_ms: i64) -> bool {
    activity.needs_attention || pane_attention_clear_remaining_ms(activity, now_ms).is_some()
}

pub(crate) fn pane_attention_clear_remaining_ms(
    activity: &PaneActivity,
    now_ms: i64,
) -> Option<i64> {
    if activity.needs_attention
        || activity.needs_attention_cleared_at_ms == 0
        || activity.needs_attention_cleared_at_ms < activity.needs_attention_at_ms
    {
        return None;
    }
    let elapsed = now_ms.saturating_sub(activity.needs_attention_cleared_at_ms);
    (elapsed < PANE_ATTENTION_CLEAR_GRACE_MS).then_some(PANE_ATTENTION_CLEAR_GRACE_MS - elapsed)
}

pub(crate) fn pane_attention_clear_delay(
    snapshot: Option<&WorkspaceSnapshot>,
    pane_activity: &HashMap<(String, String), PaneActivity>,
) -> Option<Duration> {
    let snapshot = snapshot?;
    let now = now_ms();
    snapshot
        .panes
        .keys()
        .filter_map(|pane_id| {
            pane_activity
                .get(&(snapshot.workspace_id.clone(), pane_id.clone()))
                .and_then(|activity| pane_attention_clear_remaining_ms(activity, now))
        })
        .min()
        .map(|remaining_ms| Duration::from_millis(remaining_ms as u64))
}

pub(crate) fn workspace_activity_indicator(
    workspace: &WorkspaceSummary,
    pane_activity: &HashMap<(String, String), PaneActivity>,
) -> WorkspaceActivityIndicator {
    let now = now_ms();
    let mut needs_attention = false;
    let activity_state = derive_workspace_activity(pane_activity.iter().filter_map(
        |((workspace_id, _), activity)| {
            if workspace_id != &workspace.id {
                return None;
            }
            needs_attention |= activity.needs_attention;
            Some(activity.state_at(now, PANE_ACTIVITY_ACTIVE_WINDOW_MS))
        },
    ));
    WorkspaceActivityIndicator {
        activity_state,
        needs_attention,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PaneBorderMarker {
    Attention,
    Focused,
    Unfocused,
}

pub(crate) fn pane_border_marker(
    active: bool,
    activity_indicator: PaneActivityIndicator,
) -> PaneBorderMarker {
    if activity_indicator.show_attention {
        PaneBorderMarker::Attention
    } else if active {
        PaneBorderMarker::Focused
    } else {
        PaneBorderMarker::Unfocused
    }
}

pub(crate) fn pane_border_color(active: bool, activity_indicator: PaneActivityIndicator) -> Hsla {
    match pane_border_marker(active, activity_indicator) {
        PaneBorderMarker::Attention => rgb(0xe5484d).into(),
        PaneBorderMarker::Focused => rgb(0x4e86d8).into(),
        PaneBorderMarker::Unfocused => rgb(0x444444).into(),
    }
}

pub(crate) fn split_live_terminal_key(key: &str) -> Option<(&str, &str)> {
    key.split_once(':')
}

pub(crate) fn terminal_rgb_to_rgba(color: TerminalRgb) -> Rgba {
    Rgba {
        r: color.r as f32 / 255.0,
        g: color.g as f32 / 255.0,
        b: color.b as f32 / 255.0,
        a: 1.0,
    }
}

pub(crate) fn terminal_dim_color(color: Rgba, target: Rgba) -> Rgba {
    Rgba {
        r: (color.r + target.r) * 0.5,
        g: (color.g + target.g) * 0.5,
        b: (color.b + target.b) * 0.5,
        a: color.a,
    }
}
