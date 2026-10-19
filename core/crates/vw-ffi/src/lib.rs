//! Shared UniFFI boundary. Storage and input run on distinct bounded core-owned workers.
mod os_images;
pub use os_images::*;
mod session_resources;
pub use session_resources::*;
mod giant_images;
pub use giant_images::*;
mod avif_images;
pub use avif_images::*;
mod format_images;
pub use format_images::OriginalFormatInfo;
mod ai_edit;
mod assisted_source;
pub use assisted_source::{
    AssistedSourcePixels, AssistedSourcePixelsDelivery, AssistedSourcePixelsInfo,
};
mod camera;
mod documents;
pub use documents::*;
mod precision;
mod stylus_tilt;
pub use ai_edit::*;
pub use precision::*;
pub use stylus_tilt::*;
mod creation;
mod dto;
mod editor;
mod gesture;
mod instructions;
mod packages;
mod verification;
pub use verification::*;
mod payload;
pub use packages::*;
mod project;
mod semantic_ui;
pub use semantic_ui::*;
mod capture_delivery;
mod capture_import;
pub use capture_delivery::*;
mod queries;
mod semantics;
mod streaming_ffi;
pub use capture_import::*;
mod local_views;
mod worker;
pub use local_views::{LocalViewsBinding, LocalViewsMutation, LocalViewsSnapshot, RegionBookmark};
mod document_controls;
pub use document_controls::{
    DocumentControlCommand, DocumentControlRow, DocumentControlSnapshot, DocumentControlTargets,
    GroupControlRow, LayerControlRow, LayerControlUpdate, ObjectControlRow,
};
mod object_rotation;
pub use object_rotation::rotate_object_transforms;
mod image_frames;
pub use image_frames::{
    ImageEditingBackground, ImageEditingPreview, ImageFrameCommand, ImageFrameEntry,
    ImageFramePreview, ImageFrameSettings, ImageFramesSnapshot,
};
mod tone_controls;
pub use tone_controls::{
    ImageToneSettings, ToneControlCommand, ToneControlEntry, ToneControlPreview,
    ToneControlSnapshot,
};
mod offline_review;
pub use offline_review::*;
mod workflow;
pub use streaming_ffi::*;
pub mod session;
pub use camera::*;
pub use dto::*;
pub use gesture::*;
pub use instructions::*;
pub use project::*;
pub use queries::layout_text;
pub use semantics::*;
pub use session::*;
pub use worker::Cancellation;
pub use workflow::*;
mod eraser;
pub use eraser::*;

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
    #[error("an installed decoder for this admitted image is unavailable")]
    ImageCodecUnavailable,
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
    fn from(error: vw_raster::RasterError) -> Self {
        match error {
            vw_raster::RasterError::Cancelled => Self::Cancelled,
            vw_raster::RasterError::CodecUnavailable => Self::ImageCodecUnavailable,
            _ => Self::Raster,
        }
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

mod masks;
pub use masks::*;

/// Called only after bounded decoding admitted the original container.
pub(crate) fn original_image_label(bytes: &[u8]) -> &'static str {
    if let Some(label) = format_images::label(bytes) {
        label
    } else if vw_avif_os::is_avif(bytes) {
        "avif"
    } else if vw_codec_os::is_heic(bytes) {
        "heic"
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        "jpeg"
    } else {
        "webp"
    }
}
