use crate::{RasterError, Region, checked_samples};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use std::io::Cursor;
use vw_model::AssetId;

pub(crate) const MAX_ICC_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_ICC_PARSE_BYTES: u64 = 16 * 1024 * 1024;

/// Bound allocation amplification before moxcms interprets tag offsets. Aliases
/// are charged for every occurrence because each parsed field owns its data.
pub(crate) fn admit_icc(bytes: &[u8]) -> Result<u64, RasterError> {
    if bytes.len() > MAX_ICC_BYTES {
        return Err(RasterError::Invalid("ICC size"));
    }
    let word = |data: &[u8], offset: usize| -> Result<usize, RasterError> {
        let end = offset.checked_add(4).ok_or(RasterError::Color)?;
        Ok(u32::from_be_bytes(
            data.get(offset..end)
                .ok_or(RasterError::Color)?
                .try_into()
                .map_err(|_| RasterError::Color)?,
        ) as usize)
    };
    let declared = word(bytes, 0)?;
    if declared < 132 || declared > bytes.len() {
        return Err(RasterError::Color);
    }
    let count = word(bytes, 128)?;
    if count > 256 {
        return Err(RasterError::Unsupported("ICC tag count"));
    }
    let table_end = count
        .checked_mul(12)
        .and_then(|n| n.checked_add(132))
        .ok_or(RasterError::Color)?;
    let table = bytes.get(132..table_end).ok_or(RasterError::Color)?;
    let mut estimate = 4096u64;
    let mut charge = |n: u64| -> Result<(), RasterError> {
        estimate = estimate.checked_add(n).ok_or(RasterError::Allocation)?;
        if estimate > MAX_ICC_PARSE_BYTES {
            return Err(RasterError::Unsupported("ICC expanded metadata limit"));
        }
        Ok(())
    };
    for entry in table.as_chunks::<12>().0.iter() {
        let start = word(entry, 4)?;
        let length = word(entry, 8)?;
        let end = start.checked_add(length).ok_or(RasterError::Color)?;
        if start < table_end || end > declared {
            return Err(RasterError::Color);
        }
        let tag = bytes.get(start..end).ok_or(RasterError::Color)?;
        let kind = tag.get(..4).ok_or(RasterError::Color)?;
        // Covers copied LUT/TRC storage, nested curve-group aliases, lossy
        // UTF-8 expansion, temporary buffers and parser object overhead.
        charge(
            (length as u64)
                .checked_mul(8)
                .and_then(|n| n.checked_add(4096))
                .ok_or(RasterError::Allocation)?,
        )?;
        if kind == b"mAB " || kind == b"mBA " {
            // moxcms permits a large channel count when only the other count
            // is <=4; this application supports RGB/Gray ICCs only.
            if !tag
                .get(8..10)
                .is_some_and(|channels| channels.iter().all(|n| (1..=4).contains(n)))
            {
                return Err(RasterError::Unsupported("ICC LUT channel count"));
            }
        }
        if kind == b"mluc" {
            let records = word(tag, 8)?;
            if records == 0 || records > 1024 || word(tag, 12)? != 12 {
                return Err(RasterError::Unsupported("ICC localized text record limit"));
            }
            let records_end = records
                .checked_mul(12)
                .and_then(|n| n.checked_add(16))
                .ok_or(RasterError::Color)?;
            let records = tag.get(16..records_end).ok_or(RasterError::Color)?;
            for record in records.as_chunks::<12>().0.iter() {
                let length = word(record, 4)?;
                let start = word(record, 8)?;
                let end = start.checked_add(length).ok_or(RasterError::Color)?;
                if length % 2 != 0 || start < records_end || end > tag.len() {
                    return Err(RasterError::Color);
                }
                // Repeated offsets count repeatedly. UTF-16 temporaries plus
                // worst-case UTF-8 and Vec/String growth remain bounded.
                charge(
                    (length as u64)
                        .checked_mul(4)
                        .and_then(|n| n.checked_add(256))
                        .ok_or(RasterError::Allocation)?,
                )?;
            }
        }
    }
    Ok(estimate)
}

