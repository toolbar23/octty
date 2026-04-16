use super::*;
use anyhow::Context as _;
use chrono::{DateTime, Local};
use rusqlite::{Connection, ErrorCode, OpenFlags};
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodexSessionInfo {
    pub(crate) inner_session_id: String,
    pub(crate) timestamp: String,
    pub(crate) description: String,
}

#[derive(Clone, Debug)]
struct CodexHistorySession {
    latest_ts: i64,
    description_ts: i64,
    description: Option<String>,
}

#[derive(Clone, Debug)]
struct CodexThreadMetadata {
    cwd: PathBuf,
    updated_at: i64,
}

pub(crate) fn list_resumable_codex_sessions() -> anyhow::Result<Vec<CodexSessionInfo>> {
    list_resumable_codex_sessions_in_history(&codex_history_path())
}

pub(crate) async fn list_resumable_codex_sessions_for_workspace(
    workspace_path: PathBuf,
    connected_inner_session_ids: BTreeSet<String>,
) -> anyhow::Result<Vec<CodexSessionInfo>> {
    let started_at = Instant::now();
    eprintln!(
        "[octty-app] filtering resumable codex sessions for workspace {} with {} connected inner session(s)",
        workspace_path.display(),
        connected_inner_session_ids.len()
    );

    let sessions = list_resumable_codex_sessions()?;
    let history_session_count = sessions.len();
    let thread_metadata = read_codex_thread_metadata(&codex_state_path())?;
    let thread_count = thread_metadata.len();
    let sessions = filter_codex_sessions_for_workspace(
        sessions,
        &thread_metadata,
        &workspace_path,
        &connected_inner_session_ids,
    );
    eprintln!(
        "[octty-app] filtered resumable codex sessions for workspace {}: {history_session_count} history session(s), {thread_count} codex thread row(s), {} connected candidate(s), {} returned in {:?}",
        workspace_path.display(),
        connected_inner_session_ids.len(),
        sessions.len(),
        started_at.elapsed()
    );
    Ok(sessions)
}

pub(crate) fn find_codex_inner_session_id_for_pane(
    pane_id: &str,
) -> anyhow::Result<Option<String>> {
    find_codex_inner_session_id_for_pane_in_history(&codex_history_path(), pane_id)
}

pub(crate) fn connected_codex_inner_session_ids<'a>(
    snapshots: impl IntoIterator<Item = &'a WorkspaceSnapshot>,
) -> BTreeSet<String> {
    let mut inner_session_ids = BTreeSet::new();
    for snapshot in snapshots {
        for pane in snapshot.panes.values() {
            let PanePayload::Terminal(payload) = &pane.payload else {
                continue;
            };
            if payload.inner_session_handler == InnerSessionHandler::Codex
                && let Some(inner_session_id) = &payload.inner_session_id
            {
                inner_session_ids.insert(inner_session_id.clone());
            }
        }
    }
    inner_session_ids
}

pub(crate) fn find_codex_inner_session_id_for_pane_in_history(
    history_path: &Path,
    pane_id: &str,
) -> anyhow::Result<Option<String>> {
    let marker = codex_inner_session_prompt(pane_id);
    let started_at = Instant::now();
    eprintln!(
        "[octty-app] codex inner session history lookup started for pane {pane_id} in {}",
        history_path.display()
    );
    match find_codex_inner_session_id_in_history(history_path, pane_id, &marker)? {
        Some(inner_session_id) => {
            eprintln!(
                "[octty-app] codex inner session history lookup for pane {pane_id} found {inner_session_id} in {:?}",
                started_at.elapsed()
            );
            Ok(Some(inner_session_id))
        }
        None => {
            eprintln!(
                "[octty-app] codex inner session history lookup for pane {pane_id} found nothing in {:?}",
                started_at.elapsed()
            );
            Ok(None)
        }
    }
}

