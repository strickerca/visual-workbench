//! Document-bound selection operations. All model reads, fences and commits run
//! on the existing project worker; mask bytes are immutable content-addresses.
pub(crate) mod ai_bridge;
mod dto;
mod edit;
mod export;
pub use dto::*;

use crate::{Cancellation, CoreError, ProjectSession, project::ProjectState};
use std::{
    io::Cursor,
    sync::{Arc, atomic::Ordering},
};
use vw_mask::{Mask, Region, Size};
use vw_model::{AssetId, Id, Project};
use vw_proto::v1 as pb;

const MAX_MEMORY: u64 = 256 * 1024 * 1024;
const MAX_ENCODED: u64 = 64 * 1024 * 1024;
const CODEC_RESERVE: u64 = 16 * 1024 * 1024;
const MASK_METADATA: &str = "{\"software\":\"Visual Workbench mask algorithm 1\"}";

#[derive(Clone, Debug, thiserror::Error, uniffi::Error)]
pub enum SelectionError {
    #[error("invalid selection request")]
    Invalid,
    #[error("document or selection changed; refresh before making another edit")]
    Conflict,
    #[error("transaction identifier has already been used")]
    ReusedTransaction,
    #[error("selection or layer is locked")]
    Locked,
    #[error("selection representation or source format is unsupported")]
    Unsupported,
    #[error("selection data does not match its immutable asset")]
    Corrupt,
    #[error("selection memory estimate exceeds the supplied budget")]
    Memory { estimated: u64, budget: u64 },
    #[error("selection work or payload limit exceeded")]
    Limit,
    #[error("selection export exceeds its encoded byte limit")]
    EncodedLimit,
    #[error("selection storage failure")]
    Storage,
    #[error("selection cancelled")]
    Cancelled,
    #[error("project closed")]
    Closed,
    #[error("selection worker capacity exhausted")]
    Backpressure,
}
pub type SelectionResult<T> = std::result::Result<T, SelectionError>;
impl From<CoreError> for SelectionError {
    fn from(value: CoreError) -> Self {
        match value {
            CoreError::Cancelled => Self::Cancelled,
            CoreError::Closed => Self::Closed,
            CoreError::Backpressure | CoreError::Worker => Self::Backpressure,
            CoreError::Storage => Self::Storage,
            CoreError::Unsupported => Self::Unsupported,
            _ => Self::Invalid,
        }
    }
}
impl From<vw_model::ModelError> for SelectionError {
    fn from(_: vw_model::ModelError) -> Self {
        Self::Invalid
    }
}
impl From<vw_store::StoreError> for SelectionError {
    fn from(_: vw_store::StoreError) -> Self {
        Self::Storage
    }
}
impl From<std::io::Error> for SelectionError {
    fn from(_: std::io::Error) -> Self {
        Self::Storage
    }
}
impl From<vw_mask::MaskError> for SelectionError {
    fn from(value: vw_mask::MaskError) -> Self {
        match value {
            vw_mask::MaskError::Corrupt => Self::Corrupt,
            vw_mask::MaskError::Limit
            | vw_mask::MaskError::Allocation
            | vw_mask::MaskError::VersionExhausted => Self::Limit,
            vw_mask::MaskError::Encoding => Self::Storage,
            _ => Self::Invalid,
        }
    }
}
impl From<vw_raster::RasterError> for SelectionError {
    fn from(value: vw_raster::RasterError) -> Self {
        match value {
            vw_raster::RasterError::Memory { estimated, budget } => {
                Self::Memory { estimated, budget }
            }
            vw_raster::RasterError::Allocation => Self::Limit,
            vw_raster::RasterError::Cancelled => Self::Cancelled,
            vw_raster::RasterError::Io => Self::Storage,
            vw_raster::RasterError::Unsupported(_) => Self::Unsupported,
            _ => Self::Corrupt,
        }
    }
}

