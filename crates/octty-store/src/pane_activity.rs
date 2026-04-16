use octty_core::PaneActivity;
use turso::params;

use crate::{
    StoreError, TursoStore,
    codecs::{
        bool_to_int, integer, optional_i64_to_value, optional_integer, optional_str_to_value,
        optional_text, text,
    },
};

impl TursoStore {
    pub async fn upsert_pane_activity(&self, activity: &PaneActivity) -> Result<(), StoreError> {
        let conn = self.connection().await?;
        conn.execute(
            "insert into pane_activity (
               workspace_id, pane_id, last_activity_at_ms, last_seen_at_ms,
               last_seen_activity_at_ms, last_tmux_activity_at_s,
               last_seen_tmux_activity_at_s, last_screen_fingerprint,
               last_seen_screen_fingerprint, needs_attention, needs_attention_at_ms,
               needs_attention_cleared_at_ms, updated_at_ms
             )
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             on conflict(workspace_id, pane_id) do update set
               last_activity_at_ms = excluded.last_activity_at_ms,
               last_seen_at_ms = excluded.last_seen_at_ms,
               last_seen_activity_at_ms = excluded.last_seen_activity_at_ms,
               last_tmux_activity_at_s = excluded.last_tmux_activity_at_s,
               last_seen_tmux_activity_at_s = excluded.last_seen_tmux_activity_at_s,
               last_screen_fingerprint = excluded.last_screen_fingerprint,
               last_seen_screen_fingerprint = excluded.last_seen_screen_fingerprint,
               needs_attention = excluded.needs_attention,
               needs_attention_at_ms = excluded.needs_attention_at_ms,
               needs_attention_cleared_at_ms = excluded.needs_attention_cleared_at_ms,
               updated_at_ms = excluded.updated_at_ms",
            params![
                activity.workspace_id.as_str(),
                activity.pane_id.as_str(),
                activity.last_activity_at_ms,
                activity.last_seen_at_ms,
                activity.last_seen_activity_at_ms,
                optional_i64_to_value(activity.last_tmux_activity_at_s),
                optional_i64_to_value(activity.last_seen_tmux_activity_at_s),
                optional_str_to_value(activity.last_screen_fingerprint.as_deref()),
                optional_str_to_value(activity.last_seen_screen_fingerprint.as_deref()),
                bool_to_int(activity.needs_attention),
                activity.needs_attention_at_ms,
                activity.needs_attention_cleared_at_ms,
                activity.updated_at_ms,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn upsert_pane_activities(
        &self,
        activities: &[PaneActivity],
    ) -> Result<(), StoreError> {
        for activity in activities {
            self.upsert_pane_activity(activity).await?;
        }
        Ok(())
    }

    pub async fn delete_pane_activity(
        &self,
        workspace_id: &str,
        pane_id: &str,
    ) -> Result<(), StoreError> {
        let conn = self.connection().await?;
        conn.execute(
            "delete from pane_activity where workspace_id = ?1 and pane_id = ?2",
            params![workspace_id, pane_id],
        )
        .await?;
        Ok(())
    }

    pub async fn list_pane_activity(&self) -> Result<Vec<PaneActivity>, StoreError> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "select workspace_id, pane_id, last_activity_at_ms, last_seen_at_ms,
                        last_seen_activity_at_ms, last_tmux_activity_at_s,
                        last_seen_tmux_activity_at_s, last_screen_fingerprint,
                        last_seen_screen_fingerprint, needs_attention,
                        needs_attention_at_ms, needs_attention_cleared_at_ms,
                        updated_at_ms
                 from pane_activity
                 order by workspace_id, pane_id",
                (),
            )
            .await?;
        let mut activities = Vec::new();
        while let Some(row) = rows.next().await? {
            activities.push(PaneActivity {
                workspace_id: text(row.get_value(0)?, "workspace_id")?,
                pane_id: text(row.get_value(1)?, "pane_id")?,
                last_activity_at_ms: integer(row.get_value(2)?, "last_activity_at_ms")?,
                last_seen_at_ms: integer(row.get_value(3)?, "last_seen_at_ms")?,
                last_seen_activity_at_ms: integer(row.get_value(4)?, "last_seen_activity_at_ms")?,
                last_tmux_activity_at_s: optional_integer(
                    row.get_value(5)?,
                    "last_tmux_activity_at_s",
                )?,
                last_seen_tmux_activity_at_s: optional_integer(
                    row.get_value(6)?,
                    "last_seen_tmux_activity_at_s",
                )?,
                last_screen_fingerprint: optional_text(
                    row.get_value(7)?,
                    "last_screen_fingerprint",
                )?,
                last_seen_screen_fingerprint: optional_text(
                    row.get_value(8)?,
                    "last_seen_screen_fingerprint",
                )?,
                needs_attention: integer(row.get_value(9)?, "needs_attention")? != 0,
                needs_attention_at_ms: integer(row.get_value(10)?, "needs_attention_at_ms")?,
                needs_attention_cleared_at_ms: integer(
                    row.get_value(11)?,
                    "needs_attention_cleared_at_ms",
                )?,
                updated_at_ms: integer(row.get_value(12)?, "updated_at_ms")?,
            });
        }
        Ok(activities)
    }
}
