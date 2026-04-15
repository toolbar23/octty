use thiserror::Error;
use turso::{Connection, Value};

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
    conn: Connection,
}

mod codecs;
mod connection;
mod migrations;
mod pane_activity;
mod paths;
mod project_roots;
mod sessions;
mod snapshots;
mod workspaces;

pub use paths::default_store_path;

#[cfg(test)]
mod tests;
