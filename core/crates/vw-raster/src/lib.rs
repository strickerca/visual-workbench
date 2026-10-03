//! Source-resolution CPU raster exports. No network, UI or preview fallback.
//! Streaming PNG can use a caller-owned private scratch file.
mod export;
mod ffi;
mod metadata;
mod pixels;
mod render;
mod source;
mod stream;
mod text;
pub use export::{
    AlphaPolicy, ColorPolicy, ExportFormat, ExportMetadata, ExportRequest, ExportedImage, Region,
    export, preflight,
};
pub use ffi::webp_encoder_version;
pub use pixels::{DecodeLimits, DecodedImage, Pixels, decode};
pub use render::{AssetResolver, RenderOptions, render_document};
pub use source::{BorrowedSource, JpegSpool, PngSpool, RasterSource, SourceInfo, SpoolLimits};
pub use stream::{
    MarkedDocument, StreamLimits, StreamPlan, StreamReport, export_document_png_to, export_png_to,
    plan_png,
};
pub use text::{FontFamily, Glyph, OutlineCommand, TextLayout, layout_text};

#[derive(Debug, thiserror::Error)]
pub enum RasterError {
    #[error("invalid raster input: {0}")]
    Invalid(&'static str),
    #[error("unsupported raster input: {0}")]
    Unsupported(&'static str),
    #[error("original source is unavailable; export cannot use a preview")]
    OriginalRequired,
    #[error("{format} dimension limit {limit}px exceeded; use PNG, split, or tiles")]
    Dimensions { format: &'static str, limit: u32 },
    #[error(
        "raster memory estimate {estimated} bytes exceeds budget {budget}; use a smaller region, split, or tiles"
    )]
    Memory { estimated: u64, budget: u64 },
    #[error("depth conversion requires explicit permission; use 16-bit PNG")]
    Depth,
    #[error("JPEG cannot preserve alpha; choose PNG/WebP or an explicit matte")]
    Alpha,
    #[error("raster codec rejected the image")]
    Codec,
    #[error("ICC color conversion failed or profile is unsupported")]
    Color,
    #[error("text requires an unsupported glyph U+{0:04X}")]
    MissingGlyph(u32),
    #[error("raster resource is missing")]
    MissingAsset,
    #[error("allocation failed")]
    Allocation,
    #[error("raster operation cancelled; discard partial output and scratch")]
    Cancelled,
    #[error("raster input/output failed; discard partial output and scratch")]
    Io,
    #[error("encoded output exceeds {limit} bytes; choose a larger output budget, split, or tiles")]
    EncodedLimit { limit: u64 },
    #[error(
        "scratch storage exceeds {limit} bytes; choose a larger scratch budget, split, or tiles"
    )]
    ScratchLimit { limit: u64 },
    #[error("model is invalid: {0}")]
    Model(#[from] vw_model::ModelError),
    #[error("stroke is invalid: {0}")]
    Ink(#[from] vw_ink::InkError),
    #[error("metadata serialization failed")]
    Metadata,
}
pub(crate) fn checked_samples(width: u32, height: u32) -> Result<usize, RasterError> {
    if width == 0 || height == 0 {
        return Err(RasterError::Invalid("zero dimensions"));
    }
    let count = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|v| v.checked_mul(4))
        .ok_or(RasterError::Allocation)?;
    usize::try_from(count).map_err(|_| RasterError::Allocation)
}