pub(crate) fn list_resumable_codex_sessions_in_history(
    history_path: &Path,
) -> anyhow::Result<Vec<CodexSessionInfo>> {
    let started_at = Instant::now();
    eprintln!(
        "[octty-app] listing resumable codex sessions from history {}",
        history_path.display()
    );
    let (rows, mut sessions) = read_codex_sessions_from_history(history_path)?;
    sessions.sort_by(|left, right| right.0.cmp(&left.0));
    let sessions = sessions
        .into_iter()
        .map(|(_, session)| session)
        .collect::<Vec<_>>();
    eprintln!(
        "[octty-app] listing resumable codex sessions scanned {rows} history row(s), found {} session(s) in {:?}",
        sessions.len(),
        started_at.elapsed()
    );
    Ok(sessions)
}

fn codex_history_path() -> PathBuf {
    codex_home().join("history.jsonl")
}

fn codex_state_path() -> PathBuf {
    codex_home().join("state_5.sqlite")
}

fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

fn read_codex_thread_metadata(path: &Path) -> anyhow::Result<HashMap<String, CodexThreadMetadata>> {
    let started_at = Instant::now();
    eprintln!(
        "[octty-app] loading codex thread metadata from {}",
        path.display()
    );
    if !path.exists() {
        eprintln!(
            "[octty-app] codex thread metadata path does not exist: {}",
            path.display()
        );
        return Ok(HashMap::new());
    }

    let thread_metadata = match read_codex_thread_metadata_readonly(path) {
        Ok(thread_metadata) => thread_metadata,
        Err(error) if is_sqlite_lock_error(&error) => {
            eprintln!(
                "[octty-app] codex thread metadata read hit a sqlite lock for {}; retrying immutable read: {error:#}",
                path.display()
            );
            read_codex_thread_metadata_immutable(path)?
        }
        Err(error) => return Err(error),
    };
    eprintln!(
        "[octty-app] loaded {} codex thread metadata row(s) from {} in {:?}",
        thread_metadata.len(),
        path.display(),
        started_at.elapsed()
    );
    Ok(thread_metadata)
}

fn read_codex_thread_metadata_readonly(
    path: &Path,
) -> anyhow::Result<HashMap<String, CodexThreadMetadata>> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("failed to open Codex state database {}", path.display()))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    read_codex_thread_metadata_from_connection(&conn, path)
}

fn read_codex_thread_metadata_immutable(
    path: &Path,
) -> anyhow::Result<HashMap<String, CodexThreadMetadata>> {
    let uri = format!("file:{}?mode=ro&immutable=1", path.display());
    let conn = Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| {
        format!(
            "failed to open immutable Codex state database {}",
            path.display()
        )
    })?;
    read_codex_thread_metadata_from_connection(&conn, path)
}

fn read_codex_thread_metadata_from_connection(
    conn: &Connection,
    path: &Path,
) -> anyhow::Result<HashMap<String, CodexThreadMetadata>> {
    let mut stmt = conn
        .prepare("select id, cwd, updated_at from threads where archived = 0")
        .with_context(|| {
            format!(
                "failed to prepare Codex threads query for {}",
                path.display()
            )
        })?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                CodexThreadMetadata {
                    cwd: PathBuf::from(row.get::<_, String>(1)?),
                    updated_at: row.get::<_, i64>(2)?,
                },
            ))
        })
        .with_context(|| format!("failed to query Codex threads from {}", path.display()))?;

    let mut thread_metadata = HashMap::new();
    for row in rows {
        let (id, metadata) = row
            .with_context(|| format!("failed to read Codex thread row from {}", path.display()))?;
        thread_metadata.insert(id, metadata);
    }
    Ok(thread_metadata)
}

fn is_sqlite_lock_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<rusqlite::Error>()
            .and_then(|error| match error {
                rusqlite::Error::SqliteFailure(error, _) => Some(error.code),
                _ => None,
            })
            .is_some_and(|code| matches!(code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked))
    })
}

