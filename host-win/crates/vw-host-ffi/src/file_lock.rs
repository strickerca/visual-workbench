//! Advisory file locks through the same operating-system calls the retired
//! `fs2` crate made: `flock` on Unix, `LockFileEx`/`UnlockFile` on Windows.
//! A copy of the lock half of `vw_store::os_fs`, which this crate does not
//! depend on. Errors keep their raw OS codes, so a lock that another open
//! file holds compares equal to [`lock_contended_error`].

use std::fs::File;
use std::io;

/// Take an exclusive lock on the whole file without waiting. The lock belongs
/// to the open file and ends with [`unlock`] or when the file is closed.
pub(crate) fn try_lock_exclusive(file: &File) -> io::Result<()> {
    sys::try_lock_exclusive(file)
}

/// Give up a lock taken with [`try_lock_exclusive`].
pub(crate) fn unlock(file: &File) -> io::Result<()> {
    sys::unlock(file)
}

/// The error a contended [`try_lock_exclusive`] returns: `EWOULDBLOCK` on
/// Unix, `ERROR_LOCK_VIOLATION` on Windows.
pub(crate) fn lock_contended_error() -> io::Error {
    sys::lock_contended_error()
}

#[cfg(unix)]
mod sys {
    use rustix::fs::{FlockOperation, flock};
    use std::fs::File;
    use std::io;

    pub(super) fn try_lock_exclusive(file: &File) -> io::Result<()> {
        flock(file, FlockOperation::NonBlockingLockExclusive).map_err(io::Error::from)
    }

    pub(super) fn unlock(file: &File) -> io::Result<()> {
        flock(file, FlockOperation::Unlock).map_err(io::Error::from)
    }

    pub(super) fn lock_contended_error() -> io::Error {
        io::Error::from(rustix::io::Errno::WOULDBLOCK)
    }
}

#[cfg(windows)]
mod sys {
    use std::fs::File;
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::ERROR_LOCK_VIOLATION;
    use windows_sys::Win32::Storage::FileSystem::{
        LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx, UnlockFile,
    };
    use windows_sys::Win32::System::IO::OVERLAPPED;

    pub(super) fn try_lock_exclusive(file: &File) -> io::Result<()> {
        let mut overlapped = OVERLAPPED::default();
        // SAFETY: the handle belongs to `file`, which outlives the call. Every
        // caller opens its lock file for synchronous I/O, so the call finishes
        // before it returns and nothing refers to the zeroed OVERLAPPED (offset
        // 0, no event) afterwards. The range is the whole file.
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

    pub(super) fn unlock(file: &File) -> io::Result<()> {
        // SAFETY: the handle belongs to `file`, which outlives the call; the
        // call only releases a byte-range lock on it.
        let unlocked = unsafe { UnlockFile(file.as_raw_handle(), 0, 0, u32::MAX, u32::MAX) };
        if unlocked == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub(super) fn lock_contended_error() -> io::Error {
        io::Error::from_raw_os_error(ERROR_LOCK_VIOLATION as i32)
    }
}
