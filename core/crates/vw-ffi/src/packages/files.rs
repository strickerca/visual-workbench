use super::*;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
pub(super) fn regular(meta: &fs::Metadata, dir: bool) -> PackageResult<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(PackageError::Integrity);
        }
    }
    if meta.file_type().is_symlink() || (dir && !meta.is_dir()) || (!dir && !meta.is_file()) {
        return Err(PackageError::Integrity);
    }
    Ok(())
}
pub(super) fn directory(path: &Path) -> PackageResult<PathBuf> {
    if !path.is_absolute() || path.as_os_str().len() > 32767 {
        return Err(PackageError::Invalid);
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::CurDir | Component::ParentDir) {
            return Err(PackageError::Invalid);
        }
        current.push(component);
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        regular(&fs::symlink_metadata(&current)?, true)?;
    }
    // Canonicalization resolves platform prefixes/case; symlinks were refused.
    Ok(path.canonicalize()?)
}
fn options() -> OpenOptions {
    let mut o = OpenOptions::new();
    o.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        o.custom_flags(0x0020_0000).share_mode(1);
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.custom_flags(0x0002_0000);
    }
    o
}
pub(super) fn read_exact(path: &Path, expected: u64, limit: usize) -> PackageResult<Vec<u8>> {
    if expected > limit as u64 {
        return Err(PackageError::Limit);
    }
    regular(&fs::symlink_metadata(path)?, false)?;
    let mut f = options().open(path)?;
    let meta = f.metadata()?;
    regular(&meta, false)?;
    if meta.len() != expected {
        return Err(PackageError::Integrity);
    }
    let len = usize::try_from(expected).map_err(|_| PackageError::Limit)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(len)
        .map_err(|_| PackageError::Limit)?;
    bytes.resize(len, 0);
    f.read_exact(&mut bytes)?;
    if f.read(&mut [0u8; 1])? != 0 {
        return Err(PackageError::Integrity);
    }
    Ok(bytes)
}
pub(super) fn read(path: &Path, limit: usize) -> PackageResult<Vec<u8>> {
    let m = fs::symlink_metadata(path)?;
    regular(&m, false)?;
    read_exact(path, m.len(), limit)
}
pub(super) fn write_new(path: &Path, bytes: &[u8]) -> PackageResult<()> {
    let mut f = OpenOptions::new().create_new(true).write(true).open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
pub(super) fn sync(path: &Path) -> PackageResult<()> {
    crate::creation::sync_directory(path)?;
    Ok(())
}
pub(super) fn package(root: &Path, cancel: &Cancellation) -> PackageResult<vw_package::Package> {
    directory(root)?;
    let mut names = Vec::new();
    let mut bytes = 0u64;
    for entry in fs::read_dir(root)? {
        cancel.check()?;
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| PackageError::Integrity)?;
        if name == "images" {
            regular(&fs::symlink_metadata(entry.path())?, true)?;
            for image in fs::read_dir(entry.path())? {
                let image = image?;
                let leaf = image
                    .file_name()
                    .into_string()
                    .map_err(|_| PackageError::Integrity)?;
                let meta = fs::symlink_metadata(image.path())?;
                regular(&meta, false)?;
                bytes = bytes.checked_add(meta.len()).ok_or(PackageError::Limit)?;
                names.push((format!("images/{leaf}"), meta.len()));
                if names.len() > 69 || bytes > MAX_PACKAGE as u64 {
                    return Err(PackageError::Limit);
                }
            }
        } else {
            let meta = fs::symlink_metadata(entry.path())?;
            regular(&meta, false)?;
            bytes = bytes.checked_add(meta.len()).ok_or(PackageError::Limit)?;
            names.push((name, meta.len()));
        }
        if names.len() > 69 || bytes > MAX_PACKAGE as u64 {
            return Err(PackageError::Limit);
        }
    }
    let mut content = Vec::new();
    for (name, n) in names {
        cancel.check()?;
        safe_name(&name)?;
        content.push((name.clone(), read_exact(&root.join(&name), n, MAX_PACKAGE)?));
    }
    Ok(vw_package::Package::from_files(
        content,
        limits(MAX_MEMORY - 8 * 1024 * 1024),
        cancel,
    )?)
}
pub(super) fn safe_name(name: &str) -> PackageResult<()> {
    if matches!(name, "manifest.json" | "semantic.json" | "prompt.md") {
        return Ok(());
    }
    let leaf = name
        .strip_prefix("images/")
        .and_then(|v| v.strip_suffix(".png"))
        .ok_or(PackageError::Integrity)?;
    if leaf.is_empty()
        || leaf.len() > 64
        || !leaf
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-'))
    {
        return Err(PackageError::Integrity);
    }
    Ok(())
}
pub(super) struct RootLock {
    _file: File,
}
pub(super) fn lock(root: &Path) -> PackageResult<RootLock> {
    let path = root.join("catalog-owner-v1.lock");
    let mut o = options();
    o.write(true).create(true);
    let f = o.open(path)?;
    regular(&f.metadata()?, false)?;
    if f.metadata()?.len() != 0 {
        return Err(PackageError::Integrity);
    }
    fs2::FileExt::try_lock_exclusive(&f).map_err(|_| PackageError::Busy)?;
    Ok(RootLock { _file: f })
}

