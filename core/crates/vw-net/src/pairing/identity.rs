use super::{PairingError, Result, entropy};
use ring::digest;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Serialize};
use std::fmt;
use vw_model::DeviceId;
use zeroize::Zeroize;

pub const SERVER_NAME: &str = "vworkbench.local";
pub(crate) const MAX_CERTIFICATE_BYTES: usize = 16 * 1024;

/// Secret-bearing bytes deliberately do not expose their contents through Debug.
/// Serialization is for the protected app store or an explicitly handled QR only.
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretBytes(pub(crate) Vec<u8>);
impl SecretBytes {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}
impl Drop for SecretBytes {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}
impl fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretBytes([REDACTED])")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint([u8; 32]);
impl Fingerprint {
    pub fn certificate(der: &[u8]) -> Result<Self> {
        validate_certificate(der)?;
        let hash = digest::digest(&digest::SHA256, der);
        let bytes = hash
            .as_ref()
            .try_into()
            .map_err(|_| PairingError::Authentication)?;
        Ok(Self(bytes))
    }
    pub fn display_hex(&self) -> String {
        const DIGITS: &[u8] = b"0123456789abcdef";
        let mut text = String::with_capacity(64);
        for byte in self.0 {
            text.push(char::from(DIGITS[usize::from(byte >> 4)]));
            text.push(char::from(DIGITS[usize::from(byte & 15)]));
        }
        text
    }
    pub fn parse(text: &str) -> Result<Self> {
        if text.len() != 64 {
            return Err(PairingError::Invalid("fingerprint"));
        }
        let mut bytes = [0; 32];
        for (index, pair) in text.as_bytes().as_chunks::<2>().0.iter().enumerate() {
            let digit = |b| match b {
                b'0'..=b'9' => Ok(b - b'0'),
                b'a'..=b'f' => Ok(b - b'a' + 10),
                _ => Err(PairingError::Invalid("fingerprint")),
            };
            bytes[index] = digit(pair[0])? * 16 + digit(pair[1])?;
        }
        Ok(Self(bytes))
    }
}
impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Fingerprint([REDACTED])")
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceIdentity {
    pub(crate) device: DeviceId,
    pub(crate) certificate: Vec<u8>,
    pub(crate) key: SecretBytes,
}
impl DeviceIdentity {
    pub fn generate() -> Result<Self> {
        Self::generate_for(DeviceId::from_bytes(entropy()?))
    }
    /// Adopt an existing installation ID only when provisioning a new protected
    /// identity. This never changes a persisted certificate or project history.
    pub fn generate_for(device: DeviceId) -> Result<Self> {
        let key = rcgen::KeyPair::generate().map_err(|_| PairingError::Entropy)?;
        let params = rcgen::CertificateParams::new(vec![SERVER_NAME.into()])
            .map_err(|_| PairingError::Authentication)?;
        let certificate = params
            .self_signed(&key)
            .map_err(|_| PairingError::Authentication)?;
        let identity = Self {
            device,
            certificate: certificate.der().to_vec(),
            key: SecretBytes(key.serialize_der()),
        };
        identity.validate()?;
        Ok(identity)
    }
    pub fn device(&self) -> &DeviceId {
        &self.device
    }
    pub fn certificate(&self) -> &[u8] {
        &self.certificate
    }
    pub fn fingerprint(&self) -> Result<Fingerprint> {
        Fingerprint::certificate(&self.certificate)
    }
    pub fn transport_identity(&self) -> Result<crate::carrier::Identity> {
        self.validate()?;
        Ok(crate::carrier::Identity {
            certificate: CertificateDer::from(self.certificate.clone()),
            key: PrivatePkcs8KeyDer::from(self.key.0.clone()).into(),
        })
    }
    pub(crate) fn validate(&self) -> Result<()> {
        validate_certificate(&self.certificate)?;
        if self.key.0.is_empty() || self.key.0.len() > MAX_CERTIFICATE_BYTES {
            return Err(PairingError::Invalid("private key"));
        }
        let key = PrivatePkcs8KeyDer::from(self.key.0.clone()).into();
        let certified = rustls::sign::CertifiedKey::from_der(
            vec![CertificateDer::from(self.certificate.clone())],
            key,
            &rustls::crypto::ring::default_provider(),
        )
        .map_err(|_| PairingError::Authentication)?;
        certified
            .keys_match()
            .map_err(|_| PairingError::Authentication)
    }
}
impl fmt::Debug for DeviceIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DeviceIdentity([REDACTED])")
    }
}
pub(crate) fn validate_certificate(der: &[u8]) -> Result<()> {
    if der.is_empty() || der.len() > MAX_CERTIFICATE_BYTES {
        return Err(PairingError::Invalid("certificate"));
    }
    rustls::server::ParsedCertificate::try_from(&CertificateDer::from(der))
        .map_err(|_| PairingError::Authentication)?;
    Ok(())
}
