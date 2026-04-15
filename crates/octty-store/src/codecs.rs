use octty_core::{
    AgentAttentionState, SessionState, TerminalKind, WorkspaceBookmarkRelation, WorkspaceState,
};
use turso::Value;

use crate::StoreError;

pub(crate) fn text(value: Value, column: &'static str) -> Result<String, StoreError> {
    match value {
        Value::Text(value) => Ok(value),
        value => Err(StoreError::UnexpectedValue { column, value }),
    }
}

pub(crate) fn integer(value: Value, column: &'static str) -> Result<i64, StoreError> {
    match value {
        Value::Integer(value) => Ok(value),
        value => Err(StoreError::UnexpectedValue { column, value }),
    }
}

pub(crate) fn optional_integer(
    value: Value,
    column: &'static str,
) -> Result<Option<i64>, StoreError> {
    match value {
        Value::Null => Ok(None),
        Value::Integer(value) => Ok(Some(value)),
        value => Err(StoreError::UnexpectedValue { column, value }),
    }
}

pub(crate) fn optional_text(
    value: Value,
    column: &'static str,
) -> Result<Option<String>, StoreError> {
    match value {
        Value::Null => Ok(None),
        Value::Text(value) => Ok(Some(value)),
        value => Err(StoreError::UnexpectedValue { column, value }),
    }
}

pub(crate) fn optional_i64_to_value(value: Option<i64>) -> Result<Value, turso::Error> {
    Ok(value.map(Value::Integer).unwrap_or(Value::Null))
}

pub(crate) fn optional_str_to_value(value: Option<&str>) -> Result<Value, turso::Error> {
    Ok(value
        .map(|value| Value::Text(value.to_owned()))
        .unwrap_or(Value::Null))
}

pub(crate) fn bool_to_int(value: bool) -> i64 {
    i64::from(value)
}

pub(crate) fn workspace_state_to_str(value: &WorkspaceState) -> &'static str {
    match value {
        WorkspaceState::Published => "published",
        WorkspaceState::MergedLocal => "merged-local",
        WorkspaceState::Draft => "draft",
        WorkspaceState::Conflicted => "conflicted",
        WorkspaceState::Unknown => "unknown",
    }
}

pub(crate) fn parse_workspace_state(value: &str) -> WorkspaceState {
    match value {
        "published" => WorkspaceState::Published,
        "merged-local" => WorkspaceState::MergedLocal,
        "draft" => WorkspaceState::Draft,
        "conflicted" => WorkspaceState::Conflicted,
        _ => WorkspaceState::Unknown,
    }
}

pub(crate) fn bookmark_relation_to_str(value: &WorkspaceBookmarkRelation) -> &'static str {
    match value {
        WorkspaceBookmarkRelation::None => "none",
        WorkspaceBookmarkRelation::Exact => "exact",
        WorkspaceBookmarkRelation::Above => "above",
    }
}

pub(crate) fn parse_bookmark_relation(value: &str) -> WorkspaceBookmarkRelation {
    match value {
        "exact" => WorkspaceBookmarkRelation::Exact,
        "above" => WorkspaceBookmarkRelation::Above,
        _ => WorkspaceBookmarkRelation::None,
    }
}

pub(crate) fn optional_agent_attention_to_value(
    value: &Option<AgentAttentionState>,
) -> Result<Value, turso::Error> {
    Ok(match value {
        Some(AgentAttentionState::IdleSeen) => Value::Text("idle-seen".to_owned()),
        Some(AgentAttentionState::Thinking) => Value::Text("thinking".to_owned()),
        Some(AgentAttentionState::IdleUnseen) => Value::Text("idle-unseen".to_owned()),
        None => Value::Null,
    })
}

pub(crate) fn parse_optional_agent_attention(
    value: Value,
) -> Result<Option<AgentAttentionState>, StoreError> {
    match value {
        Value::Null => Ok(None),
        Value::Text(value) => Ok(match value.as_str() {
            "idle-seen" => Some(AgentAttentionState::IdleSeen),
            "thinking" => Some(AgentAttentionState::Thinking),
            "idle-unseen" => Some(AgentAttentionState::IdleUnseen),
            _ => None,
        }),
        value => Err(StoreError::UnexpectedValue {
            column: "agent_attention_state",
            value,
        }),
    }
}

pub(crate) fn terminal_kind_to_str(value: &TerminalKind) -> &'static str {
    match value {
        TerminalKind::Shell => "shell",
        TerminalKind::Codex => "codex",
        TerminalKind::Pi => "pi",
        TerminalKind::Nvim => "nvim",
        TerminalKind::Jjui => "jjui",
    }
}

pub(crate) fn parse_terminal_kind(value: &str) -> TerminalKind {
    match value {
        "codex" => TerminalKind::Codex,
        "pi" => TerminalKind::Pi,
        "nvim" => TerminalKind::Nvim,
        "jjui" => TerminalKind::Jjui,
        _ => TerminalKind::Shell,
    }
}

pub(crate) fn session_state_to_str(value: &SessionState) -> &'static str {
    match value {
        SessionState::Live => "live",
        SessionState::Stopped => "stopped",
        SessionState::Missing => "missing",
    }
}

pub(crate) fn parse_session_state(value: &str) -> SessionState {
    match value {
        "live" => SessionState::Live,
        "missing" => SessionState::Missing,
        _ => SessionState::Stopped,
    }
}
