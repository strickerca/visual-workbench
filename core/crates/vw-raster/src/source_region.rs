//! Canonical composition of a bounded D-space region from a verified original.
//! The caller owns source I/O and keeps its source, result and output allocations
//! admitted in the same endpoint pool for the duration of this call.
use crate::{AssetResolver, DecodedImage, RasterError, Region, RenderOptions, SourceInfo};
use vw_model::{Id, Project};
use vw_proto::v1;

fn binding(info: &SourceInfo, project: &Project, document: &Id) -> Result<(), RasterError> {
    let definition = &project
        .documents
        .get(document)
        .ok_or(RasterError::Invalid("document"))?
        .definition;
    if !matches!(
        v1::DocumentKind::try_from(definition.kind),
        Ok(v1::DocumentKind::Image | v1::DocumentKind::Capture)
    ) || definition.primary_asset_id != info.source_asset.as_str()
    {
        return Err(RasterError::Invalid("document source binding"));
    }
    let asset = project
        .assets
        .get(&info.source_asset)
        .ok_or(RasterError::MissingAsset)?;
    let (width, height) = if asset.orientation >= 5 {
        (asset.height, asset.width)
    } else {
        (asset.width, asset.height)
    };
    if (width, height) != (info.width, info.height)
        || asset.bit_depth != u32::from(info.bit_depth)
        || asset.orientation != u32::from(info.orientation_applied)
        || asset.icc_profile.as_slice() != info.icc.as_deref().unwrap_or_default()
    {
        return Err(RasterError::Invalid("document source metadata"));
    }
    Ok(())
}

/// Conservative composition workspace for one original-resolution region.
/// Result-asset decoder and caller-owned input/output/ABI reserves are additional.
pub fn region_composition_estimate(
    info: &SourceInfo,
    project: &Project,
    document: &Id,
    options: RenderOptions,
    region: Region,
) -> Result<u64, RasterError> {
    binding(info, project, document)?;
    region.validate(info.width, info.height)?;
    let pixels = u64::from(region.width) * u64::from(region.height);
    let geometry = crate::stream::geometry_reserve(project, document, options.include_guides)?;
    // Reject a tiny budget before inspecting ICC structures or invoking the
    // semantic profile parser. Once this floor fits, icc_workspace calculates
    // the additional profile allowance without allocating parser state.
    let fixed = pixels
        .checked_mul(64)
        .and_then(|n| n.checked_add(32 * 1024 * 1024))
        .and_then(|n| n.checked_add(geometry))
        .ok_or(RasterError::Allocation)?;
    crate::source::check_memory(fixed, options.memory_budget_bytes)?;
    let profile = crate::export::icc_workspace(info.icc.as_ref())?;
    let estimated = fixed
        .checked_add(profile)
        .and_then(|n| n.checked_add(info.icc.as_ref().map_or(0, |v| v.capacity() as u64)))
        .ok_or(RasterError::Allocation)?;
    crate::source::check_memory(estimated, options.memory_budget_bytes)?;
    info.validate()?;
    project.validate()?;
    Ok(estimated)
}

/// The input is the exact original-resolution region at `region` in the full D
/// grid. The existing layer/tonal/result renderer receives its real D origin.
/// Output crop and O-space dimensions have no bearing on editing visibility.
pub fn compose_document_region(
    source: &DecodedImage,
    full: &SourceInfo,
    project: &Project,
    document: &Id,
    assets: &impl AssetResolver,
    options: RenderOptions,
    region: Region,
    cancelled: &dyn Fn() -> bool,
) -> Result<DecodedImage, RasterError> {
    crate::source::check_cancel(cancelled)?;
    let estimated = region_composition_estimate(full, project, document, options, region)?;
    if (source.width, source.height) != (region.width, region.height)
        || source.source_asset != full.source_asset
        || source.pixels.bit_depth() != full.bit_depth
        || source.icc != full.icc
        || !source.original_available
        || source.orientation_applied != full.orientation_applied
    {
        return Err(RasterError::Invalid("original region binding"));
    }
    source.validate()?;
    crate::source::check_cancel(cancelled)?;
    let result = crate::render::render_buffer_mode(
        source,
        project,
        document,
        assets,
        options,
        crate::render::RegionRender {
            origin: (region.x, region.y),
            estimated,
            cancelled,
        },
        true,
    )?;
    crate::source::check_cancel(cancelled)?;
    Ok(result)
}

/// Deterministic premultiplied box reduction after D-space composition. The
/// final source edge may contain fewer than `factor` samples. This is a
/// committed editing derivative, not a claim about the cache JPEG's filter.
pub fn downsample_composed_box_rgba8(
    image: &DecodedImage,
    factor: u32,
    width: u32,
    height: u32,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>, RasterError> {
    crate::source::check_cancel(cancelled)?;
    if !matches!(factor, 2 | 4)
        || image.width.div_ceil(factor) != width
        || image.height.div_ceil(factor) != height
    {
        return Err(RasterError::Unsupported("bounded composed box level"));
    }
    image.validate()?;
    let crate::Pixels::Rgba8(source) = &image.pixels else {
        return Err(RasterError::Unsupported("composed box depth"));
    };
    let len = crate::checked_samples(width, height)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(len)
        .map_err(|_| RasterError::Allocation)?;
    for y in 0..height {
        crate::source::check_cancel(cancelled)?;
        for x in 0..width {
            let sx = x * factor;
            let sy = y * factor;
            let mut alpha = 0u64;
            let mut color = [0u64; 3];
            let mut samples = 0u64;
            for row in sy..(sy + factor).min(image.height) {
                for col in sx..(sx + factor).min(image.width) {
                    let at =
                        ((u64::from(row) * u64::from(image.width) + u64::from(col)) * 4) as usize;
                    let pixel = source
                        .get(at..at + 4)
                        .ok_or(RasterError::Invalid("composed box source"))?;
                    let a = u64::from(pixel[3]);
                    alpha += a;
                    for channel in 0..3 {
                        color[channel] += u64::from(pixel[channel]) * a;
                    }
                    samples += 1;
                }
            }
            for channel in color {
                output.push(if alpha == 0 {
                    0
                } else {
                    ((channel + alpha / 2) / alpha) as u8
                });
            }
            output.push(((alpha + samples / 2) / samples) as u8);
        }
    }
    Ok(output)
}
