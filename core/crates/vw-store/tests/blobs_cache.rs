#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::error::Error;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use vw_model::AssetId;
use vw_store::{
    BlobStore, DESKTOP_RAM_CACHE_BYTES, DiskCache, DiskSpace, FreeSpaceQuery,
    PHONE_RAM_CACHE_BYTES, RamCacheBudget, RealFreeSpace, StoreError, disk_reserve_bytes,
};
type TestResult = Result<(), Box<dyn Error>>;
fn fixture() -> tempfile::TempDir {
    #[cfg(target_os = "android")]
    {
        tempfile::tempdir_in(std::env::current_dir().expect("runner directory"))
            .expect("Android fixture")
    }
    #[cfg(not(target_os = "android"))]
    {
        tempfile::tempdir().expect("fixture")
    }
}
#[derive(Clone)]
struct SimulatedSpace {
    total: u64,
    free: Arc<AtomicU64>,
    subtract_cache: bool,
}
impl SimulatedSpace {
    fn plenty() -> Self {
        Self::new(200_000_000_000, 150_000_000_000)
    }
    fn new(total: u64, free: u64) -> Self {
        Self {
            total,
            free: Arc::new(AtomicU64::new(free)),
            subtract_cache: false,
        }
    }
    fn set(&self, bytes: u64) {
        self.free.store(bytes, Ordering::SeqCst);
    }
}
impl FreeSpaceQuery for SimulatedSpace {
    fn query(&self, path: &Path) -> Result<DiskSpace, StoreError> {
        let mut available = self.free.load(Ordering::SeqCst);
        if self.subtract_cache {
            for item in fs::read_dir(path)? {
                let item = item?;
                if item.path().extension().is_some_and(|ext| ext == "cache") {
                    available = available.saturating_sub(item.metadata()?.len());
                }
            }
        }
        Ok(DiskSpace {
            total_bytes: self.total,
            available_bytes: available,
        })
    }
}
#[test]
fn blob_layout_streaming_roundtrip_and_empty_asset() -> TestResult {
    let temp = fixture();
    let store = BlobStore::new(temp.path())?;
    let bytes: Vec<u8> = (0..200_000).map(|i| (i % 251) as u8).collect();
    let id = store.put_reader(&mut std::io::Cursor::new(&bytes))?;
    assert_eq!(id, AssetId::hash(&bytes));
    assert_eq!(store.verify(&id)?, bytes.len() as u64);
    assert_eq!(store.read(&id)?, bytes);
    assert_eq!(
        store.path(&id)?,
        temp.path()
            .canonicalize()?
            .join("blobs")
            .join(&id.as_str()[..2])
            .join(&id.as_str()[2..4])
            .join(id.as_str())
    );
    let empty = store.put(&[])?;
    assert_eq!(store.read(&empty)?, Vec::<u8>::new());
    assert_eq!(store.verify(&empty)?, 0);
    assert_eq!(
        fs::read_dir(temp.path().join("blobs/.incoming-v1"))?.count(),
        0
    );
    Ok(())
}
#[test]
fn eight_concurrent_writers_install_identical_original_once() -> TestResult {
    let temp = fixture();
    let store = BlobStore::new(temp.path())?;
    let bytes = Arc::new(vec![77; 1_048_577]);
    let barrier = Arc::new(Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let store = store.clone();
            let bytes = Arc::clone(&bytes);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                store.put(&bytes)
            })
        })
        .collect();
    let expected = AssetId::hash(&bytes);
    for worker in workers {
        assert_eq!(worker.join().expect("writer thread")?, expected);
    }
    assert_eq!(store.verify(&expected)?, bytes.len() as u64);
    assert_eq!(
        fs::read_dir(store.path(&expected)?.parent().expect("shard"))?.count(),
        1
    );
    assert_eq!(
        fs::read_dir(temp.path().join("blobs/.incoming-v1"))?.count(),
        0
    );
    Ok(())
}
#[test]
fn corruption_is_detected_and_original_is_never_overwritten() -> TestResult {
    let temp = fixture();
    let store = BlobStore::new(temp.path())?;
    let original = b"immutable-original";
    let id = store.put(original)?;
    let path = store.path(&id)?;
    fs::write(&path, b"changed!--original")?;
    assert!(matches!(store.read(&id), Err(StoreError::Corrupt(_))));
    assert!(matches!(store.verify(&id), Err(StoreError::Corrupt(_))));
    assert!(matches!(store.put(original), Err(StoreError::Corrupt(_))));
    assert_eq!(fs::read(&path)?, b"changed!--original");
    Ok(())
}
#[test]
fn reader_failure_leaves_no_partial_original_or_temp_file() -> TestResult {
    struct FailingReader(bool);
    impl std::io::Read for FailingReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.0 {
                return Err(std::io::Error::other("synthetic fixture failure"));
            }
            self.0 = true;
            buffer[..4].copy_from_slice(b"part");
            Ok(4)
        }
    }
    let temp = fixture();
    let store = BlobStore::new(temp.path())?;
    assert!(matches!(
        store.put_reader(&mut FailingReader(false)),
        Err(StoreError::Io(_))
    ));
    assert_eq!(
        fs::read_dir(temp.path().join("blobs/.incoming-v1"))?.count(),
        0
    );
    assert!(!store.path(&AssetId::hash(b"part"))?.exists());
    Ok(())
}
#[test]
fn non_directory_shard_is_rejected_and_preserved() -> TestResult {
    let temp = fixture();
    let store = BlobStore::new(temp.path())?;
    let bytes = b"shard-target";
    let id = AssetId::hash(bytes);
    let shard = temp.path().join("blobs").join(&id.as_str()[..2]);
    fs::write(&shard, b"unrelated file")?;
    assert!(store.put(bytes).is_err());
    assert!(store.path(&id).is_err());
    assert_eq!(fs::read(shard)?, b"unrelated file");
    Ok(())
}
#[cfg(unix)]
#[test]
fn blob_symlink_shards_and_file_targets_are_rejected() -> TestResult {
    use std::os::unix::fs::symlink;
    let temp = fixture();
    let outside = fixture();
    let store = BlobStore::new(temp.path())?;
    let bytes = b"symlink fixture";
    let id = AssetId::hash(bytes);
    let shard = temp.path().join("blobs").join(&id.as_str()[..2]);
    symlink(outside.path(), &shard)?;
    assert!(store.put(bytes).is_err());
    assert!(store.path(&id).is_err());
    assert_eq!(fs::read_dir(outside.path())?.count(), 0);
    fs::remove_file(&shard)?;
    let id = store.put(bytes)?;
    let path = store.path(&id)?;
    fs::remove_file(&path)?;
    let foreign = outside.path().join("foreign");
    fs::write(&foreign, bytes)?;
    symlink(&foreign, &path)?;
    assert!(store.read(&id).is_err());
    assert!(store.put(bytes).is_err());
    assert_eq!(fs::read(&foreign)?, bytes);
    Ok(())
}
#[test]
fn ram_defaults_are_bytes_and_named_usage_is_exact() -> TestResult {
    assert_eq!(RamCacheBudget::phone().budget(), 256 * 1024 * 1024);
    assert_eq!(RamCacheBudget::desktop().budget(), 512 * 1024 * 1024);
    assert_eq!(PHONE_RAM_CACHE_BYTES, 268_435_456);
    assert_eq!(DESKTOP_RAM_CACHE_BYTES, 536_870_912);
    let mut budget = RamCacheBudget::new(100);
    assert!(budget.insert("tiles", "a", 30)?.is_empty());
    assert!(budget.insert("thumbnails", "a", 20)?.is_empty());
    assert!(budget.insert("tiles", "b", 10)?.is_empty());
    assert_eq!(budget.used(), 60);
    assert_eq!(budget.used_by("tiles"), 40);
    assert_eq!(budget.used_by("thumbnails"), 20);
    assert_eq!(budget.used_by("other"), 0);
    Ok(())
}
#[test]
fn ram_lru_crosses_named_caches_and_touch_changes_order() -> TestResult {
    let mut budget = RamCacheBudget::new(60);
    budget.insert("tiles", "a", 20)?;
    budget.insert("thumbs", "b", 20)?;
    budget.insert("tiles", "c", 20)?;
    assert!(budget.touch("tiles", "a")?);
    let removed = budget.insert("stroke", "d", 35)?;
    assert_eq!(
        removed
            .iter()
            .map(|e| (e.entry.cache.as_str(), e.entry.key.as_str(), e.bytes))
            .collect::<Vec<_>>(),
        vec![("thumbs", "b", 20), ("tiles", "c", 20)]
    );
    assert_eq!(budget.used(), 55);
    assert!(budget.contains("tiles", "a"));
    assert!(!budget.touch("missing", "none")?);
    Ok(())
}
#[test]
fn ram_replacement_shrink_and_remove_have_no_double_accounting() -> TestResult {
    let mut budget = RamCacheBudget::new(100);
    budget.insert("tiles", "a", 80)?;
    budget.insert("tiles", "a", 20)?;
    budget.insert("tiles", "b", 70)?;
    assert_eq!(budget.used(), 90);
    let removed = budget.set_budget(70);
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].entry.key, "a");
    assert_eq!(budget.remove("tiles", "b"), Some(70));
    assert_eq!(budget.remove("tiles", "b"), None);
    assert_eq!(budget.used(), 0);
    Ok(())
}
#[test]
fn ram_invalid_oversize_and_overflow_requests_are_atomic() -> TestResult {
    let mut budget = RamCacheBudget::new(u64::MAX);
    budget.insert("tiles", "a", u64::MAX - 4)?;
    assert!(budget.insert("tiles", "b", 8).is_err());
    assert_eq!(budget.used(), u64::MAX - 4);
    assert!(!budget.contains("tiles", "b"));
    assert!(budget.insert("../bad", "a", 0).is_err());
    budget.set_budget(10);
    budget.insert("tiles", "a", 4)?;
    assert!(budget.insert("tiles", "a", 11).is_err());
    assert_eq!(budget.used(), 4);
    assert_eq!(budget.set_budget(0).len(), 1);
    assert_eq!(budget.used(), 0);
    Ok(())
}
#[test]
fn reserve_formula_has_exact_decimal_floor_slope_and_ceiling() {
    for (total, expected) in [
        (0, 5_000_000_000),
        (99_999_999_999, 5_000_000_000),
        (100_000_000_000, 5_000_000_000),
        (150_000_000_000, 7_500_000_000),
        (200_000_000_000, 10_000_000_000),
        (u64::MAX, 10_000_000_000),
    ] {
        assert_eq!(disk_reserve_bytes(total), expected);
    }
}
#[test]
fn real_space_query_reports_a_consistent_filesystem_capacity() -> TestResult {
    let temp = fixture();
    let space = RealFreeSpace.query(temp.path())?;
    assert!(space.total_bytes > 0);
    assert!(space.available_bytes <= space.total_bytes);
    Ok(())
}
#[test]
fn disk_lru_is_durable_and_eviction_preserves_originals_and_unknown_files() -> TestResult {
    let temp = fixture();
    let originals = BlobStore::new(temp.path())?;
    let original_id = originals.put(b"irreplaceable original")?;
    let query = SimulatedSpace::plenty();
    {
        let mut cache = DiskCache::with_query(temp.path(), 12, query.clone())?;
        fs::write(temp.path().join("cache/other-cache"), b"other cache owner")?;
        fs::write(
            temp.path().join("cache/vw-store-v1/foreign.cache"),
            b"unknown",
        )?;
        cache.put("a", b"aaaa")?;
        cache.put("b", b"bbbb")?;
        cache.put("c", b"cccc")?;
        assert_eq!(cache.get("a")?, Some(b"aaaa".to_vec()));
    }
    let mut cache = DiskCache::with_query(temp.path(), 12, query)?;
    cache.put("d", b"dddddddd")?;
    assert!(cache.contains("a"));
    assert!(!cache.contains("b"));
    assert!(!cache.contains("c"));
    assert!(cache.contains("d"));
    assert_eq!(cache.status()?.used_bytes, 12);
    cache.set_budget(0)?;
    assert_eq!(cache.status()?.used_bytes, 0);
    assert_eq!(originals.read(&original_id)?, b"irreplaceable original");
    assert_eq!(
        fs::read(temp.path().join("cache/other-cache"))?,
        b"other cache owner"
    );
    assert_eq!(
        fs::read(temp.path().join("cache/vw-store-v1/foreign.cache"))?,
        b"unknown"
    );
    Ok(())
}
#[test]
fn disk_reserve_exact_boundary_passes_and_one_byte_short_is_clear_low_space() -> TestResult {
    let temp = fixture();
    let query = SimulatedSpace::new(100_000_000_000, 5_000_000_004);
    let mut cache = DiskCache::with_query(temp.path(), 16, query.clone())?;
    cache.put("exact", b"1234")?;
    query.set(5_000_000_003);
    let error = cache
        .put("short", b"1234")
        .expect_err("one byte below reserve allocation");
    assert!(matches!(error, StoreError::LowSpace));
    assert_eq!(
        error.to_string(),
        "not enough free space for caching; original assets are preserved"
    );
    assert!(!cache.contains("short"));
    query.set(4_999_999_999);
    assert!(cache.status()?.low_space);
    Ok(())
}
#[test]
fn disk_eviction_can_restore_drive_reserve_before_writing() -> TestResult {
    let temp = fixture();
    let mut query = SimulatedSpace::new(100_000_000_000, 5_000_000_008);
    query.subtract_cache = true;
    let mut cache = DiskCache::with_query(temp.path(), 100, query)?;
    cache.put("old", b"12345678")?;
    assert_eq!(cache.status()?.available_bytes, 5_000_000_000);
    cache.put("new", b"abcd")?;
    assert!(!cache.contains("old"));
    assert_eq!(cache.get("new")?, Some(b"abcd".to_vec()));
    assert_eq!(cache.status()?.available_bytes, 5_000_000_004);
    Ok(())
}
#[test]
fn disk_oversize_and_invalid_keys_preserve_existing_cache() -> TestResult {
    let temp = fixture();
    let mut cache = DiskCache::with_query(temp.path(), 4, SimulatedSpace::plenty())?;
    cache.put("old", b"abcd")?;
    assert!(cache.put("old", b"abcde").is_err());
    assert!(cache.put("../blobs", b"x").is_err());
    assert_eq!(cache.get("old")?, Some(b"abcd".to_vec()));
    assert_eq!(cache.status()?.used_bytes, 4);
    Ok(())
}
#[test]
fn disk_duplicate_and_replacement_do_not_leak_accounting_or_old_files() -> TestResult {
    let temp = fixture();
    let mut cache = DiskCache::with_query(temp.path(), 16, SimulatedSpace::plenty())?;
    cache.put("a", b"old")?;
    cache.put("a", b"old")?;
    assert_eq!(cache.status()?.used_bytes, 3);
    cache.put("a", b"newbytes")?;
    assert_eq!(cache.status()?.used_bytes, 8);
    let files = fs::read_dir(temp.path().join("cache/vw-store-v1"))?
        .map(|entry| entry.expect("cache entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "cache")
        })
        .count();
    assert_eq!(files, 1);
    assert!(cache.remove("a")?);
    assert!(!cache.remove("a")?);
    assert_eq!(cache.status()?.used_bytes, 0);
    Ok(())
}
#[test]
fn disk_corrupted_entry_is_not_read_or_deleted_as_owned_content() -> TestResult {
    let temp = fixture();
    let mut cache = DiskCache::with_query(temp.path(), 8, SimulatedSpace::plenty())?;
    cache.put("a", b"original")?;
    let path = fs::read_dir(temp.path().join("cache/vw-store-v1"))?
        .map(|entry| entry.expect("cache entry").path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "cache")
        })
        .expect("cached file");
    fs::write(&path, b"replaced")?;
    assert!(matches!(cache.get("a"), Err(StoreError::Corrupt(_))));
    assert!(matches!(cache.remove("a"), Err(StoreError::Corrupt(_))));
    assert!(matches!(cache.put("b", b"b"), Err(StoreError::Corrupt(_))));
    assert_eq!(fs::read(path)?, b"replaced");
    Ok(())
}
#[test]
fn disk_lock_excludes_other_managers_and_releases_on_drop() -> TestResult {
    let temp = fixture();
    let cache = DiskCache::with_query(temp.path(), 1, SimulatedSpace::plenty())?;
    assert!(matches!(
        DiskCache::with_query(temp.path(), 1, SimulatedSpace::plenty()),
        Err(StoreError::Busy)
    ));
    drop(cache);
    let _cache = DiskCache::with_query(temp.path(), 1, SimulatedSpace::plenty())?;
    Ok(())
}
#[test]
fn missing_cache_entry_after_interrupted_eviction_recovers_without_touching_unknowns() -> TestResult
{
    let temp = fixture();
    {
        let mut cache = DiskCache::with_query(temp.path(), 8, SimulatedSpace::plenty())?;
        cache.put("gone", b"data")?;
    }
    for entry in fs::read_dir(temp.path().join("cache/vw-store-v1"))? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "cache")
        {
            fs::remove_file(path)?;
        }
    }
    fs::write(
        temp.path().join("cache/vw-store-v1/.pending-interrupted"),
        b"foreign or orphan",
    )?;
    let cache = DiskCache::with_query(temp.path(), 8, SimulatedSpace::plenty())?;
    assert!(!cache.contains("gone"));
    assert_eq!(cache.status()?.used_bytes, 0);
    assert!(
        temp.path()
            .join("cache/vw-store-v1/.pending-interrupted")
            .exists()
    );
    Ok(())
}
#[test]
fn invalid_free_space_response_fails_closed() {
    let temp = fixture();
    let query = SimulatedSpace::new(10, 11);
    assert!(matches!(
        DiskCache::with_query(temp.path(), 8, query),
        Err(StoreError::Invalid(_))
    ));
}
#[cfg(unix)]
#[test]
fn cache_symlink_directory_cannot_redirect_eviction_or_writes() -> TestResult {
    use std::os::unix::fs::symlink;
    let temp = fixture();
    let outside = fixture();
    fs::write(outside.path().join("original"), b"preserve")?;
    symlink(outside.path(), temp.path().join("cache"))?;
    assert!(DiskCache::with_query(temp.path(), 8, SimulatedSpace::plenty()).is_err());
    assert_eq!(fs::read_dir(outside.path())?.count(), 1);
    assert_eq!(fs::read(outside.path().join("original"))?, b"preserve");
    Ok(())
}

