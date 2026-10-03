//! Bounded T0.11 image-edit foundation. Live API acceptance remains separate.

pub mod budget;
pub mod config;
pub mod geometry;
pub mod pixels;
pub mod platform;
pub mod proof;
pub mod request;

use sha2::{Digest, Sha256};

/// Maximum decoded document area for this memory-bounded measurement CLI.
pub const MAX_PIXELS: u64 = 20_000_000;
pub const MAX_EDGE: u32 = 16_384;
pub const MAX_FEATHER: u32 = 64;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    Invalid(&'static str),
    #[error("resource limit: {0}")]
    Limit(&'static str),
    #[error("file operation failed")]
    Io(#[from] std::io::Error),
    #[error("invalid JSON")]
    Json(#[from] serde_json::Error),
    #[error("image codec rejected the input")]
    Image(#[from] image::ImageError),
    #[error("color profile conversion failed")]
    Color,
    #[error("exterior proof failed: {0} changed pixels")]
    Exterior(u64),
    #[error("explicit request confirmation and a reviewed estimate are required")]
    Confirmation,
    #[error(
        "this reservation already consumed its single attempt; keep any uncertain charge reserved"
    )]
    AttemptConsumed,
    #[error("cost reservation exceeds the spike budget")]
    Budget,
    #[error("credential unavailable or invalid; configure Windows Credential Manager locally")]
    Credential,
    #[error("live transport is unavailable on this platform")]
    UnsupportedPlatform,
    #[error("network request failed; charge may be unresolved; no retry was made")]
    Transport,
    #[error("provider returned HTTP {0}; no retry was made")]
    Http(u32),
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|byte| {
            [
                char::from(DIGITS[usize::from(byte >> 4)]),
                char::from(DIGITS[usize::from(byte & 15)]),
            ]
        })
        .collect()
}

pub fn pixel_count(width: u32, height: u32) -> Result<usize> {
    let count = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE || count > MAX_PIXELS {
        return Err(Error::Limit("document dimensions"));
    }
    usize::try_from(count).map_err(|_| Error::Limit("address space"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn sha256_text_encoding_matches_published_known_answers() {
        assert_eq!(
            super::sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            super::sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
