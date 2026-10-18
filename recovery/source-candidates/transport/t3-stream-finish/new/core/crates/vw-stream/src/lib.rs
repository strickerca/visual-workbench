//! Bounded capture-stream primitives. Authentication, OS capture, transport and
//! actual display presentation remain explicit adapter responsibilities.
mod consent;
mod frame;
mod freeze;
mod lossless;
mod pacer;
mod ring;
mod tiles;

pub use consent::*;
pub use frame::*;
pub use freeze::*;
pub use lossless::*;
pub use pacer::*;
pub use ring::*;
pub use tiles::*;

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StreamError {
    #[error("invalid stream data")]
    Invalid,
    #[error("stream geometry or frame changed")]
    Stale,
    #[error("stream resource limit")]
    Limit,
    #[error("stream source unavailable")]
    Unavailable,
    #[error("stream keyframe required")]
    KeyframeRequired,
    #[error("stream capture consent required")]
    Consent,
    #[error("stream operation cancelled")]
    Cancelled,
    #[error("stream codec refused data")]
    Codec,
}
pub type Result<T> = std::result::Result<T, StreamError>;

#[derive(Default, Debug)]
pub struct Cancellation(AtomicBool);
impl Cancellation {
    pub fn cancel(&self) { self.0.store(true, Ordering::Release); }
    pub fn check(&self) -> Result<()> {
        if self.0.load(Ordering::Acquire) { Err(StreamError::Cancelled) } else { Ok(()) }
    }
}

pub const MAX_SIDE: u32 = 8192;
pub const MAX_PIXELS: u64 = 50_000_000;
pub const MAX_RING_BYTES: u64 = 300_000_000;
pub const DEFAULT_RING_FRAMES: usize = 8;
pub const MAX_TILES: usize = 128;
// Leave explicit Envelope/protobuf headroom within the existing 8 MiB MEDIA cap.
pub const MAX_TILE_BYTES: usize = 8 * 1024 * 1024 - 64 * 1024;

pub fn rgba_bytes(width: u32, height: u32) -> Result<usize> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE || pixels > MAX_PIXELS {
        return Err(StreamError::Limit);
    }
    usize::try_from(pixels.checked_mul(4).ok_or(StreamError::Limit)?).map_err(|_| StreamError::Limit)
}
