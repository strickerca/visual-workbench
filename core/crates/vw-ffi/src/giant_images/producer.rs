//! Deliberate promotion boundary for the preserved platform producers.
//!
//! The recovered publisher trusted a caller-selected directory after checking
//! its inventory. An inventory authenticates bytes, not the producer that made
//! those bytes. No path, manifest, receipt file or caller boolean can manufacture
//! approval here. The Android region-decoder / pinned Windows worker callbacks
//! still need root-owned input contracts and platform verification.
use super::source::CapturedSource;
use super::{GiantImageError as Error, GiantImageResult as Result};
use crate::Cancellation;
use vw_assets::{MAX_ENCODED_TILE_BYTES, MAX_MANIFEST_BYTES, Manifest, TILE_SIZE};

pub(super) const MAX_DISK_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const BAND_WORKSPACE_BYTES: u64 = 32 * 1024 * 1024;

/// Internal planning data, not an approval token or a claim of runtime support.
/// Root may lower this through its own explicit producer/import API later.
#[allow(
    dead_code,
    reason = "Bounded staged plan awaits the root-owned producer/import API; no runtime producer is admitted"
)]
pub(crate) struct StagedProducerPlan {
    pub asset_id: String,
    pub original_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub orientation: u8,
    pub format: &'static str,
    pub tile_size: u32,
    pub tile_count: usize,
    pub disk_cache_bytes: u64,
    pub maximum_tile_bytes: usize,
    pub maximum_manifest_bytes: usize,
    pub maximum_band_workspace_bytes: u64,
    pub android_band_fits: bool,
}

pub(super) fn stage(source: &CapturedSource, disk_cache_bytes: u64) -> Result<StagedProducerPlan> {
    if !(1..=MAX_DISK_BYTES).contains(&disk_cache_bytes) {
        return Err(Error::Limit);
    }
    let pyramid = source.source.validate().map_err(|_| Error::Limit)?;
    if !matches!(source.format, "png" | "jpeg")
        || source.source.bit_depth != 8
        || source.source.icc.is_some()
        || source.source.display_conversion
    {
        return Err(Error::Unsupported);
    }
    let band = u64::from(source.source.width)
        .checked_mul(u64::from(source.source.height.min(TILE_SIZE)))
        .and_then(|bytes| bytes.checked_mul(8))
        .ok_or(Error::Limit)?;
    Ok(StagedProducerPlan {
        asset_id: super::tiles::hex(&source.source.original),
        original_bytes: source.source.original_bytes,
        width: source.source.width,
        height: source.source.height,
        orientation: source.source.orientation,
        format: source.format,
        tile_size: TILE_SIZE,
        tile_count: pyramid.tile_count(),
        disk_cache_bytes,
        maximum_tile_bytes: MAX_ENCODED_TILE_BYTES,
        maximum_manifest_bytes: MAX_MANIFEST_BYTES,
        maximum_band_workspace_bytes: BAND_WORKSPACE_BYTES,
        android_band_fits: band <= BAND_WORKSPACE_BYTES,
    })
}

/// Only a future verified platform runner inside this module may create a
/// completion witness. Until that runner and its input contract are admitted,
/// even a well-formed cache is Unsupported. Missing cache is handled earlier by
/// the bounded reader. A persisted self-authored "approved" marker is not proof.
pub(super) fn require_approved_producer(
    _manifest: &Manifest,
    cancellation: &Cancellation,
) -> Result<()> {
    cancellation.check().map_err(Error::from)?;
    Err(Error::Unsupported)
}
