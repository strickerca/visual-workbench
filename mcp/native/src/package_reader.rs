use crate::Result;
use std::{
    fs::{self, File},
    io::Read,
    path::{Component, Path},
};
const MAX: usize = 32 * 1024 * 1024;
// Canonical inventory: clean + overview + one crop per marker, followed by
// manifest, semantic data and prompt. The compiler admits at most 64 markers.
const MAX_FILES: usize = 64 + 5;
/// Reject every redirect component, not just the final filename. This helper is
/// called only on an owner-published directory, never an MCP-supplied path.
pub fn no_redirect(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err("absolute_path");
    }
    let mut current = std::path::PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err("path");
        }
        current.push(component);
        let meta = fs::symlink_metadata(&current).map_err(|_| "path")?;
        if meta.file_type().is_symlink() {
            return Err("redirect");
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("redirect");
            }
        }
    }
    Ok(())
}
fn read(path: &Path, remaining: usize) -> Result<Vec<u8>> {
    no_redirect(path)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let file = options.open(path).map_err(|_| "file")?;
    read_open(file, remaining)
}
fn read_open(file: File, remaining: usize) -> Result<Vec<u8>> {
    let metadata = file.metadata().map_err(|_| "file")?;
    if !metadata.is_file() || metadata.len() > remaining as u64 {
        return Err("package_limit");
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(remaining as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "file")?;
    if bytes.len() > remaining {
        return Err("package_limit");
    }
    Ok(bytes)
}
pub fn verify(root: &Path) -> Result<serde_json::Value> {
    no_redirect(root)?;
    if !root.is_dir() {
        return Err("directory");
    }
    let mut files = Vec::new();
    let mut total = 0;
    for entry in fs::read_dir(root).map_err(|_| "directory")? {
        let entry = entry.map_err(|_| "entry")?;
        let name = entry.file_name().into_string().map_err(|_| "name")?;
        if name == "images" {
            no_redirect(&entry.path())?;
            if !entry.path().is_dir() {
                return Err("directory");
            }
            for image in fs::read_dir(entry.path()).map_err(|_| "directory")? {
                let image = image.map_err(|_| "entry")?;
                let name = image.file_name().into_string().map_err(|_| "name")?;
                if files.len() >= MAX_FILES {
                    return Err("inventory_limit");
                }
                let bytes = read(&image.path(), MAX - total)?;
                total += bytes.len();
                files.push((format!("images/{name}"), bytes));
            }
        } else {
            if files.len() >= MAX_FILES
                || !matches!(
                    name.as_str(),
                    "manifest.json" | "semantic.json" | "prompt.md"
                )
            {
                return Err("inventory");
            }
            let bytes = read(&entry.path(), MAX - total)?;
            total += bytes.len();
            files.push((name, bytes));
        }
    }
    let limits = vw_package::Limits {
        memory_bytes: 256 * 1024 * 1024,
        package_bytes: MAX,
        image_bytes: 16 * 1024 * 1024,
        ..Default::default()
    };
    let package = vw_package::Package::from_files(files, limits, &vw_package::NeverCancel)
        .map_err(|_| "package_invalid")?;
    Ok(
        serde_json::json!({"manifest_sha256":package.manifest_sha256(),"manifest":package.manifest()}),
    )
}
