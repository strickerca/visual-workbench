//! Authenticated, bounded derived-tile reads. No original byte fallback.
use super::source::{reserve, store_error};
use super::{GiantImageError as Error, GiantImageResult as Result};
use crate::Cancellation;
use vw_assets::{
    MAX_ENCODED_TILE_BYTES, MAX_MANIFEST_BYTES, Manifest, Source, TileKey, TileRegion,
};
use vw_store::DiskCache;
use vw_stream::{MemoryCharge, MemoryKind, SessionBudget};

// The successfully validated <=100k-entry map has bounded keys (128 bytes)
// and computed 70-byte filenames. Keep its resident envelope separately from
// transient JSON/index work: holding the entire 128 MiB peak forever would
// prevent any Android decode beside the required 96 MiB rendering surface.
pub(super) const STORE_RESIDENT: u64 = 64 * 1024 * 1024;
pub(super) const STORE_WORKSPACE: u64 = 64 * 1024 * 1024;
// Requires DiskCache::get_bounded_streaming's fixed-buffer persistence contract.
// Constructor and cache mutations retain the full STORE_WORKSPACE envelope.
pub(super) const STORE_READ_WORKSPACE: u64 = 1024 * 1024;
pub(super) const MANIFEST_WORKSPACE: u64 = 24 * 1024 * 1024;
pub(super) const DECODE_WORKSPACE: u64 = 32 * 1024 * 1024;
pub(super) const TILE_RGBA_BYTES: u64 = 256 * 256 * 4;

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(super) fn manifest_key(source: &Source) -> String {
    format!("giant-v1-{}-manifest", hex(&source.original))
}
pub(super) fn tile_key(source: &Source, key: TileKey) -> String {
    format!(
        "giant-v1-{}-{}-{}-{}",
        hex(&source.original),
        key.level,
        key.x,
        key.y
    )
}

pub(super) struct AdmittedManifest {
    pub manifest: Manifest,
    pub charge: MemoryCharge,
}
fn read_manifest<Q: vw_store::FreeSpaceQuery>(
    cache: &mut DiskCache<Q>,
    source: &Source,
    cancel: &Cancellation,
    budget: &SessionBudget,
) -> Result<AdmittedManifest> {
    cancel.check().map_err(Error::from)?;
    let charge = reserve(budget, MemoryKind::Transfer, MANIFEST_WORKSPACE)?;
    let raw = {
        let _index = reserve(budget, MemoryKind::CodecWorkspace, STORE_READ_WORKSPACE)?;
        cache
            .get_bounded_streaming(&manifest_key(source), MAX_MANIFEST_BYTES as u64)
            .map_err(store_error)?
            .ok_or(Error::Missing)?
    };
    cancel.check().map_err(Error::from)?;
    let manifest = Manifest::decode(&raw, source).map_err(|_| Error::Invalid)?;
    drop(raw);
    Ok(AdmittedManifest { manifest, charge })
}
/// Root-scoped process-private admission. Persisted caches cannot authenticate
/// their producer after process restart; rebuilding mints a fresh witness.
pub(super) fn manifest_scoped<Q: vw_store::FreeSpaceQuery>(
    cache: &mut DiskCache<Q>,
    root: &std::path::Path,
    source: &Source,
    cancel: &Cancellation,
    budget: &SessionBudget,
) -> Result<AdmittedManifest> {
    let admitted = read_manifest(cache, source, cancel, budget)?;
    super::producer::approved_manifest(root, &admitted.manifest, budget, cancel)?;
    Ok(admitted)
}
#[cfg(test)]
pub(super) fn manifest(
    cache: &mut DiskCache,
    source: &Source,
    cancel: &Cancellation,
    budget: &SessionBudget,
) -> Result<(Manifest, MemoryCharge)> {
    let admitted = read_manifest(cache, source, cancel, budget)?;
    super::producer::require_approved_producer(&admitted.manifest, cancel)?;
    Ok((admitted.manifest, admitted.charge))
}

