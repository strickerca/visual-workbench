//! Exact operation-directory cleanup. A failed deletion retains its admission
//! and input permit, so another build cannot turn leftovers into unbounded disk.
use super::{Error, Result};
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

struct Pending {
    directory: tempfile::TempDir,
    // Actual directory first, then its retained memory/operation owners.
    _retained: Box<dyn Send + Sync>,
}
static QUARANTINE: OnceLock<Mutex<Vec<Pending>>> = OnceLock::new();
fn quarantine() -> &'static Mutex<Vec<Pending>> {
    // Two admitted inputs, each with at most one producer output and publication attempt. Failed
    // cleanup retains an input permit (directly or through its source Arc).
    QUARANTINE.get_or_init(|| Mutex::new(Vec::with_capacity(6)))
}
fn remove(path: &Path) -> Result<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(Error::Storage),
    }
}
/// No automatic retry of quarantined directories: a failed deletion may mean a
/// platform resource is still live. Keep its owners until an explicit verified
/// recovery policy exists. Process restart does not make these directories cache
/// approval; free-space admission still counts the remaining disk use.
pub(super) struct OwnedDirectory {
    path: PathBuf,
    pending: Option<Pending>,
}
impl OwnedDirectory {
    pub fn new(directory: tempfile::TempDir, retained: impl Send + Sync + 'static) -> Self {
        Self {
            path: directory.path().to_path_buf(),
            pending: Some(Pending {
                directory,
                _retained: Box::new(retained),
            }),
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn cleanup(&mut self) -> Result<()> {
        let Some(pending) = self.pending.take() else {
            return Ok(());
        };
        let path = std::mem::take(&mut self.path);
        let result = remove(pending.directory.path());
        drop(path);
        if let Err(error) = result {
            let mut entries = match quarantine().lock() {
                Ok(entries) => entries,
                Err(poisoned) => poisoned.into_inner(),
            };
            entries.push(pending);
            return Err(error);
        }
        drop(pending);
        Ok(())
    }
}
impl Drop for OwnedDirectory {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
