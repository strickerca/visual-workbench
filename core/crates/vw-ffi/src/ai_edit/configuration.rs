use super::{AiConfiguration, AiEditError, AiEditResult, AiTokenEstimate};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use vw_ai::config::{ProviderConfig, Tokens};

pub(super) const MAX_CONFIG: usize = 65_536;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Configuration {
    pub schema: u32,
    pub provider: ProviderConfig,
    pub expires_on: String,
    pub provenance: String,
    pub daily_soft_budget_microusd: u64,
}
impl Configuration {
    pub fn parse(bytes: &[u8]) -> AiEditResult<Self> {
        if bytes.is_empty() || bytes.len() > MAX_CONFIG {
            return Err(AiEditError::Invalid);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| AiEditError::Invalid)?;
        value.provider.validate().map_err(AiEditError::from)?;
        let from = day(&value.provider.verified_on)?;
        let until = day(&value.expires_on)?;
        if value.schema != 1
            || until < from
            || until - from > 366
            || value.provenance.is_empty()
            || value.provenance.len() > 4096
            || value.provenance.chars().any(|c| c == '\0' || c == '\r')
            || value.daily_soft_budget_microusd == 0
            || value.daily_soft_budget_microusd > 1_000_000_000
        {
            return Err(AiEditError::Invalid);
        }
        Ok(value)
    }
    pub fn json(&self) -> AiEditResult<String> {
        serde_json::to_string(self).map_err(|_| AiEditError::Invalid)
    }
    pub fn fingerprint(&self) -> AiEditResult<String> {
        Ok(vw_ai::sha256(self.json()?.as_bytes()))
    }
    pub fn current(&self, today: u32) -> AiEditResult<bool> {
        Ok(day(&self.provider.verified_on)? <= today && today <= day(&self.expires_on)?)
    }
    pub fn describe(&self) -> AiEditResult<AiConfiguration> {
        Ok(AiConfiguration {
            json: self.json()?,
            fingerprint: self.fingerprint()?,
            verified_on: self.provider.verified_on.clone(),
            expires_on: self.expires_on.clone(),
            model: self.provider.model.clone(),
            quality: self.provider.quality.clone(),
            daily_soft_budget_microusd: self.daily_soft_budget_microusd,
        })
    }
}
pub(super) fn estimate(value: &AiTokenEstimate, today: u32) -> AiEditResult<Tokens> {
    let start = day(&value.verified_on)?;
    let end = day(&value.expires_on)?;
    if value.schema != 1
        || start > today
        || end < today
        || end < start
        || end - start > 31
        || value.provenance.is_empty()
        || value.provenance.len() > 1024
        || value.provenance.chars().any(|c| c.is_control())
        || value.text_input > 100_000_000
        || value.image_input == 0
        || value.image_input > 100_000_000
        || value.image_output == 0
        || value.image_output > 100_000_000
    {
        return Err(AiEditError::Estimate);
    }
    Ok(Tokens {
        text_input: value.text_input,
        image_input: value.image_input,
        image_output: value.image_output,
    })
}
pub(super) fn today() -> AiEditResult<u32> {
    u32::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| AiEditError::Invalid)?
            .as_secs()
            / 86_400,
    )
    .map_err(|_| AiEditError::Invalid)
}
/// Gregorian civil date to UTC day, without local timezone or permissive parsing.
pub(super) fn day(value: &str) -> AiEditResult<u32> {
    let b = value.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || b.iter()
            .enumerate()
            .any(|(i, c)| i != 4 && i != 7 && !c.is_ascii_digit())
    {
        return Err(AiEditError::Invalid);
    }
    let number = |s: &[u8]| s.iter().fold(0i64, |n, b| n * 10 + i64::from(b - b'0'));
    let mut y = number(&b[..4]);
    let m = number(&b[5..7]);
    let d = number(&b[8..]);
    if !(1970..=9999).contains(&y) || !(1..=12).contains(&m) {
        return Err(AiEditError::Invalid);
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let count = match m {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if d < 1 || d > count {
        return Err(AiEditError::Invalid);
    }
    y -= i64::from(m <= 2);
    let era = y / 400;
    let year = y - era * 400;
    let adjusted = m + if m > 2 { -3 } else { 9 };
    let days = era * 146_097 + year * 365 + year / 4 - year / 100 + (153 * adjusted + 2) / 5 + d
        - 1
        - 719_468;
    u32::try_from(days).map_err(|_| AiEditError::Invalid)
}

#[uniffi::export]
pub fn default_ai_configuration() -> AiEditResult<AiConfiguration> {
    Configuration::parse(include_bytes!("provider-2026-10-02.json"))?.describe()
}
#[uniffi::export]
pub fn validate_ai_configuration(json: String) -> AiEditResult<AiConfiguration> {
    Configuration::parse(json.as_bytes())?.describe()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn dates_are_strict_and_expiry_is_inclusive() {
        assert_eq!(day("1970-01-01").unwrap(), 0);
        assert_eq!(day("2000-02-29").unwrap() + 1, day("2000-03-01").unwrap());
        for s in ["2025-02-29", "2026-04-31", "2026-1-01", "1969-12-31"] {
            assert!(day(s).is_err());
        }
        let c = Configuration::parse(include_bytes!("provider-2026-10-02.json")).unwrap();
        assert!(c.current(day("2026-10-03").unwrap()).unwrap());
        assert!(!c.current(day("2026-11-03").unwrap()).unwrap());
    }
    #[test]
    fn dated_default_uses_preserved_rates_and_exact_ceiling_arithmetic() {
        let c = Configuration::parse(include_bytes!("provider-2026-10-02.json")).unwrap();
        assert_eq!(
            c.provider
                .prices
                .cost(Tokens {
                    text_input: 1,
                    image_input: 1,
                    image_output: 1
                })
                .unwrap(),
            43
        );
        assert_eq!(
            c.provider
                .prices
                .cost(Tokens {
                    text_input: 100,
                    image_input: 1000,
                    image_output: 2000
                })
                .unwrap(),
            68_500
        );
        assert_eq!(c.provider.model, "gpt-image-2.5-sunburst-2026-09-08");
    }
    #[test]
    fn estimate_is_explicit_dated_data_not_a_guessed_token_formula() {
        let mut value = AiTokenEstimate {
            schema: 1,
            text_input: 100,
            image_input: 1000,
            image_output: 2000,
            provenance: "Owner reviewed request estimate v1".into(),
            verified_on: "2026-10-03".into(),
            expires_on: "2026-10-03".into(),
        };
        assert!(estimate(&value, day("2026-10-03").unwrap()).is_ok());
        assert!(estimate(&value, day("2026-10-04").unwrap()).is_err());
        value.image_output = 0;
        assert!(estimate(&value, day("2026-10-03").unwrap()).is_err());
    }
}
