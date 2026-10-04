//! Revision-bound instruction workflow adapters. Canonical state, transaction
//! application, history, undo, synchronization and persistence remain in their
//! existing model/ops/store crates. No OS recognition or network IO lives here.
#![forbid(unsafe_code)]

mod entry;
mod export;
mod focus;
mod plan;
mod view;
pub use entry::*;
pub use export::*;
pub use focus::*;
pub use plan::*;
pub use view::*;

use serde::{Deserialize, Serialize};
use vw_proto::v1;

pub const MAX_MARKERS: usize = 512;
pub const MAX_DOCUMENT_OBJECTS: usize = 16_384;
pub const MAX_INSTRUCTIONS: usize = 2048;
pub const MAX_TARGETS: usize = 64;
pub const MAX_TEXT_BYTES: usize = 32 * 1024;
pub const MAX_TOTAL_TEXT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_ELEMENT_REFS: usize = 64;
pub const MAX_ELEMENT_ID_BYTES: usize = 4096;
pub const MAX_FOCUS_BYTES: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid instruction workflow input: {0}")]
    Invalid(&'static str),
    #[error("instruction workflow resource limit: {0}")]
    Limit(&'static str),
    #[error("instruction workflow references missing content")]
    Missing,
    #[error("instruction workflow revision changed; refresh before retrying")]
    Stale,
    #[error("marker numbering or instruction links need explicit reconciliation")]
    Reconciliation,
    #[error("instruction targets include deleted objects")]
    Detached,
    #[error("instruction or containing layer is locked")]
    Locked,
    #[error("entry session is closed or its focus changed")]
    Closed,
    #[error("focus sender, connection or payload does not match")]
    Unauthorized,
    #[error("sequence or counter exhausted")]
    Exhausted,
    #[error("canonical model validation failed")]
    Model(#[from] vw_model::ModelError),
    #[error("canonical transaction was rejected")]
    Operation(#[from] vw_ops::OpsError),
    #[error("invalid document geometry")]
    Geometry(#[from] vw_geom::GeometryError),
}
pub type Result<T> = std::result::Result<T, Error>;

/// Provenance is supplied explicitly by the platform entry adapter, never
/// inferred from text contents or claimed as evidence of OS recognition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryMethod {
    PcKeyboard,
    PhoneKeyboard,
    Voice,
    Handwriting,
}
impl EntryMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PcKeyboard => "pc_keyboard",
            Self::PhoneKeyboard => "phone_keyboard",
            Self::Voice => "voice",
            Self::Handwriting => "handwriting",
        }
    }
    pub fn from_canonical(value: &str) -> Result<Self> {
        match value {
            "pc_keyboard" => Ok(Self::PcKeyboard),
            "phone_keyboard" => Ok(Self::PhoneKeyboard),
            "voice" => Ok(Self::Voice),
            "handwriting" => Ok(Self::Handwriting),
            _ => Err(Error::Invalid("entry method")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    None,
    Change,
    Preserve,
    Reference,
    Explain,
}
impl Role {
    pub fn from_canonical(value: i32) -> Result<Self> {
        match v1::Role::try_from(value) {
            Ok(v1::Role::None) => Ok(Self::None),
            Ok(v1::Role::Change) => Ok(Self::Change),
            Ok(v1::Role::Preserve) => Ok(Self::Preserve),
            Ok(v1::Role::Reference) => Ok(Self::Reference),
            Ok(v1::Role::Explain) => Ok(Self::Explain),
            _ => Err(Error::Invalid("role")),
        }
    }
    pub fn canonical(self) -> i32 {
        match self {
            Self::None => v1::Role::None as i32,
            Self::Change => v1::Role::Change as i32,
            Self::Preserve => v1::Role::Preserve as i32,
            Self::Reference => v1::Role::Reference as i32,
            Self::Explain => v1::Role::Explain as i32,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Change => "change",
            Self::Preserve => "preserve",
            Self::Reference => "reference",
            Self::Explain => "explain",
        }
    }
}

pub(crate) fn text(value: &str) -> Result<()> {
    if value.len() > MAX_TEXT_BYTES {
        return Err(Error::Limit("instruction text"));
    }
    if value.contains('\0') {
        return Err(Error::Invalid("instruction text"));
    }
    Ok(())
}
pub(crate) fn language(value: &str) -> Result<()> {
    // Preserve the platform-supplied language tag literally. Empty means unknown;
    // this is a bounded label, not a claim of complete BCP-47 registry validation.
    if value.len() > 128 || value.chars().any(char::is_control) {
        return Err(Error::Invalid("language label"));
    }
    Ok(())
}
