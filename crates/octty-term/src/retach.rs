use std::{collections::BTreeMap, path::PathBuf, process::Stdio, time::Duration};

use bincode::Options;
use octty_core::{TerminalExitBehavior, TerminalKind};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

use crate::{RetachLaunch, RetachSessionActivity, TerminalError, TerminalSessionSpec};

const DEFAULT_RETACH_HISTORY: usize = 10_000;
const SERVER_POLL_INTERVAL: Duration = Duration::from_millis(100);
const SERVER_POLL_MAX: usize = 50;
const CONTROL_READ_TIMEOUT: Duration = Duration::from_millis(250);
const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;
const READ_BUF_SIZE: usize = 64 * 1024;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
enum ConnectMode {
    CreateOrAttach,
    CreateOnly,
    AttachOnly,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
enum ClientMsg {
    Input(Vec<u8>),
    Resize {
        cols: u16,
        rows: u16,
    },
    Detach,
    ListSessions,
    Connect {
        name: String,
        history: usize,
        cols: u16,
        rows: u16,
        mode: ConnectMode,
    },
    KillSession {
        name: String,
    },
    RefreshScreen,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RetachSessionInfo {
    pub name: String,
    pub pid: u32,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
enum ServerMsg {
    ScreenUpdate(Vec<u8>),
    History(Vec<Vec<u8>>),
    SessionList(Vec<RetachSessionInfo>),
    SessionEnded,
    Error(String),
    Connected { name: String, new_session: bool },
    SessionKilled { name: String },
    Passthrough(Vec<u8>),
}

pub fn build_retach_launch(spec: &TerminalSessionSpec) -> RetachLaunch {
    let session_name = stable_retach_session_name(spec);
    RetachLaunch {
        program: retach_binary(),
        session_name: session_name.clone(),
        args: vec![
            "open".to_owned(),
            session_name,
            "--history".to_owned(),
            retach_history().to_string(),
        ],
        clean_env: BTreeMap::new(),
    }
}

pub fn build_retach_pty_launch(spec: &TerminalSessionSpec) -> RetachLaunch {
    build_retach_launch(spec)
}

pub async fn ensure_retach_session(spec: &TerminalSessionSpec) -> Result<String, TerminalError> {
    let launch = build_retach_launch(spec);
    if retach_session_exists(&launch.session_name).await? {
        return Ok(launch.session_name);
    }
    let (mut stream, _) = connect_retach_session(
        &launch.session_name,
        spec.cols,
        spec.rows,
        ConnectMode::CreateOrAttach,
    )
    .await?;
    let _ = write_message(&mut stream, &ClientMsg::Detach).await;
    Ok(launch.session_name)
}

pub async fn capture_retach_pane(spec: &TerminalSessionSpec) -> Result<String, TerminalError> {
    let session_name = ensure_retach_session(spec).await?;
    capture_retach_pane_by_session_with_size(&session_name, spec.cols, spec.rows).await
}

async fn capture_retach_pane_by_session_with_size(
    session_name: &str,
    cols: u16,
    rows: u16,
) -> Result<String, TerminalError> {
    let (mut stream, mut frames) =
        connect_retach_session(session_name, cols, rows, ConnectMode::AttachOnly).await?;
    let mut screen = String::new();
    let mut saw_screen = false;
    let deadline = tokio::time::Instant::now() + CONTROL_READ_TIMEOUT;

    loop {
        while let Some(msg) = frames.decode_next::<ServerMsg>()? {
            match msg {
                ServerMsg::History(lines) => {
                    for line in lines {
                        screen.push_str(&ansi_visible_text(&line));
                        screen.push('\n');
                    }
                }
                ServerMsg::ScreenUpdate(bytes) => {
                    screen.push_str(&ansi_visible_text(&bytes));
                    saw_screen = true;
                }
                ServerMsg::Error(message) => return Err(TerminalError::Retach(message)),
                ServerMsg::SessionEnded => return Ok(screen),
                _ => {}
            }
        }
        if saw_screen || tokio::time::Instant::now() >= deadline {
            break;
        }
        match tokio::time::timeout_at(deadline, frames.fill_from(&mut stream)).await {
            Ok(Ok(true)) => {}
            Ok(Ok(false)) => break,
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => break,
        }
    }

    let _ = write_message(&mut stream, &ClientMsg::Detach).await;
    Ok(screen)
}

pub async fn capture_retach_pane_by_session(session_name: &str) -> Result<String, TerminalError> {
    capture_retach_pane_by_session_with_size(session_name, 120, 40).await
}

pub async fn retach_session_activity(
    session_name: &str,
) -> Result<Option<RetachSessionActivity>, TerminalError> {
    let sessions = retach_sessions().await?;
    Ok(sessions
        .into_iter()
        .find(|session| session.name == session_name)
        .map(|session| RetachSessionActivity {
            session_name: session.name,
            session_activity_at_s: None,
            window_activity_at_s: None,
            window_activity_flag: false,
            pane_dead: false,
            pane_dead_status: None,
            pane_current_command: Some(format!("pid:{}", session.pid)),
        }))
}

pub async fn send_retach_text(spec: &TerminalSessionSpec, text: &str) -> Result<(), TerminalError> {
    let session_name = ensure_retach_session(spec).await?;
    send_retach_text_to_session(&session_name, text).await
}

pub async fn send_retach_text_to_session(
    session_name: &str,
    text: &str,
) -> Result<(), TerminalError> {
    send_retach_bytes_to_session(session_name, text.as_bytes()).await
}

pub async fn send_retach_enter(spec: &TerminalSessionSpec) -> Result<(), TerminalError> {
    send_retach_keys(spec, &["Enter"]).await
}

pub async fn send_retach_keys(
    spec: &TerminalSessionSpec,
    keys: &[&str],
) -> Result<(), TerminalError> {
    let session_name = ensure_retach_session(spec).await?;
    send_retach_keys_to_session(&session_name, keys).await
}

pub async fn send_retach_keys_to_session(
    session_name: &str,
    keys: &[&str],
) -> Result<(), TerminalError> {
    let mut bytes = Vec::new();
    for key in keys {
        bytes.extend(retach_key_bytes(key));
    }
    send_retach_bytes_to_session(session_name, &bytes).await
}

pub async fn send_retach_bytes_to_session(
    session_name: &str,
    bytes: &[u8],
) -> Result<(), TerminalError> {
    let (mut stream, _) =
        connect_retach_session(session_name, 120, 40, ConnectMode::AttachOnly).await?;
    write_message(&mut stream, &ClientMsg::Input(bytes.to_vec())).await?;
    write_message(&mut stream, &ClientMsg::Detach).await?;
    Ok(())
}

pub async fn resize_retach_session(
    spec: &TerminalSessionSpec,
    cols: u16,
    rows: u16,
) -> Result<(), TerminalError> {
    let session_name = ensure_retach_session(spec).await?;
    let (mut stream, _) =
        connect_retach_session(&session_name, cols, rows, ConnectMode::AttachOnly).await?;
    write_message(&mut stream, &ClientMsg::Resize { cols, rows }).await?;
    write_message(&mut stream, &ClientMsg::Detach).await?;
    Ok(())
}

pub async fn kill_retach_session(session_name: &str) -> Result<(), TerminalError> {
    ensure_retach_server_running().await?;
    let mut stream = UnixStream::connect(retach_socket_path()?).await?;
    write_message(
        &mut stream,
        &ClientMsg::KillSession {
            name: session_name.to_owned(),
        },
    )
    .await?;
    match read_one_message::<ServerMsg>(&mut stream).await? {
        ServerMsg::SessionKilled { .. } => Ok(()),
        ServerMsg::Error(message) if retach_missing_target(&message) => Ok(()),
        ServerMsg::Error(message) => Err(TerminalError::Retach(message)),
        _ => Err(TerminalError::Retach(
            "unexpected response from retach server".to_owned(),
        )),
    }
}

pub fn retach_binary() -> String {
    std::env::var("OCTTY_RETACH_BIN").unwrap_or_else(|_| "retach".to_owned())
}

pub fn terminal_kind_command(kind: &TerminalKind) -> &'static str {
    match kind {
        TerminalKind::Shell => "",
        TerminalKind::Codex => "codex",
        TerminalKind::Pi => "pi",
        TerminalKind::Nvim => "nvim",
        TerminalKind::Jjui => "jjui",
    }
}

pub fn terminal_command(spec: &TerminalSessionSpec) -> Option<String> {
    let command = spec.command.trim();
    let command = if command.is_empty() {
        terminal_kind_command(&spec.kind)
    } else {
        command
    };
    if command.is_empty() {
        return None;
    }

    let line = shell_command_line(command, &spec.command_parameters);
    Some(match spec.on_exit {
        TerminalExitBehavior::RestartAuto => restart_auto_shell_command(&line),
        TerminalExitBehavior::RestartManually | TerminalExitBehavior::Close => line,
    })
}

pub fn retach_startup_command(spec: &TerminalSessionSpec) -> Option<Vec<u8>> {
    let cwd = shell_single_quote(&spec.cwd);
    let Some(command) = terminal_command(spec) else {
        return Some(format!("cd -- {cwd}\r").into_bytes());
    };
    let exec = if spec.on_exit == TerminalExitBehavior::Close {
        "exec "
    } else {
        ""
    };
    Some(format!("cd -- {cwd} && {exec}{command}\r").into_bytes())
}

pub fn stable_retach_session_name(spec: &TerminalSessionSpec) -> String {
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

pub fn ensure_retach_config() -> Result<PathBuf, TerminalError> {
    retach_socket_path()
}

pub async fn retach_sessions() -> Result<Vec<RetachSessionInfo>, TerminalError> {
    ensure_retach_server_running().await?;
    let mut stream = UnixStream::connect(retach_socket_path()?).await?;
    write_message(&mut stream, &ClientMsg::ListSessions).await?;
    match read_one_message::<ServerMsg>(&mut stream).await? {
        ServerMsg::SessionList(sessions) => Ok(sessions),
        ServerMsg::Error(message) => Err(TerminalError::Retach(message)),
        _ => Err(TerminalError::Retach(
            "unexpected response from retach server".to_owned(),
        )),
    }
}

async fn retach_session_exists(session_name: &str) -> Result<bool, TerminalError> {
    Ok(retach_sessions()
        .await?
        .into_iter()
        .any(|session| session.name == session_name))
}

async fn connect_retach_session(
    session_name: &str,
    cols: u16,
    rows: u16,
    mode: ConnectMode,
) -> Result<(UnixStream, FrameReader), TerminalError> {
    ensure_retach_server_running().await?;
    let mut stream = UnixStream::connect(retach_socket_path()?).await?;
    write_message(
        &mut stream,
        &ClientMsg::Connect {
            name: session_name.to_owned(),
            history: retach_history(),
            cols,
            rows,
            mode,
        },
    )
    .await?;

    let mut frames = FrameReader::new();
    loop {
        if !frames.fill_from(&mut stream).await? {
            return Err(TerminalError::Retach(
                "retach server closed the connection".to_owned(),
            ));
        }
        if let Some(msg) = frames.decode_next::<ServerMsg>()? {
            match msg {
                ServerMsg::Connected { .. } => return Ok((stream, frames)),
                ServerMsg::Error(message) => return Err(TerminalError::Retach(message)),
                _ => {
                    return Err(TerminalError::Retach(
                        "unexpected response from retach server".to_owned(),
                    ));
                }
            }
        }
    }
}

async fn ensure_retach_server_running() -> Result<(), TerminalError> {
    let path = retach_socket_path()?;
    if UnixStream::connect(&path).await.is_ok() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::process::Command::new(retach_binary())
        .arg("server")
        .env_remove("GHOSTTY_RESOURCES_DIR")
        .env_remove("GHOSTTY_SHELL_INTEGRATION")
        .env_remove("TERM_PROGRAM")
        .env_remove("TERM_PROGRAM_VERSION")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            TerminalError::Retach(format!("failed to start retach server: {error}"))
        })?;

