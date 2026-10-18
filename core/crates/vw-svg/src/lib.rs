//! Bounded static SVG engine. Windows callers must use the isolated codec worker.
#![forbid(unsafe_code)]

mod engine;
mod export;
mod limits;
mod metadata;
mod preflight;

pub use engine::{SvgDocument, SvgInfo, SvgTile};
pub use export::{Outline, Point};
pub use limits::{Limits, TileRequest};
pub use metadata::{MAX_EXPORT_METADATA_BYTES, verify_svg_copy_metadata};
pub use preflight::FidelityNotice;

pub type SvgResult<T> = Result<T, SvgError>;

/// Diagnostics intentionally contain no SVG text, paths, URLs, or font names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SvgError {
    #[error("the SVG limits are invalid")]
    InvalidLimits,
    #[error("the SVG exceeds a size or complexity limit")]
    Limit,
    #[error("SVG workspace needs {required} bytes; budget is {budget}")]
    Memory { required: u64, budget: u64 },
    /// Includes marker/pattern definitions, which are excluded until their
    /// per-instance expansion and intermediate surfaces have bounded admission.
    #[error("the SVG is malformed or unsupported")]
    Malformed,
    #[error("XML document type declarations and entities are disabled")]
    DtdDenied,
    #[error("compressed SVG requires an explicit bounded decompression step")]
    CompressedUnsupported,
    #[error("the SVG has cyclic internal references")]
    ReferenceCycle,
    #[error("reference-bearing or escaped CSS is outside the static SVG subset")]
    UnsupportedCss,
    #[error("the SVG document dimensions exceed the supported limit")]
    DocumentSize,
    #[error("the requested SVG raster tile is invalid")]
    InvalidTile,
    #[error("an SVG annotation has invalid geometry")]
    InvalidOverlay,
    #[error("the operation was cancelled")]
    Cancelled,
}
