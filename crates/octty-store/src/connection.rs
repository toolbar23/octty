use std::path::Path;

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
}
