use thiserror::Error;

#[derive(Error, Debug)]
pub enum SessionStoreError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("pool error: {0}")]
    Pool(#[from] r2d2::Error),

    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("migration error: {0}")]
    Migration(String),
}
