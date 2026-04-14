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
    let socket_name = tmux_socket_name();
    let session_name = stable_tmux_session_name(spec);
    let command = terminal_command(&spec.kind);
    let mut args = vec![
        "-L".to_owned(),
        socket_name.clone(),
        "new-session".to_owned(),
        "-d".to_owned(),
        "-s".to_owned(),
        session_name.clone(),
        "-x".to_owned(),
        spec.cols.to_string(),
        "-y".to_owned(),
        spec.rows.to_string(),
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

pub async fn ensure_tmux_session(spec: &TerminalSessionSpec) -> Result<String, TerminalError> {
    let launch = build_tmux_launch(spec);
    if tmux_has_session(&launch).await? {
        return Ok(launch.session_name);
    }
    run_tmux(&launch.args).await?;
    Ok(launch.session_name)
}

pub async fn capture_tmux_pane(spec: &TerminalSessionSpec) -> Result<String, TerminalError> {
    let session_name = ensure_tmux_session(spec).await?;
    let args = vec![
        "-L".to_owned(),
        tmux_socket_name(),
        "capture-pane".to_owned(),
        "-p".to_owned(),
        "-t".to_owned(),
        format!("{session_name}:0.0"),
    ];
    let output = tmux_output(&args).await?;
    Ok(String::from_utf8_lossy(&output).to_string())
}

pub async fn kill_tmux_session(session_name: &str) -> Result<(), TerminalError> {
    let args = vec![
        "-L".to_owned(),
        tmux_socket_name(),
        "kill-session".to_owned(),
        "-t".to_owned(),
        session_name.to_owned(),
    ];
    run_tmux(&args).await
}

async fn tmux_has_session(launch: &TmuxLaunch) -> Result<bool, TerminalError> {
    let args = vec![
        "-L".to_owned(),
        launch.socket_name.clone(),
        "has-session".to_owned(),
        "-t".to_owned(),
        launch.session_name.clone(),
    ];
    let output = Command::new("tmux")
        .args(&args)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .await?;
    Ok(output.status.success())
}

async fn run_tmux(args: &[String]) -> Result<(), TerminalError> {
    let output = Command::new("tmux")
        .args(args)
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

async fn tmux_output(args: &[String]) -> Result<Vec<u8>, TerminalError> {
    let output = Command::new("tmux")
        .args(args)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .await?;
    if output.status.success() {
        Ok(output.stdout)
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

pub fn tmux_socket_name() -> String {
    std::env::var("OCTTY_RS_TMUX_SOCKET").unwrap_or_else(|_| "octty-rs".to_owned())
}

pub fn stable_tmux_session_name(spec: &TerminalSessionSpec) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in spec
        .workspace_id
        .bytes()
        .chain([0])
        .chain(spec.pane_id.bytes())
    {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("octty-{hash:016x}")
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

        assert_eq!(launch.socket_name, tmux_socket_name());
        assert!(launch.args.starts_with(&[
            "-L".to_owned(),
            tmux_socket_name(),
            "new-session".to_owned()
        ]));
        assert!(launch.args.contains(&"-d".to_owned()));
        assert!(!launch.session_name.contains(':'));
        assert!(launch.args.contains(&"codex".to_owned()));
        assert!(launch.clean_env.contains_key("TMUX"));
        assert!(launch.clean_env.contains_key("TMUX_PANE"));
    }

    #[test]
    fn tmux_session_names_are_stable_and_target_safe() {
        let spec = TerminalSessionSpec {
            workspace_id: "workspace:1".to_owned(),
            pane_id: "pane:1".to_owned(),
            kind: TerminalKind::Shell,
            cwd: "/tmp/repo".to_owned(),
            cols: 80,
            rows: 24,
        };

        assert_eq!(
            stable_tmux_session_name(&spec),
            stable_tmux_session_name(&spec)
        );
        assert!(!stable_tmux_session_name(&spec).contains(':'));
    }
}
