//! Packed DIB parsing and DIBV5 construction. No Win32 access, external profile
//! paths, decompression fallback, resizing or implicit sample-depth reduction.
use crate::{DibOptions, HostError, HostResult};
use std::{borrow::Cow, io::Write};
use vw_raster::{DecodedImage, Pixels};

pub(crate) const MAX_DIB_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_CLIPBOARD_PIXELS: u64 = 16_000_000;
const MAX_ICC: usize = 4 * 1024 * 1024;
const SRGB: u32 = 0x7352_4742;
const WINDOWS_RGB: u32 = 0x5769_6e20;
const EMBEDDED: u32 = 0x4d42_4544;
const LINKED: u32 = 0x4c49_4e4b;

pub(crate) struct PreparedDib {
    pub bytes: Vec<u8>,
    pub depth_reduced: bool,
    pub assumed_srgb: bool,
}

pub(crate) fn decode_png(png: &[u8]) -> HostResult<DecodedImage> {
    if png.len() > crate::worker::MAX_PNG_BYTES {
        return Err(HostError::SizeLimit);
    }
    if png.len() < 33 || !png.starts_with(b"\x89PNG\r\n\x1a\n") || &png[12..16] != b"IHDR" {
        return Err(HostError::InvalidPng);
    }
    let width = u32::from_be_bytes(png[16..20].try_into().map_err(|_| HostError::InvalidPng)?);
    let height = u32::from_be_bytes(png[20..24].try_into().map_err(|_| HostError::InvalidPng)?);
    if u64::from(width) * u64::from(height) > MAX_CLIPBOARD_PIXELS {
        return Err(HostError::SizeLimit);
    }
    vw_raster::decode(
        png,
        vw_raster::DecodeLimits {
            max_encoded_bytes: crate::worker::MAX_PNG_BYTES,
            max_pixels: MAX_CLIPBOARD_PIXELS,
            max_memory_bytes: 512 * 1024 * 1024 - png.len() as u64 * 2,
        },
    )
    .map_err(|error| match error {
        vw_raster::RasterError::Memory { .. }
        | vw_raster::RasterError::Allocation
        | vw_raster::RasterError::Dimensions { .. } => HostError::SizeLimit,
        _ => HostError::InvalidPng,
    })
}

pub(crate) fn prepare(image: &DecodedImage, options: DibOptions) -> HostResult<PreparedDib> {
    let reduced = image.pixels.bit_depth() == 16;
    if reduced && !options.allow_depth_reduction {
        return Err(HostError::DepthConversionRequired);
    }
    if image.icc.is_none() && !options.assume_untagged_srgb {
        return Err(HostError::ColorAssumptionRequired);
    }
    if let Some(icc) = &image.icc {
        if icc.len() > MAX_ICC {
            return Err(HostError::SizeLimit);
        }
        // The DIB samples are RGB. Embedding a Gray profile would mislabel them;
        // the caller can request a color-managed sRGB export from vw-raster.
        if icc.get(16..20) != Some(b"RGB ") {
            return Err(HostError::ColorConversionRequired);
        }
    }
    let pixels = u64::from(image.width) * u64::from(image.height);
    if pixels > MAX_CLIPBOARD_PIXELS
        || image.width > i32::MAX as u32
        || image.height > i32::MAX as u32
    {
        return Err(HostError::SizeLimit);
    }
    let image_bytes = usize::try_from(pixels.checked_mul(4).ok_or(HostError::SizeLimit)?)
        .map_err(|_| HostError::SizeLimit)?;
    let size = 124usize
        .checked_add(image_bytes)
        .and_then(|n| n.checked_add(image.icc.as_ref().map_or(0, Vec::len)))
        .filter(|n| *n <= MAX_DIB_BYTES)
        .ok_or(HostError::SizeLimit)?;
    let mut bytes = allocate(size)?;
    put32(&mut bytes, 0, 124);
    put32(&mut bytes, 4, image.width);
    put32(&mut bytes, 8, (-(image.height as i32)) as u32);
    put16(&mut bytes, 12, 1);
    put16(&mut bytes, 14, 32);
    put32(&mut bytes, 16, 3);
    put32(&mut bytes, 20, image_bytes as u32);
    for (offset, mask) in [
        (40, 0x00ff_0000),
        (44, 0x0000_ff00),
        (48, 0x0000_00ff),
        (52, 0xff00_0000),
    ] {
        put32(&mut bytes, offset, mask);
    }
    put32(
        &mut bytes,
        56,
        if image.icc.is_some() { EMBEDDED } else { SRGB },
    );
    put32(&mut bytes, 108, 4); // LCS_GM_IMAGES; bytes remain in their original profile.
    for (index, out) in bytes[124..124 + image_bytes]
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .enumerate()
    {
        let rgba = match &image.pixels {
            Pixels::Rgba8(p) => [
                p[index * 4],
                p[index * 4 + 1],
                p[index * 4 + 2],
                p[index * 4 + 3],
            ],
            Pixels::Rgba16(p) => {
                std::array::from_fn(|c| ((u32::from(p[index * 4 + c]) + 128) / 257) as u8)
            }
        };
        out.copy_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
    }
    if let Some(icc) = &image.icc {
        // Clipboard data is a packed DIB: the embedded profile follows pixels.
        put32(&mut bytes, 112, (124 + image_bytes) as u32);
        put32(&mut bytes, 116, icc.len() as u32);
        bytes[124 + image_bytes..].copy_from_slice(icc);
    }
    Ok(PreparedDib {
        bytes,
        depth_reduced: reduced,
        assumed_srgb: image.icc.is_none(),
    })
}

