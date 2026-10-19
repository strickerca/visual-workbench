//! Windows-only dynamic libvips pyramid worker. No libvips is linked on Android.
//! The parent owns the suspended child Job, time/cancel/memory limits and pins.
mod admission;
mod budget;
#[cfg(windows)]
mod native;
#[cfg(windows)]
mod ownership;
#[cfg(windows)]
mod process;
#[cfg(windows)]
mod worker;
pub use admission::{BuildRequest, admit_input, estimated_disk_bytes};
pub use budget::{BudgetedManifest, WORKER_MEMORY_BYTES};
#[cfg(windows)]
pub use ownership::{LockedHelper, LockedRuntime};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[cfg(windows)]
pub use worker::{build_budgeted, helper};

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid pyramid request")]
    Invalid,
    #[error("unsupported giant-image profile, depth, alpha or codec")]
    Unsupported,
    #[error("pyramid resource limit exceeded")]
    Limit,
    #[error("pyramid memory estimate {estimated} bytes exceeds session budget {budget}")]
    Memory { estimated: u64, budget: u64 },
    #[error("pyramid input or runtime identity changed")]
    Binding,
    #[error("pyramid storage unavailable")]
    Storage,
    #[error("pyramid native worker unavailable")]
    WorkerUnavailable,
    #[error("pyramid native codec failed")]
    Codec,
    #[error("pyramid build timed out")]
    Timeout,
    #[error("pyramid build cancelled")]
    Cancelled,
    #[error("pyramid builder is busy")]
    Busy,
}
/// Local cancellation plus a caller-owned operation/image-close probe. The
/// synchronous worker checks it during hashing, output verification and its
/// process wait loop. No watcher thread or independent lifetime is created.
pub struct Cancellation {
    local: AtomicBool,
    probe: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}
impl Default for Cancellation {
    fn default() -> Self {
        Self {
            local: AtomicBool::new(false),
            probe: None,
        }
    }
}
impl Cancellation {
    /// A true probe result means cancelled or owner closed. Capture retained
    /// owners in the probe; avoid blocking or panicking in its implementation.
    pub fn from_probe(probe: Arc<dyn Fn() -> bool + Send + Sync>) -> Self {
        Self {
            local: AtomicBool::new(false),
            probe: Some(probe),
        }
    }
    pub fn cancel(&self) {
        self.local.store(true, Ordering::Release);
    }
    pub fn check(&self) -> Result<()> {
        if self.local.load(Ordering::Acquire) || self.probe.as_ref().is_some_and(|probe| probe()) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}
#[cfg(windows)]
fn wide(value: &std::ffi::OsStr) -> Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let mut units: Vec<u16> = value.encode_wide().collect();
    if units.is_empty() || units.len() > 30_000 || units.contains(&0) {
        return Err(Error::Invalid);
    }
    units.push(0);
    Ok(units)
}
#[cfg(windows)]
struct Handle(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Handle {
    fn checked(raw: windows_sys::Win32::Foundation::HANDLE) -> Result<Self> {
        if raw.is_null() || raw == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            Err(Error::WorkerUnavailable)
        } else {
            Ok(Self(raw))
        }
    }
}
#[cfg(windows)]
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: unique owned Win32 handle.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