    for _ in 0..SERVER_POLL_MAX {
        tokio::time::sleep(SERVER_POLL_INTERVAL).await;
        if UnixStream::connect(&path).await.is_ok() {
            return Ok(());
        }
    }

    Err(TerminalError::Retach(
        "timed out waiting for retach server".to_owned(),
    ))
}

fn retach_socket_path() -> Result<PathBuf, TerminalError> {
    let base = if let Some(runtime_dir) = env_path("XDG_RUNTIME_DIR") {
        runtime_dir
    } else {
        PathBuf::from("/tmp").join(format!("retach-{}", current_uid()))
    };
    Ok(base.join("retach").join("retach.sock"))
}

fn env_path(key: &str) -> Option<PathBuf> {
    let value = std::env::var_os(key)?;
    let path = std::path::Path::new(&value);
    (!path.as_os_str().is_empty()).then(|| path.to_path_buf())
}

fn current_uid() -> String {
    std::env::var("UID")
        .ok()
        .filter(|uid| !uid.is_empty())
        .or_else(|| {
            std::process::Command::new("id")
                .arg("-u")
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        })
        .filter(|uid| !uid.is_empty())
        .unwrap_or_else(|| "0".to_owned())
}

fn retach_history() -> usize {
    std::env::var("OCTTY_RETACH_HISTORY")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_RETACH_HISTORY)
}