pub(crate) struct ImportedDib {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub assumed_srgb: bool,
}

/// Supported: BITMAPINFOHEADER/V4/V5, uncompressed 16/24/32-bit truecolor and
/// contiguous BI_BITFIELDS/BI_ALPHABITFIELDS. Palettes/RLE/linked profiles refuse.
pub(crate) fn import(bytes: &[u8], assume_srgb: bool) -> HostResult<ImportedDib> {
    if bytes.len() > MAX_DIB_BYTES {
        return Err(HostError::SizeLimit);
    }
    let header = word(bytes, 0)? as usize;
    if !matches!(header, 40 | 108 | 124) {
        return Err(HostError::UnsupportedDib);
    }
    if bytes.len() < header || half(bytes, 12)? != 1 {
        return Err(HostError::InvalidDib);
    }
    let width = word(bytes, 4)? as i32;
    let signed_height = word(bytes, 8)? as i32;
    if width <= 0 || signed_height == 0 || signed_height == i32::MIN {
        return Err(HostError::InvalidDib);
    }
    let height = signed_height.unsigned_abs();
    let width = width as u32;
    let count = u64::from(width) * u64::from(height);
    if count > MAX_CLIPBOARD_PIXELS {
        return Err(HostError::SizeLimit);
    }
    let depth = half(bytes, 14)?;
    let compression = word(bytes, 16)?;
    if !matches!(depth, 16 | 24 | 32)
        || !matches!(compression, 0 | 3 | 6)
        || (depth == 24 && compression != 0)
        || word(bytes, 32)? != 0
    {
        return Err(HostError::UnsupportedDib);
    }
    let extra_masks = if header == 40 && compression != 0 {
        if compression == 6 { 16 } else { 12 }
    } else {
        0
    };
    let offset = header + extra_masks;
    let row = (u64::from(width) * u64::from(depth)).div_ceil(32) * 4;
    let pixel_bytes = row
        .checked_mul(u64::from(height))
        .ok_or(HostError::SizeLimit)?;
    let end = usize::try_from(pixel_bytes)
        .ok()
        .and_then(|n| n.checked_add(offset))
        .filter(|n| *n <= bytes.len())
        .ok_or(HostError::InvalidDib)?;
    let declared_size = u64::from(word(bytes, 20)?);
    if declared_size != 0 && declared_size != pixel_bytes {
        return Err(HostError::InvalidDib);
    }
    let masks = if compression == 0 {
        if depth == 16 {
            [0x7c00, 0x03e0, 0x001f, 0]
        } else {
            [0xff0000, 0xff00, 0xff, 0]
        }
    } else {
        [
            word(bytes, 40)?,
            word(bytes, 44)?,
            word(bytes, 48)?,
            if header >= 108 || compression == 6 {
                word(bytes, 52)?
            } else {
                0
            },
        ]
    };
    if masks[..3].contains(&0) {
        return Err(HostError::InvalidDib);
    }
    let mut used = 0u32;
    let mut needs_depth_conversion = false;
    for mask in masks {
        if mask == 0 {
            continue;
        }
        let shifted = mask >> mask.trailing_zeros();
        if used & mask != 0
            || shifted.count_ones() > 16
            || shifted.checked_add(1).is_none_or(|n| !n.is_power_of_two())
            || (depth < 32 && mask >> depth != 0)
        {
            return Err(HostError::UnsupportedDib);
        }
        needs_depth_conversion |= shifted.count_ones() > 8;
        used |= mask;
    }
    // The import contract produces PNG8. A 10:10:10:2 (or wider-channel)
    // bitmap must not silently lose distinct channel values during expansion.
    if needs_depth_conversion {
        return Err(HostError::DepthConversionRequired);
    }
    let color_space = if header >= 108 { word(bytes, 56)? } else { 0 };
    if color_space == LINKED {
        return Err(HostError::ColorConversionRequired);
    }
    let profile = if color_space == EMBEDDED {
        if header != 124 {
            return Err(HostError::InvalidDib);
        }
        let start = word(bytes, 112)? as usize;
        let length = word(bytes, 116)? as usize;
        let profile_end = start.checked_add(length).ok_or(HostError::InvalidDib)?;
        if length == 0 || length > MAX_ICC || start < end || profile_end > bytes.len() {
            return Err(HostError::InvalidDib);
        }
        let profile = &bytes[start..profile_end];
        if profile.get(16..20) != Some(b"RGB ") {
            return Err(HostError::ColorConversionRequired);
        }
        Some(profile)
    } else {
        if color_space != 0 && !matches!(color_space, SRGB | WINDOWS_RGB) {
            return Err(HostError::ColorConversionRequired);
        }
        // Nonzero calibrated endpoints/gamma describe actual color; never erase
        // them by treating the header as an untagged bitmap.
        if header >= 108 && color_space == 0 && bytes[60..108].iter().any(|v| *v != 0) {
            return Err(HostError::ColorConversionRequired);
        }
        if color_space == 0 && !assume_srgb {
            return Err(HostError::ColorAssumptionRequired);
        }
        None
    };
    let estimate = count
        .checked_mul(32)
        .and_then(|n| n.checked_add(bytes.len() as u64 * 2))
        .and_then(|n| n.checked_add(64 * 1024 * 1024))
        .ok_or(HostError::SizeLimit)?;
    if estimate > 512 * 1024 * 1024 {
        return Err(HostError::SizeLimit);
    }
    if let Some(profile) = profile {
        validate_profile(profile)?;
    }
    let mut rgba = allocate(usize::try_from(count * 4).map_err(|_| HostError::SizeLimit)?)?;
    let step = usize::from(depth / 8);
    for y in 0..height as usize {
        let source_y = if signed_height < 0 {
            y
        } else {
            height as usize - 1 - y
        };
        let at = offset + source_y * row as usize;
        for x in 0..width as usize {
            let p = &bytes[at + x * step..at + (x + 1) * step];
            let raw = p
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (i * 8)));
            let dest = &mut rgba[(y * width as usize + x) * 4..(y * width as usize + x + 1) * 4];
            for c in 0..4 {
                let mask = masks[c];
                dest[c] = if mask == 0 {
                    255
                } else {
                    let maximum = mask >> mask.trailing_zeros();
                    let value = (raw & mask) >> mask.trailing_zeros();
                    ((u64::from(value) * 255 + u64::from(maximum) / 2) / u64::from(maximum)) as u8
                };
            }
        }
    }
    let mut info = png::Info::with_size(width, height);
    info.color_type = png::ColorType::Rgba;
    info.bit_depth = png::BitDepth::Eight;
    if let Some(profile) = profile {
        info.icc_profile = Some(Cow::Borrowed(profile));
    } else {
        info.srgb = Some(png::SrgbRenderingIntent::Perceptual);
    }
    let mut output = LimitedPng {
        bytes: Vec::new(),
        exceeded: false,
    };
    let result = (|| {
        let encoder = png::Encoder::with_info(&mut output, info)?;
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&rgba)?;
        writer.finish()
    })();
    if result.is_err() {
        return Err(if output.exceeded {
            HostError::SizeLimit
        } else {
            HostError::InvalidDib
        });
    }
    Ok(ImportedDib {
        png: output.bytes,
        width,
        height,
        assumed_srgb: color_space == 0,
    })
}

