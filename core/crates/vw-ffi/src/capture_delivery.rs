//! Lossless capture append to an existing authoritative project. The normal
//! OPS/BLOB protocol owns delivery; this capability never opens another link.
use crate::{Cancellation, ProjectSession, workflow::*};
use std::{
    io::{Read, Seek, SeekFrom, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use vw_model::{AssetId, DeviceId, Id};
use vw_proto::v1 as pb;
use vw_raster::RasterSource;

const ORIGINAL_LIMIT: u64 = 64 * 1024 * 1024; // Existing authenticated BLOB limit.
const SOURCE_WORK: u64 = 160 * 1024 * 1024;
const PLAN_WORK: u64 = 64 * 1024 * 1024;
static PLANS: AtomicUsize = AtomicUsize::new(0);
struct Permit;
impl Permit {
    fn new() -> WorkflowResult<Self> {
        PLANS
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 2).then_some(n + 1)
            })
            .map_err(|_| WorkflowError::Backpressure)?;
        Ok(Self)
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        PLANS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct CaptureDeliveryInfo {
    pub transaction_id: String,
    pub source: WorkflowBinding,
    pub target: WorkflowBinding,
    pub document_id: String,
    pub source_asset_id: String,
    pub capture_session_id: String,
    pub frame_id: u64,
    pub geometry_revision: u32,
    pub encoded_bytes: u64,
    pub operation_count: u32,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ReceivedCapture {
    pub binding: WorkflowBinding,
    pub created_host_seq: u64,
    pub source_asset_id: String,
    pub capture_session_id: String,
    pub frame_id: u64,
    pub geometry_revision: u32,
    pub original_verified: bool,
}
struct Captured {
    original: tempfile::NamedTempFile,
    asset: pb::AddAsset,
    document: pb::CreateDocument,
    layer: pb::CreateLayer,
    snapshots: Vec<pb::AddSemanticSnapshot>,
    _permit: Permit,
}
struct Prepared {
    captured: Captured,
    transaction: pb::Transaction,
    info: CaptureDeliveryInfo,
    budget: u64,
}
#[derive(uniffi::Object)]
pub struct CaptureDeliveryPlan {
    worker: crate::worker::Worker<crate::project::ProjectState>,
    closed: Arc<AtomicBool>,
    disposed: Arc<AtomicBool>,
    prepared: Mutex<Option<Arc<Prepared>>>,
}
fn storage(_: std::io::Error) -> WorkflowError {
    WorkflowError::Storage
}
fn identity(value: &pb::CreateDocument) -> WorkflowResult<(String, u64, u32)> {
    let capture = value.capture.as_ref().ok_or(WorkflowError::Invalid)?;
    vw_model::validate_capture(capture)?;
    if value.kind != pb::DocumentKind::Capture as i32
        || capture.platform != "windows"
        || !capture.lossless
        || capture.degraded
    {
        return Err(WorkflowError::Invalid);
    }
    Ok((
        Id::from_proto(capture.capture_session_id.as_ref())?.to_string(),
        capture.frame_id,
        capture
            .geometry
            .as_ref()
            .ok_or(WorkflowError::Invalid)?
            .geometry_revision,
    ))
}
fn copy_original(
    state: &crate::project::ProjectState,
    asset: &pb::AddAsset,
    cancel: &Cancellation,
) -> WorkflowResult<tempfile::NamedTempFile> {
    let id = AssetId::try_from(asset.asset_id.clone())?;
    let path = state.blobs.path(&id)?;
    let (mut input, size) = crate::streaming_ffi::open_input(&path, ORIGINAL_LIMIT)
        .map_err(|_| WorkflowError::Invalid)?;
    if size != asset.byte_size {
        return Err(WorkflowError::Invalid);
    }
    let mut original = state.store()?.transfer_file()?;
    let mut hasher = blake3::Hasher::new();
    let mut count = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        cancel.check()?;
        let n = input.read(&mut buffer).map_err(storage)?;
        if n == 0 {
            break;
        }
        count = count.checked_add(n as u64).ok_or(WorkflowError::Limit)?;
        if count > size {
            return Err(WorkflowError::Invalid);
        }
        hasher.update(&buffer[..n]);
        original.write_all(&buffer[..n]).map_err(storage)?;
    }
    if count != size || hasher.finalize().as_bytes() != &id.bytes() {
        return Err(WorkflowError::Invalid);
    }
    original.as_file().sync_all().map_err(storage)?;
    original
        .as_file_mut()
        .seek(SeekFrom::Start(0))
        .map_err(storage)?;
    Ok(original)
}
fn validate_png(
    state: &crate::project::ProjectState,
    original: &mut tempfile::NamedTempFile,
    asset: &pb::AddAsset,
    cancel: &Cancellation,
) -> WorkflowResult<()> {
    if asset.byte_size == 0
        || asset.byte_size > ORIGINAL_LIMIT
        || asset.format != "png"
        || asset.source != "capture"
        || asset.orientation != 1
        || !matches!(asset.bit_depth, 8 | 16)
        || asset.width == 0
        || asset.height == 0
        || asset.width > 32768
        || asset.height > 32768
        || u64::from(asset.width) * u64::from(asset.height) > 50_000_000
    {
        return Err(WorkflowError::Invalid);
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(asset.byte_size as usize)
        .map_err(|_| WorkflowError::Memory {
            estimated: asset.byte_size,
            budget: SOURCE_WORK,
        })?;
    original
        .as_file_mut()
        .seek(SeekFrom::Start(0))
        .map_err(storage)?;
    original
        .as_file_mut()
        .take(asset.byte_size + 1)
        .read_to_end(&mut bytes)
        .map_err(storage)?;
    if bytes.len() as u64 != asset.byte_size {
        return Err(WorkflowError::Invalid);
    }
    let mut scratch = state.store()?.transfer_file()?;
    let mut source = vw_raster::PngSpool::decode(
        &bytes,
        scratch.as_file_mut(),
        vw_raster::SpoolLimits {
            decode: vw_raster::DecodeLimits {
                max_encoded_bytes: ORIGINAL_LIMIT as usize,
                max_pixels: 50_000_000,
                max_memory_bytes: SOURCE_WORK - 8 * 1024 * 1024,
            },
            max_scratch_bytes: 400_000_000,
        },
        &|| cancel.is_cancelled(),
    )
    .map_err(|_| {
        if cancel.is_cancelled() {
            WorkflowError::Cancelled
        } else {
            WorkflowError::Invalid
        }
    })?;
    let info = source.info();
    if info.width != asset.width
        || info.height != asset.height
        || u32::from(info.bit_depth) != asset.bit_depth
        || info.orientation_applied != 1
        || info.source_asset.as_str() != asset.asset_id
        || info.icc.as_deref().unwrap_or_default() != asset.icc_profile
        || asset.color_space
            != if info.icc.is_some() {
                "ICC"
            } else {
                "untagged"
            }
    {
        return Err(WorkflowError::Invalid);
    }
    let mut alpha = false;
    for y in (0..asset.height).step_by(16) {
        cancel.check()?;
        alpha |= source
            .read_region(
                vw_raster::Region {
                    x: 0,
                    y,
                    width: asset.width,
                    height: 16.min(asset.height - y),
                },
                SOURCE_WORK - 8 * 1024 * 1024,
                &|| cancel.is_cancelled(),
            )
            .map_err(|_| WorkflowError::Invalid)?
            .pixels
            .has_alpha();
    }
    if alpha != asset.has_alpha {
        return Err(WorkflowError::Invalid);
    }
    Ok(())
}

#[uniffi::export]
pub async fn prepare_capture_delivery(
    source: Arc<ProjectSession>,
    target: Arc<ProjectSession>,
    source_binding: WorkflowBinding,
    target_binding: WorkflowBinding,
    metadata: WorkflowMetadata,
    memory_budget_bytes: u64,
    cancellation: Arc<Cancellation>,
) -> WorkflowResult<Arc<CaptureDeliveryPlan>> {
    source_binding.validate()?;
    target_binding.validate()?;
    metadata.validate()?;
    if source_binding.project_id == target_binding.project_id {
        return Err(WorkflowError::Invalid);
    }
    let permit = Permit::new()?;
    let target_id = Id::try_from(target_binding.project_id.clone())?;
    let src = source.clone();
    let token = cancellation.clone();
    let expected = source_binding.clone();
    let captured = source
        .worker
        .call(move |state| {
            Ok((|| {
                check_live(&src.closed, &token)?;
                admit(state, memory_budget_bytes, false, SOURCE_WORK)?;
                check_binding(state, &expected)?;
                let project = state.project()?;
                // This is a just-captured, immutable recovery project, not a general
                // merge command that could silently omit annotations or instructions.
                if project.documents.len() != 1
                    || project.assets.len() != 1
                    || project.layers.len() != 1
                    || project.semantic_snapshots.len() > 1
                    || !project.objects.is_empty()
                    || !project.groups.is_empty()
                    || !project.instructions.is_empty()
                    || !project.results.is_empty()
                    || !project.mask_versions.is_empty()
                    || state.replica.is_some()
                {
                    return Err(WorkflowError::Invalid);
                }
                let doc = project
                    .documents
                    .values()
                    .next()
                    .ok_or(WorkflowError::Missing)?;
                identity(&doc.definition)?;
                if !doc.pages.is_empty() {
                    return Err(WorkflowError::Invalid);
                }
                let asset = project
                    .assets
                    .get(&AssetId::try_from(doc.definition.primary_asset_id.clone())?)
                    .ok_or(WorkflowError::Missing)?;
                let capture = doc
                    .definition
                    .capture
                    .as_ref()
                    .ok_or(WorkflowError::Invalid)?;
                let rect = capture
                    .geometry
                    .as_ref()
                    .and_then(|g| g.client_rect_host.as_ref())
                    .ok_or(WorkflowError::Invalid)?;
                if i64::from(rect.w) != i64::from(asset.width)
                    || i64::from(rect.h) != i64::from(asset.height)
                {
                    return Err(WorkflowError::Invalid);
                }
                let layer = project
                    .layers
                    .values()
                    .next()
                    .ok_or(WorkflowError::Missing)?;
                if layer.definition.document_id != doc.definition.document_id
                    || !layer.visible
                    || layer.locked
                    || layer.opacity != 1.0
                    || layer.blend != "normal"
                    || layer.definition.kind != "annotation"
                {
                    return Err(WorkflowError::Invalid);
                }
                if asset.captured_at_ms
                    != doc
                        .definition
                        .capture
                        .as_ref()
                        .ok_or(WorkflowError::Invalid)?
                        .captured_at_ms
                {
                    return Err(WorkflowError::Invalid);
                }
                let mut original = copy_original(state, asset, &token)?;
                validate_png(state, &mut original, asset, &token)?;
                let view = vw_semantics::CaptureView::new(
                    project,
                    &state.view_revision()?,
                    &Id::try_from(expected.document_id)?,
                )?;
                let mut snapshots = Vec::new();
                for (id, stored) in &project.semantic_snapshots {
                    let exact = view
                        .load(id, token.as_ref())?
                        .for_project(target_id.clone(), token.as_ref())?;
                    let mut op = stored.definition.clone();
                    op.elements_json_zstd = exact.encode(token.as_ref())?;
                    snapshots.push(op);
                }
                check_live(&src.closed, &token)?;
                Ok(Captured {
                    original,
                    asset: asset.clone(),
                    document: doc.definition.clone(),
                    layer: layer.definition.clone(),
                    snapshots,
                    _permit: permit,
                })
            })())
        })
        .await??;
    let owner = target.clone();
    let token = cancellation;
    target
        .worker
        .call(move |state| {
            Ok((|| {
                check_live(&owner.closed, &token)?;
                admit(state, memory_budget_bytes, true, PLAN_WORK)?;
                check_binding(state, &target_binding)?;
                check_metadata(state, &metadata)?;
                if state.replica.is_some()
                    || state.store()?.local_device()? != *state.store()?.host_device()
                {
                    return Err(WorkflowError::Invalid);
                }
                let document_id = Id::from_proto(captured.document.document_id.as_ref())?;
                if state.project()?.documents.contains_key(&document_id)
                    || state
                        .project()?
                        .layers
                        .contains_key(&Id::from_proto(captured.layer.layer_id.as_ref())?)
                    || captured.snapshots.iter().any(|s| {
                        Id::from_proto(s.snapshot_id.as_ref()).is_ok_and(|id| {
                            state
                                .project()
                                .is_ok_and(|p| p.semantic_snapshots.contains_key(&id))
                        })
                    })
                {
                    return Err(WorkflowError::ReusedTransaction);
                }
                // An unchanged screen may have the same original hash on several
                // captures. Reuse compatible immutable pixels; capture time belongs to
                // each document. Never rewrite the first asset's provenance fields.
                let mut kinds = Vec::new();
                if let Some(existing) = state
                    .project()?
                    .assets
                    .get(&AssetId::try_from(captured.asset.asset_id.clone())?)
                {
                    if !same_pixels(existing, &captured.asset) {
                        return Err(WorkflowError::Invalid);
                    }
                } else {
                    kinds.push(pb::op::Kind::AddAsset(captured.asset.clone()));
                }
                kinds.push(pb::op::Kind::CreateDocument(captured.document.clone()));
                kinds.push(pb::op::Kind::CreateLayer(captured.layer.clone()));
                kinds.extend(
                    captured
                        .snapshots
                        .iter()
                        .cloned()
                        .map(pb::op::Kind::AddSemanticSnapshot),
                );
                let mut ops = Vec::new();
                for (index, kind) in kinds.into_iter().enumerate() {
                    ops.push(pb::Op {
                        op_id: Some(pb::OpId {
                            device_id: metadata.device_id.clone(),
                            lamport: metadata
                                .first_lamport
                                .checked_add(index as u64)
                                .ok_or(WorkflowError::Limit)?,
                        }),
                        kind: Some(kind),
                    });
                }
                metadata
                    .first_lamport
                    .checked_add(ops.len() as u64)
                    .ok_or(WorkflowError::Limit)?;
                let transaction = pb::Transaction {
                    txn_id: Some(Id::try_from(metadata.transaction_id.clone())?.to_proto()),
                    project_id: Some(Id::try_from(target_binding.project_id.clone())?.to_proto()),
                    device_id: metadata.device_id,
                    base_revision: Some(state.store()?.revision()?),
                    created_at_wall_ms: metadata.created_at_ms,
                    gesture_id: None,
                    ops,
                };
                admit_transaction(state, &transaction, memory_budget_bytes, PLAN_WORK)?;
                let (capture_session_id, frame_id, geometry_revision) =
                    identity(&captured.document)?;
                let info = CaptureDeliveryInfo {
                    transaction_id: metadata.transaction_id,
                    source: source_binding,
                    target: target_binding,
                    document_id: document_id.to_string(),
                    source_asset_id: captured.asset.asset_id.clone(),
                    capture_session_id,
                    frame_id,
                    geometry_revision,
                    encoded_bytes: captured.asset.byte_size,
                    operation_count: transaction.ops.len() as u32,
                };
                check_live(&owner.closed, &token)?;
                Ok(Arc::new(CaptureDeliveryPlan {
                    worker: owner.worker.clone(),
                    closed: owner.closed.clone(),
                    disposed: Arc::new(AtomicBool::new(false)),
                    prepared: Mutex::new(Some(Arc::new(Prepared {
                        captured,
                        transaction,
                        info,
                        budget: memory_budget_bytes,
                    }))),
                }))
            })())
        })
        .await?
}

impl CaptureDeliveryPlan {
    fn get(&self) -> WorkflowResult<Arc<Prepared>> {
        if self.closed.load(Ordering::Acquire) || self.disposed.load(Ordering::Acquire) {
            return Err(WorkflowError::Closed);
        }
        self.prepared
            .lock()
            .map_err(|_| WorkflowError::Closed)?
            .as_ref()
            .cloned()
            .ok_or(WorkflowError::Closed)
    }
}
struct CancelledRead<'a> {
    file: std::fs::File,
    token: &'a Cancellation,
    left: u64,
}
impl Read for CancelledRead<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if self.token.is_cancelled() {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        let amount = bytes.len().min(self.left as usize);
        if amount == 0 {
            return Ok(0);
        }
        let size = self.file.read(&mut bytes[..amount])?;
        self.left -= size as u64;
        Ok(size)
    }
}
#[uniffi::export]
impl CaptureDeliveryPlan {
    pub fn describe(&self) -> WorkflowResult<CaptureDeliveryInfo> {
        Ok(self.get()?.info.clone())
    }
    pub async fn commit(&self, cancellation: Arc<Cancellation>) -> WorkflowResult<WorkflowReceipt> {
        let prepared = self.get()?;
        let closed = self.closed.clone();
        let disposed = self.disposed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&closed, &cancellation)?;
                    if disposed.load(Ordering::Acquire) {
                        return Err(WorkflowError::Closed);
                    }
                    admit_transaction(state, &prepared.transaction, prepared.budget, PLAN_WORK)?;
                    let id = prepared
                        .transaction
                        .txn_id
                        .as_ref()
                        .ok_or(WorkflowError::Invalid)?;
                    let duplicate = if let Some(previous) = state.previous(id)? {
                        if previous != &prepared.transaction {
                            return Err(WorkflowError::ReusedTransaction);
                        }
                        true
                    } else {
                        check_binding(state, &prepared.info.target)?;
                        false
                    };
                    if !duplicate {
                        let mut file = prepared
                            .captured
                            .original
                            .as_file()
                            .try_clone()
                            .map_err(storage)?;
                        file.seek(SeekFrom::Start(0)).map_err(storage)?;
                        let mut reader = CancelledRead {
                            file,
                            token: &cancellation,
                            left: prepared.info.encoded_bytes,
                        };
                        let stored = state.blobs.put_reader(&mut reader).map_err(|_| {
                            if cancellation.is_cancelled() {
                                WorkflowError::Cancelled
                            } else {
                                WorkflowError::Storage
                            }
                        })?;
                        if reader.left != 0 || stored.as_str() != prepared.info.source_asset_id {
                            return Err(WorkflowError::Invalid);
                        }
                        check_live(&closed, &cancellation)?;
                        if disposed.load(Ordering::Acquire) {
                            return Err(WorkflowError::Closed);
                        }
                    }
                    let revision = if duplicate {
                        state.info()?
                    } else {
                        state.commit(
                            &prepared.transaction,
                            &DeviceId::try_from(prepared.transaction.device_id.clone())?,
                            prepared.transaction.created_at_wall_ms,
                        )?
                    };
                    Ok(WorkflowReceipt {
                        transaction_id: prepared.info.transaction_id.clone(),
                        revision,
                        duplicate,
                    })
                })())
            })
            .await?
    }
    pub fn dispose(&self) {
        self.disposed.store(true, Ordering::Release);
        if let Ok(mut value) = self.prepared.lock() {
            *value = None;
        }
    }
}
impl Drop for CaptureDeliveryPlan {
    fn drop(&mut self) {
        self.dispose();
    }
}

