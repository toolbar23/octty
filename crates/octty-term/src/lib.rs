use std::collections::BTreeMap;

use octty_core::{InnerSessionHandler, TerminalExitBehavior, TerminalKind};
use thiserror::Error;

#[cfg(feature = "ghostty-vt")]
pub mod ghostty_vt;
#[cfg(feature = "ghostty-vt")]
pub mod live;

mod retach;
pub use retach::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSessionSpec {
    pub workspace_id: String,
    pub pane_id: String,
    pub kind: TerminalKind,
    pub cwd: String,
    pub command: String,
    pub command_parameters: Vec<String>,
    pub inner_session_handler: InnerSessionHandler,
    pub inner_session_id: Option<String>,
    pub on_exit: TerminalExitBehavior,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetachLaunch {
    pub program: String,
    pub session_name: String,
    pub args: Vec<String>,
    pub clean_env: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetachSessionActivity {
    pub session_name: String,
    pub session_activity_at_s: Option<i64>,
    pub window_activity_at_s: Option<i64>,
    pub window_activity_flag: bool,
    pub pane_dead: bool,
    pub pane_dead_status: Option<i64>,
    pub pane_current_command: Option<String>,
}

#[derive(Debug, Error)]
pub enum TerminalError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("retach command failed: {0}")]
    Retach(String),
    #[error("pty error: {0}")]
    Pty(String),
    #[error("terminal renderer error: {0}")]
    Renderer(String),
}
