use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use octty_core::TerminalExitBehavior;
use tokio::process::Command;

use crate::{TerminalError, TerminalSessionSpec, TmuxLaunch, TmuxSessionActivity};

const OCTTY_TMUX_CONFIG: &str = r#"# Octty owns the outer UI chrome, so tmux should stay invisible and inert.
set -g prefix None
set -g prefix2 None
set -g status off
set -g pane-border-status off
set -g mouse off
unbind-key -a
unbind-key -a -T root
"#;

static TMUX_CONFIG_SOURCED: OnceLock<()> = OnceLock::new();

pub fn build_tmux_launch(spec: &TerminalSessionSpec) -> TmuxLaunch {
    let socket_name = tmux_socket_name();
    let session_name = stable_tmux_session_name(spec);
    let command = terminal_command(spec);
    let mut args = tmux_command_prefix_for_socket(&socket_name);
    args.extend([
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
    ]);
    if let Some(command) = command {
        args.push(command);
    }
    append_tmux_exit_behavior(&mut args, spec, &session_name);

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

pub fn build_tmux_pty_launch(spec: &TerminalSessionSpec) -> TmuxLaunch {
    let socket_name = tmux_socket_name();
    let session_name = stable_tmux_session_name(spec);
    let command = terminal_command(spec);
    let mut args = tmux_command_prefix_for_socket(&socket_name);
    args.extend([
        "new-session".to_owned(),
        "-A".to_owned(),
        "-s".to_owned(),
        session_name.clone(),
        "-c".to_owned(),
        spec.cwd.clone(),
    ]);
    if let Some(command) = command {
        args.push(command);
    }
    append_tmux_exit_behavior(&mut args, spec, &session_name);

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
    capture_tmux_pane_by_session(&session_name).await
}

pub async fn capture_tmux_pane_by_session(session_name: &str) -> Result<String, TerminalError> {
    let mut args = tmux_command_prefix();
    args.extend([
        "capture-pane".to_owned(),
        "-p".to_owned(),
        "-t".to_owned(),
        session_name.to_owned(),
    ]);
    let output = tmux_output(&args).await?;
    Ok(String::from_utf8_lossy(&output).to_string())
}

pub async fn tmux_session_activity(
    session_name: &str,
) -> Result<Option<TmuxSessionActivity>, TerminalError> {
    let mut args = tmux_command_prefix();
    args.extend([
        "display-message".to_owned(),
        "-p".to_owned(),
        "-t".to_owned(),
        session_name.to_owned(),
        "#{session_name}\t#{session_activity}\t#{window_activity}\t#{window_activity_flag}\t#{pane_dead}\t#{pane_dead_status}\t#{pane_current_command}".to_owned(),
    ]);
    match tmux_output(&args).await {
        Ok(output) => Ok(parse_tmux_session_activity(&String::from_utf8_lossy(
            &output,
        ))),
        Err(TerminalError::Tmux(message)) if tmux_missing_target(&message) => Ok(None),
        Err(error) => Err(error),
    }
}

pub async fn send_tmux_text(spec: &TerminalSessionSpec, text: &str) -> Result<(), TerminalError> {
    let session_name = ensure_tmux_session(spec).await?;
    send_tmux_text_to_session(&session_name, text).await
}

pub async fn send_tmux_text_to_session(
    session_name: &str,
    text: &str,
) -> Result<(), TerminalError> {
    let mut args = tmux_command_prefix();
    args.extend([
        "send-keys".to_owned(),
        "-t".to_owned(),
        session_name.to_owned(),
        "-l".to_owned(),
        text.to_owned(),
    ]);
    run_tmux(&args).await
}

pub async fn send_tmux_enter(spec: &TerminalSessionSpec) -> Result<(), TerminalError> {
    send_tmux_keys(spec, &["Enter"]).await
}

pub async fn send_tmux_keys(
    spec: &TerminalSessionSpec,
    keys: &[&str],
) -> Result<(), TerminalError> {
    let session_name = ensure_tmux_session(spec).await?;
    send_tmux_keys_to_session(&session_name, keys).await
}

pub async fn send_tmux_keys_to_session(
    session_name: &str,
    keys: &[&str],
) -> Result<(), TerminalError> {
    let mut args = tmux_command_prefix();
    args.extend([
        "send-keys".to_owned(),
        "-t".to_owned(),
        session_name.to_owned(),
    ]);
    args.extend(keys.iter().map(|key| (*key).to_owned()));
    run_tmux(&args).await
}

pub async fn resize_tmux_session(
    spec: &TerminalSessionSpec,
    cols: u16,
    rows: u16,
) -> Result<(), TerminalError> {
    let session_name = ensure_tmux_session(spec).await?;
    let mut args = tmux_command_prefix();
    args.extend([
        "resize-window".to_owned(),
        "-t".to_owned(),
        session_name,
        "-x".to_owned(),
        cols.to_string(),
        "-y".to_owned(),
        rows.to_string(),
    ]);
    run_tmux(&args).await
}

pub async fn kill_tmux_session(session_name: &str) -> Result<(), TerminalError> {
    let mut args = tmux_command_prefix();
    args.extend([
        "kill-session".to_owned(),
        "-t".to_owned(),
        session_name.to_owned(),
    ]);
    run_tmux(&args).await
}

async fn tmux_has_session(launch: &TmuxLaunch) -> Result<bool, TerminalError> {
    let mut args = tmux_command_prefix_for_socket(&launch.socket_name);
    args.extend([
        "has-session".to_owned(),
        "-t".to_owned(),
        launch.session_name.clone(),
    ]);
    ensure_tmux_config()?;
    let output = Command::new("tmux")
        .args(&args)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .await?;
    Ok(output.status.success())
}

async fn run_tmux(args: &[String]) -> Result<(), TerminalError> {
    ensure_tmux_config()?;
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
    ensure_tmux_config()?;
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

fn tmux_missing_target(message: &str) -> bool {
    message.contains("can't find") || message.contains("can't establish current")
}

fn parse_tmux_session_activity(output: &str) -> Option<TmuxSessionActivity> {
    let mut fields = output.trim_end_matches(['\r', '\n']).split('\t');
    let session_name = fields.next()?.to_owned();
    let session_activity_at_s = parse_optional_tmux_i64(fields.next()?);
    let window_activity_at_s = parse_optional_tmux_i64(fields.next()?);
    let window_activity_flag = parse_tmux_bool(fields.next()?);
    let pane_dead = parse_tmux_bool(fields.next()?);
    let pane_dead_status = parse_optional_tmux_i64(fields.next()?);
    let pane_current_command = fields
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    Some(TmuxSessionActivity {
        session_name,
        session_activity_at_s,
        window_activity_at_s,
        window_activity_flag,
        pane_dead,
        pane_dead_status,
        pane_current_command,
    })
}

fn parse_optional_tmux_i64(value: &str) -> Option<i64> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        value.parse().ok()
    }
}

