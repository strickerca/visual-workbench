//! App-level pairing. Unauthenticated pairing channels cannot become normal
//! carriers: successful pairing persists trust, then reconnects with pinned TLS.
mod identity;
mod offer;
mod pake;
mod protocol;
mod store;
mod tls;

pub use identity::{DeviceIdentity, Fingerprint, SERVER_NAME, SecretBytes};
pub(crate) use offer::validate_endpoints;
pub use offer::{CodeDisplay, LOCKOUT_MS, PAIRING_LIFETIME_MS, PairingManager, QrPayload};
pub use protocol::{ClientPairing, FingerprintConfirmation, PairingHost, PendingPairing};
pub use store::{
    CallbackTrustStore, InMemoryTrustStore, MAX_TRUST_STATE_BYTES, PeerRecord, TrustState,
    TrustStore, TrustStoreCallback,
};
pub use tls::{PairingChannel, PairingListener};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PairingError {
    #[error("invalid pairing field: {0}")]
    Invalid(&'static str),
    #[error("pairing authentication failed")]
    Authentication,
    #[error("pairing offer expired")]
    Expired,
    #[error("pairing offer was already used or replaced")]
    Used,
    #[error("pairing is temporarily locked")]
    LockedOut,
    #[error("pairing clock moved backwards")]
    ClockRollback,
    #[error("peer is unknown or revoked")]
    Untrusted,
    #[error("app trust store is full")]
    Capacity,
    #[error("app trust store changed concurrently")]
    Conflict,
    #[error("protected app trust storage failed")]
    Storage,
    #[error("secure random generation failed")]
    Entropy,
    #[error("bounded pairing operation timed out")]
    Timeout,
    #[error("pairing connection failed")]
    Connection,
    #[error("fingerprint confirmation was declined")]
    Declined,
}
pub type Result<T> = std::result::Result<T, PairingError>;

pub(crate) fn entropy<const N: usize>() -> Result<[u8; N]> {
    use ring::rand::SecureRandom;
    let mut bytes = [0; N];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| PairingError::Entropy)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests;
