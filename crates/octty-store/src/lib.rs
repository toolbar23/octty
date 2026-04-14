use std::path::{Path, PathBuf};

use octty_core::{
    AgentAttentionState, ProjectRootRecord, WorkspaceBookmarkRelation, WorkspaceSnapshot,
    WorkspaceState, WorkspaceStatus, WorkspaceSummary,
};
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

    pub async fn upsert_workspace(&self, workspace: &WorkspaceSummary) -> Result<(), StoreError> {
        let conn = self.connection().await?;
        let bookmarks_json = serde_json::to_string(&workspace.status.bookmarks)?;
        conn.execute(
            "insert into workspaces (
               id, root_id, root_path, project_label, workspace_name, display_name,
               workspace_path, workspace_state, has_working_copy_changes, bookmarks,
               bookmark_relation, unread_notes, active_agent_count, agent_attention_state,
               recent_activity_at, diff_text, created_at, updated_at, last_opened_at
             )
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
             on conflict(id) do update set
               root_id = excluded.root_id,
               root_path = excluded.root_path,
               project_label = excluded.project_label,
               workspace_name = excluded.workspace_name,
               display_name = excluded.display_name,
               workspace_path = excluded.workspace_path,
               workspace_state = excluded.workspace_state,
               has_working_copy_changes = excluded.has_working_copy_changes,
               bookmarks = excluded.bookmarks,
               bookmark_relation = excluded.bookmark_relation,
               unread_notes = excluded.unread_notes,
               active_agent_count = excluded.active_agent_count,
               agent_attention_state = excluded.agent_attention_state,
               recent_activity_at = excluded.recent_activity_at,
               diff_text = excluded.diff_text,
               updated_at = excluded.updated_at,
               last_opened_at = excluded.last_opened_at",
            params![
                workspace.id.as_str(),
                workspace.root_id.as_str(),
                workspace.root_path.as_str(),
                workspace.project_display_name.as_str(),
                workspace.workspace_name.as_str(),
                workspace.display_name.as_str(),
                workspace.workspace_path.as_str(),
                workspace_state_to_str(&workspace.status.workspace_state),
                bool_to_int(workspace.status.has_working_copy_changes),
                bookmarks_json.as_str(),
                bookmark_relation_to_str(&workspace.status.bookmark_relation),
                workspace.status.unread_notes,
                workspace.status.active_agent_count,
                optional_agent_attention_to_value(&workspace.status.agent_attention_state),
                workspace.status.recent_activity_at,
                workspace.status.diff_text.as_str(),
                workspace.created_at,
                workspace.updated_at,
                workspace.last_opened_at
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn list_workspaces(&self) -> Result<Vec<WorkspaceSummary>, StoreError> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "select id, root_id, root_path, project_label, workspace_name, display_name,
                        workspace_path, workspace_state, has_working_copy_changes, bookmarks,
                        bookmark_relation, unread_notes, active_agent_count, agent_attention_state,
                        recent_activity_at, diff_text, created_at, updated_at, last_opened_at
                 from workspaces
                 order by project_label, workspace_name",
                (),
            )
            .await?;
        let mut workspaces = Vec::new();
        while let Some(row) = rows.next().await? {
            let bookmarks = text(row.get_value(9)?, "bookmarks")?;
            workspaces.push(WorkspaceSummary {
                id: text(row.get_value(0)?, "id")?,
                root_id: text(row.get_value(1)?, "root_id")?,
                root_path: text(row.get_value(2)?, "root_path")?,
                project_display_name: text(row.get_value(3)?, "project_label")?,
                workspace_name: text(row.get_value(4)?, "workspace_name")?,
                display_name: text(row.get_value(5)?, "display_name")?,
                workspace_path: text(row.get_value(6)?, "workspace_path")?,
                status: WorkspaceStatus {
                    workspace_state: parse_workspace_state(&text(
                        row.get_value(7)?,
                        "workspace_state",
                    )?),
                    has_working_copy_changes: integer(
                        row.get_value(8)?,
                        "has_working_copy_changes",
                    )? != 0,
                    bookmarks: serde_json::from_str(&bookmarks).unwrap_or_default(),
                    bookmark_relation: parse_bookmark_relation(&text(
                        row.get_value(10)?,
                        "bookmark_relation",
                    )?),
                    unread_notes: integer(row.get_value(11)?, "unread_notes")?,
                    active_agent_count: integer(row.get_value(12)?, "active_agent_count")?,
                    agent_attention_state: parse_optional_agent_attention(row.get_value(13)?)?,
                    recent_activity_at: integer(row.get_value(14)?, "recent_activity_at")?,
                    diff_text: text(row.get_value(15)?, "diff_text")?,
                    ..WorkspaceStatus::default()
                },
                created_at: integer(row.get_value(16)?, "created_at")?,
                updated_at: integer(row.get_value(17)?, "updated_at")?,
                last_opened_at: integer(row.get_value(18)?, "last_opened_at")?,
            });
        }
        Ok(workspaces)
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
    if let Some(path) = std::env::var_os("OCTTY_RS_STATE_PATH") {
        return PathBuf::from(path);
    }
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

fn bool_to_int(value: bool) -> i64 {
    i64::from(value)
}

fn workspace_state_to_str(value: &WorkspaceState) -> &'static str {
    match value {
        WorkspaceState::Published => "published",
        WorkspaceState::MergedLocal => "merged-local",
        WorkspaceState::Draft => "draft",
        WorkspaceState::Conflicted => "conflicted",
        WorkspaceState::Unknown => "unknown",
    }
}