fn same_pixels(a: &pb::AddAsset, b: &pb::AddAsset) -> bool {
    a.asset_id == b.asset_id
        && a.format == b.format
        && a.width == b.width
        && a.height == b.height
        && a.orientation == b.orientation
        && a.bit_depth == b.bit_depth
        && a.has_alpha == b.has_alpha
        && a.color_space == b.color_space
        && a.icc_profile == b.icc_profile
        && a.byte_size == b.byte_size
}

#[uniffi::export]
impl ProjectSession {
    /// Latest host-authored lossless Windows capture after an accepted journal
    /// position. Coalesces notifications only: earlier documents remain saved.
    /// Readiness requires the exact original hash, not merely accepted metadata.
    pub async fn received_capture(
        &self,
        after_host_seq: u64,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<Option<ReceivedCapture>> {
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&closed, &cancellation)?;
                    admit(state, memory_budget_bytes, false, 1024 * 1024)?;
                    let store = state.store()?;
                    if after_host_seq > store.revision()?.host_seq {
                        return Err(WorkflowError::Stale);
                    }
                    let mut latest = None;
                    for (txn, ack) in store.accepted_transactions() {
                        cancellation.check()?;
                        if ack.host_seq <= after_host_seq
                            || txn.device_id != store.host_device().as_str()
                        {
                            continue;
                        }
                        for op in &txn.ops {
                            let Some(pb::op::Kind::CreateDocument(definition)) = &op.kind else {
                                continue;
                            };
                            let Ok((session, frame, geometry)) = identity(definition) else {
                                continue;
                            };
                            let id = Id::from_proto(definition.document_id.as_ref())?;
                            if !state
                                .project()?
                                .documents
                                .get(&id)
                                .is_some_and(|d| &d.definition == definition)
                            {
                                continue;
                            }
                            if latest
                                .as_ref()
                                .is_none_or(|(seq, _, _, _, _)| ack.host_seq > *seq)
                            {
                                latest = Some((ack.host_seq, id, session, frame, geometry));
                            }
                        }
                    }
                    let Some((
                        created_host_seq,
                        id,
                        capture_session_id,
                        frame_id,
                        geometry_revision,
                    )) = latest
                    else {
                        return Ok(None);
                    };
                    let document = state
                        .project()?
                        .documents
                        .get(&id)
                        .ok_or(WorkflowError::Missing)?;
                    let source_asset_id = document.definition.primary_asset_id.clone();
                    let asset = state
                        .project()?
                        .assets
                        .get(&AssetId::try_from(source_asset_id.clone())?)
                        .ok_or(WorkflowError::Missing)?;
                    let path = state
                        .blobs
                        .path(&AssetId::try_from(source_asset_id.clone())?)?;
                    let original_verified = if !path.try_exists().map_err(storage)? {
                        false
                    } else {
                        let mut original = copy_original(state, asset, &cancellation)?;
                        // copy_original is streamed, hash-verified and owns exact bytes.
                        original
                            .as_file_mut()
                            .seek(SeekFrom::Start(0))
                            .map_err(storage)?;
                        true
                    };
                    Ok(Some(ReceivedCapture {
                        binding: current(state, id.as_str())?,
                        created_host_seq,
                        source_asset_id,
                        capture_session_id,
                        frame_id,
                        geometry_revision,
                        original_verified,
                    }))
                })())
            })
            .await?
    }
}