fn shell_single_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
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
    shell_single_quote(value)
}

fn retach_missing_target(message: &str) -> bool {
    message.contains("not found") || message.contains("doesn't exist")
}

fn retach_key_bytes(key: &str) -> Vec<u8> {
    match key {
        "Enter" => b"\r".to_vec(),
        "BSpace" => vec![0x7f],
        "Delete" => b"\x1b[3~".to_vec(),
        "Tab" => b"\t".to_vec(),
        "Escape" => b"\x1b".to_vec(),
        "Left" => b"\x1b[D".to_vec(),
        "Right" => b"\x1b[C".to_vec(),
        "Up" => b"\x1b[A".to_vec(),
        "Down" => b"\x1b[B".to_vec(),
        "Home" => b"\x1b[H".to_vec(),
        "End" => b"\x1b[F".to_vec(),
        "PageUp" => b"\x1b[5~".to_vec(),
        "PageDown" => b"\x1b[6~".to_vec(),
        "Insert" => b"\x1b[2~".to_vec(),
        "F1" => b"\x1bOP".to_vec(),
        "F2" => b"\x1bOQ".to_vec(),
        "F3" => b"\x1bOR".to_vec(),
        "F4" => b"\x1bOS".to_vec(),
        "F5" => b"\x1b[15~".to_vec(),
        "F6" => b"\x1b[17~".to_vec(),
        "F7" => b"\x1b[18~".to_vec(),
        "F8" => b"\x1b[19~".to_vec(),
        "F9" => b"\x1b[20~".to_vec(),
        "F10" => b"\x1b[21~".to_vec(),
        "F11" => b"\x1b[23~".to_vec(),
        "F12" => b"\x1b[24~".to_vec(),
        key if key.starts_with("C-") => key
            .as_bytes()
            .get(2)
            .map(|byte| vec![byte.to_ascii_lowercase() & 0x1f])
            .unwrap_or_default(),
        other => other.as_bytes().to_vec(),
    }
}

