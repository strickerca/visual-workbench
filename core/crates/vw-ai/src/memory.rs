//! Pure admission arithmetic shared by preparation and allocation-free tests.
use crate::{Error, Result};

pub(crate) fn retained_inputs(
    encoded_bytes: u64,
    mask_count: u64,
    tile_bytes: impl IntoIterator<Item = u64>,
) -> Result<u64> {
    // Conservatively reserve each immutable Arc allocation and map entry, even
    // when masks share tile bytes. BTree node packing must not reduce admission.
    let initial = mask_count
        .checked_mul(128)
        .and_then(|value| value.checked_add(encoded_bytes))
        .ok_or(Error::Limit("preparation memory arithmetic"))?;
    tile_bytes.into_iter().try_fold(initial, |total, bytes| {
        total
            .checked_add(bytes)
            .and_then(|value| value.checked_add(128))
            .ok_or(Error::Limit("preparation memory arithmetic"))
    })
}

pub(crate) fn preparation_peak(
    source_pixels: u64,
    crop_and_model_pixels: u64,
    retained: u64,
    stage_overhead: u64,
) -> Result<u64> {
    source_pixels
        .checked_mul(24)
        .and_then(|value| crop_and_model_pixels.checked_mul(48)?.checked_add(value))
        .and_then(|value| value.checked_add(retained))
        .and_then(|value| value.checked_add(stage_overhead))
        .ok_or(Error::Limit("preparation memory arithmetic"))
}

pub(crate) fn retained_prepared(
    source_pixels: u64,
    image_bytes: u64,
    mask_bytes: u64,
    profile_bytes: u64,
) -> Result<u64> {
    // Original RGBA16 plus conservative sparse union storage. Encoded request
    // lengths are the actual retained PNGs, not their pixel-count estimates.
    source_pixels
        .checked_mul(10)
        .and_then(|value| value.checked_add(image_bytes))
        .and_then(|value| value.checked_add(mask_bytes))
        .and_then(|value| value.checked_add(profile_bytes))
        .ok_or(Error::Limit("completion memory arithmetic"))
}

pub(crate) fn completion_peak(
    source_pixels: u64,
    crop_and_model_pixels: u64,
    retained: u64,
    response_bytes: u64,
) -> Result<u64> {
    source_pixels
        .checked_mul(40)
        .and_then(|value| crop_and_model_pixels.checked_mul(48)?.checked_add(value))
        .and_then(|value| value.checked_add(retained))
        .and_then(|value| value.checked_add(response_bytes))
        .and_then(|value| value.checked_add(64 * 1024 * 1024))
        .ok_or(Error::Limit("completion memory arithmetic"))
}