fn filter_codex_sessions_for_workspace(
    sessions: Vec<CodexSessionInfo>,
    thread_metadata: &HashMap<String, CodexThreadMetadata>,
    workspace_path: &Path,
    connected_inner_session_ids: &BTreeSet<String>,
) -> Vec<CodexSessionInfo> {
    let mut missing_thread = 0usize;
    let mut outside_workspace = 0usize;
    let mut connected = 0usize;
    let mut kept = Vec::new();

    for mut session in sessions {
        let Some(metadata) = thread_metadata.get(&session.inner_session_id) else {
            missing_thread += 1;
            continue;
        };
        if !path_is_in_workspace(&metadata.cwd, workspace_path) {
            outside_workspace += 1;
            continue;
        }
        if connected_inner_session_ids.contains(&session.inner_session_id) {
            connected += 1;
            continue;
        }
        session.timestamp = format_codex_history_timestamp(metadata.updated_at);
        kept.push(session);
    }

    eprintln!(
        "[octty-app] codex resume filter kept {} session(s), skipped {missing_thread} without thread metadata, {outside_workspace} outside workspace, {connected} connected",
        kept.len()
    );
    kept
}

fn path_is_in_workspace(path: &Path, workspace_path: &Path) -> bool {
    path == workspace_path || path.starts_with(workspace_path)
}

