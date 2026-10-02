use crate::StoreError;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use vw_model::AssetId;

/// Immutable, content-addressed originals. Every read verifies the full BLAKE3 hash.
#[derive(Debug, Clone)]
pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    /// Create the blob hierarchy beneath a caller-selected project directory.
    pub fn new(project_root: &Path) -> Result<Self, StoreError> {
        let root = project_root.canonicalize()?;
        check_directory(&root)?;
        ensure_directory(&root.join("blobs"))?;
        Ok(Self { root })
    }

    /// Atomically install bytes without replacing an existing original.
    pub fn put(&self, bytes: &[u8]) -> Result<AssetId, StoreError> {
        self.put_reader(&mut std::io::Cursor::new(bytes))
    }

    /// Stream a potentially large asset through a bounded buffer and hash it.
    pub fn put_reader<R: Read>(&self, reader: &mut R) -> Result<AssetId, StoreError> {
        let blobs = self.root.join("blobs");
        check_directory(&self.root)?;
        check_directory(&blobs)?;
        let incoming = blobs.join(".incoming-v1");
        ensure_directory(&incoming)?;
        let mut temporary = tempfile::Builder::new()
            .prefix("asset-")
            .tempfile_in(&incoming)?;
        let mut hasher = blake3::Hasher::new();
        let mut buffer = [0u8; 65_536];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
            temporary.write_all(&buffer[..count])?;
        }
        temporary.as_file().sync_all()?;
        let id = AssetId::try_from(hasher.finalize().to_hex().to_string())?;
        let first = blobs.join(&id.as_str()[..2]);
        ensure_directory(&first)?;
        let second = first.join(&id.as_str()[2..4]);
        ensure_directory(&second)?;
        let path = self.path(&id)?;
        match temporary.persist_noclobber(&path) {
            Ok(file) => {
                file.sync_all()?;
                sync_directory(&second)?;
                sync_directory(&incoming)?;
            }
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.verify(&id)?;
            }
            Err(error) => return Err(StoreError::Io(error.error)),
        }
        Ok(id)
    }

    /// Read an original and reject any changed bytes, including same-length damage.
    pub fn read(&self, id: &AssetId) -> Result<Vec<u8>, StoreError> {
        let path = self.path(id)?;
        let mut file = open_regular_file(&path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        if AssetId::hash(&bytes) != *id {
            return Err(StoreError::Corrupt("blob hash"));
        }
        self.path(id)?;
        Ok(bytes)
    }

    /// Verify a large original without retaining its contents in memory.
    pub fn verify(&self, id: &AssetId) -> Result<u64, StoreError> {
        let path = self.path(id)?;
        let mut file = open_regular_file(&path)?;
        let mut hasher = blake3::Hasher::new();
        let mut count = 0u64;
        let mut buffer = [0u8; 65_536];
        loop {
            let length = file.read(&mut buffer)?;
            if length == 0 {
                break;
            }
            count = count
                .checked_add(length as u64)
                .ok_or(StoreError::Invalid("blob size overflow"))?;
            hasher.update(&buffer[..length]);
        }
        if hasher.finalize().as_bytes() != &id.bytes() {
            return Err(StoreError::Corrupt("blob hash"));
        }
        self.path(id)?;
        Ok(count)
    }

    /// Return the canonical layout path after checking existing child components.
    /// Call `read` or `verify` when content integrity is required.
    pub fn path(&self, id: &AssetId) -> Result<PathBuf, StoreError> {
        check_directory(&self.root)?;
        let mut path = self.root.join("blobs");
        check_directory(&path)?;
        for part in [&id.as_str()[..2], &id.as_str()[2..4]] {
            path.push(part);
            check_optional_directory(&path)?;
        }
        path.push(id.as_str());
        match fs::symlink_metadata(&path) {
            Ok(metadata) => check_regular_metadata(&metadata)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(path)
    }
}

pub(crate) fn ensure_directory(path: &Path) -> Result<(), StoreError> {
    match fs::create_dir(path) {
        Ok(()) => {
            if let Some(parent) = path.parent() {
                sync_directory(parent)?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    check_directory(path)
}

pub(crate) fn check_directory(path: &Path) -> Result<(), StoreError> {
    let metadata = fs::symlink_metadata(path)?;
    if is_link(&metadata) || !metadata.is_dir() {
        return Err(StoreError::Invalid("linked or non-directory storage path"));
    }
    Ok(())
}

fn check_optional_directory(path: &Path) -> Result<(), StoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !is_link(&metadata) && metadata.is_dir() => Ok(()),
        Ok(_) => Err(StoreError::Invalid("linked or non-directory storage path")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn check_regular_metadata(metadata: &Metadata) -> Result<(), StoreError> {
    if is_link(metadata) || !metadata.is_file() {
        return Err(StoreError::Invalid("linked or non-file storage path"));
    }
    Ok(())
}

pub(crate) fn is_link(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub(crate) fn open_regular_file(path: &Path) -> Result<File, StoreError> {
    check_regular_metadata(&fs::symlink_metadata(path)?)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT prevents following a replaced final link.
        options.custom_flags(0x0020_0000);
    }
    #[cfg(any(target_os = "android", target_os = "linux"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Linux/Android O_NOFOLLOW prevents following a replaced final symlink.
        options.custom_flags(0x0002_0000);
    }
    let file = options.open(path)?;
    check_regular_metadata(&file.metadata()?)?;
    Ok(file)
}

pub(crate) fn sync_directory(path: &Path) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        // Windows does not expose directory fsync through the safe standard API.
        // Installed files are explicitly flushed before/after their atomic rename.
        let _ = path;
    }
    Ok(())
}