fn parse_workspace_state(value: &str) -> WorkspaceState {
    match value {
        "published" => WorkspaceState::Published,
        "merged-local" => WorkspaceState::MergedLocal,
        "draft" => WorkspaceState::Draft,
        "conflicted" => WorkspaceState::Conflicted,
        _ => WorkspaceState::Unknown,
    }
}

fn bookmark_relation_to_str(value: &WorkspaceBookmarkRelation) -> &'static str {
    match value {
        WorkspaceBookmarkRelation::None => "none",
        WorkspaceBookmarkRelation::Exact => "exact",
        WorkspaceBookmarkRelation::Above => "above",
    }
}

fn parse_bookmark_relation(value: &str) -> WorkspaceBookmarkRelation {
    match value {
        "exact" => WorkspaceBookmarkRelation::Exact,
        "above" => WorkspaceBookmarkRelation::Above,
        _ => WorkspaceBookmarkRelation::None,
    }
}

fn optional_agent_attention_to_value(
    value: &Option<AgentAttentionState>,
) -> Result<Value, turso::Error> {
    Ok(match value {
        Some(AgentAttentionState::IdleSeen) => Value::Text("idle-seen".to_owned()),
        Some(AgentAttentionState::Thinking) => Value::Text("thinking".to_owned()),
        Some(AgentAttentionState::IdleUnseen) => Value::Text("idle-unseen".to_owned()),
        None => Value::Null,
    })
}

fn parse_optional_agent_attention(value: Value) -> Result<Option<AgentAttentionState>, StoreError> {
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
    async fn round_trips_workspace_summaries() {
        let store = TursoStore::open_memory().await.unwrap();
        let root = ProjectRootRecord {
            id: "root-1".to_owned(),
            root_path: "/tmp/repo".to_owned(),
            display_name: "repo".to_owned(),
            created_at: 1,
            updated_at: 2,
        };
        store.upsert_project_root(&root).await.unwrap();
        let workspace = WorkspaceSummary {
            id: "workspace-1".to_owned(),
            root_id: root.id,
            root_path: "/tmp/repo".to_owned(),
            project_display_name: "repo".to_owned(),
            workspace_name: "default".to_owned(),
            display_name: "default".to_owned(),
            workspace_path: "/tmp/repo".to_owned(),
            status: WorkspaceStatus {
                workspace_state: WorkspaceState::Draft,
                has_working_copy_changes: true,
                bookmarks: vec!["main".to_owned()],
                bookmark_relation: WorkspaceBookmarkRelation::Exact,
                ..WorkspaceStatus::default()
            },
            created_at: 3,
            updated_at: 4,
            last_opened_at: 5,
        };

        store.upsert_workspace(&workspace).await.unwrap();

        assert_eq!(store.list_workspaces().await.unwrap(), vec![workspace]);
    }

    #[tokio::test]
    async fn creates_parent_directories_for_file_databases() {
        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path().join("nested").join("state.turso");

        let _store = TursoStore::open(&db_path).await.unwrap();

        assert!(db_path.exists());
    }
}
