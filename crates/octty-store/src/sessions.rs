use octty_core::SessionSnapshot;
use turso::params;

use crate::{
    StoreError, TursoStore,
    codecs::{
        optional_i64_to_value, optional_integer, optional_str_to_value, optional_text,
        parse_session_state, parse_terminal_kind, session_state_to_str, terminal_kind_to_str, text,
    },
};

impl TursoStore {
    pub async fn upsert_session_state(&self, session: &SessionSnapshot) -> Result<(), StoreError> {
        let conn = self.connection().await?;
        conn.execute(
            "insert into session_state (
               pane_id, workspace_id, session_id, inner_session_id, kind, cwd, command, state, exit_code, buffer, updated_at
             )
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, unixepoch() * 1000)
             on conflict(pane_id) do update set
               workspace_id = excluded.workspace_id,
               session_id = excluded.session_id,
               inner_session_id = excluded.inner_session_id,
               kind = excluded.kind,
               cwd = excluded.cwd,
               command = excluded.command,
               state = excluded.state,
               exit_code = excluded.exit_code,
               buffer = excluded.buffer,
               updated_at = excluded.updated_at",
            params![
                session.pane_id.as_str(),
                session.workspace_id.as_str(),
                session.id.as_str(),
                optional_str_to_value(session.inner_session_id.as_deref())?,
                terminal_kind_to_str(&session.kind),
                session.cwd.as_str(),
                session.command.as_str(),
                session_state_to_str(&session.state),
                optional_i64_to_value(session.exit_code),
                session.buffer.as_str()
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn get_session_state_by_pane(
        &self,
        pane_id: &str,
    ) -> Result<Option<SessionSnapshot>, StoreError> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "select pane_id, workspace_id, session_id, inner_session_id, kind, cwd, command, state, exit_code, buffer
                 from session_state
                 where pane_id = ?1",
                [pane_id],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        let pane_id = text(row.get_value(0)?, "pane_id")?;
        Ok(Some(SessionSnapshot {
            pane_id,
            workspace_id: text(row.get_value(1)?, "workspace_id")?,
            id: text(row.get_value(2)?, "session_id")?,
            inner_session_id: optional_text(row.get_value(3)?, "inner_session_id")?,
            kind: parse_terminal_kind(&text(row.get_value(4)?, "kind")?),
            cwd: text(row.get_value(5)?, "cwd")?,
            command: text(row.get_value(6)?, "command")?,
            state: parse_session_state(&text(row.get_value(7)?, "state")?),
            exit_code: optional_integer(row.get_value(8)?, "exit_code")?,
            buffer: text(row.get_value(9)?, "buffer")?,
            screen: None,
            embedded_session: None,
            embedded_session_correlation_id: None,
            agent_attention_state: None,
        }))
    }

    pub async fn list_session_states(&self) -> Result<Vec<SessionSnapshot>, StoreError> {
        let conn = self.connection().await?;
        let mut rows = conn
            .query(
                "select pane_id, workspace_id, session_id, inner_session_id, kind, cwd, command, state, exit_code, buffer
                 from session_state
                 order by workspace_id, pane_id",
                (),
            )
            .await?;
        let mut sessions = Vec::new();
        while let Some(row) = rows.next().await? {
            sessions.push(SessionSnapshot {
                pane_id: text(row.get_value(0)?, "pane_id")?,
                workspace_id: text(row.get_value(1)?, "workspace_id")?,
                id: text(row.get_value(2)?, "session_id")?,
                inner_session_id: optional_text(row.get_value(3)?, "inner_session_id")?,
                kind: parse_terminal_kind(&text(row.get_value(4)?, "kind")?),
                cwd: text(row.get_value(5)?, "cwd")?,
                command: text(row.get_value(6)?, "command")?,
                state: parse_session_state(&text(row.get_value(7)?, "state")?),
                exit_code: optional_integer(row.get_value(8)?, "exit_code")?,
                buffer: text(row.get_value(9)?, "buffer")?,
                screen: None,
                embedded_session: None,
                embedded_session_correlation_id: None,
                agent_attention_state: None,
            });
        }
        Ok(sessions)
    }
}
