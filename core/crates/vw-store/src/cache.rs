use crate::StoreError;
use crate::blobs::{
    check_directory, check_regular_metadata, ensure_directory, open_regular_file, sync_directory,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use vw_model::AssetId;

/// Default phone tile-memory budget, expressed in bytes (256 MiB).
pub const PHONE_RAM_CACHE_BYTES: u64 = 256 * 1024 * 1024;
/// Default desktop tile-memory budget, expressed in bytes (512 MiB).
pub const DESKTOP_RAM_CACHE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;
const MANIFEST_MAX_BYTES: u64 = 16 * 1024 * 1024;
const MANIFEST_BUFFER_BYTES: usize = 64 * 1024;
const STREAMING_READ_PATH_BYTES: usize = 16 * 1024;

/// Identity returned to cache owners when their payload must be released.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CacheEntryKey {
    pub cache: String,
    pub key: String,
}

/// Budget accounting does not retain the caller's actual memory payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEviction {
    pub entry: CacheEntryKey,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
struct RamEntry {
    bytes: u64,
    touched: u64,
}

/// Shared RAM accounting with global LRU eviction and per-cache totals.
/// Owners must immediately release every entry returned from `insert`/`set_budget`.
#[derive(Debug, Clone)]
pub struct RamCacheBudget {
    budget: u64,
    used: u64,
    clock: u64,
    entries: BTreeMap<CacheEntryKey, RamEntry>,
}

impl RamCacheBudget {
    pub fn new(budget: u64) -> Self {
        Self {
            budget,
            used: 0,
            clock: 0,
            entries: BTreeMap::new(),
        }
    }

    pub fn phone() -> Self {
        Self::new(PHONE_RAM_CACHE_BYTES)
    }
    pub fn desktop() -> Self {
        Self::new(DESKTOP_RAM_CACHE_BYTES)
    }
    pub fn budget(&self) -> u64 {
        self.budget
    }
    pub fn used(&self) -> u64 {
        self.used
    }

    pub fn used_by(&self, cache: &str) -> u64 {
        self.entries
            .iter()
            .filter(|(key, _)| key.cache == cache)
            .map(|(_, entry)| entry.bytes)
            .sum()
    }

    pub fn contains(&self, cache: &str, key: &str) -> bool {
        self.entries.contains_key(&CacheEntryKey {
            cache: cache.into(),
            key: key.into(),
        })
    }

    /// Register or replace a payload and return the globally oldest evictions.
    /// An item larger than the budget is rejected without modifying accounting.
    pub fn insert(
        &mut self,
        cache: &str,
        key: &str,
        bytes: u64,
    ) -> Result<Vec<CacheEviction>, StoreError> {
        validate_key(cache)?;
        validate_key(key)?;
        if bytes > self.budget {
            return Err(StoreError::Invalid("RAM cache item exceeds budget"));
        }
        let identity = CacheEntryKey {
            cache: cache.into(),
            key: key.into(),
        };
        if !self.entries.contains_key(&identity) && self.entries.len() >= MAX_ENTRIES {
            return Err(StoreError::Invalid("RAM cache entry limit"));
        }
        let next_clock = self
            .clock
            .checked_add(1)
            .ok_or(StoreError::Invalid("cache clock overflow"))?;
        let previous = self.entries.get(&identity).map_or(0, |entry| entry.bytes);
        let total = self
            .used
            .checked_sub(previous)
            .and_then(|value| value.checked_add(bytes))
            .ok_or(StoreError::Invalid("RAM cache accounting overflow"))?;
        self.clock = next_clock;
        self.used = total;
        self.entries.insert(
            identity,
            RamEntry {
                bytes,
                touched: next_clock,
            },
        );
        Ok(self.evict_to_budget())
    }

