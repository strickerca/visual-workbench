use super::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
#[cfg(test)]
pub(super) static FAIL_AFTER_PUBLISH: AtomicBool = AtomicBool::new(false);
const MARKER: &[u8] = b"VisualWorkbench/mcp-result-inbox/v1\n";
pub(super) struct State {
    root: PathBuf,
    _lock: files::RootLock,
    _permit: Permit,
}
impl State {
    pub(super) fn open(parent: &str, cancel: &Cancellation, permit: Permit) -> PackageResult<Self> {
        cancel.check()?;
        let parent = files::directory(Path::new(parent))?;
        for path in parent.ancestors() {
            if path.join("project.sqlite").exists() || path.join("project.db").exists() {
                return Err(PackageError::Invalid);
            }
        }
        let root = parent.join("mcp-inbox-v1");
        match fs::create_dir(&root) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
                }
                files::write_new(&root.join("owner-v1"), MARKER)?;
                files::sync(&root)?;
                files::sync(&parent)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        let lock = files::lock(&root)?;
        let state = Self {
            root,
            _lock: lock,
            _permit: permit,
        };
        state.guard()?;
        state.usage()?;
        state.list(cancel)?;
        Ok(state)
    }
    fn guard(&self) -> PackageResult<()> {
        files::directory(&self.root)?;
        if files::read(&self.root.join("owner-v1"), MARKER.len())? != MARKER {
            return Err(PackageError::Integrity);
        }
        Ok(())
    }
    fn usage(&self) -> PackageResult<(u64, usize)> {
        self.guard()?;
        let mut bytes = 0u64;
        let mut nodes = 0usize;
        let mut logical = BTreeSet::new();
        let mut scratch = 0usize;
        let mut stack = vec![(self.root.clone(), 0u8)];
        while let Some((path, depth)) = stack.pop() {
            for item in fs::read_dir(path)? {
                let item = item?;
                nodes += 1;
                if nodes > 1024 {
                    return Err(PackageError::Limit);
                }
                let meta = fs::symlink_metadata(item.path())?;
                if meta.is_dir() {
                    files::regular(&meta, true)?;
                    if depth >= 2 {
                        return Err(PackageError::Integrity);
                    }
                    if depth == 0 {
                        let name = item
                            .file_name()
                            .into_string()
                            .map_err(|_| PackageError::Integrity)?;
                        if name.starts_with("incoming-") || name.starts_with("retiring-write-") {
                            scratch = scratch.checked_add(1).ok_or(PackageError::Limit)?;
                        } else {
                            let key = name.strip_prefix("retiring-").unwrap_or(&name);
                            id(key)?;
                            logical.insert(key.to_owned());
                        }
                    }
                    stack.push((item.path(), depth + 1));
                } else {
                    files::regular(&meta, false)?;
                    bytes = bytes.checked_add(meta.len()).ok_or(PackageError::Limit)?;
                    if bytes > DISK_LIMIT {
                        return Err(PackageError::Limit);
                    }
                }
            }
        }
        // A live directory and its retirement inventory are one logical item.
        // Every crash-staging directory consumes a separate admission slot.
        // Do not make reads/recovery fail merely because a full inbox has one
        // interrupted retirement write: only new publication requires headroom.
        // list() still independently refuses more than64 logical receipts.
        let entries = logical
            .len()
            .checked_add(scratch)
            .ok_or(PackageError::Limit)?;
        Ok((bytes, entries))
    }
    fn record(&self, path: &Path, expected: &str) -> PackageResult<(Record, Vec<u8>)> {
        files::directory(path)?;
        let bytes = files::read(&path.join("manifest.json"), RECORD_LIMIT)?;
        let record: Record = serde_json::from_slice(&bytes).map_err(|_| PackageError::Integrity)?;
        record.validate()?;
        if record.id != expected
            || serde_json::to_vec(&record).map_err(|_| PackageError::Integrity)? != bytes
        {
            return Err(PackageError::Integrity);
        }
        Ok((record, bytes))
    }
    fn data(&self, path: &Path, info: &McpInboxImage, name: &str) -> PackageResult<Vec<u8>> {
        let bytes = files::read_exact(&path.join(name), info.encoded_bytes, IMAGE_LIMIT)?;
        if blake3::hash(&bytes).to_hex().as_str() != info.blake3 {
            return Err(PackageError::Integrity);
        }
        Ok(bytes)
    }
    fn inventory(record: &Record, bytes: &[u8]) -> Vec<(String, u64, String)> {
        let mut out = vec![
            (
                "manifest.json".into(),
                bytes.len() as u64,
                blake3::hash(bytes).to_hex().to_string(),
            ),
            (
                "images/before.png".into(),
                record.before.encoded_bytes,
                record.before.blake3.clone(),
            ),
        ];
        if let Some(after) = &record.after {
            out.push((
                "images/after.png".into(),
                after.encoded_bytes,
                after.blake3.clone(),
            ));
        }
        out
    }
    fn present(
        &self,
        path: &Path,
        inventory: &[(String, u64, String)],
        complete: bool,
    ) -> PackageResult<(bool, Vec<usize>)> {
        let mut present = Vec::new();
        let mut images = false;
        for item in fs::read_dir(path)? {
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
                    let name = format!(
                        "images/{}",
                        image
                            .file_name()
                            .into_string()
                            .map_err(|_| PackageError::Integrity)?
                    );
                    files::regular(&fs::symlink_metadata(image.path())?, false)?;
                    present.push(
                        inventory
                            .iter()
                            .position(|v| v.0 == name)
                            .ok_or(PackageError::Integrity)?,
                    );
                    if present.len() > 3 {
                        return Err(PackageError::Integrity);
                    }
                }
            } else {
                files::regular(&fs::symlink_metadata(item.path())?, false)?;
                present.push(
                    inventory
                        .iter()
                        .position(|v| v.0 == name)
                        .ok_or(PackageError::Integrity)?,
                );
            }
            if present.len() > 3 {
                return Err(PackageError::Integrity);
            }
        }
        if complete && present.len() != inventory.len() {
            return Err(PackageError::Integrity);
        }
        Ok((images, present))
    }
    fn verify(
        &self,
        path: &Path,
        record: &Record,
        bytes: &[u8],
        cancel: &Cancellation,
    ) -> PackageResult<()> {
        let inventory = Self::inventory(record, bytes);
        self.present(path, &inventory, true)?;
        cancel.check()?;
        self.data(path, &record.before, "images/before.png")?;
        if let Some(after) = &record.after {
            cancel.check()?;
            self.data(path, after, "images/after.png")?;
        }
        Ok(())
    }
    pub(super) fn list(&self, cancel: &Cancellation) -> PackageResult<Vec<McpInboxReceipt>> {
        self.guard()?;
        self.usage()?;
        let mut ids = BTreeMap::new();
        for item in fs::read_dir(&self.root)? {
            cancel.check()?;
            let item = item?;
            let name = item
                .file_name()
                .into_string()
                .map_err(|_| PackageError::Integrity)?;
            if matches!(name.as_str(), "owner-v1" | "catalog-owner-v1.lock") {
                continue;
            }
            if name.starts_with("incoming-") || name.starts_with("retiring-write-") {
                continue;
            } // crash scratch counts toward disk/entry quota
            let (retired, key) = if let Some(id) = name.strip_prefix("retiring-") {
                (true, id)
            } else {
                (false, name.as_str())
            };
            id(key)?;
            ids.entry(key.to_owned())
                .and_modify(|v| *v |= retired)
                .or_insert(retired);
            if ids.len() > MAX_ENTRIES {
                return Err(PackageError::Limit);
            }
        }
        let mut out = Vec::with_capacity(ids.len());
        for (key, retired) in ids {
            cancel.check()?;
            let path = self.root.join(if retired {
                format!("retiring-{key}")
            } else {
                key.clone()
            });
            let (record, bytes) = self.record(&path, &key)?;
            if !retired {
                self.verify(&path, &record, &bytes, cancel)?;
            } else {
                self.present(
                    &path,
                    &[(
                        "manifest.json".into(),
                        bytes.len() as u64,
                        blake3::hash(&bytes).to_hex().to_string(),
                    )],
                    true,
                )?;
            }
            out.push(record.receipt(&bytes, retired));
        }
        Ok(out)
    }
    pub(super) fn submit(
        &self,
        submission: McpInboxSubmission,
        package: PackageInfo,
        before: Vec<u8>,
        cancel: &Cancellation,
    ) -> PackageResult<McpInboxReceipt> {
        self.guard()?;
        input(&submission)?;
        cancel.check()?;
        if package.package_id != submission.package_id
            || package.target != submission.target
            || package.manifest_sha256 != submission.manifest_sha256
        {
            return Err(PackageError::Identity);
        }
        let before_info = pixels::inspect(&before, cancel)?;
        let mut clean = package.images.iter().filter(|i| i.role == "clean_source");
        let image = clean.next().ok_or(PackageError::Integrity)?;
        if clean.next().is_some()
            || image.width != before_info.width
            || image.height != before_info.height
            || image.encoded_bytes != before_info.encoded_bytes
        {
            return Err(PackageError::Integrity);
        }
        let after = submission
            .png
            .as_ref()
            .map(|b| pixels::inspect(b, cancel))
            .transpose()?;
        let record = Record {
            schema: 1,
            id: submission.receipt_id,
            package_id: submission.package_id,
            target: submission.target,
            manifest_sha256: submission.manifest_sha256,
            project_id: package.binding.project_id,
            document_id: package.binding.document_id,
            host_seq: package.binding.host_seq,
            state_hash: package.binding.state_hash,
            source_asset_id: package.source_asset_id,
            connection: submission.connection,
            created_at_ms: submission.created_at_ms,
            text: submission.text,
            note: submission.note,
            before: before_info,
            after,
        };
        record.validate()?;
        let bytes = serde_json::to_vec(&record).map_err(|_| PackageError::Integrity)?;
        if bytes.len() > RECORD_LIMIT {
            return Err(PackageError::Limit);
        }
        let path = self.root.join(&record.id);
        if self.root.join(format!("retiring-{}", record.id)).exists() {
            return Err(PackageError::Stale);
        }
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                let (_, old) = self.record(&path, &record.id)?;
                if old != bytes {
                    return Err(PackageError::Identity);
                }
                self.verify(&path, &record, &bytes, cancel)?;
                return Ok(record.receipt(&bytes, false));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let (used, count) = self.usage()?;
        let added = bytes.len() as u64
            + before.len() as u64
            + submission.png.as_ref().map_or(0, |v| v.len() as u64);
        if count >= MAX_ENTRIES
            || used
                .checked_add(added)
                .and_then(|v| v.checked_add(RECORD_LIMIT as u64))
                .is_none_or(|v| v > DISK_LIMIT)
        {
            return Err(PackageError::Limit);
        }
        let stage = tempfile::Builder::new()
            .prefix("incoming-")
            .tempdir_in(&self.root)?;
        fs::create_dir(stage.path().join("images"))?;
        files::write_new(&stage.path().join("images/before.png"), &before)?;
        if let Some(png) = &submission.png {
            cancel.check()?;
            files::write_new(&stage.path().join("images/after.png"), png)?;
        }
        files::write_new(&stage.path().join("manifest.json"), &bytes)?;
        files::sync(&stage.path().join("images"))?;
        files::sync(stage.path())?;
        self.verify(stage.path(), &record, &bytes, cancel)?;
        cancel.check()?;
        self.guard()?;
        crate::creation::publish(stage.path(), &path)?;
        #[cfg(test)]
        if FAIL_AFTER_PUBLISH.swap(false, Ordering::AcqRel) {
            return Err(PackageError::Storage);
        }
        files::sync(&self.root)?;
        // No cancellation check after durable publication: abandoned replies
        // retain the completed item, discoverable by list/exact receipt identity.
        Ok(record.receipt(&bytes, false))
    }
    pub(super) fn image(
        &self,
        key: &str,
        wanted: &str,
        side: McpInboxSide,
        cancel: &Cancellation,
    ) -> PackageResult<(Record, Vec<u8>)> {
        self.guard()?;
        if self.root.join(format!("retiring-{key}")).exists() {
            return Err(PackageError::Stale);
        }
        let path = self.root.join(key);
        let (record, receipt) = self.record(&path, key)?;
        if blake3::hash(&receipt).to_hex().as_str() != wanted {
            return Err(PackageError::Identity);
        }
        cancel.check()?;
        let (info, name) = match side {
            McpInboxSide::Before => (&record.before, "images/before.png"),
            McpInboxSide::After => (
                record.after.as_ref().ok_or(PackageError::Invalid)?,
                "images/after.png",
            ),
        };
        let bytes = self.data(&path, info, name)?;
        cancel.check()?;
        Ok((record, bytes))
    }
    pub(super) fn retire(
        &self,
        key: &str,
        wanted: &str,
        cancel: &Cancellation,
    ) -> PackageResult<McpInboxRetirement> {
        self.guard()?;
        let marker = self.root.join(format!("retiring-{key}"));
        let path = self.root.join(key);
        let (record, bytes) = if marker.exists() {
            self.record(&marker, key)?
        } else {
            let (record, bytes) = self.record(&path, key)?;
            if blake3::hash(&bytes).to_hex().as_str() != wanted {
                return Err(PackageError::Identity);
            }
            self.verify(&path, &record, &bytes, cancel)?;
            if self
                .usage()?
                .0
                .checked_add(bytes.len() as u64)
                .is_none_or(|n| n > DISK_LIMIT)
            {
                return Err(PackageError::Limit);
            }
            let stage = tempfile::Builder::new()
                .prefix("retiring-write-")
                .tempdir_in(&self.root)?;
            files::write_new(&stage.path().join("manifest.json"), &bytes)?;
            files::sync(stage.path())?;
            cancel.check()?;
            crate::creation::publish(stage.path(), &marker)?;
            files::sync(&self.root)?;
            (record, bytes)
        };
        if blake3::hash(&bytes).to_hex().as_str() != wanted {
            return Err(PackageError::Identity);
        }
        cancel.check()?;
        let removed = (|| -> PackageResult<()> {
            match fs::symlink_metadata(&path) {
                Ok(meta) => {
                    files::regular(&meta, true)?;
                    let inventory = Self::inventory(&record, &bytes);
                    let (images, present) = self.present(&path, &inventory, false)?;
                    let refs = present
                        .iter()
                        .map(|&i| {
                            (
                                inventory[i].0.as_str(),
                                inventory[i].1,
                                inventory[i].2.as_str(),
                            )
                        })
                        .collect::<Vec<_>>();
                    retirement::remove(&self.root, &path, images, &refs, || {})?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            let hash = blake3::hash(&bytes).to_hex().to_string();
            self.present(
                &marker,
                &[("manifest.json".into(), bytes.len() as u64, hash.clone())],
                true,
            )?;
            retirement::remove(
                &self.root,
                &marker,
                false,
                &[("manifest.json", bytes.len() as u64, hash.as_str())],
                || {},
            )?;
            files::sync(&self.root)?;
            Ok(())
        })()
        .is_ok();
        Ok(McpInboxRetirement {
            receipt_id: key.into(),
            receipt_blake3: wanted.into(),
            files_removed: removed,
        })
    }
}
