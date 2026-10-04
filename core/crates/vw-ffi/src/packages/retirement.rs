//! The catalog is portable; destructive byte retirement is admitted only where
//! retained kernel handles enforce its exact-byte/ancestor-identity contract.
use super::*;
use std::path::Path;
#[cfg(not(windows))]
pub(super) fn remove(
    _root: &Path,
    _package: &Path,
    _images: bool,
    _inventory: &[(&str, u64, &str)],
    _before_delete: impl FnOnce(),
) -> PackageResult<()> {
    Err(PackageError::Unsupported)
}

#[cfg(windows)]
pub(super) fn remove(
    root: &Path,
    package: &Path,
    images: bool,
    inventory: &[(&str, u64, &str)],
    before_delete: impl FnOnce(),
) -> PackageResult<()> {
    use std::{
        fs::{File, OpenOptions},
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
        path::{Component, PathBuf},
    };
    fn pin(path: &Path, directory: bool, delete: bool) -> PackageResult<File> {
        let mut o = OpenOptions::new();
        // FILE_LIST_DIRECTORY/FILE_READ_DATA plus READ_ATTRIBUTES. Merely
        // READ_ATTRIBUTES does not prevent directory rename on Windows.
        o.access_mode(0x81 | if delete { 0x10000 } else { 0 })
            .share_mode(if directory { 3 } else { 1 })
            .custom_flags(0x0020_0000 | if directory { 0x0200_0000 } else { 0 });
        let f = o.open(path)?;
        files::regular(&f.metadata()?, directory)?;
        Ok(f)
    }
    #[repr(C)]
    struct Disposition {
        delete: u8,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetFileInformationByHandle(
            handle: *mut std::ffi::c_void,
            class: i32,
            info: *const std::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    fn dispose(file: &File) -> PackageResult<()> {
        let info = Disposition { delete: 1 };
        // SAFETY: this owned live File was opened with DELETE; the API borrows
        // the exact one-byte FILE_DISPOSITION_INFO for this synchronous call.
        // No path is re-resolved. Actual close belongs to File's RAII owner.
        let result = unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                4,
                (&info as *const Disposition).cast(),
                std::mem::size_of::<Disposition>() as u32,
            )
        };
        if result == 0 {
            return Err(PackageError::Storage);
        }
        Ok(())
    }
    files::directory(root)?;
    files::directory(package)?;
    if package.parent() != Some(root) {
        return Err(PackageError::Integrity);
    }
    // Retain every ancestor, including the root, so no path component can be
    // renamed/replaced between validation and the final handle disposition.
    let mut parents = Vec::new();
    let mut current = PathBuf::new();
    for component in root.components() {
        current.push(component);
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        parents.push(pin(&current, true, false)?);
    }
    let directory = pin(package, true, true)?;
    let image_directory = if images {
        Some(pin(&package.join("images"), true, true)?)
    } else {
        None
    };
    let mut pinned = Vec::with_capacity(inventory.len());
    for (name, n, hash) in inventory {
        files::safe_name(name)?;
        let mut file = pin(&package.join(name), false, true)?;
        files::verify_open_hash(&mut file, *n, hash)?;
        pinned.push(file);
    }
    before_delete();
    for file in pinned {
        dispose(&file)?;
        drop(file);
    }
    if let Some(directory) = image_directory {
        dispose(&directory)?;
        drop(directory);
    }
    dispose(&directory)?;
    drop(directory);
    drop(parents);
    Ok(())
}