fn parse_tmux_bool(value: &str) -> bool {
    value.trim() == "1"
}

pub fn terminal_command(spec: &TerminalSessionSpec) -> Option<String> {
    let command = spec.command.trim();
    if command.is_empty() {
        return None;
    }

    let line = shell_command_line(command, &spec.command_parameters);
    Some(match spec.on_exit {
        TerminalExitBehavior::RestartAuto => restart_auto_shell_command(&line),
        TerminalExitBehavior::RestartManually | TerminalExitBehavior::Close => line,
    })
}

fn append_tmux_exit_behavior(
    args: &mut Vec<String>,
    spec: &TerminalSessionSpec,
    session_name: &str,
) {
    if spec.command.trim().is_empty() || spec.on_exit != TerminalExitBehavior::RestartManually {
        return;
    }
    args.extend([
        ";".to_owned(),
        "set-option".to_owned(),
        "-t".to_owned(),
        session_name.to_owned(),
        "remain-on-exit".to_owned(),
        "on".to_owned(),
    ]);
}

fn shell_command_line(command: &str, args: &[String]) -> String {
    std::iter::once(shell_quote(command))
        .chain(args.iter().map(|arg| shell_quote(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn restart_auto_shell_command(command_line: &str) -> String {
    format!(
        "while true; do {command_line}; status=$?; printf '\n[octty] exited with status %s; restarting in 1s...\n' \"$status\"; sleep 1; done"
    )
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_owned();
    }
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':' | '='))
    {
        return value.to_owned();
    }
    format!("'{}'", value.replace("'", "'\\''"))
}

pub fn tmux_socket_name() -> String {
    std::env::var("OCTTY_RS_TMUX_SOCKET").unwrap_or_else(|_| "octty-rs".to_owned())
}

fn tmux_command_prefix() -> Vec<String> {
    tmux_command_prefix_for_socket(&tmux_socket_name())
}

fn tmux_command_prefix_for_socket(socket_name: &str) -> Vec<String> {
    vec![
        "-L".to_owned(),
        socket_name.to_owned(),
        "-f".to_owned(),
        tmux_config_path().to_string_lossy().to_string(),
    ]
}

pub(crate) fn ensure_tmux_config() -> Result<PathBuf, TerminalError> {
    let config_path = tmux_config_path();
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&config_path, OCTTY_TMUX_CONFIG)?;
    TMUX_CONFIG_SOURCED.get_or_init(|| {
        let _ = source_tmux_config_for_existing_server(&config_path);
    });
    Ok(config_path)
}

fn source_tmux_config_for_existing_server(config_path: &Path) -> Result<(), TerminalError> {
    let output = std::process::Command::new("tmux")
        .args([
            "-L",
            tmux_socket_name().as_str(),
            "source-file",
            config_path.to_string_lossy().as_ref(),
        ])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(TerminalError::Tmux(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ))
    }
}

fn tmux_config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("OCTTY_RS_TMUX_CONFIG_PATH") {
        return PathBuf::from(path);
    }
    tmux_cache_dir().join("tmux.conf")
}