fn ansi_visible_text(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            0x1b => {
                index += 1;
                if index < bytes.len() && bytes[index] == b'[' {
                    index += 1;
                    while index < bytes.len() && !(0x40..=0x7e).contains(&bytes[index]) {
                        index += 1;
                    }
                    index += usize::from(index < bytes.len());
                } else if index < bytes.len() && bytes[index] == b']' {
                    index += 1;
                    while index < bytes.len() {
                        if bytes[index] == 0x07 {
                            index += 1;
                            break;
                        }
                        if bytes[index] == 0x1b
                            && index + 1 < bytes.len()
                            && bytes[index + 1] == b'\\'
                        {
                            index += 2;
                            break;
                        }
                        index += 1;
                    }
                } else {
                    index += usize::from(index < bytes.len());
                }
            }
            b'\r' => index += 1,
            b'\n' => {
                out.push('\n');
                index += 1;
            }
            byte if byte >= b' ' || byte == b'\t' => {
                out.push(byte as char);
                index += 1;
            }
            _ => index += 1,
        }
    }
    out
}

async fn write_message(stream: &mut UnixStream, msg: &ClientMsg) -> Result<(), TerminalError> {
    let encoded = encode(msg)?;
    stream.write_all(&encoded).await?;
    Ok(())
}