pub(crate) fn parse_color_profile(bytes: &[u8]) -> Result<moxcms::ColorProfile, RasterError> {
    admit_icc(bytes)?;
    moxcms::ColorProfile::new_from_slice_with_options(
        bytes,
        moxcms::ParsingOptions {
            max_profile_size: MAX_ICC_BYTES + 1,
            max_allowed_clut_size: MAX_ICC_BYTES,
            max_allowed_trc_size: 40_000,
        },
    )
    .map_err(|_| RasterError::Color)
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pixels {
    Rgba8(Vec<u8>),
    Rgba16(Vec<u16>),
}
impl Pixels {
    pub fn bit_depth(&self) -> u8 {
        match self {
            Self::Rgba8(_) => 8,
            Self::Rgba16(_) => 16,
        }
    }
    pub fn len(&self) -> usize {
        match self {
            Self::Rgba8(p) => p.len(),
            Self::Rgba16(p) => p.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn has_alpha(&self) -> bool {
        match self {
            Self::Rgba8(p) => p.as_chunks::<4>().0.iter().any(|p| p[3] != 255),
            Self::Rgba16(p) => p.as_chunks::<4>().0.iter().any(|p| p[3] != 65535),
        }
    }
    pub fn rgba8(&self) -> Vec<u8> {
        match self {
            Self::Rgba8(p) => p.clone(),
            Self::Rgba16(p) => p
                .iter()
                .map(|v| ((u32::from(*v) + 128) / 257) as u8)
                .collect(),
        }
    }
    pub(crate) fn sample(&self, index: usize) -> u16 {
        match self {
            Self::Rgba8(p) => u16::from(p[index]) * 257,
            Self::Rgba16(p) => p[index],
        }
    }
    pub(crate) fn set_sample(&mut self, index: usize, value: u16) {
        match self {
            Self::Rgba8(p) => p[index] = ((u32::from(value) + 128) / 257) as u8,
            Self::Rgba16(p) => p[index] = value,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels,
    pub icc: Option<Vec<u8>>,
    /// Hash of the immutable encoded original, never a preview or re-encode.
    pub source_asset: AssetId,
    pub original_available: bool,
    pub orientation_applied: u8,
}
impl DecodedImage {
    pub fn validate(&self) -> Result<(), RasterError> {
        if self.pixels.len() != checked_samples(self.width, self.height)? {
            return Err(RasterError::Invalid("pixel buffer length"));
        }
        if !(1..=8).contains(&self.orientation_applied) {
            return Err(RasterError::Invalid("orientation"));
        }
        if self.icc.as_ref().is_some_and(|v| v.len() > 4 * 1024 * 1024) {
            return Err(RasterError::Invalid("ICC size"));
        }
        if let Some(profile) = &self.icc {
            let profile = parse_color_profile(profile)?;
            if !matches!(
                profile.color_space,
                moxcms::DataColorSpace::Rgb | moxcms::DataColorSpace::Gray
            ) {
                return Err(RasterError::Unsupported(
                    "ICC color space; RGB and grayscale are supported",
                ));
            }
            if profile.color_space == moxcms::DataColorSpace::Gray
                && (0..self.pixels.len()).step_by(4).any(|i| {
                    self.pixels.sample(i) != self.pixels.sample(i + 1)
                        || self.pixels.sample(i) != self.pixels.sample(i + 2)
                })
            {
                return Err(RasterError::Invalid("grayscale ICC with colored pixels"));
            }
        }
        Ok(())
    }
    pub fn crop(&self, r: Region) -> Result<Self, RasterError> {
        self.validate()?;
        r.validate(self.width, self.height)?;
        let samples = checked_samples(r.width, r.height)?;
        let pixels = match &self.pixels {
            Pixels::Rgba8(p) => {
                let mut q = Vec::new();
                q.try_reserve_exact(samples)
                    .map_err(|_| RasterError::Allocation)?;
                for y in r.y..r.y + r.height {
                    let start =
                        ((u64::from(y) * u64::from(self.width) + u64::from(r.x)) * 4) as usize;
                    q.extend_from_slice(&p[start..start + r.width as usize * 4]);
                }
                Pixels::Rgba8(q)
            }
            Pixels::Rgba16(p) => {
                let mut q = Vec::new();
                q.try_reserve_exact(samples)
                    .map_err(|_| RasterError::Allocation)?;
                for y in r.y..r.y + r.height {
                    let start =
                        ((u64::from(y) * u64::from(self.width) + u64::from(r.x)) * 4) as usize;
                    q.extend_from_slice(&p[start..start + r.width as usize * 4]);
                }
                Pixels::Rgba16(q)
            }
        };
        Ok(Self {
            width: r.width,
            height: r.height,
            pixels,
            icc: self.icc.clone(),
            source_asset: self.source_asset.clone(),
            original_available: self.original_available,
            orientation_applied: self.orientation_applied,
        })
    }
    pub(crate) fn convert_profile(
        &mut self,
        destination: Option<&[u8]>,
        assume_srgb: bool,
    ) -> Result<(), RasterError> {
        let source = match &self.icc {
            Some(p) => parse_color_profile(p)?,
            None if assume_srgb => moxcms::ColorProfile::new_srgb(),
            None => {
                return Err(RasterError::Unsupported(
                    "untagged color; explicitly assume sRGB",
                ));
            }
        };
        let dest = match destination {
            Some(p) => parse_color_profile(p)?,
            None => moxcms::ColorProfile::new_srgb(),
        };
        let source_gray = source.color_space == moxcms::DataColorSpace::Gray;
        let dest_gray = dest.color_space == moxcms::DataColorSpace::Gray;
        if !matches!(
            source.color_space,
            moxcms::DataColorSpace::Rgb | moxcms::DataColorSpace::Gray
        ) || !matches!(
            dest.color_space,
            moxcms::DataColorSpace::Rgb | moxcms::DataColorSpace::Gray
        ) {
            return Err(RasterError::Color);
        }
        let sl = if source_gray {
            moxcms::Layout::GrayAlpha
        } else {
            moxcms::Layout::Rgba
        };
        let dl = if dest_gray {
            moxcms::Layout::GrayAlpha
        } else {
            moxcms::Layout::Rgba
        };
        match &mut self.pixels {
            Pixels::Rgba8(p) => {
                let input = if source_gray {
                    gray_alpha(p)
                } else {
                    p.clone()
                };
                let mut output = vec![0; if dest_gray { p.len() / 2 } else { p.len() }];
                source
                    .create_transform_8bit(sl, &dest, dl, moxcms::TransformOptions::default())
                    .map_err(|_| RasterError::Color)?
                    .transform(&input, &mut output)
                    .map_err(|_| RasterError::Color)?;
                if dest_gray {
                    output = expand_gray_alpha(&output);
                }
                for (old, new) in p
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(output.as_chunks_mut::<4>().0.iter_mut())
                {
                    new[3] = old[3];
                }
                *p = output;
            }
            Pixels::Rgba16(p) => {
                let input = if source_gray {
                    gray_alpha(p)
                } else {
                    p.clone()
                };
                let mut output = vec![0; if dest_gray { p.len() / 2 } else { p.len() }];
                source
                    .create_transform_16bit(sl, &dest, dl, moxcms::TransformOptions::default())
                    .map_err(|_| RasterError::Color)?
                    .transform(&input, &mut output)
                    .map_err(|_| RasterError::Color)?;
                if dest_gray {
                    output = expand_gray_alpha(&output);
                }
                for (old, new) in p
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(output.as_chunks_mut::<4>().0.iter_mut())
                {
                    new[3] = old[3];
                }
                *p = output;
            }
        }
        // Preserve an explicitly supplied destination profile byte-for-byte.
        // Generated sRGB headers must not contain the wall clock.
        self.icc = Some(match destination {
            Some(bytes) => bytes.to_vec(),
            None => standard_srgb_profile()?,
        });
        Ok(())
    }
}
pub(crate) fn gray_alpha<T: Copy>(p: &[T]) -> Vec<T> {
    p.as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[3]])
        .collect()
}
pub fn standard_srgb_profile() -> Result<Vec<u8>, RasterError> {
    let mut bytes = moxcms::ColorProfile::new_srgb()
        .encode()
        .map_err(|_| RasterError::Color)?;
    let date = bytes.get_mut(24..36).ok_or(RasterError::Color)?;
    date.copy_from_slice(&[0x07, 0xd0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0]);
    Ok(bytes)
}
fn expand_gray_alpha<T: Copy>(p: &[T]) -> Vec<T> {
    p.as_chunks::<2>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[0], p[0], p[1]])
        .collect()
}
pub(crate) fn gray_profile(image: &DecodedImage) -> Result<bool, RasterError> {
    match &image.icc {
        Some(p) => Ok(parse_color_profile(p)?.color_space == moxcms::DataColorSpace::Gray),
        None => Ok(false),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DecodeLimits {
    pub max_encoded_bytes: usize,
    pub max_pixels: u64,
    pub max_memory_bytes: u64,
}
impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_encoded_bytes: 256 * 1024 * 1024,
            max_pixels: 50_000_000,
            max_memory_bytes: 1024 * 1024 * 1024,
        }
    }
}

