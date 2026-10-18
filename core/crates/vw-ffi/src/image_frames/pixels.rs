//! Borrowed worker adapters for the one canonical D-to-O pixel pipeline.
use super::*;
use crate::{
    CoreError, ExportOptions, ExportResult, ImageFormat, SelectionError, project::ProjectState,
};
use std::time::{SystemTime, UNIX_EPOCH};
use vw_raster::{AssetResolver, DecodedImage, ExportRequest, RasterError, Region, RenderOptions};

const MIB: u64 = 1024 * 1024;
const MAX_ENCODED: u64 = 64 * MIB;

fn add(a: u64, b: u64) -> WorkflowResult<u64> {
    a.checked_add(b).ok_or(WorkflowError::Limit)
}
fn count(width: u32, height: u32) -> WorkflowResult<u64> {
    let value = u64::from(width) * u64::from(height);
    if value == 0 || value > 50_000_000 {
        return Err(WorkflowError::Limit);
    }
    Ok(value)
}
fn asset_pixels(asset: &pb::AddAsset) -> WorkflowResult<u64> {
    count(asset.width, asset.height)
}
fn memory(estimated: u64, budget: u64) -> WorkflowResult<()> {
    if estimated > budget {
        Err(WorkflowError::Memory { estimated, budget })
    } else {
        Ok(())
    }
}
fn raster(error: RasterError) -> WorkflowError {
    match error {
        RasterError::Memory { estimated, budget } => WorkflowError::Memory { estimated, budget },
        RasterError::Cancelled => WorkflowError::Cancelled,
        RasterError::MissingAsset | RasterError::OriginalRequired => WorkflowError::Missing,
        RasterError::Io => WorkflowError::Storage,
        RasterError::Allocation
        | RasterError::EncodedLimit { .. }
        | RasterError::ScratchLimit { .. } => WorkflowError::Limit,
        _ => WorkflowError::Invalid,
    }
}
fn raster_in(error: RasterError, budget: u64, available: u64) -> WorkflowError {
    match raster(error) {
        WorkflowError::Memory { estimated, .. } => WorkflowError::Memory {
            estimated: estimated.saturating_add(budget.saturating_sub(available)),
            budget,
        },
        other => other,
    }
}
fn selection_raster(error: SelectionError) -> RasterError {
    match error {
        SelectionError::Memory { estimated, budget } => RasterError::Memory { estimated, budget },
        SelectionError::Cancelled => RasterError::Cancelled,
        SelectionError::Storage => RasterError::Io,
        SelectionError::Limit | SelectionError::EncodedLimit => {
            RasterError::EncodedLimit { limit: MAX_ENCODED }
        }
        _ => RasterError::MissingAsset,
    }
}
fn decode_work(asset: &pb::AddAsset) -> WorkflowResult<u64> {
    if asset.byte_size == 0 || asset.byte_size > MAX_ENCODED {
        return Err(WorkflowError::Limit);
    }
    // Full decode/orientation and checked profile copies; the floor also
    // admits the existing BMP/TIFF adapter before its codec starts.
    add(
        add(
            asset_pixels(asset)?
                .checked_mul(40)
                .ok_or(WorkflowError::Limit)?,
            64 * MIB,
        )?,
        asset.byte_size,
    )
}
fn resolver_work(
    project: &Project,
    document: &Id,
    marked: bool,
    cancel: &Cancellation,
) -> WorkflowResult<u64> {
    if !marked {
        return Ok(0);
    }
    let mut peak = 0;
    for object in project
        .objects
        .values()
        .filter(|v| &v.document_id == document)
    {
        cancel.check()?;
        let Some(pb::object_state::Shape::ResultId(result)) = &object.state.shape else {
            continue;
        };
        let value = project
            .results
            .get(&Id::from_proto(Some(result))?)
            .ok_or(WorkflowError::Missing)?;
        let asset_id = if value.definition.composite_asset_id.is_empty() {
            &value.definition.output_asset_id
        } else {
            &value.definition.composite_asset_id
        };
        let asset = project
            .assets
            .get(&AssetId::try_from(asset_id.clone())?)
            .ok_or(WorkflowError::Missing)?;
        // The canonical renderer checks estimated+resolver before requesting a
        // decode. Pass this allowance through in its complete working budget.
        let profile = if asset.icc_profile.is_empty() {
            0
        } else {
            72 * MIB
        };
        let render = add(
            add(
                add(
                    asset_pixels(asset)?
                        .checked_mul(32)
                        .ok_or(WorkflowError::Limit)?,
                    asset.byte_size,
                )?,
                32 * MIB,
            )?,
            profile,
        )?;
        peak = peak.max(decode_work(asset)?.max(render));
    }
    Ok(peak)
}
#[derive(Clone, Copy)]
enum Mode<'a> {
    Output {
        command: Option<&'a ImageFrameCommand>,
        marked: bool,
    },
    Editing {
        region: Region,
    },
}
struct Working {
    available: u64,
    extra: u64,
    display: u64,
    output_size: [u32; 2],
}
fn options(budget: u64, assume: bool) -> RenderOptions {
    RenderOptions {
        memory_budget_bytes: budget,
        include_guides: false,
        assume_untagged_srgb: assume,
    }
}
#[allow(clippy::too_many_arguments)]
fn working(
    state: &ProjectState,
    document: &Id,
    mode: Mode<'_>,
    publication: Option<Region>,
    request: Option<&ExportRequest>,
    assume: bool,
    budget: u64,
    cancel: &Cancellation,
) -> WorkflowResult<Working> {
    cancel.check()?;
    let mutation = matches!(
        mode,
        Mode::Output {
            command: Some(_),
            ..
        }
    );
    // Canonical/history/hash/host copies are metered before project validation,
    // visible-state hashing, draft simulation or any encoded/pixel allocation.
    admit(state, budget, mutation, FRAME_WORK)?;
    let canonical = canonical_workspace(state, budget - FRAME_WORK, mutation)?;
    let available = budget
        .checked_sub(add(canonical, FRAME_WORK)?)
        .ok_or(WorkflowError::Limit)?;
    let project = state.project()?;
    let asset = bounds(project, document)?;
    let source_size = dimensions(asset);
    let (reservation, output_size, marked, new_rows) = match mode {
        Mode::Output { command, marked } => {
            let original = project.active_image_output(document)?;
            let frame = match command {
                Some(ImageFrameCommand::Set {
                    settings, enabled, ..
                }) => {
                    let value = settings.frame(source_size)?;
                    enabled.then_some(value)
                }
                Some(ImageFrameCommand::Remove { .. }) => None,
                None => original,
            };
            let original_size = original.map_or(source_size, |v| v.output_size());
            let output_size = frame.map_or(source_size, |v| v.output_size());
            // Probe only the nonallocating raster reservation. A draft Remove
            // or smaller Set must not inherit a huge current O allocation.
            let reservation = vw_raster::document_output_reservation(
                project,
                document,
                options(u64::MAX, assume),
            )
            .map_err(raster)?;
            let original_bytes = count(original_size[0], original_size[1])?
                .checked_mul(8)
                .ok_or(WorkflowError::Limit)?;
            let next_bytes = count(output_size[0], output_size[1])?
                .checked_mul(8)
                .ok_or(WorkflowError::Limit)?;
            let reservation = add(
                reservation
                    .checked_sub(original_bytes)
                    .ok_or(WorkflowError::Limit)?,
                next_bytes,
            )?;
            (
                reservation,
                output_size,
                marked,
                if command.is_some() { 64 } else { 0 },
            )
        }
        Mode::Editing { region } => {
            region
                .validate(source_size[0], source_size[1])
                .map_err(|_| WorkflowError::Invalid)?;
            let reservation = vw_raster::document_editing_region_reservation(
                project,
                document,
                options(u64::MAX, assume),
                region,
            )
            .map_err(raster)?;
            (reservation, source_size, true, 0)
        }
    };
    let render = add(
        add(
            reservation,
            resolver_work(project, document, marked, cancel)?,
        )?,
        new_rows,
    )?;
    let mut required = render.max(decode_work(asset)?);
    let mut display = 0;
    let mut publication_copies = 0;
    if let Some(region) = publication {
        region
            .validate(output_size[0], output_size[1])
            .map_err(|_| WorkflowError::Invalid)?;
        let published_pixels = count(region.width, region.height)?;
        let profile = if asset.icc_profile.is_empty() {
            0
        } else {
            72 * MIB
        };
        display = add(
            add(
                published_pixels
                    .checked_mul(32)
                    .ok_or(WorkflowError::Limit)?,
                32 * MIB,
            )?,
            profile,
        )?;
        let retained_pixels = match mode {
            Mode::Output { .. } => count(output_size[0], output_size[1])?,
            Mode::Editing { .. } => published_pixels,
        };
        let retained = add(
            retained_pixels.checked_mul(8).ok_or(WorkflowError::Limit)?,
            asset.icc_profile.len() as u64,
        )?;
        required = required.max(add(display, retained)?);
        // Complete native reply, generated transfer, common owner and bitmap
        // publication may coexist, including a full source-sized background.
        publication_copies = published_pixels
            .checked_mul(16)
            .ok_or(WorkflowError::Limit)?;
    }
    if let Some(request) = request {
        // Policies/region are validated before decoding. Actual alpha and ICC
        // checks remain with the canonical codec after composition.
        let codec = vw_raster::preflight(
            output_size[0],
            output_size[1],
            asset.bit_depth as u8,
            false,
            request,
        )
        .map_err(raster)?;
        let output_pixels = count(output_size[0], output_size[1])?;
        let retained = output_pixels.checked_mul(8).ok_or(WorkflowError::Limit)?;
        let retained_source = asset_pixels(asset)?
            .checked_mul(8)
            .ok_or(WorkflowError::Limit)?;
        let profile = if asset.icc_profile.is_empty() {
            0
        } else {
            72 * MIB
        };
        let mut export_peak = add(add(add(codec, retained)?, retained_source)?, profile)?;
        if crate::format_images::external(request.format) {
            let region = request.region.unwrap_or(Region {
                x: 0,
                y: 0,
                width: output_size[0],
                height: output_size[1],
            });
            let bound = add(
                add(
                    count(region.width, region.height)?
                        .checked_mul(8)
                        .ok_or(WorkflowError::Limit)?,
                    asset.icc_profile.len() as u64,
                )?,
                MIB,
            )?;
            // The existing buffered format adapter retains its staged bytes
            // plus four bounded native/generated publication representations.
            export_peak = add(
                export_peak,
                bound.checked_mul(4).ok_or(WorkflowError::Limit)?,
            )?;
        }
        required = required.max(export_peak);
    }
    let extra = add(add(required, publication_copies)?, FRAME_WORK)?;
    // Reuse the reviewed mutation/canonical meter, including the newly
    // authored transaction's separate borrowed meter before private cloning.
    admit(state, budget, mutation, extra)?;
    memory(add(required, publication_copies)?, available)?;
    cancel.check()?;
    Ok(Working {
        available,
        extra,
        display,
        output_size,
    })
}
struct Assets<'a> {
    state: &'a ProjectState,
    cancel: &'a Cancellation,
}
impl AssetResolver for Assets<'_> {
    fn image(&self, value: &str) -> Result<DecodedImage, RasterError> {
        if self.cancel.is_cancelled() {
            return Err(RasterError::Cancelled);
        }
        let id = AssetId::try_from(value.to_owned()).map_err(|_| RasterError::MissingAsset)?;
        let asset = self
            .state
            .project()
            .map_err(|_| RasterError::MissingAsset)?
            .assets
            .get(&id)
            .ok_or(RasterError::MissingAsset)?;
        let work =
            decode_work(asset).map_err(|_| RasterError::EncodedLimit { limit: MAX_ENCODED })?;
        // Reuse the existing bounded, no-follow, complete-BLAKE3 blob reader.
        let bytes = crate::masks::ai_bridge::blob(
            self.state,
            &id,
            asset.byte_size,
            MAX_ENCODED,
            self.cancel,
        )
        .map_err(selection_raster)?;
        let budget = work
            .checked_sub(bytes.capacity() as u64)
            .ok_or(RasterError::Allocation)?;
        let image = crate::format_images::decode(
            &bytes,
            vw_raster::DecodeLimits {
                max_encoded_bytes: MAX_ENCODED as usize,
                max_pixels: 50_000_000,
                max_memory_bytes: budget,
            },
            &|| self.cancel.is_cancelled(),
        )?;
        let [width, height] = dimensions(asset);
        if image.source_asset != id
            || !image.original_available
            || image.width != width
            || image.height != height
            || u32::from(image.orientation_applied) != asset.orientation
            || u32::from(image.pixels.bit_depth()) != asset.bit_depth
            || image.icc.as_deref().unwrap_or_default() != asset.icc_profile.as_slice()
        {
            return Err(RasterError::Invalid("canonical original metadata"));
        }
        if self.cancel.is_cancelled() {
            return Err(RasterError::Cancelled);
        }
        Ok(image)
    }
}
fn draft(
    state: &ProjectState,
    document: &Id,
    command: Option<ImageFrameCommand>,
    working: &Working,
    budget: u64,
    cancel: &Cancellation,
) -> WorkflowResult<Option<vw_ops::HostSequencer>> {
    let Some(command) = command else {
        return Ok(None);
    };
    let device = state.store()?.local_device()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| WorkflowError::Invalid)?
        .as_millis();
    let metadata = WorkflowMetadata {
        transaction_id: "018bcfe5-6800-7ffe-bffe-ffffffffffff".into(),
        device_id: device.to_string(),
        first_lamport: state.history.next_lamport,
        created_at_ms: i64::try_from(now).map_err(|_| WorkflowError::Limit)?,
    };
    metadata.validate()?;
    let (mut transaction, _) = plan::transaction(state, document, &metadata, command)?;
    admit_transaction(state, &transaction, budget, working.extra)?;
    cancel.check()?;
    let mut host = vw_ops::HostSequencer::new(state.project()?.clone(), device.clone())
        .map_err(plan::operation_error)?;
    // Visible state can include offline pending operations. This private host
    // starts at zero, independently of the accepted store's revision sequence.
    transaction.base_revision = Some(host.revision().map_err(plan::operation_error)?);
    host.submit(transaction, &device, metadata.created_at_ms)
        .map_err(plan::operation_error)?;
    cancel.check()?;
    Ok(Some(host))
}
fn region(value: crate::SelectionRegion) -> Region {
    Region {
        x: value.x,
        y: value.y,
        width: value.width,
        height: value.height,
    }
}
fn tile(value: Region) -> WorkflowResult<()> {
    if value.width == 0 || value.height == 0 || value.width > 512 || value.height > 512 {
        Err(WorkflowError::Invalid)
    } else {
        Ok(())
    }
}
fn consent(asset: &pb::AddAsset, assume: bool) -> WorkflowResult<()> {
    if asset.icc_profile.is_empty() && !assume {
        Err(WorkflowError::Invalid)
    } else {
        Ok(())
    }
}
fn display(
    image: DecodedImage,
    working: &Working,
    assume: bool,
    budget: u64,
    cancel: &Cancellation,
) -> WorkflowResult<vw_raster::DisplayImage> {
    cancel.check()?;
    let result =
        vw_raster::display_rgba8(image, working.display, assume, &|| cancel.is_cancelled())
            .map_err(|e| raster_in(e, budget, working.display))?;
    cancel.check()?;
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn preview(
    state: &ProjectState,
    binding: WorkflowBinding,
    command: Option<ImageFrameCommand>,
    region: crate::SelectionRegion,
    marked: bool,
    assume: bool,
    budget: u64,
    cancel: &Cancellation,
) -> WorkflowResult<ImageFramePreview> {
    let region = self::region(region);
    tile(region)?;
    if let Some(value) = &command {
        plan::validate(value)?;
    }
    let document = id(&binding.document_id)?;
    let working = working(
        state,
        &document,
        Mode::Output {
            command: command.as_ref(),
            marked,
        },
        Some(region),
        None,
        assume,
        budget,
        cancel,
    )?;
    check_binding(state, &binding)?;
    let source_asset_id = bounds(state.project()?, &document)?.asset_id.clone();
    consent(bounds(state.project()?, &document)?, assume)?;
    let draft = draft(state, &document, command, &working, budget, cancel)?;
    let project = draft
        .as_ref()
        .map_or(state.project()?, vw_ops::HostSequencer::project);
    let assets = Assets { state, cancel };
    let source = assets
        .image(&source_asset_id)
        .map_err(|e| raster_in(e, budget, working.available))?;
    cancel.check()?;
    let output = vw_raster::prepare_document_output(
        &source,
        project,
        &document,
        &assets,
        options(working.available, assume),
        marked,
        &|| cancel.is_cancelled(),
    )
    .map_err(|e| raster_in(e, budget, working.available))?;
    cancel.check()?;
    let frame = project
        .active_image_output(&document)?
        .map(ImageFrameSettings::from_frame);
    if [output.image.width, output.image.height] != working.output_size {
        return Err(WorkflowError::Invalid);
    }
    drop(source);
    drop(draft);
    let cropped = output.image.crop(region).map_err(raster)?;
    drop(output);
    let display = display(cropped, &working, assume, budget, cancel)?;
    Ok(ImageFramePreview {
        binding,
        source_asset_id,
        frame,
        output_width: working.output_size[0],
        output_height: working.output_size[1],
        x: region.x,
        y: region.y,
        width: region.width,
        height: region.height,
        rgba: display.rgba,
    })
}
fn editing_pixels(
    state: &ProjectState,
    binding: &WorkflowBinding,
    region: Region,
    assume: bool,
    budget: u64,
    cancel: &Cancellation,
) -> WorkflowResult<(vw_raster::DisplayImage, String)> {
    let document = id(&binding.document_id)?;
    let working = working(
        state,
        &document,
        Mode::Editing { region },
        Some(region),
        None,
        assume,
        budget,
        cancel,
    )?;
    check_binding(state, binding)?;
    let asset = bounds(state.project()?, &document)?;
    consent(asset, assume)?;
    let source_asset_id = asset.asset_id.clone();
    let assets = Assets { state, cancel };
    let source = assets
        .image(&source_asset_id)
        .map_err(|e| raster_in(e, budget, working.available))?;
    cancel.check()?;
    let rendered = vw_raster::render_document_editing_region(
        &source,
        state.project()?,
        &document,
        &assets,
        options(working.available, assume),
        region,
        &|| cancel.is_cancelled(),
    )
    .map_err(|e| raster_in(e, budget, working.available))?;
    cancel.check()?;
    drop(source);
    Ok((
        display(rendered, &working, assume, budget, cancel)?,
        source_asset_id,
    ))
}
pub(super) fn editing(
    state: &ProjectState,
    binding: WorkflowBinding,
    region: crate::SelectionRegion,
    assume: bool,
    budget: u64,
    cancel: &Cancellation,
) -> WorkflowResult<ImageEditingPreview> {
    let region = self::region(region);
    tile(region)?;
    let (display, source_asset_id) =
        editing_pixels(state, &binding, region, assume, budget, cancel)?;
    Ok(ImageEditingPreview {
        binding,
        source_asset_id,
        x: region.x,
        y: region.y,
        width: region.width,
        height: region.height,
        rgba: display.rgba,
    })
}
pub(super) fn background(
    state: &ProjectState,
    binding: WorkflowBinding,
    assume: bool,
    budget: u64,
    cancel: &Cancellation,
) -> WorkflowResult<ImageEditingBackground> {
    let document = id(&binding.document_id)?;
    let [width, height] = dimensions(bounds(state.project()?, &document)?);
    let (display, source_asset_id) = editing_pixels(
        state,
        &binding,
        Region {
            x: 0,
            y: 0,
            width,
            height,
        },
        assume,
        budget,
        cancel,
    )?;
    Ok(ImageEditingBackground {
        binding,
        source_asset_id,
        source_bit_depth: display.source_bit_depth,
        width,
        height,
        rgba: display.rgba,
        icc_profile: display.icc_profile,
    })
}

/// One owned O-grid image for existing export/file owners. The returned pixel
/// budget excludes the admitted canonical workspace and this adapter's reserve,
/// and includes the returned image's complete resident allocation.
pub(crate) struct PreparedOutput {
    pub output: vw_raster::DocumentOutput,
    pub memory_budget_bytes: u64,
    pub binding: WorkflowBinding,
    pub source_asset_id: String,
    pub source_width: u32,
    pub source_height: u32,
}
pub(crate) fn prepare_output(
    state: &ProjectState,
    document: &Id,
    marked: bool,
    assume: bool,
    budget: u64,
    cancel: &Cancellation,
) -> WorkflowResult<PreparedOutput> {
    let working = working(
        state,
        document,
        Mode::Output {
            command: None,
            marked,
        },
        None,
        None,
        assume,
        budget,
        cancel,
    )?;
    let binding = current(state, document.as_str())?;
    let asset = bounds(state.project()?, document)?;
    let [source_width, source_height] = dimensions(asset);
    let source_asset_id = asset.asset_id.clone();
    let assets = Assets { state, cancel };
    let source = assets
        .image(&source_asset_id)
        .map_err(|e| raster_in(e, budget, working.available))?;
    cancel.check()?;
    let output = vw_raster::prepare_document_output(
        &source,
        state.project()?,
        document,
        &assets,
        options(working.available, assume),
        marked,
        &|| cancel.is_cancelled(),
    )
    .map_err(|e| raster_in(e, budget, working.available))?;
    cancel.check()?;
    drop(source);
    Ok(PreparedOutput {
        output,
        memory_budget_bytes: working.available,
        binding,
        source_asset_id,
        source_width,
        source_height,
    })
}
fn export_request(
    state: &ProjectState,
    document: &Id,
    value: &ExportOptions,
    budget: u64,
    hash: bool,
) -> WorkflowResult<ExportRequest> {
    let definition = &state
        .project()?
        .documents
        .get(document)
        .ok_or(WorkflowError::Missing)?
        .definition;
    let region = value
        .region
        .map(|v| {
            if [v.x, v.y, v.width, v.height]
                .iter()
                .any(|v| !v.is_finite() || *v < 0.0 || *v > f64::from(u32::MAX) || v.fract() != 0.0)
            {
                return Err(WorkflowError::Invalid);
            }
            Ok(Region {
                x: v.x as u32,
                y: v.y as u32,
                width: v.width as u32,
                height: v.height as u32,
            })
        })
        .transpose()?;
    Ok(ExportRequest {
        format: match value.format {
            ImageFormat::Png8 => vw_raster::ExportFormat::Png8,
            ImageFormat::Png16 => vw_raster::ExportFormat::Png16,
            ImageFormat::Bmp8 => vw_raster::ExportFormat::Bmp8,
            ImageFormat::Tiff8 => vw_raster::ExportFormat::Tiff8,
            ImageFormat::Tiff16 => vw_raster::ExportFormat::Tiff16,
            ImageFormat::Jpeg { quality } => vw_raster::ExportFormat::Jpeg { quality },
            ImageFormat::WebpLossless => vw_raster::ExportFormat::WebpLossless,
            ImageFormat::WebpLossy { quality } => vw_raster::ExportFormat::WebpLossy { quality },
        },
        region,
        revision: if hash {
            state.view_revision()?
        } else {
            pb::Revision {
                host_seq: 0,
                state_hash: vec![0; 32],
            }
        },
        alpha: value
            .matte_rgb
            .map_or(vw_raster::AlphaPolicy::Preserve, |v| {
                vw_raster::AlphaPolicy::Matte([(v >> 16) as u8, (v >> 8) as u8, v as u8])
            }),
        color: if value.convert_to_srgb {
            vw_raster::ColorPolicy::ConvertToSrgb {
                assume_untagged_srgb: value.assume_untagged_srgb,
            }
        } else {
            vw_raster::ColorPolicy::Preserve
        },
        allow_depth_reduction: value.allow_depth_reduction,
        memory_budget_bytes: budget,
        capture_session: definition
            .capture
            .as_ref()
            .map(|v| Id::from_proto(v.capture_session_id.as_ref()))
            .transpose()?,
        frame_id: definition.capture.as_ref().map(|v| v.frame_id),
    })
}
pub(crate) fn legacy_export(
    state: &ProjectState,
    value: &ExportOptions,
    cancel: &Cancellation,
) -> crate::Result<ExportResult> {
    (|| -> WorkflowResult<ExportResult> {
        let document = id(&value.document_id)?;
        // Tiny request construction does not hash canonical state. Actual
        // revision is captured only after complete workspace admission.
        admit(state, value.memory_budget_bytes, false, FRAME_WORK)?;
        let probe = export_request(state, &document, value, u64::MAX, false)?;
        let working = working(
            state,
            &document,
            Mode::Output {
                command: None,
                marked: value.marked,
            },
            None,
            Some(&probe),
            value.assume_untagged_srgb,
            value.memory_budget_bytes,
            cancel,
        )?;
        let request = export_request(state, &document, value, working.available, true)?;
        let asset = bounds(state.project()?, &document)?;
        let assets = Assets { state, cancel };
        let source = assets
            .image(&asset.asset_id)
            .map_err(|e| raster_in(e, value.memory_budget_bytes, working.available))?;
        cancel.check()?;
        let encoded = if crate::format_images::external(request.format) {
            let output = vw_raster::prepare_document_output(
                &source,
                state.project()?,
                &document,
                &assets,
                options(working.available, value.assume_untagged_srgb),
                value.marked,
                &|| cancel.is_cancelled(),
            )
            .map_err(|e| raster_in(e, value.memory_budget_bytes, working.available))?;
            drop(source);
            // The existing adapter keeps its BMP refusal. Its additive hook
            // embeds the true D extent/O map before writing codec bytes.
            crate::format_images::encode_buffered_with_output(
                &output.image,
                &request,
                output.frame,
                cancel,
            )
            .map_err(|e| raster_in(e, value.memory_budget_bytes, working.available))?
        } else {
            vw_raster::export_document_output(
                &source,
                state.project()?,
                &document,
                &assets,
                options(working.available, value.assume_untagged_srgb),
                value.marked,
                &request,
                &|| cancel.is_cancelled(),
            )
            .map_err(|e| raster_in(e, value.memory_budget_bytes, working.available))?
        };
        cancel.check()?;
        if encoded.bytes.len() as u64 > MAX_ENCODED {
            return Err(WorkflowError::Limit);
        }
        Ok(ExportResult {
            blake3: AssetId::hash(&encoded.bytes).to_string(),
            bytes: encoded.bytes,
            metadata_json: serde_json::to_string(&encoded.metadata)
                .map_err(|_| WorkflowError::Invalid)?,
            revision: state.info()?,
        })
    })()
    .map_err(core_error)
}
pub(crate) fn admit_legacy_export(
    state: &ProjectState,
    value: &ExportOptions,
) -> crate::Result<()> {
    (|| -> WorkflowResult<()> {
        admit(state, value.memory_budget_bytes, false, FRAME_WORK)?;
        let document = id(&value.document_id)?;
        if !vw_raster::requires_document_output(state.project()?, &document, value.marked)
            .map_err(raster)?
        {
            return Ok(());
        }
        let probe = export_request(state, &document, value, u64::MAX, false)?;
        working(
            state,
            &document,
            Mode::Output {
                command: None,
                marked: value.marked,
            },
            None,
            Some(&probe),
            value.assume_untagged_srgb,
            value.memory_budget_bytes,
            &Cancellation::new(),
        )?;
        Ok(())
    })()
    .map_err(core_error)
}
fn core_error(error: WorkflowError) -> CoreError {
    match error {
        WorkflowError::Cancelled => CoreError::Cancelled,
        WorkflowError::Closed => CoreError::Closed,
        WorkflowError::Storage => CoreError::Storage,
        WorkflowError::Memory { .. } | WorkflowError::Limit => CoreError::Backpressure,
        _ => CoreError::Raster,
    }
}
