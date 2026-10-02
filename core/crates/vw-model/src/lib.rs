//! Shared document state and canonical identifiers (specification 4.1 and 4.3).
//! No original asset bytes or device-local camera state are mutated here.

mod ids;
mod order;
mod state;
mod validate;
pub use ids::{AssetId, DeviceId, Id, StateHash};
pub use order::OrderKey;
pub use state::{
    Document, Group, Instruction, Layer, Object, PdfPage, Project, ResultCandidate,
    SemanticSnapshot,
};
pub use validate::{validate_asset, validate_capture, validate_object_state, validate_stroke};

/// Validation failures never include captured text or private source identifiers.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    /// A UUID, device ID or content hash is not in its canonical representation.
    #[error("invalid {0} identifier")]
    InvalidId(&'static str),
    /// The platform random source failed; no weaker fallback is used.
    #[error("secure random source unavailable")]
    Entropy,
    /// Timeline and unrecognized document kinds are reserved, not loadable.
    #[error("unsupported document kind {0}")]
    UnsupportedKind(i32),
    /// Loading a schema this build does not understand must fail explicitly.
    #[error("unsupported document schema {0}")]
    UnsupportedSchema(u32),
    /// A named field violates its range or shape contract.
    #[error("invalid {0}")]
    Invalid(&'static str),
    /// A required field is absent.
    #[error("missing {0}")]
    Missing(&'static str),
    /// A state JSON encoding is malformed.
    #[error("invalid serialized model")]
    Serialization(#[from] serde_json::Error),
    /// The shared geometry contract rejected a value.
    #[error(transparent)]
    Geometry(#[from] vw_geom::GeometryError),
}
