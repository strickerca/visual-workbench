use crate::{MAX_BUFFER_BYTES, Mask, MaskError, Region, Result, Size, filled};

/// Straight-alpha pixels in the source color space and depth. The source ICC
/// profile belongs to the caller/raster exporter and must accompany these bytes.
/// Hidden RGB is preserved even when the resulting alpha is zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cutout8 {
    pub region: Region,
    pub size: Size,
    pub rgba: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cutout16 {
    pub region: Region,
    pub size: Size,
    pub rgba: Vec<u16>,
}

impl Mask {
    /// A grayscale 8-bit PNG with values identical to the mask. No gamma/ICC
    /// conversion applies to coverage data. This is not a provider-specific
    /// transparent-area convention; provider adapters must map that explicitly.
    pub fn encode_mask_png(&self, region: Region) -> Result<Vec<u8>> {
        let size = region.size(self.size())?;
        let pixels = self.crop(region)?;
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, size.width(), size.height());
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().map_err(|_| MaskError::Encoding)?;
            writer
                .write_image_data(&pixels)
                .map_err(|_| MaskError::Encoding)?;
            writer.finish().map_err(|_| MaskError::Encoding)?;
        }
        if bytes.len() > MAX_BUFFER_BYTES {
            return Err(MaskError::Limit);
        }
        Ok(bytes)
    }
    /// `source` is the complete oriented document as row-major, straight RGBA8.
    /// Crop coordinates are integer document pixels; there is no resampling.
    pub fn cutout_rgba8(&self, source: &[u8], region: Region) -> Result<Cutout8> {
        if source.len()
            != self
                .size()
                .pixels()
                .checked_mul(4)
                .ok_or(MaskError::Limit)?
        {
            return Err(MaskError::Invalid);
        }
        let size = region.size(self.size())?;
        let mut rgba = filled(size.pixels().checked_mul(4).ok_or(MaskError::Limit)?, 0u8)?;
        for (i, target) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let x = region.x + i as u32 % region.width;
            let y = region.y + i as u32 / region.width;
            let start = (u64::from(y) * u64::from(self.size().width()) + u64::from(x)) as usize * 4;
            target.copy_from_slice(&source[start..start + 4]);
            target[3] = ((u32::from(target[3]) * u32::from(self.coverage(x, y)) + 127) / 255) as u8;
        }
        Ok(Cutout8 { region, size, rgba })
    }
    /// Same operation at the source's full 16-bit precision. RGB is never
    /// quantized; alpha is multiplied by the 8-bit mask with one integer round.
    pub fn cutout_rgba16(&self, source: &[u16], region: Region) -> Result<Cutout16> {
        if source.len()
            != self
                .size()
                .pixels()
                .checked_mul(4)
                .ok_or(MaskError::Limit)?
        {
            return Err(MaskError::Invalid);
        }
        let size = region.size(self.size())?;
        let mut rgba = filled(size.pixels().checked_mul(4).ok_or(MaskError::Limit)?, 0u16)?;
        for (i, target) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let x = region.x + i as u32 % region.width;
            let y = region.y + i as u32 / region.width;
            let start = (u64::from(y) * u64::from(self.size().width()) + u64::from(x)) as usize * 4;
            target.copy_from_slice(&source[start..start + 4]);
            target[3] =
                ((u32::from(target[3]) * u32::from(self.coverage(x, y)) + 127) / 255) as u16;
        }
        Ok(Cutout16 { region, size, rgba })
    }
}
