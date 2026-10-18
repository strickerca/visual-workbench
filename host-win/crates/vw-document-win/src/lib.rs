//! Owned Windows document worker. No shell, system PDFium search, reusable
//! untrusted process, ambient user-file grant, or automatic privilege elevation.
mod metadata;
mod policy;
pub use metadata::{DocumentFormat, Glyph, Notice, ResultMetadata};
pub use policy::{ArtifactEntry, RuntimeManifest, WorkerLimits};
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{
    PinnedRuntime, WorkerReply, cleanup_pending, retry_cleanup, retry_runtime_cleanup, run,
    verify_worker_token,
};

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    #[cfg(windows)]
    pub(crate) fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid document worker request or artifact manifest")]
    Invalid,
    #[error("document worker resource limit exceeded")]
    Limit,
    #[error("document worker is busy")]
    Busy,
    #[error("document worker cancelled")]
    Cancelled,
    #[error("document worker timed out")]
    Timeout,
    #[error("document runtime identity or path validation failed")]
    Integrity,
    #[error("document worker isolation could not be established")]
    Isolation,
    #[error("document worker cleanup remains pending; new launches are blocked")]
    CleanupPending,
    #[error("document worker returned an unsuccessful exit code")]
    WorkerExit,
    #[error("document worker output or diagnostics exceeded its limit")]
    Output,
    #[error("document worker operating-system call failed ({0})")]
    Windows(u32),
    #[error("document worker private I/O failed ({0:?})")]
    Io(std::io::ErrorKind),
    #[error(transparent)]
    Protocol(#[from] vw_document_protocol::ProtocolError),
}
pub type Result<T> = std::result::Result<T, Error>;
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.kind())
    }
}
