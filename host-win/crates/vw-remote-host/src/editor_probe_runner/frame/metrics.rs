//! Bounded retained-file RGBA8/sRGB decoder and numeric canvas summaries.
use super::*;
use std::io::{BufReader, Cursor};
pub(super) const MAX_PNG: usize = 32 * 1024 * 1024;
const MAX_PIXELS: u64 = 4 * 1024 * 1024;

#[derive(Clone, Serialize)]
pub(super) struct Metrics {
    pub png_sha256: String,
    pub png_blake3: String,
    pub png_bytes: usize,
    pub width: u32,
    pub height: u32,
    pub canvas_pixels: u64,
    pub canvas_offset_x: u32,
    pub canvas_offset_y: u32,
    pub rgba_min: [u8; 4],
    pub rgba_max: [u8; 4],
    pub rgba_sum: [u64; 4],
    pub alpha_not_opaque_pixels: u64,
    pub decoded_rgba_sha256: String,
    pub explicit_srgb: bool,
    pub numeric_semantics_proved: bool,
}
/// A separate canvas-only image; original window/client identity remains in
/// the capture receipt. Never substitute this record for full-client Metrics.
#[derive(Clone, Serialize)]
pub(super) struct CanvasMetrics {
    pub sampling_scope: &'static str,
    pub image: Metrics,
    pub source_client_rect: vw_remote::Rect,
    pub canvas_rect_host: vw_remote::Rect,
    pub crop_in_frame: vw_capture::AlphaRegion,
}
fn dimensions(width: u32, height: u32) -> Result<usize> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(Error::Limit)?;
    if width == 0 || height == 0 || width > 4096 || height > 4096 || pixels > MAX_PIXELS {
        return Err(Error::Limit);
    }
    usize::try_from(pixels.checked_mul(4).ok_or(Error::Limit)?).map_err(|_| Error::Limit)
}
fn decode(bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let size = dimensions(width, height)?;
    if bytes.is_empty() || bytes.len() > MAX_PNG {
        return Err(Error::Limit);
    }
    let mut decoder = png::Decoder::new(BufReader::new(Cursor::new(bytes)));
    decoder.set_limits(png::Limits {
        bytes: 128 * 1024 * 1024,
    });
    let mut reader = decoder.read_info().map_err(|_| Error::Invalid)?;
    let info = reader.info();
    if info.width != width
        || info.height != height
        || info.color_type != png::ColorType::Rgba
        || info.bit_depth != png::BitDepth::Eight
        || info.srgb.is_none()
        || info.icc_profile.is_some()
        || info.animation_control.is_some()
        || info.frame_control.is_some()
        || reader.output_buffer_size() != Some(size)
    {
        return Err(Error::Invalid);
    }
    let mut pixels = vec![0; size];
    let output = reader.next_frame(&mut pixels).map_err(|_| Error::Invalid)?;
    if output.width != width
        || output.height != height
        || output.color_type != png::ColorType::Rgba
        || output.bit_depth != png::BitDepth::Eight
        || output.buffer_size() != size
    {
        return Err(Error::Invalid);
    }
    reader.finish().map_err(|_| Error::Invalid)?;
    let final_info = reader.info();
    if final_info.srgb.is_none()
        || final_info.icc_profile.is_some()
        || final_info.animation_control.is_some()
        || final_info.frame_control.is_some()
    {
        return Err(Error::Invalid);
    }
    Ok(pixels)
}
fn region(canvas: vw_remote::Rect, client: vw_remote::Rect) -> Result<(u32, u32)> {
    if !canvas.inside(client) {
        return Err(Error::TargetChanged);
    }
    Ok((
        u32::try_from(i64::from(canvas.x) - i64::from(client.x)).map_err(|_| Error::Invalid)?,
        u32::try_from(i64::from(canvas.y) - i64::from(client.y)).map_err(|_| Error::Invalid)?,
    ))
}
pub(super) fn measure(
    pin: &Pin,
    receipt: &vw_capture::FrameReceipt,
    canvas: vw_remote::Rect,
    client: vw_remote::Rect,
) -> Result<Metrics> {
    if receipt.width != client.width || receipt.height != client.height {
        return Err(Error::TargetChanged);
    }
    measure_png(
        pin,
        receipt.png_bytes,
        receipt.width,
        receipt.height,
        &receipt.source_asset_id,
        canvas,
        client,
    )
}
fn canvas_mapping(
    receipt: &vw_capture::CanvasFrameReceipt,
    canvas: vw_remote::Rect,
    client: vw_remote::Rect,
) -> Result<()> {
    let actual = vw_capture::Rect {
        x: canvas.x,
        y: canvas.y,
        width: canvas.width,
        height: canvas.height,
    };
    let original = vw_capture::Rect {
        x: client.x,
        y: client.y,
        width: client.width,
        height: client.height,
    };
    actual
        .offset_inside(original)
        .map_err(|_| Error::TargetChanged)?;
    let (x, y) = actual
        .offset_inside(receipt.target.frame)
        .map_err(|_| Error::TargetChanged)?;
    if receipt.target.client != original
        || receipt.source_identity.client_rect != original
        || receipt.canvas_rect_host != actual
        || receipt.width != canvas.width
        || receipt.height != canvas.height
        || receipt.crop_in_frame
            != (vw_capture::AlphaRegion {
                x,
                y,
                width: canvas.width,
                height: canvas.height,
            })
    {
        return Err(Error::TargetChanged);
    }
    Ok(())
}
pub(super) fn measure_canvas(
    pin: &Pin,
    receipt: &vw_capture::CanvasFrameReceipt,
    canvas: vw_remote::Rect,
    client: vw_remote::Rect,
) -> Result<CanvasMetrics> {
    canvas_mapping(receipt, canvas, client)?;
    let image = measure_png(
        pin,
        receipt.png_bytes,
        receipt.width,
        receipt.height,
        &receipt.source_asset_id,
        canvas,
        canvas,
    )?;
    require_opaque_summary(&image)?;
    Ok(CanvasMetrics {
        sampling_scope: "verified_editor_canvas",
        image,
        source_client_rect: client,
        canvas_rect_host: canvas,
        crop_in_frame: receipt.crop_in_frame,
    })
}
fn require_opaque_summary(image: &Metrics) -> Result<()> {
    if image.alpha_not_opaque_pixels != 0
        || image.rgba_min[3] != 255
        || image.rgba_max[3] != 255
        || image.rgba_sum[3] != image.canvas_pixels.checked_mul(255).ok_or(Error::Limit)?
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn summarize(
    pixels: &[u8],
    width: u32,
    height: u32,
    canvas: vw_remote::Rect,
    image_rect: vw_remote::Rect,
) -> Result<Metrics> {
    if pixels.len() != dimensions(width, height)?
        || width != image_rect.width
        || height != image_rect.height
    {
        return Err(Error::TargetChanged);
    }
    let (x, y) = region(canvas, image_rect)?;
    let mut result = Metrics {
        png_sha256: String::new(),
        png_blake3: String::new(),
        png_bytes: 0,
        width,
        height,
        canvas_pixels: u64::from(canvas.width) * u64::from(canvas.height),
        canvas_offset_x: x,
        canvas_offset_y: y,
        rgba_min: [255; 4],
        rgba_max: [0; 4],
        rgba_sum: [0; 4],
        alpha_not_opaque_pixels: 0,
        decoded_rgba_sha256: sha(pixels),
        explicit_srgb: true,
        numeric_semantics_proved: false,
    };
    for row in y..y + canvas.height {
        for column in x..x + canvas.width {
            let at = (u64::from(row) * u64::from(width) + u64::from(column)) * 4;
            let at = usize::try_from(at).map_err(|_| Error::Limit)?;
            let value = pixels.get(at..at + 4).ok_or(Error::Invalid)?;
            for (channel, value) in value.iter().copied().enumerate() {
                result.rgba_min[channel] = result.rgba_min[channel].min(value);
                result.rgba_max[channel] = result.rgba_max[channel].max(value);
                result.rgba_sum[channel] += u64::from(value);
            }
            if value[3] != 255 {
                result.alpha_not_opaque_pixels += 1;
            }
        }
    }
    Ok(result)
}
fn measure_png(
    pin: &Pin,
    png_bytes: u64,
    width: u32,
    height: u32,
    asset_id: &str,
    canvas: vw_remote::Rect,
    image_rect: vw_remote::Rect,
) -> Result<Metrics> {
    pin.verify()?;
    let mut file = pin.file().try_clone()?;
    let size = file.metadata()?.len();
    if size != png_bytes || size == 0 || size > MAX_PNG as u64 {
        return Err(Error::Limit);
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_PNG as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != size {
        return Err(Error::Invalid);
    }
    let hash = blake3::hash(&bytes).to_hex().to_string();
    if hash != asset_id {
        return Err(Error::TargetChanged);
    }
    let pixels = decode(&bytes, width, height)?;
    let mut result = summarize(&pixels, width, height, canvas, image_rect)?;
    result.png_sha256 = sha(&bytes);
    result.png_blake3 = hash;
    result.png_bytes = bytes.len();
    pin.verify()?;
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn png(rgba: bool, srgb: bool) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
            encoder.set_color(if rgba {
                png::ColorType::Rgba
            } else {
                png::ColorType::Rgb
            });
            encoder.set_depth(png::BitDepth::Eight);
            if srgb {
                encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
            }
            let mut writer = encoder.write_header().map_err(|_| Error::Invalid)?;
            writer
                .write_image_data(if rgba {
                    &[1, 2, 3, 255, 4, 5, 6, 255]
                } else {
                    &[1, 2, 3, 4, 5, 6]
                })
                .map_err(|_| Error::Invalid)?;
        }
        Ok(bytes)
    }
    #[test]
    fn decode_requires_actual_rgba8_srgb_and_receipt_dimensions() -> Result<()> {
        let bytes = png(true, true)?;
        assert_eq!(decode(&bytes, 2, 1)?, vec![1, 2, 3, 255, 4, 5, 6, 255]);
        assert_eq!(decode(&bytes, 1, 2), Err(Error::Invalid));
        assert_eq!(decode(&png(false, true)?, 2, 1), Err(Error::Invalid));
        assert_eq!(decode(&png(true, false)?, 2, 1), Err(Error::Invalid));
        Ok(())
    }
    #[test]
    fn negative_origin_canvas_maps_only_inside_actual_client_crop() -> Result<()> {
        let client = vw_remote::Rect {
            x: -100,
            y: -50,
            width: 100,
            height: 80,
        };
        let mut canvas = vw_remote::Rect {
            x: -90,
            y: -30,
            width: 50,
            height: 30,
        };
        assert_eq!(region(canvas, client)?, (10, 20));
        canvas.x = -101;
        assert_eq!(region(canvas, client), Err(Error::TargetChanged));
        Ok(())
    }
    #[test]
    fn decoder_dimension_admission_refuses_giant_and_empty_images() {
        assert_eq!(dimensions(0, 1), Err(Error::Limit));
        assert_eq!(dimensions(4097, 1), Err(Error::Limit));
        assert_eq!(dimensions(4096, 4096), Err(Error::Limit));
        assert_eq!(dimensions(2048, 2048), Ok(16 * 1024 * 1024));
    }

    #[test]
    fn canvas_summary_uses_only_actual_roi_pixels_and_zero_local_offset() -> Result<()> {
        let canvas = vw_remote::Rect {
            x: -20,
            y: -10,
            width: 2,
            height: 1,
        };
        let bytes = png(true, true)?;
        let pixels = decode(&bytes, 2, 1)?;
        let summary = summarize(&pixels, 2, 1, canvas, canvas)?;
        require_opaque_summary(&summary)?;
        assert_eq!((summary.canvas_offset_x, summary.canvas_offset_y), (0, 0));
        assert_eq!(summary.canvas_pixels, 2);
        assert_eq!(summary.rgba_sum, [5, 7, 9, 510]);
        assert!(!summary.numeric_semantics_proved);
        let source = vw_remote::Rect {
            x: -30,
            y: -20,
            width: 10,
            height: 10,
        };
        assert!(summarize(&pixels, 2, 1, canvas, source).is_err());
        Ok(())
    }
    #[test]
    fn decoded_canvas_alpha_remains_exact_255_or_refused() -> Result<()> {
        let canvas = vw_remote::Rect {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
        };
        let mut pixels = decode(&png(true, true)?, 2, 1)?;
        pixels[7] = 228;
        let summary = summarize(&pixels, 2, 1, canvas, canvas)?;
        assert_eq!(summary.alpha_not_opaque_pixels, 1);
        assert_eq!(summary.rgba_min[3], 228);
        assert_eq!(require_opaque_summary(&summary), Err(Error::Invalid));
        assert_eq!(pixels[7], 228);
        Ok(())
    }
}
