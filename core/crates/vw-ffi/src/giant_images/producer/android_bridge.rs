//! Root-owned Android completion boundary. Only the actual native-invoked JVM
//! runner's nested call can construct CompletedOutput in the JNI child module.
//! There is no exported completed-directory, manifest or approval-boolean API.
use super::{Error, Result, cache, cleanup, input, io_error};
use crate::{
    Cancellation,
    giant_images::{source, tiles},
};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};
use vw_assets::{MAX_ENCODED_TILE_BYTES, MAX_MANIFEST_BYTES, Manifest, TileRecord};
use vw_stream::{MemoryKind, SessionBudget};

#[cfg(target_os = "android")]
mod jni;
#[cfg(target_os = "android")]
pub(super) use jni::build;

/// Private fields: only the trusted JNI runner's active nested callback and
/// this module's synthetic tests can construct this receipt. It borrows no files
/// after copy_output returns; the returned native directory is independent.
struct CompletedOutput {
    directory: PathBuf,
    source_sha256: String,
    encoded_bytes: u64,
    raw_width: u32,
    raw_height: u32,
    orientation: u8,
    width: u32,
    height: u32,
    preview_width: u32,
    preview_height: u32,
}
struct Context {
    source: Arc<input::PreparedSource>,
    closed: Arc<AtomicBool>,
    cancel: Arc<Cancellation>,
    budget: SessionBudget,
    disk_cache_bytes: u64,
    deadline: Instant,
}
impl Context {
    fn check(&self) -> Result<()> {
        source::live(&self.closed, &self.cancel)?;
        if Instant::now() >= self.deadline {
            return Err(Error::Limit);
        }
        Ok(())
    }
}
fn copy_output(output: CompletedOutput, context: &Context) -> Result<cache::ApprovedBuild> {
    context.check()?;
    let prepared = &context.source;
    let info = &prepared.info;
    if prepared.format != "jpeg"
        || output.source_sha256 != tiles::hex(&info.sha256)
        || output.encoded_bytes != info.encoded_bytes
        || output.raw_width != info.raw_width
        || output.raw_height != info.raw_height
        || output.orientation != info.orientation
        || output.width != info.width
        || output.height != info.height
        || output.preview_width == 0
        || output.preview_height == 0
        || output.preview_width.max(output.preview_height) > 2048
        || !(1..=super::MAX_DISK_BYTES).contains(&context.disk_cache_bytes)
    {
        return Err(Error::Stale);
    }
    prepared.verify(&context.closed, &context.cancel, &context.budget)?;
    input::admit_space(
        prepared.staging.path(),
        prepared.required_work_bytes,
        &vw_store::RealFreeSpace,
    )?;
    // Reserve before any retained manifest/inventory or independently owned
    // output paths. Output cleanup retains the input permit and exact charge.
    let manifest_charge = source::reserve(
        &context.budget,
        MemoryKind::Transfer,
        tiles::MANIFEST_WORKSPACE,
    )?;
    let directory_charge = source::reserve(&context.budget, MemoryKind::Transfer, 1024 * 1024)?;
    let root = checked_output_root(&output, prepared)?;
    let pyramid = prepared.source.validate().map_err(|_| Error::Limit)?;
    check_inventory(&root, &pyramid, context)?;
    let mut preview = source::regular(&root.join("preview.jpg")).map_err(Error::from)?;
    if preview.metadata().map_err(io_error)?.len() > 16 * 1024 * 1024 {
        return Err(Error::Limit);
    }
    let header = source::inspect(&mut preview, &context.cancel).map_err(Error::from)?;
    if header.format != "jpeg"
        || header.orientation != 1
        || header.dimensions() != (output.preview_width, output.preview_height)
    {
        return Err(Error::Invalid);
    }
    drop(preview);
    let temporary = tempfile::Builder::new()
        .prefix("vw-giant-android-native-")
        .tempdir_in(prepared.staging.path())
        .map_err(io_error)?;
    let owned = cleanup::OwnedDirectory::new(temporary, (prepared.clone(), directory_charge));
    let destination = owned.path().join("pyramid");
    std::fs::create_dir(&destination).map_err(io_error)?;
    for level in pyramid.levels() {
        std::fs::create_dir(destination.join(format!("L{}", level.index))).map_err(io_error)?;
    }
    let mut records = Vec::new();
    records
        .try_reserve_exact(pyramid.tile_count())
        .map_err(|_| Error::Limit)?;
    if records.capacity() > pyramid.tile_count() {
        return Err(Error::Limit);
    }
    let mut written = 0u64;
    for key in pyramid.keys() {
        context.check()?;
        let mut record = TileRecord {
            key,
            encoded_bytes: 0,
            hash: [0; 32],
        };
        let relative = record.relative_path();
        let encoded = cache::bytes(
            &root.join(&relative),
            MAX_ENCODED_TILE_BYTES as u64,
            &context.budget,
            &context.cancel,
        )?;
        written = written
            .checked_add(encoded.bytes().len() as u64)
            .ok_or(Error::Limit)?;
        if written
            .checked_add(MAX_MANIFEST_BYTES as u64)
            .is_none_or(|bytes| bytes > context.disk_cache_bytes)
        {
            return Err(Error::Limit);
        }
        {
            let _codec = source::reserve(
                &context.budget,
                MemoryKind::CodecWorkspace,
                tiles::DECODE_WORKSPACE,
            )?;
            drop(tiles::decode(
                encoded.bytes(),
                key,
                pyramid.region(key).map_err(|_| Error::Invalid)?,
                &context.cancel,
            )?);
        }
        record.encoded_bytes = u32::try_from(encoded.bytes().len()).map_err(|_| Error::Limit)?;
        record.hash = *blake3::hash(encoded.bytes()).as_bytes();
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination.join(&relative))
            .map_err(io_error)?;
        // Individually charged bounded bytes; never a whole-source/pyramid Vec.
        file.write_all(encoded.bytes())
            .and_then(|()| file.flush())
            .and_then(|()| file.sync_all())
            .map_err(io_error)?;
        records.push(record);
    }
    context.check()?;
    let manifest = Manifest::new(prepared.source.clone(), records).map_err(|_| Error::Invalid)?;
    // Bundle inventory, directory and charge before fallible final verification.
    let build = cache::ApprovedBuild {
        manifest,
        output: owned,
        source: prepared.clone(),
        platform: "android-bitmap-region-sdk30",
        disk_cache_bytes: context.disk_cache_bytes,
        _manifest_charge: manifest_charge,
    };
    cache::verify_output(
        &build,
        &context.closed,
        None,
        &context.cancel,
        &context.budget,
    )?;
    context.check()?;
    // JNI keeps this result private until the JVM returns after joined successful
    // AndroidTileBuild.close. Thus no publication can overlap its 80 MiB owner.
    Ok(build)
}
/// The native copy is not usable until the actual Java runner returns after
/// successful joined close. Error status drops an independently owned build,
/// never the still-held/quarantined Android owner or its foreign reservation.
fn after_android_retirement(
    context: &Context,
    consumed: bool,
    status: i32,
    result: Option<Result<cache::ApprovedBuild>>,
) -> Result<cache::ApprovedBuild> {
    context.check()?;
    if status != 0 {
        return match result {
            Some(Err(error)) => Err(error),
            _ => Err(match status {
                2 => Error::Cancelled,
                3 => Error::Backpressure,
                4 => Error::Unsupported,
                5 => Error::Invalid,
                6 => Error::Memory {
                    estimated: context.budget.used_bytes().saturating_add(80 * 1024 * 1024),
                    budget: context.budget.limit_bytes(),
                },
                7 => Error::Limit,
                8 => Error::Closed,
                _ => Error::Storage,
            }),
        };
    }
    if !consumed {
        return Err(Error::Unsupported);
    }
    result.ok_or(Error::Unsupported)?
}

