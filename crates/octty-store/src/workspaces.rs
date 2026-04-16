use octty_core::{BaselineRelation, WorkspaceSnapshot, WorkspaceStatus, WorkspaceSummary};
use turso::params;

use crate::{
    StoreError, TursoStore,
    codecs::{
        bookmark_relation_to_str, bool_to_int, integer, optional_agent_attention_to_value,
        optional_i64_to_value, optional_str_to_value, optional_text, parse_bookmark_relation,
        parse_optional_agent_attention, parse_primary_relation, parse_workspace_state,
        primary_relation_to_str, text, workspace_state_to_str,
    },
};

impl TursoStore {
    pub async fn upsert_workspace(&self, workspace: &WorkspaceSummary) -> Result<(), StoreError> {
        let conn = self.connection().await?;
        let bookmarks_json = serde_json::to_string(&workspace.status.bookmarks)?;
        conn.execute(
            "insert into workspaces (
	               id, root_id, root_path, project_label, workspace_name, display_name,
	               workspace_path, workspace_state, has_working_copy_changes, bookmarks,
	               has_conflicts, local_baseline_name, local_baseline_detail, local_ahead_count,
	               local_behind_count, remote_baseline_name, remote_baseline_detail,
	               remote_ahead_count, remote_behind_count, primary_relation, bookmark_relation,
	               unread_notes, active_agent_count, agent_attention_state, recent_activity_at,
	               diff_text, created_at, updated_at, last_opened_at
	             )
	             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29)
             on conflict(id) do update set
               root_id = excluded.root_id,
               root_path = excluded.root_path,
               project_label = excluded.project_label,
               workspace_name = excluded.workspace_name,
               display_name = excluded.display_name,
               workspace_path = excluded.workspace_path,
               workspace_state = excluded.workspace_state,
	               has_working_copy_changes = excluded.has_working_copy_changes,
	               has_conflicts = excluded.has_conflicts,
	               local_baseline_name = excluded.local_baseline_name,
	               local_baseline_detail = excluded.local_baseline_detail,
	               local_ahead_count = excluded.local_ahead_count,
	               local_behind_count = excluded.local_behind_count,
	               remote_baseline_name = excluded.remote_baseline_name,
	               remote_baseline_detail = excluded.remote_baseline_detail,
	               remote_ahead_count = excluded.remote_ahead_count,
               remote_behind_count = excluded.remote_behind_count,
               primary_relation = excluded.primary_relation,
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
                bool_to_int(workspace.status.has_conflicts),
                optional_str_to_value(
                    workspace
                        .status
                        .local_relation
                        .as_ref()
                        .map(|relation| relation.target_name.as_str())
                )?,
                optional_str_to_value(
                    workspace
                        .status
                        .local_relation
                        .as_ref()
                        .and_then(|relation| relation.detail_name.as_deref())
                )?,
                optional_i64_to_value(
                    workspace
                        .status
                        .local_relation
                        .as_ref()
                        .map(|relation| relation.ahead_count)
                )?,
                optional_i64_to_value(
                    workspace
                        .status
                        .local_relation
                        .as_ref()
                        .map(|relation| relation.behind_count)
                )?,
                optional_str_to_value(
                    workspace
                        .status
                        .remote_relation
                        .as_ref()
                        .map(|relation| relation.target_name.as_str())
                )?,
                optional_str_to_value(
                    workspace
                        .status
                        .remote_relation
                        .as_ref()
                        .and_then(|relation| relation.detail_name.as_deref())
                )?,
                optional_i64_to_value(
                    workspace
                        .status
                        .remote_relation
                        .as_ref()
                        .map(|relation| relation.ahead_count)
                )?,
                optional_i64_to_value(
                    workspace
                        .status
                        .remote_relation
                        .as_ref()
                        .map(|relation| relation.behind_count)
                )?,
                primary_relation_to_str(&workspace.status.primary_relation),
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

    pub async fn update_workspace_project_display_name(
        &self,
        root_id: &str,
        display_name: &str,
    ) -> Result<(), StoreError> {
        let conn = self.connection().await?;
        conn.execute(
            "update workspaces
             set project_label = ?2, updated_at = unixepoch() * 1000
             where root_id = ?1",
            params![root_id, display_name],
        )
        .await?;
        Ok(())
    }

    pub async fn update_workspace_display_name(
        &self,
        workspace_id: &str,
        display_name: &str,
    ) -> Result<(), StoreError> {
        let conn = self.connection().await?;
        conn.execute(
            "update workspaces
             set display_name = ?2, updated_at = unixepoch() * 1000
             where id = ?1",
            params![workspace_id, display_name],
        )
        .await?;
        Ok(())
    }