#[test]
fn opening_under_reserve_allows_existing_reads_but_rejects_new_caching() -> TestResult {
    let temp = fixture();
    let query = SimulatedSpace::plenty();
    {
        let mut cache = DiskCache::with_query(temp.path(), 8, query.clone())?;
        cache.put("old", b"old")?;
    }
    query.set(1);
    let mut cache = DiskCache::with_query(temp.path(), 8, query)?;
    assert!(cache.status()?.low_space);
    assert_eq!(cache.get("old")?, Some(b"old".to_vec()));
    assert!(matches!(
        cache.put("new", b"new"),
        Err(StoreError::LowSpace)
    ));
    Ok(())
}

#[test]
fn malicious_manifest_path_is_rejected_before_outside_file_mutation() -> TestResult {
    let temp = fixture();
    let original = temp.path().join("precious-original");
    fs::write(&original, b"outside-original")?;
    {
        let mut cache = DiskCache::with_query(temp.path(), 8, SimulatedSpace::plenty())?;
        cache.put("old", b"data")?;
    }
    let manifest = temp.path().join("cache/vw-store-v1/index.json");
    let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&manifest)?)?;
    document["entries"]["old"]["filename"] = serde_json::json!("../../precious-original");
    fs::write(&manifest, serde_json::to_vec(&document)?)?;
    assert!(matches!(
        DiskCache::with_query(temp.path(), 0, SimulatedSpace::plenty()),
        Err(StoreError::Corrupt(_))
    ));
    assert_eq!(fs::read(original)?, b"outside-original");
    Ok(())
}

