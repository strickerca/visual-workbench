//! OS credential adapters and application-owned Android verifier initialization.
//! There is no environment, project-file, command-line or arbitrary-key fallback.

#[cfg(target_os = "android")]
pub mod android;
#[cfg(windows)]
pub mod windows;

use std::time::Instant;
use vw_ai_provider::CredentialFailure;

pub type Result<T> = std::result::Result<T, CredentialFailure>;
pub(crate) fn check(stop: &dyn vw_ai::Cancellation, deadline: Instant) -> Result<()> {
    if stop.is_cancelled() || Instant::now() >= deadline {
        Err(CredentialFailure::Cancelled)
    } else {
        Ok(())
    }
}
pub(crate) fn validate(bytes: &[u8], max: usize) -> Result<()> {
    if bytes.is_empty() || bytes.len() > max || !bytes.iter().all(u8::is_ascii_graphic) {
        return Err(CredentialFailure::Invalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credential_admission_has_no_string_or_header_injection_fallback() {
        for value in [
            &b""[..],
            &b"space key"[..],
            &b"key\r\nheader"[..],
            &[0xff][..],
        ] {
            assert_eq!(validate(value, 2560), Err(CredentialFailure::Invalid));
        }
        assert!(validate(&vec![b'x'; 2560], 2560).is_ok());
        assert_eq!(
            validate(&vec![b'x'; 2561], 2560),
            Err(CredentialFailure::Invalid)
        );
    }
    #[test]
    fn cancellation_and_deadline_admission_are_explicit() {
        let stop = std::sync::atomic::AtomicBool::new(true);
        assert_eq!(
            check(&stop, Instant::now()),
            Err(CredentialFailure::Cancelled)
        );
        assert_eq!(
            check(&vw_ai::NeverCancel, Instant::now()),
            Err(CredentialFailure::Cancelled)
        );
    }
}
