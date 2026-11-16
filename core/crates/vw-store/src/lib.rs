//! Crash-consistent project persistence, immutable blobs and bounded caches.

mod archive;
mod blobs;
mod cache;
mod conflict_review;
mod database;
mod local_views;
mod migrations;
pub mod os_fs;
mod projections;
mod verification_review;
pub use archive::*;
pub use blobs::*;
pub use cache::*;
pub use conflict_review::*;
pub use database::*;
pub use local_views::*;
pub use verification_review::*;

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
    /// The stored log is as it was written, but this build does not decide one
    /// of its transactions the way the build that stored it did. Nothing is
    /// wrong with the project and nothing in it was changed; the build that
    /// saved it opens it.
    #[error("project was last saved by a different version of the app")]
    EngineMismatch,
    #[error("not enough free space for caching; original assets are preserved")]
    LowSpace,
}