fn checked_output_root(
    output: &CompletedOutput,
    prepared: &input::PreparedSource,
) -> Result<PathBuf> {
    let root = input::checked_directory(&output.directory)?;
    if root != output.directory
        || root.parent() != Some(prepared.staging.path().join("android-work").as_path())
    {
        return Err(Error::Invalid);
    }
    let parent = root.parent().ok_or(Error::Invalid)?;
    if input::checked_directory(parent)? != parent {
        return Err(Error::Invalid);
    }
    let name = root
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or(Error::Invalid)?;
    let marker = name.strip_prefix("vw-jpeg-").ok_or(Error::Invalid)?;
    if marker.len() != 36
        || !marker.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
    {
        return Err(Error::Invalid);
    }
    let mut file = source::regular(&root.join(".owner")).map_err(Error::from)?;
    let mut bytes = [0u8; 37];
    let count = file.read(&mut bytes).map_err(io_error)?;
    if count != 36
        || &bytes[..36] != marker.as_bytes()
        || file.read(&mut bytes[..1]).map_err(io_error)? != 0
    {
        return Err(Error::Invalid);
    }
    Ok(root)
}
fn check_inventory(
    root: &std::path::Path,
    pyramid: &vw_assets::Pyramid,
    context: &Context,
) -> Result<()> {
    let mut top = 0usize;
    for entry in std::fs::read_dir(root).map_err(io_error)? {
        context.check()?;
        let entry = entry.map_err(io_error)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| Error::Invalid)?;
        if matches!(name.as_str(), ".owner" | "preview.jpg") {
            let _file = source::regular(&entry.path()).map_err(Error::from)?;
        } else {
            let index = name
                .strip_prefix('L')
                .and_then(|v| v.parse::<u8>().ok())
                .ok_or(Error::Invalid)?;
            if name != format!("L{index}") {
                return Err(Error::Invalid);
            }
            let level = pyramid.level(index).map_err(|_| Error::Invalid)?;
            input::checked_directory(&entry.path())?;
            let expected = u64::from(level.columns) * u64::from(level.rows);
            let mut count = 0u64;
            for tile in std::fs::read_dir(entry.path()).map_err(io_error)? {
                context.check()?;
                let tile = tile.map_err(io_error)?;
                let _file = source::regular(&tile.path()).map_err(Error::from)?;
                count += 1;
                if count > expected {
                    return Err(Error::Invalid);
                }
            }
            if count != expected {
                return Err(Error::Missing);
            }
        }
        top += 1;
        if top > pyramid.levels().len() + 2 {
            return Err(Error::Invalid);
        }
    }
    if top != pyramid.levels().len() + 2 {
        return Err(Error::Missing);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