/// Pure Rust decoders. Orientation is consumed once; exports do not retain stale EXIF.
pub fn decode(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedImage, RasterError> {
    if bytes.len() > limits.max_encoded_bytes {
        return Err(RasterError::Invalid("encoded size"));
    }
    let format = image::guess_format(bytes).map_err(|_| RasterError::Codec)?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err(RasterError::Unsupported(
            "format; supported PNG, JPEG, WebP",
        ));
    }
    // image's PNG adapter clones ICC metadata before exposing its length. Admit
    // it under a small decoder limit before starting the pixel decoder.
    let png_icc = if format == ImageFormat::Png {
        png_profile_preflight(bytes, limits.max_memory_bytes)?
    } else {
        None
    };
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut image_limits = image::Limits::default();
    image_limits.max_alloc = Some(limits.max_memory_bytes);
    reader.limits(image_limits);
    let mut decoder = reader.into_decoder().map_err(|_| RasterError::Codec)?;
    let (w, h) = decoder.dimensions();
    let count = u64::from(w) * u64::from(h);
    if count > limits.max_pixels {
        return Err(RasterError::Unsupported(
            "image exceeds pixel limit; split or use tiled import",
        ));
    }
    let icc = if format == ImageFormat::Png {
        png_icc
    } else {
        decoder.icc_profile().map_err(|_| RasterError::Codec)?
    };
    let color_bytes = icc.as_ref().map_or(Ok(0), |p| {
        admit_icc(p).and_then(|parsed| {
            parsed
                .checked_add(p.len() as u64 * 4)
                .ok_or(RasterError::Allocation)
        })
    })?;
    let estimate = count
        .checked_mul(32)
        .and_then(|n| n.checked_add(color_bytes))
        .ok_or(RasterError::Allocation)?;
    if estimate > limits.max_memory_bytes {
        return Err(RasterError::Memory {
            estimated: estimate,
            budget: limits.max_memory_bytes,
        });
    }
    let orientation = decoder.orientation().map_err(|_| RasterError::Codec)?;
    let applied = orientation.to_exif();
    let mut image = DynamicImage::from_decoder(decoder).map_err(|_| RasterError::Codec)?;
    image.apply_orientation(orientation);
    let depth = image.color().bits_per_pixel() / u16::from(image.color().channel_count());
    let pixels = if depth > 8 {
        Pixels::Rgba16(image.to_rgba16().into_raw())
    } else {
        Pixels::Rgba8(image.to_rgba8().into_raw())
    };
    let result = DecodedImage {
        width: image.width(),
        height: image.height(),
        pixels,
        icc,
        source_asset: AssetId::hash(bytes),
        original_available: true,
        orientation_applied: applied,
    };
    result.validate()?;
    Ok(result)
}

