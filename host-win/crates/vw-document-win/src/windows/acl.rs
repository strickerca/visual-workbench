use super::{Handle, last_error};
use crate::{Error, Result};
use std::{mem::size_of, ptr};
use windows_sys::Win32::{
    Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, LocalFree},
    Security::{
        ACL,
        Authorization::{
            EXPLICIT_ACCESS_W, GRANT_ACCESS, GetSecurityInfo, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT,
            SetEntriesInAclW, SetSecurityInfo, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
        },
        DACL_SECURITY_INFORMATION, GetSecurityDescriptorControl, NO_INHERITANCE,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
        UNPROTECTED_DACL_SECURITY_INFORMATION,
    },
    Storage::FileSystem::{FILE_GENERIC_EXECUTE, FILE_GENERIC_READ},
    System::Threading::GetCurrentProcess,
};

struct Snapshot {
    descriptor: PSECURITY_DESCRIPTOR,
    acl: *mut ACL,
    bytes: Vec<u8>,
    protected: bool,
}
// SAFETY: allocation/pointers are owned, immutable and released only by Drop.
unsafe impl Send for Snapshot {}
impl Snapshot {
    fn read(handle: &Handle) -> Result<Self> {
        let mut descriptor = ptr::null_mut();
        let mut acl = ptr::null_mut();
        // SAFETY: writable outputs and a READ_CONTROL retained file handle.
        let result = unsafe {
            GetSecurityInfo(
                handle.0,
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut acl,
                ptr::null_mut(),
                &mut descriptor,
            )
        };
        if result != 0 {
            return Err(Error::Windows(result));
        }
        let mut out = Self {
            descriptor,
            acl,
            bytes: Vec::new(),
            protected: false,
        };
        if descriptor.is_null() || acl.is_null() {
            return Err(Error::Integrity);
        }
        let mut control = 0_u16;
        let mut revision = 0_u32;
        // SAFETY: OS-returned descriptor owns acl for its full lifetime.
        if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
            return Err(last_error());
        }
        out.protected = control & SE_DACL_PROTECTED != 0;
        out.bytes = acl_bytes(acl)?;
        Ok(out)
    }
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        /* SAFETY: GetSecurityInfo allocation, including its ACL. */
        unsafe {
            LocalFree(self.descriptor);
        }
    }
}
fn acl_bytes(acl: *const ACL) -> Result<Vec<u8>> {
    if acl.is_null() {
        return Err(Error::Integrity);
    }
    // SAFETY: only called with OS-returned live ACL allocations.
    let length = unsafe { (*acl).AclSize } as usize;
    if length < size_of::<ACL>() || length > 65_535 {
        return Err(Error::Integrity);
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| Error::Limit)?;
    // SAFETY: AclSize is the allocated length returned by the trusted OS API.
    bytes.extend_from_slice(unsafe { std::slice::from_raw_parts(acl.cast::<u8>(), length) });
    Ok(bytes)
}

pub(super) struct Grant {
    handle: Handle,
    original: Snapshot,
    replacement: *mut ACL,
    expected: Vec<u8>,
    attempted: bool,
    restored: bool,
}
// SAFETY: exclusive owner of duplicated handle and two ACL allocations.
unsafe impl Send for Grant {}
impl Grant {
    /// Pure preparation. The caller retains this object BEFORE invoking apply,
    /// so even a partial/uncertain OS mutation has a recoverable owner.
    pub(super) fn prepare(handle: &Handle, sid: PSID) -> Result<Self> {
        let mut duplicate = ptr::null_mut();
        // SAFETY: private noninheritable duplicate, same access as retained pin.
        if unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                handle.0,
                GetCurrentProcess(),
                &mut duplicate,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        } == 0
        {
            return Err(last_error());
        }
        let handle = Handle::checked(duplicate)?;
        let original = Snapshot::read(&handle)?;
        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: NO_INHERITANCE,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: ptr::null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_UNKNOWN,
                ptstrName: sid.cast(),
            },
        };
        let mut replacement = ptr::null_mut();
        // SAFETY: immutable old ACL and fresh profile SID; produces an owned ACL.
        let result = unsafe { SetEntriesInAclW(1, &entry, original.acl, &mut replacement) };
        if result != 0 {
            return Err(Error::Windows(result));
        }
        let mut grant = Self {
            handle,
            original,
            replacement,
            expected: Vec::new(),
            attempted: false,
            restored: false,
        };
        grant.expected = acl_bytes(replacement)?;
        Ok(grant)
    }
    pub(super) fn apply(&mut self) -> Result<()> {
        apply_owned(
            &self.original.bytes,
            self.original.protected,
            &self.expected,
            &mut self.attempted,
            &mut NativeAcl {
                handle: &self.handle,
                original: &self.original,
                replacement: self.replacement,
            },
        )
    }
    pub(super) fn restore(&mut self) -> Result<()> {
        restore_owned(
            &self.original.bytes,
            self.original.protected,
            &self.expected,
            self.attempted,
            &mut self.restored,
            &mut NativeAcl {
                handle: &self.handle,
                original: &self.original,
                replacement: self.replacement,
            },
        )
    }
}
impl Drop for Grant {
    fn drop(&mut self) {
        /* SAFETY: SetEntriesInAcl allocated exactly this buffer. */
        unsafe {
            LocalFree(self.replacement.cast());
        }
    }
}

