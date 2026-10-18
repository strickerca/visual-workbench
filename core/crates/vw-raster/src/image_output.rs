//! One sample-space output transform for preview, export and package pixels.
use crate::{
    AssetResolver, DecodedImage, ExportRequest, ExportedImage, Pixels, RasterError, Region,
    RenderOptions,
};
use serde::{Deserialize, Serialize};
use vw_geom::{ImageOutputFrame, QuarterTurn};
use vw_model::{Id, Project};

/// Embedded export/package mapping; D stays the oriented immutable source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageOutputMetadata {
    pub version: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub crop: Region,
    pub clockwise_degrees: u16,
    pub output_width: u32,
    pub output_height: u32,
    /// Fixed, explicit policy. This is interpolation of encoded source samples,
    /// not a claim of linear-light resampling or an automatic color conversion.
    pub resampling: ImageOutputResampling,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageOutputResampling {
    PremultipliedBilinearSourceSamplesV1,
}
impl ImageOutputMetadata {
    pub fn new(frame: ImageOutputFrame) -> Self {
        let [source_width, source_height] = frame.source_size();
        let [x, y, width, height] = frame.crop();
        let [output_width, output_height] = frame.output_size();
        Self {
            version: 1,
            source_width,
            source_height,
            crop: Region {
                x,
                y,
                width,
                height,
            },
            clockwise_degrees: match frame.rotation() {
                QuarterTurn::Zero => 0,
                QuarterTurn::Clockwise90 => 90,
                QuarterTurn::Half => 180,
                QuarterTurn::Clockwise270 => 270,
            },
            output_width,
            output_height,
            resampling: ImageOutputResampling::PremultipliedBilinearSourceSamplesV1,
        }
    }
    pub fn frame(self) -> Result<ImageOutputFrame, RasterError> {
        if self.version != 1 {
            return Err(RasterError::Unsupported("image output version"));
        }
        let rotation = QuarterTurn::from_degrees(i32::from(self.clockwise_degrees))
            .map_err(|_| RasterError::Invalid("output rotation"))?;
        if self.clockwise_degrees >= 360 {
            return Err(RasterError::Invalid("output rotation"));
        }
        ImageOutputFrame::new(
            [self.source_width, self.source_height],
            [self.crop.x, self.crop.y, self.crop.width, self.crop.height],
            rotation,
            [self.output_width, self.output_height],
        )
        .map_err(|_| RasterError::Invalid("image output grid"))
    }
}

