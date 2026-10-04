//! The only Win32 unsafe boundary. Tests never call the operating-system backend.
use crate::{Result, check, validate};
use std::{ptr::NonNull, time::Instant};
use vw_ai_provider::{CredentialFailure, Secret, SecretProvider};
use windows_sys::Win32::{
    Foundation::{ERROR_NOT_FOUND, GetLastError},
    Security::Credentials::{
        CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW,
        CredDeleteW, CredFree, CredReadW, CredWriteW,
    },
};
use zeroize::{Zeroize, Zeroizing};

pub const MAX_WINDOWS_KEY_BYTES: usize = CRED_MAX_CREDENTIAL_BLOB_SIZE as usize;
const TARGET: &str = "VisualWorkbench/OpenAI/APIKey/v1";

/// Fixed current-user target. LOCAL_MACHINE persistence means subsequent logons
/// of this same user on this machine, not another user's credentials or roaming.
#[derive(Default)]
pub struct WindowsCredentials;
impl WindowsCredentials {
    /// Blocking OS mutation; callers must use their bounded settings worker.
    /// Cancellation after the OS call may mean the explicit save already took
    /// effect. Never report that such cancellation rolled storage back.
    pub fn save(
        &self,
        mut bytes: Zeroizing<Vec<u8>>,
        stop: &dyn vw_ai::Cancellation,
        deadline: Instant,
    ) -> Result<()> {
        check(stop, deadline)?;
        validate(&bytes, MAX_WINDOWS_KEY_BYTES)?;
        let mut target = target();
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: bytes.len() as u32,
            CredentialBlob: bytes.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        check(stop, deadline)?;
        // SAFETY: the fixed target is terminated UTF-16; all record fields are
        // initialized and the owned bounded blob/target outlive this call.
        let written = unsafe { CredWriteW(&credential, 0) };
        if written == 0 {
            return Err(CredentialFailure::Unavailable);
        }
        Ok(())
    }
    pub fn remove(&self, stop: &dyn vw_ai::Cancellation, deadline: Instant) -> Result<()> {
        check(stop, deadline)?;
        let target = target();
        // SAFETY: target is our fixed, terminated UTF-16 credential name. This
        // never enumerates or deletes another credential's target.
        let removed = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
        if removed == 0 {
            // SAFETY: reads the calling thread's last Win32 failure only.
            if unsafe { GetLastError() } != ERROR_NOT_FOUND {
                return Err(CredentialFailure::Unavailable);
            }
        }
        Ok(())
    }
    /// Explicit settings/readiness action. Decrypted bytes are dropped and wiped;
    /// no key suffix, prefix, fingerprint or raw OS error is returned for logging.
    pub fn is_configured(&self, stop: &dyn vw_ai::Cancellation, deadline: Instant) -> Result<bool> {
        match self.load(stop, deadline) {
            Ok(secret) => {
                drop(secret);
                Ok(true)
            }
            Err(CredentialFailure::Missing) => Ok(false),
            Err(other) => Err(other),
        }
    }
}
impl SecretProvider for WindowsCredentials {
    fn load(&self, stop: &dyn vw_ai::Cancellation, deadline: Instant) -> Result<Secret> {
        check(stop, deadline)?;
        let target = target();
        let mut pointer = std::ptr::null_mut();
        // SAFETY: fixed terminated target; pointer is initialized writable output
        // owned by CredReadW on success, then unconditionally freed by OwnedRecord.
        if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut pointer) } == 0 {
            // SAFETY: reads the current thread's immediately preceding OS error.
            return Err(if unsafe { GetLastError() } == ERROR_NOT_FOUND {
                CredentialFailure::Missing
            } else {
                CredentialFailure::Unavailable
            });
        }
        let record = OwnedRecord(NonNull::new(pointer).ok_or(CredentialFailure::Unavailable)?);
        check(stop, deadline)?;
        let bytes = record.copy()?;
        check(stop, deadline)?;
        Secret::from_protected_bytes(bytes).map_err(|_| CredentialFailure::Invalid)
    }
}
fn target() -> Vec<u16> {
    TARGET.encode_utf16().chain(std::iter::once(0)).collect()
}
struct OwnedRecord(NonNull<CREDENTIALW>);
impl OwnedRecord {
    fn copy(&self) -> Result<Zeroizing<Vec<u8>>> {
        // SAFETY: only a successful CredReadW creates OwnedRecord and the OS
        // record remains allocated until this object's Drop.
        copy_record(unsafe { self.0.as_ref() })
    }
}
fn copy_record(record: &CREDENTIALW) -> Result<Zeroizing<Vec<u8>>> {
    let size = record.CredentialBlobSize as usize;
    if record.Type != CRED_TYPE_GENERIC
        || record.Persist != CRED_PERSIST_LOCAL_MACHINE
        || !(1..=MAX_WINDOWS_KEY_BYTES).contains(&size)
        || record.CredentialBlob.is_null()
    {
        return Err(CredentialFailure::Invalid);
    }
    // SAFETY: this private helper receives either the owned OS record or a test
    // record backed by a live array; the reported length is checked before read.
    let source = unsafe { std::slice::from_raw_parts(record.CredentialBlob, size) };
    validate(source, MAX_WINDOWS_KEY_BYTES)?;
    Ok(Zeroizing::new(source.to_vec()))
}
impl Drop for OwnedRecord {
    fn drop(&mut self) {
        // SAFETY: CredReadW owns this record/blob for the declared length. Wipe
        // only the API's bounded blob before freeing the single allocation.
        unsafe {
            let record = self.0.as_ref();
            if !record.CredentialBlob.is_null()
                && record.CredentialBlobSize <= CRED_MAX_CREDENTIAL_BLOB_SIZE
            {
                std::slice::from_raw_parts_mut(
                    record.CredentialBlob,
                    record.CredentialBlobSize as usize,
                )
                .zeroize();
            }
            CredFree(self.0.as_ptr().cast());
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synthetic_record_copy_is_bounded_and_does_not_access_a_store() -> Result<()> {
        let mut source = b"synthetic-only-key".to_vec();
        let record = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            CredentialBlob: source.as_mut_ptr(),
            CredentialBlobSize: source.len() as u32,
            ..Default::default()
        };
        let copied = copy_record(&record)?;
        assert_eq!(&copied[..], &source);
        source.fill(0);
        assert_eq!(&copied[..], b"synthetic-only-key");
        Ok(())
    }
    #[test]
    fn malformed_record_metadata_is_refused_without_dereferencing_bad_blob() {
        for record in [
            CREDENTIALW {
                Type: CRED_TYPE_GENERIC,
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                CredentialBlobSize: 1,
                ..Default::default()
            },
            CREDENTIALW {
                Type: CRED_TYPE_GENERIC,
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                CredentialBlobSize: CRED_MAX_CREDENTIAL_BLOB_SIZE + 1,
                ..Default::default()
            },
            CREDENTIALW::default(),
        ] {
            assert!(matches!(
                copy_record(&record),
                Err(CredentialFailure::Invalid)
            ));
        }
    }
    #[test]
    fn target_is_fixed_namespaced_and_terminated_without_enumeration() {
        let value = target();
        assert_eq!(value.last(), Some(&0));
        assert_eq!(value.iter().filter(|&&v| v == 0).count(), 1);
        assert!(TARGET.starts_with("VisualWorkbench/"));
    }
}