pub(crate) fn validate_profile(profile: &[u8]) -> HostResult<()> {
    if profile.len() > MAX_ICC {
        return Err(HostError::SizeLimit);
    }
    // Shared raster validation includes nonallocating ICC tag-expansion admission.
    let mut icc = Vec::new();
    icc.try_reserve_exact(profile.len())
        .map_err(|_| HostError::SizeLimit)?;
    icc.extend_from_slice(profile);
    DecodedImage {
        width: 1,
        height: 1,
        pixels: Pixels::Rgba8(vec![0, 0, 0, 255]),
        icc: Some(icc),
        source_asset: vw_model::AssetId::hash(b"profile validation"),
        original_available: true,
        orientation_applied: 1,
    }
    .validate()
    .map_err(|_| HostError::ColorConversionRequired)
}

struct LimitedPng {
    bytes: Vec<u8>,
    exceeded: bool,
}
impl Write for LimitedPng {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|n| n > crate::worker::MAX_PNG_BYTES)
        {
            self.exceeded = true;
            return Err(std::io::Error::other("PNG output limit"));
        }
        self.bytes
            .try_reserve_exact(bytes.len())
            .map_err(|_| std::io::Error::other("allocation"))?;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn allocate(size: usize) -> HostResult<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| HostError::SizeLimit)?;
    bytes.resize(size, 0);
    Ok(bytes)
}
fn word(bytes: &[u8], offset: usize) -> HostResult<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or(HostError::InvalidDib)?
            .try_into()
            .map_err(|_| HostError::InvalidDib)?,
    ))
}
fn half(bytes: &[u8], offset: usize) -> HostResult<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or(HostError::InvalidDib)?
            .try_into()
            .map_err(|_| HostError::InvalidDib)?,
    ))
}
fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