/// Inspect chunk headers without allocating another decoded profile. A declared
/// ICC must survive the decoder's bounded metadata parse; other declarations must
/// not silently become untagged source pixels.
pub(crate) fn png_color_profile(
    bytes: &[u8],
    icc: Option<Vec<u8>>,
) -> Result<Option<Vec<u8>>, RasterError> {
    let declaration = png_color_declaration(bytes)?;
    if declaration.icc && icc.is_none() {
        return Err(RasterError::Unsupported(
            "declared PNG ICC profile is invalid or exceeds its resource limit",
        ));
    }
    if let Some(profile) = &icc {
        admit_icc(profile)?;
        return Ok(icc);
    }
    if declaration.srgb {
        return standard_srgb_profile().map(Some);
    }
    if declaration.legacy_color {
        return Err(RasterError::Unsupported(
            "PNG gamma/chromaticities without ICC or sRGB; convert with a color-aware decoder",
        ));
    }
    Ok(None)
}

/// png 0.18 discards iCCP decompression errors. Admit its declared profile in a
/// dedicated small decoder before creating a large-budget pixel decoder.
pub(crate) fn png_profile_preflight(
    bytes: &[u8],
    budget: u64,
) -> Result<Option<Vec<u8>>, RasterError> {
    if !png_color_declaration(bytes)?.icc {
        return png_color_profile(bytes, None);
    }
    let decoder_limit = (MAX_ICC_BYTES as u64).min(budget / 4) as usize;
    if decoder_limit == 0 {
        return Err(RasterError::Memory {
            estimated: 4,
            budget,
        });
    }
    let mut options = png::DecodeOptions::default();
    options.set_ignore_text_chunk(true);
    options.set_ignore_adler32(false);
    options.set_skip_ancillary_crc_failures(false);
    let mut decoder = png::Decoder::new_with_options(Cursor::new(bytes), options);
    decoder.set_limits(png::Limits {
        bytes: decoder_limit,
    });
    let reader = decoder.read_info().map_err(|_| RasterError::Codec)?;
    let icc = match reader.info().icc_profile.as_deref() {
        None => None,
        Some(profile) => {
            let parsed = admit_icc(profile)?;
            // fdeflate's Vec can grow beyond its logical output limit. Reserve
            // twice that allowance plus the checked clone and parser sizes.
            let estimated = decoder_limit as u64 * 2 + profile.len() as u64 + parsed;
            if estimated > budget {
                return Err(RasterError::Memory { estimated, budget });
            }
            let mut owned = Vec::new();
            owned
                .try_reserve_exact(profile.len())
                .map_err(|_| RasterError::Allocation)?;
            owned.extend_from_slice(profile);
            Some(owned)
        }
    };
    png_color_profile(bytes, icc)
}

