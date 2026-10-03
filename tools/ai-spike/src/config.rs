use crate::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub mask_support: String,
    pub max_long_edge: u32,
    pub min_pixels: u64,
    pub max_pixels: u64,
    pub size_multiple: u32,
    pub max_aspect_ratio: u32,
    pub formats: Vec<String>,
    pub max_image_bytes_exclusive: usize,
    pub max_mask_bytes_exclusive: usize,
    pub experimental_above_pixels: u64,
}

impl Capabilities {
    pub fn valid_size(&self, width: u32, height: u32) -> bool {
        let pixels = u64::from(width) * u64::from(height);
        self.size_multiple > 0
            && width > 0
            && height > 0
            && width <= self.max_long_edge
            && height <= self.max_long_edge
            && width.is_multiple_of(self.size_multiple)
            && height.is_multiple_of(self.size_multiple)
            && pixels >= self.min_pixels
            && pixels <= self.max_pixels
            && u64::from(width.max(height))
                <= u64::from(width.min(height)) * u64::from(self.max_aspect_ratio)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prices {
    pub text_input_usd_per_million: u64,
    pub image_input_usd_per_million: u64,
    pub image_output_usd_per_million: u64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Tokens {
    pub text_input: u64,
    pub image_input: u64,
    pub image_output: u64,
}

impl Prices {
    /// USD per million tokens equals micro-USD per token; integer arithmetic.
    pub fn cost(&self, tokens: Tokens) -> Result<u64> {
        [
            (tokens.text_input, self.text_input_usd_per_million),
            (tokens.image_input, self.image_input_usd_per_million),
            (tokens.image_output, self.image_output_usd_per_million),
        ]
        .into_iter()
        .try_fold(0u64, |total, (count, price)| {
            total
                .checked_add(
                    count
                        .checked_mul(price)
                        .ok_or(Error::Limit("cost arithmetic"))?,
                )
                .ok_or(Error::Limit("cost arithmetic"))
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub schema: u32,
    pub verified_on: String,
    pub endpoint: String,
    pub model: String,
    pub model_ids: Vec<String>,
    pub quality: String,
    pub qualities: Vec<String>,
    pub capabilities: Capabilities,
    pub prices: Prices,
    pub token_estimate: Option<Tokens>,
    pub spike_budget_microusd: u64,
    pub daily_soft_budget_microusd: u64,
    pub timeout_seconds: u32,
    pub max_response_bytes: usize,
    pub rate_limits: serde_json::Value,
    pub sources: Vec<String>,
    pub uncertainties: Vec<String>,
}

impl ProviderConfig {
    pub fn bundled() -> Result<Self> {
        Self::from_json(include_bytes!("../config/openai-2026-10-02.json"))
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 65_536 {
            return Err(Error::Limit("configuration bytes"));
        }
        let config: Self = serde_json::from_slice(bytes)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        let caps = &self.capabilities;
        if self.schema != 1
            || self.verified_on.len() != 10
            || self.endpoint != "https://api.openai.com/v1/images/edits"
            || !self.model_ids.contains(&self.model)
            || self.model.is_empty()
            || self.model.len() > 100
            || !self
                .model
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'.')
            || !self.qualities.contains(&self.quality)
            || !["low", "medium", "high", "xhigh", "max"].contains(&self.quality.as_str())
            || caps.mask_support != "alpha_png_guidance"
            || caps.size_multiple == 0
            || caps.size_multiple > 3840
            || caps.max_long_edge == 0
            || caps.max_long_edge > 3840
            || caps.min_pixels == 0
            || caps.max_pixels < caps.min_pixels
            || caps.max_pixels > 8_294_400
            || caps.max_aspect_ratio == 0
            || caps.max_aspect_ratio > 3
            || caps.max_image_bytes_exclusive == 0
            || caps.max_image_bytes_exclusive > 50_000_000
            || caps.max_mask_bytes_exclusive == 0
            || caps.max_mask_bytes_exclusive > 4_000_000
            || self.spike_budget_microusd == 0
            || self.spike_budget_microusd > 2_000_000
            || self.daily_soft_budget_microusd == 0
            || !(1..=300).contains(&self.timeout_seconds)
            || self.max_response_bytes == 0
            || self.max_response_bytes > 100_000_000
            || self.prices.text_input_usd_per_million == 0
            || self.prices.image_input_usd_per_million == 0
            || self.prices.image_output_usd_per_million == 0
        {
            return Err(Error::Invalid("provider configuration"));
        }
        if let Some(tokens) = self.token_estimate {
            if tokens.text_input == 0 || tokens.image_input == 0 || tokens.image_output == 0 {
                return Err(Error::Invalid("nonzero token estimate required"));
            }
            self.prices.cost(tokens)?;
        }
        Ok(())
    }

    pub fn estimate(&self) -> Result<Option<u64>> {
        self.token_estimate
            .map(|tokens| self.prices.cost(tokens))
            .transpose()
    }
}
