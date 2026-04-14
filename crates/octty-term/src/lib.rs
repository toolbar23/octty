use std::collections::BTreeMap;

use octty_core::TerminalKind;
use thiserror::Error;
use tokio::process::Command;

#[cfg(feature = "ghostty-vt")]
pub mod ghostty_vt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSessionSpec {
    pub workspace_id: String,
    pub pane_id: String,
    pub kind: TerminalKind,
    pub cwd: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TmuxLaunch {
    pub socket_name: String,
    pub session_name: String,
    pub args: Vec<String>,
    pub clean_env: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
pub enum TerminalError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("tmux command failed: {0}")]
    Tmux(String),
}

pub fn build_tmux_launch(spec: &TerminalSessionSpec) -> TmuxLaunch {
    let socket_name = "octty-rs".to_owned();
    let session_name = format!("{}:{}", spec.workspace_id, spec.pane_id);
    let command = terminal_command(&spec.kind);
    let mut args = vec![
        "-L".to_owned(),
        socket_name.clone(),
        "new-session".to_owned(),
        "-A".to_owned(),
        "-s".to_owned(),
        session_name.clone(),
        "-c".to_owned(),
        spec.cwd.clone(),
    ];
    if !command.is_empty() {
        args.push(command.to_owned());
    }

    let mut clean_env = BTreeMap::new();
    clean_env.insert("TMUX".to_owned(), String::new());
    clean_env.insert("TMUX_PANE".to_owned(), String::new());

    TmuxLaunch {
        socket_name,
        session_name,
        args,
        clean_env,
    }
}

pub async fn ensure_tmux_session(spec: &TerminalSessionSpec) -> Result<(), TerminalError> {
    let launch = build_tmux_launch(spec);
    let output = Command::new("tmux")
        .args(&launch.args)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .await?;
    if output.status.success() {
        Ok(())
    } else {
        Err(TerminalError::Tmux(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ))
    }
}

pub fn terminal_command(kind: &TerminalKind) -> &'static str {
    match kind {
        TerminalKind::Shell => "",
        TerminalKind::Codex => "codex",
        TerminalKind::Pi => "pi",
        TerminalKind::Nvim => "nvim",
        TerminalKind::Jjui => "jjui",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmux_launch_uses_dedicated_socket_and_strips_tmux_env() {
        let spec = TerminalSessionSpec {
            workspace_id: "workspace-1".to_owned(),
            pane_id: "pane-1".to_owned(),
            kind: TerminalKind::Codex,
            cwd: "/tmp/repo".to_owned(),
            cols: 120,
            rows: 40,
        };

        let launch = build_tmux_launch(&spec);

        assert_eq!(launch.socket_name, "octty-rs");
        assert!(launch.args.starts_with(&[
            "-L".to_owned(),
            "octty-rs".to_owned(),
            "new-session".to_owned()
        ]));
        assert!(launch.args.contains(&"codex".to_owned()));
        assert!(launch.clean_env.contains_key("TMUX"));
        assert!(launch.clean_env.contains_key("TMUX_PANE"));
    }
}