/// Actual retained source allocation, including unused Vec capacity.
pub fn image_resident_bytes(source: &DecodedImage) -> Result<u64, RasterError> {
    let pixels = match &source.pixels {
        Pixels::Rgba8(v) => v.capacity() as u64,
        Pixels::Rgba16(v) => (v.capacity() as u64)
            .checked_mul(2)
            .ok_or(RasterError::Allocation)?,
    };
    pixels
        .checked_add(source.icc.as_ref().map_or(0, |v| v.capacity() as u64))
        .ok_or(RasterError::Allocation)
}
/// Nonallocating admission before ICC parsing or any output sample allocation.
/// Budget includes the complete caller-owned source allocation, one output,
/// copied metadata and admitted ICC parsing. At most 50 million output pixels.
pub fn image_output_estimate(
    source: &DecodedImage,
    frame: ImageOutputFrame,
    region: Region,
) -> Result<u64, RasterError> {
    crate::export::image_shape(source)?;
    if !source.original_available {
        return Err(RasterError::OriginalRequired);
    }
    if frame.source_size() != [source.width, source.height] {
        return Err(RasterError::Invalid("image output source extent"));
    }
    let [width, height] = frame.output_size();
    region.validate(width, height)?;
    if u64::from(width) * u64::from(height) > 50_000_000 {
        return Err(RasterError::Unsupported("image output pixel limit"));
    }
    let output =
        u64::from(region.width) * u64::from(region.height) * u64::from(source.pixels.bit_depth())
            / 2;
    let profile = crate::export::icc_workspace(source.icc.as_ref())?;
    image_resident_bytes(source)?
        .checked_add(output)
        .and_then(|v| v.checked_add(1024 * 1024))
        .and_then(|v| v.checked_add(profile))
        .ok_or(RasterError::Allocation)
}
fn axis(index: u32, source: u32, output: u32) -> ([u32; 2], [u64; 2], u64) {
    let denominator = 2 * u64::from(output);
    let numerator = (2 * i64::from(index) + 1) * i64::from(source) - i64::from(output);
    let whole = numerator.div_euclid(denominator as i64);
    let fraction = numerator.rem_euclid(denominator as i64) as u64;
    let left = whole.clamp(0, i64::from(source) - 1) as u32;
    let right = (whole + 1).clamp(0, i64::from(source) - 1) as u32;
    (
        [left, right],
        [denominator - fraction, fraction],
        denominator,
    )
}
/// Produce a full-resolution region of O; no thumbnail can serve as its input.
/// Exact crop/quarter-turn sampling preserves even hidden RGB under zero alpha.
/// Resizing uses integer rational weights and premultiplied alpha in the source
/// sample space, preserving 8/16-bit storage depth and the exact ICC bytes.
pub fn render_image_output_region(
    source: &DecodedImage,
    frame: ImageOutputFrame,
    region: Region,
    budget: u64,
    cancelled: &dyn Fn() -> bool,
) -> Result<DecodedImage, RasterError> {
    crate::source::check_cancel(cancelled)?;
    crate::source::check_memory(image_output_estimate(source, frame, region)?, budget)?;
    source.validate()?;
    let count = crate::checked_samples(region.width, region.height)?;
    let mut pixels = match &source.pixels {
        Pixels::Rgba8(_) => {
            let mut v = crate::source::reserve::<u8>(count)?;
            v.resize(count, 0);
            Pixels::Rgba8(v)
        }
        Pixels::Rgba16(_) => {
            let mut v = crate::source::reserve::<u16>(count)?;
            v.resize(count, 0);
            Pixels::Rgba16(v)
        }
    };
    let [rw, rh] = frame.rotated_size();
    let [ow, oh] = frame.output_size();
    for y in 0..region.height {
        crate::source::check_cancel(cancelled)?;
        let (ys, wy, dy) = axis(region.y + y, rh, oh);
        for x in 0..region.width {
            let (xs, wx, dx) = axis(region.x + x, rw, ow);
            let at = ((u64::from(y) * u64::from(region.width) + u64::from(x)) * 4) as usize;
            let mut sums = [0u128; 4];
            let mut exact = None;
            let mut multiple = false;
            for (sample_y, weight_y) in ys.into_iter().zip(wy) {
                for (sample_x, weight_x) in xs.into_iter().zip(wx) {
                    let weight = u128::from(weight_x) * u128::from(weight_y);
                    if weight == 0 {
                        continue;
                    }
                    let [sx, sy] = frame
                        .source_pixel(sample_x, sample_y)
                        .map_err(|_| RasterError::Invalid("output sample"))?;
                    let pos =
                        ((u64::from(sy) * u64::from(source.width) + u64::from(sx)) * 4) as usize;
                    if exact.is_none() {
                        exact = Some(pos);
                    } else if exact != Some(pos) {
                        multiple = true;
                    }
                    let a = u128::from(source.pixels.sample(pos + 3));
                    sums[3] += a * weight;
                    for (channel, sum) in sums[..3].iter_mut().enumerate() {
                        *sum += u128::from(source.pixels.sample(pos + channel)) * a * weight;
                    }
                }
            }
            if !multiple {
                let pos = exact.ok_or(RasterError::Invalid("empty output sample"))?;
                for channel in 0..4 {
                    pixels.set_sample(at + channel, source.pixels.sample(pos + channel));
                }
            } else {
                let area = u128::from(dx) * u128::from(dy);
                pixels.set_sample(at + 3, ((sums[3] + area / 2) / area) as u16);
                for (channel, sum) in sums[..3].iter().enumerate() {
                    let sample = (*sum + sums[3] / 2).checked_div(sums[3]).unwrap_or(0);
                    pixels.set_sample(at + channel, sample as u16);
                }
            }
        }
    }
    crate::source::check_cancel(cancelled)?;
    Ok(DecodedImage {
        width: region.width,
        height: region.height,
        pixels,
        icc: source.icc.clone(),
        source_asset: source.source_asset.clone(),
        original_available: source.original_available,
        orientation_applied: source.orientation_applied,
    })
}
/// Full explicit output grid, subject to the same memory/work admission.
pub fn render_image_output(
    source: &DecodedImage,
    frame: ImageOutputFrame,
    budget: u64,
    cancelled: &dyn Fn() -> bool,
) -> Result<DecodedImage, RasterError> {
    let [width, height] = frame.output_size();
    render_image_output_region(
        source,
        frame,
        Region {
            x: 0,
            y: 0,
            width,
            height,
        },
        budget,
        cancelled,
    )
}

