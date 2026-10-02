//! Host-authoritative, atomic transactions over the shared document model.
//! Persistence is supplied by vw-store; network delivery is supplied by vw-net.

mod gesture;
mod host;
mod patch;
mod plan;
mod replica;
mod undo;
pub use gesture::GestureStore;
pub use host::{Acceptance, HostSequencer, HostSnapshot};
pub use replica::{PendingTransaction, Replica};
pub use undo::UndoManager;

/// Rejections are typed and do not include captured text or private payloads.
#[derive(Debug, thiserror::Error)]
pub enum OpsError {
    #[error(transparent)]
    Model(#[from] vw_model::ModelError),
    #[error("malformed transaction state")]
    Serialization(#[from] serde_json::Error),
    #[error("missing or invalid transaction field: {0}")]
    Invalid(&'static str),
    #[error("entity not found")]
    NotFound,
    #[error("entity already exists")]
    AlreadyExists,
    #[error("transaction identifier was reused with different content")]
    IdCollision,
    #[error("transaction identity does not match the authenticated device")]
    DeviceMismatch,
    #[error("base revision is unknown or has a different hash")]
    RevisionMismatch,
    #[error("entity or containing layer is locked")]
    Locked,
    #[error("sequence or Lamport counter exhausted")]
    CounterExhausted,
    #[error("a transaction can only undo its own device's accepted history")]
    UndoOwnership,
}
