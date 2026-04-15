use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::types::{
    BrowserPanePayload, DiffPanePayload, NotePanePayload, PanePayload, PaneState, PaneType,
    SessionState, TerminalKind, TerminalPanePayload, WorkspaceColumn, WorkspaceSnapshot,
};

const TERMINAL_COLUMN_WIDTH_PX: f64 = 720.0;
const AGENT_TERMINAL_COLUMN_WIDTH_PX: f64 = 840.0;
const NOTE_COLUMN_WIDTH_PX: f64 = 420.0;
const BROWSER_COLUMN_WIDTH_PX: f64 = 960.0;
const DIFF_COLUMN_WIDTH_PX: f64 = 900.0;
pub const LAYOUT_VERSION: u32 = 2;

static ID_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LayoutError {
    #[error("pane `{0}` does not exist")]
    MissingPane(String),
}

pub fn create_default_snapshot(workspace_id: impl Into<String>) -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        layout_version: LAYOUT_VERSION,
        workspace_id: workspace_id.into(),
        active_pane_id: None,
        panes: Default::default(),
        columns: Default::default(),
        center_column_ids: Vec::new(),
        pinned_left_column_id: None,
        pinned_right_column_id: None,
        updated_at: now_ms(),
    }
}

pub fn create_pane_state(
    pane_type: PaneType,
    workspace_path: impl Into<String>,
    terminal_kind: Option<TerminalKind>,
) -> PaneState {
    let workspace_path = workspace_path.into();
    let id = next_id("pane");
    let (title, payload) = match pane_type {
        PaneType::Shell => {
            let kind = terminal_kind.unwrap_or(TerminalKind::Shell);
            (
                terminal_title(&kind).to_owned(),
                terminal_payload(kind, workspace_path),
            )
        }
        PaneType::AgentShell => {
            let kind = terminal_kind.unwrap_or(TerminalKind::Codex);
            (
                terminal_title(&kind).to_owned(),
                terminal_payload(kind, workspace_path),
            )
        }
        PaneType::Note => (
            "Note".to_owned(),
            PanePayload::Note(NotePanePayload { note_path: None }),
        ),
        PaneType::Browser => (
            "Browser".to_owned(),
            PanePayload::Browser(BrowserPanePayload {
                url: "about:blank".to_owned(),
                title: "Browser".to_owned(),
                zoom_factor: 1.0,
                pending_popup_id: None,
            }),
        ),
        PaneType::Diff => (
            "Diff".to_owned(),
            PanePayload::Diff(DiffPanePayload { pinned: false }),
        ),
    };

    PaneState {
        id,
        pane_type,
        title,
        payload,
    }
}

pub fn add_pane(mut snapshot: WorkspaceSnapshot, pane: PaneState) -> WorkspaceSnapshot {
    let pane_id = pane.id.clone();
    let column_id = next_id("column");
    let column_width_px = default_column_width_px(&pane);
    snapshot.panes.insert(pane_id.clone(), pane);
    snapshot.columns.insert(
        column_id.clone(),
        WorkspaceColumn {
            id: column_id.clone(),
            pane_ids: vec![pane_id.clone()],
            width_px: column_width_px,
            height_fractions: vec![1.0],
            pinned: None,
        },
    );
    snapshot.center_column_ids.push(column_id);
    snapshot.active_pane_id = Some(pane_id);
    snapshot.updated_at = now_ms();
    snapshot
}

pub fn remove_pane(
    mut snapshot: WorkspaceSnapshot,
    pane_id: &str,
) -> Result<WorkspaceSnapshot, LayoutError> {
    if snapshot.panes.remove(pane_id).is_none() {
        return Err(LayoutError::MissingPane(pane_id.to_owned()));
    }

    let mut empty_columns = Vec::new();
    for (column_id, column) in snapshot.columns.iter_mut() {
        if let Some(index) = column.pane_ids.iter().position(|id| id == pane_id) {
            column.pane_ids.remove(index);
            column.height_fractions.remove(index);
        }
        if column.pane_ids.is_empty() {
            empty_columns.push(column_id.clone());
        } else {
            normalize_heights(column);
        }
    }

    for column_id in empty_columns {
        snapshot.columns.remove(&column_id);
        snapshot.center_column_ids.retain(|id| id != &column_id);
        if snapshot.pinned_left_column_id.as_deref() == Some(&column_id) {
            snapshot.pinned_left_column_id = None;
        }
        if snapshot.pinned_right_column_id.as_deref() == Some(&column_id) {
            snapshot.pinned_right_column_id = None;
        }
    }

    if snapshot.active_pane_id.as_deref() == Some(pane_id) {
        snapshot.active_pane_id = snapshot
            .center_column_ids
            .iter()
            .filter_map(|column_id| snapshot.columns.get(column_id))
            .flat_map(|column| column.pane_ids.iter())
            .next()
            .cloned();
    }
    snapshot.updated_at = now_ms();
    Ok(snapshot)
}