/// The actual cache payload holds its shared-session charge through every Arc
/// lease. Destroying a cache manager never credits an externally retained lease.
pub(super) struct CachedTile {
    pub rgba: Vec<u8>,
    _charge: MemoryCharge,
}
impl CachedTile {
    pub fn decode(
        cache: &mut DiskCache,
        manifest: &Manifest,
        key: TileKey,
        cancel: &Cancellation,
        budget: &SessionBudget,
    ) -> Result<Self> {
        cancel.check().map_err(Error::from)?;
        let record = manifest.tile(key).map_err(|_| Error::Invalid)?;
        let region = manifest.pyramid().region(key).map_err(|_| Error::Invalid)?;
        let len = u64::from(record.encoded_bytes);
        if len == 0 || len > MAX_ENCODED_TILE_BYTES as u64 {
            return Err(Error::Limit);
        }
        // DiskCache reserves one sentinel byte in addition to the declared size.
        let encoded_charge = reserve(budget, MemoryKind::EncodedTiles, len + 1)?;
        let encoded = {
            let _index = reserve(budget, MemoryKind::CodecWorkspace, STORE_READ_WORKSPACE)?;
            cache
                .get_bounded_streaming(&tile_key(manifest.source(), key), len)
                .map_err(store_error)?
                .ok_or(Error::Missing)?
        };
        if encoded.len() as u64 != len || encoded.capacity() as u64 > len + 1 {
            return Err(Error::Limit);
        }
        record.verify(&encoded).map_err(|_| Error::Invalid)?;
        let workspace = reserve(budget, MemoryKind::CodecWorkspace, DECODE_WORKSPACE)?;
        let retained = reserve(budget, MemoryKind::DisplayFrame, TILE_RGBA_BYTES)?;
        let rgba = decode(&encoded, key, region, cancel)?;
        if rgba.capacity() as u64 > TILE_RGBA_BYTES {
            return Err(Error::Limit);
        }
        drop(workspace);
        drop(encoded);
        drop(encoded_charge);
        cancel.check().map_err(Error::from)?;
        Ok(Self {
            rgba,
            _charge: retained,
        })
    }
}
pub(super) fn decode(
    bytes: &[u8],
    key: TileKey,
    region: TileRegion,
    cancel: &Cancellation,
) -> Result<Vec<u8>> {
    cancel.check().map_err(Error::from)?;
    if bytes.is_empty() || bytes.len() > MAX_ENCODED_TILE_BYTES {
        return Err(Error::Limit);
    }
    if key.level == 0 {
        // One exact lossless chunk: no auxiliary ICC/EXIF/animation/alpha tag
        // can smuggle a conversion before the normal decoder is invoked.
        if bytes.len() < 21
            || bytes.get(..4) != Some(b"RIFF")
            || bytes.get(8..16) != Some(b"WEBPVP8L")
        {
            return Err(Error::Unsupported);
        }
        let total =
            u32::from_le_bytes(bytes[4..8].try_into().map_err(|_| Error::Invalid)?) as usize;
        let chunk =
            u32::from_le_bytes(bytes[16..20].try_into().map_err(|_| Error::Invalid)?) as usize;
        if total.checked_add(8) != Some(bytes.len())
            || chunk.checked_add(20).and_then(|n| n.checked_add(chunk % 2)) != Some(bytes.len())
        {
            return Err(Error::Invalid);
        }
    } else {
        vw_stream::validate_jpeg_tile_header(bytes, region.width, region.height)
            .map_err(|_| Error::Unsupported)?;
    }
    let decoded = vw_raster::decode(
        bytes,
        vw_raster::DecodeLimits {
            max_encoded_bytes: MAX_ENCODED_TILE_BYTES,
            max_pixels: 256 * 256,
            max_memory_bytes: DECODE_WORKSPACE,
        },
    )
    .map_err(|_| Error::Unsupported)?;
    cancel.check().map_err(Error::from)?;
    if decoded.width != region.width
        || decoded.height != region.height
        || decoded.orientation_applied != 1
        || decoded.icc.is_some()
    {
        return Err(Error::Unsupported);
    }
    match decoded.pixels {
        vw_raster::Pixels::Rgba8(rgba) if rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255) => {
            Ok(rgba)
        }
        _ => Err(Error::Unsupported),
    }
}
