//! Preserved-original AVIF admission and installed-OS decode boundary.
//! No codec is downloaded, no lossy proxy is retained, and refused variants
//! never reach an OS decoder. The first adapter accepts opaque 8-bit, identity
//! orientation, single-item AV1 images; other AVIF features fail explicitly.
mod av1;
mod container;
#[cfg(windows)]
mod windows;
pub use container::{inspect, is_avif};
use vw_model::AssetId;
#[cfg(windows)]
pub use windows::decode;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid AVIF container or bitstream metadata")]
    Invalid,
    #[error("AVIF feature is not preserved by this OS adapter")]
    Unsupported,
    #[error("installed OS AVIF decoder is unavailable")]
    CodecUnavailable,
    #[error("AVIF source depth requires a higher-depth OS adapter")]
    Depth,
    #[error("AVIF color profile cannot be preserved by this OS adapter")]
    Color,
    #[error("AVIF orientation/crop requires an admitted transform adapter")]
    Orientation,
    #[error("AVIF memory or pixel limit exceeded")]
    Limit,
    #[error("AVIF decoder capacity exhausted")]
    Busy,
    #[error("AVIF decoding cancelled")]
    Cancelled,
    #[error("OS AVIF decoder rejected the source")]
    Decode,
}
pub type Result<T> = std::result::Result<T, Error>;
pub const MAX_ENCODED: usize = 64 * 1024 * 1024;
pub const MAX_ICC: usize = 4 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Nclx {
    pub primaries: u16,
    pub transfer: u16,
    pub matrix: u16,
    pub full_range: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub source_asset: AssetId,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub orientation: u8,
    /// Exact bytes from the original colr/prof or colr/rICC property.
    pub icc: Option<Vec<u8>>,
    /// Exact container declaration, matched to the admitted sequence header.
    pub nclx: Option<Nclx>,
    pub encoded_bytes: u64,
}
impl Header {
    /// Counts source retention/callback copies, RGBA copies, native intermediate
    /// decode surfaces and maximum bounded ICC parsing. Not measured process RSS.
    pub fn admit(&self, budget: u64) -> Result<u64> {
        let pixels = u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .ok_or(Error::Limit)?;
        if self.width == 0
            || self.height == 0
            || self.width > 32768
            || self.height > 32768
            || pixels > 50_000_000
            || budget > 512 * 1024 * 1024
        {
            return Err(Error::Limit);
        }
        let coded =
            u64::from(self.width.div_ceil(128) * 128) * u64::from(self.height.div_ceil(128) * 128);
        let estimate = coded
            .checked_mul(64)
            .and_then(|n| n.checked_add(self.encoded_bytes.checked_mul(4)?))
            .and_then(|n| n.checked_add(32 * 1024 * 1024))
            .ok_or(Error::Limit)?;
        if estimate > budget {
            return Err(Error::Limit);
        }
        Ok(estimate)
    }
}
pub struct Decoded {
    pub header: Header,
    pub rgba: Vec<u8>,
}
impl std::fmt::Debug for Decoded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoded")
            .field("width", &self.header.width)
            .field("height", &self.header.height)
            .field("bit_depth", &self.header.bit_depth)
            .finish_non_exhaustive()
    }
}
impl Decoded {
    pub fn validate(&self, expected: &Header, budget: u64) -> Result<()> {
        expected.admit(budget)?;
        let length = u64::from(expected.width) * u64::from(expected.height) * 4;
        if &self.header != expected
            || self.rgba.len() as u64 != length
            || self.rgba.as_chunks::<4>().0.iter().any(|p| p[3] != 255)
        {
            return Err(Error::Decode);
        }
        Ok(())
    }
}