fn find_codex_inner_session_id_in_history(
    path: &Path,
    pane_id: &str,
    marker: &str,
) -> anyhow::Result<Option<String>> {
    if !path.exists() {
        eprintln!(
            "[octty-app] codex history path does not exist: {}",
            path.display()
        );
        return Ok(None);
    }

    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut latest = None::<(i64, String)>;
    let mut rows = 0usize;
    for line in reader.lines() {
        rows += 1;
        let line = line?;
        if !line.contains(pane_id) {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if !value
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains(marker))
        {
            continue;
        }
        let Some(session_id) = value
            .get("session_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
        else {
            continue;
        };
        let ts = value.get("ts").and_then(Value::as_i64).unwrap_or_default();
        latest = match latest {
            Some((latest_ts, latest_id)) if latest_ts > ts => Some((latest_ts, latest_id)),
            _ => Some((ts, session_id)),
        };
    }
    eprintln!(
        "[octty-app] codex history lookup scanned {rows} row(s) in {}",
        path.display()
    );
    Ok(latest.map(|(_, session_id)| session_id))
}

fn read_codex_sessions_from_history(
    path: &Path,
) -> anyhow::Result<(usize, Vec<(i64, CodexSessionInfo)>)> {
    if !path.exists() {
        eprintln!(
            "[octty-app] codex history path does not exist: {}",
            path.display()
        );
        return Ok((0, Vec::new()));
    }
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut rows = 0usize;
    let mut by_session = HashMap::<String, CodexHistorySession>::new();
    for line in reader.lines() {
        rows += 1;
        let line = line?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(session_id) = value
            .get("session_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
        else {
            continue;
        };
        let ts = value.get("ts").and_then(Value::as_i64).unwrap_or_default();
        let text = value
            .get("text")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let entry = by_session
            .entry(session_id)
            .or_insert_with(|| CodexHistorySession {
                latest_ts: ts,
                description_ts: i64::MIN,
                description: None,
            });
        entry.latest_ts = entry.latest_ts.max(ts);
        if let Some(text) = text
            && is_codex_session_description_candidate(&text)
            && ts >= entry.description_ts
        {
            entry.description_ts = ts;
            entry.description = Some(text);
        }
    }

    let sessions = by_session
        .into_iter()
        .map(|(inner_session_id, session)| {
            (
                session.latest_ts,
                CodexSessionInfo {
                    inner_session_id,
                    timestamp: format_codex_history_timestamp(session.latest_ts),
                    description: session
                        .description
                        .unwrap_or_else(|| "Untitled Codex session".to_owned()),
                },
            )
        })
        .collect();
    Ok((rows, sessions))
}

fn is_codex_session_description_candidate(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && !text.starts_with("<permissions instructions>")
        && !text.starts_with("<collaboration_mode>")
        && !text.starts_with("<apps_instructions>")
        && !text.starts_with("<skills_instructions>")
        && !text.starts_with("<environment_context>")
        && !text.starts_with("# AGENTS.md instructions")
        && !text.starts_with("You are running inside Octty pane ")
        && !text.starts_with("FYI: You are running inside Octty pane ")
}

fn format_codex_history_timestamp(ts: i64) -> String {
    DateTime::from_timestamp(ts, 0)
        .map(|timestamp| {
            timestamp
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| ts.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::Write,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn codex_history_lists_sessions_with_descriptions() {
        let history_path = create_temp_history_path();
        write_history_entry(
            &history_path,
            "019d96c4-8591-7353-9119-26e630293b23",
            1776350954,
            "implement PLAN.md",
        );

        let sessions = list_resumable_codex_sessions_in_history(&history_path).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].inner_session_id,
            "019d96c4-8591-7353-9119-26e630293b23"
        );
        assert_ne!(sessions[0].timestamp, "1776350954");
        assert!(sessions[0].timestamp.contains('-'));
        assert_eq!(sessions[0].description, "implement PLAN.md");
    }

    #[test]
    fn codex_history_timestamp_falls_back_to_raw_value_when_unformattable() {
        assert_eq!(
            format_codex_history_timestamp(i64::MAX),
            i64::MAX.to_string()
        );
    }

    #[test]
    fn codex_history_ignores_init_prompt_as_description() {
        let history_path = create_temp_history_path();
        write_history_entry(
            &history_path,
            "019d96c4-8591-7353-9119-26e630293b23",
            1776350954,
            "FYI: You are running inside Octty pane \"pane-1\". Reply \"ok\"",
        );
        write_history_entry(
            &history_path,
            "019d96c4-8591-7353-9119-26e630293b23",
            1776350959,
            "continue previous task",
        );

        let sessions = list_resumable_codex_sessions_in_history(&history_path).unwrap();

        assert_eq!(sessions[0].description, "continue previous task");
    }

    #[test]
    fn codex_history_lookup_finds_octty_pane_marker() {
        let history_path = create_temp_history_path();
        write_history_entry(
            &history_path,
            "019d96da-76eb-7c00-90c1-ed9ddc30c613",
            1776352395,
            "FYI: You are running inside Octty pane \"pane-1776352389726-1\". Reply \"ok\"",
        );

        assert_eq!(
            find_codex_inner_session_id_for_pane_in_history(&history_path, "pane-1776352389726-1")
                .unwrap(),
            Some("019d96da-76eb-7c00-90c1-ed9ddc30c613".to_owned())
        );
    }

    #[test]
    fn codex_resume_filter_keeps_workspace_sessions_and_excludes_connected() {
        let sessions = vec![
            codex_session("workspace-session"),
            codex_session("workspace-child-session"),
            codex_session("other-session"),
            codex_session("connected-session"),
            codex_session("missing-thread-session"),
        ];
        let thread_metadata = HashMap::from([
            (
                "workspace-session".to_owned(),
                codex_thread_metadata("/repo/workspace", 1776350954),
            ),
            (
                "workspace-child-session".to_owned(),
                codex_thread_metadata("/repo/workspace/crate", 1776352395),
            ),
            (
                "other-session".to_owned(),
                codex_thread_metadata("/repo/workspace-other", 1776353000),
            ),
            (
                "connected-session".to_owned(),
                codex_thread_metadata("/repo/workspace", 1776354000),
            ),
        ]);
        let connected = BTreeSet::from(["connected-session".to_owned()]);

        let sessions = filter_codex_sessions_for_workspace(
            sessions,
            &thread_metadata,
            Path::new("/repo/workspace"),
            &connected,
        );

        assert_eq!(
            sessions
                .iter()
                .map(|session| session.inner_session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["workspace-session", "workspace-child-session"]
        );
        assert!(sessions.iter().all(|session| session.timestamp != "1"));
    }

    fn create_temp_history_path() -> PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("octty-codex-sessions-{id}"));
        fs::create_dir_all(&root).unwrap();
        root.join("history.jsonl")
    }

    fn write_history_entry(path: &Path, session_id: &str, ts: i64, text: &str) {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "session_id": session_id,
                "ts": ts,
                "text": text
            })
        )
        .unwrap();
    }

    fn codex_session(inner_session_id: &str) -> CodexSessionInfo {
        CodexSessionInfo {
            inner_session_id: inner_session_id.to_owned(),
            timestamp: "1".to_owned(),
            description: inner_session_id.to_owned(),
        }
    }

    fn codex_thread_metadata(cwd: &str, updated_at: i64) -> CodexThreadMetadata {
        CodexThreadMetadata {
            cwd: PathBuf::from(cwd),
            updated_at,
        }
    }
}