    pub fn touch(&mut self, cache: &str, key: &str) -> Result<bool, StoreError> {
        let identity = CacheEntryKey {
            cache: cache.into(),
            key: key.into(),
        };
        let Some(entry) = self.entries.get_mut(&identity) else {
            return Ok(false);
        };
        self.clock = self
            .clock
            .checked_add(1)
            .ok_or(StoreError::Invalid("cache clock overflow"))?;
        entry.touched = self.clock;
        Ok(true)
    }

    pub fn remove(&mut self, cache: &str, key: &str) -> Option<u64> {
        let entry = self.entries.remove(&CacheEntryKey {
            cache: cache.into(),
            key: key.into(),
        })?;
        self.used -= entry.bytes;
        Some(entry.bytes)
    }

    pub fn set_budget(&mut self, budget: u64) -> Vec<CacheEviction> {
        self.budget = budget;
        self.evict_to_budget()
    }

    fn evict_to_budget(&mut self) -> Vec<CacheEviction> {
        let mut removed = Vec::new();
        while self.used > self.budget {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.touched)
                .map(|(key, _)| key.clone());
            let Some(key) = oldest else {
                break;
            };
            if let Some(entry) = self.entries.remove(&key) {
                self.used -= entry.bytes;
                removed.push(CacheEviction {
                    entry: key,
                    bytes: entry.bytes,
                });
            }
        }
        removed
    }
}

/// Drive capacity and bytes currently available to this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskSpace {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

