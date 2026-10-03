use super::{
    DeviceIdentity, Fingerprint, PairingError, Result, SERVER_NAME, SecretBytes,
    identity::validate_certificate, offer::StoredOffer,
};
use rustls::pki_types::CertificateDer;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
};
use vw_model::DeviceId;
use zeroize::Zeroizing;

pub const MAX_TRUST_STATE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PEERS: usize = 64;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerRecord {
    pub(crate) device: DeviceId,
    pub(crate) certificate: Vec<u8>,
    pub(crate) paired_at_ms: u64,
    pub(crate) revoked_at_ms: Option<u64>,
}
impl PeerRecord {
    pub fn device(&self) -> &DeviceId {
        &self.device
    }
    pub fn fingerprint(&self) -> Result<Fingerprint> {
        Fingerprint::certificate(&self.certificate)
    }
    pub const fn paired_at_ms(&self) -> u64 {
        self.paired_at_ms
    }
    pub const fn revoked_at_ms(&self) -> Option<u64> {
        self.revoked_at_ms
    }
    pub const fn is_revoked(&self) -> bool {
        self.revoked_at_ms.is_some()
    }
}
impl fmt::Debug for PeerRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PeerRecord([REDACTED])")
    }
}

/// Protected, app-level state. No project/storage API accepts this type.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustState {
    pub(crate) schema: u32,
    pub(crate) revision: u64,
    pub(crate) identity: DeviceIdentity,
    pub(crate) peers: BTreeMap<DeviceId, PeerRecord>,
    pub(crate) offer: Option<StoredOffer>,
    pub(crate) failures: u8,
    pub(crate) locked_until_ms: u64,
    pub(crate) clock_watermark_ms: u64,
}
impl TrustState {
    pub fn new(identity: DeviceIdentity) -> Result<Self> {
        identity.validate()?;
        Ok(Self {
            schema: 1,
            revision: 1,
            identity,
            peers: BTreeMap::new(),
            offer: None,
            failures: 0,
            locked_until_ms: 0,
            clock_watermark_ms: 0,
        })
    }
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    pub fn identity(&self) -> DeviceIdentity {
        self.identity.clone()
    }
    pub fn peers(&self) -> impl Iterator<Item = &PeerRecord> {
        self.peers.values()
    }
    pub const fn locked_until_ms(&self) -> u64 {
        self.locked_until_ms
    }
    /// Plaintext exists only in memory while the platform adapter encrypts it.
    pub fn encode_for_protection(&self) -> Result<SecretBytes> {
        self.validate()?;
        let bytes = SecretBytes(serde_json::to_vec(self).map_err(|_| PairingError::Storage)?);
        if bytes.0.len() > MAX_TRUST_STATE_BYTES {
            return Err(PairingError::Capacity);
        }
        Ok(bytes)
    }
    pub fn decode_unprotected(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_TRUST_STATE_BYTES {
            return Err(PairingError::Storage);
        }
        let state: Self = serde_json::from_slice(bytes).map_err(|_| PairingError::Storage)?;
        state.validate()?;
        Ok(state)
    }
    pub fn trusted_peer(&self, device: &DeviceId) -> Result<crate::carrier::TrustedPeer> {
        let peer = self
            .peers
            .get(device)
            .filter(|p| !p.is_revoked())
            .ok_or(PairingError::Untrusted)?;
        Ok(crate::carrier::TrustedPeer {
            device: peer.device.clone(),
            certificate: CertificateDer::from(peer.certificate.clone()),
            server_name: SERVER_NAME.into(),
        })
    }
    pub fn authorize(&self, device: &DeviceId, certificate: &[u8]) -> Result<()> {
        let peer = self
            .peers
            .get(device)
            .filter(|p| !p.is_revoked())
            .ok_or(PairingError::Untrusted)?;
        if peer.certificate != certificate {
            return Err(PairingError::Untrusted);
        }
        Ok(())
    }
    pub(crate) fn validate(&self) -> Result<()> {
        if self.schema != 1
            || self.revision == 0
            || self.peers.len() > MAX_PEERS
            || self.failures > 5
        {
            return Err(PairingError::Storage);
        }
        self.identity.validate()?;
        let mut fingerprints = std::collections::BTreeSet::new();
        for (device, peer) in &self.peers {
            if device != &peer.device
                || device == self.identity.device()
                || peer
                    .revoked_at_ms
                    .is_some_and(|time| time < peer.paired_at_ms)
            {
                return Err(PairingError::Storage);
            }
            validate_certificate(&peer.certificate)?;
            if !fingerprints.insert(peer.fingerprint()?.display_hex()) {
                return Err(PairingError::Storage);
            }
        }
        if let Some(offer) = &self.offer {
            offer.validate()?;
        }
        Ok(())
    }
    pub(crate) fn advance_clock(&mut self, now_ms: u64) -> Result<()> {
        if now_ms < self.clock_watermark_ms {
            return Err(PairingError::ClockRollback);
        }
        self.clock_watermark_ms = now_ms;
        Ok(())
    }
    pub(crate) fn pin(&mut self, device: DeviceId, certificate: &[u8], now_ms: u64) -> Result<()> {
        validate_certificate(certificate)?;
        if device == self.identity.device || certificate == self.identity.certificate {
            return Err(PairingError::Authentication);
        }
        if let Some(previous) = self.peers.get(&device) {
            if previous.is_revoked() || previous.certificate != certificate {
                return Err(PairingError::Untrusted);
            }
            return Ok(());
        }
        if self.peers.len() >= MAX_PEERS {
            return Err(PairingError::Capacity);
        }
        if self.peers.values().any(|p| p.certificate == certificate) {
            return Err(PairingError::Untrusted);
        }
        self.peers.insert(
            device.clone(),
            PeerRecord {
                device,
                certificate: certificate.to_vec(),
                paired_at_ms: now_ms,
                revoked_at_ms: None,
            },
        );
        Ok(())
    }
}
impl fmt::Debug for TrustState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TrustState([REDACTED])")
    }
}

