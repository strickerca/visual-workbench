//! Shared coordinate math, implementing BUILD_SPECIFICATION sections 4.2 and 4.16.
//!
//! "Raw asset space R" is the stored decoded grid before orientation. "Document
//! space D" has a top-left origin, x right and y down; its units are oriented
//! source pixels, PDF points, or SVG units after the intrinsic viewBox mapping.
//! "View space V" is a device-local similarity transform of D. "Screen space P"
//! uses physical device pixels after display rotation and insets. "Lens space L"
//! has its own transform, latched at pen-down. "Frame space F" is a captured
//! physical-pixel grid; "host input space H" is the Windows virtual screen and
//! may have a negative origin. F to H is valid only for its geometry revision.
//! "Android display space A" uses phone display pixels in natural orientation
//! plus rotation; full-panel A to P does not apply content insets a second time.
//!
//! Pixel (i,j) occupies [i,i+1) x [j,j+1); its center is (i+0.5,j+0.5).
//! Affines transform continuous edges/centers, while capture pixel-index methods
//! use integer arithmetic. DPI is metadata on an already physical capture.
//! Transcendentals and snapping use libm, never platform math intrinsics.
#![deny(missing_docs)]

mod affine;
mod asset;
mod capture;
mod display;
mod lens;

pub use affine::{Affine, Camera, Point, Rect, Size, snap_edge, snap_pixel_center};
pub use asset::{ExifOrientation, PdfPage, SvgFit, SvgViewport};
pub use capture::{CaptureGeometry, GeometryRevision, HostRect, PixelPoint};
pub use display::{Insets, QuarterTurn, ScreenTransform};
pub use lens::LensTransform;

/// Invalid or unrepresentable geometry is rejected before crossing an input API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GeometryError {
    /// An input or computed coordinate is NaN or infinite.
    #[error("geometry contains a non-finite value")]
    NonFinite,
    /// A dimension is zero or negative where a nonempty viewport is required.
    #[error("geometry requires a positive extent")]
    EmptyExtent,
    /// A rectangle has a negative dimension.
    #[error("rectangle dimensions cannot be negative")]
    NegativeExtent,
    /// A scale is zero or negative.
    #[error("scale must be positive")]
    InvalidScale,
    /// An affine's linear part has no representable inverse.
    #[error("affine transform is singular")]
    Singular,
    /// Insets consume the viewport or contain negative values.
    #[error("invalid viewport insets")]
    InvalidInsets,
    /// Only the eight EXIF orientations or quarter-turn PDF rotations are valid.
    #[error("unsupported orientation")]
    InvalidOrientation,
    /// Neither a viewBox nor content bounds was supplied for an SVG.
    #[error("SVG bounds are unavailable")]
    MissingSvgBounds,
    /// A pixel lies outside the corresponding frame or host client rectangle.
    #[error("pixel is outside capture bounds")]
    OutOfBounds,
    /// A host pixel coordinate cannot be represented as an i32.
    #[error("integer coordinate overflow")]
    IntegerOverflow,
    /// The caller is using a capture made before the geometry changed.
    #[error("stale capture geometry: expected {expected:?}, actual {actual:?}")]
    StaleGeometry {
        /// Geometry revision carried by the capture.
        expected: GeometryRevision,
        /// Current revision supplied by the input caller.
        actual: GeometryRevision,
    },
    /// A second pen-down arrived while the transform was already latched.
    #[error("lens transform is already latched")]
    AlreadyLatched,
    /// Pen-up or cancel arrived without a matching latch.
    #[error("lens transform is not latched")]
    NotLatched,
    /// Precision-lens magnification must be two, four or eight.
    #[error("unsupported precision lens magnification")]
    InvalidMagnification,
}

fn finite(values: &[f64]) -> Result<(), GeometryError> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(GeometryError::NonFinite)
    }
}