/// Whether project-aware transfer needs tonal or output-grid composition.
/// Ordinary Crop objects are guides and do not enable output geometry.
pub fn requires_document_output(
    project: &Project,
    document: &Id,
    marked: bool,
) -> Result<bool, RasterError> {
    project.validate()?;
    project
        .documents
        .get(document)
        .ok_or(RasterError::Invalid("document"))?;
    if project.active_image_output(document)?.is_some() {
        return Ok(true);
    }
    Ok(!marked
        && project.objects.values().any(|object| {
            &object.document_id == document
                && !object.state.hidden
                && matches!(
                    object.state.shape,
                    Some(vw_proto::v1::object_state::Shape::Adjustment(_))
                )
                && object
                    .state
                    .layer_id
                    .as_ref()
                    .and_then(|wire| Id::from_proto(Some(wire)).ok())
                    .and_then(|id| project.layers.get(&id))
                    .is_some_and(|layer| layer.visible && layer.opacity > 0.0)
        }))
}

const MAX_DOCUMENT_OUTPUT_PIXELS: u64 = 50_000_000;

fn pixel_count(width: u32, height: u32) -> Result<u64, RasterError> {
    let count = u64::from(width) * u64::from(height);
    if count == 0 || count > MAX_DOCUMENT_OUTPUT_PIXELS {
        return Err(RasterError::Unsupported("image output pixel limit"));
    }
    Ok(count)
}

fn document_asset<'a>(
    project: &'a Project,
    document: &Id,
) -> Result<&'a vw_proto::v1::AddAsset, RasterError> {
    let definition = &project
        .documents
        .get(document)
        .ok_or(RasterError::Invalid("document"))?
        .definition;
    if !matches!(
        vw_proto::v1::DocumentKind::try_from(definition.kind),
        Ok(vw_proto::v1::DocumentKind::Image | vw_proto::v1::DocumentKind::Capture)
    ) {
        return Err(RasterError::Unsupported(
            "document output requires image or capture",
        ));
    }
    project
        .assets
        .get(&vw_model::AssetId::try_from(
            definition.primary_asset_id.clone(),
        )?)
        .ok_or(RasterError::MissingAsset)
}

fn asset_dimensions(asset: &vw_proto::v1::AddAsset) -> [u32; 2] {
    if asset.orientation >= 5 {
        [asset.height, asset.width]
    } else {
        [asset.width, asset.height]
    }
}

/// Check complete original binding before a crop can hide malformed source
/// dimensions, depth, orientation, profile or sample-buffer length.
fn validate_source_binding(
    source: &DecodedImage,
    project: &Project,
    document: &Id,
) -> Result<(), RasterError> {
    crate::export::image_shape(source)?;
    if !source.original_available {
        return Err(RasterError::OriginalRequired);
    }
    let asset = document_asset(project, document)?;
    if asset.asset_id != source.source_asset.as_str()
        || asset_dimensions(asset) != [source.width, source.height]
        || asset.orientation != u32::from(source.orientation_applied)
        || asset.bit_depth.max(8) != u32::from(source.pixels.bit_depth())
        || asset.icc_profile.as_slice() != source.icc.as_deref().unwrap_or_default()
    {
        return Err(RasterError::Invalid("document source metadata"));
    }
    Ok(())
}