async fn read_one_message<T: DeserializeOwned>(
    stream: &mut UnixStream,
) -> Result<T, TerminalError> {
    let mut frames = FrameReader::new();
    loop {
        if !frames.fill_from(stream).await? {
            return Err(TerminalError::Retach(
                "retach server closed the connection".to_owned(),
            ));
        }
        if let Some(msg) = frames.decode_next()? {
            return Ok(msg);
        }
    }
}

fn encode(msg: &impl Serialize) -> Result<Vec<u8>, TerminalError> {
    let data = bincode_config()
        .serialize(msg)
        .map_err(|error| TerminalError::Retach(error.to_string()))?;
    let len = u32::try_from(data.len()).map_err(|_| {
        TerminalError::Retach(format!("retach protocol message too large: {}", data.len()))
    })?;
    let mut buf = Vec::with_capacity(4 + data.len());
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&data);
    Ok(buf)
}

fn decode<T: DeserializeOwned>(data: &[u8]) -> Result<T, TerminalError> {
    bincode_config()
        .deserialize(data)
        .map_err(|error| TerminalError::Retach(error.to_string()))
}

fn bincode_config() -> impl Options + Copy {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_FRAME_SIZE as u64)
}

struct FrameReader {
    read_buf: Vec<u8>,
    offset: usize,
    tmp_buf: Vec<u8>,
}

impl FrameReader {
    fn new() -> Self {
        Self {
            read_buf: Vec::new(),
            offset: 0,
            tmp_buf: vec![0; READ_BUF_SIZE],
        }
    }

    async fn fill_from(&mut self, stream: &mut UnixStream) -> Result<bool, TerminalError> {
        if self.offset > 0 {
            self.read_buf.drain(..self.offset);
            self.offset = 0;
        }
        let size = stream.read(&mut self.tmp_buf).await?;
        if size == 0 {
            return Ok(false);
        }
        self.read_buf.extend_from_slice(&self.tmp_buf[..size]);
        if self.read_buf.len() > MAX_FRAME_SIZE * 2 + 8 {
            return Err(TerminalError::Retach(format!(
                "retach protocol frame too large: {} bytes",
                self.read_buf.len()
            )));
        }
        Ok(true)
    }

