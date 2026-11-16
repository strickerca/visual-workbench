//! Advisory file lock through the same operating-system call the retired
//! `fs2` crate made: `flock` on Unix, `LockFileEx` on Windows. A copy of the
//! lock part of `vw_store::os_fs`, which this tool does not depend on. Errors
//! keep their raw OS codes.

use std::fs::File;
use std::io;

/// Take an exclusive lock on the whole file without waiting. The lock belongs
/// to the open file and ends when the file is closed.
#[cfg(unix)]
pub(crate) fn try_lock_exclusive(file: &File) -> io::Result<()> {
    rustix::fs::flock(file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .map_err(io::Error::from)
}

/// Take an exclusive lock on the whole file without waiting. The lock belongs
/// to the open file and ends when the file is closed.
#[cfg(windows)]
pub(crate) fn try_lock_exclusive(file: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx,
    };
    let mut overlapped = windows_sys::Win32::System::IO::OVERLAPPED::default();
    // SAFETY: the handle belongs to `file`, which outlives the call. The ledger
    // is opened for synchronous I/O, so the call finishes before it returns and
    // nothing refers to the zeroed OVERLAPPED (offset 0, no event) afterwards.
    // The range is the whole file.
    let locked = unsafe {
        LockFileEx(
            file.as_raw_handle(),
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
    if locked == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
