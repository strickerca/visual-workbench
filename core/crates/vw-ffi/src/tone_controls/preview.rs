use super::*;
use crate::project::ProjectState;
use std::{
    fs::File,
    io::Read,
    time::{SystemTime, UNIX_EPOCH},
};
use vw_model::DeviceId;
use vw_raster::{AssetResolver, DecodedImage, RasterError, Region};

const MIB: u64 = 1024 * 1024;
const MAX_ENCODED: u64 = 64 * MIB;

fn add(a: u64, b: u64) -> WorkflowResult<u64> {
    a.checked_add(b).ok_or(WorkflowError::Limit)
}
fn pixels(asset: &pb::AddAsset) -> WorkflowResult<u64> {
    u64::from(asset.width)
        .checked_mul(u64::from(asset.height))
        .ok_or(WorkflowError::Limit)
}
fn decode_work(asset: &pb::AddAsset) -> WorkflowResult<u64> {
    // Encoded bytes plus full decode/orientation/profile workspace. The 64MiB
    // floor also admits the bounded BMP/TIFF adapter before its codec starts.
    add(
        add(
            pixels(asset)?.checked_mul(40).ok_or(WorkflowError::Limit)?,
            64 * MIB,
        )?,
        asset.byte_size,
    )
}
struct Admission {
    extra: u64,
    render: u64,
    display: u64,
}
fn admission(
    state: &ProjectState,
    document: &Id,
    region: Region,
    budget: u64,
    cancel: &Cancellation,
) -> WorkflowResult<Admission> {
    let project = state.project()?;
    let asset = bounds(project, document)?;
    let [width, height] = dimensions(asset);
    region
        .validate(width, height)
        .map_err(|_| WorkflowError::Invalid)?;
    if pixels(asset)? > 50_000_000 || asset.byte_size > MAX_ENCODED {
        return Err(WorkflowError::Limit);
    }
    let mut geometry = 16 * MIB;
    let mut resolver = 0;
    // Match or exceed the existing renderer's geometry reserve without making
    // a project clone or allocating a path. Also charge nested result decoding;
    // source-resolution render accounting cannot cover a larger result asset.
    for object in project
        .objects
        .values()
        .filter(|v| &v.document_id == document)
    {
        cancel.check()?;
        use pb::object_state::Shape as S;
        let work = match &object.state.shape {
            Some(S::Stroke(v)) => (v.x.len() as u64).checked_mul(16_384),
            Some(S::Text(v)) => (v.text.len() as u64).checked_mul(65_536),
            Some(S::Line(v) | S::Arrow(v) | S::Polygon(v) | S::SelectionVector(v)) => {
                (v.points.len() as u64).checked_mul(1024)
            }
            _ => Some(0),
        }
        .ok_or(WorkflowError::Limit)?;
        geometry = geometry.max(work);
        if let Some(S::ResultId(result)) = &object.state.shape {
            let value = project
                .results
                .get(&Id::from_proto(Some(result))?)
                .ok_or(WorkflowError::Missing)?;
            let asset_id = if value.definition.composite_asset_id.is_empty() {
                &value.definition.output_asset_id
            } else {
                &value.definition.composite_asset_id
            };
            let result_asset = project
                .assets
                .get(&AssetId::try_from(asset_id.clone())?)
                .ok_or(WorkflowError::Missing)?;
            if result_asset.byte_size > MAX_ENCODED || pixels(result_asset)? > 50_000_000 {
                return Err(WorkflowError::Limit);
            }
            resolver = resolver.max(decode_work(result_asset)?);
        }
    }
    geometry = add(
        geometry,
        (project.objects.len() as u64 + project.layers.len() as u64)
            .checked_mul(32)
            .ok_or(WorkflowError::Limit)?,
    )?;
    // A private draft can add its own normal layer and adjustment object after
    // this preallocation check. The canonical renderer charges32bytes per row;
    // reserve both rows now rather than granting a larger budget after cloning.
    geometry = add(geometry, 2 * 32)?;
    // An untagged source has no retained ICC or profile parser workspace.
    // Keep the conservative tagged-source allowance; the existing renderer
    // still validates its actual ICC workspace against its supplied budget.
    let profile = if asset.icc_profile.is_empty() {
        0
    } else {
        64 * MIB
    };
    // Renderer: 64 bytes/source pixel + 16MiB + geometry + profile. Its
    // result-asset checks add decoder/resolver work to that same estimate, so
    // pass the already admitted resolver allowance through to RenderOptions.
    let render = add(
        add(
            add(
                pixels(asset)?.checked_mul(64).ok_or(WorkflowError::Limit)?,
                add(16 * MIB, profile)?,
            )?,
            geometry,
        )?,
        resolver,
    )?;
    let retained_source = add(
        pixels(asset)?.checked_mul(8).ok_or(WorkflowError::Limit)?,
        4 * MIB,
    )?;
    let render_peak = add(render, retained_source)?;
    let tile_pixels = u64::from(region.width) * u64::from(region.height);
    let display = add(
        tile_pixels.checked_mul(32).ok_or(WorkflowError::Limit)?,
        64 * MIB,
    )?;
    // Cropping briefly retains the complete render and an exact tile; no resize
    // or export/redecode path can substitute a smaller source allocation.
    let display_peak = add(display, retained_source)?;
    let peak = decode_work(asset)?.max(render_peak).max(display_peak);
    // Four tile copies cover native/UniFFI/common publication. Canonical host,
    // transaction and hash clones are independently metered by workflow admit.
    let extra = add(
        add(peak, TONE_WORK)?,
        tile_pixels.checked_mul(16).ok_or(WorkflowError::Limit)?,
    )?;
    admit(state, budget, true, extra)?;
    Ok(Admission {
        extra,
        render,
        display,
    })
}
fn raster_error(error: RasterError, budget: u64, workspace: u64) -> WorkflowError {
    match error {
        RasterError::Memory { estimated, .. } => WorkflowError::Memory {
            estimated: estimated.saturating_add(budget.saturating_sub(workspace)),
            budget,
        },
        RasterError::Allocation => WorkflowError::Memory {
            estimated: budget.saturating_add(1),
            budget,
        },
        RasterError::Cancelled => WorkflowError::Cancelled,
        RasterError::MissingAsset | RasterError::OriginalRequired => WorkflowError::Missing,
        RasterError::Io => WorkflowError::Storage,
        _ => WorkflowError::Invalid,
    }
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
        let asset_id =
            AssetId::try_from(value.to_owned()).map_err(|_| RasterError::MissingAsset)?;
        let asset = self
            .state
            .project()
            .map_err(|_| RasterError::MissingAsset)?
            .assets
            .get(&asset_id)
            .ok_or(RasterError::MissingAsset)?;
        let path = self
            .state
            .blobs
            .path(&asset_id)
            .map_err(|_| RasterError::MissingAsset)?;
        let mut file = File::open(path).map_err(|_| RasterError::MissingAsset)?;
        let length = file.metadata().map_err(|_| RasterError::Io)?.len();
        // Admission used canonical byte_size. Refuse a changed file before
        // allocating from its size, then verify its complete content address.
        if length != asset.byte_size || length > MAX_ENCODED {
            return Err(RasterError::MissingAsset);
        }
        let size = usize::try_from(length).map_err(|_| RasterError::Allocation)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| RasterError::Allocation)?;
        bytes.resize(size, 0);
        for chunk in bytes.chunks_mut(65_536) {
            if self.cancel.is_cancelled() {
                return Err(RasterError::Cancelled);
            }
            file.read_exact(chunk).map_err(|_| RasterError::Io)?;
        }
        if file.read(&mut [0u8; 1]).map_err(|_| RasterError::Io)? != 0
            || AssetId::hash(&bytes) != asset_id
        {
            return Err(RasterError::MissingAsset);
        }
        if self.cancel.is_cancelled() {
            return Err(RasterError::Cancelled);
        }
        let decode_budget = decode_work(asset)
            .map_err(|_| RasterError::Allocation)?
            .checked_sub(length)
            .ok_or(RasterError::Allocation)?;
        let image = crate::format_images::decode(
            &bytes,
            vw_raster::DecodeLimits {
                max_encoded_bytes: MAX_ENCODED as usize,
                max_pixels: 50_000_000,
                max_memory_bytes: decode_budget,
            },
            &|| self.cancel.is_cancelled(),
        )?;
        let [width, height] = dimensions(asset);
        if image.width != width
            || image.height != height
            || u32::from(image.pixels.bit_depth()) != asset.bit_depth
            || u32::from(image.orientation_applied) != asset.orientation
            || image.source_asset != asset_id
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

