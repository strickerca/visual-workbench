//! Bounded shared AI edit preparation, explicit-send accounting and local proof.
//! No network, credential, upload or spending implementation is present here.
#![forbid(unsafe_code)]

pub mod budget;
pub mod config;
pub mod geometry;
mod memory;
mod metrics;
mod pixels;
pub mod proof;
mod request;
pub use pixels::EditImage;
pub use request::*;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_PIXELS: u64 = 20_000_000;
pub const MAX_EDGE: u32 = 16_384;
pub const MAX_FEATHER: u32 = 64;
pub const DEFAULT_FEATHER: u32 = 8;
pub const MAX_ENCODED: usize = 64 * 1024 * 1024;
pub const MAX_MEMORY: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid AI edit input: {0}")]
    Invalid(&'static str),
    #[error("AI edit resource limit: {0}")]
    Limit(&'static str),
    #[error("AI image encoding or decoding failed")]
    Codec,
    #[error("AI source color profile is unsupported")]
    Color,
    #[error("source depth reduction for the provider requires explicit permission")]
    Depth,
    #[error("AI edit was cancelled")]
    Cancelled,
    #[error("outside-mask proof failed: {0} changed pixels")]
    Exterior(u64),
    #[error("explicit confirmation must match the reviewed request and estimate")]
    Confirmation,
    #[error("daily soft budget warning requires explicit acknowledgement")]
    SoftBudget,
    #[error("an unresolved reservation or attempt requires review")]
    Unresolved,
    #[error("the request's one allowed attempt is already consumed")]
    AttemptConsumed,
    #[error("budget ledger is corrupt, incompatible or unavailable; do not reset it")]
    Storage,
    #[error("budget clock moved backwards")]
    ClockRegression,
    #[error("unsupported provider mapping")]
    Unsupported,
}
pub type Result<T> = std::result::Result<T, Error>;
impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}
impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::Storage
    }
}
impl From<serde_json::Error> for Error {
    fn from(_: serde_json::Error) -> Self {
        Self::Invalid("JSON data")
    }
}
impl From<image::ImageError> for Error {
    fn from(_: image::ImageError) -> Self {
        Self::Codec
    }
}
impl From<vw_mask::MaskError> for Error {
    fn from(_: vw_mask::MaskError) -> Self {
        Self::Invalid("mask data or limits")
    }
}
impl From<vw_raster::RasterError> for Error {
    fn from(_: vw_raster::RasterError) -> Self {
        Self::Codec
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        result.push(char::from(HEX[(b >> 4) as usize]));
        result.push(char::from(HEX[(b & 15) as usize]));
    }
    result
}
pub fn sha256(bytes: &[u8]) -> String {
    hex(Sha256::digest(bytes).as_ref())
}
pub(crate) fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&serde_json::to_value(value)?)?)
}
pub(crate) fn pixel_count(width: u32, height: u32) -> Result<usize> {
    let count = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE || count > MAX_PIXELS {
        return Err(Error::Limit("document dimensions"));
    }
    usize::try_from(count).map_err(|_| Error::Limit("address space"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub memory_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_bytes: 512 * 1024 * 1024,
        }
    }
}
impl Limits {
    pub(crate) fn check(self, estimate: u64) -> Result<()> {
        if self.memory_bytes == 0 || self.memory_bytes > MAX_MEMORY || estimate > self.memory_bytes
        {
            Err(Error::Limit(
                "working memory; use a smaller region or a tiled adapter",
            ))
        } else {
            Ok(())
        }
    }
}
/// Cooperative cancellation is checked between CPU/codec stages and inside
/// AI pixel/metric loops. Codecs and bounded vw-mask morphology cannot be
/// interrupted halfway through their calls.
pub trait Cancellation: Send + Sync {
    fn is_cancelled(&self) -> bool;
}
impl Cancellation for std::sync::atomic::AtomicBool {
    fn is_cancelled(&self) -> bool {
        self.load(std::sync::atomic::Ordering::Acquire)
    }
}
pub struct NeverCancel;
impl Cancellation for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}
pub(crate) fn check_cancel(cancel: &dyn Cancellation) -> Result<()> {
    if cancel.is_cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
