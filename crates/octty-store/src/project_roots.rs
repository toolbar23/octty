use octty_core::ProjectRootRecord;
use turso::params;

use crate::{
    StoreError, TursoStore,
    codecs::{integer, text},
};

impl TursoStore {
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

    pub async fn update_project_root_display_name(
        &self,
        root_id: &str,
        display_name: &str,
    ) -> Result<(), StoreError> {
        let conn = self.connection().await?;
        conn.execute(
            "update project_roots
             set label = ?2, updated_at = unixepoch() * 1000
             where id = ?1",
            params![root_id, display_name],
        )
        .await?;
        Ok(())
    }

    pub async fn delete_project_root(&self, root_id: &str) -> Result<(), StoreError> {
        let conn = self.connection().await?;
        conn.execute("delete from session_state where workspace_id in (select id from workspaces where root_id = ?1)", [root_id]).await?;
        conn.execute("delete from pane_activity where workspace_id in (select id from workspaces where root_id = ?1)", [root_id]).await?;
        conn.execute("delete from workspace_snapshots where workspace_id in (select id from workspaces where root_id = ?1)", [root_id]).await?;
        conn.execute("delete from note_state where workspace_id in (select id from workspaces where root_id = ?1)", [root_id]).await?;
        conn.execute("delete from workspaces where root_id = ?1", [root_id])
            .await?;
        conn.execute("delete from project_roots where id = ?1", [root_id])
            .await?;
        Ok(())
    }
}