#[cfg(windows)]
#[test]
fn windows_junction_children_cannot_redirect_blobs_or_cache() -> TestResult {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    fn junction(link: &Path, target: &Path) -> TestResult {
        let output = Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .creation_flags(0x0800_0000)
            .output()?;
        assert!(output.status.success(), "fixture junction creation failed");
        Ok(())
    }
    let temp = fixture();
    let outside = fixture();
    fs::write(outside.path().join("original"), b"preserve")?;
    let cache_link = temp.path().join("cache");
    junction(&cache_link, outside.path())?;
    let cache_rejected = DiskCache::with_query(temp.path(), 8, SimulatedSpace::plenty()).is_err();
    fs::remove_dir(&cache_link)?;
    assert!(cache_rejected);
    let blobs = BlobStore::new(temp.path())?;
    let bytes = b"junction blob";
    let id = AssetId::hash(bytes);
    let shard = temp.path().join("blobs").join(&id.as_str()[..2]);
    junction(&shard, outside.path())?;
    let write_rejected = blobs.put(bytes).is_err();
    let read_path_rejected = blobs.path(&id).is_err();
    fs::remove_dir(&shard)?;
    assert!(write_rejected && read_path_rejected);
    assert_eq!(fs::read_dir(outside.path())?.count(), 1);
    assert_eq!(fs::read(outside.path().join("original"))?, b"preserve");
    Ok(())
}

