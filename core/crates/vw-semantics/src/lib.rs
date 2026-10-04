//! Platform-neutral semantic capture admission, binding, snapping and exports.
//! Platform collection and durable store integration remain caller-owned.
mod binding;
mod codec;
mod export;
mod snap;
mod snapshot;

pub use binding::{CaptureBinding, CaptureContext, CaptureView, EditMetadata, SnapshotPlan};
pub use export::{ElementReference, ReferenceExport, UNTRUSTED_NOTICE};
pub use snap::{Snap, SnapQuery};
pub use snapshot::{BoundsSpace, CapturedElement, Element, Platform, Snapshot};

pub const MAX_ELEMENTS: usize = 4096;
pub const MAX_DEPTH: usize = 128;
pub const MAX_TEXT_CHARACTERS: usize = 200;
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_COMPRESSED_BYTES: usize = MAX_JSON_BYTES + 128 * 1024;
pub const MAX_REFERENCES: usize = 64;
pub const SNAP_SCREEN_PIXELS: f64 = 12.0;
pub const UIA_CAPTURE_TARGET_MS: u64 = 300;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("semantic data exceeds {0} limit")]
    Limit(&'static str),
    #[error("invalid semantic {0}")]
    Invalid(&'static str),
    #[error("semantic capture or revision is stale")]
    Stale,
    #[error("semantic reference is unavailable")]
    Missing,
    #[error("semantic author is not authenticated")]
    Unauthorized,
    #[error("semantic operation cancelled")]
    Cancelled,
    #[error("semantic compression is invalid")]
    Codec,
    #[error("semantic JSON is invalid")]
    Json,
    #[error(transparent)]
    Model(#[from] vw_model::ModelError),
    #[error(transparent)]
    Ops(#[from] vw_ops::OpsError),
    #[error(transparent)]
    Geometry(#[from] vw_geom::GeometryError),
}
pub trait Cancellation {
    fn is_cancelled(&self) -> bool;
}
pub struct NeverCancel;
impl Cancellation for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}
impl Cancellation for std::sync::atomic::AtomicBool {
    fn is_cancelled(&self) -> bool {
        self.load(std::sync::atomic::Ordering::Acquire)
    }
}
pub(crate) fn check(cancel: &dyn Cancellation) -> Result<()> {
    if cancel.is_cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
pub(crate) fn text(value: &str, max: usize, nonempty: bool) -> Result<usize> {
    if value.len() > max {
        return Err(Error::Limit("text"));
    }
    if (nonempty && value.is_empty()) || value.contains('\0') {
        return Err(Error::Invalid("text"));
    }
    Ok(value.len())
}
