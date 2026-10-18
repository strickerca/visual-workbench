use crate::{Error, Result};
use vw_model::AssetId;
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Format { Bmp, Tiff }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Notice { FirstTiffPageOnly, UntaggedColor }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub format: Format, pub source_asset: AssetId, pub width: u32, pub height: u32,
    pub bit_depth: u8, pub orientation: u8, pub alpha: bool, pub icc: Option<Vec<u8>>,
    pub encoded_bytes: u64, pub notices: Vec<Notice>, pub estimated_peak_bytes: u64,
    pub(crate) samples: u8, pub(crate) planar: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits { pub encoded_bytes: u64, pub output_bytes: u64, pub pixels: u64, pub memory_bytes: u64 }
impl Default for Limits { fn default() -> Self { Self { encoded_bytes: 64*1024*1024, output_bytes: 512*1024*1024, pixels: 50_000_000, memory_bytes: 512*1024*1024 } } }
impl Limits {
    pub fn validate(self) -> Result<Self> {
        if self.encoded_bytes == 0 || self.encoded_bytes > 256*1024*1024 || self.output_bytes == 0 || self.output_bytes > 2*1024*1024*1024
            || self.pixels == 0 || self.pixels > 50_000_000 || self.memory_bytes < 64*1024*1024 || self.memory_bytes > 1024*1024*1024 { return Err(Error::Limit); } Ok(self)
    }
    pub(crate) fn input(self, bytes: usize) -> Result<()> { self.validate()?; if bytes == 0 || bytes as u64 > self.encoded_bytes || bytes as u64 > self.memory_bytes { Err(Error::Limit) } else { Ok(()) } }
    pub(crate) fn admit(self, width: u32, height: u32, encoded: u64, icc: usize) -> Result<u64> {
        self.validate()?;
        let pixels=u64::from(width).checked_mul(u64::from(height)).ok_or(Error::Limit)?;
        if width==0 || height==0 || width>65535 || height>65535 || pixels>self.pixels || icc>4*1024*1024 { return Err(Error::Limit); }
        // Source/callback storage, native decode buffer, RGBA expansion,
        // orientation copy, bounded 16MiB ICC parse and TIFF metadata/workspace.
        let peak=pixels.checked_mul(32).and_then(|n|n.checked_add(encoded.checked_mul(2)?)).and_then(|n|n.checked_add((icc as u64).checked_mul(4)?)).and_then(|n|n.checked_add(64*1024*1024)).ok_or(Error::Limit)?;
        if peak>self.memory_bytes { Err(Error::Limit) } else { Ok(peak) }
    }
}
pub(crate) fn profile(bytes: &[u8]) -> Result<()> {
    if bytes.len()<132 || bytes.len()>4*1024*1024 || bytes.get(36..40)!=Some(b"acsp") || !matches!(bytes.get(16..20),Some(b"RGB "|b"GRAY"))
        || u32::from_be_bytes(bytes[..4].try_into().map_err(|_|Error::Color)?) as usize != bytes.len() { return Err(Error::Color); } Ok(())
}
pub(crate) fn range(bytes:&[u8], at:u64, len:u64)->Result<&[u8]> {
    let end=at.checked_add(len).ok_or(Error::Invalid)?;
    bytes.get(usize::try_from(at).map_err(|_|Error::Limit)?..usize::try_from(end).map_err(|_|Error::Limit)?).ok_or(Error::Invalid)
}
pub(crate) fn le16(b:&[u8],at:usize)->Result<u16>{Ok(u16::from_le_bytes(range(b,at as u64,2)?.try_into().map_err(|_|Error::Invalid)?))}
pub(crate) fn le32(b:&[u8],at:usize)->Result<u32>{Ok(u32::from_le_bytes(range(b,at as u64,4)?.try_into().map_err(|_|Error::Invalid)?))}
pub(crate) fn alloc<T:Default+Clone>(n:usize)->Result<Vec<T>> { let mut p=Vec::new(); p.try_reserve_exact(n).map_err(|_|Error::Allocation)?;p.resize(n,T::default());Ok(p) }