/// Project-only admission before reading or decoding original bytes. Counts
/// encoded bytes, worst-case 16-bit source storage, canonical compositor work,
/// output samples and ICC parsing. Caller-owned project memory is excluded.
pub fn document_output_reservation(
    project: &Project,
    document: &Id,
    options: RenderOptions,
) -> Result<u64, RasterError> {
    let asset = document_asset(project, document)?;
    if !matches!(asset.bit_depth, 0 | 8 | 16) {
        return Err(RasterError::Unsupported("image output source depth"));
    }
    let [width, height] = asset_dimensions(asset);
    let source_pixels = pixel_count(width, height)?;
    let [ow, oh] = project
        .active_image_output(document)?
        .map_or([width, height], |v| v.output_size());
    let output_pixels = pixel_count(ow, oh)?;
    let geometry = crate::stream::geometry_reserve(project, document, options.include_guides)?;
    let profile = crate::export::icc_workspace(
        (!asset.icc_profile.is_empty()).then_some(&asset.icc_profile),
    )?;
    let estimated = source_pixels
        .checked_mul(72)
        .and_then(|v| v.checked_add(output_pixels * 8))
        .and_then(|v| v.checked_add(asset.byte_size))
        .and_then(|v| v.checked_add(32 * 1024 * 1024))
        .and_then(|v| v.checked_add(geometry))
        .and_then(|v| v.checked_add(profile))
        .ok_or(RasterError::Allocation)?;
    crate::source::check_memory(estimated, options.memory_budget_bytes)?;
    project.validate()?;
    Ok(estimated)
}

/// Project-only predecode admission for an editable D-space tile. Output crop,
/// rotation and O dimensions do not change the original canvas reservation.
/// Result images are separately admitted before canonical resolver calls.
pub fn document_editing_region_reservation(
    project: &Project,
    document: &Id,
    options: RenderOptions,
    region: Region,
) -> Result<u64, RasterError> {
    let asset = document_asset(project, document)?;
    if !matches!(asset.bit_depth, 0 | 8 | 16) {
        return Err(RasterError::Unsupported("image output source depth"));
    }
    let [width, height] = asset_dimensions(asset);
    region.validate(width, height)?;
    let source_pixels = pixel_count(width, height)?;
    let region_pixels = pixel_count(region.width, region.height)?;
    let geometry = crate::stream::geometry_reserve(project, document, options.include_guides)?;
    let profile = crate::export::icc_workspace(
        (!asset.icc_profile.is_empty()).then_some(&asset.icc_profile),
    )?;
    let estimated = source_pixels
        .checked_mul(8)
        .and_then(|v| v.checked_add(region_pixels * 64))
        .and_then(|v| v.checked_add(asset.byte_size))
        .and_then(|v| v.checked_add(32 * 1024 * 1024))
        .and_then(|v| v.checked_add(geometry))
        .and_then(|v| v.checked_add(profile))
        .ok_or(RasterError::Allocation)?;
    crate::source::check_memory(estimated, options.memory_budget_bytes)?;
    project.validate()?;
    Ok(estimated)
}

/// Completed output pixels. Canonical objects and originals remain in D;
/// only this owned buffer uses the separately described O grid.
pub struct DocumentOutput {
    /// Composed and transformed original-resolution pixels.
    pub image: DecodedImage,
    /// Enabled output mapping, including an explicitly configured identity.
    pub frame: Option<ImageOutputMetadata>,
}

/// Conservative working-set admission before full document composition.
/// Includes actual retained Vec capacities and separately admits resolver
/// allocations through the existing canonical renderer.
pub fn document_output_estimate(
    source: &DecodedImage,
    project: &Project,
    document: &Id,
    options: RenderOptions,
) -> Result<u64, RasterError> {
    validate_source_binding(source, project, document)?;
    let source_pixels = pixel_count(source.width, source.height)?;
    let frame = project.active_image_output(document)?;
    if frame.is_some_and(|v| v.source_size() != [source.width, source.height]) {
        return Err(RasterError::Invalid("document output source binding"));
    }
    let [width, height] = frame.map_or([source.width, source.height], |v| v.output_size());
    let output_pixels = pixel_count(width, height)?;
    let geometry = crate::stream::geometry_reserve(project, document, options.include_guides)?;
    let profile = crate::export::icc_workspace(source.icc.as_ref())?;
    let resident = image_resident_bytes(source)?;
    source_pixels
        .checked_mul(64)
        .and_then(|v| v.checked_add(output_pixels * 8))
        .and_then(|v| v.checked_add(32 * 1024 * 1024))
        .and_then(|v| v.checked_add(geometry))
        .and_then(|v| v.checked_add(profile))
        .and_then(|v| v.checked_add(resident))
        .ok_or(RasterError::Allocation)
}