/// Full byte identity to an already production-verified compiler result. This
/// readback needs one fixed buffer, not a second decoded package beside it.
pub(super) fn matches(
    root: &Path,
    package: &vw_package::Package,
    cancel: &Cancellation,
) -> PackageResult<()> {
    directory(root)?;
    let expected = package.files().count();
    let mut seen = 0usize;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| PackageError::Integrity)?;
        if name == "images" {
            regular(&fs::symlink_metadata(entry.path())?, true)?;
            for image in fs::read_dir(entry.path())? {
                let image = image?;
                let leaf = image
                    .file_name()
                    .into_string()
                    .map_err(|_| PackageError::Integrity)?;
                let name = format!("images/{leaf}");
                safe_name(&name)?;
                if package.file(&name).is_none() {
                    return Err(PackageError::Integrity);
                }
                seen += 1;
                if seen > expected {
                    return Err(PackageError::Integrity);
                }
            }
        } else {
            safe_name(&name)?;
            if package.file(&name).is_none() {
                return Err(PackageError::Integrity);
            }
            seen += 1;
        }
        if seen > expected {
            return Err(PackageError::Integrity);
        }
    }
    if seen != expected {
        return Err(PackageError::Integrity);
    }
    let mut buffer = [0u8; 65536];
    for (name, bytes) in package.files() {
        cancel.check()?;
        let path = root.join(name);
        regular(&fs::symlink_metadata(&path)?, false)?;
        let mut f = options().open(&path)?;
        regular(&f.metadata()?, false)?;
        if f.metadata()?.len() != bytes.len() as u64 {
            return Err(PackageError::Integrity);
        }
        let mut at = 0;
        while at < bytes.len() {
            cancel.check()?;
            let n = (bytes.len() - at).min(buffer.len());
            f.read_exact(&mut buffer[..n])?;
            if buffer[..n] != bytes[at..at + n] {
                return Err(PackageError::Integrity);
            }
            at += n;
        }
        if f.read(&mut buffer[..1])? != 0 {
            return Err(PackageError::Integrity);
        }
    }
    Ok(())
}

/// Hash the already pinned file; the caller retains it through disposition.
#[cfg(windows)]
pub(super) fn verify_open_hash(f: &mut File, expected: u64, wanted: &str) -> PackageResult<()> {
    if expected > MAX_PACKAGE as u64 {
        return Err(PackageError::Limit);
    }
    regular(&f.metadata()?, false)?;
    if f.metadata()?.len() != expected {
        return Err(PackageError::Integrity);
    }
    let mut hash = blake3::Hasher::new();
    let mut remaining = expected;
    let mut buffer = [0u8; 65536];
    while remaining > 0 {
        let n = (remaining as usize).min(buffer.len());
        f.read_exact(&mut buffer[..n])?;
        hash.update(&buffer[..n]);
        remaining -= n as u64;
    }
    if f.read(&mut buffer[..1])? != 0 || hash.finalize().to_hex().as_str() != wanted {
        return Err(PackageError::Integrity);
    }
    Ok(())
}
