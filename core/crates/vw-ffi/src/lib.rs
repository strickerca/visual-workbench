//! Shared UniFFI boundary. Storage and input run on distinct bounded core-owned workers.
mod camera;
mod creation;
mod dto;
mod editor;
mod gesture;
mod payload;
mod project;
mod queries;
mod streaming_ffi;
mod worker;
pub use streaming_ffi::*;
pub mod session;
pub use camera::*;
pub use dto::*;
pub use gesture::*;
pub use project::*;
pub use queries::layout_text;
pub use session::*;
pub use worker::Cancellation;

uniffi::setup_scaffolding!();

#[derive(Clone, Debug, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    #[error("invalid bounded input")]
    Invalid,
    #[error("core queue is full; retain this batch and retry after completion")]
    Backpressure,
    #[error("object or project is closed")]
    Closed,
    #[error("operation cancelled before its commit point")]
    Cancelled,
    #[error("storage failed; reopen and inspect the durable revision")]
    Storage,
    #[error("unsupported operation or document feature")]
    Unsupported,
    #[error("core worker unavailable")]
    Worker,
    #[error("stroke rejected")]
    Stroke,
    #[error("raster operation rejected")]
    Raster,
}
pub type Result<T> = std::result::Result<T, CoreError>;
impl From<vw_model::ModelError> for CoreError {
    fn from(_: vw_model::ModelError) -> Self {
        Self::Invalid
    }
}
impl From<std::io::Error> for CoreError {
    fn from(_: std::io::Error) -> Self {
        Self::Storage
    }
}
impl From<vw_ops::OpsError> for CoreError {
    fn from(_: vw_ops::OpsError) -> Self {
        Self::Invalid
    }
}
impl From<vw_geom::GeometryError> for CoreError {
    fn from(_: vw_geom::GeometryError) -> Self {
        Self::Invalid
    }
}
impl From<vw_store::StoreError> for CoreError {
    fn from(_: vw_store::StoreError) -> Self {
        Self::Storage
    }
}
impl From<vw_ink::InkError> for CoreError {
    fn from(_: vw_ink::InkError) -> Self {
        Self::Stroke
    }
}
impl From<vw_raster::RasterError> for CoreError {
    fn from(_: vw_raster::RasterError) -> Self {
        Self::Raster
    }
}

#[uniffi::export]
pub fn binding_contract_version() -> u32 {
    1
}
#[uniffi::export]
pub fn generate_id(unix_ms: u64) -> Result<String> {
    Ok(vw_model::Id::generate(unix_ms)?.to_string())
}
#[uniffi::export]
pub fn generate_device_id() -> Result<String> {
    Ok(vw_model::DeviceId::generate()?.to_string())
}

// SAFETY: unique pointer-free legacy toolchain symbol; no allocation or unwind.
#[unsafe(no_mangle)]
pub extern "C" fn vw_toolchain_smoke_version() -> u32 {
    binding_contract_version()
}
