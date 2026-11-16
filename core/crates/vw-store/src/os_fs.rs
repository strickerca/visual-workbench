//! Advisory file locks and volume space through the same operating-system
//! calls the retired `fs2` crate made: `flock` on every Unix (Android
//! included) and `LockFileEx`/`UnlockFile` on Windows for locks; `statvfs` on
//! Unix and `GetVolumePathNameW` with `GetDiskFreeSpaceW` on Windows for space.
//! Errors keep their raw OS codes, so a lock that another open file holds
//! compares equal to [`lock_contended_error`].

use std::fs::File;
use std::io;
use std::path::Path;

/// Take an exclusive lock on the whole file without waiting. The lock belongs
/// to the open file and ends with [`unlock`] or when the file is closed.
pub fn try_lock_exclusive(file: &File) -> io::Result<()> {
    sys::try_lock_exclusive(file)
}

/// Give up a lock taken with [`try_lock_exclusive`].
pub fn unlock(file: &File) -> io::Result<()> {
    sys::unlock(file)
}

/// The error a contended [`try_lock_exclusive`] returns: `EWOULDBLOCK` on
/// Unix, `ERROR_LOCK_VIOLATION` on Windows.
pub fn lock_contended_error() -> io::Error {
    sys::lock_contended_error()
}

/// Capacity and available bytes of the volume holding `path`:
/// `(total, available)`.
pub(crate) fn volume_space(path: &Path) -> io::Result<(u64, u64)> {
    sys::volume_space(path)
}

#[cfg(unix)]
mod sys {
    use rustix::fs::{FlockOperation, flock};
    use std::fs::File;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    pub(super) fn try_lock_exclusive(file: &File) -> io::Result<()> {
        flock(file, FlockOperation::NonBlockingLockExclusive).map_err(io::Error::from)
    }

    pub(super) fn unlock(file: &File) -> io::Result<()> {
        flock(file, FlockOperation::Unlock).map_err(io::Error::from)
    }

    pub(super) fn lock_contended_error() -> io::Error {
        io::Error::from(rustix::io::Errno::WOULDBLOCK)
    }

    pub(super) fn volume_space(path: &Path) -> io::Result<(u64, u64)> {
        if path.as_os_str().as_bytes().contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path contained a null",
            ));
        }
        let stat = rustix::fs::statvfs(path)?;
        Ok((
            stat.f_frsize.saturating_mul(stat.f_blocks),
            stat.f_frsize.saturating_mul(stat.f_bavail),
        ))
    }
}

#[cfg(windows)]
mod sys {
    use std::fs::File;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use windows_sys::Win32::Foundation::ERROR_LOCK_VIOLATION;
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceW, GetVolumePathNameW, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
        LockFileEx, UnlockFile,
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

    pub(super) fn volume_space(path: &Path) -> io::Result<(u64, u64)> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut root = [0u16; 261];
        // SAFETY: `wide` is NUL-terminated and `root` is writable for the
        // length passed; both outlive the call.
        let found =
            unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) };
        if found == 0 {
            return Err(io::Error::last_os_error());
        }
        let (mut sectors, mut sector_bytes, mut free, mut total) = (0u32, 0u32, 0u32, 0u32);
        // SAFETY: `root` holds the NUL-terminated volume path written above,
        // and the four outputs are live locals.
        let measured = unsafe {
            GetDiskFreeSpaceW(
                root.as_ptr(),
                &mut sectors,
                &mut sector_bytes,
                &mut free,
                &mut total,
            )
        };
        if measured == 0 {
            return Err(io::Error::last_os_error());
        }
        let cluster = u64::from(sectors) * u64::from(sector_bytes);
        Ok((
            cluster.saturating_mul(u64::from(total)),
            cluster.saturating_mul(u64::from(free)),
        ))
    }
}
