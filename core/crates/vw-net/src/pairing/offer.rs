use super::{
    DeviceIdentity, Fingerprint, PairingError, Result, SecretBytes, TrustStore, entropy,
    store::update,
};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    net::SocketAddr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use vw_model::DeviceId;
use zeroize::Zeroize;

pub const PAIRING_LIFETIME_MS: u64 = 300_000;
pub const LOCKOUT_MS: u64 = 600_000;
const MAX_QR_BYTES: usize = 4096;

pub(crate) trait PairingClock: Send + Sync {
    fn now_ms(&self) -> Result<u64>;
}
struct SystemClock;
impl PairingClock for SystemClock {
    fn now_ms(&self) -> Result<u64> {
        u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| PairingError::ClockRollback)?
                .as_millis(),
        )
        .map_err(|_| PairingError::ClockRollback)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum OfferKind {
    Qr,
    Code,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredOffer {
    pub nonce: [u8; 16],
    pub kind: OfferKind,
    pub secret: SecretBytes,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub used: bool,
    pub attempt: Option<[u8; 16]>,
}
impl StoredOffer {
    pub fn validate(&self) -> Result<()> {
        if self.expires_at_ms.checked_sub(self.issued_at_ms) != Some(PAIRING_LIFETIME_MS)
            || (self.used && !self.secret.0.is_empty())
            || (!self.used
                && match self.kind {
                    OfferKind::Qr => self.secret.0.len() != 16,
                    OfferKind::Code => {
                        self.secret.0.len() != 8 || !self.secret.0.iter().all(u8::is_ascii_digit)
                    }
                })
        {
            return Err(PairingError::Storage);
        }
        Ok(())
    }
}

/// QR bytes are a credential. Only explicit QR/file handling may serialize them.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QrPayload {
    pub(crate) schema: u32,
    pub(crate) pc_device_id: DeviceId,
    pub(crate) certificate_sha256: Fingerprint,
    pub(crate) endpoints: Vec<SocketAddr>,
    pub(crate) secret: SecretBytes,
    pub(crate) expires_at_ms: u64,
}
impl QrPayload {
    pub fn device(&self) -> &DeviceId {
        &self.pc_device_id
    }
    pub const fn fingerprint(&self) -> &Fingerprint {
        &self.certificate_sha256
    }
    pub fn endpoints(&self) -> &[SocketAddr] {
        &self.endpoints
    }
    pub const fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }
    pub fn encode_for_qr(&self) -> Result<SecretBytes> {
        let encoded =
            SecretBytes(serde_json::to_vec(self).map_err(|_| PairingError::Invalid("QR"))?);
        if encoded.0.len() > MAX_QR_BYTES {
            return Err(PairingError::Invalid("QR size"));
        }
        Ok(encoded)
    }
    pub fn decode_qr(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_QR_BYTES {
            return Err(PairingError::Invalid("QR size"));
        }
        let payload: Self =
            serde_json::from_slice(bytes).map_err(|_| PairingError::Invalid("QR"))?;
        payload.validate(SystemClock.now_ms()?)?;
        Ok(payload)
    }
    pub(crate) fn validate(&self, now_ms: u64) -> Result<()> {
        if self.schema != 1 || self.secret.0.len() != 16 {
            return Err(PairingError::Invalid("QR"));
        }
        validate_endpoints(&self.endpoints)?;
        if now_ms >= self.expires_at_ms {
            return Err(PairingError::Expired);
        }
        if self.expires_at_ms - now_ms > PAIRING_LIFETIME_MS {
            return Err(PairingError::Invalid("QR expiry"));
        }
        Ok(())
    }
}
impl fmt::Debug for QrPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("QrPayload([REDACTED])")
    }
}

