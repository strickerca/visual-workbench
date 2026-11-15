use crate::ModelError;
use serde::{Deserialize, Serialize};

/// A lexicographic base-62 fractional index, with no trailing zero digit.
/// Equal keys are ordered by the stable entity ID, allowing concurrent inserts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct OrderKey(String);

const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const MAX_LENGTH: usize = 128;
const TOP: u8 = 61;
const MIDDLE: u8 = 31;
/// Number of leading end digits (`z` when appending, `0` when prepending) for
/// which an open-ended key is still bisected exactly as the first version of
/// this type did. Keys created by that version therefore stay reproducible.
const BISECTED_RUN: usize = 1;
/// Past the bisected run an open-ended key is the neighbour's value plus or
/// minus one in this many base-62 digits after the run of end digits. One run
/// length therefore holds 61 * 62^3 (about 14.5 million) sequential keys, where
/// bisecting held six.
const COUNTER_DIGITS: usize = 4;

impl OrderKey {
    /// Create a key strictly between optional bounds.
    ///
    /// With one bound absent the result is the next key in a sequence, so
    /// repeatedly appending after the last key (or prepending before the first)
    /// keeps keys short: `V k s w y z z0001 z0002 …`. With both bounds present
    /// the first differing digit is bisected.
    ///
    /// The only failures are equal or reversed bounds, two bounds that are
    /// adjacent at the full 128 digits, and an open end whose bound is already
    /// the largest (128 `z`) or smallest (127 `0` then `1`) representable key.
    /// No key exists in those intervals, so the caller has to give the
    /// neighbouring entities new keys before it can insert there.
    pub fn between(left: Option<&Self>, right: Option<&Self>) -> Result<Self, ModelError> {
        let digits = match (left, right) {
            (None, None) => vec![MIDDLE],
            (Some(left), None) => after(&values(left)),
            (None, Some(right)) => before(&values(right)),
            (Some(left), Some(right)) => {
                if left >= right {
                    return Err(ModelError::Invalid("order bounds"));
                }
                inside(&values(left), &values(right))
            }
        };
        if digits.is_empty() {
            return Err(ModelError::Invalid("exhausted order interval"));
        }
        Ok(Self(
            digits
                .into_iter()
                .map(|digit| char::from(DIGITS[usize::from(digit)]))
                .collect(),
        ))
    }
    /// The validated persisted key.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn values(key: &OrderKey) -> Vec<u8> {
    key.0.bytes().map(digit).collect()
}

/// The next key above `left` with no upper bound; empty when none exists.
fn after(left: &[u8]) -> Vec<u8> {
    let run = left.iter().take_while(|digit| **digit == TOP).count();
    if run < BISECTED_RUN {
        let low = left.get(run).copied().unwrap_or(0);
        let mut out = vec![TOP; run];
        out.push(low + (TOP + 1 - low) / 2);
        return out;
    }
    let depth = (run + COUNTER_DIGITS).min(MAX_LENGTH);
    let mut out = left
        .iter()
        .copied()
        .chain(std::iter::repeat(0))
        .take(depth)
        .collect::<Vec<_>>();
    // Dropping digits past `depth` only lowers the value, so adding one in the
    // last kept place is strictly above `left`.
    let mut carried = true;
    for digit in out.iter_mut().rev() {
        if *digit == TOP {
            *digit = 0;
        } else {
            *digit += 1;
            carried = false;
            break;
        }
    }
    if carried {
        // Every one of the 128 digits was the top digit.
        return Vec::new();
    }
    trimmed(out)
}

/// The next key below `right` with no lower bound; empty when none exists.
fn before(right: &[u8]) -> Vec<u8> {
    let run = right.iter().take_while(|digit| **digit == 0).count();
    // A valid key has a nonzero last digit, so a digit follows its zero run.
    let Some(&high) = right.get(run) else {
        return Vec::new();
    };
    if run < BISECTED_RUN {
        let mut out = vec![0; run];
        if high > 1 {
            out.push(high / 2);
        } else {
            out.extend([0, MIDDLE]);
        }
        return out;
    }
    let depth = (run + COUNTER_DIGITS).min(MAX_LENGTH);
    if right.len() > depth {
        // The dropped tail is nonzero, so the kept prefix is already lower.
        return trimmed(right[..depth].to_vec());
    }
    let mut out = right.to_vec();
    out.resize(depth, 0);
    for digit in out.iter_mut().rev() {
        if *digit == 0 {
            *digit = TOP;
        } else {
            *digit -= 1;
            break;
        }
    }
    // All zero only when `right` was one in the 128th place, the smallest key.
    trimmed(out)
}

/// Bisect the first place where two ordered bounds leave room for a digit.
fn inside(left: &[u8], right: &[u8]) -> Vec<u8> {
    let mut upper_open = false;
    let mut out = Vec::new();
    for index in 0..MAX_LENGTH {
        let low = left.get(index).copied().unwrap_or(0);
        let high = if upper_open {
            TOP + 1
        } else {
            right.get(index).copied().unwrap_or(0)
        };
        if high > low + 1 {
            out.push(low + (high - low) / 2);
            return out;
        }
        out.push(low);
        if low < high {
            upper_open = true;
        }
    }
    Vec::new()
}

fn trimmed(mut digits: Vec<u8>) -> Vec<u8> {
    while digits.last() == Some(&0) {
        digits.pop();
    }
    digits
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
