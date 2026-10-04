use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use vw_model::Id;

/// Select from the owner's configured model profile. No model-name guessing or
/// assertion that an installed client preserves image dimensions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "profile", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    Claude {
        model: String,
        tier: ClaudeTier,
    },
    OpenAiResponses {
        model: String,
        max_long_edge: u32,
    },
    CodexLocalImage {
        model: String,
        verified_max_long_edge: u32,
        installed_schema_sha256: String,
    },
    Gemini {
        model: String,
        max_long_edge: u32,
    },
    Generic {
        max_long_edge: u32,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeTier {
    Modern2576,
    Legacy1568,
}
impl Target {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Claude { .. } => "claude",
            Self::OpenAiResponses { .. } | Self::CodexLocalImage { .. } => "openai",
            Self::Gemini { .. } => "gemini",
            Self::Generic { .. } => "generic",
        }
    }
    pub fn max_long_edge(&self) -> u32 {
        match self {
            Self::Claude {
                tier: ClaudeTier::Modern2576,
                ..
            } => 2576,
            Self::Claude {
                tier: ClaudeTier::Legacy1568,
                ..
            } => 1568,
            Self::OpenAiResponses { max_long_edge, .. }
            | Self::Gemini { max_long_edge, .. }
            | Self::Generic { max_long_edge } => *max_long_edge,
            Self::CodexLocalImage {
                verified_max_long_edge,
                ..
            } => *verified_max_long_edge,
        }
    }
    pub fn model(&self) -> Option<&str> {
        match self {
            Self::Claude { model, .. }
            | Self::OpenAiResponses { model, .. }
            | Self::CodexLocalImage { model, .. }
            | Self::Gemini { model, .. } => Some(model),
            Self::Generic { .. } => None,
        }
    }
    pub fn convention(&self) -> &'static str {
        if matches!(self, Self::Gemini { .. }) {
            "normalized_1000_yxyx"
        } else {
            "absolute_px_xywh"
        }
    }
    pub(crate) fn validate(&self) -> Result<()> {
        if !(256..=8192).contains(&self.max_long_edge()) {
            return Err(Error::Limit("target edge"));
        }
        if self
            .model()
            .is_some_and(|m| m.is_empty() || m.len() > 256 || m.chars().any(char::is_control))
        {
            return Err(Error::Invalid("configured model"));
        }
        if let Self::CodexLocalImage {
            installed_schema_sha256,
            ..
        } = self
            && !crate::bounded::hash_valid(installed_schema_sha256)
        {
            return Err(Error::Invalid("Codex verification binding"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub memory_bytes: u64,
    pub source_bytes: usize,
    pub source_pixels: u64,
    pub package_bytes: usize,
    pub image_bytes: usize,
    pub markers: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_bytes: 1024 * 1024 * 1024,
            source_bytes: 64 * 1024 * 1024,
            source_pixels: 50_000_000,
            package_bytes: 128 * 1024 * 1024,
            image_bytes: 32 * 1024 * 1024,
            markers: 64,
        }
    }
}
impl Limits {
    pub(crate) fn validate(self) -> Result<()> {
        if self.memory_bytes > 4 * 1024 * 1024 * 1024
            || self.memory_bytes < 16 * 1024 * 1024
            || self.source_bytes == 0
            || self.source_bytes > 256 * 1024 * 1024
            || self.source_pixels == 0
            || self.source_pixels > 50_000_000
            || self.package_bytes == 0
            || self.package_bytes > 512 * 1024 * 1024
            || self.image_bytes == 0
            || self.image_bytes > self.package_bytes
            || self.markers > 512
        {
            return Err(Error::Limit("configuration"));
        }
        Ok(())
    }
    pub(crate) fn working(self, retained: u64) -> Result<u64> {
        // Reserve the COMPLETE eventual package, geometric Vec capacity slack,
        // and bounded canonical/semantic projections before any codec/clone.
        self.memory_bytes
            .checked_sub(retained)
            .and_then(|n| n.checked_sub(self.package_bytes as u64 * 2))
            .and_then(|n| n.checked_sub(96 * 1024 * 1024))
            .filter(|n| *n > 0)
            .ok_or(Error::Limit("memory"))
    }
}
pub struct CompileOptions {
    pub package_id: Id,
    pub created_at_ms: i64,
    pub target: Target,
    pub semantic_snapshot: Option<Id>,
    /// Explicit Send-preview choice, false by default in calling applications.
    pub include_window_title: bool,
    pub assume_untagged_srgb: bool,
    pub allow_depth_reduction: bool,
    pub limits: Limits,
}

pub(crate) fn timestamp(ms: i64) -> Result<String> {
    if !(0..=253_402_300_799_999).contains(&ms) {
        return Err(Error::Invalid("UTC timestamp"));
    }
    let mut days = ms / 86_400_000;
    let mut year = 1970;
    let leap = |y: i64| y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    loop {
        let count = if leap(year) { 366 } else { 365 };
        if days < count {
            break;
        }
        days -= count;
        year += 1;
    }
    let lengths = [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 0;
    while month < 11 && days >= lengths[month] {
        days -= lengths[month];
        month += 1;
    }
    let seconds = ms / 1000 % 86400;
    Ok(format!(
        "{year:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        month + 1,
        days + 1,
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        ms % 1000
    ))
}
