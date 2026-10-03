//! Explicit, leased, revision-bound drag files. Cleanup never recurses and only
//! considers our exact versioned layout under the operating system temp root.
use crate::{
    DragCleanupReceipt, DragFile, DragFileReceipt, ExportBinding, HostError, HostResult, binding,
    worker::RequestContext,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufReader, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) const MAX_DRAG_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const MAX_ACTIVE: usize = 16;
const MAX_RETAINED: u64 = 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 256;
const RETAIN_MS: u64 = 24 * 60 * 60 * 1000;
const ROOT_LOCK: &str = "admission.lock";
static NEXT: AtomicU64 = AtomicU64::new(1);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Owner {
    schema: u32,
    lease_id: String,
    expires_at_unix_ms: u64,
    binding: ExportBinding,
}
struct Lease {
    _lock: File,
    _image: File,
    _guards: Directories,
    released: Arc<AtomicBool>,
}
pub(crate) struct DragStore {
    root: PathBuf,
    _guards: Directories,
    active: BTreeMap<String, Lease>,
    max_retained: u64,
    max_entries: usize,
    #[cfg(test)]
    after_admission: Option<Box<dyn FnOnce() -> HostResult<()> + Send>>,
}
impl Drop for DragStore {
    fn drop(&mut self) {
        for lease in self.active.values() {
            lease.released.store(true, Ordering::Release);
        }
    }
}

