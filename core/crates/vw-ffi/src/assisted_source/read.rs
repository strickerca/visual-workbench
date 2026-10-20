use super::*;
use crate::{GiantImageError, WorkflowBinding, WorkflowError, project::ProjectState};
use std::sync::atomic::{AtomicBool, Ordering};
use vw_model::{AssetId, Id, StateHash};
use vw_proto::v1 as pb;
use vw_raster::RasterError;
use vw_stream::{MemoryCharge, MemoryKind, SessionBudget};

const MIB: u64 = 1024 * 1024;
const MAX_MEMORY: u64 = 256 * MIB;
const MAX_ENCODED: u64 = 64 * MIB;
const ENVELOPE: u64 = 16 * 1024;

pub(super) fn live(closed: &AtomicBool, cancel: &Cancellation) -> SelectionResult<()> {
    if closed.load(Ordering::Acquire) {
        return Err(SelectionError::Closed);
    }
    cancel.check()?;
    Ok(())
}
pub(super) fn validate(
    binding: &SelectionBinding,
    width: u32,
    height: u32,
    depth: u32,
    page: Option<u32>,
    budget: u64,
) -> SelectionResult<()> {
    if binding.project_id.len() != 36
        || binding.document_id.len() != 36
        || binding.source_asset_id.len() != 64
        || binding.state_hash.len() != 64
        || width == 0
        || height == 0
        || budget == 0
        || budget > MAX_MEMORY
    {
        return Err(SelectionError::Invalid);
    }
    Id::try_from(binding.project_id.clone())?;
    Id::try_from(binding.document_id.clone())?;
    AssetId::try_from(binding.source_asset_id.clone())?;
    StateHash::try_from(binding.state_hash.clone())?;
    if !matches!(depth, 8 | 16) || page.is_some() {
        return Err(SelectionError::Unsupported);
    }
    if u64::from(width) * u64::from(height) > 50_000_000 {
        return Err(SelectionError::Limit);
    }
    Ok(())
}
fn workflow(error: WorkflowError) -> SelectionError {
    match error {
        WorkflowError::Memory { estimated, budget } => SelectionError::Memory { estimated, budget },
        WorkflowError::Stale | WorkflowError::Missing => SelectionError::Conflict,
        WorkflowError::Closed => SelectionError::Closed,
        WorkflowError::Cancelled => SelectionError::Cancelled,
        WorkflowError::Backpressure => SelectionError::Backpressure,
        WorkflowError::Storage => SelectionError::Storage,
        WorkflowError::Limit => SelectionError::Limit,
        _ => SelectionError::Invalid,
    }
}
pub(super) fn giant(error: GiantImageError) -> SelectionError {
    match error {
        GiantImageError::Memory { estimated, budget } => {
            SelectionError::Memory { estimated, budget }
        }
        GiantImageError::Stale | GiantImageError::Missing => SelectionError::Conflict,
        GiantImageError::Closed => SelectionError::Closed,
        GiantImageError::Cancelled => SelectionError::Cancelled,
        GiantImageError::Backpressure => SelectionError::Backpressure,
        GiantImageError::Storage => SelectionError::Storage,
        GiantImageError::Limit => SelectionError::Limit,
        GiantImageError::Unsupported => SelectionError::Unsupported,
        GiantImageError::Invalid => SelectionError::Invalid,
    }
}
fn add(a: u64, b: u64) -> SelectionResult<u64> {
    a.checked_add(b).ok_or(SelectionError::Limit)
}
fn memory(estimated: u64, budget: u64) -> SelectionResult<()> {
    if estimated > budget {
        Err(SelectionError::Memory { estimated, budget })
    } else {
        Ok(())
    }
}
fn reserve(pool: &SessionBudget, kind: MemoryKind, bytes: u64) -> SelectionResult<MemoryCharge> {
    pool.reserve(kind, bytes)
        .map_err(|_| SelectionError::Memory {
            estimated: pool.used_bytes().saturating_add(bytes),
            budget: pool.limit_bytes(),
        })
}
fn asset<'a>(
    state: &'a ProjectState,
    binding: &SelectionBinding,
    width: u32,
    height: u32,
    depth: u32,
) -> SelectionResult<&'a pb::AddAsset> {
    let project = state.project()?;
    let document = &project
        .documents
        .get(&Id::try_from(binding.document_id.clone())?)
        .ok_or(SelectionError::Conflict)?
        .definition;
    if !matches!(
        pb::DocumentKind::try_from(document.kind),
        Ok(pb::DocumentKind::Image | pb::DocumentKind::Capture)
    ) {
        return Err(SelectionError::Unsupported);
    }
    if project.id.as_str() != binding.project_id
        || document.primary_asset_id != binding.source_asset_id
    {
        return Err(SelectionError::Conflict);
    }
    let value = project
        .assets
        .get(&AssetId::try_from(document.primary_asset_id.clone())?)
        .ok_or(SelectionError::Corrupt)?;
    let dimensions = if value.orientation >= 5 {
        [value.height, value.width]
    } else {
        [value.width, value.height]
    };
    if dimensions != [width, height] || value.bit_depth != depth {
        return Err(SelectionError::Conflict);
    }
    if !(1..=8).contains(&value.orientation) || !matches!(value.bit_depth, 8 | 16) {
        return Err(SelectionError::Unsupported);
    }
    if value.byte_size == 0 || value.byte_size > MAX_ENCODED {
        return Err(SelectionError::EncodedLimit);
    }
    Ok(value)
}
fn check_binding(state: &ProjectState, value: &SelectionBinding) -> SelectionResult<()> {
    crate::workflow::check_binding(
        state,
        &WorkflowBinding {
            project_id: value.project_id.clone(),
            document_id: value.document_id.clone(),
            host_seq: value.host_seq,
            state_hash: value.state_hash.clone(),
        },
    )
    .map_err(workflow)
}
fn raster(error: RasterError, budget: u64, admitted: u64) -> SelectionError {
    match error {
        RasterError::Memory { estimated, .. } => SelectionError::Memory {
            estimated: estimated.saturating_add(budget.saturating_sub(admitted)),
            budget,
        },
        RasterError::Cancelled => SelectionError::Cancelled,
        RasterError::Unsupported(_) | RasterError::CodecUnavailable | RasterError::Depth => {
            SelectionError::Unsupported
        }
        RasterError::Io => SelectionError::Storage,
        RasterError::Allocation => SelectionError::Limit,
        _ => SelectionError::Corrupt,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn read(
    state: &ProjectState,
    binding: SelectionBinding,
    width: u32,
    height: u32,
    depth: u32,
    assume: bool,
    budget: u64,
    closed: &AtomicBool,
    cancel: &Cancellation,
) -> SelectionResult<Arc<AssistedSourcePixels>> {
    live(closed, cancel)?;
    let source = asset(state, &binding, width, height, depth)?;
    let pixels = u64::from(width) * u64::from(height);
    let rgba_bytes = pixels.checked_mul(4).ok_or(SelectionError::Limit)?;
    // This borrowed meter does not hash or clone canonical content. The full
    // canonical/codec plus native/ABI/foreign reservations are acquired below
    // before visible-state hashing or any source/file/pixel allocation.
    let canonical = crate::workflow::canonical_workspace(state, budget, false).map_err(workflow)?;
    let decode = crate::format_images::decode_workspace_bytes(source).map_err(workflow)?;
    let profile = if source.icc_profile.is_empty() {
        0
    } else {
        72 * MIB
    };
    let display = add(
        add(
            pixels.checked_mul(24).ok_or(SelectionError::Limit)?,
            32 * MIB,
        )?,
        profile,
    )?;
    let workspace_bytes = add(add(canonical, decode.max(display))?, ENVELOPE)?;
    let abi_bytes = add(rgba_bytes, ENVELOPE)?;
    let foreign_bytes = add(rgba_bytes, ENVELOPE)?;
    let peak = add(
        add(add(workspace_bytes, rgba_bytes)?, abi_bytes)?,
        foreign_bytes,
    )?;
    memory(peak, budget)?;
    let pool = crate::session_resources::shared_budget().map_err(giant)?;
    let workspace = reserve(&pool, MemoryKind::CodecWorkspace, workspace_bytes)?;
    let native = reserve(&pool, MemoryKind::Transfer, rgba_bytes)?;
    let abi = reserve(&pool, MemoryKind::CodecWorkspace, abi_bytes)?;
    let foreign = reserve(&pool, MemoryKind::Foreign, foreign_bytes)?;
    let charges = vec![native, abi, foreign];
    live(closed, cancel)?;
    check_binding(state, &binding)?;
    if source.icc_profile.is_empty() && !assume {
        return Err(SelectionError::Unsupported);
    }
    let source_id = AssetId::try_from(binding.source_asset_id.clone())?;
    // Trusted bounded NOFOLLOW reader checks expected byte size and hashes the
    // complete original. No caller pixels, thumbnail or output image is used.
    let encoded =
        crate::masks::ai_bridge::blob(state, &source_id, source.byte_size, MAX_ENCODED, cancel)?;
    live(closed, cancel)?;
    let decode_budget = decode
        .checked_sub(encoded.capacity() as u64)
        .ok_or(SelectionError::Limit)?;
    let decoded = crate::format_images::decode(
        &encoded,
        vw_raster::DecodeLimits {
            max_encoded_bytes: MAX_ENCODED as usize,
            max_pixels: 50_000_000,
            max_memory_bytes: decode_budget,
        },
        &|| cancel.is_cancelled() || closed.load(Ordering::Acquire),
    )
    .map_err(|e| raster(e, budget, decode_budget))?;
    drop(encoded);
    live(closed, cancel)?;
    if decoded.source_asset != source_id
        || !decoded.original_available
        || decoded.width != width
        || decoded.height != height
        || u32::from(decoded.pixels.bit_depth()) != depth
        || u32::from(decoded.orientation_applied) != source.orientation
        || decoded.icc.as_deref().unwrap_or_default() != source.icc_profile.as_slice()
    {
        return Err(SelectionError::Corrupt);
    }
    // Consuming display conversion is the existing explicit 16-to-8/ICC path;
    // it performs no tone, annotation, frame, crop or resampling composition.
    let display = vw_raster::display_rgba8(decoded, display, assume, &|| {
        cancel.is_cancelled() || closed.load(Ordering::Acquire)
    })
    .map_err(|e| raster(e, budget, display))?;
    live(closed, cancel)?;
    if display.width != width
        || display.height != height
        || display.source_asset != source_id
        || u32::from(display.source_bit_depth) != depth
        || display.rgba.len() as u64 != rgba_bytes
        || display.rgba.capacity() as u64 > rgba_bytes
    {
        return Err(SelectionError::Corrupt);
    }
    // Recheck full visible revision and current original metadata after decode
    // and conversion, while still on the owning serialized project worker.
    check_binding(state, &binding)?;
    asset(state, &binding, width, height, depth)?;
    live(closed, cancel)?;
    let info = AssistedSourcePixelsInfo {
        binding,
        width,
        height,
        source_bit_depth: depth,
        page: None,
        rgba_bytes,
    };
    let result = AssistedSourcePixels::new(
        info,
        OwnedPixels {
            rgba: display.rgba,
            native_lease: crate::SessionResourceCharge::from_charges(charges),
        },
    );
    drop(display.icc_profile);
    drop(workspace);
    Ok(result)
}
