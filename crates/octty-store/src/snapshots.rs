use octty_core::WorkspaceSnapshot;
use turso::params;

use crate::{StoreError, TursoStore, codecs::text};

impl TursoStore {
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

    pub async fn list_snapshots(&self) -> Result<Vec<WorkspaceSnapshot>, StoreError> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "select snapshot_json from workspace_snapshots order by workspace_id",
                (),
            )
            .await?;
        let mut snapshots = Vec::new();
        while let Some(row) = rows.next().await? {
            snapshots.push(serde_json::from_str(&text(
                row.get_value(0)?,
                "snapshot_json",
            )?)?);
        }
        Ok(snapshots)
    }
}