#[uniffi::export]
impl ProjectSession {
    /// Capture this binding before a real gesture. Never replace it with a later
    /// revision at pointer-up: that would authorize an edit on different pixels.
    pub async fn selection_document(
        &self,
        document_id: String,
        cancellation: Arc<Cancellation>,
    ) -> SelectionResult<SelectionDocument> {
        self.check_open()?;
        short_id(&document_id)?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    cancellation.check()?;
                    if closed.load(Ordering::Acquire) {
                        return Err(SelectionError::Closed);
                    }
                    canonical_workspace(state, MAX_MEMORY, false)?;
                    document(state, &document_id)
                })())
            })
            .await?
    }

    pub async fn selection_snapshot(
        &self,
        binding: SelectionBinding,
        object_id: String,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> SelectionResult<SelectionSnapshot> {
        self.check_open()?;
        budget(memory_budget_bytes)?;
        admit_binding(&binding)?;
        short_id(&object_id)?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    cancellation.check()?;
                    if closed.load(Ordering::Acquire) {
                        return Err(SelectionError::Closed);
                    }
                    let canonical = canonical_workspace(state, memory_budget_bytes, false)?;
                    let doc = check_binding(state, &binding)?;
                    let size = Size::new(doc.width, doc.height)?;
                    memory(mask_estimate(size, canonical)?, memory_budget_bytes)?;
                    let (version, layer, editable, visible) =
                        describe(state.project()?, &binding, &object_id, size)?;
                    let mask = load_mask(state, &version, size, &cancellation)?;
                    cancellation.check()?;
                    Ok(snapshot(binding, version, layer, editable, visible, &mask))
                })())
            })
            .await?
    }

    /// One decode for up to 64 visible tiles. The returned binding must still
    /// match the app's displayed revision before the overlay is installed.
    pub async fn selection_tiles(
        &self,
        binding: SelectionBinding,
        selection: SelectionVersion,
        regions: Vec<SelectionRegion>,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> SelectionResult<SelectionTiles> {
        self.check_open()?;
        budget(memory_budget_bytes)?;
        admit_binding(&binding)?;
        admit_version(&selection)?;
        if regions.is_empty() || regions.len() > 64 {
            return Err(SelectionError::Limit);
        }
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    cancellation.check()?;
                    if closed.load(Ordering::Acquire) {
                        return Err(SelectionError::Closed);
                    }
                    let canonical = canonical_workspace(state, memory_budget_bytes, false)?;
                    let doc = check_binding(state, &binding)?;
                    let size = Size::new(doc.width, doc.height)?;
                    check_version(state.project()?, &binding, &selection, size)?;
                    let output = regions.iter().try_fold(0u64, |sum, value| {
                        let region = region(*value);
                        region.size(size)?;
                        if region.width > 256 || region.height > 256 {
                            return Err(SelectionError::Limit);
                        }
                        sum.checked_add(u64::from(region.width) * u64::from(region.height) * 4)
                            .ok_or(SelectionError::Limit)
                    })?;
                    memory(
                        mask_estimate(
                            size,
                            output.checked_add(canonical).ok_or(SelectionError::Limit)?,
                        )?,
                        memory_budget_bytes,
                    )?;
                    let mask = load_mask(state, &selection, size, &cancellation)?;
                    let mut tiles = Vec::with_capacity(regions.len());
                    for value in regions {
                        cancellation.check()?;
                        tiles.push(SelectionTile {
                            region: value,
                            coverage: mask.crop(region(value))?,
                        });
                    }
                    Ok(SelectionTiles {
                        binding,
                        selection,
                        tiles,
                    })
                })())
            })
            .await?
    }

    /// At-most-once admission: reused IDs and stale bindings are refused. The
    /// caller refreshes after cancellation/uncertain delivery; it must not send
    /// the same geometry with a new binding as an automatic retry.
    pub async fn apply_selection(
        &self,
        options: SelectionEdit,
        cancellation: Arc<Cancellation>,
    ) -> SelectionResult<SelectionReceipt> {
        self.check_open()?;
        edit::admit_options(&options)?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    cancellation.check()?;
                    if closed.load(Ordering::Acquire) {
                        return Err(SelectionError::Closed);
                    }
                    edit::apply(state, options, &cancellation)
                })())
            })
            .await?
    }

    pub async fn export_selection_file(
        &self,
        options: SelectionExportOptions,
        cancellation: Arc<Cancellation>,
    ) -> SelectionResult<SelectionExportReceipt> {
        self.check_open()?;
        budget(options.memory_budget_bytes)?;
        admit_binding(&options.binding)?;
        admit_version(&options.selection)?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    cancellation.check()?;
                    if closed.load(Ordering::Acquire) {
                        return Err(SelectionError::Closed);
                    }
                    export::write(state, options, &cancellation)
                })())
            })
            .await?
    }
}