impl DragStore {
    pub(crate) fn new(base: &Path) -> HostResult<Self> {
        let mut guards = Directories::open(base)?;
        let parent = child_directory(base, "VisualWorkbench")?;
        guards.hold(&parent)?;
        let root = child_directory(&parent, "drag-v1")?;
        guards.hold(&root)?;
        Ok(Self {
            root,
            _guards: guards,
            active: BTreeMap::new(),
            max_retained: MAX_RETAINED,
            max_entries: MAX_ENTRIES,
            #[cfg(test)]
            after_admission: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_quota(base: &Path, retained: u64, entries: usize) -> HostResult<Self> {
        let mut store = Self::new(base)?;
        store.max_retained = retained.min(MAX_RETAINED);
        store.max_entries = entries.min(MAX_ENTRIES);
        Ok(store)
    }

    #[cfg(test)]
    pub(crate) fn after_admission(
        &mut self,
        hook: impl FnOnce() -> HostResult<()> + Send + 'static,
    ) {
        self.after_admission = Some(Box::new(hook));
    }

    pub(crate) fn stage(
        &mut self,
        source: &Path,
        binding: ExportBinding,
        context: &RequestContext,
    ) -> HostResult<Arc<DragFile>> {
        context.checkpoint()?;
        self.reap();
        binding::validate(&binding)?;
        if self.active.len() >= MAX_ACTIVE {
            return Err(HostError::Busy);
        }
        // File locks cover every HostService/process sharing this root. Hold
        // the reservation through publication or rollback, including metadata.
        let _admission = RootLock::acquire(&self.root, Some(context))?;
        let now = unix_ms()?;
        self.cleanup_locked(now)?;
        let paths = self.entries()?;
        let mut retained = 0u64;
        for path in &paths {
            // Unknown entries are preserved. Their direct file size still
            // contributes to admission; directories are inspected nonrecursively.
            let metadata = fs::symlink_metadata(path).map_err(storage)?;
            if metadata.is_file() {
                retained = retained
                    .checked_add(metadata.len())
                    .ok_or(HostError::SizeLimit)?;
            } else if plain_directory(&metadata) {
                for entry in fs::read_dir(path).map_err(storage)?.take(5) {
                    let entry = entry.map_err(storage)?;
                    let metadata = fs::symlink_metadata(entry.path()).map_err(storage)?;
                    if metadata.is_file() {
                        retained = retained
                            .checked_add(metadata.len())
                            .ok_or(HostError::SizeLimit)?;
                    }
                }
            }
        }
        if paths.len() >= self.max_entries {
            return Err(HostError::Busy);
        }
        let (mut input, _source_guards) = open_source(source)?;
        let length = input.metadata().map_err(storage)?.len();
        let id = fresh_id(now);
        let expires = now
            .checked_add(RETAIN_MS)
            .ok_or(HostError::HandoffStorage)?;
        let owner = Owner {
            schema: 1,
            lease_id: id.clone(),
            expires_at_unix_ms: expires,
            binding: binding.clone(),
        };
        let encoded = serde_json::to_vec(&owner).map_err(storage)?;
        if length == 0
            || length > MAX_DRAG_BYTES
            || retained
                .checked_add(length)
                .and_then(|n| n.checked_add(encoded.len() as u64))
                .is_none_or(|n| n > self.max_retained)
        {
            return Err(HostError::SizeLimit);
        }
        #[cfg(test)]
        if let Some(hook) = self.after_admission.take() {
            hook()?;
        }
        context.checkpoint()?;
        let directory = self.root.join(format!("drag-{id}"));
        fs::create_dir(&directory).map_err(storage)?;
        let guards = Directories::open(&directory)?;
        let result: HostResult<(DragFileReceipt, File, File)> = (|| {
            let lock = create_file(&directory.join("lease.lock"))?;
            FileExt::try_lock_exclusive(&lock).map_err(storage)?;
            let mut marker = create_file(&directory.join("owner.json"))?;
            marker.write_all(&encoded).map_err(storage)?;
            marker.sync_all().map_err(storage)?;
            drop(marker);
            let target = directory.join("image.png");
            let mut output = create_file(&target)?;
            let mut hash = blake3::Hasher::new();
            let mut copied = 0u64;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                context.checkpoint()?;
                let n = input.read(&mut buffer).map_err(storage)?;
                if n == 0 {
                    break;
                }
                copied = copied
                    .checked_add(n as u64)
                    .filter(|n| *n <= length)
                    .ok_or(HostError::ExportBindingMismatch)?;
                hash.update(&buffer[..n]);
                output.write_all(&buffer[..n]).map_err(storage)?;
            }
            if copied != length || hash.finalize().to_hex().as_str() != binding.png_blake3 {
                return Err(HostError::ExportBindingMismatch);
            }
            output.sync_all().map_err(storage)?;
            output.seek(SeekFrom::Start(0)).map_err(storage)?;
            let declaration = binding::inspect(&mut output, &binding, length)?;
            if u64::from(declaration.width) * u64::from(declaration.height) > 50_000_000 {
                return Err(HostError::SizeLimit);
            }
            output.seek(SeekFrom::Start(0)).map_err(storage)?;
            validate_rows(&mut output, &declaration, context)?;
            drop(output);
            let (image, _) = open_source(&target)?;
            let receipt = DragFileReceipt {
                lease_id: id.clone(),
                path: target.to_str().ok_or(HostError::HandoffStorage)?.to_owned(),
                png_bytes: length,
                width: declaration.width,
                height: declaration.height,
                bit_depth: declaration.bit_depth,
                expires_at_unix_ms: expires,
                binding,
            };
            context.begin_publication()?;
            Ok((receipt, lock, image))
        })();
        match result {
            Ok((receipt, lock, image)) => {
                let released = Arc::new(AtomicBool::new(false));
                self.active.insert(
                    id,
                    Lease {
                        _lock: lock,
                        _image: image,
                        _guards: guards,
                        released: released.clone(),
                    },
                );
                Ok(Arc::new(DragFile { receipt, released }))
            }
            Err(error) => {
                let removed = remove_owned(&directory, true);
                drop(guards);
                if matches!(removed, Ok(true)) {
                    let _ = fs::remove_dir(&directory);
                }
                Err(error)
            }
        }
    }

    /// Releases our lease only. A receiving app may open a dropped path later;
    /// the immutable file remains until its 24-hour retention deadline.
    pub(crate) fn release(&mut self, id: &str) -> HostResult<()> {
        if !valid_id(id) {
            return Err(HostError::UnknownDragLease);
        }
        self.active
            .remove(id)
            .map(|lease| lease.released.store(true, Ordering::Release))
            .ok_or(HostError::UnknownDragLease)
    }

    pub(crate) fn reap(&mut self) {
        self.active
            .retain(|_, lease| !lease.released.load(Ordering::Acquire));
    }

    pub(crate) fn cleanup(&mut self, now: u64) -> HostResult<DragCleanupReceipt> {
        let _admission = RootLock::acquire(&self.root, None)?;
        self.cleanup_locked(now)
    }

    fn cleanup_locked(&mut self, now: u64) -> HostResult<DragCleanupReceipt> {
        self.reap();
        let mut report = DragCleanupReceipt {
            removed: 0,
            retained: 0,
        };
        for directory in self.entries()? {
            let Some(id) = directory
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_prefix("drag-"))
            else {
                report.retained += 1;
                continue;
            };
            if !valid_id(id) || self.active.contains_key(id) {
                report.retained += 1;
                continue;
            }
            let attempt: HostResult<bool> = (|| {
                let guards = Directories::open(&directory)?;
                let lock = open_cleanup_lock(&directory.join("lease.lock"))?;
                if FileExt::try_lock_exclusive(&lock).is_err() {
                    return Ok(false);
                }
                let mut marker = open_regular(&directory.join("owner.json"), false)?;
                if marker.metadata().map_err(storage)?.len() > 8192 {
                    return Ok(false);
                }
                let mut data = Vec::new();
                Read::by_ref(&mut marker)
                    .take(8193)
                    .read_to_end(&mut data)
                    .map_err(storage)?;
                let owner: Owner = serde_json::from_slice(&data).map_err(storage)?;
                if owner.schema != 1
                    || owner.lease_id != id
                    || owner.expires_at_unix_ms > now
                    || binding::validate(&owner.binding).is_err()
                {
                    return Ok(false);
                }
                drop(marker);
                let removed = remove_owned(&directory, true)?;
                drop(lock);
                drop(guards);
                if removed {
                    fs::remove_dir(&directory).map_err(storage)?;
                }
                Ok(removed)
            })();
            if matches!(attempt, Ok(true)) {
                report.removed += 1;
            } else {
                report.retained += 1;
            }
        }
        Ok(report)
    }

    fn entries(&self) -> HostResult<Vec<PathBuf>> {
        let mut entries = Vec::new();
        for item in fs::read_dir(&self.root).map_err(storage)? {
            let item = item.map_err(storage)?;
            if item.file_name() == ROOT_LOCK {
                continue;
            }
            if entries.len() >= self.max_entries {
                return Err(HostError::Busy);
            }
            entries.push(item.path());
        }
        Ok(entries)
    }
}

struct RootLock(File);
impl RootLock {
    fn acquire(root: &Path, context: Option<&RequestContext>) -> HostResult<Self> {
        let path = root.join(ROOT_LOCK);
        let file = match file_options(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                open_regular(&path, true)?
            }
            Err(error) => return Err(storage(error)),
        };
        // The coordination inode is persistent, empty and never cleaned up.
        // Refuse an unexpected existing payload without modifying it.
        if file.metadata().map_err(storage)?.len() != 0 {
            return Err(HostError::HandoffStorage);
        }
        let cleanup_deadline = Instant::now() + Duration::from_millis(500);
        loop {
            if let Some(context) = context {
                context.checkpoint()?;
            } else if Instant::now() >= cleanup_deadline {
                return Err(HostError::Busy);
            }
            match FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(Self(file)),
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {}
                Err(error) => return Err(storage(error)),
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for RootLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

fn validate_rows(
    file: &mut File,
    declaration: &binding::Declaration,
    context: &RequestContext,
) -> HostResult<()> {
    let mut options = png::DecodeOptions::default();
    options.set_ignore_text_chunk(true);
    options.set_ignore_adler32(false);
    options.set_skip_ancillary_crc_failures(false);
    let mut decoder = png::Decoder::new_with_options(BufReader::new(file), options);
    decoder.set_limits(png::Limits {
        bytes: 16 * 1024 * 1024,
    });
    let mut reader = decoder.read_info().map_err(|_| HostError::InvalidPng)?;
    if reader.info().width != declaration.width
        || reader.info().height != declaration.height
        || reader.info().animation_control.is_some()
    {
        return Err(HostError::InvalidPng);
    }
    if declaration.icc && reader.info().icc_profile.is_none() {
        return Err(HostError::InvalidPng);
    }
    if let Some(profile) = reader.info().icc_profile.as_deref() {
        crate::dib::validate_profile(profile)?;
    }
    while reader
        .next_interlaced_row()
        .map_err(|_| HostError::InvalidPng)?
        .is_some()
    {
        context.checkpoint()?;
    }
    reader.finish().map_err(|_| HostError::InvalidPng)?;
    context.checkpoint()
}

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn fresh_id(now: u64) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(&now.to_le_bytes());
    hash.update(&std::process::id().to_le_bytes());
    hash.update(&NEXT.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    hash.finalize().to_hex().as_str()[..32].to_owned()
}
pub(crate) fn unix_ms() -> HostResult<u64> {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(storage)?
            .as_millis(),
    )
    .map_err(storage)
}
fn storage(_: impl std::fmt::Debug) -> HostError {
    HostError::HandoffStorage
}
fn child_directory(parent: &Path, name: &str) -> HostResult<PathBuf> {
    let path = parent.join(name);
    match fs::create_dir(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(storage(error)),
    };
    if !plain_directory(&fs::symlink_metadata(&path).map_err(storage)?) {
        return Err(HostError::HandoffStorage);
    }
    Ok(path)
}
fn reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
fn plain_directory(metadata: &fs::Metadata) -> bool {
    metadata.is_dir() && !reparse(metadata) && !metadata.file_type().is_symlink()
}

/// Directory handles on Windows deny rename/delete while files are addressed
/// beneath them. Every existing ancestor is checked for junctions/symlinks.
struct Directories {
    _files: Vec<File>,
}
impl Directories {
    fn open(path: &Path) -> HostResult<Self> {
        if !path.is_absolute() {
            return Err(HostError::HandoffStorage);
        }
        let mut result = Self { _files: Vec::new() };
        let mut current = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Prefix(_) | Component::RootDir => current.push(component.as_os_str()),
                Component::Normal(_) => {
                    current.push(component.as_os_str());
                    result.hold(&current)?;
                }
                _ => return Err(HostError::HandoffStorage),
            }
        }
        Ok(result)
    }
    fn hold(&mut self, path: &Path) -> HostResult<()> {
        if !plain_directory(&fs::symlink_metadata(path).map_err(storage)?) {
            return Err(HostError::HandoffStorage);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
                FILE_SHARE_WRITE,
            };
            let file = OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(path)
                .map_err(storage)?;
            if !plain_directory(&file.metadata().map_err(storage)?) {
                return Err(HostError::HandoffStorage);
            }
            self._files.push(file);
        }
        Ok(())
    }
}
fn file_options(write: bool) -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(write);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        options
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options
}
fn create_file(path: &Path) -> HostResult<File> {
    file_options(true)
        .create_new(true)
        .open(path)
        .map_err(storage)
}
fn open_regular(path: &Path, write: bool) -> HostResult<File> {
    let before = fs::symlink_metadata(path).map_err(storage)?;
    if !before.is_file() || reparse(&before) {
        return Err(HostError::HandoffStorage);
    }
    let file = file_options(write).open(path).map_err(storage)?;
    let actual = file.metadata().map_err(storage)?;
    if !actual.is_file() || reparse(&actual) {
        return Err(HostError::HandoffStorage);
    }
    Ok(file)
}
fn open_cleanup_lock(path: &Path) -> HostResult<File> {
    let before = fs::symlink_metadata(path).map_err(storage)?;
    if !before.is_file() || reparse(&before) {
        return Err(HostError::HandoffStorage);
    }
    let options = file_options(true);
    #[cfg(windows)]
    let options = {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        let mut options = options;
        options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE);
        options
    };
    let file = options.open(path).map_err(storage)?;
    let metadata = file.metadata().map_err(storage)?;
    if !metadata.is_file() || reparse(&metadata) {
        return Err(HostError::HandoffStorage);
    }
    Ok(file)
}
fn open_source(path: &Path) -> HostResult<(File, Directories)> {
    let parent = path.parent().ok_or(HostError::HandoffStorage)?;
    let guards = Directories::open(parent)?;
    // Deny concurrent write/delete while verifying the immutable source bytes.
    let options = file_options(false);
    #[cfg(windows)]
    let options = {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
        let mut options = options;
        options.share_mode(FILE_SHARE_READ);
        options
    };
    let before = fs::symlink_metadata(path).map_err(storage)?;
    if !before.is_file() || reparse(&before) {
        return Err(HostError::HandoffStorage);
    }
    let file = options.open(path).map_err(storage)?;
    let actual = file.metadata().map_err(storage)?;
    if !actual.is_file() || reparse(&actual) {
        return Err(HostError::HandoffStorage);
    }
    Ok((file, guards))
}
fn remove_owned(directory: &Path, keep_directory: bool) -> HostResult<bool> {
    if !plain_directory(&fs::symlink_metadata(directory).map_err(storage)?) {
        return Ok(false);
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory).map_err(storage)? {
        if paths.len() >= 3 {
            return Ok(false);
        }
        let entry = entry.map_err(storage)?;
        let name = entry.file_name();
        if !matches!(
            name.to_str(),
            Some("image.png" | "owner.json" | "lease.lock")
        ) {
            return Ok(false);
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(storage)?;
        if !metadata.is_file() || reparse(&metadata) {
            return Ok(false);
        }
        paths.push(entry.path());
    }
    for path in paths {
        fs::remove_file(path).map_err(storage)?;
    }
    if !keep_directory {
        fs::remove_dir(directory).map_err(storage)?;
    }
    Ok(true)
}
