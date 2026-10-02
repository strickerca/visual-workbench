use crate::ModelError;
use serde::{Deserialize, Serialize};

/// A lexicographic base-62 fractional index, with no trailing zero digit.
/// Equal keys are ordered by the stable entity ID, allowing concurrent inserts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct OrderKey(String);

const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const MAX_LENGTH: usize = 128;

impl OrderKey {
    /// Create a key strictly between optional bounds. An exhausted 128-digit
    /// interval returns an error so the caller can explicitly rebalance it.
    pub fn between(left: Option<&Self>, right: Option<&Self>) -> Result<Self, ModelError> {
        if left.zip(right).is_some_and(|(l, r)| l >= r) {
            return Err(ModelError::Invalid("order bounds"));
        }
        let lower = left.map_or(&[][..], |v| v.0.as_bytes());
        let upper = right.map(|v| v.0.as_bytes());
        let mut upper_open = upper.is_none();
        let mut out = String::new();
        for index in 0..MAX_LENGTH {
            let lo = lower.get(index).map_or(0, |b| digit(*b));
            let hi = if upper_open {
                62
            } else {
                upper.and_then(|v| v.get(index)).map_or(0, |b| digit(*b))
            };
            if hi > lo + 1 {
                out.push(char::from(DIGITS[(lo + (hi - lo) / 2) as usize]));
                return Ok(Self(out));
            }
            out.push(char::from(DIGITS[lo as usize]));
            if lo < hi {
                upper_open = true;
            }
        }
        Err(ModelError::Invalid("exhausted order interval"))
    }
    /// The validated persisted key.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for OrderKey {
    type Error = ModelError;
    fn try_from(text: String) -> Result<Self, Self::Error> {
        if text.is_empty()
            || text.len() > MAX_LENGTH
            || text.ends_with('0')
            || text.bytes().any(|b| !DIGITS.contains(&b))
        {
            return Err(ModelError::Invalid("fractional order key"));
        }
        Ok(Self(text))
    }
}
impl From<OrderKey> for String {
    fn from(value: OrderKey) -> Self {
        value.0
    }
}
fn digit(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'A'..=b'Z' => byte - b'A' + 10,
        _ => byte - b'a' + 36,
    }
}
