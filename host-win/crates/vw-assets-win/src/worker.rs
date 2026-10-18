use crate::budget::{
    BudgetedManifest, DECODE_WORK_BYTES, MANIFEST_OWNER_BYTES, PARENT_WORK_BYTES, reserve,
};
use crate::{
    BuildRequest, Cancellation, Error, LockedHelper, LockedRuntime, Result,
    admission::admit_input,
    native::Api,
    ownership::{pin_ancestors, plain, read_locked},
    process, wide,
};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use vw_assets::{MAX_ENCODED_TILE_BYTES, MAX_MANIFEST_BYTES, Manifest, TileRecord};
use vw_stream::{MemoryCharge, MemoryKind, SessionBudget};
static ACTIVE: AtomicBool = AtomicBool::new(false);
struct Permit;
impl Drop for Permit {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}

/// Synchronous worker API: invoke on the app's retained bounded worker, never UI.
/// Original is copied/hash-verified into private sibling staging. Destination
/// publication is same-volume/no-clobber only after exact complete verification.
/// No project metadata is mutated. Parent app must atomically bind the returned
/// manifest to its immutable original before exposing the tiled document.
/// The root caller must first perform its full source metadata/profile admission
/// and explicit untagged-sRGB consent; admit_input is only a codec-entry guard.
/// Keep the caller's retained worker alive until this synchronous call settles,
/// including when the requesting async future is abandoned.
pub fn build_budgeted(
    helper: &LockedHelper,
    runtime: &LockedRuntime,
    input: &Path,
    destination: &Path,
    request: &BuildRequest,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<BudgetedManifest> {
    cancel.check()?;
    // Reserve before source validation's pyramid allocation, hashing buffers,
    // request serialization, path construction or process-control allocations.
    let _work = reserve(budget, MemoryKind::CodecWorkspace, PARENT_WORK_BYTES)?;
    request.validate()?;
    ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| Error::Busy)?;
    let _permit = Permit;
    if destination.file_name().is_none() || fs::symlink_metadata(destination).is_ok() {
        return Err(Error::Storage);
    }
    let parent = destination.parent().ok_or(Error::Invalid)?;
    let _parent_pins = pin_ancestors(parent)?;
    let _input_pins = pin_ancestors(input.parent().ok_or(Error::Invalid)?)?;
    let mut original = read_locked(input)?;
    if original.metadata().map_err(|_| Error::Storage)?.len() != request.original_bytes {
        return Err(Error::Binding);
    }
    admit_input(&mut original, request)?;
    let staging = tempfile::Builder::new()
        .prefix(".vw-pyramid-")
        .tempdir_in(parent)
        .map_err(|_| Error::Storage)?;
    let stage = staging.path();
    let mut copy = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stage.join("original.input"))
        .map_err(|_| Error::Storage)?;
    let mut hash = blake3::Hasher::new();
    let mut count = 0u64;
    let mut buffer = [0; 65536];
    loop {
        cancel.check()?;
        let n = original.read(&mut buffer).map_err(|_| Error::Storage)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > request.original_bytes {
            return Err(Error::Binding);
        }
        hash.update(&buffer[..n]);
        copy.write_all(&buffer[..n]).map_err(|_| Error::Storage)?;
    }
    if count != request.original_bytes || hash.finalize().as_bytes() != &request.original_hash {
        return Err(Error::Binding);
    }
    copy.sync_all().map_err(|_| Error::Storage)?;
    drop(copy);
    let json = serde_json::to_vec(request).map_err(|_| Error::Invalid)?;
    if json.len() > 4096 {
        return Err(Error::Limit);
    }
    let mut receipt = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stage.join("request.json"))
        .map_err(|_| Error::Storage)?;
    receipt
        .write_all(&json)
        .and_then(|()| receipt.sync_all())
        .map_err(|_| Error::Storage)?;
    drop(receipt);
    let args = [
        runtime.directory.to_str().ok_or(Error::Invalid)?.to_owned(),
        stage.to_str().ok_or(Error::Invalid)?.to_owned(),
    ];
    let outcome = process::run(
        &helper.path,
        &args,
        cancel,
        Duration::from_millis(request.timeout_ms),
        budget,
    )?;
    if outcome.code != 0 {
        return Err(Error::Codec);
    }
    cancel.check()?;
    let result = stage.join("result");
    plain(&result, true)?;
    let manifest = read_manifest_budgeted(&result.join("pyramid.vwt"), request, budget, cancel)?;
    verify_output(&result, manifest.manifest(), budget, cancel)?;
    cancel.check()?;
    let from = wide(result.as_os_str())?;
    let to = wide(destination.as_os_str())?;
    // SAFETY: exact owned sibling staging directory, pinned destination parent,
    // and zero MoveFileEx flags means existing destinations are never replaced.
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(from.as_ptr(), to.as_ptr(), 0)
    } == 0
    {
        return Err(Error::Storage);
    }
    // Destination now owns only complete disposable derived files. The original
    // remains untouched; TempDir cleans this operation's request/source/scratch.
    Ok(manifest)
}

