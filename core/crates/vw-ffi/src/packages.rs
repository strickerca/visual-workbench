//! Exact visible snapshot compilation and private immutable package catalog.
//! Neither compilation nor publication grants capture or sends an agent message.
mod catalog;
mod compile;
mod dto;
mod files;
mod inbox;
mod retirement;
pub use inbox::*;

use crate::{Cancellation, CoreError, WorkflowError};
pub use catalog::{PackageCatalog, PackageRetirement, open_package_catalog};
pub use compile::CompiledPackage;
pub use dto::*;
use std::sync::atomic::{AtomicBool, Ordering};
const MAX_MEMORY: u64 = 256 * 1024 * 1024;
const MAX_PACKAGE: usize = 32 * 1024 * 1024;
const MAX_SOURCE: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 64;
const MAX_DISK: u64 = 2 * 1024 * 1024 * 1024;
fn check(closed: &AtomicBool, cancel: &Cancellation) -> PackageResult<()> {
    if closed.load(Ordering::Acquire) {
        return Err(PackageError::Closed);
    }
    if cancel.is_cancelled() {
        return Err(PackageError::Cancelled);
    }
    Ok(())
}
fn limits(memory: u64) -> vw_package::Limits {
    vw_package::Limits {
        memory_bytes: memory,
        source_bytes: MAX_SOURCE,
        source_pixels: 50_000_000,
        package_bytes: MAX_PACKAGE,
        image_bytes: 4 * 1024 * 1024,
        markers: 64,
    }
}
impl From<CoreError> for PackageError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::Closed => Self::Closed,
            CoreError::Cancelled => Self::Cancelled,
            CoreError::Backpressure | CoreError::Worker => Self::Busy,
            CoreError::Storage => Self::Storage,
            CoreError::Unsupported => Self::Unsupported,
            _ => Self::Invalid,
        }
    }
}
impl From<WorkflowError> for PackageError {
    fn from(e: WorkflowError) -> Self {
        match e {
            WorkflowError::Closed => Self::Closed,
            WorkflowError::Cancelled => Self::Cancelled,
            WorkflowError::Stale => Self::Stale,
            WorkflowError::Memory { .. } | WorkflowError::Limit => Self::Limit,
            WorkflowError::Missing => Self::Original,
            WorkflowError::Storage => Self::Storage,
            WorkflowError::Backpressure => Self::Busy,
            _ => Self::Invalid,
        }
    }
}
impl From<vw_model::ModelError> for PackageError {
    fn from(_: vw_model::ModelError) -> Self {
        Self::Invalid
    }
}
impl From<vw_store::StoreError> for PackageError {
    fn from(_: vw_store::StoreError) -> Self {
        Self::Original
    }
}
impl From<std::io::Error> for PackageError {
    fn from(_: std::io::Error) -> Self {
        Self::Storage
    }
}
impl From<vw_package::Error> for PackageError {
    fn from(e: vw_package::Error) -> Self {
        use vw_package::Error as E;
        match e {
            E::Cancelled => Self::Cancelled,
            E::Limit(_) => Self::Limit,
            E::Stale => Self::Stale,
            E::Original => Self::Original,
            E::Unsupported(_) => Self::Unsupported,
            E::Integrity | E::Encoding => Self::Integrity,
            E::Raster(vw_raster::RasterError::Depth) => Self::Depth,
            E::Raster(
                vw_raster::RasterError::Memory { .. } | vw_raster::RasterError::Allocation,
            ) => Self::Limit,
            E::Raster(vw_raster::RasterError::Cancelled) => Self::Cancelled,
            _ => Self::Invalid,
        }
    }
}
