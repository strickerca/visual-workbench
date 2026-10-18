//! A capability minted by an actual approved runner, never by a result path.
use super::input::PreparedSource;
use super::{Error, Result, io_error};
use crate::Cancellation;
use crate::giant_images::{source, tiles};
use std::{
    collections::VecDeque,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, atomic::AtomicBool},
};
use vw_assets::{MAX_ENCODED_TILE_BYTES, MAX_MANIFEST_BYTES, Manifest};
use vw_store::{DiskCache, FreeSpaceQuery};
use vw_stream::{MemoryCharge, MemoryKind, SessionBudget};

pub(super) struct ApprovedBuild {
    pub manifest: Manifest,
    pub output: super::cleanup::OwnedDirectory,
    pub source: Arc<PreparedSource>,
    pub platform: &'static str,
    pub disk_cache_bytes: u64,
    pub _manifest_charge: MemoryCharge,
}
impl ApprovedBuild {
    pub fn directory(&self) -> PathBuf {
        self.output.path().join("pyramid")
    }
}

struct Approval {
    root: PathBuf,
    source: [u8; 32],
    manifest: [u8; 32],
    _charge: MemoryCharge,
}
const MAX_APPROVALS: usize = 64;
const APPROVAL_BYTES: u64 = 64 * 1024;
static APPROVALS: OnceLock<Mutex<VecDeque<Approval>>> = OnceLock::new();
fn approvals() -> &'static Mutex<VecDeque<Approval>> {
    APPROVALS.get_or_init(|| Mutex::new(VecDeque::new()))
}