/// Injectable free-space observation; tests can model exact reserve boundaries.
pub trait FreeSpaceQuery {
    fn query(&self, path: &Path) -> Result<DiskSpace, StoreError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RealFreeSpace;

impl FreeSpaceQuery for RealFreeSpace {
    fn query(&self, path: &Path) -> Result<DiskSpace, StoreError> {
        Ok(DiskSpace {
            total_bytes: fs2::total_space(path)?,
            available_bytes: fs2::available_space(path)?,
        })
    }
}

/// The specification uses decimal GB: max(5 GB, min(5% of capacity, 10 GB)).
pub fn disk_reserve_bytes(total_bytes: u64) -> u64 {
    (total_bytes / 20).clamp(5_000_000_000, 10_000_000_000)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskCacheStatus {
    pub used_bytes: u64,
    pub budget_bytes: u64,
    pub reserve_bytes: u64,
    pub available_bytes: u64,
    pub low_space: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiskEntry {
    filename: String,
    hash: AssetId,
    bytes: u64,
    touched: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    clock: u64,
    entries: BTreeMap<String, DiskEntry>,
}

/// Persistent LRU cache. Only entries in this manager's validated manifest can
/// be evicted. Unrecognized files, sibling caches and original blobs are untouched.
pub struct DiskCache<Q: FreeSpaceQuery = RealFreeSpace> {
    root: PathBuf,
    directory: PathBuf,
    _lock: File,
    query: Q,
    budget: u64,
    used: u64,
    manifest: Manifest,
}

impl DiskCache<RealFreeSpace> {
    pub fn new(project_root: &Path, budget: u64) -> Result<Self, StoreError> {
        Self::with_query(project_root, budget, RealFreeSpace)
    }
}

impl<Q: FreeSpaceQuery> DiskCache<Q> {
    pub fn with_query(project_root: &Path, budget: u64, query: Q) -> Result<Self, StoreError> {
        let root = project_root.canonicalize()?;
        check_directory(&root)?;
        let cache = root.join("cache");
        ensure_directory(&cache)?;
        let directory = cache.join("vw-store-v1");
        ensure_directory(&directory)?;
        let lock_path = directory.join("manager.lock");
        let lock = open_lock(&lock_path)?;
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock
                || error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
            {
                StoreError::Busy
            } else {
                error.into()
            }
        })?;
        let manifest_path = directory.join("index.json");
        let mut manifest = match fs::symlink_metadata(&manifest_path) {
            Ok(metadata) => {
                check_regular_metadata(&metadata)?;
                if metadata.len() > MANIFEST_MAX_BYTES {
                    return Err(StoreError::Invalid("cache manifest size"));
                }
                let mut bytes = Vec::new();
                open_regular_file(&manifest_path)?
                    .take(MANIFEST_MAX_BYTES + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() as u64 > MANIFEST_MAX_BYTES {
                    return Err(StoreError::Invalid("cache manifest size"));
                }
                serde_json::from_slice::<Manifest>(&bytes)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Manifest {
                version: 1,
                clock: 0,
                entries: BTreeMap::new(),
            },
            Err(error) => return Err(error.into()),
        };
        if manifest.version != 1 || manifest.entries.len() > MAX_ENTRIES {
            return Err(StoreError::Invalid("cache manifest version or count"));
        }
        let mut used = 0u64;
        let mut filenames = BTreeSet::new();
        let mut absent = Vec::new();
        for (key, entry) in &manifest.entries {
            validate_key(key)?;
            if entry.filename != cache_filename(key, &entry.hash)
                || entry.touched > manifest.clock
                || !filenames.insert(entry.filename.clone())
            {
                return Err(StoreError::Corrupt("cache manifest entry"));
            }
            match fs::symlink_metadata(directory.join(&entry.filename)) {
                Ok(metadata) => {
                    check_regular_metadata(&metadata)?;
                    if metadata.len() != entry.bytes {
                        return Err(StoreError::Corrupt("cache size"));
                    }
                    used = used
                        .checked_add(entry.bytes)
                        .ok_or(StoreError::Invalid("disk cache accounting overflow"))?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    absent.push(key.clone())
                }
                Err(error) => return Err(error.into()),
            }
        }
        for key in absent {
            manifest.entries.remove(&key);
        }
        let mut result = Self {
            root,
            directory,
            _lock: lock,
            query,
            budget,
            used,
            manifest,
        };
        result.save_manifest()?;
        result.space()?;
        result.enforce_budget()?;
        Ok(result)
    }

    /// Current accounting and reserve state. Reading existing entries remains
    /// possible when a drive has fallen below its reserve.
    pub fn status(&self) -> Result<DiskCacheStatus, StoreError> {
        self.check_paths()?;
        let space = self.space()?;
        let reserve = disk_reserve_bytes(space.total_bytes);
        Ok(DiskCacheStatus {
            used_bytes: self.used,
            budget_bytes: self.budget,
            reserve_bytes: reserve,
            available_bytes: space.available_bytes,
            low_space: space.available_bytes < reserve,
        })
    }

    pub fn contains(&self, key: &str) -> bool {
        self.manifest.entries.contains_key(key)
    }

    /// Return verified derived bytes and update durable LRU order.
    pub fn get(&mut self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        self.check_paths()?;
        let Some(entry) = self.manifest.entries.get(key).cloned() else {
            return Ok(None);
        };
        let bytes = self.read_entry(&entry)?;
        self.touch(key)?;
        Ok(Some(bytes))
    }

    /// Refuse an encoded entry before allocation when a consumer admits a
    /// smaller payload bound than the entire cache. Integrity/LRU semantics are
    /// identical to get; unknown files remain untouched.
    pub fn get_bounded(
        &mut self,
        key: &str,
        max_bytes: u64,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        self.check_paths()?;
        let Some(entry) = self.manifest.entries.get(key).cloned() else {
            return Ok(None);
        };
        if entry.bytes > max_bytes {
            return Err(StoreError::Invalid("cache read limit"));
        }
        let bytes = self.read_entry(&entry)?;
        self.touch(key)?;
        Ok(Some(bytes))
    }

    /// Verified bounded bytes with durable LRU and fixed-size index-write work.
    /// Before calling, reserve max_bytes + 1 for the returned allocation (or the
    /// exact known entry length + 1) and a separate 1 MiB transient envelope.
    /// Retain the existing resident index charge; opening/parsing this manager
    /// still requires its larger workspace admission. This API creates no pool.
    /// The transient envelope applies to source-controlled allocations, not RSS.
    pub fn get_bounded_streaming(
        &mut self,
        key: &str,
        max_bytes: u64,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        // Bound PathBuf/temporary/OS path-conversion scratch before allocation.
        // Existing metadata entries already have <=128-byte keys, fixed 70-byte
        // filenames and 64-byte content hashes from with_query/put validation.
        if self.root.as_os_str().as_encoded_bytes().len() > STREAMING_READ_PATH_BYTES
            || self.directory.as_os_str().as_encoded_bytes().len() > STREAMING_READ_PATH_BYTES
        {
            return Err(StoreError::Invalid("cache read path limit"));
        }
        self.get_bounded(key, max_bytes)
    }

    /// Add derived bytes after budget and drive-reserve checks. Cache entries may
    /// be evicted to make room; originals and unrecognized files never are.
    pub fn put(&mut self, key: &str, bytes: &[u8]) -> Result<(), StoreError> {
        validate_key(key)?;
        self.check_paths()?;
        let size = bytes.len() as u64;
        if size > self.budget {
            return Err(StoreError::Invalid("disk cache item exceeds budget"));
        }
        if !self.contains(key) && self.manifest.entries.len() >= MAX_ENTRIES {
            return Err(StoreError::Invalid("disk cache entry limit"));
        }
        let hash = AssetId::hash(bytes);
        if let Some(old) = self.manifest.entries.get(key).cloned()
            && old.hash == hash
        {
            self.verify_entry(&old)?;
            self.touch(key)?;
            return self.enforce_published_entry(key);
        }
        let filename = cache_filename(key, &hash);
        let final_path = self.directory.join(&filename);
        match fs::symlink_metadata(&final_path) {
            Ok(_) => return Err(StoreError::Invalid("unrecognized cache target exists")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        self.remove(key)?;
        self.enforce_limits(size)?;
        let next_clock = self
            .manifest
            .clock
            .checked_add(1)
            .ok_or(StoreError::Invalid("cache clock overflow"))?;
        let total = self
            .used
            .checked_add(size)
            .ok_or(StoreError::Invalid("disk cache accounting overflow"))?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".pending-")
            .tempfile_in(&self.directory)?;
        temporary.write_all(bytes)?;
        temporary.as_file().sync_all()?;
        self.check_paths()?;
        let file = temporary
            .persist_noclobber(&final_path)
            .map_err(|error| StoreError::Io(error.error))?;
        file.sync_all()?;
        drop(file);
        sync_directory(&self.directory)?;
        self.manifest.clock = next_clock;
        self.manifest.entries.insert(
            key.into(),
            DiskEntry {
                filename,
                hash,
                bytes: size,
                touched: next_clock,
            },
        );
        self.used = total;
        self.save_manifest()?;
        self.enforce_published_entry(key)
    }

    fn enforce_published_entry(&mut self, key: &str) -> Result<(), StoreError> {
        // File allocation, manifest overhead and other disk users can consume
        // more than the preflight estimate. Reclaim only proven cache entries.
        self.enforce_limits(0)?;
        if self.contains(key) {
            Ok(())
        } else {
            Err(StoreError::LowSpace)
        }
    }

    pub fn remove(&mut self, key: &str) -> Result<bool, StoreError> {
        self.check_paths()?;
        let Some(entry) = self.manifest.entries.get(key).cloned() else {
            return Ok(false);
        };
        // A replaced/corrupt file is no longer proven to be our managed entry.
        self.verify_entry(&entry)?;
        fs::remove_file(self.directory.join(&entry.filename))?;
        sync_directory(&self.directory)?;
        self.manifest.entries.remove(key);
        self.used = self
            .used
            .checked_sub(entry.bytes)
            .ok_or(StoreError::Corrupt("cache accounting"))?;
        self.save_manifest()?;
        Ok(true)
    }

    pub fn set_budget(&mut self, budget: u64) -> Result<(), StoreError> {
        self.budget = budget;
        self.enforce_budget()
    }

    fn enforce_budget(&mut self) -> Result<(), StoreError> {
        while self.used > self.budget {
            let key = self
                .manifest
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.touched)
                .map(|(key, _)| key.clone())
                .ok_or(StoreError::Corrupt("cache accounting"))?;
            self.remove(&key)?;
        }
        Ok(())
    }

    fn enforce_limits(&mut self, incoming: u64) -> Result<(), StoreError> {
        loop {
            self.check_paths()?;
            let space = self.space()?;
            let reserve = disk_reserve_bytes(space.total_bytes);
            let projected = self
                .used
                .checked_add(incoming)
                .ok_or(StoreError::Invalid("disk cache accounting overflow"))?;
            let needed = reserve.checked_add(incoming).ok_or(StoreError::LowSpace)?;
            if projected <= self.budget && space.available_bytes >= needed {
                return Ok(());
            }
            let oldest = self
                .manifest
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.touched)
                .map(|(key, _)| key.clone());
            let Some(key) = oldest else {
                return if space.available_bytes < needed {
                    Err(StoreError::LowSpace)
                } else {
                    Err(StoreError::Invalid("disk cache item exceeds budget"))
                };
            };
            self.remove(&key)?;
        }
    }

    fn touch(&mut self, key: &str) -> Result<(), StoreError> {
        let next_clock = self
            .manifest
            .clock
            .checked_add(1)
            .ok_or(StoreError::Invalid("cache clock overflow"))?;
        let entry = self
            .manifest
            .entries
            .get_mut(key)
            .ok_or(StoreError::Corrupt("cache entry missing"))?;
        self.manifest.clock = next_clock;
        entry.touched = next_clock;
        self.save_manifest()
    }

    fn read_entry(&self, entry: &DiskEntry) -> Result<Vec<u8>, StoreError> {
        let file = open_regular_file(&self.directory.join(&entry.filename))?;
        if file.metadata()?.len() != entry.bytes {
            return Err(StoreError::Corrupt("cache size"));
        }
        read_payload(file, entry)
    }

    fn verify_entry(&self, entry: &DiskEntry) -> Result<(), StoreError> {
        let file = open_regular_file(&self.directory.join(&entry.filename))?;
        if file.metadata()?.len() != entry.bytes {
            return Err(StoreError::Corrupt("cache size"));
        }
        let limit = entry
            .bytes
            .checked_add(1)
            .ok_or(StoreError::Invalid("cache size overflow"))?;
        let mut file = file.take(limit);
        let mut hasher = blake3::Hasher::new();
        let mut length = 0u64;
        let mut buffer = [0u8; 65_536];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            length = length
                .checked_add(count as u64)
                .ok_or(StoreError::Invalid("cache size overflow"))?;
            hasher.update(&buffer[..count]);
        }
        if length != entry.bytes || hasher.finalize().as_bytes() != &entry.hash.bytes() {
            return Err(StoreError::Corrupt("cache content"));
        }
        Ok(())
    }

    fn check_paths(&self) -> Result<(), StoreError> {
        check_directory(&self.root)?;
        check_directory(&self.root.join("cache"))?;
        check_directory(&self.directory)
    }

    fn space(&self) -> Result<DiskSpace, StoreError> {
        let space = self.query.query(&self.directory)?;
        if space.available_bytes > space.total_bytes {
            return Err(StoreError::Invalid("free-space response"));
        }
        Ok(space)
    }

    fn save_manifest(&self) -> Result<(), StoreError> {
        self.save_manifest_with_limit(MANIFEST_MAX_BYTES)
    }

    fn save_manifest_with_limit(&self, limit: u64) -> Result<(), StoreError> {
        if limit > MANIFEST_MAX_BYTES {
            return Err(StoreError::Invalid("cache manifest size"));
        }
        self.check_paths()?;
        let path = self.directory.join("index.json");
        match fs::symlink_metadata(&path) {
            Ok(metadata) => check_regular_metadata(&metadata)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut temporary = tempfile::Builder::new()
            .prefix(".manifest-")
            .tempfile_in(&self.directory)?;
        {
            let buffered = BufWriter::with_capacity(MANIFEST_BUFFER_BYTES, &mut temporary);
            if buffered.capacity() != MANIFEST_BUFFER_BYTES {
                return Err(StoreError::Invalid("cache manifest allocation"));
            }
            let mut bounded = ManifestWriter {
                inner: buffered,
                written: 0,
                limit,
                exceeded: false,
            };
            let serialized = serde_json::to_writer(&mut bounded, &self.manifest);
            if bounded.exceeded {
                return Err(StoreError::Invalid("cache manifest size"));
            }
            serialized?;
            bounded.flush()?;
        }
        temporary.as_file().sync_all()?;
        let file = temporary
            .persist(&path)
            .map_err(|error| StoreError::Io(error.error))?;
        file.sync_all()?;
        sync_directory(&self.directory)
    }
}

// Metadata was checked on the opened regular file before this call. A later
// truncate/growth still fails via read_exact/the stack sentinel, with no Vec
// append or allocation beyond the declared-length admission.
fn read_payload(mut file: impl Read, entry: &DiskEntry) -> Result<Vec<u8>, StoreError> {
    let limit = entry
        .bytes
        .checked_add(1)
        .ok_or(StoreError::Invalid("cache size overflow"))?;
    let size = usize::try_from(limit).map_err(|_| StoreError::Invalid("cache address space"))?;
    let declared = size - 1;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| StoreError::Invalid("cache allocation"))?;
    if bytes.capacity() > size {
        return Err(StoreError::Invalid("cache allocation"));
    }
    bytes.resize(declared, 0);
    if let Err(error) = file.read_exact(&mut bytes) {
        return Err(if error.kind() == std::io::ErrorKind::UnexpectedEof {
            StoreError::Corrupt("cache content")
        } else {
            error.into()
        });
    }
    // A stack sentinel detects growth without extending or reallocating Vec.
    let mut sentinel = [0];
    loop {
        match file.read(&mut sentinel) {
            Ok(0) => break,
            Ok(_) => return Err(StoreError::Corrupt("cache content")),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
    if AssetId::hash(&bytes) != entry.hash {
        return Err(StoreError::Corrupt("cache content"));
    }
    Ok(bytes)
}

/// Counts serialized bytes before they enter BufWriter, including buffered data.
/// An over-limit write is refused wholly; it never reaches the temporary file.
struct ManifestWriter<W> {
    inner: W,
    written: u64,
    limit: u64,
    exceeded: bool,
}
impl<W: Write> Write for ManifestWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let count = match u64::try_from(bytes.len()) {
            Ok(count) => count,
            Err(_) => {
                self.exceeded = true;
                return Err(std::io::Error::other("cache manifest size"));
            }
        };
        if self
            .written
            .checked_add(count)
            .is_none_or(|end| end > self.limit)
        {
            self.exceeded = true;
            return Err(std::io::Error::other("cache manifest size"));
        }
        let written = self.inner.write(bytes)?;
        self.written += written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn validate_key(key: &str) -> Result<(), StoreError> {
    if key.is_empty()
        || key.len() > 128
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(StoreError::Invalid("cache key"));
    }
    Ok(())
}

fn cache_filename(key: &str, hash: &AssetId) -> String {
    let input = format!("VisualWorkbench.Cache.v1\0{key}\0{hash}");
    format!("{}.cache", AssetId::hash(input.as_bytes()))
}

fn open_lock(path: &Path) -> Result<File, StoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => check_regular_metadata(&metadata)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000);
    }
    #[cfg(any(target_os = "android", target_os = "linux"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x0002_0000);
    }
    let file = options.open(path)?;
    check_regular_metadata(&file.metadata()?)?;
    Ok(file)
}

