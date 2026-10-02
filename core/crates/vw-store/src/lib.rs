//! Crash-consistent project persistence, immutable blobs and bounded caches.

mod archive;
mod blobs;
mod cache;
mod database;
mod migrations;
mod projections;
pub use archive::*;
pub use blobs::*;
pub use cache::*;
pub use database::*;

/// Storage errors expose categories without captured text or private paths.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("project filesystem operation failed")]
    Io(#[from] std::io::Error),
    #[error("project database operation failed")]
    Database(#[from] rusqlite::Error),
    #[error("invalid serialized storage data")]
    Json(#[from] serde_json::Error),
    #[error("invalid project archive")]
    Zip(#[from] zip::result::ZipError),
    #[error(transparent)]
    Model(#[from] vw_model::ModelError),
    #[error(transparent)]
    Operation(#[from] vw_ops::OpsError),
    #[error("invalid storage field: {0}")]
    Invalid(&'static str),
    #[error("project integrity check failed: {0}")]
    Corrupt(&'static str),
    #[error("unsupported project schema version {0}")]
    UnsupportedSchema(i64),
    #[error("project is already open for writing")]
    Busy,
    #[error("not enough free space for caching; original assets are preserved")]
    LowSpace,
}
