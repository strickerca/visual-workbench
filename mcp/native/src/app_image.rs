//! The final bridge embeds the completed app inventory after jpackage. This
//! verifier never trusts an adjacent editable manifest as its authority.
use crate::Result;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
const LIMIT: usize = 4 * 1024 * 1024;
const MAX_FILE: u64 = 256 * 1024 * 1024;
const MAX_TOTAL: u64 = 1024 * 1024 * 1024;
const COMPILED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/app-image.inventory"));
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    hash: String,
    bytes: u64,
}
fn parse(bytes: &[u8]) -> Result<BTreeMap<String, Entry>> {
    if bytes.is_empty()
        || bytes.len() > LIMIT
        || !bytes.iter().all(|b| *b == b'\n' || (32..=126).contains(b))
    {
        return Err("app_inventory");
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "app_inventory")?;
    let mut entries = BTreeMap::new();
    let mut keys = BTreeSet::new();
    let mut total = 0u64;
    for line in text.lines() {
        if entries.len() >= 20_000 || line.is_empty() {
            return Err("app_inventory");
        }
        let mut parts = line.splitn(3, ' ');
        let hash = parts.next().ok_or("app_inventory")?;
        let size = parts.next().ok_or("app_inventory")?;
        let name = parts.next().ok_or("app_inventory")?;
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
        {
            return Err("app_hash");
        }
        if size.is_empty() || !size.bytes().all(|v| v.is_ascii_digit()) {
            return Err("app_size");
        }
        let bytes = size.parse::<u64>().map_err(|_| "app_size")?;
        if bytes > MAX_FILE {
            return Err("app_size");
        }
        total = total
            .checked_add(bytes)
            .filter(|n| *n <= MAX_TOTAL)
            .ok_or("app_size")?;
        valid_name(name)?;
        if !keys.insert(name.to_ascii_lowercase())
            || matches!(name, "vw-mcp.exe" | "vw-app-image.sha256")
        {
            return Err("app_duplicate");
        }
        entries.insert(
            name.to_owned(),
            Entry {
                hash: hash.to_owned(),
                bytes,
            },
        );
    }
    let launchers = ["VisualWorkbench.exe", "VisualWorkbenchDev.exe"]
        .iter()
        .filter(|n| entries.contains_key(**n))
        .count();
    if launchers != 1
        || !entries
            .keys()
            .any(|n| n.starts_with("app/") && n.ends_with(".jar"))
        || !entries.keys().any(|n| n.starts_with("runtime/"))
    {
        return Err("app_layout");
    }
    Ok(entries)
}
fn valid_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 256 || name.split('/').count() > 32 {
        return Err("app_path");
    }
    for part in name.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || !part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-@+ ".contains(&b))
        {
            return Err("app_path");
        }
        let stem = part
            .split('.')
            .next()
            .ok_or("app_path")?
            .to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err("app_path");
        }
    }
    Ok(())
}
/// All files and ancestor handles stay alive through app startup and the bridge
/// session. A changed/partial installed image is a refusal, never a path fallback.
pub struct AppImage {
    pub executable: PathBuf,
    _pins: Vec<File>,
}
impl AppImage {
    #[cfg(windows)]
    pub fn installed() -> Result<Self> {
        let executable = std::env::current_exe().map_err(|_| "executable")?;
        let root = executable.parent().ok_or("executable")?;
        Self::verify(root, COMPILED)
    }
    #[cfg(windows)]
    fn verify(root: &Path, compiled: &[u8]) -> Result<Self> {
        let entries = parse(compiled)?;
        crate::package_reader::no_redirect(root)?;
        let mut pins = Vec::new();
        let mut chain = root.ancestors().collect::<Vec<_>>();
        chain.reverse();
        for directory in chain {
            pins.push(pin(directory, true)?);
        }
        let mut manifest = pin(&root.join("vw-app-image.sha256"), false)?;
        if manifest.metadata().map_err(|_| "app_file")?.len() != compiled.len() as u64 {
            return Err("app_inventory");
        }
        let mut bytes = Vec::with_capacity(compiled.len());
        (&mut manifest)
            .take(LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "app_file")?;
        if bytes != compiled {
            return Err("app_inventory");
        }
        pins.push(manifest);
        let mut stack = vec![root.to_owned()];
        let mut found = BTreeSet::new();
        let mut nodes = 0;
        while let Some(folder) = stack.pop() {
            for child in std::fs::read_dir(folder).map_err(|_| "app_directory")? {
                let path = child.map_err(|_| "app_directory")?.path();
                nodes += 1;
                if nodes > 30_000 {
                    return Err("app_nodes");
                }
                crate::package_reader::no_redirect(&path)?;
                let name = path
                    .strip_prefix(root)
                    .map_err(|_| "app_path")?
                    .to_str()
                    .ok_or("app_path")?
                    .replace('\\', "/");
                valid_name(&name)?;
                let metadata = std::fs::symlink_metadata(&path).map_err(|_| "app_file")?;
                if metadata.is_dir() {
                    pins.push(pin(&path, true)?);
                    stack.push(path);
                    continue;
                }
                if matches!(name.as_str(), "vw-mcp.exe" | "vw-app-image.sha256") {
                    continue;
                }
                let expected = entries.get(&name).ok_or("app_unknown")?;
                if !found.insert(name) {
                    return Err("app_duplicate");
                }
                let mut file = pin(&path, false)?;
                if file.metadata().map_err(|_| "app_file")?.len() != expected.bytes {
                    return Err("app_size");
                }
                let mut hash = Sha256::new();
                let mut buffer = [0u8; 65536];
                let mut read = 0u64;
                loop {
                    let n = file.read(&mut buffer).map_err(|_| "app_file")?;
                    if n == 0 {
                        break;
                    }
                    read = read
                        .checked_add(n as u64)
                        .filter(|v| *v <= expected.bytes)
                        .ok_or("app_size")?;
                    hash.update(&buffer[..n]);
                }
                let digest = hash
                    .finalize()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>();
                if read != expected.bytes || digest != expected.hash {
                    return Err("app_hash");
                }
                pins.push(file);
            }
        }
        if found.len() != entries.len() {
            return Err("app_missing");
        }
        let launcher = if entries.contains_key("VisualWorkbench.exe") {
            "VisualWorkbench.exe"
        } else {
            "VisualWorkbenchDev.exe"
        };
        Ok(Self {
            executable: root.join(launcher),
            _pins: pins,
        })
    }
}
#[cfg(windows)]
fn pin(path: &Path, directory: bool) -> Result<File> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let mut options = OpenOptions::new();
    options.read(true);
    if directory {
        options
            .access_mode(0x81)
            .share_mode(3)
            .custom_flags(0x0200_0000 | 0x0020_0000);
    } else {
        options.share_mode(1).custom_flags(0x0020_0000);
    }
    let file = options.open(path).map_err(|_| "app_pin")?;
    let meta = file.metadata().map_err(|_| "app_file")?;
    if meta.file_attributes() & 0x400 != 0
        || meta.is_dir() != directory
        || (!directory && !meta.is_file())
    {
        return Err("app_redirect");
    }
    Ok(file)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(extra: &str) -> Vec<u8> {
        format!(
            "{} 1 VisualWorkbenchDev.exe\n{} 1 app/main.jar\n{} 1 runtime/bin/java.dll\n{extra}",
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64)
        )
        .into_bytes()
    }
    #[test]
    fn exact_app_inventory_admits_only_bounded_complete_layout() {
        assert_eq!(parse(&fixture("")).map(|v| v.len()), Ok(3));
        assert!(parse(b"").is_err());
        assert!(parse(&fixture(&format!("{} 1 APP/main.jar\n", "f".repeat(64)))).is_err());
    }
    #[test]
    fn traversal_reserved_paths_and_unknown_counts_refuse() {
        for path in ["../x", "app//x", "app/NUL.txt", "C:/x", "app/x ", "app/x."] {
            assert!(valid_name(path).is_err());
        }
        assert!(
            parse(&fixture(&format!(
                "{} {} app/huge\n",
                "a".repeat(64),
                MAX_FILE + 1
            )))
            .is_err()
        );
    }
    #[cfg(windows)]
    #[test]
    fn production_verification_refuses_changed_missing_and_extra_files() {
        use std::io::Write;
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path();
        std::fs::create_dir(root.join("app")).expect("dir");
        std::fs::create_dir_all(root.join("runtime/bin")).expect("dir");
        let digest = Sha256::digest(b"x")
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect::<String>();
        let bytes = format!(
            "{digest} 1 VisualWorkbenchDev.exe\n{digest} 1 app/main.jar\n{digest} 1 runtime/bin/java.dll\n"
        );
        for name in [
            "VisualWorkbenchDev.exe",
            "app/main.jar",
            "runtime/bin/java.dll",
        ] {
            std::fs::write(root.join(name), b"x").expect("file");
        }
        std::fs::write(root.join("vw-app-image.sha256"), &bytes).expect("manifest");
        let image = AppImage::verify(root, bytes.as_bytes()).expect("verified");
        assert!(
            OpenOptions::new()
                .write(true)
                .open(root.join("app/main.jar"))
                .is_err()
        );
        drop(image);
        std::fs::write(root.join("app/main.jar"), b"y").expect("change");
        assert!(AppImage::verify(root, bytes.as_bytes()).is_err());
        std::fs::write(root.join("app/main.jar"), b"x").expect("restore");
        let mut extra = File::create(root.join("app/injected.jar")).expect("extra");
        extra.write_all(b"x").expect("write");
        drop(extra);
        assert!(AppImage::verify(root, bytes.as_bytes()).is_err());
    }
}
