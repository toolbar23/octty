use std::path::{Path, PathBuf};

use octty_core::{ProjectRootRecord, WorkspaceSnapshot};
use thiserror::Error;
use turso::{Builder, Connection, Database, Value, params};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] turso::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unexpected value for `{column}`: {value:?}")]
    UnexpectedValue { column: &'static str, value: Value },
}

pub struct TursoStore {
    db: Database,
}

impl TursoStore {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        if let Some(parent) = path.as_ref().parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let db = Builder::new_local(path.as_ref().to_string_lossy().as_ref())
            .build()
            .await?;
        let store = Self { db };
        store.migrate().await?;
        Ok(store)
    }

    pub async fn open_memory() -> Result<Self, StoreError> {
        let db = Builder::new_local(":memory:").build().await?;
        let store = Self { db };
        store.migrate().await?;
        Ok(store)
    }

    pub async fn connection(&self) -> Result<Connection, StoreError> {
        Ok(self.db.connect()?)
    }

    pub async fn upsert_project_root(&self, root: &ProjectRootRecord) -> Result<(), StoreError> {
        let conn = self.connection().await?;
        conn.execute(
            "insert into project_roots (id, root_path, label, created_at, updated_at)
             values (?1, ?2, ?3, ?4, ?5)
             on conflict(id) do update set
               root_path = excluded.root_path,
               label = excluded.label,
               updated_at = excluded.updated_at",
            params![
                root.id.as_str(),
                root.root_path.as_str(),
                root.display_name.as_str(),
                root.created_at,
                root.updated_at
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn list_project_roots(&self) -> Result<Vec<ProjectRootRecord>, StoreError> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "select id, root_path, label, created_at, updated_at
                 from project_roots
                 order by label, root_path",
                (),
            )
            .await?;
        let mut roots = Vec::new();
        while let Some(row) = rows.next().await? {
            roots.push(ProjectRootRecord {
                id: text(row.get_value(0)?, "id")?,
                root_path: text(row.get_value(1)?, "root_path")?,
                display_name: text(row.get_value(2)?, "label")?,
                created_at: integer(row.get_value(3)?, "created_at")?,
                updated_at: integer(row.get_value(4)?, "updated_at")?,
            });
        }
        Ok(roots)
    }

    pub async fn save_snapshot(&self, snapshot: &WorkspaceSnapshot) -> Result<(), StoreError> {
        let conn = self.connection().await?;
        let snapshot_json = serde_json::to_string(snapshot)?;
        conn.execute(
            "insert into workspace_snapshots (workspace_id, snapshot_json, updated_at)
             values (?1, ?2, ?3)
             on conflict(workspace_id) do update set
               snapshot_json = excluded.snapshot_json,
               updated_at = excluded.updated_at",
            params![
                snapshot.workspace_id.as_str(),
                snapshot_json.as_str(),
                snapshot.updated_at
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn get_snapshot(
        &self,
        workspace_id: &str,
    ) -> Result<Option<WorkspaceSnapshot>, StoreError> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "select snapshot_json from workspace_snapshots where workspace_id = ?1",
                [workspace_id],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        let snapshot_json = text(row.get_value(0)?, "snapshot_json")?;
        Ok(Some(serde_json::from_str(&snapshot_json)?))
    }

    async fn migrate(&self) -> Result<(), StoreError> {
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

            insert or ignore into schema_migrations (version, applied_at)
            values (1, unixepoch() * 1000);
            ",
        )
        .await?;
        Ok(())
    }
}

pub fn default_store_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".local")
        .join("share")
        .join("octty-rs")
        .join("state.turso")
}

fn text(value: Value, column: &'static str) -> Result<String, StoreError> {
    match value {
        Value::Text(value) => Ok(value),
        value => Err(StoreError::UnexpectedValue { column, value }),
    }
}

fn integer(value: Value, column: &'static str) -> Result<i64, StoreError> {
    match value {
        Value::Integer(value) => Ok(value),
        value => Err(StoreError::UnexpectedValue { column, value }),
    }
}

#[cfg(test)]
mod tests {
    use octty_core::{PaneType, add_pane, create_default_snapshot, create_pane_state};

    use super::*;

    #[tokio::test]
    async fn migrates_and_round_trips_project_roots() {
        let store = TursoStore::open_memory().await.unwrap();
        let root = ProjectRootRecord {
            id: "root-1".to_owned(),
            root_path: "/tmp/repo".to_owned(),
            display_name: "repo".to_owned(),
            created_at: 1,
            updated_at: 2,
        };

        store.upsert_project_root(&root).await.unwrap();

        assert_eq!(store.list_project_roots().await.unwrap(), vec![root]);
    }

    #[tokio::test]
    async fn round_trips_workspace_snapshots() {
        let store = TursoStore::open_memory().await.unwrap();
        let snapshot = add_pane(
            create_default_snapshot("workspace-1"),
            create_pane_state(PaneType::Shell, "/tmp/repo", None),
        );

        store.save_snapshot(&snapshot).await.unwrap();

        assert_eq!(
            store.get_snapshot("workspace-1").await.unwrap(),
            Some(snapshot)
        );
    }

    #[tokio::test]
    async fn creates_parent_directories_for_file_databases() {
        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path().join("nested").join("state.turso");

        let _store = TursoStore::open(&db_path).await.unwrap();

        assert!(db_path.exists());
    }
}