    pub async fn rename_workspace(
        &self,
        workspace_id: &str,
        next_workspace_id: &str,
        workspace_name: &str,
        display_name: &str,
    ) -> Result<(), StoreError> {
        let conn = self.connection().await?;
        let mut snapshot_rows = conn
            .query(
                "select snapshot_json from workspace_snapshots where workspace_id = ?1",
                [workspace_id],
            )
            .await?;
        let snapshot_json = if let Some(row) = snapshot_rows.next().await? {
            let mut snapshot: WorkspaceSnapshot =
                serde_json::from_str(&text(row.get_value(0)?, "snapshot_json")?)?;
            snapshot.workspace_id = next_workspace_id.to_owned();
            Some(serde_json::to_string(&snapshot)?)
        } else {
            None
        };
        drop(snapshot_rows);

        conn.execute("begin concurrent", ()).await?;
        let result = async {
            conn.execute(
                "update workspaces
                 set id = ?2,
                     workspace_name = ?3,
                     display_name = ?4,
                     updated_at = unixepoch() * 1000
                 where id = ?1",
                params![
                    workspace_id,
                    next_workspace_id,
                    workspace_name,
                    display_name
                ],
            )
            .await?;
            if let Some(snapshot_json) = snapshot_json.as_deref() {
                conn.execute(
                    "update workspace_snapshots
                     set workspace_id = ?2,
                         snapshot_json = ?3,
                         updated_at = unixepoch() * 1000
                     where workspace_id = ?1",
                    params![workspace_id, next_workspace_id, snapshot_json],
                )
                .await?;
            } else {
                conn.execute(
                    "update workspace_snapshots
                     set workspace_id = ?2,
                         updated_at = unixepoch() * 1000
                     where workspace_id = ?1",
                    params![workspace_id, next_workspace_id],
                )
                .await?;
            }
            conn.execute(
                "update note_state set workspace_id = ?2 where workspace_id = ?1",
                params![workspace_id, next_workspace_id],
            )
            .await?;
            conn.execute(
                "update session_state set workspace_id = ?2 where workspace_id = ?1",
                params![workspace_id, next_workspace_id],
            )
            .await?;
            conn.execute(
                "update pane_activity set workspace_id = ?2 where workspace_id = ?1",
                params![workspace_id, next_workspace_id],
            )
            .await?;
            Ok::<(), StoreError>(())
        }
        .await;

        match result {
            Ok(()) => {
                conn.execute("commit", ()).await?;
                Ok(())
            }
            Err(error) => {
                let _ = conn.execute("rollback", ()).await;
                Err(error)
            }
        }
    }

    pub async fn delete_workspace(&self, workspace_id: &str) -> Result<(), StoreError> {
        let conn = self.connection().await?;
        conn.execute(
            "delete from session_state where workspace_id = ?1",
            [workspace_id],
        )
        .await?;
        conn.execute(
            "delete from pane_activity where workspace_id = ?1",
            [workspace_id],
        )
        .await?;
        conn.execute(
            "delete from workspace_snapshots where workspace_id = ?1",
            [workspace_id],
        )
        .await?;
        conn.execute(
            "delete from note_state where workspace_id = ?1",
            [workspace_id],
        )
        .await?;
        conn.execute("delete from workspaces where id = ?1", [workspace_id])
            .await?;
        Ok(())
    }

    pub async fn list_workspaces(&self) -> Result<Vec<WorkspaceSummary>, StoreError> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "select id, root_id, root_path, project_label, workspace_name, display_name,
	                        workspace_path, workspace_state, has_working_copy_changes, bookmarks,
	                        has_conflicts, local_baseline_name, local_baseline_detail,
	                        local_ahead_count, local_behind_count, remote_baseline_name,
	                        remote_baseline_detail, remote_ahead_count, remote_behind_count,
	                        primary_relation, bookmark_relation, unread_notes, active_agent_count,
	                        agent_attention_state, recent_activity_at, diff_text, created_at,
	                        updated_at, last_opened_at
                 from workspaces
                 order by project_label, workspace_name",
                (),
            )
            .await?;
        let mut workspaces = Vec::new();
        while let Some(row) = rows.next().await? {
            let bookmarks = text(row.get_value(9)?, "bookmarks")?;
            let local_relation = match optional_text(row.get_value(11)?, "local_baseline_name")? {
                Some(target_name) => Some(BaselineRelation {
                    target_name,
                    detail_name: optional_text(row.get_value(12)?, "local_baseline_detail")?,
                    ahead_count: integer(row.get_value(13)?, "local_ahead_count")?,
                    behind_count: integer(row.get_value(14)?, "local_behind_count")?,
                }),
                None => None,
            };
            let remote_relation = match optional_text(row.get_value(15)?, "remote_baseline_name")? {
                Some(target_name) => Some(BaselineRelation {
                    target_name,
                    detail_name: optional_text(row.get_value(16)?, "remote_baseline_detail")?,
                    ahead_count: integer(row.get_value(17)?, "remote_ahead_count")?,
                    behind_count: integer(row.get_value(18)?, "remote_behind_count")?,
                }),
                None => None,
            };
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
                    has_conflicts: integer(row.get_value(10)?, "has_conflicts")? != 0,
                    local_relation,
                    remote_relation,
                    primary_relation: parse_primary_relation(&text(
                        row.get_value(19)?,
                        "primary_relation",
                    )?),
                    bookmarks: serde_json::from_str(&bookmarks).unwrap_or_default(),
                    bookmark_relation: parse_bookmark_relation(&text(
                        row.get_value(20)?,
                        "bookmark_relation",
                    )?),
                    unread_notes: integer(row.get_value(21)?, "unread_notes")?,
                    active_agent_count: integer(row.get_value(22)?, "active_agent_count")?,
                    agent_attention_state: parse_optional_agent_attention(row.get_value(23)?)?,
                    recent_activity_at: integer(row.get_value(24)?, "recent_activity_at")?,
                    diff_text: text(row.get_value(25)?, "diff_text")?,
                    ..WorkspaceStatus::default()
                },
                created_at: integer(row.get_value(26)?, "created_at")?,
                updated_at: integer(row.get_value(27)?, "updated_at")?,
                last_opened_at: integer(row.get_value(28)?, "last_opened_at")?,
            });
        }
        Ok(workspaces)
    }
}