#[test]
fn externally_grown_cache_file_is_rejected_before_unbounded_allocation() -> TestResult {
    let temp = fixture();
    let mut cache = DiskCache::with_query(temp.path(), 8, SimulatedSpace::plenty())?;
    cache.put("small", b"small")?;
    let path = fs::read_dir(temp.path().join("cache/vw-store-v1"))?
        .map(|entry| entry.expect("cache entry").path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "cache")
        })
        .expect("cached file");
    fs::OpenOptions::new()
        .write(true)
        .open(&path)?
        .set_len(8 * 1024 * 1024)?;
    assert!(matches!(
        cache.get("small"),
        Err(StoreError::Corrupt("cache size"))
    ));
    assert!(matches!(
        cache.remove("small"),
        Err(StoreError::Corrupt("cache size"))
    ));
    assert_eq!(fs::metadata(&path)?.len(), 8 * 1024 * 1024);
    Ok(())
}

#[test]
fn post_publication_reserve_decline_reclaims_cache_or_reports_low_space() -> TestResult {
    struct PublicationSpace {
        headroom: u64,
        overhead: u64,
        trigger_clock: Arc<AtomicU64>,
    }
    impl FreeSpaceQuery for PublicationSpace {
        fn query(&self, path: &Path) -> Result<DiskSpace, StoreError> {
            let index: serde_json::Value =
                serde_json::from_slice(&fs::read(path.join("index.json"))?)?;
            let clock = index["clock"]
                .as_u64()
                .ok_or(StoreError::Invalid("fixture clock"))?;
            let mut payload = 0u64;
            for item in fs::read_dir(path)? {
                let item = item?;
                if item
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "cache")
                {
                    payload += item.metadata()?.len();
                }
            }
            let trigger = self.trigger_clock.load(Ordering::SeqCst);
            let overhead = if trigger > 0 && clock >= trigger {
                self.overhead
            } else {
                0
            };
            Ok(DiskSpace {
                total_bytes: 100_000_000_000,
                available_bytes: 5_000_000_000 + self.headroom - payload - overhead,
            })
        }
    }
    // The first case can keep the requested data by reclaiming an older entry.
    // The rest must report failure even if evicting the new entry restores space.
    for (headroom, overhead, seed_bytes, duplicate, kept, low_space) in [
        (12, 4, 8, false, true, false),
        (4, 4, 0, false, false, false),
        (4, 8, 0, false, false, true),
        (4, 4, 4, true, false, false),
    ] {
        let temp = fixture();
        let originals = BlobStore::new(temp.path())?;
        let original = originals.put(b"original survives reserve correction")?;
        let trigger = Arc::new(AtomicU64::new(0));
        let query = PublicationSpace {
            headroom,
            overhead,
            trigger_clock: Arc::clone(&trigger),
        };
        let mut cache = DiskCache::with_query(temp.path(), 64, query)?;
        let foreign = temp.path().join("cache/unmanaged.bin");
        fs::write(&foreign, b"another cache owner")?;
        if seed_bytes > 0 {
            if duplicate {
                cache.put("target", b"data")?;
            } else {
                cache.put("older", &vec![7; seed_bytes])?;
            }
        }
        trigger.store(if seed_bytes > 0 { 2 } else { 1 }, Ordering::SeqCst);
        let result = cache.put("target", b"data");
        assert_eq!(result.is_ok(), kept, "post-publication outcome");
        if !kept {
            assert!(matches!(result, Err(StoreError::LowSpace)));
        }
        assert_eq!(cache.contains("target"), kept);
        assert!(!cache.contains("older"));
        let status = cache.status()?;
        assert_eq!(status.used_bytes, if kept { 4 } else { 0 });
        assert_eq!(status.low_space, low_space);
        assert_eq!(
            originals.read(&original)?,
            b"original survives reserve correction"
        );
        assert_eq!(fs::read(foreign)?, b"another cache owner");
    }
    Ok(())
}
