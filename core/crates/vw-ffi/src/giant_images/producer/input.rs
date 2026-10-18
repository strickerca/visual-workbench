//! Recovered streaming preparation, with explicit consent and shared admission.
use super::{Error, Result, io_error};
use crate::Cancellation;
use crate::giant_images::source;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use vw_assets::Source;
use vw_store::{FreeSpaceQuery, disk_reserve_bytes};
use vw_stream::{MemoryKind, SessionBudget};

const OWNER_BYTES: u64 = 4 * 1024 * 1024;
static INPUTS: AtomicUsize = AtomicUsize::new(0);
struct InputPermit;
impl InputPermit {
    fn acquire() -> Result<Self> {
        INPUTS
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                (used < 2).then_some(used + 1)
            })
            .map_err(|_| Error::Backpressure)?;
        Ok(Self)
    }
}
impl Drop for InputPermit {
    fn drop(&mut self) {
        INPUTS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Both values are already owned by the root picker/blob adapter before async dispatch.
pub(crate) struct RootOwnedPickedOriginal {
    pub file: File,
    pub work: tempfile::TempDir,
}
pub(crate) struct GiantInputLimits {
    pub max_original_bytes: u64,
    pub max_work_disk_bytes: u64,
    pub assume_untagged_srgb: bool,
}
#[derive(Clone)]
pub(crate) struct GiantSourceInfo {
    pub source_path: PathBuf,
    pub asset_id: [u8; 32],
    pub sha256: [u8; 32],
    pub encoded_bytes: u64,
    pub raw_width: u32,
    pub raw_height: u32,
    pub width: u32,
    pub height: u32,
    pub orientation: u8,
    pub format: String,
}

pub(super) struct PreparedSource {
    // Drop the actual file before its exact private staging directory.
    _input: File,
    pub info: GiantSourceInfo,
    pub source: Source,
    pub format: &'static str,
    pub max_work_bytes: u64,
    pub required_work_bytes: u64,
    pub staging: super::cleanup::OwnedDirectory,
}
impl PreparedSource {
    pub fn verify(
        &self,
        closed: &AtomicBool,
        cancel: &Cancellation,
        budget: &SessionBudget,
    ) -> Result<()> {
        self.captured().verify(closed, cancel, budget)
    }
    pub fn captured(&self) -> source::CapturedSource {
        source::CapturedSource {
            root: self.staging.path().to_path_buf(),
            path: self.info.source_path.clone(),
            source: self.source.clone(),
            format: self.format,
        }
    }
}

pub(super) fn checked_directory(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() || path.as_os_str().as_encoded_bytes().len() > 8192 {
        return Err(Error::Invalid);
    }
    let metadata = std::fs::symlink_metadata(path).map_err(io_error)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::Invalid);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(Error::Invalid);
        }
    }
    path.canonicalize().map_err(io_error)
}
pub(super) fn admit_space(path: &Path, required: u64, query: &impl FreeSpaceQuery) -> Result<()> {
    let space = query.query(path).map_err(source::store_error)?;
    if space
        .available_bytes
        .saturating_sub(disk_reserve_bytes(space.total_bytes))
        < required
    {
        return Err(Error::Limit);
    }
    Ok(())
}
pub(super) fn work_bytes(source: &Source) -> Result<u64> {
    // Private input + worker/source/blob publication envelopes, full-resolution
    // random-access scratch and both producer/cache pyramids. Conservative
    // storage admission from the recovered phone intake, not measured usage.
    source
        .original_bytes
        .checked_mul(4)
        .and_then(|n| {
            u64::from(source.width)
                .checked_mul(u64::from(source.height))
                .and_then(|pixels| pixels.checked_mul(24))
                .and_then(|pixels| n.checked_add(pixels))
        })
        .and_then(|n| n.checked_add(128 * 1024 * 1024))
        .ok_or(Error::Limit)
}
pub(super) fn prepare(
    original: RootOwnedPickedOriginal,
    limits: GiantInputLimits,
    closed: &AtomicBool,
    cancel: &Cancellation,
    budget: &SessionBudget,
    query: &impl FreeSpaceQuery,
) -> Result<Arc<PreparedSource>> {
    source::live(closed, cancel)?;
    if !limits.assume_untagged_srgb {
        return Err(Error::Unsupported);
    }
    if !(1..=source::MAX_ORIGINAL).contains(&limits.max_original_bytes)
        || !(1..=super::MAX_DISK_BYTES).contains(&limits.max_work_disk_bytes)
    {
        return Err(Error::Limit);
    }
    let owner_charge = source::reserve(budget, MemoryKind::Transfer, OWNER_BYTES)?;
    let permit = InputPermit::acquire()?;
    let RootOwnedPickedOriginal {
        file: mut original,
        work: temporary,
    } = original;
    if !original.metadata().map_err(io_error)?.is_file() {
        return Err(Error::Invalid);
    }
    let original_size = original.metadata().map_err(io_error)?.len();
    if !(33..=limits.max_original_bytes).contains(&original_size)
        || original_size > limits.max_work_disk_bytes
    {
        return Err(Error::Limit);
    }
    let work = checked_directory(temporary.path())?;
    admit_space(&work, original_size, query)?;
    let staging = super::cleanup::OwnedDirectory::new(temporary, (permit, owner_charge));
    let path = staging.path().join("original.input");
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(io_error)?;
    let (hash, sha256, bytes) = source::hash_copy_checked(
        &mut original,
        Some(&mut output),
        limits.max_original_bytes,
        || {
            if closed.load(Ordering::Acquire) {
                return Err(crate::CoreError::Closed);
            }
            cancel.check()
        },
    )
    .map_err(Error::from)?;
    if bytes != original_size {
        return Err(Error::Stale);
    }
    output
        .flush()
        .and_then(|()| output.sync_all())
        .map_err(io_error)?;
    drop(output);
    let mut input = source::regular(&path).map_err(Error::from)?;
    let metadata = source::inspect(&mut input, cancel).map_err(Error::from)?;
    let (width, height) = metadata.dimensions();
    let source = Source {
        original: hash,
        original_bytes: bytes,
        width,
        height,
        orientation: metadata.orientation,
        bit_depth: 8,
        icc: None,
        display_conversion: false,
    };
    source.validate().map_err(|_| Error::Limit)?;
    let required_work_bytes = work_bytes(&source)?;
    if required_work_bytes > limits.max_work_disk_bytes {
        return Err(Error::Limit);
    }
    admit_space(&work, required_work_bytes, query)?;
    source::live(closed, cancel)?;
    let info = GiantSourceInfo {
        source_path: path,
        asset_id: hash,
        sha256: parse_hash(&sha256)?,
        encoded_bytes: bytes,
        raw_width: metadata.width,
        raw_height: metadata.height,
        width,
        height,
        orientation: metadata.orientation,
        format: metadata.format.into(),
    };
    Ok(Arc::new(PreparedSource {
        _input: input,
        info,
        source,
        format: metadata.format,
        max_work_bytes: limits.max_work_disk_bytes,
        required_work_bytes,
        staging,
    }))
}

/// Retire the exact source only after every typed producer output has released
/// it. Explicit close surfaces deletion failure; implicit drops use the same
/// quarantining directory owner.
pub(super) fn release(source: Arc<PreparedSource>) -> Result<()> {
    let Ok(source) = Arc::try_unwrap(source) else {
        return Ok(());
    };
    let PreparedSource {
        _input,
        info,
        source: _,
        format: _,
        max_work_bytes: _,
        required_work_bytes: _,
        mut staging,
    } = source;
    drop(_input);
    drop(info);
    staging.cleanup()
}

fn parse_hash(text: &str) -> Result<[u8; 32]> {
    if text.len() != 64 {
        return Err(Error::Invalid);
    }
    let mut hash = [0; 32];
    for (index, pair) in text.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let hex = std::str::from_utf8(pair).map_err(|_| Error::Invalid)?;
        hash[index] = u8::from_str_radix(hex, 16).map_err(|_| Error::Invalid)?;
    }
    Ok(hash)
}
