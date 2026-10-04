//! Deterministic, bounded VIP compilation. No network, credentials, capture,
//! filesystem publication or mutable project authority lives here.
#![forbid(unsafe_code)]

mod admission;
mod bounded;
mod compiler;
mod config;
mod geometry;
#[cfg(test)]
mod goldens;
mod image_receipt;
mod manifest;
mod pixels;
mod prompt;
mod verify;

pub use compiler::{CompileInput, compile};
pub use config::*;
pub use manifest::*;
pub use verify::Package;
pub use vw_instructions::UNTRUSTED_TEXT_NOTICE;
pub use vw_semantics::{Cancellation, NeverCancel};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid package {0}")]
    Invalid(&'static str),
    #[error("package resource limit: {0}")]
    Limit(&'static str),
    #[error("package source or revision binding changed")]
    Stale,
    #[error("package requires an available original asset")]
    Original,
    #[error("unsupported package input: {0}")]
    Unsupported(&'static str),
    #[error("package compilation cancelled")]
    Cancelled,
    #[error("package JSON or PNG encoding is invalid")]
    Encoding,
    #[error("package hash or inventory verification failed")]
    Integrity,
    #[error("canonical model admission failed")]
    Model(#[from] vw_model::ModelError),
    #[error("instruction projection was refused")]
    Instructions(#[from] vw_instructions::Error),
    #[error("semantic projection was refused")]
    Semantics(#[from] vw_semantics::Error),
    #[error("raster admission or rendering was refused")]
    Raster(#[from] vw_raster::RasterError),
}
pub type Result<T> = std::result::Result<T, Error>;
pub(crate) fn check(cancel: &dyn Cancellation) -> Result<()> {
    if cancel.is_cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
