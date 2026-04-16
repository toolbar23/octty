use std::collections::HashSet;

use crate::{StoreError, TursoStore};

impl TursoStore {
    pub(crate) async fn migrate(&self) -> Result<(), StoreError> {
        let conn = self.connection().await?;
        conn.execute_batch(
            "
            create table if not exists schema_migrations (
              version integer primary key,
              applied_at integer not null
            );

            create table if not exists project_roots (
              id text primary key,
              root_path text not null unique,
              label text not null,
              created_at integer not null,
              updated_at integer not null
            );

            create table if not exists workspaces (
              id text primary key,
              root_id text not null references project_roots(id) on delete cascade,
              root_path text not null,
              project_label text not null,
              workspace_name text not null,
              display_name text not null default '',
		              workspace_path text not null unique,
		              workspace_state text not null default 'unknown',
		              has_working_copy_changes integer not null default 0,
		              has_conflicts integer not null default 0,
		              local_baseline_name text,
		              local_baseline_detail text,
		              local_ahead_count integer,
		              local_behind_count integer,
		              remote_baseline_name text,
		              remote_baseline_detail text,
		              remote_ahead_count integer,
		              remote_behind_count integer,
		              primary_relation text not null default 'none',
		              bookmarks text not null default '',
		              bookmark_relation text not null default 'none',
              unread_notes integer not null default 0,
              active_agent_count integer not null default 0,
              agent_attention_state text,
              recent_activity_at integer not null default 0,
              diff_text text not null default '',
              created_at integer not null,
              updated_at integer not null,
              last_opened_at integer not null default 0
            );

            create table if not exists workspace_snapshots (
              workspace_id text primary key,
              snapshot_json text not null,
              updated_at integer not null
            );

            create table if not exists note_state (
              workspace_id text not null,
              note_path text not null,
              title text not null default '',
              last_read_at integer not null default 0,
              last_known_mtime integer not null default 0,
              primary key (workspace_id, note_path)
            );

            create table if not exists session_state (
              pane_id text primary key,
              workspace_id text not null,
              session_id text,
              kind text not null,
              cwd text not null,
              command text not null,
              state text not null,
              exit_code integer,
              buffer text not null default '',
              updated_at integer not null
            );

            create table if not exists pane_activity (
              workspace_id text not null,
              pane_id text not null,
              last_activity_at_ms integer not null default 0,
              last_seen_at_ms integer not null default 0,
              last_seen_activity_at_ms integer not null default 0,
              last_tmux_activity_at_s integer,
              last_seen_tmux_activity_at_s integer,
              last_screen_fingerprint text,
              last_seen_screen_fingerprint text,
              needs_attention integer not null default 0,
              needs_attention_at_ms integer not null default 0,
              needs_attention_cleared_at_ms integer not null default 0,
              updated_at_ms integer not null,
              primary key (workspace_id, pane_id)
            );

            insert or ignore into schema_migrations (version, applied_at)
            values (1, unixepoch() * 1000);
            ",
        )
        .await?;
        ensure_workspace_status_columns(&conn).await?;
        ensure_pane_activity_columns(&conn).await?;
        Ok(())
    }
}

async fn ensure_workspace_status_columns(conn: &turso::Connection) -> Result<(), StoreError> {
    let mut rows = conn.query("pragma table_info(workspaces)", ()).await?;
    let mut columns = HashSet::new();
    while let Some(row) = rows.next().await? {
        if let turso::Value::Text(name) = row.get_value(1)? {
            columns.insert(name);
        }
    }
    drop(rows);

    for (name, definition) in [
        ("has_conflicts", "has_conflicts integer not null default 0"),
        ("local_baseline_name", "local_baseline_name text"),
        ("local_baseline_detail", "local_baseline_detail text"),
        ("local_ahead_count", "local_ahead_count integer"),
        ("local_behind_count", "local_behind_count integer"),
        ("remote_baseline_name", "remote_baseline_name text"),
        ("remote_baseline_detail", "remote_baseline_detail text"),
        ("remote_ahead_count", "remote_ahead_count integer"),
        ("remote_behind_count", "remote_behind_count integer"),
        (
            "primary_relation",
            "primary_relation text not null default 'none'",
        ),
    ] {
        if !columns.contains(name) {
            conn.execute(
                format!("alter table workspaces add column {definition}").as_str(),
                (),
            )
            .await?;
        }
    }

    Ok(())
}

async fn ensure_pane_activity_columns(conn: &turso::Connection) -> Result<(), StoreError> {
    let mut rows = conn.query("pragma table_info(pane_activity)", ()).await?;
    let mut columns = HashSet::new();
    while let Some(row) = rows.next().await? {
        if let turso::Value::Text(name) = row.get_value(1)? {
            columns.insert(name);
        }
    }
    drop(rows);

    for (name, definition) in [
        (
            "needs_attention",
            "needs_attention integer not null default 0",
        ),
        (
            "needs_attention_at_ms",
            "needs_attention_at_ms integer not null default 0",
        ),
        (
            "needs_attention_cleared_at_ms",
            "needs_attention_cleared_at_ms integer not null default 0",
        ),
    ] {
        if !columns.contains(name) {
            conn.execute(
                format!("alter table pane_activity add column {definition}").as_str(),
                (),
            )
            .await?;
        }
    }

    Ok(())
}
