//! Numeric analysis and raster codec measurements. No user document inputs.
use image::{ExtendedColorType, ImageEncoder, ImageFormat};
use serde::Serialize;
use std::time::Instant;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid or unbounded benchmark data")]
    Invalid,
    #[error("platform phase {phase} failed (HRESULT {code:08x})")]
    Platform { phase: &'static str, code: u32 },
    #[error("bounded phase {0} timed out")]
    Timeout(&'static str),
    #[error("required hardware behavior is unavailable: {0}")]
    Unsupported(&'static str),
    #[error("codec operation failed")]
    Codec(#[from] image::ImageError),
    #[error("file operation failed")]
    Io(#[from] std::io::Error),
    #[error("numeric report serialization failed")]
    Json(#[from] serde_json::Error),
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Serialize)]
pub struct Stats {
    pub count: usize,
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
}
pub fn stats(values: &[f64]) -> Result<Stats> {
    if values.is_empty() || values.iter().any(|n| !n.is_finite() || *n < 0.0) {
        return Err(Error::Invalid);
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let at = |p: f64| sorted[(p * sorted.len() as f64).ceil() as usize - 1];
    Ok(Stats {
        count: values.len(),
        p50: at(0.5),
        p95: at(0.95),
        max: sorted[sorted.len() - 1],
    })
}
pub fn byte_count(width: u32, height: u32, channels: usize) -> Result<usize> {
    if width == 0 || height == 0 || width > 4096 || height > 4096 || ![3, 4].contains(&channels) {
        return Err(Error::Invalid);
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|v| v.checked_mul(channels))
        .ok_or(Error::Invalid)
}

/// Tight changed row spans, coalesced vertically. Every differing RGB pixel is
/// covered; highly fragmented input falls back to one bounded full-frame region.
pub fn changed_regions(
    previous: &[u8],
    current: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<(u32, u32, u32, u32)>> {
    let length = byte_count(width, height, 3)?;
    if previous.len() != length || current.len() != length {
        return Err(Error::Invalid);
    }
    let stride = width as usize * 3;
    let mut regions: Vec<(u32, u32, u32, u32)> = Vec::new();
    for y in 0..height {
        let offset = y as usize * stride;
        let before = &previous[offset..offset + stride];
        let after = &current[offset..offset + stride];
        if before == after {
            continue;
        }
        let first = before
            .iter()
            .zip(after)
            .position(|(a, b)| a != b)
            .ok_or(Error::Invalid)?
            / 3;
        let last = before
            .iter()
            .zip(after)
            .rposition(|(a, b)| a != b)
            .ok_or(Error::Invalid)?
            / 3;
        let w = (last - first + 1) as u32;
        if let Some(row) = regions.last_mut()
            && row.0 == first as u32
            && row.2 == w
            && row.1 + row.3 == y
        {
            row.3 += 1;
        } else {
            if regions.len() == 128 {
                return Ok(vec![(0, 0, width, height)]);
            }
            regions.push((first as u32, y, w, 1));
        }
    }
    Ok(regions)
}

pub fn encode(rgb: &[u8], width: u32, height: u32, format: ImageFormat) -> Result<Vec<u8>> {
    if rgb.len() != byte_count(width, height, 3)? {
        return Err(Error::Invalid);
    }
    let mut bytes = Vec::new();
    match format {
        ImageFormat::Jpeg => image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 85)
            .encode(rgb, width, height, ExtendedColorType::Rgb8)?,
        ImageFormat::Png => image::codecs::png::PngEncoder::new_with_quality(
            &mut bytes,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::Sub,
        )
        .write_image(rgb, width, height, ExtendedColorType::Rgb8)?,
        ImageFormat::Qoi => image::codecs::qoi::QoiEncoder::new(&mut bytes).write_image(
            rgb,
            width,
            height,
            ExtendedColorType::Rgb8,
        )?,
        _ => return Err(Error::Invalid),
    }
    Ok(bytes)
}

#[derive(Serialize)]
pub struct RasterResult {
    pub format: &'static str,
    pub width: u32,
    pub height: u32,
    pub encode_ms: Stats,
    pub bytes: Stats,
    pub lossless_roundtrip_verified: bool,
}
pub fn measure_raster(
    rgb: &[u8],
    width: u32,
    height: u32,
    format: ImageFormat,
    count: usize,
) -> Result<RasterResult> {
    if !(1..=100).contains(&count) {
        return Err(Error::Invalid);
    }
    let mut times = Vec::new();
    let mut sizes = Vec::new();
    for i in 0..count + 2 {
        let start = Instant::now();
        let output = encode(rgb, width, height, format)?;
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        // Validation is deliberately outside the encode clock.
        let decoded = image::load_from_memory_with_format(&output, format)?.to_rgb8();
        if decoded.width() != width
            || decoded.height() != height
            || (format != ImageFormat::Jpeg && decoded.as_raw() != rgb)
        {
            return Err(Error::Invalid);
        }
        if i >= 2 {
            times.push(elapsed);
            sizes.push(output.len() as f64);
        }
    }
    Ok(RasterResult {
        format: match format {
            ImageFormat::Png => "PNG fast/Sub",
            ImageFormat::Qoi => "QOI",
            ImageFormat::Jpeg => "JPEG q85",
            _ => return Err(Error::Invalid),
        },
        width,
        height,
        encode_ms: stats(&times)?,
        bytes: stats(&sizes)?,
        lossless_roundtrip_verified: format != ImageFormat::Jpeg,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn derived_regions_reconstruct_every_changed_pixel() -> Result<()> {
        let (w, h) = (37, 263);
        let before = vec![0; byte_count(w, h, 3)?];
        for fragmented in [false, true] {
            let mut after = before.clone();
            for y in 0..h {
                for x in 0..w {
                    if (fragmented && y % 2 == 0 && x == y % w) || (!fragmented && y < 26) {
                        let start = (y as usize * w as usize + x as usize) * 3;
                        after[start..start + 3].copy_from_slice(&[37, 71, 199]);
                    }
                }
            }
            let regions = changed_regions(&before, &after, w, h)?;
            let mut restored = before.clone();
            for (x, y, rw, rh) in &regions {
                for row in *y..y + rh {
                    let start = (row as usize * w as usize + *x as usize) * 3;
                    let end = start + *rw as usize * 3;
                    restored[start..end].copy_from_slice(&after[start..end]);
                }
            }
            assert_eq!(restored, after);
            assert!(regions.len() <= 128);
            if !fragmented {
                assert_eq!(regions, vec![(0, 0, w, 26)])
            }
        }
        assert!(changed_regions(&before, &before, w, h)?.is_empty());
        assert!(changed_regions(&before[..3], &before, w, h).is_err());
        Ok(())
    }
    #[test]
    fn reject_unbounded_dimensions_and_nonfinite_measurements() {
        assert!(byte_count(0, 1, 3).is_err());
        assert!(byte_count(5000, 5000, 3).is_err());
        assert!(stats(&[f64::NAN]).is_err());
        assert!(stats(&[]).is_err());
    }
    #[test]
    fn png_and_qoi_preserve_arbitrary_channels_exactly() -> Result<()> {
        let rgb: Vec<u8> = (0..17 * 13 * 3).map(|n| (n * 37) as u8).collect();
        for format in [ImageFormat::Png, ImageFormat::Qoi] {
            let bytes = encode(&rgb, 17, 13, format)?;
            let decoded = image::load_from_memory_with_format(&bytes, format)?.to_rgb8();
            assert_eq!(decoded.as_raw(), &rgb);
        }
        Ok(())
    }
}