/// The transition never learns ownership from a post-write descriptor. The
/// intended replacement is fixed during preparation and stays immutable.
trait AclAccess {
    fn read(&mut self) -> Result<(Vec<u8>, bool)>;
    fn write_replacement(&mut self) -> Result<()>;
    fn write_original(&mut self) -> Result<()>;
}
struct NativeAcl<'a> {
    handle: &'a Handle,
    original: &'a Snapshot,
    replacement: *mut ACL,
}
impl AclAccess for NativeAcl<'_> {
    fn read(&mut self) -> Result<(Vec<u8>, bool)> {
        let mut value = Snapshot::read(self.handle)?;
        Ok((std::mem::take(&mut value.bytes), value.protected))
    }
    fn write_replacement(&mut self) -> Result<()> {
        // SAFETY: exact retained handle and prepared OS-owned ACL allocation.
        // No ancestor, network capability or arbitrary path receives access.
        let result = unsafe {
            SetSecurityInfo(
                self.handle.0,
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                self.replacement,
                ptr::null(),
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(Error::Windows(result))
        }
    }
    fn write_original(&mut self) -> Result<()> {
        let flags = DACL_SECURITY_INFORMATION
            | if self.original.protected {
                PROTECTED_DACL_SECURITY_INFORMATION
            } else {
                UNPROTECTED_DACL_SECURITY_INFORMATION
            };
        // SAFETY: restores this owner's saved OS allocation through its retained
        // handle only after the transition verifies the current owned value.
        let result = unsafe {
            SetSecurityInfo(
                self.handle.0,
                SE_FILE_OBJECT,
                flags,
                ptr::null_mut(),
                ptr::null_mut(),
                self.original.acl,
                ptr::null(),
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(Error::Windows(result))
        }
    }
}
fn same(value: &(Vec<u8>, bool), bytes: &[u8], protected: bool) -> bool {
    value.0 == bytes && value.1 == protected
}
fn apply_owned(
    original: &[u8],
    original_protected: bool,
    expected: &[u8],
    attempted: &mut bool,
    io: &mut dyn AclAccess,
) -> Result<()> {
    if *attempted {
        return Err(Error::Invalid);
    }
    // Refuse an already observable third-party edit before attempting mutation.
    if !same(&io.read()?, original, original_protected) {
        return Err(Error::CleanupPending);
    }
    *attempted = true;
    io.write_replacement()?;
    // SetSecurityInfo does not reorder ACEs. Any different readback, including
    // canonicalization we cannot prove, is an unowned descriptor and fails closed.
    if !same(&io.read()?, expected, true) {
        return Err(Error::CleanupPending);
    }
    Ok(())
}
fn restore_owned(
    original: &[u8],
    original_protected: bool,
    expected: &[u8],
    attempted: bool,
    restored: &mut bool,
    io: &mut dyn AclAccess,
) -> Result<()> {
    if !attempted || *restored {
        return Ok(());
    }
    let current = io.read()?;
    if same(&current, original, original_protected) {
        *restored = true;
        return Ok(());
    }
    if !same(&current, expected, true) {
        return Err(Error::CleanupPending);
    }
    io.write_original().map_err(|_| Error::CleanupPending)?;
    if !same(&io.read()?, original, original_protected) {
        return Err(Error::CleanupPending);
    }
    *restored = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    struct ScriptedAcl {
        current: (Vec<u8>, bool),
        after_replacement: Option<(Vec<u8>, bool)>,
        replacements: u32,
        restores: u32,
    }
    impl AclAccess for ScriptedAcl {
        fn read(&mut self) -> Result<(Vec<u8>, bool)> {
            Ok(self.current.clone())
        }
        fn write_replacement(&mut self) -> Result<()> {
            self.replacements += 1;
            self.current = self
                .after_replacement
                .take()
                .unwrap_or_else(|| (vec![2], true));
            Ok(())
        }
        fn write_original(&mut self) -> Result<()> {
            self.restores += 1;
            self.current = (vec![1], false);
            Ok(())
        }
    }
    fn fixture() -> ScriptedAcl {
        ScriptedAcl {
            current: (vec![1], false),
            after_replacement: None,
            replacements: 0,
            restores: 0,
        }
    }
    #[test]
    fn post_write_unrelated_acl_is_never_adopted_or_overwritten_during_cleanup() {
        let mut io = fixture();
        io.after_replacement = Some((vec![9], true));
        let (mut attempted, mut restored) = (false, false);
        assert!(matches!(
            apply_owned(&[1], false, &[2], &mut attempted, &mut io),
            Err(Error::CleanupPending)
        ));
        assert!(attempted);
        assert_eq!(io.replacements, 1);
        assert!(matches!(
            restore_owned(&[1], false, &[2], attempted, &mut restored, &mut io),
            Err(Error::CleanupPending)
        ));
        assert!(!restored);
        assert_eq!(io.restores, 0);
        assert_eq!(io.current, (vec![9], true));
    }
    #[test]
    fn intervening_pre_write_edit_refuses_without_any_setter_or_cleanup_overwrite() {
        let mut io = fixture();
        io.current = (vec![7], false);
        let (mut attempted, mut restored) = (false, false);
        assert!(matches!(
            apply_owned(&[1], false, &[2], &mut attempted, &mut io),
            Err(Error::CleanupPending)
        ));
        assert!(!attempted);
        restore_owned(&[1], false, &[2], attempted, &mut restored, &mut io).unwrap();
        assert_eq!((io.replacements, io.restores), (0, 0));
        assert_eq!(io.current, (vec![7], false));
    }
    #[test]
    fn exact_owned_replacement_restores_once_and_keeps_original_protection_mode() {
        let mut io = fixture();
        let (mut attempted, mut restored) = (false, false);
        apply_owned(&[1], false, &[2], &mut attempted, &mut io).unwrap();
        restore_owned(&[1], false, &[2], attempted, &mut restored, &mut io).unwrap();
        restore_owned(&[1], false, &[2], attempted, &mut restored, &mut io).unwrap();
        assert!(restored);
        assert_eq!((io.replacements, io.restores), (1, 1));
        assert_eq!(io.current, (vec![1], false));
    }
}
