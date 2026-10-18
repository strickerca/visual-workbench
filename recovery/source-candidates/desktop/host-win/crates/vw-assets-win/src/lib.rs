//! Windows-only dynamic libvips pyramid worker. No libvips is linked on Android.
//! The parent owns the suspended child Job, time/cancel/memory limits and pins.
mod admission;
#[cfg(windows)] mod native;
#[cfg(windows)] mod ownership;
#[cfg(windows)] mod process;
#[cfg(windows)] mod worker;
#[cfg(windows)] pub use ownership::{LockedHelper, LockedRuntime};
#[cfg(windows)] pub use worker::{build, helper};
pub use admission::{BuildRequest, admit_input, estimated_disk_bytes};
use std::sync::atomic::{AtomicBool, Ordering};

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid pyramid request")] Invalid,
    #[error("unsupported giant-image profile, depth, alpha or codec")] Unsupported,
    #[error("pyramid resource limit exceeded")] Limit,
    #[error("pyramid input or runtime identity changed")] Binding,
    #[error("pyramid storage unavailable")] Storage,
    #[error("pyramid native worker unavailable")] WorkerUnavailable,
    #[error("pyramid native codec failed")] Codec,
    #[error("pyramid build timed out")] Timeout,
    #[error("pyramid build cancelled")] Cancelled,
    #[error("pyramid builder is busy")] Busy,
}
pub struct Cancellation(AtomicBool);
impl Default for Cancellation { fn default() -> Self { Self(AtomicBool::new(false)) } }
impl Cancellation {
    pub fn cancel(&self) { self.0.store(true,Ordering::Release); }
    pub fn check(&self) -> Result<()> { if self.0.load(Ordering::Acquire) { Err(Error::Cancelled) } else { Ok(()) } }
}
#[cfg(windows)]
fn wide(value: &std::ffi::OsStr) -> Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let mut units: Vec<u16> = value.encode_wide().collect();
    if units.is_empty() || units.len() > 30_000 || units.contains(&0) { return Err(Error::Invalid); }
    units.push(0); Ok(units)
}
#[cfg(windows)]
struct Handle(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Handle {
    fn checked(raw:windows_sys::Win32::Foundation::HANDLE)->Result<Self>{
        if raw.is_null() || raw==windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {Err(Error::WorkerUnavailable)}else{Ok(Self(raw))}
    }
}
#[cfg(windows)]
impl Drop for Handle { fn drop(&mut self) { // SAFETY: unique owned Win32 handle.
    unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0); }
} }
