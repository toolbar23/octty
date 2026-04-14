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
    pub session_id: Option<String>,
    pub session_state: SessionState,
    pub cwd: String,
    pub command: String,
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
