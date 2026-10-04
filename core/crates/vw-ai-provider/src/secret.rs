use crate::{Error, MAX_SECRET_BYTES, Result};
use std::{fmt, time::Instant};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CredentialFailure {
    #[error("not configured")]
    Missing,
    #[error("protected store unavailable")]
    Unavailable,
    #[error("invalid protected credential")]
    Invalid,
    #[error("cancelled")]
    Cancelled,
}

/// Non-cloneable, non-serializable owned key. The platform should read directly
/// into Zeroizing bytes. No public accessor; Debug is always redacted.
pub struct Secret(Zeroizing<Vec<u8>>);
impl Secret {
    pub fn from_protected_bytes(bytes: Zeroizing<Vec<u8>>) -> Result<Self> {
        if bytes.is_empty()
            || bytes.len() > MAX_SECRET_BYTES
            || !bytes.iter().all(|&b| b.is_ascii_graphic())
        {
            return Err(Error::Credential(CredentialFailure::Invalid));
        }
        Ok(Self(bytes))
    }
    pub(crate) fn authorization(&self) -> Result<reqwest::header::HeaderValue> {
        let mut bytes = Zeroizing::new(Vec::with_capacity(7 + self.0.len()));
        bytes.extend_from_slice(b"Bearer ");
        bytes.extend_from_slice(&self.0);
        let mut header = reqwest::header::HeaderValue::from_bytes(&bytes)
            .map_err(|_| Error::Credential(CredentialFailure::Invalid))?;
        header.set_sensitive(true);
        Ok(header)
    }
}
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}

/// Implement in the trusted platform layer using Windows Credential Manager or
/// Android Keystore protected storage. Never read from project paths, command
/// arguments, environment variables or logs. Called once per consumed attempt.
/// Implementations should honor stop/deadline; an uninterruptible OS call keeps
/// an occupied global worker slot after the caller returns.
pub trait SecretProvider: Send + Sync + 'static {
    fn load(
        &self,
        stop: &dyn vw_ai::Cancellation,
        deadline: Instant,
    ) -> std::result::Result<Secret, CredentialFailure>;
}