/// Successful CAS must be durable before returning. Failure must preserve the
/// previous state. Implementations must serialize across processes, not only threads.
pub trait TrustStore: Send + Sync {
    fn load(&self) -> Result<TrustState>;
    fn compare_exchange(&self, expected_revision: u64, replacement: &TrustState) -> Result<()>;
}
pub(crate) fn update<T>(
    store: &dyn TrustStore,
    mut change: impl FnMut(&mut TrustState) -> Result<T>,
) -> Result<T> {
    for _ in 0..16 {
        let mut state = store.load()?;
        let expected = state.revision;
        let result = change(&mut state)?;
        state.revision = expected.checked_add(1).ok_or(PairingError::Capacity)?;
        state.validate()?;
        match store.compare_exchange(expected, &state) {
            Ok(()) => return Ok(result),
            Err(PairingError::Conflict) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(PairingError::Conflict)
}

pub struct InMemoryTrustStore(Mutex<TrustState>);
impl InMemoryTrustStore {
    /// Non-durable adapter for tests and the explicitly ephemeral phone CLI only.
    pub fn new(identity: DeviceIdentity) -> Result<Self> {
        Ok(Self(Mutex::new(TrustState::new(identity)?)))
    }
    pub fn from_state(state: TrustState) -> Result<Self> {
        state.validate()?;
        Ok(Self(Mutex::new(state)))
    }
}
impl TrustStore for InMemoryTrustStore {
    fn load(&self) -> Result<TrustState> {
        Ok(self.0.lock().map_err(|_| PairingError::Storage)?.clone())
    }
    fn compare_exchange(&self, expected: u64, replacement: &TrustState) -> Result<()> {
        replacement.validate()?;
        let mut state = self.0.lock().map_err(|_| PairingError::Storage)?;
        if state.revision != expected {
            return Err(PairingError::Conflict);
        }
        if replacement.revision != expected.checked_add(1).ok_or(PairingError::Capacity)?
            || replacement.identity.device != state.identity.device
            || replacement.identity.certificate != state.identity.certificate
        {
            return Err(PairingError::Storage);
        }
        *state = replacement.clone();
        Ok(())
    }
}
impl fmt::Debug for InMemoryTrustStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InMemoryTrustStore([REDACTED])")
    }
}

/// Callback-shaped interface for UniFFI/T1.07 and the T1.10 Android adapter.
/// The Android implementation MUST unwrap/wrap with its Keystore-backed key,
/// atomically CAS the revision, fsync before success, bound input to 2 MiB, and
/// clear plaintext buffers after use. These are app-private bytes, never project
/// content. No unprotected filesystem fallback is provided here.
pub trait TrustStoreCallback: Send + Sync {
    fn load_app_state(&self) -> Result<Option<Vec<u8>>>;
    fn compare_exchange_app_state(
        &self,
        expected_revision: u64,
        replacement: Vec<u8>,
    ) -> Result<bool>;
}
pub struct CallbackTrustStore {
    callback: Arc<dyn TrustStoreCallback>,
}
impl CallbackTrustStore {
    pub fn open(callback: Arc<dyn TrustStoreCallback>) -> Result<Self> {
        Self::open_for(callback, None)
    }
    pub fn open_for(
        callback: Arc<dyn TrustStoreCallback>,
        device: Option<DeviceId>,
    ) -> Result<Self> {
        let store = Self { callback };
        let existing = store.callback.load_app_state()?.map(Zeroizing::new);
        if existing.is_none() {
            let identity = match &device {
                Some(id) => DeviceIdentity::generate_for(id.clone())?,
                None => DeviceIdentity::generate()?,
            };
            let state = TrustState::new(identity)?;
            let encoded = state.encode_for_protection()?;
            let _ = store
                .callback
                .compare_exchange_app_state(0, encoded.expose().to_vec())?;
        }
        // Re-read even after a lost initial CAS; racing installations may not
        // silently replace the caller's already persisted project author ID.
        let state = store.load()?;
        if device
            .as_ref()
            .is_some_and(|id| id != state.identity.device())
        {
            return Err(PairingError::Authentication);
        }
        Ok(store)
    }
}
impl TrustStore for CallbackTrustStore {
    fn load(&self) -> Result<TrustState> {
        let bytes = Zeroizing::new(
            self.callback
                .load_app_state()?
                .ok_or(PairingError::Storage)?,
        );
        TrustState::decode_unprotected(&bytes)
    }
    fn compare_exchange(&self, expected: u64, replacement: &TrustState) -> Result<()> {
        if replacement.revision != expected.checked_add(1).ok_or(PairingError::Capacity)? {
            return Err(PairingError::Storage);
        }
        let bytes = replacement.encode_for_protection()?;
        if !self
            .callback
            .compare_exchange_app_state(expected, bytes.expose().to_vec())?
        {
            return Err(PairingError::Conflict);
        }
        Ok(())
    }
}
impl fmt::Debug for CallbackTrustStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CallbackTrustStore([REDACTED])")
    }
}