pub(super) fn approved(
    root: &Path,
    manifest: &Manifest,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<()> {
    cancel.check().map_err(Error::from)?;
    let identity = manifest.identity();
    let root = super::input::checked_directory(root)?;
    let entries = approvals().lock().map_err(|_| Error::Storage)?;
    if entries.iter().any(|entry| {
        entry.root == root
            && entry.source == manifest.source().original
            && entry.manifest == identity
            && entry._charge.belongs_to(budget)
    }) {
        Ok(())
    } else {
        Err(Error::Unsupported)
    }
}
pub(super) fn register(
    root: &Path,
    build: &ApprovedBuild,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<()> {
    cancel.check().map_err(Error::from)?;
    let root = super::input::checked_directory(root)?;
    if root.as_os_str().as_encoded_bytes().len() > 8192 {
        return Err(Error::Limit);
    }
    let identity = build.manifest.identity();
    let charge = source::reserve(budget, MemoryKind::Transfer, APPROVAL_BYTES)?;
    let mut entries = approvals().lock().map_err(|_| Error::Storage)?;
    entries.retain(|entry| !(entry.root == root && entry.source == build.source.source.original));
    if entries.len() == MAX_APPROVALS {
        entries.pop_front();
    }
    entries.push_back(Approval {
        root,
        source: build.source.source.original,
        manifest: identity,
        _charge: charge,
    });
    Ok(())
}

fn live(
    closed: &AtomicBool,
    project_closed: Option<&AtomicBool>,
    cancel: &Cancellation,
) -> Result<()> {
    source::live(closed, cancel)?;
    if let Some(project_closed) = project_closed {
        source::live(project_closed, cancel)?;
    }
    Ok(())
}
fn verify_source(
    build: &ApprovedBuild,
    closed: &AtomicBool,
    project_closed: Option<&AtomicBool>,
    cancel: &Cancellation,
    budget: &SessionBudget,
) -> Result<()> {
    if let Some(project_closed) = project_closed {
        build
            .source
            .captured()
            .verify_owned(project_closed, closed, cancel, budget)
    } else {
        build.source.verify(closed, cancel, budget)
    }
}

pub(super) struct EncodedFile {
    bytes: Vec<u8>,
    _charge: MemoryCharge,
}
impl EncodedFile {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
pub(super) fn bytes(
    path: &Path,
    max: u64,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<EncodedFile> {
    cancel.check().map_err(Error::from)?;
    let file = source::regular(path).map_err(Error::from)?;
    let len = file.metadata().map_err(io_error)?.len();
    if len == 0 || len > max {
        return Err(Error::Limit);
    }
    let allowance = len.checked_add(1).ok_or(Error::Limit)?;
    let charge = source::reserve(budget, MemoryKind::EncodedTiles, allowance)?;
    let capacity = usize::try_from(allowance).map_err(|_| Error::Limit)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|_| Error::Limit)?;
    file.take(allowance)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 != len || bytes.capacity() as u64 > allowance {
        return Err(Error::Invalid);
    }
    cancel.check().map_err(Error::from)?;
    Ok(EncodedFile {
        bytes,
        _charge: charge,
    })
}

/// Recovered inventory census, tightened before adoption. The caller must own
/// the operation directory and actual runner completion capability first.
pub(super) fn verify_output(
    build: &ApprovedBuild,
    closed: &AtomicBool,
    project_closed: Option<&AtomicBool>,
    cancel: &Cancellation,
    budget: &SessionBudget,
) -> Result<()> {
    live(closed, project_closed, cancel)?;
    verify_source(build, closed, project_closed, cancel, budget)?;
    let root = super::input::checked_directory(&build.directory())?;
    let pyramid = build.manifest.pyramid();
    let mut top = 0usize;
    for entry in std::fs::read_dir(&root).map_err(io_error)? {
        live(closed, project_closed, cancel)?;
        let entry = entry.map_err(io_error)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| Error::Invalid)?;
        if name == "pyramid.vwt" {
            let _file = source::regular(&entry.path()).map_err(Error::from)?;
        } else {
            let level = name
                .strip_prefix('L')
                .and_then(|value| value.parse::<u8>().ok())
                .ok_or(Error::Invalid)?;
            if name != format!("L{level}") {
                return Err(Error::Invalid);
            }
            pyramid.level(level).map_err(|_| Error::Invalid)?;
            super::input::checked_directory(&entry.path())?;
        }
        top += 1;
        if top > pyramid.levels().len() + 1 {
            return Err(Error::Invalid);
        }
    }
    // Android's real producer does not write a manifest; native inventory and
    // its completed runner witness are still mandatory before it can mint this
    // type. Windows supplies a manifest and its exact bytes are checked below.
    let has_manifest = root.join("pyramid.vwt").try_exists().map_err(io_error)?;
    if top != pyramid.levels().len() + usize::from(has_manifest) {
        return Err(Error::Invalid);
    }
    if has_manifest {
        let raw = bytes(
            &root.join("pyramid.vwt"),
            MAX_MANIFEST_BYTES as u64,
            budget,
            cancel,
        )?;
        let checked =
            Manifest::decode(raw.bytes(), &build.source.source).map_err(|_| Error::Invalid)?;
        if checked != build.manifest {
            return Err(Error::Stale);
        }
    }
    for level in pyramid.levels() {
        let mut count = 0u64;
        for entry in std::fs::read_dir(root.join(format!("L{}", level.index))).map_err(io_error)? {
            live(closed, project_closed, cancel)?;
            let entry = entry.map_err(io_error)?;
            let _file = source::regular(&entry.path()).map_err(Error::from)?;
            count += 1;
            if count > u64::from(level.columns) * u64::from(level.rows) {
                return Err(Error::Invalid);
            }
        }
        if count != u64::from(level.columns) * u64::from(level.rows) {
            return Err(Error::Missing);
        }
    }
    let envelope = build
        .manifest
        .encoded_bytes()
        .checked_add(MAX_MANIFEST_BYTES as u64)
        .ok_or(Error::Limit)?;
    if envelope > build.disk_cache_bytes {
        return Err(Error::Limit);
    }
    for record in build.manifest.tiles() {
        live(closed, project_closed, cancel)?;
        let encoded = bytes(
            &root.join(record.relative_path()),
            u64::from(record.encoded_bytes),
            budget,
            cancel,
        )?;
        if encoded.bytes().len() > MAX_ENCODED_TILE_BYTES {
            return Err(Error::Limit);
        }
        record.verify(encoded.bytes()).map_err(|_| Error::Stale)?;
        let _decode = source::reserve(budget, MemoryKind::CodecWorkspace, tiles::DECODE_WORKSPACE)?;
        drop(tiles::decode(
            encoded.bytes(),
            record.key,
            pyramid.region(record.key).map_err(|_| Error::Invalid)?,
            cancel,
        )?);
    }
    verify_source(build, closed, project_closed, cancel, budget)?;
    live(closed, project_closed, cancel)
}

/// Only a private completed build can enter this function. No directory, raw
/// manifest or caller-provided approval flag can authorize an installation.
#[allow(
    clippy::too_many_arguments,
    reason = "Explicit original/build, disk query, independent close fences, cancellation and shared admission"
)]
pub(super) fn install<Q: FreeSpaceQuery>(
    root: &Path,
    build: &ApprovedBuild,
    query: Q,
    closed: &AtomicBool,
    project_closed: Option<&AtomicBool>,
    cancel: &Cancellation,
    budget: &SessionBudget,
) -> Result<()> {
    verify_output(build, closed, project_closed, cancel, budget)?;
    let _resident = source::reserve(budget, MemoryKind::CodecWorkspace, tiles::STORE_RESIDENT)?;
    let mut cache = {
        let _index = source::reserve(budget, MemoryKind::CodecWorkspace, tiles::STORE_WORKSPACE)?;
        DiskCache::with_query(root, build.disk_cache_bytes, query).map_err(source::store_error)?
    };
    {
        let _index = source::reserve(budget, MemoryKind::CodecWorkspace, tiles::STORE_WORKSPACE)?;
        cache
            .remove(&tiles::manifest_key(&build.source.source))
            .map_err(source::store_error)?;
    }
    for record in build.manifest.tiles() {
        live(closed, project_closed, cancel)?;
        let encoded = bytes(
            &build.directory().join(record.relative_path()),
            u64::from(record.encoded_bytes),
            budget,
            cancel,
        )?;
        record.verify(encoded.bytes()).map_err(|_| Error::Stale)?;
        let _index = source::reserve(budget, MemoryKind::CodecWorkspace, tiles::STORE_WORKSPACE)?;
        cache
            .put(
                &tiles::tile_key(&build.source.source, record.key),
                encoded.bytes(),
            )
            .map_err(source::store_error)?;
    }
    live(closed, project_closed, cancel)?;
    {
        let _index = source::reserve(budget, MemoryKind::CodecWorkspace, tiles::STORE_WORKSPACE)?;
        cache
            .put(
                &tiles::manifest_key(&build.source.source),
                &build.manifest.encode(),
            )
            .map_err(source::store_error)?;
    }
    for record in build.manifest.tiles() {
        if !cache.contains(&tiles::tile_key(&build.source.source, record.key)) {
            return Err(Error::Limit);
        }
    }
    if !cache.contains(&tiles::manifest_key(&build.source.source)) {
        return Err(Error::Limit);
    }
    live(closed, project_closed, cancel)?;
    // The caller registers only after its final full-binding fence or after
    // no-clobber project publication at the final (not staging) root.
    Ok(())
}

pub(super) fn release(build: Arc<ApprovedBuild>) -> Result<()> {
    let Ok(build) = Arc::try_unwrap(build) else {
        return Ok(());
    };
    let ApprovedBuild {
        manifest,
        mut output,
        source,
        platform: _,
        disk_cache_bytes: _,
        _manifest_charge,
    } = build;
    drop(manifest);
    drop(_manifest_charge);
    let output_result = output.cleanup();
    // A quarantined output retains another source Arc and its input permit.
    let source_result = super::input::release(source);
    output_result.and(source_result)
}
