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
              updated_at_ms integer not null,
              primary key (workspace_id, pane_id)
            );

            insert or ignore into schema_migrations (version, applied_at)
            values (1, unixepoch() * 1000);
            ",
        )
        .await?;
        Ok(())
    }
}
