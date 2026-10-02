use crate::ModelError;
use serde::{Deserialize, Serialize};
use std::fmt;
use vw_proto::v1;

/// Canonical lowercase UUIDv7 for projects, documents, objects and transactions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Id(String);

impl Id {
    /// Construct UUIDv7 from a 48-bit Unix millisecond timestamp and 80 random bits.
    /// Version/variant consume six entropy bits; fixtures may supply fixed entropy.
    pub fn from_parts(unix_ms: u64, entropy: [u8; 10]) -> Result<Self, ModelError> {
        if unix_ms >= (1u64 << 48) {
            return Err(ModelError::InvalidId("UUIDv7 timestamp"));
        }
        let mut bytes = [0u8; 16];
        bytes[..6].copy_from_slice(&unix_ms.to_be_bytes()[2..]);
        bytes[6..].copy_from_slice(&entropy);
        bytes[6] = (bytes[6] & 0x0f) | 0x70;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Self::from_bytes(bytes)
    }
    /// Generate a UUIDv7 using the OS random source and the caller's wall clock.
    /// Clock rollback affects ordering only; 74 random bits preserve uniqueness.
    pub fn generate(unix_ms: u64) -> Result<Self, ModelError> {
        let mut entropy = [0u8; 10];
        getrandom::fill(&mut entropy).map_err(|_| ModelError::Entropy)?;
        Self::from_parts(unix_ms, entropy)
    }
    /// Validate version 7 and the RFC variant before accepting 16 wire bytes.
    pub fn from_bytes(bytes: [u8; 16]) -> Result<Self, ModelError> {
        if bytes[6] >> 4 != 7 || bytes[8] >> 6 != 2 {
            return Err(ModelError::InvalidId("UUIDv7"));
        }
        let mut text = String::with_capacity(36);
        for (index, byte) in bytes.iter().enumerate() {
            if matches!(index, 4 | 6 | 8 | 10) {
                text.push('-');
            }
            text.push(HEX[(byte >> 4) as usize] as char);
            text.push(HEX[(byte & 15) as usize] as char);
        }
        Ok(Self(text))
    }
    /// Parse and validate a required protobuf UUID.
    pub fn from_proto(value: Option<&v1::Uuid>) -> Result<Self, ModelError> {
        let value = value.ok_or(ModelError::Missing("UUID"))?;
        let bytes: [u8; 16] = value
            .value
            .as_slice()
            .try_into()
            .map_err(|_| ModelError::InvalidId("UUIDv7"))?;
        Self::from_bytes(bytes)
    }
    /// Stable 16-byte protobuf representation of an already validated UUID.
    pub fn to_proto(&self) -> v1::Uuid {
        let mut value = Vec::with_capacity(16);
        let digits: Vec<u8> = self.0.bytes().filter(|b| *b != b'-').collect();
        for pair in digits.as_chunks::<2>().0 {
            value.push(hex_value(pair[0]) * 16 + hex_value(pair[1]));
        }
        v1::Uuid { value }
    }
    /// Canonical UUID string, suitable for persistent keys.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Id {
    type Error = ModelError;
    fn try_from(text: String) -> Result<Self, Self::Error> {
        let bytes = text.as_bytes();
        if bytes.len() != 36
            || bytes.iter().enumerate().any(|(i, b)| {
                if matches!(i, 8 | 13 | 18 | 23) {
                    *b != b'-'
                } else {
                    !HEX.contains(b)
                }
            })
            || bytes[14] != b'7'
            || !b"89ab".contains(&bytes[19])
        {
            return Err(ModelError::InvalidId("UUIDv7"));
        }
        Ok(Self(text))
    }
}

/// Install-random 128-bit device identity encoded as 26 canonical Crockford digits.
/// It must never be derived from a hardware, account or network identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DeviceId(String);

impl DeviceId {
    /// Encode entropy directly; fixed inputs are useful only for synthetic fixtures.
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        let mut value = u128::from_be_bytes(bytes);
        let mut digits = [b'0'; 26];
        for digit in digits.iter_mut().rev() {
            *digit = CROCKFORD[(value & 31) as usize];
            value >>= 5;
        }
        Self(digits.into_iter().map(char::from).collect())
    }
    /// Generate an install identity from OS randomness, without a fallback source.
    pub fn generate() -> Result<Self, ModelError> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| ModelError::Entropy)?;
        Ok(Self::from_bytes(bytes))
    }
    /// Canonical uppercase Crockford representation (aliases are not accepted).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for DeviceId {
    type Error = ModelError;
    fn try_from(text: String) -> Result<Self, Self::Error> {
        let bytes = text.as_bytes();
        if bytes.len() != 26
            || !b"01234567".contains(&bytes[0])
            || bytes.iter().any(|b| !CROCKFORD.contains(b))
        {
            return Err(ModelError::InvalidId("DeviceId"));
        }
        Ok(Self(text))
    }
}

/// BLAKE3-256 of immutable original bytes, represented by 64 lowercase hex digits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AssetId(String);

/// BLAKE3-256 of canonical visible project state; host sequence is separate.
pub type StateHash = AssetId;

impl AssetId {
    /// Hash bytes without changing them. Blob verification/storage is a separate layer.
    pub fn hash(bytes: &[u8]) -> Self {
        Self(blake3::hash(bytes).to_hex().to_string())
    }
    /// Canonical lowercase content-address key.
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Bytes for protobuf revision fields.
    pub fn bytes(&self) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        for (output, pair) in bytes.iter_mut().zip(self.0.as_bytes().as_chunks::<2>().0) {
            *output = hex_value(pair[0]) * 16 + hex_value(pair[1]);
        }
        bytes
    }
}

impl TryFrom<String> for AssetId {
    type Error = ModelError;
    fn try_from(text: String) -> Result<Self, Self::Error> {
        if text.len() != 64 || text.bytes().any(|b| !HEX.contains(&b)) {
            return Err(ModelError::InvalidId("BLAKE3"));
        }
        Ok(Self(text))
    }
}

macro_rules! string_id {
    ($kind:ty) => {
        impl From<$kind> for String {
            fn from(value: $kind) -> Self {
                value.0
            }
        }
        impl fmt::Display for $kind {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}
string_id!(Id);
string_id!(DeviceId);
string_id!(AssetId);

const HEX: &[u8; 16] = b"0123456789abcdef";
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
// Called only on bytes already validated by the private identifier constructors.
fn hex_value(byte: u8) -> u8 {
    if byte <= b'9' {
        byte - b'0'
    } else {
        byte - b'a' + 10
    }
}
