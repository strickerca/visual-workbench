use crate::{Error, MAX_EDGE, MAX_ENCODED, MAX_PIXELS, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskSupport {
    None,
    AlphaPngGuidance,
    AlphaPngExact,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Png,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenAiImageEdits,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub mask_support: MaskSupport,
    pub max_long_edge: u32,
    pub min_pixels: u64,
    pub max_pixels: u64,
    pub size_multiple: u32,
    /// Supported range is [1/max_aspect_ratio,max_aspect_ratio].
    pub max_aspect_ratio: u32,
    pub formats: Vec<Format>,
    pub max_image_bytes_exclusive: usize,
    pub max_mask_bytes_exclusive: usize,
    pub experimental_above_pixels: u64,
}
impl Capabilities {
    pub fn validate(&self) -> Result<()> {
        if self.size_multiple == 0
            || self.max_long_edge == 0
            || self.max_long_edge > MAX_EDGE
            || self.size_multiple > self.max_long_edge
            || self.min_pixels == 0
            || self.max_pixels < self.min_pixels
            || self.max_pixels > MAX_PIXELS
            || self.max_aspect_ratio == 0
            || self.max_aspect_ratio > 16
            || self.formats != [Format::Png]
            || self.max_image_bytes_exclusive == 0
            || self.max_image_bytes_exclusive > MAX_ENCODED
            || self.max_mask_bytes_exclusive == 0
            || self.max_mask_bytes_exclusive > MAX_ENCODED
            || self.experimental_above_pixels == 0
        {
            return Err(Error::Invalid("provider capabilities"));
        }
        let grid = u64::from(self.max_long_edge / self.size_multiple);
        if grid * grid > 262_144 {
            return Err(Error::Limit("provider size search"));
        }
        Ok(())
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tokens {
    pub text_input: u64,
    pub image_input: u64,
    pub image_output: u64,
}
/// Rates are micro-USD per million tokens, so fractional USD prices are exactly
/// expressible. The summed rational charge is rounded upward once to micro-USD.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prices {
    pub text_input_microusd_per_million: u64,
    pub image_input_microusd_per_million: u64,
    pub image_output_microusd_per_million: u64,
}
impl Prices {
    pub fn cost(self, tokens: Tokens) -> Result<u64> {
        let total = [
            (tokens.text_input, self.text_input_microusd_per_million),
            (tokens.image_input, self.image_input_microusd_per_million),
            (tokens.image_output, self.image_output_microusd_per_million),
        ]
        .into_iter()
        .try_fold(0u128, |sum, (n, rate)| {
            sum.checked_add(u128::from(n) * u128::from(rate))
                .ok_or(Error::Limit("cost arithmetic"))
        })?;
        u64::try_from(total.div_ceil(1_000_000)).map_err(|_| Error::Limit("cost arithmetic"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub schema: u32,
    pub kind: ProviderKind,
    pub verified_on: String,
    pub endpoint: String,
    pub model: String,
    pub quality: String,
    pub capabilities: Capabilities,
    pub prices: Prices,
    pub timeout_seconds: u32,
    pub max_response_bytes: usize,
}
impl ProviderConfig {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 65_536 {
            return Err(Error::Limit("provider configuration bytes"));
        }
        let value: Self = serde_json::from_slice(bytes)?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        self.capabilities.validate()?;
        let identifier = |value: &str| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        };
        if self.schema != 1
            || self.verified_on.len() != 10
            || !self.verified_on.bytes().enumerate().all(|(i, b)| {
                if i == 4 || i == 7 {
                    b == b'-'
                } else {
                    b.is_ascii_digit()
                }
            })
            || !identifier(&self.model)
            || !identifier(&self.quality)
            || !(1..=300).contains(&self.timeout_seconds)
            || self.max_response_bytes == 0
            || self.max_response_bytes > 100_000_000
            || self.prices.text_input_microusd_per_million == 0
            || self.prices.image_input_microusd_per_million == 0
            || self.prices.image_output_microusd_per_million == 0
        {
            return Err(Error::Invalid("provider configuration"));
        }
        match self.kind {
            ProviderKind::OpenAiImageEdits => {
                if self.endpoint != "https://api.openai.com/v1/images/edits"
                    || self.capabilities.mask_support != MaskSupport::AlphaPngGuidance
                {
                    return Err(Error::Unsupported);
                }
            }
        }
        Ok(())
    }
    pub fn fingerprint(&self) -> Result<String> {
        self.validate()?;
        Ok(crate::sha256(&crate::canonical(self)?))
    }
}