/// Render a source-space region of the complete editable canvas. Visible
/// annotations, accepted results and tones use canonical layer order; global
/// output crop, quarter-turn and resize do not affect these D-space pixels.
/// Callers displaying this image must not paint the annotations a second time.
pub fn render_document_editing_region(
    source: &DecodedImage,
    project: &Project,
    document: &Id,
    assets: &impl AssetResolver,
    options: RenderOptions,
    region: Region,
    cancelled: &dyn Fn() -> bool,
) -> Result<DecodedImage, RasterError> {
    render_document_region_mode(
        source, project, document, assets, options, region, true, cancelled,
    )
}

#[allow(clippy::too_many_arguments)] // Shared canonical render mode and explicit source-space region.
fn render_document_region_mode(
    source: &DecodedImage,
    project: &Project,
    document: &Id,
    assets: &impl AssetResolver,
    options: RenderOptions,
    region: Region,
    annotations: bool,
    cancelled: &dyn Fn() -> bool,
) -> Result<DecodedImage, RasterError> {
    crate::source::check_cancel(cancelled)?;
    validate_source_binding(source, project, document)?;
    region.validate(source.width, source.height)?;
    let pixels = pixel_count(region.width, region.height)?;
    let geometry = crate::stream::geometry_reserve(project, document, options.include_guides)?;
    let profile = crate::export::icc_workspace(source.icc.as_ref())?;
    let resident = image_resident_bytes(source)?;
    let estimated = pixels
        .checked_mul(64)
        .and_then(|v| v.checked_add(32 * 1024 * 1024))
        .and_then(|v| v.checked_add(geometry))
        .and_then(|v| v.checked_add(profile))
        .and_then(|v| v.checked_add(resident))
        .ok_or(RasterError::Allocation)?;
    crate::source::check_memory(estimated, options.memory_budget_bytes)?;
    // Validation precedes the crop: a seemingly valid small region must never
    // conceal an invalid original or a malformed canonical output object.
    project.validate()?;
    source.validate()?;
    crate::source::check_cancel(cancelled)?;
    let cropped = copy_document_region(source, region, cancelled)?;
    crate::source::check_cancel(cancelled)?;
    let result = crate::render::render_buffer_mode(
        &cropped,
        project,
        document,
        assets,
        options,
        crate::render::RegionRender {
            origin: (region.x, region.y),
            estimated,
            cancelled,
        },
        annotations,
    )?;
    crate::source::check_cancel(cancelled)?;
    Ok(result)
}

fn copy_document_region(
    source: &DecodedImage,
    region: Region,
    cancelled: &dyn Fn() -> bool,
) -> Result<DecodedImage, RasterError> {
    let pixels = match &source.pixels {
        Pixels::Rgba8(samples) => Pixels::Rgba8(copy_region_samples(
            samples,
            source.width,
            region,
            cancelled,
        )?),
        Pixels::Rgba16(samples) => Pixels::Rgba16(copy_region_samples(
            samples,
            source.width,
            region,
            cancelled,
        )?),
    };
    Ok(DecodedImage {
        width: region.width,
        height: region.height,
        pixels,
        icc: source.icc.clone(),
        source_asset: source.source_asset.clone(),
        original_available: true,
        orientation_applied: source.orientation_applied,
    })
}