#[allow(clippy::too_many_arguments)]
pub(super) fn preview(
    state: &ProjectState,
    binding: WorkflowBinding,
    command: Option<ToneControlCommand>,
    region: crate::SelectionRegion,
    assume_untagged_srgb: bool,
    budget: u64,
    cancel: &Cancellation,
) -> WorkflowResult<ToneControlPreview> {
    let document = id(&binding.document_id)?;
    let region = Region {
        x: region.x,
        y: region.y,
        width: region.width,
        height: region.height,
    };
    // This precedes visible-state hashing, temporary transaction simulation,
    // source file buffers and all pixel decoding/rendering allocations.
    let admitted = admission(state, &document, region, budget, cancel)?;
    check_binding(state, &binding)?;
    let project = state.project()?;
    snapshot::tones(project, &document, cancel)?;
    let asset = bounds(project, &document)?;
    if asset.icc_profile.is_empty() && !assume_untagged_srgb {
        return Err(WorkflowError::Invalid);
    }
    let source_asset_id = asset.asset_id.clone();
    let draft = if let Some(command) = command {
        let device = state.store()?.local_device()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| WorkflowError::Invalid)?
            .as_millis();
        let created_at_ms = i64::try_from(now).map_err(|_| WorkflowError::Limit)?;
        let metadata = WorkflowMetadata {
            // This fixed valid UUID only exists inside the new private host.
            // Its history is empty, so it never consumes a durable identity.
            transaction_id: "018bcfe5-6800-7fff-bfff-ffffffffffff".into(),
            device_id: device.to_string(),
            first_lamport: state.history.next_lamport,
            created_at_ms,
        };
        metadata.validate()?;
        let (mut transaction, _) = plan::transaction(state, &document, &metadata, command)?;
        admit_transaction(state, &transaction, budget, admitted.extra)?;
        cancel.check()?;
        let mut host = vw_ops::HostSequencer::new(project.clone(), device.clone())
            .map_err(plan::operation_error)?;
        // The visible project may include offline pending transactions. A new
        // host starts at sequence zero; never feed it the accepted store head.
        transaction.base_revision = Some(host.revision().map_err(plan::operation_error)?);
        host.submit(
            transaction,
            &DeviceId::try_from(metadata.device_id)?,
            created_at_ms,
        )
        .map_err(plan::operation_error)?;
        Some(host)
    } else {
        None
    };
    cancel.check()?;
    let assets = Assets { state, cancel };
    let source = assets
        .image(&source_asset_id)
        .map_err(|e| raster_error(e, budget, admitted.extra))?;
    cancel.check()?;
    let rendered = vw_raster::render_document(
        &source,
        draft
            .as_ref()
            .map_or(project, vw_ops::HostSequencer::project),
        &document,
        &assets,
        vw_raster::RenderOptions {
            memory_budget_bytes: admitted.render,
            include_guides: false,
            assume_untagged_srgb,
        },
    )
    .map_err(|e| raster_error(e, budget, admitted.render))?;
    // The existing whole-buffer renderer is not interruptible. Keep native
    // settlement ownership until it returns, then discard cancelled results.
    cancel.check()?;
    drop(source);
    drop(draft);
    let tile = rendered
        .crop(region)
        .map_err(|e| raster_error(e, budget, admitted.display))?;
    drop(rendered);
    cancel.check()?;
    let display = vw_raster::display_rgba8(tile, admitted.display, assume_untagged_srgb, &|| {
        cancel.is_cancelled()
    })
    .map_err(|e| raster_error(e, budget, admitted.display))?;
    cancel.check()?;
    Ok(ToneControlPreview {
        binding,
        source_asset_id,
        x: region.x,
        y: region.y,
        width: region.width,
        height: region.height,
        rgba: display.rgba,
    })
}