pub struct CodeDisplay(SecretBytes);
impl CodeDisplay {
    pub fn parse_for_entry(digits: &str) -> Result<Self> {
        if digits.len() != 8 || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(PairingError::Invalid("8-digit code"));
        }
        Ok(Self(SecretBytes(digits.as_bytes().to_vec())))
    }
    /// UI-only accessor. Never include the returned value in logs or diagnostics.
    pub fn digits_for_display(&self) -> Result<&str> {
        std::str::from_utf8(self.0.expose()).map_err(|_| PairingError::Invalid("8-digit code"))
    }
    pub(crate) fn bytes(&self) -> &[u8] {
        self.0.expose()
    }
    fn generate() -> Result<Self> {
        for _ in 0..16 {
            let random = u32::from_be_bytes(entropy()?);
            // Rejection sampling avoids a bias from reducing all 2^32 values.
            if random >= 4_200_000_000 {
                continue;
            }
            let mut number = random % 100_000_000;
            let mut digits = vec![b'0'; 8];
            for digit in digits.iter_mut().rev() {
                *digit += (number % 10) as u8;
                number /= 10;
            }
            return Ok(Self(SecretBytes(digits)));
        }
        Err(PairingError::Entropy)
    }
}
impl fmt::Debug for CodeDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CodeDisplay([REDACTED])")
    }
}

