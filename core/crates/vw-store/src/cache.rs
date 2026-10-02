use crate::StoreError;
use crate::blobs::{
    check_directory, check_regular_metadata, ensure_directory, open_regular_file, sync_directory,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use vw_model::AssetId;

/// Default phone tile-memory budget, expressed in bytes (256 MiB).
pub const PHONE_RAM_CACHE_BYTES: u64 = 256 * 1024 * 1024;
/// Default desktop tile-memory budget, expressed in bytes (512 MiB).
pub const DESKTOP_RAM_CACHE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;
const MANIFEST_MAX_BYTES: u64 = 16 * 1024 * 1024;

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
        let limit = entry
            .bytes
            .checked_add(1)
            .ok_or(StoreError::Invalid("cache size overflow"))?;
        let size =
            usize::try_from(limit).map_err(|_| StoreError::Invalid("cache address space"))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| StoreError::Invalid("cache allocation"))?;
        file.take(limit).read_to_end(&mut bytes)?;
        if bytes.len() as u64 != entry.bytes || AssetId::hash(&bytes) != entry.hash {
            return Err(StoreError::Corrupt("cache content"));
        }
        Ok(bytes)
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
        self.check_paths()?;
        let path = self.directory.join("index.json");
        match fs::symlink_metadata(&path) {
            Ok(metadata) => check_regular_metadata(&metadata)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let bytes = serde_json::to_vec(&self.manifest)?;
        if bytes.len() as u64 > MANIFEST_MAX_BYTES {
            return Err(StoreError::Invalid("cache manifest size"));
        }
        let mut temporary = tempfile::Builder::new()
            .prefix(".manifest-")
            .tempfile_in(&self.directory)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        let file = temporary
            .persist(&path)
            .map_err(|error| StoreError::Io(error.error))?;
        file.sync_all()?;
        sync_directory(&self.directory)
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
