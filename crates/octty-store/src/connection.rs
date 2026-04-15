use std::{path::Path, time::Duration};

use turso::{Builder, Connection};

use crate::{StoreError, TursoStore};

impl TursoStore {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        if let Some(parent) = path.as_ref().parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let db = Builder::new_local(path.as_ref().to_string_lossy().as_ref())
            .build()
            .await?;
        let conn = db.connect()?;
        conn.busy_timeout(Duration::from_secs(5))?;
        enable_concurrent_mode(&conn).await?;
        let store = Self { db };
        store.migrate().await?;
        Ok(store)
    }

    pub async fn open_memory() -> Result<Self, StoreError> {
        let db = Builder::new_local(":memory:").build().await?;
        let conn = db.connect()?;
        conn.busy_timeout(Duration::from_secs(5))?;
        enable_concurrent_mode(&conn).await?;
        let store = Self { db };
        store.migrate().await?;
        Ok(store)
    }

    pub async fn connection(&self) -> Result<Connection, StoreError> {
        let conn = self.db.connect()?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Ok(conn)
    }
}

async fn enable_concurrent_mode(conn: &Connection) -> Result<(), StoreError> {
    let mut rows = conn.query("PRAGMA journal_mode = 'mvcc'", ()).await?;
    while rows.next().await?.is_some() {}
    Ok(())
}