fn budget(value: u64) -> SelectionResult<()> {
    if value == 0 || value > MAX_MEMORY {
        Err(SelectionError::Invalid)
    } else {
        Ok(())
    }
}
fn short_id(value: &str) -> SelectionResult<()> {
    if value.len() > 128 {
        return Err(SelectionError::Invalid);
    }
    Id::try_from(value.to_owned())?;
    Ok(())
}
fn admit_binding(value: &SelectionBinding) -> SelectionResult<()> {
    short_id(&value.project_id)?;
    short_id(&value.document_id)?;
    if value.source_asset_id.len() != 64
        || value.state_hash.len() != 64
        || !value
            .state_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(SelectionError::Invalid);
    }
    AssetId::try_from(value.source_asset_id.clone())?;
    Ok(())
}
fn admit_version(value: &SelectionVersion) -> SelectionResult<()> {
    short_id(&value.object_id)?;
    if value.asset_id.len() != 64 {
        return Err(SelectionError::Invalid);
    }
    AssetId::try_from(value.asset_id.clone())?;
    Ok(())
}
fn memory(estimated: u64, budget: u64) -> SelectionResult<()> {
    if estimated > budget {
        Err(SelectionError::Memory { estimated, budget })
    } else {
        Ok(())
    }
}
fn canonical_workspace(state: &ProjectState, budget: u64, mutation: bool) -> SelectionResult<u64> {
    crate::workflow::canonical_workspace(state, budget, mutation).map_err(|error| match error {
        crate::WorkflowError::Memory { estimated, budget } => {
            SelectionError::Memory { estimated, budget }
        }
        crate::WorkflowError::Storage => SelectionError::Storage,
        crate::WorkflowError::Closed => SelectionError::Closed,
        crate::WorkflowError::Backpressure => SelectionError::Backpressure,
        crate::WorkflowError::Limit => SelectionError::Limit,
        _ => SelectionError::Invalid,
    })
}
fn mask_estimate(size: Size, extra: u64) -> SelectionResult<u64> {
    // Two retained PNG inputs, decoder rows/dense buffers, sparse tile copies,
    // result/encoder capacity, marshaling, morphology scratch and codec reserve.
    (size.pixels() as u64)
        .checked_mul(16)
        .and_then(|n| n.checked_add(CODEC_RESERVE))
        .and_then(|n| n.checked_add(extra))
        .ok_or(SelectionError::Limit)
}
fn png_limit(size: Size) -> SelectionResult<u64> {
    (size.pixels() as u64)
        .checked_mul(2)
        .and_then(|n| n.checked_add(1024 * 1024))
        .map(|n| n.min(MAX_ENCODED))
        .ok_or(SelectionError::Limit)
}
fn region(value: SelectionRegion) -> Region {
    Region {
        x: value.x,
        y: value.y,
        width: value.width,
        height: value.height,
    }
}
fn public_region(value: Region) -> SelectionRegion {
    SelectionRegion {
        x: value.x,
        y: value.y,
        width: value.width,
        height: value.height,
    }
}
fn binding(info: &crate::ProjectInfo, document: &str, asset: &str) -> SelectionBinding {
    SelectionBinding {
        project_id: info.project_id.clone(),
        document_id: document.into(),
        source_asset_id: asset.into(),
        host_seq: info.host_seq,
        state_hash: info.state_hash.clone(),
    }
}
fn document(state: &ProjectState, id: &str) -> SelectionResult<SelectionDocument> {
    let project = state.project()?;
    let id = Id::try_from(id.to_owned())?;
    let doc = &project
        .documents
        .get(&id)
        .ok_or(SelectionError::Invalid)?
        .definition;
    if !matches!(
        pb::DocumentKind::try_from(doc.kind),
        Ok(pb::DocumentKind::Image | pb::DocumentKind::Capture)
    ) {
        return Err(SelectionError::Unsupported);
    }
    let asset = project
        .assets
        .get(&AssetId::try_from(doc.primary_asset_id.clone())?)
        .ok_or(SelectionError::Corrupt)?;
    let (width, height) = if asset.orientation >= 5 {
        (asset.height, asset.width)
    } else {
        (asset.width, asset.height)
    };
    Size::new(width, height)?;
    let revision = state.info()?;
    Ok(SelectionDocument {
        binding: binding(&revision, id.as_str(), &doc.primary_asset_id),
        width,
        height,
        source_bit_depth: asset.bit_depth,
        revision,
    })
}
fn check_binding(
    state: &ProjectState,
    expected: &SelectionBinding,
) -> SelectionResult<SelectionDocument> {
    let actual = document(state, &expected.document_id)?;
    if &actual.binding != expected {
        return Err(SelectionError::Conflict);
    }
    Ok(actual)
}
fn describe(
    project: &Project,
    binding: &SelectionBinding,
    object_id: &str,
    size: Size,
) -> SelectionResult<(SelectionVersion, String, bool, bool)> {
    let id = Id::try_from(object_id.to_owned())?;
    let value = project.objects.get(&id).ok_or(SelectionError::Conflict)?;
    if value.document_id.as_str() != binding.document_id {
        return Err(SelectionError::Conflict);
    }
    let layer_id = Id::from_proto(value.state.layer_id.as_ref())?;
    let layer = project
        .layers
        .get(&layer_id)
        .ok_or(SelectionError::Corrupt)?;
    let Some(pb::object_state::Shape::SelectionRaster(mask)) = &value.state.shape else {
        return Err(SelectionError::Unsupported);
    };
    let Some(bounds) = &mask.bounds else {
        return Err(SelectionError::Corrupt);
    };
    if value.state.transform != Some(identity())
        || mask.feather != 0.0
        || bounds.x != 0.0
        || bounds.y != 0.0
        || bounds.w != f64::from(size.width())
        || bounds.h != f64::from(size.height())
    {
        return Err(SelectionError::Unsupported);
    }
    if layer.definition.kind != "mask" {
        return Err(SelectionError::Unsupported);
    }
    let version = u64::try_from(project.mask_versions.get(&id).map_or(0, Vec::len))
        .map_err(|_| SelectionError::Limit)?;
    Ok((
        SelectionVersion {
            object_id: id.to_string(),
            asset_id: mask.mask_asset_id.clone(),
            version,
        },
        layer_id.to_string(),
        !value.state.locked && !layer.locked,
        !value.state.hidden && layer.visible,
    ))
}
fn check_version(
    project: &Project,
    binding: &SelectionBinding,
    expected: &SelectionVersion,
    size: Size,
) -> SelectionResult<(String, bool, bool)> {
    let (actual, layer, editable, visible) = describe(project, binding, &expected.object_id, size)?;
    if &actual != expected {
        return Err(SelectionError::Conflict);
    }
    Ok((layer, editable, visible))
}
fn identity() -> pb::Affine {
    pb::Affine {
        a: 1.0,
        d: 1.0,
        ..Default::default()
    }
}
fn mask_asset(mask: &Mask, id: &AssetId, length: u64) -> pb::AddAsset {
    pb::AddAsset {
        asset_id: id.to_string(),
        format: "png".into(),
        width: mask.size().width(),
        height: mask.size().height(),
        orientation: 1,
        bit_depth: 8,
        has_alpha: false,
        color_space: "coverage".into(),
        icc_profile: vec![],
        byte_size: length,
        source: "import".into(),
        captured_at_ms: 0,
        metadata_json: MASK_METADATA.into(),
    }
}
fn snapshot(
    binding: SelectionBinding,
    selection: SelectionVersion,
    layer_id: String,
    editable: bool,
    visible: bool,
    mask: &Mask,
) -> SelectionSnapshot {
    SelectionSnapshot {
        binding,
        selection,
        layer_id,
        width: mask.size().width(),
        height: mask.size().height(),
        nonzero_bounds: mask.bounds().map(public_region),
        editable,
        visible,
    }
}
fn load_mask(
    state: &ProjectState,
    version: &SelectionVersion,
    size: Size,
    cancel: &Cancellation,
) -> SelectionResult<Mask> {
    let id = AssetId::try_from(version.asset_id.clone())?;
    let asset = state
        .project()?
        .assets
        .get(&id)
        .ok_or(SelectionError::Corrupt)?;
    if asset.width != size.width()
        || asset.height != size.height()
        || asset.format != "png"
        || asset.orientation != 1
        || asset.bit_depth != 8
        || asset.has_alpha
        || !asset.icc_profile.is_empty()
    {
        return Err(SelectionError::Unsupported);
    }
    let bytes = export::read_blob(state, &id, asset.byte_size, png_limit(size)?, cancel)?;
    canonical_png(&bytes, size)?;
    let decoder_limit = size
        .pixels()
        .checked_add(8 * 1024 * 1024)
        .ok_or(SelectionError::Limit)?;
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(&bytes),
        png::Limits {
            bytes: decoder_limit,
        },
    );
    decoder.set_ignore_text_chunk(true);
    let mut reader = decoder.read_info().map_err(|_| SelectionError::Corrupt)?;
    if reader.output_buffer_size() != Some(size.pixels()) {
        return Err(SelectionError::Corrupt);
    }
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(size.pixels())
        .map_err(|_| SelectionError::Limit)?;
    pixels.resize(size.pixels(), 0);
    let info = reader
        .next_frame(&mut pixels)
        .map_err(|_| SelectionError::Corrupt)?;
    if info.width != size.width()
        || info.height != size.height()
        || info.color_type != png::ColorType::Grayscale
        || info.bit_depth != png::BitDepth::Eight
    {
        return Err(SelectionError::Corrupt);
    }
    reader.finish().map_err(|_| SelectionError::Corrupt)?;
    cancel.check()?;
    Ok(Mask::from_dense(size, &pixels)?)
}

