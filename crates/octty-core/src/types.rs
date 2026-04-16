use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaneType {
    Shell,
    AgentShell,
    Note,
    Browser,
    Diff,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TerminalKind {
    Shell,
    Codex,
    Pi,
    Nvim,
    Jjui,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TerminalExitBehavior {
    RestartAuto,
    RestartManually,
    Close,
}

impl Default for TerminalExitBehavior {
    fn default() -> Self {
        Self::Close
    }
}

pub const DEFAULT_TERMINAL_WIDTH_CHARS: u16 = 90;

pub fn default_shell_type_name() -> String {
    "plain".to_owned()
}

pub fn default_terminal_width_chars() -> u16 {
    DEFAULT_TERMINAL_WIDTH_CHARS
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PanePlacement {
    NewColumn,
    Stack,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SidebarTarget {
    Left,
    Right,
}

pub type ColumnPin = Option<SidebarTarget>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionState {
    Live,
    Stopped,
    Missing,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentAttentionState {
    IdleSeen,
    Thinking,
    IdleUnseen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActivityState {
    Active,
    IdleUnseen,
    IdleSeen,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneActivity {
    pub workspace_id: String,
    pub pane_id: String,
    pub last_activity_at_ms: i64,
    pub last_seen_at_ms: i64,
    pub last_seen_activity_at_ms: i64,
    pub last_tmux_activity_at_s: Option<i64>,
    pub last_seen_tmux_activity_at_s: Option<i64>,
    pub last_screen_fingerprint: Option<String>,
    pub last_seen_screen_fingerprint: Option<String>,
    pub updated_at_ms: i64,
}

impl PaneActivity {
    pub fn new(workspace_id: impl Into<String>, pane_id: impl Into<String>, now_ms: i64) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            pane_id: pane_id.into(),
            last_activity_at_ms: 0,
            last_seen_at_ms: 0,
            last_seen_activity_at_ms: 0,
            last_tmux_activity_at_s: None,
            last_seen_tmux_activity_at_s: None,
            last_screen_fingerprint: None,
            last_seen_screen_fingerprint: None,
            updated_at_ms: now_ms,
        }
    }

    pub fn record_activity(
        &mut self,
        at_ms: i64,
        tmux_activity_at_s: Option<i64>,
        screen_fingerprint: Option<String>,
    ) {
        self.last_activity_at_ms = self.last_activity_at_ms.max(at_ms);
        if let Some(tmux_activity_at_s) = tmux_activity_at_s {
            self.last_tmux_activity_at_s = Some(
                self.last_tmux_activity_at_s
                    .unwrap_or_default()
                    .max(tmux_activity_at_s),
            );
        }
        if let Some(screen_fingerprint) = screen_fingerprint {
            self.last_screen_fingerprint = Some(screen_fingerprint);
        }
        self.updated_at_ms = self.updated_at_ms.max(at_ms);
    }

    pub fn record_tmux_observation(
        &mut self,
        observed_at_ms: i64,
        tmux_activity_at_s: Option<i64>,
        screen_fingerprint: Option<String>,
    ) {
        if let Some(tmux_activity_at_s) = tmux_activity_at_s {
            if self
                .last_tmux_activity_at_s
                .is_none_or(|current| tmux_activity_at_s > current)
            {
                self.last_tmux_activity_at_s = Some(tmux_activity_at_s);
                self.last_activity_at_ms = self.last_activity_at_ms.max(observed_at_ms);
            }
        }
        if let Some(screen_fingerprint) = screen_fingerprint {
            self.last_screen_fingerprint = Some(screen_fingerprint);
        }
        self.updated_at_ms = self.updated_at_ms.max(observed_at_ms);
    }

    pub fn record_seen(&mut self, seen_at_ms: i64) {
        self.last_seen_at_ms = seen_at_ms;
        self.last_seen_activity_at_ms = self.last_activity_at_ms;
        self.last_seen_tmux_activity_at_s = self.last_tmux_activity_at_s;
        self.last_seen_screen_fingerprint = self.last_screen_fingerprint.clone();
        self.updated_at_ms = self.updated_at_ms.max(seen_at_ms);
    }

    pub fn state_at(&self, now_ms: i64, active_window_ms: i64) -> ActivityState {
        if self.last_activity_at_ms > 0
            && self.last_activity_at_ms > self.last_seen_activity_at_ms
            && now_ms.saturating_sub(self.last_activity_at_ms) <= active_window_ms
        {
            ActivityState::Active
        } else if self.has_unseen_activity() {
            ActivityState::IdleUnseen
        } else {
            ActivityState::IdleSeen
        }
    }

    pub fn has_unseen_activity(&self) -> bool {
        self.last_activity_at_ms > self.last_seen_activity_at_ms
            || self
                .last_tmux_activity_at_s
                .zip(self.last_seen_tmux_activity_at_s)
                .is_some_and(|(activity, seen)| activity > seen)
            || (self.last_tmux_activity_at_s.is_some()
                && self.last_seen_tmux_activity_at_s.is_none())
            || (self.last_screen_fingerprint.is_some()
                && self.last_screen_fingerprint != self.last_seen_screen_fingerprint)
    }
}

pub fn derive_workspace_activity(states: impl IntoIterator<Item = ActivityState>) -> ActivityState {
    let mut has_unseen = false;
    for state in states {
        match state {
            ActivityState::Active => return ActivityState::Active,
            ActivityState::IdleUnseen => has_unseen = true,
            ActivityState::IdleSeen => {}
        }
    }
    if has_unseen {
        ActivityState::IdleUnseen
    } else {
        ActivityState::IdleSeen
    }
}

pub fn screen_fingerprint(screen: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in screen.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceBookmarkRelation {
    None,
    Exact,
    Above,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceState {
    Published,
    MergedLocal,
    Draft,
    Conflicted,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddedSessionRef {
    pub provider: String,
    pub id: String,
    pub label: Option<String>,
    pub detected_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRootRecord {
    pub id: String,
    pub root_path: String,
    pub display_name: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceStatus {
    pub workspace_state: WorkspaceState,
    pub has_working_copy_changes: bool,
    pub effective_added_lines: i64,
    pub effective_removed_lines: i64,
    pub has_conflicts: bool,
    pub unpublished_change_count: i64,
    pub unpublished_added_lines: i64,
    pub unpublished_removed_lines: i64,
    pub not_in_default_available: bool,
    pub not_in_default_change_count: i64,
    pub not_in_default_added_lines: i64,
    pub not_in_default_removed_lines: i64,
    pub bookmarks: Vec<String>,
    pub bookmark_relation: WorkspaceBookmarkRelation,
    pub unread_notes: i64,
    pub active_agent_count: i64,
    pub agent_attention_state: Option<AgentAttentionState>,
    pub recent_activity_at: i64,
    pub diff_text: String,
}

impl Default for WorkspaceStatus {
    fn default() -> Self {
        Self {
            workspace_state: WorkspaceState::Unknown,
            has_working_copy_changes: false,
            effective_added_lines: 0,
            effective_removed_lines: 0,
            has_conflicts: false,
            unpublished_change_count: 0,
            unpublished_added_lines: 0,
            unpublished_removed_lines: 0,
            not_in_default_available: false,
            not_in_default_change_count: 0,
            not_in_default_added_lines: 0,
            not_in_default_removed_lines: 0,
            bookmarks: Vec::new(),
            bookmark_relation: WorkspaceBookmarkRelation::None,
            unread_notes: 0,
            active_agent_count: 0,
            agent_attention_state: None,
            recent_activity_at: 0,
            diff_text: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSummary {
    pub id: String,
    pub root_id: String,
    pub root_path: String,
    pub project_display_name: String,
    pub workspace_name: String,
    pub display_name: String,
    pub workspace_path: String,
    pub status: WorkspaceStatus,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_opened_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteRecord {
    pub workspace_id: String,
    pub path: String,
    pub file_name: String,
    pub title: String,
    pub body: String,
    pub unread: bool,
    pub updated_at: i64,
    pub last_read_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalPanePayload {
    pub kind: TerminalKind,
    #[serde(default = "default_shell_type_name")]
    pub shell_type: String,
    pub session_id: Option<String>,
    pub session_state: SessionState,
    pub cwd: String,
    pub command: String,
    #[serde(default)]
    pub command_parameters: Vec<String>,
    #[serde(default)]
    pub on_exit: TerminalExitBehavior,
    #[serde(default = "default_terminal_width_chars")]
    pub default_width_chars: u16,
    pub exit_code: Option<i64>,
    pub auto_start: bool,
    pub restored_buffer: String,
    pub embedded_session: Option<EmbeddedSessionRef>,
    pub embedded_session_correlation_id: Option<String>,
    pub agent_attention_state: Option<AgentAttentionState>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotePanePayload {
    pub note_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrowserPanePayload {
    pub url: String,
    pub title: String,
    pub zoom_factor: f64,
    pub pending_popup_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffPanePayload {
    pub pinned: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum PanePayload {
    Terminal(TerminalPanePayload),
    Note(NotePanePayload),
    Browser(BrowserPanePayload),
    Diff(DiffPanePayload),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaneState {
    pub id: String,
    pub pane_type: PaneType,
    pub title: String,
    pub payload: PanePayload,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceColumn {
    pub id: String,
    pub pane_ids: Vec<String>,
    pub width_px: f64,
    pub height_fractions: Vec<f64>,
    pub pinned: ColumnPin,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    pub layout_version: u32,
    pub workspace_id: String,
    pub active_pane_id: Option<String>,
    pub panes: BTreeMap<String, PaneState>,
    pub columns: BTreeMap<String, WorkspaceColumn>,
    pub center_column_ids: Vec<String>,
    pub pinned_left_column_id: Option<String>,
    pub pinned_right_column_id: Option<String>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceDetail {
    pub workspace: WorkspaceSummary,
    pub snapshot: WorkspaceSnapshot,
    pub notes: Vec<NoteRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub id: String,
    pub workspace_id: String,
    pub pane_id: String,
    pub kind: TerminalKind,
    pub cwd: String,
    pub command: String,
    pub buffer: String,
    pub screen: Option<String>,
    pub state: SessionState,
    pub exit_code: Option<i64>,
    pub embedded_session: Option<EmbeddedSessionRef>,
    pub embedded_session_correlation_id: Option<String>,
    pub agent_attention_state: Option<AgentAttentionState>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserRefRecord {
    pub workspace_id: String,
    pub pane_id: String,
    pub url: String,
    pub title: String,
    pub updated_at: i64,
}

pub const MISSING_WORKSPACE_PATH_PREFIX: &str = "jj-missing://";

pub fn encode_missing_workspace_path(workspace_name: &str) -> String {
    format!("{MISSING_WORKSPACE_PATH_PREFIX}{workspace_name}")
}

pub fn has_recorded_workspace_path(workspace_path: &str) -> bool {
    !workspace_path.starts_with(MISSING_WORKSPACE_PATH_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_activity_moves_from_active_to_unseen_until_seen() {
        let mut activity = PaneActivity::new("workspace-1", "pane-1", 1_000);

        activity.record_activity(2_000, None, Some(screen_fingerprint("hello")));

        assert_eq!(activity.state_at(2_500, 1_000), ActivityState::Active);
        activity.record_seen(2_600);
        assert_eq!(activity.state_at(2_700, 1_000), ActivityState::IdleSeen);

        activity.record_activity(3_000, None, Some(screen_fingerprint("hello again")));
        assert_eq!(activity.state_at(4_100, 1_000), ActivityState::IdleUnseen);

        activity.record_seen(4_200);

        assert_eq!(activity.state_at(4_300, 1_000), ActivityState::IdleSeen);
    }

    #[test]
    fn pane_activity_uses_tmux_and_fingerprint_as_restart_signals() {
        let mut activity = PaneActivity::new("workspace-1", "pane-1", 1_000);
        activity.record_tmux_observation(2_000, Some(10), Some(screen_fingerprint("first screen")));
        activity.record_seen(2_100);

        activity.record_tmux_observation(
            30_000,
            Some(11),
            Some(screen_fingerprint("first screen")),
        );

        assert_eq!(activity.state_at(30_000, 1_000), ActivityState::Active);

        activity.record_seen(30_100);
        activity.record_tmux_observation(
            40_000,
            Some(11),
            Some(screen_fingerprint("changed screen")),
        );

        assert_eq!(activity.state_at(40_000, 1_000), ActivityState::IdleUnseen);
    }

    #[test]
    fn pane_activity_uses_tmux_observation_time_for_active_window() {
        let mut activity = PaneActivity::new("workspace-1", "pane-1", 1_000);

        activity.record_tmux_observation(30_000, Some(11), None);

        assert_eq!(activity.last_tmux_activity_at_s, Some(11));
        assert_eq!(activity.last_activity_at_ms, 30_000);
        assert_eq!(activity.state_at(32_900, 3_000), ActivityState::Active);
        assert_eq!(activity.state_at(33_100, 3_000), ActivityState::IdleUnseen);
    }

    #[test]
    fn stale_tmux_observation_does_not_fabricate_activity() {
        let mut activity = PaneActivity::new("workspace-1", "pane-1", 1_000);

        activity.record_tmux_observation(10_000, Some(10), None);
        activity.record_seen(10_100);
        activity.record_tmux_observation(20_000, Some(10), None);

        assert_eq!(activity.last_activity_at_ms, 10_000);
        assert_eq!(activity.state_at(20_000, 3_000), ActivityState::IdleSeen);
    }

    #[test]
    fn workspace_activity_uses_active_then_unseen_precedence() {
        assert_eq!(
            derive_workspace_activity([
                ActivityState::IdleSeen,
                ActivityState::IdleUnseen,
                ActivityState::IdleSeen,
            ]),
            ActivityState::IdleUnseen
        );
        assert_eq!(
            derive_workspace_activity([
                ActivityState::IdleUnseen,
                ActivityState::Active,
                ActivityState::IdleSeen,
            ]),
            ActivityState::Active
        );
        assert_eq!(
            derive_workspace_activity([ActivityState::IdleSeen]),
            ActivityState::IdleSeen
        );
    }
}