/// Called only by our dedicated binary under the parent-owned bounded Job.
/// # Safety
/// Caller must be the isolated single-threaded helper process before codec init;
/// this function sets its process-local scratch environment before threads exist.
pub unsafe fn helper(runtime: &Path, stage: &Path) -> Result<()> {
    use windows_sys::Win32::System::{JobObjects::IsProcessInJob, Threading::GetCurrentProcess};
    let mut in_job = 0;
    // SAFETY: valid current-process pseudo handle and writable BOOL.
    if unsafe { IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut in_job) } == 0
        || in_job == 0
    {
        return Err(Error::WorkerUnavailable);
    }
    let _stage_pins = pin_ancestors(stage)?;
    let runtime = LockedRuntime::open(runtime)?;
    let input = read_locked(&stage.join("request.json"))?;
    let len = input.metadata().map_err(|_| Error::Storage)?.len();
    if len > 4096 {
        return Err(Error::Limit);
    }
    let mut bytes = Vec::with_capacity(len as usize);
    input
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Storage)?;
    if bytes.len() > 4096 {
        return Err(Error::Limit);
    }
    let request: BuildRequest = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
    request.validate()?;
    let input_path = stage.join("original.input");
    let mut original = read_locked(&input_path)?;
    admit_input(&mut original, &request)?;
    let mut hasher = blake3::Hasher::new();
    let mut total = 0u64;
    let mut chunk = [0; 65536];
    loop {
        let n = original.read(&mut chunk).map_err(|_| Error::Storage)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > request.original_bytes {
            return Err(Error::Binding);
        }
        hasher.update(&chunk[..n]);
    }
    if total != request.original_bytes || hasher.finalize().as_bytes() != &request.original_hash {
        return Err(Error::Binding);
    }
    // SAFETY: this isolated process is still single-threaded before libvips init.
    // It inherited an empty environment. Scratch belongs exclusively to staging.
    unsafe {
        std::env::set_var("TMP", stage);
        std::env::set_var("TEMP", stage);
        std::env::set_var("VIPS_DISC_THRESHOLD", "64m");
    }
    let api = Api::new(&runtime)?;
    api.pyramid(&input_path, &stage.join("pyramid"), &request)?;
    let result = stage.join("result");
    fs::create_dir(&result).map_err(|_| Error::Storage)?;
    let pyramid = request.source().validate().map_err(|_| Error::Limit)?;
    let top = pyramid.levels().len() - 1;
    let mut records = Vec::with_capacity(pyramid.tile_count());
    for level in pyramid.levels() {
        fs::create_dir(result.join(format!("L{}", level.index))).map_err(|_| Error::Storage)?;
    }
    for key in pyramid.keys() {
        let input = stage
            .join("pyramid_files")
            .join((top - key.level as usize).to_string())
            .join(format!("{}_{}.webp", key.x, key.y));
        plain(&input, false)?;
        let path = result.join(format!("L{}", key.level)).join(format!(
            "{}_{}.{}",
            key.x,
            key.y,
            if key.level == 0 { "webp" } else { "jpg" }
        ));
        if key.level == 0 {
            fs::rename(&input, &path).map_err(|_| Error::Storage)?;
        } else {
            api.jpeg(&input, &path)?;
            fs::remove_file(&input).map_err(|_| Error::Storage)?;
        }
        let bytes = tile_bytes(&path)?;
        validate_tile(
            &bytes,
            key.level,
            pyramid.region(key).map_err(|_| Error::Invalid)?,
        )?;
        let mut file = OpenOptions::new()
            .write(true)
            .open(&path)
            .map_err(|_| Error::Storage)?;
        file.flush()
            .and_then(|()| file.sync_all())
            .map_err(|_| Error::Storage)?;
        records.push(TileRecord {
            key,
            encoded_bytes: bytes.len() as u32,
            hash: *blake3::hash(&bytes).as_bytes(),
        });
    }
    drop(api);
    let manifest = Manifest::new(request.source(), records).map_err(|_| Error::Invalid)?;
    if manifest.encoded_bytes() > request.disk_budget {
        return Err(Error::Limit);
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(result.join("pyramid.vwt"))
        .map_err(|_| Error::Storage)?;
    file.write_all(&manifest.encode())
        .and_then(|()| file.sync_all())
        .map_err(|_| Error::Storage)?;
    Ok(())
}
fn read_manifest_budgeted(
    path: &Path,
    r: &BuildRequest,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<BudgetedManifest> {
    cancel.check()?;
    // This owner covers both the raw bounded input and the decoded collection;
    // move it into the returned owner only after the raw bytes have retired.
    let charge = reserve(budget, MemoryKind::Transfer, MANIFEST_OWNER_BYTES)?;
    let bytes = read_bytes(path, MAX_MANIFEST_BYTES, None)?;
    cancel.check()?;
    let manifest = Manifest::decode(&bytes, &r.source()).map_err(|_| Error::Binding)?;
    drop(bytes);
    cancel.check()?;
    Ok(BudgetedManifest { manifest, charge })
}
// Child-only use: all allocations live under the parent's whole-worker charge.
fn tile_bytes(path: &Path) -> Result<Vec<u8>> {
    read_bytes(path, MAX_ENCODED_TILE_BYTES, None)
}
fn read_bytes(path: &Path, max: usize, expected: Option<usize>) -> Result<Vec<u8>> {
    let mut file = read_locked(path)?;
    let size = file.metadata().map_err(|_| Error::Storage)?.len();
    if size == 0 || size > max as u64 {
        return Err(Error::Limit);
    }
    let size = size as usize;
    if expected.is_some_and(|expected| size != expected) {
        return Err(Error::Binding);
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size).map_err(|_| Error::Limit)?;
    if bytes.capacity() > size {
        return Err(Error::Limit);
    }
    bytes.resize(size, 0);
    file.read_exact(&mut bytes).map_err(|_| Error::Binding)?;
    // A stack sentinel refuses changed length without allowing Vec growth.
    let mut sentinel = [0];
    if file.read(&mut sentinel).map_err(|_| Error::Storage)? != 0 {
        return Err(Error::Binding);
    }
    Ok(bytes)
}
struct EncodedTile {
    bytes: Vec<u8>,
    _charge: MemoryCharge,
}
fn tile_bytes_budgeted(
    path: &Path,
    expected: usize,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<EncodedTile> {
    cancel.check()?;
    if expected == 0 || expected > MAX_ENCODED_TILE_BYTES {
        return Err(Error::Limit);
    }
    let charge = reserve(budget, MemoryKind::EncodedTiles, expected as u64)?;
    let bytes = read_bytes(path, MAX_ENCODED_TILE_BYTES, Some(expected))?;
    cancel.check()?;
    Ok(EncodedTile {
        bytes,
        _charge: charge,
    })
}
fn validate_tile(bytes: &[u8], level: u8, region: vw_assets::TileRegion) -> Result<()> {
    if level == 0 {
        // The worker strips ancillary metadata, yielding one VP8L chunk. Refuse
        // lossy VP8 even when renamed .webp and perfectly hash-bound.
        if bytes.len() < 25
            || &bytes[..4] != b"RIFF"
            || &bytes[8..16] != b"WEBPVP8L"
            || bytes[20] != 0x2f
        {
            return Err(Error::Binding);
        }
        let total =
            u32::from_le_bytes(bytes[4..8].try_into().map_err(|_| Error::Binding)?) as usize;
        let chunk =
            u32::from_le_bytes(bytes[16..20].try_into().map_err(|_| Error::Binding)?) as usize;
        if total.checked_add(8) != Some(bytes.len())
            || chunk.checked_add(20).and_then(|n| n.checked_add(chunk % 2)) != Some(bytes.len())
        {
            return Err(Error::Binding);
        }
        let header = u32::from_le_bytes(bytes[21..25].try_into().map_err(|_| Error::Binding)?);
        if (header & 0x3fff) + 1 != region.width
            || ((header >> 14) & 0x3fff) + 1 != region.height
            || header >> 29 != 0
        {
            return Err(Error::Binding);
        }
    } else {
        vw_stream::validate_jpeg_tile_header(bytes, region.width, region.height)
            .map_err(|_| Error::Binding)?;
    }
    let decoded = vw_raster::decode(
        bytes,
        vw_raster::DecodeLimits {
            max_encoded_bytes: MAX_ENCODED_TILE_BYTES,
            max_pixels: 256 * 256,
            max_memory_bytes: DECODE_WORK_BYTES,
        },
    )
    .map_err(|_| Error::Codec)?;
    if decoded.width != region.width
        || decoded.height != region.height
        || decoded.pixels.bit_depth() != 8
        || decoded.icc.is_some()
        || decoded.orientation_applied != 1
    {
        return Err(Error::Binding);
    }
    if let vw_raster::Pixels::Rgba8(p) = &decoded.pixels {
        if p.chunks_exact(4).any(|rgba| rgba[3] != 255) {
            return Err(Error::Unsupported);
        }
    }
    Ok(())
}
fn validate_tile_budgeted(
    bytes: &[u8],
    level: u8,
    region: vw_assets::TileRegion,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<()> {
    cancel.check()?;
    let _decode = reserve(budget, MemoryKind::CodecWorkspace, DECODE_WORK_BYTES)?;
    validate_tile(bytes, level, region)?;
    cancel.check()
}
fn verify_output(
    root: &Path,
    manifest: &Manifest,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<()> {
    let mut entries = 0usize;
    for entry in fs::read_dir(root).map_err(|_| Error::Storage)? {
        cancel.check()?;
        let e = entry.map_err(|_| Error::Storage)?;
        let n = e.file_name().into_string().map_err(|_| Error::Binding)?;
        if n == "pyramid.vwt" {
            plain(&e.path(), false)?;
        } else {
            let level = n
                .strip_prefix('L')
                .and_then(|n| n.parse::<u8>().ok())
                .ok_or(Error::Binding)?;
            if n != format!("L{level}") {
                return Err(Error::Binding);
            }
            manifest
                .pyramid()
                .level(level)
                .map_err(|_| Error::Binding)?;
            plain(&e.path(), true)?;
        }
        entries += 1;
        if entries > manifest.pyramid().levels().len() + 1 {
            return Err(Error::Binding);
        }
    }
    if entries != manifest.pyramid().levels().len() + 1 {
        return Err(Error::Binding);
    }
    for level in manifest.pyramid().levels() {
        cancel.check()?;
        let dir = root.join(format!("L{}", level.index));
        let expected = level.columns as usize * level.rows as usize;
        let mut count = 0;
        for entry in fs::read_dir(&dir).map_err(|_| Error::Storage)? {
            cancel.check()?;
            plain(&entry.map_err(|_| Error::Storage)?.path(), false)?;
            count += 1;
            if count > expected {
                return Err(Error::Binding);
            }
        }
        if count != expected {
            return Err(Error::Binding);
        }
    }
    for tile in manifest.tiles() {
        cancel.check()?;
        let encoded = tile_bytes_budgeted(
            &root.join(tile.relative_path()),
            tile.encoded_bytes as usize,
            budget,
            cancel,
        )?;
        tile.verify(&encoded.bytes).map_err(|_| Error::Binding)?;
        validate_tile_budgeted(
            &encoded.bytes,
            tile.key.level,
            manifest
                .pyramid()
                .region(tile.key)
                .map_err(|_| Error::Binding)?,
            budget,
            cancel,
        )?;
        cancel.check()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> BuildRequest {
        BuildRequest {
            original_hash: [1; 32],
            original_bytes: 1,
            width: 1,
            height: 1,
            orientation: 1,
            disk_budget: 128 * 1024 * 1024,
            timeout_ms: 1000,
        }
    }

    #[test]
    fn manifest_admission_precedes_file_access() -> Result<()> {
        let budget = SessionBudget::new(MANIFEST_OWNER_BYTES - 1).map_err(|_| Error::Invalid)?;
        assert_eq!(
            read_manifest_budgeted(
                Path::new("never-read.vwt"),
                &request(),
                &budget,
                &Cancellation::default()
            )
            .err(),
            Some(Error::Memory {
                estimated: MANIFEST_OWNER_BYTES,
                budget: MANIFEST_OWNER_BYTES - 1
            })
        );
        assert_eq!(budget.used_bytes(), 0);
        Ok(())
    }

    #[test]
    fn manifest_failure_retires_raw_bytes_and_session_reservation() -> Result<()> {
        let work = tempfile::tempdir().map_err(|_| Error::Storage)?;
        let path = work.path().join("manifest");
        fs::write(&path, b"malformed").map_err(|_| Error::Storage)?;
        let budget = SessionBudget::new(MANIFEST_OWNER_BYTES).map_err(|_| Error::Invalid)?;
        assert_eq!(
            read_manifest_budgeted(&path, &request(), &budget, &Cancellation::default()).err(),
            Some(Error::Binding)
        );
        assert_eq!(budget.used_bytes(), 0);
        Ok(())
    }

    #[test]
    fn successful_manifest_read_returns_a_retained_same_pool_owner() -> Result<()> {
        let request = request();
        let source = request.source();
        let pyramid = source.validate().map_err(|_| Error::Invalid)?;
        let records = pyramid
            .keys()
            .map(|key| TileRecord {
                key,
                encoded_bytes: 1,
                hash: [2; 32],
            })
            .collect();
        let manifest = Manifest::new(source, records).map_err(|_| Error::Invalid)?;
        let work = tempfile::tempdir().map_err(|_| Error::Storage)?;
        let path = work.path().join("manifest");
        fs::write(&path, manifest.encode()).map_err(|_| Error::Storage)?;
        let budget = SessionBudget::new(MANIFEST_OWNER_BYTES).map_err(|_| Error::Invalid)?;
        let owner = read_manifest_budgeted(&path, &request, &budget, &Cancellation::default())?;
        assert_eq!(owner.manifest(), &manifest);
        assert!(owner.charge().belongs_to(&budget));
        assert_eq!(budget.used_bytes(), MANIFEST_OWNER_BYTES);
        drop(owner);
        assert_eq!(budget.used_bytes(), 0);
        Ok(())
    }

    #[test]
    fn encoded_read_is_exact_and_remains_charged_until_retirement() -> Result<()> {
        let work = tempfile::tempdir().map_err(|_| Error::Storage)?;
        let path = work.path().join("tile");
        fs::write(&path, b"abc").map_err(|_| Error::Storage)?;
        let budget = SessionBudget::new(3).map_err(|_| Error::Invalid)?;
        let owner = tile_bytes_budgeted(&path, 3, &budget, &Cancellation::default())?;
        assert_eq!(owner.bytes, b"abc");
        assert_eq!(budget.used_bytes(), 3);
        drop(owner);
        assert_eq!(budget.used_bytes(), 0);
        assert_eq!(
            tile_bytes_budgeted(&path, 2, &budget, &Cancellation::default()).err(),
            Some(Error::Binding)
        );
        assert_eq!(budget.used_bytes(), 0);
        assert_eq!(
            tile_bytes_budgeted(
                Path::new("never-read.webp"),
                4,
                &budget,
                &Cancellation::default()
            )
            .err(),
            Some(Error::Memory {
                estimated: 4,
                budget: 3
            })
        );
        assert_eq!(budget.used_bytes(), 0);
        Ok(())
    }

    #[test]
    fn decode_workspace_is_admitted_before_codec_entry() -> Result<()> {
        let budget = SessionBudget::new(DECODE_WORK_BYTES - 1).map_err(|_| Error::Invalid)?;
        let region = vw_assets::TileRegion {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        assert_eq!(
            validate_tile_budgeted(&[], 0, region, &budget, &Cancellation::default()),
            Err(Error::Memory {
                estimated: DECODE_WORK_BYTES,
                budget: DECODE_WORK_BYTES - 1
            })
        );
        assert_eq!(budget.used_bytes(), 0);
        Ok(())
    }

    #[test]
    fn base_tile_preflight_refuses_auxiliary_chunks_and_changed_geometry() {
        let region = vw_assets::TileRegion {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        let mut header = b"RIFF".to_vec();
        header.extend_from_slice(&18u32.to_le_bytes());
        header.extend_from_slice(b"WEBPVP8L");
        header.extend_from_slice(&5u32.to_le_bytes());
        header.extend_from_slice(&[0x2f, 1, 0, 0, 0, 0]);
        assert_eq!(validate_tile(&header, 0, region), Err(Error::Binding));
        header.extend_from_slice(b"ICCP");
        assert_eq!(validate_tile(&header, 0, region), Err(Error::Binding));
    }
}