    fn decode_next<T: DeserializeOwned>(&mut self) -> Result<Option<T>, TerminalError> {
        if self.read_buf.len() < self.offset + 4 {
            return Ok(None);
        }
        let start = self.offset;
        let len = u32::from_be_bytes([
            self.read_buf[start],
            self.read_buf[start + 1],
            self.read_buf[start + 2],
            self.read_buf[start + 3],
        ]) as usize;
        if len > MAX_FRAME_SIZE {
            return Err(TerminalError::Retach(format!(
                "retach protocol frame too large: {len} bytes"
            )));
        }
        if self.read_buf.len() < start + 4 + len {
            return Ok(None);
        }
        let msg = decode(&self.read_buf[start + 4..start + 4 + len])?;
        self.offset = start + 4 + len;
        Ok(Some(msg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retach_launch_uses_open_with_stable_session() {
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

        let launch = build_retach_launch(&spec);

        assert_eq!(launch.program, retach_binary());
        assert_eq!(launch.args[0], "open");
        assert_eq!(launch.args[1], stable_retach_session_name(&spec));
        assert!(launch.args.contains(&"--history".to_owned()));
        assert!(!launch.session_name.contains(':'));
    }

    #[test]
    fn retach_session_names_are_stable_and_target_safe() {
        let spec = TerminalSessionSpec {
            workspace_id: "workspace:1".to_owned(),
            pane_id: "pane:1".to_owned(),
            kind: TerminalKind::Shell,
            cwd: "/tmp/repo".to_owned(),
            command: String::new(),
            command_parameters: Vec::new(),
            on_exit: TerminalExitBehavior::Close,
            cols: 80,
            rows: 24,
        };

        assert_eq!(
            stable_retach_session_name(&spec),
            stable_retach_session_name(&spec)
        );
        assert!(!stable_retach_session_name(&spec).contains(':'));
    }

    #[test]
    fn retach_key_mapping_covers_terminal_controls() {
        assert_eq!(retach_key_bytes("Enter"), b"\r");
        assert_eq!(retach_key_bytes("BSpace"), vec![0x7f]);
        assert_eq!(retach_key_bytes("Left"), b"\x1b[D");
        assert_eq!(retach_key_bytes("C-j"), vec![0x0a]);
    }

    #[test]
    fn retach_startup_command_sets_cwd_for_shells() {
        let spec = TerminalSessionSpec {
            workspace_id: "workspace-1".to_owned(),
            pane_id: "pane-1".to_owned(),
            kind: TerminalKind::Shell,
            cwd: "/tmp/repo with ' quote".to_owned(),
            command: String::new(),
            command_parameters: Vec::new(),
            on_exit: TerminalExitBehavior::Close,
            cols: 80,
            rows: 24,
        };

        assert_eq!(
            retach_startup_command(&spec),
            Some(b"cd -- '/tmp/repo with '\\'' quote'\r".to_vec())
        );
    }

    #[test]
    fn retach_startup_command_execs_non_shell_kind_after_cd() {
        let spec = TerminalSessionSpec {
            workspace_id: "workspace-1".to_owned(),
            pane_id: "pane-1".to_owned(),
            kind: TerminalKind::Codex,
            cwd: "/tmp/repo".to_owned(),
            command: "codex".to_owned(),
            command_parameters: Vec::new(),
            on_exit: TerminalExitBehavior::Close,
            cols: 80,
            rows: 24,
        };

        assert_eq!(
            retach_startup_command(&spec),
            Some(b"cd -- '/tmp/repo' && exec codex\r".to_vec())
        );
    }

    #[test]
    fn retach_startup_command_uses_configured_command_parameters_and_exit_behavior() {
        let spec = TerminalSessionSpec {
            workspace_id: "workspace-1".to_owned(),
            pane_id: "pane-1".to_owned(),
            kind: TerminalKind::Shell,
            cwd: "/tmp/repo".to_owned(),
            command: "codex".to_owned(),
            command_parameters: vec![
                "--dangerously-bypass-approvals-and-sandbox".to_owned(),
                "two words".to_owned(),
            ],
            on_exit: TerminalExitBehavior::RestartManually,
            cols: 80,
            rows: 24,
        };

        assert_eq!(
            retach_startup_command(&spec),
            Some(
                b"cd -- '/tmp/repo' && codex --dangerously-bypass-approvals-and-sandbox 'two words'\r"
                    .to_vec()
            )
        );
    }

    #[test]
    fn strips_common_ansi_sequences_for_capture_text() {
        assert_eq!(
            ansi_visible_text(b"\x1b[2J\x1b[Hhello\r\n\x1b]0;title\x07world"),
            "hello\nworld"
        );
    }

    #[test]
    fn retach_missing_target_matches_common_errors() {
        assert!(retach_missing_target("session 'abc' not found"));
        assert!(retach_missing_target("session doesn't exist"));
        assert!(!retach_missing_target("permission denied"));
    }
}
