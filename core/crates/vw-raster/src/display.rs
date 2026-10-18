//! Full-resolution display derivatives. No compositor, encoded output or preview
//! fallback is involved; the caller retains its immutable original on disk.
use crate::{DecodedImage, Pixels, RasterError, export, source};
use vw_model::AssetId;

const MAX_DISPLAY_PIXELS: u64 = 50_000_000;
const FIXED_WORKSPACE_BYTES: u64 = 32 * 1024 * 1024;

/// An oriented, sRGB, 8-bit display derivative of an immutable original.
/// `source_bit_depth` and `source_asset` describe the original, not the derivative.
#[derive(Debug)]
pub struct DisplayImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub icc_profile: Vec<u8>,
    pub source_asset: AssetId,
    pub source_bit_depth: u8,
    pub orientation_applied: u8,
}

/// Consume an already decoded, oriented original and convert its pixels to sRGB.
/// The decoder's encoded-input and decode workspace must be admitted separately
/// and released before calling this function. This working-set estimate covers
/// the resident pixel capacity, the existing color transform's input and output
/// buffers, checked ICC expansion/copies and a conservative fixed workspace.
/// It also covers simultaneous RGBA16 plus RGBA8 reduction, and the native
/// returned Vec / serialized RGBA8 copy. It is not a measured process-RSS limit
/// and does not account for a caller's retained source clones or UI bitmap.
pub fn display_rgba8(
    mut image: DecodedImage,
    memory_budget_bytes: u64,
    assume_untagged_srgb: bool,
    cancelled: &dyn Fn() -> bool,
) -> Result<DisplayImage, RasterError> {
    source::check_cancel(cancelled)?;
    export::image_shape(&image)?;
    if !image.original_available {
        return Err(RasterError::OriginalRequired);
    }
    if !(1..=8).contains(&image.orientation_applied) {
        return Err(RasterError::Invalid("orientation"));
    }
    if u64::from(image.width) * u64::from(image.height) > MAX_DISPLAY_PIXELS {
        return Err(RasterError::Unsupported("display pixel limit"));
    }
    let (resident_pixels, sample_bytes) = match &image.pixels {
        Pixels::Rgba8(samples) => (samples.capacity() as u64, samples.len() as u64),
        Pixels::Rgba16(samples) => (
            (samples.capacity() as u64)
                .checked_mul(2)
                .ok_or(RasterError::Allocation)?,
            (samples.len() as u64)
                .checked_mul(2)
                .ok_or(RasterError::Allocation)?,
        ),
    };
    // convert_profile targets RGB: gray input is at most half a full RGBA
    // buffer, RGB input is one full buffer, and output is one full buffer.
    // The 16-to-8 reduction needs only half of sample_bytes while retaining its
    // RGBA16 input, so it is below this three-buffer conversion reservation.
    let icc_workspace = export::icc_workspace(image.icc.as_ref())?;
    let estimated = sample_bytes
        .checked_mul(2)
        .and_then(|n| n.checked_add(resident_pixels))
        .and_then(|n| n.checked_add(FIXED_WORKSPACE_BYTES))
        .and_then(|n| n.checked_add(icc_workspace))
        .ok_or(RasterError::Allocation)?;
    source::check_memory(estimated, memory_budget_bytes)?;
    // validate parses ICCs; it must follow nonallocating shape/profile admission.
    image.validate()?;
    source::check_cancel(cancelled)?;
    let source_bit_depth = image.pixels.bit_depth();
    image.convert_profile(None, assume_untagged_srgb)?;
    source::check_cancel(cancelled)?;
    let rgba = match image.pixels {
        Pixels::Rgba8(samples) => samples,
        Pixels::Rgba16(samples) => {
            let mut rgba = source::reserve(samples.len())?;
            for chunk in samples.chunks(16_384) {
                source::check_cancel(cancelled)?;
                rgba.extend(
                    chunk
                        .iter()
                        .map(|value| ((u32::from(*value) + 128) / 257) as u8),
                );
            }
            rgba
        }
    };
    source::check_cancel(cancelled)?;
    Ok(DisplayImage {
        width: image.width,
        height: image.height,
        rgba,
        icc_profile: image.icc.ok_or(RasterError::Color)?,
        source_asset: image.source_asset,
        source_bit_depth,
        orientation_applied: image.orientation_applied,
    })
}
