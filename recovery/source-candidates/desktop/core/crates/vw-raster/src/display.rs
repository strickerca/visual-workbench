//! Bounded color-tagged display derivatives. Never an original/export encoder.
use crate::{DecodedImage, Pixels, RasterError, Region, checked_samples};
use crate::pixels::{admit_icc, parse_color_profile};

const MIB: u64 = 1024 * 1024;
pub const MAX_DISPLAY_PIXELS: u64 = 4_194_304;
/// Matrix/TRC transform workspace, parsed profiles and destination encoding.
/// This is deliberately separate from source, crop and foreign image copies.
pub const DISPLAY_COLOR_WORK: u64 = 48 * MIB;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplaySpace { Srgb, DisplayP3 }
#[derive(Clone, Copy, Debug)]
pub struct DisplayOptions {
    pub space: DisplaySpace,
    pub assume_untagged_srgb: bool,
    /// Display-only 16→8 conversion; original and export settings are untouched.
    pub allow_depth_reduction: bool,
    pub memory_budget_bytes: u64,
    /// Retained encoded original, project snapshot and other caller allocations.
    pub retained_bytes: u64,
}
#[derive(Debug)]
pub struct DisplayFrame {
    pub region: Region,
    pub space: DisplaySpace,
    /// Straight-alpha RGBA8, encoded in `space`, not necessarily sRGB.
    pub rgba: Vec<u8>,
    pub icc: Vec<u8>,
    pub source_asset: vw_model::AssetId,
    pub source_bit_depth: u8,
    pub assumed_srgb: bool,
    pub depth_reduced: bool,
    pub estimated_peak_bytes: u64,
}

fn add(a: u64, b: u64) -> Result<u64, RasterError> { a.checked_add(b).ok_or(RasterError::Allocation) }
fn mul(a: u64, b: u64) -> Result<u64, RasterError> { a.checked_mul(b).ok_or(RasterError::Allocation) }

/// Nonallocating admission happens before `validate`, ICC parsing or pixel cloning.
pub fn display_peak(source: &DecodedImage, region: Region, options: DisplayOptions) -> Result<u64, RasterError> {
    if options.memory_budget_bytes == 0 || options.memory_budget_bytes > 512 * MIB {
        return Err(RasterError::Invalid("display memory budget"));
    }
    if !source.original_available { return Err(RasterError::OriginalRequired); }
    if source.pixels.len() != checked_samples(source.width, source.height)? || !(1..=8).contains(&source.orientation_applied) {
        return Err(RasterError::Invalid("display source shape"));
    }
    region.validate(source.width, source.height)?;
    let count = mul(u64::from(region.width), u64::from(region.height))?;
    if count > MAX_DISPLAY_PIXELS { return Err(RasterError::Unsupported("display region exceeds 4 megapixels; request tiles")); }
    if source.pixels.bit_depth() != 8 && !options.allow_depth_reduction { return Err(RasterError::Depth); }
    let source_bytes = match &source.pixels {
        Pixels::Rgba8(p) => p.capacity() as u64,
        Pixels::Rgba16(p) => mul(p.capacity() as u64, 2)?,
    };
    let profile = match &source.icc {
        Some(p) => add(mul(p.capacity() as u64, 4)?, admit_icc(p)?)?,
        None if options.assume_untagged_srgb => 0,
        None => return Err(RasterError::Unsupported("untagged display source; explicitly assume sRGB")),
    };
    // RGBA16 crop + transform input/output (24N), final8 (4N), JNI/owned
    // response + upload + native renderer copies (20N). Source is separate.
    let peak = add(add(add(options.retained_bytes, source_bytes)?, profile)?, add(mul(count, 48)?, DISPLAY_COLOR_WORK)?)?;
    if peak > options.memory_budget_bytes { return Err(RasterError::Memory { estimated: peak, budget: options.memory_budget_bytes }); }
    Ok(peak)
}

/// The bounded wide-display route admits matrix/TRC RGB/gray profiles. Existing
/// export conversion remains responsible for other supported ICC LUT profiles.
fn matrix_profile(bytes: &[u8]) -> Result<(), RasterError> {
    admit_icc(bytes)?;
    let count = u32::from_be_bytes(bytes.get(128..132).ok_or(RasterError::Color)?.try_into().map_err(|_| RasterError::Color)?) as usize;
    for i in 0..count {
        let tag = bytes.get(132 + 12 * i..136 + 12 * i).ok_or(RasterError::Color)?;
        if matches!(tag, b"A2B0" | b"A2B1" | b"A2B2" | b"B2A0" | b"B2A1" | b"B2A2" | b"D2B0" | b"D2B1" | b"D2B2" | b"D2B3" | b"B2D0" | b"B2D1" | b"B2D2" | b"B2D3") {
            return Err(RasterError::Unsupported("display ICC lookup-table profile; use explicit sRGB export"));
        }
    }
    let parsed = parse_color_profile(bytes)?;
    if !matches!(parsed.color_space, moxcms::DataColorSpace::Rgb | moxcms::DataColorSpace::Gray) {
        return Err(RasterError::Color);
    }
    Ok(())
}
pub fn display_profile(space: DisplaySpace) -> Result<Vec<u8>, RasterError> {
    let profile = match space { DisplaySpace::Srgb => moxcms::ColorProfile::new_srgb(), DisplaySpace::DisplayP3 => moxcms::ColorProfile::new_display_p3() };
    let mut bytes = profile.encode().map_err(|_| RasterError::Color)?;
    bytes.get_mut(24..36).ok_or(RasterError::Color)?.copy_from_slice(&[7,208,0,1,0,1,0,0,0,0,0,0]);
    Ok(bytes)
}
pub fn display_region(source: &DecodedImage, region: Region, options: DisplayOptions, cancel: &dyn Fn() -> bool) -> Result<DisplayFrame, RasterError> {
    if cancel() { return Err(RasterError::Cancelled); }
    let peak = display_peak(source, region, options)?;
    if let Some(profile) = &source.icc { matrix_profile(profile)?; }
    source.validate()?;
    if cancel() { return Err(RasterError::Cancelled); }
    let profile = display_profile(options.space)?;
    let mut crop = source.crop(region)?;
    crop.convert_profile(Some(&profile), options.assume_untagged_srgb)?;
    if cancel() { return Err(RasterError::Cancelled); }
    let rgba = match crop.pixels {
        Pixels::Rgba8(p) => p,
        Pixels::Rgba16(p) => {
            let mut out = Vec::new(); out.try_reserve_exact(p.len()).map_err(|_| RasterError::Allocation)?;
            for row in p.chunks(region.width as usize * 4) {
                if cancel() { return Err(RasterError::Cancelled); }
                out.extend(row.iter().map(|v| ((u32::from(*v) + 128) / 257) as u8));
            }
            out
        }
    };
    Ok(DisplayFrame { region, space: options.space, rgba, icc: profile, source_asset: source.source_asset.clone(),
        source_bit_depth: source.pixels.bit_depth(), assumed_srgb: source.icc.is_none(), depth_reduced: source.pixels.bit_depth() > 8, estimated_peak_bytes: peak })
}