pub(crate) struct Reservation {
    pub offer: [u8; 16],
    pub attempt: [u8; 16],
    pub secret: SecretBytes,
    pub kind: OfferKind,
}
#[derive(Clone)]
pub struct PairingManager {
    pub(crate) store: Arc<dyn TrustStore>,
    pub(crate) clock: Arc<dyn PairingClock>,
}
impl PairingManager {
    pub fn new(store: Arc<dyn TrustStore>) -> Result<Self> {
        let _ = store.load()?;
        Ok(Self {
            store,
            clock: Arc::new(SystemClock),
        })
    }
    pub fn identity(&self) -> Result<DeviceIdentity> {
        Ok(self.store.load()?.identity())
    }
    /// Build normal pinned TLS with live durable revocation checks. Previously
    /// constructed configs also reject a peer after revoke() commits.
    pub fn pinned_tls(&self, peer: &DeviceId) -> Result<crate::carrier::PinnedTls> {
        let state = self.store.load()?;
        let identity = state.identity().transport_identity()?;
        crate::carrier::PinnedTls::new_authorized(
            identity,
            state.trusted_peer(peer)?,
            Arc::new(LiveTrust(self.store.clone())),
        )
        .map_err(|_| PairingError::Untrusted)
    }
    pub fn issue_qr(&self, endpoints: Vec<SocketAddr>) -> Result<QrPayload> {
        validate_endpoints(&endpoints)?;
        let secret = SecretBytes(entropy::<16>()?.to_vec());
        let offer = self.issue(OfferKind::Qr, secret.clone())?;
        let identity = self.identity()?;
        Ok(QrPayload {
            schema: 1,
            pc_device_id: identity.device.clone(),
            certificate_sha256: identity.fingerprint()?,
            endpoints,
            secret,
            expires_at_ms: offer.expires_at_ms,
        })
    }
    pub fn issue_code(&self) -> Result<CodeDisplay> {
        let code = CodeDisplay::generate()?;
        let _ = self.issue(OfferKind::Code, code.0.clone())?;
        Ok(code)
    }
    fn issue(&self, kind: OfferKind, secret: SecretBytes) -> Result<StoredOffer> {
        let now = self.clock.now_ms()?;
        let offer = StoredOffer {
            nonce: entropy()?,
            kind,
            secret,
            issued_at_ms: now,
            expires_at_ms: now
                .checked_add(PAIRING_LIFETIME_MS)
                .ok_or(PairingError::ClockRollback)?,
            used: false,
            attempt: None,
        };
        update(self.store.as_ref(), |state| {
            state.advance_clock(now)?;
            check_lockout(state, now)?;
            // Displaying another offer never resets the persistent failure budget.
            state.offer = Some(offer.clone());
            Ok(())
        })?;
        Ok(offer)
    }
    pub fn revoke(&self, device: &DeviceId) -> Result<()> {
        let now = self.clock.now_ms()?;
        update(self.store.as_ref(), |state| {
            state.advance_clock(now)?;
            let peer = state.peers.get_mut(device).ok_or(PairingError::Untrusted)?;
            if peer.revoked_at_ms.is_none() {
                peer.revoked_at_ms = Some(now);
            }
            Ok(())
        })
    }
    pub fn is_locked(&self) -> Result<bool> {
        let state = self.store.load()?;
        let now = self.clock.now_ms()?;
        if now < state.clock_watermark_ms {
            return Err(PairingError::ClockRollback);
        }
        Ok(now < state.locked_until_ms)
    }
    /// Reserve the attempt durably BEFORE sending a PAKE response. Interrupted
    /// attempts conservatively spend a guess, so reconnect/restart cannot bypass
    /// lockout. The fifth reservation may still complete successfully; otherwise
    /// every further attempt is locked for ten minutes.
    pub(crate) fn reserve(&self, kind: OfferKind) -> Result<Reservation> {
        let now = self.clock.now_ms()?;
        let attempt = entropy()?;
        update(self.store.as_ref(), |state| {
            state.advance_clock(now)?;
            check_lockout(state, now)?;
            let offer = state.offer.as_mut().ok_or(PairingError::Used)?;
            if offer.used {
                return Err(PairingError::Used);
            }
            if offer.kind != kind {
                return Err(PairingError::Authentication);
            }
            if now >= offer.expires_at_ms {
                return Err(PairingError::Expired);
            }
            offer.attempt = Some(attempt);
            state.failures = state
                .failures
                .checked_add(1)
                .ok_or(PairingError::Capacity)?;
            if state.failures == 5 {
                state.locked_until_ms = now
                    .checked_add(LOCKOUT_MS)
                    .ok_or(PairingError::ClockRollback)?;
            }
            Ok(Reservation {
                offer: offer.nonce,
                attempt,
                secret: offer.secret.clone(),
                kind,
            })
        })
    }
    pub(crate) fn complete(
        &self,
        reservation: &Reservation,
        device: &DeviceId,
        certificate: &[u8],
    ) -> Result<()> {
        let now = self.clock.now_ms()?;
        update(self.store.as_ref(), |state| {
            state.advance_clock(now)?;
            let offer = state.offer.as_ref().ok_or(PairingError::Used)?;
            if offer.used
                || offer.nonce != reservation.offer
                || offer.attempt != Some(reservation.attempt)
                || offer.kind != reservation.kind
            {
                return Err(PairingError::Used);
            }
            if now >= offer.expires_at_ms {
                return Err(PairingError::Expired);
            }
            state.pin(device.clone(), certificate, now)?;
            let offer = state.offer.as_mut().ok_or(PairingError::Used)?;
            offer.used = true;
            offer.secret.0.zeroize();
            offer.secret.0.clear();
            state.failures = 0;
            state.locked_until_ms = 0;
            Ok(())
        })
    }
    pub(crate) fn pin_confirmed_server(&self, device: &DeviceId, certificate: &[u8]) -> Result<()> {
        let now = self.clock.now_ms()?;
        update(self.store.as_ref(), |state| {
            state.advance_clock(now)?;
            state.pin(device.clone(), certificate, now)
        })
    }
}
impl fmt::Debug for PairingManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PairingManager([REDACTED])")
    }
}
struct LiveTrust(Arc<dyn TrustStore>);
impl crate::carrier::PeerAuthorization for LiveTrust {
    fn authorize(&self, device: &DeviceId, certificate: &[u8]) -> crate::Result<()> {
        self.0
            .load()
            .and_then(|state| state.authorize(device, certificate))
            .map_err(|_| crate::NetError::Authentication)
    }
}
fn check_lockout(state: &mut super::TrustState, now: u64) -> Result<()> {
    if now < state.locked_until_ms {
        return Err(PairingError::LockedOut);
    }
    if state.locked_until_ms != 0 {
        state.failures = 0;
        state.locked_until_ms = 0;
    }
    Ok(())
}
pub(crate) fn validate_endpoints(endpoints: &[SocketAddr]) -> Result<()> {
    if endpoints.is_empty() || endpoints.len() > 8 || endpoints.iter().any(|endpoint| {
        endpoint.port() == 0 || endpoint.ip().is_unspecified() || endpoint.ip().is_multicast() ||
        matches!(endpoint, SocketAddr::V4(v4) if v4.ip().is_broadcast()) ||
        matches!(endpoint, SocketAddr::V6(v6) if v6.ip().is_unicast_link_local() && v6.scope_id() == 0)
    }) { return Err(PairingError::Invalid("pairing endpoints")); }
    Ok(())
}