fn tmux_cache_dir() -> PathBuf {
    env_path("OCTTY_RS_CACHE_PATH")
        .or_else(|| env_path("OCTTY_CACHE_PATH"))
        .unwrap_or_else(|| {
            home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".cache")
                .join("octty")
        })
}

fn env_path(key: &str) -> Option<PathBuf> {
    let value = std::env::var_os(key)?;
    let path = Path::new(&value);
    (!path.as_os_str().is_empty()).then(|| path.to_path_buf())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
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
    use octty_core::TerminalKind;

    use super::*;

    #[test]
    fn tmux_launch_uses_dedicated_socket_and_strips_tmux_env() {
        let spec = TerminalSessionSpec {
            workspace_id: "workspace-1".to_owned(),
            pane_id: "pane-1".to_owned(),
            kind: TerminalKind::Codex,
            cwd: "/tmp/repo".to_owned(),
            command: "codex".to_owned(),
            command_parameters: Vec::new(),
            on_exit: TerminalExitBehavior::Close,
            cols: 120,
            rows: 40,
        };

        let launch = build_tmux_launch(&spec);

        assert_eq!(launch.socket_name, tmux_socket_name());
        assert!(launch.args.starts_with(&[
            "-L".to_owned(),
            tmux_socket_name(),
            "-f".to_owned(),
            tmux_config_path().to_string_lossy().to_string(),
            "new-session".to_owned()
        ]));
        assert!(launch.args.contains(&"-d".to_owned()));
        assert!(!launch.session_name.contains(':'));
        assert!(launch.args.contains(&"codex".to_owned()));
        assert!(launch.clean_env.contains_key("TMUX"));
        assert!(launch.clean_env.contains_key("TMUX_PANE"));
    }

    #[test]
    fn tmux_pty_launch_uses_octty_config() {
        let spec = TerminalSessionSpec {
            workspace_id: "workspace-1".to_owned(),
            pane_id: "pane-1".to_owned(),
            kind: TerminalKind::Shell,
            cwd: "/tmp/repo".to_owned(),
            command: "codex".to_owned(),
            command_parameters: Vec::new(),
            on_exit: TerminalExitBehavior::Close,
            cols: 80,
            rows: 24,
        };

        let launch = build_tmux_pty_launch(&spec);

        assert!(launch.args.starts_with(&[
            "-L".to_owned(),
            tmux_socket_name(),
            "-f".to_owned(),
            tmux_config_path().to_string_lossy().to_string(),
            "new-session".to_owned()
        ]));
        assert!(launch.args.contains(&"-A".to_owned()));
    }

    #[test]
    fn tmux_launch_uses_configured_command_parameters_and_exit_behavior() {
        let spec = TerminalSessionSpec {
            workspace_id: "workspace-1".to_owned(),
            pane_id: "pane-1".to_owned(),
            kind: TerminalKind::Codex,
            cwd: "/tmp/repo".to_owned(),
            command: "codex".to_owned(),
            command_parameters: vec!["--dangerously-bypass-approvals-and-sandbox".to_owned()],
            on_exit: TerminalExitBehavior::RestartManually,
            cols: 120,
            rows: 40,
        };

        let launch = build_tmux_launch(&spec);

        assert!(
            launch
                .args
                .contains(&"codex --dangerously-bypass-approvals-and-sandbox".to_owned())
        );
        assert!(launch.args.windows(6).any(|window| {
            window[0] == ";"
                && window[1] == "set-option"
                && window[2] == "-t"
                && window[3] == launch.session_name.as_str()
                && window[4] == "remain-on-exit"
                && window[5] == "on"
        }));
    }

    #[test]
    fn tmux_session_names_are_stable_and_target_safe() {
        let spec = TerminalSessionSpec {
            workspace_id: "workspace:1".to_owned(),
            pane_id: "pane:1".to_owned(),
            kind: TerminalKind::Shell,
            cwd: "/tmp/repo".to_owned(),
            command: "codex".to_owned(),
            command_parameters: Vec::new(),
            on_exit: TerminalExitBehavior::Close,
            cols: 80,
            rows: 24,
        };

        assert_eq!(
            stable_tmux_session_name(&spec),
            stable_tmux_session_name(&spec)
        );
        assert!(!stable_tmux_session_name(&spec).contains(':'));
    }

    #[test]
    fn parses_tmux_session_activity() {
        assert_eq!(
            parse_tmux_session_activity("octty-123\t1765871234\t1765871235\t1\t0\t\tbash\n"),
            Some(TmuxSessionActivity {
                session_name: "octty-123".to_owned(),
                session_activity_at_s: Some(1765871234),
                window_activity_at_s: Some(1765871235),
                window_activity_flag: true,
                pane_dead: false,
                pane_dead_status: None,
                pane_current_command: Some("bash".to_owned()),
            })
        );
    }

    #[test]
    fn tmux_missing_target_matches_common_errors() {
        assert!(tmux_missing_target("can't find session: octty-123"));
        assert!(tmux_missing_target("can't establish current session"));
        assert!(!tmux_missing_target("permission denied"));
    }
}
