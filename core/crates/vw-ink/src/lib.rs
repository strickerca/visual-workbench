//! Provisional deterministic stroke-engine candidate, algorithm version 1.
//!
//! This software does not decide D6 or certify S23/S Pen feel or latency. Samples
//! are document-space coordinates. Contours form one compound NONZERO fill: draw
//! them together once, never independently alpha-blend their overlaps. The same
//! quantized contours serve live ink, committed ink, export and hit testing.
//!
//! Filtering is causal and append-only. Pen-up performs no resampling or tail
//! correction, so its geometry is byte-identical to the last live update. Input
//! prediction belongs in a separate disposable builder, never in saved samples.

mod brush;
mod builder;
pub mod eraser;
mod geometry;

pub use brush::{Brush, BrushFamily, PressureCurve, Sample};
pub use builder::{
    GeometryRange, StrokeBuilder, StrokeState, geometry_from_stroke, samples_from_stroke,
};
pub use geometry::{Bounds, Geometry, Polygon, QPoint};

/// Canonical geometry algorithm. Unknown versions require a different engine.
pub const ALGORITHM_VERSION: u32 = 1;
/// Every output coordinate is an exact integer multiple of 1/256 document unit.
pub const QUANTIZATION: f64 = 256.0;
/// Input coordinates and samples have explicit limits before geometry allocation.
pub const MAX_COORDINATE: f64 = 1_000_000_000.0;
pub const MAX_SAMPLES: usize = 1_000_000;
pub const MAX_VERTICES: usize = 4_000_000;
pub const MAX_BASE_WIDTH: f64 = 4_096.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InkError {
    #[error("unsupported stroke algorithm version {0}")]
    UnsupportedVersion(u32),
    #[error("invalid stroke input: {0}")]
    Invalid(&'static str),
    #[error("stroke samples are out of time order")]
    TimeOrder,
    #[error("stroke is already finished or cancelled")]
    Closed,
    #[error("stroke has no samples")]
    EmptyStroke,
    #[error("stroke exceeds the geometry resource limit")]
    ResourceLimit,
    #[error("stroke geometry allocation failed")]
    Allocation,
}

pub(crate) fn valid_coordinate(value: f64) -> bool {
    value.is_finite() && (-MAX_COORDINATE..=MAX_COORDINATE).contains(&value)
}