/// Persisted coverage is the narrow PNG format our writer emits. Reject ICC,
/// gamma, EXIF, palette, animation and unknown ancillary chunks before a decoder
/// can allocate or reinterpret them. CRC/DEFLATE checks remain with png.
fn canonical_png(bytes: &[u8], size: Size) -> SelectionResult<()> {
    if bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n") {
        return Err(SelectionError::Corrupt);
    }
    let mut offset = 8usize;
    let mut header = false;
    let mut data = false;
    while let Some(head) = bytes.get(offset..offset.saturating_add(8)) {
        let length =
            u32::from_be_bytes(head[..4].try_into().map_err(|_| SelectionError::Corrupt)?) as usize;
        let end = offset
            .checked_add(12)
            .and_then(|n| n.checked_add(length))
            .ok_or(SelectionError::Corrupt)?;
        if end > bytes.len() {
            return Err(SelectionError::Corrupt);
        }
        let chunk = &bytes[offset + 8..end - 4];
        match &head[4..8] {
            b"IHDR" if !header && offset == 8 && length == 13 => {
                if chunk[..4] != size.width().to_be_bytes()
                    || chunk[4..8] != size.height().to_be_bytes()
                    || chunk[8..] != [8, 0, 0, 0, 0]
                {
                    return Err(SelectionError::Corrupt);
                }
                header = true;
            }
            b"IDAT" if header => data = true,
            b"IEND" if data && length == 0 && end == bytes.len() => return Ok(()),
            _ => return Err(SelectionError::Corrupt),
        }
        offset = end;
    }
    Err(SelectionError::Corrupt)
}