#[derive(Default)]
struct PngColorDeclaration {
    icc: bool,
    srgb: bool,
    legacy_color: bool,
}
fn png_color_declaration(bytes: &[u8]) -> Result<PngColorDeclaration, RasterError> {
    if bytes.get(..8) != Some(&[137, 80, 78, 71, 13, 10, 26, 10]) {
        return Err(RasterError::Codec);
    }
    let mut offset = 8usize;
    let mut declaration = PngColorDeclaration::default();
    for _ in 0..65536 {
        let header_end = offset.checked_add(8).ok_or(RasterError::Allocation)?;
        let header = bytes.get(offset..header_end).ok_or(RasterError::Codec)?;
        let length =
            u32::from_be_bytes(header[..4].try_into().map_err(|_| RasterError::Codec)?) as usize;
        let end = header_end
            .checked_add(length)
            .and_then(|v| v.checked_add(4))
            .ok_or(RasterError::Allocation)?;
        if end > bytes.len() {
            return Err(RasterError::Codec);
        }
        match &header[4..] {
            b"IDAT" => return Ok(declaration),
            b"iCCP" => declaration.icc = true,
            b"sRGB" => declaration.srgb = true,
            b"gAMA" | b"cHRM" => declaration.legacy_color = true,
            b"cICP" | b"mDCV" | b"cLLI" => {
                return Err(RasterError::Unsupported(
                    "PNG cICP/HDR metadata; convert with a supported color decoder",
                ));
            }
            b"IEND" => return Err(RasterError::Codec),
            _ => {}
        }
        offset = end;
    }
    Err(RasterError::Invalid("PNG header chunk limit"))
}