#[cfg(test)]
mod streaming_tests {
    use super::*;

    #[test]
    fn post_metadata_growth_and_truncation_refuse_without_vec_growth() -> Result<(), StoreError> {
        let mut original = tempfile::NamedTempFile::new()?;
        original.write_all(b"abcdef")?;
        let entry = DiskEntry {
            filename: String::new(),
            bytes: 6,
            hash: AssetId::hash(b"abcdef"),
            touched: 0,
        };
        let reader = open_regular_file(original.path())?;
        assert_eq!(reader.metadata()?.len(), entry.bytes);
        original.write_all(b"g")?;
        assert!(matches!(
            read_payload(reader, &entry),
            Err(StoreError::Corrupt("cache content"))
        ));
        original.as_file().set_len(6)?;
        let reader = open_regular_file(original.path())?;
        assert_eq!(reader.metadata()?.len(), entry.bytes);
        original.as_file().set_len(5)?;
        assert!(matches!(
            read_payload(reader, &entry),
            Err(StoreError::Corrupt("cache content"))
        ));
        Ok(())
    }

    #[test]
    fn manifest_writer_accepts_exact_limit_and_never_forwards_an_excess_byte()
    -> Result<(), StoreError> {
        let mut sink = Vec::new();
        let mut writer = ManifestWriter {
            inner: &mut sink,
            written: 0,
            limit: 7,
            exceeded: false,
        };
        writer.write_all(b"1234567")?;
        assert!(writer.write_all(b"8").is_err());
        assert!(writer.exceeded);
        assert_eq!(writer.written, 7);
        assert_eq!(sink, b"1234567");
        Ok(())
    }

