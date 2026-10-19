use super::*;
use std::io::Write;
use vw_model::AssetId;
const ENCODED: usize = 64 * 1024 * 1024;
struct Output<'a> {
    bytes: Vec<u8>,
    cancel: &'a Cancellation,
}
impl Write for Output<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.cancel.is_cancelled() {
            return Err(std::io::Error::other("cancelled"));
        }
        let next = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|n| *n <= ENCODED)
            .ok_or_else(|| std::io::Error::other("encoded limit"))?;
        if next > self.bytes.capacity() {
            return Err(std::io::Error::other("allocation admission"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) fn preview_png(
    background: &CaptureBackground,
    cancel: &Cancellation,
) -> WorkflowResult<Vec<u8>> {
    let frame = background.info();
    let pixels = background.borrowed_pixels();
    let expected =
        vw_stream::rgba_bytes(frame.width, frame.height).map_err(|_| WorkflowError::Invalid)?;
    if pixels.len() != expected {
        return Err(WorkflowError::Invalid);
    }
    encode(pixels, frame.width, frame.height, cancel)
}
fn encode(
    pixels: &[u8],
    width: u32,
    height: u32,
    cancel: &Cancellation,
) -> WorkflowResult<Vec<u8>> {
    cancel.check()?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(ENCODED)
        .map_err(|_| WorkflowError::Limit)?;
    if bytes.capacity() > ENCODED {
        return Err(WorkflowError::Limit);
    }
    let mut output = Output { bytes, cancel };
    let result = (|| -> std::result::Result<(), png::EncodingError> {
        let mut encoder = png::Encoder::new(&mut output, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        encoder.set_compression(png::Compression::Fast);
        encoder.set_filter(png::Filter::Sub);
        let mut writer = encoder.write_header()?;
        {
            let mut rows = writer.stream_writer()?;
            for row in pixels.chunks_exact(width as usize * 4) {
                rows.write_all(row)?;
            }
            rows.finish()?;
        }
        writer.finish()
    })();
    cancel.check()?;
    result.map_err(|_| WorkflowError::Limit)?;
    Ok(output.bytes)
}
/// Raster owns the standard-profile normalization. This tiny fixed image asks
/// that authority for its exact canonical ICC bytes without decoding the frame.
fn profile(cancel: &Cancellation) -> WorkflowResult<Vec<u8>> {
    cancel.check()?;
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder.write_header().map_err(|_| WorkflowError::Invalid)?;
        writer
            .write_image_data(&[0, 0, 0, 255])
            .map_err(|_| WorkflowError::Invalid)?;
        writer.finish().map_err(|_| WorkflowError::Invalid)?;
    }
    let decoded = vw_raster::decode(
        &bytes,
        vw_raster::DecodeLimits {
            max_encoded_bytes: 4096,
            max_pixels: 1,
            max_memory_bytes: 8 * 1024 * 1024,
        },
    )
    .map_err(|_| {
        if cancel.is_cancelled() {
            WorkflowError::Cancelled
        } else {
            WorkflowError::Invalid
        }
    })?;
    cancel.check()?;
    decoded.icc.ok_or(WorkflowError::Invalid)
}
pub(super) fn asset(
    bytes: &[u8],
    frame: &StreamFrameInfo,
    cancel: &Cancellation,
) -> WorkflowResult<pb::AddAsset> {
    if bytes.is_empty() || bytes.len() > ENCODED {
        return Err(WorkflowError::Limit);
    }
    definition(AssetId::hash(bytes), bytes.len() as u64, frame, cancel)
}
pub(super) fn definition(
    id: AssetId,
    bytes: u64,
    frame: &StreamFrameInfo,
    cancel: &Cancellation,
) -> WorkflowResult<pb::AddAsset> {
    if bytes == 0 || bytes > ENCODED as u64 {
        return Err(WorkflowError::Limit);
    }
    Ok(pb::AddAsset {
        asset_id: id.to_string(),
        format: "png".into(),
        width: frame.width,
        height: frame.height,
        orientation: 1,
        bit_depth: 8,
        has_alpha: false,
        color_space: "ICC".into(),
        icc_profile: profile(cancel)?,
        byte_size: bytes,
        source: "capture".into(),
        captured_at_ms: frame.captured_at_ms,
        metadata_json: "{}".into(),
    })
}