fn default_column_width_px(pane: &PaneState) -> f64 {
    match pane.pane_type {
        PaneType::Shell => TERMINAL_COLUMN_WIDTH_PX,
        PaneType::AgentShell => AGENT_TERMINAL_COLUMN_WIDTH_PX,
        PaneType::Note => NOTE_COLUMN_WIDTH_PX,
        PaneType::Browser => BROWSER_COLUMN_WIDTH_PX,
        PaneType::Diff => DIFF_COLUMN_WIDTH_PX,
    }
}

fn terminal_payload(kind: TerminalKind, cwd: String) -> PanePayload {
    PanePayload::Terminal(TerminalPanePayload {
        command: terminal_command(&kind).to_owned(),
        kind,
        session_id: None,
        session_state: SessionState::Stopped,
        cwd,
        exit_code: None,
        auto_start: true,
        restored_buffer: String::new(),
        embedded_session: None,
        embedded_session_correlation_id: None,
        agent_attention_state: None,
    })
}

fn terminal_title(kind: &TerminalKind) -> &'static str {
    match kind {
        TerminalKind::Shell => "Shell",
        TerminalKind::Codex => "Codex",
        TerminalKind::Pi => "PI",
        TerminalKind::Nvim => "Nvim",
        TerminalKind::Jjui => "JJ UI",
    }
}

fn terminal_command(kind: &TerminalKind) -> &'static str {
    match kind {
        TerminalKind::Shell => "",
        TerminalKind::Codex => "codex",
        TerminalKind::Pi => "pi",
        TerminalKind::Nvim => "nvim",
        TerminalKind::Jjui => "jjui",
    }
}

fn normalize_heights(column: &mut WorkspaceColumn) {
    if column.height_fractions.len() != column.pane_ids.len() || column.height_fractions.is_empty()
    {
        let fraction = 1.0 / column.pane_ids.len() as f64;
        column.height_fractions = vec![fraction; column.pane_ids.len()];
    }
}

fn next_id(prefix: &str) -> String {
    let id = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{id}", now_ms())
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use crate::types::{PanePayload, PaneType};

    use super::*;

    #[test]
    fn new_workspaces_start_without_panes() {
        let snapshot = create_default_snapshot("workspace-1");
        assert!(snapshot.panes.is_empty());
        assert!(snapshot.columns.is_empty());
        assert!(snapshot.active_pane_id.is_none());
    }

    #[test]
    fn add_pane_creates_a_center_column() {
        let snapshot = create_default_snapshot("workspace-1");
        let pane = create_pane_state(PaneType::Shell, "/tmp/ws", None);
        let pane_id = pane.id.clone();

        let snapshot = add_pane(snapshot, pane);

        assert_eq!(snapshot.active_pane_id.as_deref(), Some(pane_id.as_str()));
        assert_eq!(snapshot.center_column_ids.len(), 1);
        assert!(snapshot.panes.contains_key(&pane_id));
        assert!(matches!(
            snapshot.panes[&pane_id].payload,
            PanePayload::Terminal(_)
        ));
        let column = snapshot
            .columns
            .get(&snapshot.center_column_ids[0])
            .expect("created column");
        assert_eq!(column.width_px, TERMINAL_COLUMN_WIDTH_PX);
    }

    #[test]
    fn pane_types_get_individual_default_widths() {
        let mut snapshot = create_default_snapshot("workspace-1");
        snapshot = add_pane(snapshot, create_pane_state(PaneType::Note, "/tmp/ws", None));
        snapshot = add_pane(snapshot, create_pane_state(PaneType::Diff, "/tmp/ws", None));
        snapshot = add_pane(
            snapshot,
            create_pane_state(PaneType::Browser, "/tmp/ws", None),
        );

        let widths: Vec<_> = snapshot
            .center_column_ids
            .iter()
            .map(|column_id| snapshot.columns[column_id].width_px)
            .collect();
        assert_eq!(
            widths,
            vec![
                NOTE_COLUMN_WIDTH_PX,
                DIFF_COLUMN_WIDTH_PX,
                BROWSER_COLUMN_WIDTH_PX
            ]
        );
    }

    #[test]
    fn new_ids_include_time_and_counter() {
        let first = create_pane_state(PaneType::Shell, "/tmp/ws", None);
        let second = create_pane_state(PaneType::Shell, "/tmp/ws", None);

        assert_ne!(first.id, second.id);
        assert!(first.id.starts_with("pane-"));
    }

    #[test]
    fn remove_active_pane_selects_next_available_pane() {
        let snapshot = add_pane(
            create_default_snapshot("workspace-1"),
            create_pane_state(PaneType::Shell, "/tmp/ws", None),
        );
        let first_id = snapshot.active_pane_id.clone().unwrap();
        let snapshot = add_pane(snapshot, create_pane_state(PaneType::Diff, "/tmp/ws", None));
        let second_id = snapshot.active_pane_id.clone().unwrap();

        let snapshot = remove_pane(snapshot, &second_id).unwrap();

        assert_eq!(snapshot.active_pane_id.as_deref(), Some(first_id.as_str()));
        assert!(!snapshot.panes.contains_key(&second_id));
    }
}