    #[test]
    fn production_manifest_limit_and_buffer_capacity_are_exact() -> Result<(), StoreError> {
        let buffer = BufWriter::with_capacity(MANIFEST_BUFFER_BYTES, std::io::sink());
        assert_eq!(buffer.capacity(), 64 * 1024);
        let mut writer = ManifestWriter {
            inner: buffer,
            written: 0,
            limit: MANIFEST_MAX_BYTES,
            exceeded: false,
        };
        let chunk = [b'x'; MANIFEST_BUFFER_BYTES];
        for _ in 0..MANIFEST_MAX_BYTES / MANIFEST_BUFFER_BYTES as u64 {
            writer.write_all(&chunk)?;
        }
        assert_eq!(writer.written, 16 * 1024 * 1024);
        assert!(writer.write_all(b"x").is_err());
        assert!(writer.exceeded);
        assert_eq!(writer.written, MANIFEST_MAX_BYTES);
        writer.flush()?;
        Ok(())
    }

    #[test]
    fn serialization_limit_failure_preserves_old_index_and_leaves_no_temporary()
    -> Result<(), StoreError> {
        struct Plenty;
        impl FreeSpaceQuery for Plenty {
            fn query(&self, _path: &Path) -> Result<DiskSpace, StoreError> {
                Ok(DiskSpace {
                    total_bytes: 200_000_000_000,
                    available_bytes: 150_000_000_000,
                })
            }
        }
        let owned = tempfile::tempdir()?;
        let mut cache = DiskCache::with_query(owned.path(), 32, Plenty)?;
        cache.put("tile", b"encoded")?;
        let index = cache.directory.join("index.json");
        let before = fs::read(&index)?;
        assert!(matches!(
            cache.save_manifest_with_limit(8),
            Err(StoreError::Invalid("cache manifest size"))
        ));
        assert_eq!(fs::read(&index)?, before);
        for entry in fs::read_dir(&cache.directory)? {
            assert!(
                !entry?
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".manifest-")
            );
        }
        assert_eq!(
            cache.get_bounded_streaming("tile", 7)?,
            Some(b"encoded".to_vec())
        );
        Ok(())
    }
}
