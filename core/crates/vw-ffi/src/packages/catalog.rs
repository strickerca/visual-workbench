use super::*;
use crate::worker::{Worker, startup};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicUsize},
};
const MARKER: &[u8] = b"VisualWorkbench/package-catalog/v1\n";
const INDEX_LIMIT: usize = 128 * 1024;
static CATALOGS: AtomicUsize = AtomicUsize::new(0);
struct Permit;
impl Permit {
    fn acquire() -> PackageResult<Self> {
        CATALOGS
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n == 0).then_some(1)
            })
            .map_err(|_| PackageError::Busy)?;
        Ok(Self)
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        CATALOGS.fetch_sub(1, Ordering::AcqRel);
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    package_id: String,
    target: String,
    manifest_sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    schema: u32,
    entries: Vec<Entry>,
    retiring: Option<Retiring>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Retiring {
    entry: Entry,
    files: Vec<RetiredFile>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetiredFile {
    name: String,
    bytes: u64,
    blake3: String,
}
impl Retiring {
    fn validate(&self) -> PackageResult<()> {
        key(&self.entry.package_id, &self.entry.target)?;
        hash(&self.entry.manifest_sha256)?;
        if self.files.is_empty() || self.files.len() > 69 {
            return Err(PackageError::Integrity);
        }
        let mut names = std::collections::BTreeSet::new();
        let mut bytes = 0u64;
        for f in &self.files {
            files::safe_name(&f.name)?;
            hash(&f.blake3)?;
            if !names.insert(f.name.as_str()) {
                return Err(PackageError::Integrity);
            }
            bytes = bytes.checked_add(f.bytes).ok_or(PackageError::Limit)?;
        }
        if bytes > MAX_PACKAGE as u64 || !names.contains("manifest.json") {
            return Err(PackageError::Integrity);
        }
        Ok(())
    }
}
fn key(id: &str, target: &str) -> PackageResult<String> {
    if id.len() != 36 || !matches!(target, "claude" | "openai" | "gemini" | "generic") {
        return Err(PackageError::Invalid);
    }
    vw_model::Id::try_from(id.to_owned())?;
    Ok(format!("{id}.{target}"))
}
fn hash(value: &str) -> PackageResult<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(PackageError::Invalid);
    }
    Ok(())
}
fn root_guard(root: &Path) -> PackageResult<()> {
    files::directory(root)?;
    if files::read(&root.join("owner-v1"), MARKER.len())? != MARKER {
        return Err(PackageError::Integrity);
    }
    Ok(())
}
struct State {
    root: PathBuf,
    entries: BTreeMap<String, Entry>,
    retiring: Option<Retiring>,
    poisoned: bool,
    _lock: files::RootLock,
    _permit: Permit,
}
impl State {
    fn guard(&self) -> PackageResult<()> {
        if self.poisoned {
            return Err(PackageError::Storage);
        }
        root_guard(&self.root)
    }
    fn open(parent: &str, cancel: &Cancellation, permit: Permit) -> PackageResult<Self> {
        cancel.check()?;
        let parent = files::directory(Path::new(parent))?;
        for p in parent.ancestors() {
            if p.join("project.sqlite").exists() || p.join("project.db").exists() {
                return Err(PackageError::Invalid);
            }
        }
        let root = parent.join("packages-v1");
        let created = match fs::create_dir(&root) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
                }
                files::write_new(&root.join("owner-v1"), MARKER)?;
                files::sync(&root)?;
                files::sync(&parent)?;
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(e) => return Err(e.into()),
        };
        root_guard(&root)?;
        let lock = files::lock(&root)?;
        let index = root.join("index-v1.json");
        if !index.exists() {
            if !created {
                return Err(PackageError::Integrity);
            }
            let bytes = serde_json::to_vec(&Index {
                schema: 1,
                entries: vec![],
                retiring: None,
            })
            .map_err(|_| PackageError::Integrity)?;
            files::write_new(&index, &bytes)?;
            files::sync(&root)?;
        }
        let stored: Index = serde_json::from_slice(&files::read(&index, INDEX_LIMIT)?)
            .map_err(|_| PackageError::Integrity)?;
        if stored.schema != 1 || stored.entries.len() > MAX_ENTRIES {
            return Err(PackageError::Integrity);
        }
        let mut state = Self {
            root,
            entries: BTreeMap::new(),
            retiring: stored.retiring,
            poisoned: false,
            _lock: lock,
            _permit: permit,
        };
        for entry in stored.entries {
            cancel.check()?;
            hash(&entry.manifest_sha256)?;
            let k = key(&entry.package_id, &entry.target)?;
            if state.entries.contains_key(&k) {
                return Err(PackageError::Integrity);
            }
            state.verify(&entry, cancel)?;
            state.entries.insert(k, entry);
        }
        if let Some(retiring) = &state.retiring {
            retiring.validate()?;
            if state
                .entries
                .contains_key(&key(&retiring.entry.package_id, &retiring.entry.target)?)
            {
                return Err(PackageError::Integrity);
            }
        }
        state.usage()?;
        cancel.check()?;
        Ok(state)
    }
    fn verify(&self, entry: &Entry, cancel: &Cancellation) -> PackageResult<PublishedPackage> {
        self.guard()?;
        let path = self.root.join(key(&entry.package_id, &entry.target)?);
        let package = files::package(&path, cancel)?;
        let info = dto::describe(&package)?;
        if info.package_id != entry.package_id
            || info.target != entry.target
            || info.manifest_sha256 != entry.manifest_sha256
        {
            return Err(PackageError::Integrity);
        }
        Ok(PublishedPackage {
            info,
            directory: path.to_str().ok_or(PackageError::Invalid)?.into(),
        })
    }
    fn persist(
        &mut self,
        entries: &BTreeMap<String, Entry>,
        retiring: Option<Retiring>,
    ) -> PackageResult<()> {
        self.guard()?;
        if entries.len() > MAX_ENTRIES {
            return Err(PackageError::Limit);
        }
        let bytes = serde_json::to_vec(&Index {
            schema: 1,
            entries: entries.values().cloned().collect(),
            retiring,
        })
        .map_err(|_| PackageError::Integrity)?;
        if bytes.len() > INDEX_LIMIT {
            return Err(PackageError::Limit);
        }
        files::regular(
            &fs::symlink_metadata(self.root.join("index-v1.json"))?,
            false,
        )?;
        let mut staged = tempfile::Builder::new()
            .prefix("index-")
            .tempfile_in(&self.root)?;
        staged.write_all(&bytes)?;
        staged.as_file().sync_all()?;
        // From replacement onward an error has an ambiguous durable outcome.
        // Never allow another operation to overwrite it from the old cache.
        self.poisoned = true;
        staged
            .persist(self.root.join("index-v1.json"))
            .map_err(|_| PackageError::Storage)?;
        #[cfg(test)]
        if FAIL_AFTER_REPLACE.swap(false, Ordering::AcqRel) {
            return Err(PackageError::Storage);
        }
        files::sync(&self.root)?;
        self.poisoned = false;
        Ok(())
    }
    /// Includes unindexed/crash-retained bytes. Nothing may bypass quota by
    /// failing between immutable directory publication and index publication.
    fn usage(&self) -> PackageResult<u64> {
        self.guard()?;
        let mut total = 0u64;
        let mut nodes = 0usize;
        let mut pending = vec![(self.root.clone(), 0u8)];
        while let Some((path, depth)) = pending.pop() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                nodes += 1;
                if nodes > 10_000 {
                    return Err(PackageError::Limit);
                }
                let meta = fs::symlink_metadata(entry.path())?;
                if meta.is_dir() {
                    files::regular(&meta, true)?;
                    if depth >= 2 {
                        return Err(PackageError::Integrity);
                    }
                    pending.push((entry.path(), depth + 1));
                } else {
                    files::regular(&meta, false)?;
                    total = total.checked_add(meta.len()).ok_or(PackageError::Limit)?;
                    if total > MAX_DISK {
                        return Err(PackageError::Limit);
                    }
                }
            }
        }
        Ok(total)
    }
    fn publish(
        &mut self,
        prepared: &compile::Prepared,
        cancel: &Cancellation,
    ) -> PackageResult<PublishedPackage> {
        self.guard()?;
        cancel.check()?;
        let info = dto::describe(&prepared.package)?;
        let k = key(&info.package_id, &info.target)?;
        if self
            .retiring
            .as_ref()
            .is_some_and(|r| r.entry.package_id == info.package_id && r.entry.target == info.target)
        {
            return Err(PackageError::Busy);
        }
        if let Some(old) = self.entries.get(&k) {
            if old.manifest_sha256 != info.manifest_sha256 {
                return Err(PackageError::Identity);
            }
            let path = self.root.join(&k);
            self.guard()?;
            files::matches(&path, &prepared.package, cancel)?;
            return Ok(PublishedPackage {
                info,
                directory: path.to_str().ok_or(PackageError::Invalid)?.into(),
            });
        }
        if self.entries.len() >= MAX_ENTRIES {
            return Err(PackageError::Limit);
        }
        let entry = Entry {
            package_id: info.package_id.clone(),
            target: info.target.clone(),
            manifest_sha256: info.manifest_sha256.clone(),
        };
        let path = self.root.join(&k);
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                self.guard()?;
                files::matches(&path, &prepared.package, cancel)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let used = self.usage()?;
                if used
                    .checked_add(info.total_bytes)
                    .and_then(|n| n.checked_add(INDEX_LIMIT as u64))
                    .is_none_or(|n| n > MAX_DISK)
                {
                    return Err(PackageError::Limit);
                }
                let staging = tempfile::Builder::new()
                    .prefix("stage-")
                    .tempdir_in(&self.root)?;
                fs::create_dir(staging.path().join("images"))?;
                for (name, bytes) in prepared.package.files() {
                    cancel.check()?;
                    files::safe_name(name)?;
                    files::write_new(&staging.path().join(name), bytes)?;
                }
                files::sync(&staging.path().join("images"))?;
                files::sync(staging.path())?;
                // Verify on disk before publication; no foreign/mutated package
                // can acquire this compiler-produced identity merely by name.
                files::matches(staging.path(), &prepared.package, cancel)?;
                cancel.check()?;
                crate::creation::publish(staging.path(), &path)?;
                files::sync(&self.root)?;
            }
            Err(e) => return Err(e.into()),
        }
        // Once complete bytes are published, cancellation cannot delete them.
        // An index failure preserves those exact bytes for the same-hash retry.
        let mut next = self.entries.clone();
        next.insert(k, entry.clone());
        self.persist(&next, self.retiring.clone())?;
        self.entries = next;
        Ok(PublishedPackage {
            info,
            directory: path.to_str().ok_or(PackageError::Invalid)?.into(),
        })
    }
    fn retire(
        &mut self,
        id: String,
        target: String,
        expected: String,
        cancel: &Cancellation,
    ) -> PackageResult<PackageRetirement> {
        self.guard()?;
        hash(&expected)?;
        let k = key(&id, &target)?;
        if let Some(retiring) = &self.retiring {
            if retiring.entry.package_id != id || retiring.entry.target != target {
                return Err(PackageError::Busy);
            }
            if retiring.entry.manifest_sha256 != expected {
                return Err(PackageError::Identity);
            }
        } else {
            let entry = self.entries.get(&k).cloned().ok_or(PackageError::Stale)?;
            if entry.manifest_sha256 != expected {
                return Err(PackageError::Identity);
            }
            let p = files::package(&self.root.join(&k), cancel)?;
            if p.manifest_sha256() != expected {
                return Err(PackageError::Integrity);
            }
            let retirement = Retiring {
                entry,
                files: p
                    .files()
                    .map(|(name, bytes)| RetiredFile {
                        name: name.into(),
                        bytes: bytes.len() as u64,
                        blake3: blake3::hash(bytes).to_hex().to_string(),
                    })
                    .collect(),
            };
            retirement.validate()?;
            cancel.check()?;
            let mut next = self.entries.clone();
            next.remove(&k);
            // The exact ownership inventory and removal from the served index
            // commit together. A crash midway through cleanup remains retryable.
            self.persist(&next, Some(retirement.clone()))?;
            self.entries = next;
            self.retiring = Some(retirement);
        }
        cancel.check()?;
        let retirement = self.retiring.as_ref().ok_or(PackageError::Integrity)?;
        let cleaned = self.cleanup(retirement).is_ok();
        if cleaned {
            self.persist(&self.entries.clone(), None)?;
            self.retiring = None;
        }
        Ok(PackageRetirement {
            package_id: id,
            target,
            manifest_sha256: expected,
            files_removed: cleaned,
        })
    }
    fn cleanup(&self, retiring: &Retiring) -> PackageResult<()> {
        retiring.validate()?;
        self.guard()?;
        let path = self
            .root
            .join(key(&retiring.entry.package_id, &retiring.entry.target)?);
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
            Ok(m) => files::regular(&m, true)?,
        }
        files::directory(&path)?;
        let expected = retiring
            .files
            .iter()
            .map(|f| (f.name.as_str(), f))
            .collect::<BTreeMap<_, _>>();
        let mut present = Vec::new();
        let mut images = false;
        for item in fs::read_dir(&path)? {
            let item = item?;
            let name = item
                .file_name()
                .into_string()
                .map_err(|_| PackageError::Integrity)?;
            if name == "images" {
                files::regular(&fs::symlink_metadata(item.path())?, true)?;
                images = true;
                for image in fs::read_dir(item.path())? {
                    let image = image?;
                    let leaf = image
                        .file_name()
                        .into_string()
                        .map_err(|_| PackageError::Integrity)?;
                    let name = format!("images/{leaf}");
                    if !expected.contains_key(name.as_str()) {
                        return Err(PackageError::Integrity);
                    }
                    present.push(name);
                    if present.len() > 69 {
                        return Err(PackageError::Integrity);
                    }
                }
            } else {
                if !expected.contains_key(name.as_str()) {
                    return Err(PackageError::Integrity);
                }
                present.push(name);
            }
            if present.len() > 69 {
                return Err(PackageError::Integrity);
            }
        }
        let inventory = present
            .iter()
            .map(|name| {
                let f = expected.get(name.as_str()).ok_or(PackageError::Integrity)?;
                Ok((name.as_str(), f.bytes, f.blake3.as_str()))
            })
            .collect::<PackageResult<Vec<_>>>()?;
        // Windows keeps deny-write/delete file handles and deny-rename directory
        // handles from verification through delete-by-handle. Other platforms
        // retain bytes: path-based unlink cannot establish the same guarantee.
        super::retirement::remove(&self.root, &path, images, &inventory, || {
            #[cfg(test)]
            if let Ok(mut hook) = RETIRE_HOOK.lock()
                && let Some(hook) = hook.take()
            {
                hook(&path);
            }
        })?;
        files::sync(&self.root)?;
        Ok(())
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct PackageRetirement {
    pub package_id: String,
    pub target: String,
    pub manifest_sha256: String,
    pub files_removed: bool,
}
#[derive(uniffi::Object)]
pub struct PackageCatalog {
    worker: Worker<State>,
    closed: Arc<AtomicBool>,
}
#[uniffi::export]
pub async fn open_package_catalog(
    application_private_directory: String,
    cancellation: Arc<Cancellation>,
) -> PackageResult<Arc<PackageCatalog>> {
    let permit = Permit::acquire()?;
    startup(move || {
        Ok((|| {
            let state = State::open(&application_private_directory, &cancellation, permit)?;
            Ok(Arc::new(PackageCatalog {
                worker: Worker::new("vw-package-catalog", state, 4)?,
                closed: Arc::new(AtomicBool::new(false)),
            }))
        })())
    })
    .await?
}
#[uniffi::export]
impl PackageCatalog {
    pub async fn publish(
        &self,
        package: Arc<CompiledPackage>,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<PublishedPackage> {
        let p = package.get()?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check(&closed, &cancellation)?;
                    state.guard()?;
                    state.publish(&p, &cancellation)
                })())
            })
            .await?
    }
    pub async fn list(
        &self,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<Vec<PublishedPackage>> {
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check(&closed, &cancellation)?;
                    state.guard()?;
                    state
                        .entries
                        .values()
                        .map(|entry| state.verify(entry, &cancellation))
                        .collect()
                })())
            })
            .await?
    }
    pub async fn pending_retirement(
        &self,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<Option<PackageRetirement>> {
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check(&closed, &cancellation)?;
                    state.guard()?;
                    Ok(state.retiring.as_ref().map(|r| PackageRetirement {
                        package_id: r.entry.package_id.clone(),
                        target: r.entry.target.clone(),
                        manifest_sha256: r.entry.manifest_sha256.clone(),
                        files_removed: false,
                    }))
                })())
            })
            .await?
    }
    pub async fn lookup(
        &self,
        package_id: String,
        target: String,
        manifest_sha256: String,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<PublishedPackage> {
        let k = key(&package_id, &target)?;
        hash(&manifest_sha256)?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check(&closed, &cancellation)?;
                    state.guard()?;
                    let e = state.entries.get(&k).ok_or(PackageError::Stale)?;
                    if e.manifest_sha256 != manifest_sha256 {
                        return Err(PackageError::Identity);
                    }
                    state.verify(e, &cancellation)
                })())
            })
            .await?
    }
    pub async fn read_file(
        &self,
        package_id: String,
        target: String,
        manifest_sha256: String,
        name: String,
        max_bytes: u32,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<Vec<u8>> {
        let k = key(&package_id, &target)?;
        hash(&manifest_sha256)?;
        files::safe_name(&name)?;
        if max_bytes == 0 || max_bytes > 4 * 1024 * 1024 {
            return Err(PackageError::Limit);
        }
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check(&closed, &cancellation)?;
                    state.guard()?;
                    let e = state.entries.get(&k).ok_or(PackageError::Stale)?;
                    if e.manifest_sha256 != manifest_sha256 {
                        return Err(PackageError::Identity);
                    }
                    let p = files::package(&state.root.join(&k), &cancellation)?;
                    if p.manifest_sha256() != manifest_sha256 {
                        return Err(PackageError::Integrity);
                    }
                    let bytes = p.file(&name).ok_or(PackageError::Invalid)?;
                    if bytes.len() > max_bytes as usize {
                        return Err(PackageError::Limit);
                    }
                    cancellation.check()?;
                    Ok(bytes.to_vec())
                })())
            })
            .await?
    }
    /// Caller must first await MCP unpublish/drain and release owned UI/drag
    /// leases. This is a deliberate owner action, never catalog age-based expiry.
    pub async fn retire(
        &self,
        package_id: String,
        target: String,
        manifest_sha256: String,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<PackageRetirement> {
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check(&closed, &cancellation)?;
                    state.guard()?;
                    state.retire(package_id, target, manifest_sha256, &cancellation)
                })())
            })
            .await?
    }
    pub async fn shutdown(&self) -> PackageResult<()> {
        self.closed.store(true, Ordering::Release);
        self.worker.shutdown(|_| Ok(())).await?;
        Ok(())
    }
}
impl Drop for PackageCatalog {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
    }
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;

#[cfg(test)]
static FAIL_AFTER_REPLACE: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
type RetireHook = Box<dyn FnOnce(&Path) + Send>;
#[cfg(test)]
static RETIRE_HOOK: std::sync::Mutex<Option<RetireHook>> = std::sync::Mutex::new(None);
