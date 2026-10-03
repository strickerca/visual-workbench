use crate::{DecodedImage, Pixels, RasterError, metadata};
use image::ImageEncoder;
use serde::{Deserialize, Serialize};
use vw_proto::v1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl Region {
    pub fn validate(self, w: u32, h: u32) -> Result<(), RasterError> {
        if self.width == 0
            || self.height == 0
            || self.x.checked_add(self.width).is_none_or(|v| v > w)
            || self.y.checked_add(self.height).is_none_or(|v| v > h)
        {
            Err(RasterError::Invalid("region outside source"))
        } else {
            Ok(())
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportFormat {
    Png8,
    Png16,
    Jpeg { quality: u8 },
    WebpLossless,
    WebpLossy { quality: u8 },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlphaPolicy {
    Preserve,
    Matte([u8; 3]),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorPolicy {
    Preserve,
    ConvertToSrgb { assume_untagged_srgb: bool },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportRequest {
    pub format: ExportFormat,
    pub region: Option<Region>,
    pub revision: v1::Revision,
    pub alpha: AlphaPolicy,
    pub color: ColorPolicy,
    pub allow_depth_reduction: bool,
    pub memory_budget_bytes: u64,
    /// Optional capture identity accompanies the document revision.
    pub capture_session: Option<vw_model::Id>,
    pub frame_id: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportMetadata {
    pub schema_version: u32,
    pub revision: String,
    pub source_asset: String,
    pub source_width: u32,
    pub source_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub source_bit_depth: u8,
    pub orientation_applied: u8,
    pub source_icc_hash: Option<String>,
    pub output_icc_hash: Option<String>,
    pub color_conversion: String,
    pub settings: ExportRequest,
}
#[derive(Debug)]
pub struct ExportedImage {
    pub bytes: Vec<u8>,
    pub metadata: ExportMetadata,
}

/// Nonallocating admission for retained ICC bytes, transient profile copies and
/// simultaneous source/destination parser objects during color conversion.
/// The codec/geometry reserves account their other fixed workspaces separately.
pub(crate) fn icc_workspace(profile: Option<&Vec<u8>>) -> Result<u64, RasterError> {
    let Some(profile) = profile else {
        return Ok(0);
    };
    let parsed = crate::pixels::admit_icc(profile)?;
    (profile.capacity() as u64)
        .checked_add(
            (profile.len() as u64)
                .checked_mul(8)
                .ok_or(RasterError::Allocation)?,
        )
        .and_then(|n| {
            parsed
                .checked_mul(2)
                .and_then(|parsed| n.checked_add(parsed))
        })
        .ok_or(RasterError::Allocation)
}

/// Pixel shape can be checked before ICC semantic validation without indexing
/// caller-provided buffers or invoking the allocating color parser.
pub(crate) fn image_shape(image: &DecodedImage) -> Result<(), RasterError> {
    if image.pixels.len() != crate::checked_samples(image.width, image.height)? {
        return Err(RasterError::Invalid("pixel buffer length"));
    }
    Ok(())
}

/// Checked before allocation/encoding; budget is a conservative working-set estimate,
/// not a claim about measured peak RSS. Native lossy WebP has a larger working set.
/// `has_alpha` describes the selected region, rather than pixels outside it.
pub fn preflight(
    width: u32,
    height: u32,
    depth: u8,
    has_alpha: bool,
    r: &ExportRequest,
) -> Result<u64, RasterError> {
    if width == 0 || height == 0 || !matches!(depth, 8 | 16) {
        return Err(RasterError::Invalid("dimensions/depth"));
    }
    if r.revision.state_hash.len() != 32 || r.capture_session.is_some() != r.frame_id.is_some() {
        return Err(RasterError::Invalid("revision/capture identity"));
    }
    let region = r.region.unwrap_or(Region {
        x: 0,
        y: 0,
        width,
        height,
    });
    region.validate(width, height)?;
    let (w, h) = (region.width, region.height);
    let (name, limit) = match r.format {
        ExportFormat::WebpLossless | ExportFormat::WebpLossy { .. } => ("WebP", 16383),
        ExportFormat::Jpeg { .. } => ("JPEG", 65535),
        _ => ("PNG", 0x7fffffff),
    };
    if w > limit || h > limit {
        return Err(RasterError::Dimensions {
            format: name,
            limit,
        });
    }
    if let ExportFormat::Jpeg { quality } | ExportFormat::WebpLossy { quality } = r.format
        && !(1..=100).contains(&quality)
    {
        return Err(RasterError::Invalid("quality must be 1..100"));
    }
    if depth == 16 && !matches!(r.format, ExportFormat::Png16) && !r.allow_depth_reduction {
        return Err(RasterError::Depth);
    }
    if matches!(r.format, ExportFormat::Jpeg { .. })
        && has_alpha
        && matches!(r.alpha, AlphaPolicy::Preserve)
    {
        return Err(RasterError::Alpha);
    }
    let per_pixel = if matches!(r.format, ExportFormat::WebpLossy { .. }) {
        160
    } else {
        64
    };
    let estimated = u64::from(w)
        .checked_mul(u64::from(h))
        .and_then(|v| v.checked_mul(per_pixel))
        .and_then(|v| v.checked_add(32 * 1024 * 1024))
        .ok_or(RasterError::Allocation)?;
    if estimated > r.memory_budget_bytes {
        return Err(RasterError::Memory {
            estimated,
            budget: r.memory_budget_bytes,
        });
    }
    Ok(estimated)
}

/// Export a full-resolution buffer. Call `render_document` first for marked output.
/// Caller bytes are never changed. Lossy/depth/matte conversions are explicit settings.
pub fn export(source: &DecodedImage, r: &ExportRequest) -> Result<ExportedImage, RasterError> {
    image_shape(source)?;
    if !source.original_available {
        return Err(RasterError::OriginalRequired);
    }
    let region = r.region.unwrap_or(Region {
        x: 0,
        y: 0,
        width: source.width,
        height: source.height,
    });
    region.validate(source.width, source.height)?;
    let has_alpha = (region.y..region.y + region.height).any(|y| {
        (region.x..region.x + region.width).any(|x| {
            let i = ((u64::from(y) * u64::from(source.width) + u64::from(x)) * 4) as usize;
            source.pixels.sample(i + 3) != u16::MAX
        })
    });
    let estimated = preflight(
        source.width,
        source.height,
        source.pixels.bit_depth(),
        has_alpha,
        r,
    )?;
    crate::source::check_memory(
        estimated
            .checked_add(icc_workspace(source.icc.as_ref())?)
            .ok_or(RasterError::Allocation)?,
        r.memory_budget_bytes,
    )?;
    source.validate()?;
    let mut image = source.crop(region)?;
    let conversion = match r.color {
        ColorPolicy::Preserve => "none",
        ColorPolicy::ConvertToSrgb {
            assume_untagged_srgb,
        } => {
            image.convert_profile(None, assume_untagged_srgb)?;
            if source.icc.is_some() {
                "ICC to sRGB"
            } else {
                "untagged explicitly assumed sRGB"
            }
        }
    };
    let gray = crate::pixels::gray_profile(&image)?;
    if gray
        && matches!(
            r.format,
            ExportFormat::WebpLossless | ExportFormat::WebpLossy { .. }
        )
    {
        return Err(RasterError::Unsupported(
            "grayscale ICC cannot label WebP RGB pixels; convert to sRGB or use PNG/JPEG",
        ));
    }
    if let AlphaPolicy::Matte(matte) = r.alpha {
        if gray && (matte[0] != matte[1] || matte[0] != matte[2]) {
            return Err(RasterError::Unsupported(
                "colored matte with grayscale profile; convert to sRGB first",
            ));
        }
        for i in (0..image.pixels.len()).step_by(4) {
            let a = u64::from(image.pixels.sample(i + 3));
            for (c, m) in matte.iter().enumerate() {
                let v = (u64::from(image.pixels.sample(i + c)) * a
                    + u64::from(*m) * 257 * (65535 - a)
                    + 32767)
                    / 65535;
                image.pixels.set_sample(i + c, v as u16);
            }
            image.pixels.set_sample(i + 3, 65535);
        }
    }
    let hash8 = r
        .revision
        .state_hash
        .iter()
        .take(4)
        .map(|v| format!("{v:02x}"))
        .collect::<String>();
    let meta = ExportMetadata {
        schema_version: 1,
        revision: format!("r{}-{hash8}", r.revision.host_seq),
        source_asset: source.source_asset.to_string(),
        source_width: source.width,
        source_height: source.height,
        output_width: image.width,
        output_height: image.height,
        source_bit_depth: source.pixels.bit_depth(),
        orientation_applied: source.orientation_applied,
        source_icc_hash: source
            .icc
            .as_ref()
            .map(|v| vw_model::AssetId::hash(v).to_string()),
        output_icc_hash: image
            .icc
            .as_ref()
            .map(|v| vw_model::AssetId::hash(v).to_string()),
        color_conversion: conversion.into(),
        settings: r.clone(),
    };
    let json = serde_json::to_string(&meta).map_err(|_| RasterError::Metadata)?;
    let bytes = match r.format {
        ExportFormat::Png8 | ExportFormat::Png16 => encode_png(&image, r.format, &json)?,
        ExportFormat::Jpeg { quality } => {
            let rgba = image.pixels.rgba8();
            let rgb: Vec<u8> = if gray {
                rgba.as_chunks::<4>().0.iter().map(|p| p[0]).collect()
            } else {
                rgba.as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|p| [p[0], p[1], p[2]])
                    .collect()
            };
            let mut encoded = Vec::new();
            let mut encoder =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, quality);
            if let Some(profile) = &image.icc {
                encoder
                    .set_icc_profile(profile.clone())
                    .map_err(|_| RasterError::Color)?;
            }
            encoder
                .encode(
                    &rgb,
                    image.width,
                    image.height,
                    if gray {
                        image::ExtendedColorType::L8
                    } else {
                        image::ExtendedColorType::Rgb8
                    },
                )
                .map_err(|_| RasterError::Codec)?;
            metadata::jpeg_xmp(encoded, &json)?
        }
        ExportFormat::WebpLossless => {
            let mut encoded = Vec::new();
            let encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut encoded);
            encoder
                .write_image(
                    &image.pixels.rgba8(),
                    image.width,
                    image.height,
                    image::ExtendedColorType::Rgba8,
                )
                .map_err(|_| RasterError::Codec)?;
            metadata::webp_xmp(
                encoded,
                image.width,
                image.height,
                image.pixels.has_alpha(),
                image.icc.as_deref(),
                &json,
            )?
        }
        ExportFormat::WebpLossy { quality } => {
            let rgba = image.pixels.rgba8();
            let encoded = crate::ffi::encode_lossy_webp(
                &rgba,
                image.width,
                image.height,
                quality,
                r.memory_budget_bytes,
            )?;
            metadata::webp_xmp(
                encoded,
                image.width,
                image.height,
                image.pixels.has_alpha(),
                image.icc.as_deref(),
                &json,
            )?
        }
    };
    Ok(ExportedImage {
        bytes,
        metadata: meta,
    })
}
fn encode_png(
    image: &DecodedImage,
    format: ExportFormat,
    json: &str,
) -> Result<Vec<u8>, RasterError> {
    let gray = crate::pixels::gray_profile(image)?;
    let mut info = png::Info::with_size(image.width, image.height);
    info.color_type = if gray {
        png::ColorType::GrayscaleAlpha
    } else {
        png::ColorType::Rgba
    };
    info.bit_depth = if format == ExportFormat::Png16 {
        png::BitDepth::Sixteen
    } else {
        png::BitDepth::Eight
    };
    if let Some(profile) = &image.icc {
        info.icc_profile = Some(std::borrow::Cow::Borrowed(profile));
    }
    let mut encoded = Vec::new();
    {
        let mut encoder =
            png::Encoder::with_info(&mut encoded, info).map_err(|_| RasterError::Codec)?;
        encoder.set_compression(png::Compression::Balanced);
        encoder.set_filter(png::Filter::Paeth);
        encoder
            .add_itxt_chunk("VisualWorkbench".into(), json.into())
            .map_err(|_| RasterError::Metadata)?;
        let mut writer = encoder.write_header().map_err(|_| RasterError::Codec)?;
        let bytes = if format == ExportFormat::Png16 {
            let samples = match &image.pixels {
                Pixels::Rgba16(p) => p.clone(),
                Pixels::Rgba8(p) => p.iter().map(|v| u16::from(*v) * 257).collect(),
            };
            let samples = if gray {
                crate::pixels::gray_alpha(&samples)
            } else {
                samples
            };
            samples.iter().flat_map(|v| v.to_be_bytes()).collect()
        } else {
            let bytes = image.pixels.rgba8();
            if gray {
                crate::pixels::gray_alpha(&bytes)
            } else {
                bytes
            }
        };
        writer
            .write_image_data(&bytes)
            .map_err(|_| RasterError::Codec)?;
        writer.finish().map_err(|_| RasterError::Codec)?;
    }
    Ok(encoded)
}
