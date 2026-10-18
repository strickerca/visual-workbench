use crate::{Error, Result};
use ring::digest::{Context, SHA256};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

pub(crate) fn plain(path: &Path, directory: bool) -> Result<()> {
    let m = fs::symlink_metadata(path).map_err(|_| Error::Storage)?;
    if m.file_attributes() & 0x400 != 0 || m.is_dir() != directory || (!directory && !m.is_file()) {
        return Err(Error::Binding);
    }
    Ok(())
}
pub(crate) fn pin_ancestors(path: &Path) -> Result<Vec<File>> {
    if !path.is_absolute() {
        return Err(Error::Invalid);
    }
    let mut pins = Vec::new();
    for ancestor in path.ancestors() {
        plain(ancestor, true)?;
        pins.push(
            OpenOptions::new()
                .access_mode(0x81)
                .share_mode(3)
                .custom_flags(0x02000000)
                .open(ancestor)
                .map_err(|_| Error::Storage)?,
        );
    }
    Ok(pins)
}
pub(crate) fn read_locked(path: &Path) -> Result<File> {
    plain(path, false)?;
    OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(path)
        .map_err(|_| Error::Storage)
}
pub(crate) fn sha256(file: &mut File, max: u64) -> Result<String> {
    file.seek(SeekFrom::Start(0)).map_err(|_| Error::Storage)?;
    let mut context = Context::new(&SHA256);
    let mut total = 0u64;
    let mut buf = [0; 65536];
    loop {
        let n = file.read(&mut buf).map_err(|_| Error::Storage)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > max {
            return Err(Error::Limit);
        }
        context.update(&buf[..n]);
    }
    Ok(context
        .finish()
        .as_ref()
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect())
}
pub struct LockedHelper {
    pub(crate) path: PathBuf,
    _parents: Vec<File>,
    _file: File,
}
impl LockedHelper {
    /// expected is from the trusted packaged inventory, never a selected file.
    pub fn open(path: &Path, expected: &str) -> Result<Self> {
        if expected.len() != 64
            || !expected
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::Invalid);
        }
        let parents = pin_ancestors(path.parent().ok_or(Error::Invalid)?)?;
        let mut file = read_locked(path)?;
        if sha256(&mut file, 256 * 1024 * 1024)? != expected {
            return Err(Error::Binding);
        }
        Ok(Self {
            path: fs::canonicalize(path).map_err(|_| Error::Storage)?,
            _parents: parents,
            _file: file,
        })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pins {
    schema: u32,
    version: String,
    archive_sha256: String,
    files: BTreeMap<String, String>,
}
pub struct LockedRuntime {
    pub(crate) directory: PathBuf,
    _parents: Vec<File>,
    _files: Vec<File>,
}
impl LockedRuntime {
    pub fn open(directory: &Path) -> Result<Self> {
        let parents = pin_ancestors(directory)?;
        let directory = fs::canonicalize(directory).map_err(|_| Error::Storage)?;
        let pins: Pins = serde_json::from_str(include_str!("../runtime-pins.json"))
            .map_err(|_| Error::Binding)?;
        if pins.schema != 1
            || pins.version != "8.18.7"
            || pins.archive_sha256
                != "3a122eb3d588690008216f786338e2f9329dba3c3c2768b5933cf7299d549241"
        {
            return Err(Error::Binding);
        }
        // Only the exact packaged DLL set is eligible for loader search. Runtime
        // packaging uses a dedicated flat directory; plugin subdirectories fail.
        let mut actual = BTreeMap::new();
        for entry in fs::read_dir(&directory).map_err(|_| Error::Storage)? {
            let entry = entry.map_err(|_| Error::Storage)?;
            plain(&entry.path(), false)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| Error::Binding)?;
            if !pins.files.contains_key(&name) {
                return Err(Error::Binding);
            }
            actual.insert(name, ());
            if actual.len() > pins.files.len() {
                return Err(Error::Binding);
            }
        }
        if actual.len() != pins.files.len() {
            return Err(Error::Binding);
        }
        let mut files = Vec::with_capacity(pins.files.len());
        for (name, hash) in pins.files {
            let mut file = read_locked(&directory.join(name))?;
            if sha256(&mut file, 128 * 1024 * 1024)? != hash {
                return Err(Error::Binding);
            }
            files.push(file);
        }
        Ok(Self {
            directory,
            _parents: parents,
            _files: files,
        })
    }
}