fn copy_region_samples<T: Copy>(
    samples: &[T],
    width: u32,
    region: Region,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<T>, RasterError> {
    let mut output =
        crate::source::reserve::<T>(crate::checked_samples(region.width, region.height)?)?;
    // Bounds were checked against the complete source before this helper is
    // called. Check each row so even copying a full 16-bit original can stop.
    for y in region.y..region.y + region.height {
        crate::source::check_cancel(cancelled)?;
        let start = ((u64::from(y) * u64::from(width) + u64::from(region.x)) * 4) as usize;
        let end = start + region.width as usize * 4;
        output.extend_from_slice(
            samples
                .get(start..end)
                .ok_or(RasterError::Invalid("pixel buffer length"))?,
        );
    }
    Ok(output)
}

/// Compose D through the canonical renderer, then apply the single output
/// frame. Clean output retains visible tonal layers and omits annotations and
/// results. Marked output retains all visible content in canonical layer order.
pub fn prepare_document_output(
    source: &DecodedImage,
    project: &Project,
    document: &Id,
    assets: &impl AssetResolver,
    options: RenderOptions,
    marked: bool,
    cancelled: &dyn Fn() -> bool,
) -> Result<DocumentOutput, RasterError> {
    crate::source::check_cancel(cancelled)?;
    let estimated = document_output_estimate(source, project, document, options)?;
    crate::source::check_memory(estimated, options.memory_budget_bytes)?;
    let frame = project.active_image_output(document)?;
    let rendered = render_document_region_mode(
        source,
        project,
        document,
        assets,
        options,
        Region {
            x: 0,
            y: 0,
            width: source.width,
            height: source.height,
        },
        marked,
        cancelled,
    )?;
    crate::source::check_cancel(cancelled)?;
    let image = if let Some(frame) = frame.filter(|v| !v.is_identity()) {
        let external = image_resident_bytes(source)?;
        let available = options
            .memory_budget_bytes
            .checked_sub(external)
            .ok_or(RasterError::Allocation)?;
        render_image_output(&rendered, frame, available, cancelled)
            .map_err(|e| with_external(e, external, options.memory_budget_bytes))?
    } else {
        rendered
    };
    crate::source::check_cancel(cancelled)?;
    Ok(DocumentOutput {
        image,
        frame: frame.map(ImageOutputMetadata::new),
    })
}

/// Export the canonical O grid with the ordinary explicit codec, color, depth
/// and alpha policies. An optional request region is expressed in O pixels;
/// metadata retains the immutable D source extent and the complete D-to-O map.
#[allow(clippy::too_many_arguments)] // Composition plus explicit codec and cancellation policies.
pub fn export_document_output(
    source: &DecodedImage,
    project: &Project,
    document: &Id,
    assets: &impl AssetResolver,
    options: RenderOptions,
    marked: bool,
    request: &ExportRequest,
    cancelled: &dyn Fn() -> bool,
) -> Result<ExportedImage, RasterError> {
    crate::source::check_cancel(cancelled)?;
    if request.memory_budget_bytes != options.memory_budget_bytes {
        return Err(RasterError::Invalid("output compositor budget binding"));
    }
    let budget = request.memory_budget_bytes;
    let composition = document_output_estimate(source, project, document, options)?;
    let frame = project.active_image_output(document)?;
    let [width, height] = frame.map_or([source.width, source.height], |v| v.output_size());
    let external = image_resident_bytes(source)?;
    let profile = crate::export::icc_workspace(source.icc.as_ref())?;
    let mut probe = request.clone();
    probe.memory_budget_bytes = u64::MAX;
    let codec = crate::preflight(width, height, source.pixels.bit_depth(), false, &probe)?;
    let output_bytes = pixel_count(width, height)? * 8;
    let export_peak = external
        .checked_add(output_bytes)
        .and_then(|v| v.checked_add(codec))
        .and_then(|v| v.checked_add(profile))
        .ok_or(RasterError::Allocation)?;
    crate::source::check_memory(export_peak.max(composition), budget)?;
    let output = prepare_document_output(
        source, project, document, assets, options, marked, cancelled,
    )?;
    let external = external
        .checked_add(image_resident_bytes(&output.image)?)
        .ok_or(RasterError::Allocation)?;
    let mut encode_request = request.clone();
    encode_request.memory_budget_bytes = budget
        .checked_sub(external)
        .ok_or(RasterError::Allocation)?;
    crate::source::check_cancel(cancelled)?;
    let encoded = crate::export::export_with_output(&output.image, &encode_request, output.frame)
        .map_err(|e| with_external(e, external, budget))?;
    crate::source::check_cancel(cancelled)?;
    Ok(encoded)
}

fn with_external(error: RasterError, external: u64, budget: u64) -> RasterError {
    match error {
        RasterError::Memory { estimated, .. } => RasterError::Memory {
            estimated: estimated.saturating_add(external),
            budget,
        },
        other => other,
    }
}
