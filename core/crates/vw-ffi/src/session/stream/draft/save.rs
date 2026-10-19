use super::*;
use crate::project::ProjectState;
use std::{
    collections::BTreeSet,
    io::{Read, Seek, SeekFrom},
    sync::Mutex as StdMutex,
};
use vw_model::AssetId;
use vw_proto::Message;

#[derive(Clone, Debug, uniffi::Record)]
pub struct CaptureDraftSaveInfo {
    pub draft: CaptureDraftInfo,
    pub source: WorkflowBinding,
    pub target: WorkflowBinding,
    pub original_asset_id: Option<String>,
    pub saved_asset_id: String,
    pub degraded: bool,
    pub transaction_id: String,
    pub operation_count: u32,
}
struct SavePixels {
    // A read-only file clone retains exact draft bytes while Life owns its
    // SQLite/TempDir lifetime. Lossless retains the verified decoder owner too.
    file: std::fs::File,
    asset: AssetId,
    bytes: u64,
    lossless: bool,
    _original: Option<Arc<super::super::verified::Original>>,
}
struct Prepared {
    life: Arc<Life>,
    pixels: Arc<SavePixels>,
    transaction: pb::Transaction,
    info: CaptureDraftSaveInfo,
    budget: u64,
    _seal: Arc<Seal>,
}
#[derive(uniffi::Object)]
pub struct CaptureDraftSave {
    prepared: StdMutex<Option<Arc<Prepared>>>,
    disposed: Arc<AtomicBool>,
}

// Mirrors both CAS bindings, exact background owner and cancellation at the native boundary.
#[allow(clippy::too_many_arguments)]
pub(super) async fn prepare(
    life: Arc<Life>,
    source: WorkflowBinding,
    target: WorkflowBinding,
    metadata: WorkflowMetadata,
    background: Arc<CaptureBackground>,
    allow_degraded: bool,
    budget: u64,
    cancellation: Arc<Cancellation>,
) -> WorkflowResult<Arc<CaptureDraftSave>> {
    source.validate()?;
    target.validate()?;
    metadata.validate()?;
    let call = life.calls.enter()?;
    life.check(&cancellation)?;
    if life.saved.load(Ordering::Acquire)
        || source.project_id != life.info.draft_id
        || source.document_id != life.info.document_id
        || source.project_id == target.project_id
    {
        return Err(WorkflowError::Invalid);
    }
    // Exact handle lease and complete immutable metadata, not a matching numeric
    // FrameId from a later link. A timeout preview has no original owner to take.
    if !Arc::ptr_eq(&background.lease, &life.lease)
        || background.info() != life.info.frame
        || background.draft_id() != life.info.draft_id
        || background.capture_info(false)? != life.capture
    {
        return Err(WorkflowError::Stale);
    }
    let original = background.verified_original();
    if allow_degraded {
        let signal = life.lease.endpoint.signal.borrow().clone();
        if original.is_some()
            || signal.stage != super::super::api::StreamStage::FrozenDegraded
            || signal.source_generation != life.info.frame.source_generation
        {
            return Err(WorkflowError::Stale);
        }
    } else if original.is_none() {
        return Err(WorkflowError::Missing);
    }
    let proof = pb::StreamSaveProvenance {
        draft_id: Some(Id::try_from(life.info.draft_id.clone())?.to_proto()),
        transaction_id: Some(Id::try_from(metadata.transaction_id.clone())?.to_proto()),
        target_project_id: Some(Id::try_from(target.project_id.clone())?.to_proto()),
        target_host_seq: target.host_seq,
        target_state_hash: target.state_hash.clone(),
        connection_epoch: life.info.connection_epoch,
        source_generation: life.info.frame.source_generation,
        saved_asset_id: String::new(),
        original_asset_id: String::new(),
    };
    let seal = Seal::take(life.clone())?;
    let retained_seal = seal.clone();
    let owner = life.clone();
    let expected = source.clone();
    let token = cancellation.clone();
    let image = original.clone();
    let first_call = call.clone();
    let (kinds, pixels) = life
        .project
        .worker
        .call(move |state| {
            Ok((|| {
                let _seal = retained_seal;
                let _call = first_call;
                owner.check(&token)?;
                if owner.project.active_gestures.load(Ordering::Acquire) != 0 {
                    return Err(WorkflowError::Backpressure);
                }
                admit(state, budget, true, PLAN_WORK)?;
                check_binding(state, &expected)?;
                let pixels = if let Some(original) = image {
                    SavePixels {
                        file: original
                            .encoded_file()
                            .try_clone()
                            .map_err(|_| WorkflowError::Storage)?,
                        asset: original.offer().asset.clone(),
                        bytes: original.offer().bytes,
                        lossless: true,
                        _original: Some(original),
                    }
                } else {
                    let asset = AssetId::try_from(owner.info.preview_asset_id.clone())?;
                    let bytes = state.blobs.verify(&asset)?;
                    token.check()?;
                    owner.check(&token)?;
                    let file = std::fs::File::open(state.blobs.path(&asset)?)
                        .map_err(|_| WorkflowError::Storage)?;
                    SavePixels {
                        file,
                        asset,
                        bytes,
                        lossless: false,
                        _original: None,
                    }
                };
                let kinds = build_kinds(state, &owner, &pixels, proof, &token)?;
                Ok((kinds, Arc::new(pixels)))
            })())
        })
        .await??;
    let owner = life.clone();
    let expected = target.clone();
    let token = cancellation.clone();
    life.target
        .worker
        .call(move |state| {
            Ok((|| {
                let _call = call;
                owner.check(&token)?;
                admit(state, budget, true, PLAN_WORK)?;
                check_binding(state, &expected)?;
                check_metadata(state, &metadata)?;
                let mut kinds = kinds;
                if let Some(pb::op::Kind::AddAsset(asset)) = kinds.first()
                    && let Some(existing) = state
                        .project()?
                        .assets
                        .get(&AssetId::try_from(asset.asset_id.clone())?)
                {
                    if !same_pixels(existing, asset) {
                        return Err(WorkflowError::Invalid);
                    }
                    kinds.remove(0);
                }
                let count = kinds.len();
                if count == 0 || count > 4096 {
                    return Err(WorkflowError::Limit);
                }
                metadata
                    .first_lamport
                    .checked_add(count as u64)
                    .ok_or(WorkflowError::Limit)?;
                let transaction = pb::Transaction {
                    txn_id: Some(Id::try_from(metadata.transaction_id.clone())?.to_proto()),
                    project_id: Some(Id::try_from(expected.project_id.clone())?.to_proto()),
                    device_id: metadata.device_id.clone(),
                    base_revision: Some(state.store()?.revision()?),
                    created_at_wall_ms: metadata.created_at_ms,
                    gesture_id: None,
                    ops: kinds
                        .into_iter()
                        .enumerate()
                        .map(|(index, kind)| pb::Op {
                            op_id: Some(pb::OpId {
                                device_id: metadata.device_id.clone(),
                                lamport: metadata.first_lamport + index as u64,
                            }),
                            kind: Some(kind),
                        })
                        .collect(),
                };
                if transaction.encoded_len() > 4 * 1024 * 1024 {
                    return Err(WorkflowError::Limit);
                }
                admit_transaction(state, &transaction, budget, PLAN_WORK)?;
                owner.check(&token)?;
                let info = CaptureDraftSaveInfo {
                    draft: owner.info.clone(),
                    source,
                    target,
                    original_asset_id: pixels.lossless.then(|| pixels.asset.to_string()),
                    saved_asset_id: pixels.asset.to_string(),
                    degraded: !pixels.lossless,
                    transaction_id: metadata.transaction_id,
                    operation_count: count as u32,
                };
                Ok(Arc::new(CaptureDraftSave {
                    disposed: Arc::new(AtomicBool::new(false)),
                    prepared: StdMutex::new(Some(Arc::new(Prepared {
                        life: owner,
                        pixels,
                        transaction,
                        info,
                        budget,
                        _seal: seal,
                    }))),
                }))
            })())
        })
        .await?
}

fn build_kinds(
    state: &ProjectState,
    life: &Life,
    pixels: &SavePixels,
    mut proof: pb::StreamSaveProvenance,
    cancel: &Cancellation,
) -> WorkflowResult<Vec<pb::op::Kind>> {
    use pb::op::Kind;
    let project = state.project()?;
    // This concrete annotation draft has one immutable background. Unsupported
    // dependencies refuse rather than quietly losing Results/masks/semantics.
    if project.documents.len() != 1
        || project.assets.len() != 1
        || !project.results.is_empty()
        || !project.mask_versions.is_empty()
        || !project.semantic_snapshots.is_empty()
        || state.replica.is_some()
        || project.layers.len() > 64
        || project.objects.len() > 1024
        || project.groups.len() > 256
        || project.instructions.len() > 256
    {
        return Err(WorkflowError::Invalid);
    }
    let document = Id::try_from(life.info.document_id.clone())?;
    let document_value = project
        .documents
        .get(&document)
        .ok_or(WorkflowError::Missing)?;
    let definition = &document_value.definition;
    if definition.primary_asset_id != life.info.preview_asset_id
        || definition.capture.as_ref() != Some(&life.capture)
        || !document_value.pages.is_empty()
    {
        return Err(WorkflowError::Stale);
    }
    let asset = pixels::definition(pixels.asset.clone(), pixels.bytes, &life.info.frame, cancel)?;
    let mut capture = life.capture.clone();
    capture.lossless = pixels.lossless;
    capture.degraded = !pixels.lossless;
    proof.saved_asset_id = pixels.asset.to_string();
    proof.original_asset_id = if pixels.lossless {
        pixels.asset.to_string()
    } else {
        String::new()
    };
    capture.stream_save = Some(proof);
    vw_model::validate_capture(&capture)?;
    let mut doc = definition.clone();
    doc.primary_asset_id = asset.asset_id.clone();
    doc.capture = Some(capture);
    let mut kinds = vec![Kind::AddAsset(asset), Kind::CreateDocument(doc)];
    for layer in project.layers.values() {
        cancel.check()?;
        if layer.definition.document_id != Some(document.to_proto())
            || layer.definition.kind != "annotation"
            || f64::from(layer.opacity as f32) != layer.opacity
        {
            return Err(WorkflowError::Invalid);
        }
        kinds.push(Kind::CreateLayer(layer.definition.clone()));
    }
    for object in project.objects.values() {
        cancel.check()?;
        if object.document_id != document {
            return Err(WorkflowError::Invalid);
        }
        match &object.state.shape {
            Some(pb::object_state::Shape::Stroke(value))
                if value
                    .brush
                    .as_ref()
                    .is_some_and(|b| b.family == "vector_eraser") =>
            {
                return Err(WorkflowError::Invalid);
            }
            Some(
                pb::object_state::Shape::ResultId(_)
                | pb::object_state::Shape::SelectionRaster(_)
                | pb::object_state::Shape::Adjustment(_),
            )
            | None => return Err(WorkflowError::Invalid),
            Some(pb::object_state::Shape::Marker(value)) if !value.element_eids.is_empty() => {
                return Err(WorkflowError::Invalid);
            }
            _ => {}
        }
        let mut value = object.state.clone();
        value.group_id = None;
        value.locked = false;
        kinds.push(Kind::CreateObject(pb::CreateObject {
            document_id: Some(document.to_proto()),
            state: Some(value),
        }));
    }
    // Bottom-up existing canonical group forest; bounded <=256^2 scans. Group
    // operations precede lock restoration and keep every compound membership.
    let mut made = BTreeSet::new();
    while made.len() < project.groups.len() {
        let before = made.len();
        for (id, group) in &project.groups {
            cancel.check()?;
            if made.contains(id) {
                continue;
            }
            if group.document_id != document {
                return Err(WorkflowError::Invalid);
            }
            let children = project
                .groups
                .iter()
                .filter(|(_, g)| g.parent.as_ref() == Some(id));
            if children.clone().any(|(child, _)| !made.contains(child)) {
                continue;
            }
            let members: Vec<_> = project
                .objects
                .iter()
                .filter(|(_, o)| o.state.group_id.as_ref() == Some(&id.to_proto()))
                .map(|(member, _)| member.to_proto())
                .chain(children.map(|(child, _)| child.to_proto()))
                .collect();
            if members.is_empty() {
                return Err(WorkflowError::Invalid);
            }
            kinds.push(Kind::Group(pb::Group {
                group_id: Some(id.to_proto()),
                object_ids: members,
            }));
            made.insert(id.clone());
        }
        if made.len() == before {
            return Err(WorkflowError::Invalid);
        }
    }
    for instruction in project.instructions.values() {
        cancel.check()?;
        if instruction.definition.document_id != Some(document.to_proto()) {
            return Err(WorkflowError::Invalid);
        }
        kinds.push(Kind::SetInstruction(instruction.definition.clone()));
    }
    for object in project.objects.values().filter(|o| o.state.locked) {
        kinds.push(Kind::SetProperty(pb::SetProperty {
            object_id: object.state.object_id.clone(),
            property: "locked".into(),
            value: Some(pb::PropertyValue {
                value: Some(pb::property_value::Value::Flag(true)),
            }),
        }));
    }
    for layer in project.layers.values() {
        kinds.push(Kind::UpdateLayer(pb::UpdateLayer {
            layer_id: layer.definition.layer_id.clone(),
            name: None,
            order_key: None,
            visible: Some(layer.visible),
            locked: Some(layer.locked),
            opacity: Some(layer.opacity as f32),
            blend: Some(layer.blend.clone()),
        }));
    }
    if kinds.len() > 4096 {
        return Err(WorkflowError::Limit);
    }
    Ok(kinds)
}

struct CopyOriginal<'a> {
    file: std::fs::File,
    left: u64,
    token: &'a Cancellation,
    life: &'a Life,
}
impl Read for CopyOriginal<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if self.token.is_cancelled()
            || self.life.disposed.load(Ordering::Acquire)
            || self.life.lease.check().is_err()
        {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        let amount = bytes.len().min(self.left as usize).min(65536);
        let n = self.file.read(&mut bytes[..amount])?;
        self.left -= n as u64;
        Ok(n)
    }
}
impl CaptureDraftSave {
    fn get(&self) -> WorkflowResult<Arc<Prepared>> {
        if self.disposed.load(Ordering::Acquire) {
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
#[uniffi::export]
impl CaptureDraftSave {
    pub fn describe(&self) -> WorkflowResult<CaptureDraftSaveInfo> {
        Ok(self.get()?.info.clone())
    }
    pub async fn commit(&self, cancellation: Arc<Cancellation>) -> WorkflowResult<WorkflowReceipt> {
        let prepared = self.get()?;
        let call = prepared.life.calls.enter()?;
        let owner = prepared.life.target.clone();
        let disposed = self.disposed.clone();
        owner
            .worker
            .call(move |state| {
                Ok((|| {
                    let _call = call;
                    // Exact already-durable retries remain inspectable after a link
                    // ends. A new write still requires the original live epoch.
                    check_live(&prepared.life.target.closed, &cancellation)?;
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
                        false
                    };
                    if !duplicate {
                        prepared.life.check(&cancellation)?;
                        if disposed.load(Ordering::Acquire) {
                            return Err(WorkflowError::Closed);
                        }
                        check_binding(state, &prepared.info.target)?;
                        let mut file = prepared
                            .pixels
                            .file
                            .try_clone()
                            .map_err(|_| WorkflowError::Storage)?;
                        file.seek(SeekFrom::Start(0))
                            .map_err(|_| WorkflowError::Storage)?;
                        let mut read = CopyOriginal {
                            file,
                            left: prepared.pixels.bytes,
                            token: &cancellation,
                            life: &prepared.life,
                        };
                        let asset = state.blobs.put_reader(&mut read).map_err(|_| {
                            if cancellation.is_cancelled() {
                                WorkflowError::Cancelled
                            } else {
                                WorkflowError::Storage
                            }
                        })?;
                        if read.left != 0 || asset != prepared.pixels.asset {
                            return Err(WorkflowError::Invalid);
                        }
                        prepared.life.check(&cancellation)?;
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
                    // No cancellation check after durable commit. Retain the exact
                    // result for retry; volatile stream notification is secondary.
                    prepared.life.saved.store(true, Ordering::Release);
                    if let Ok(mut notice) = prepared.life.lease.saved.lock() {
                        *notice = Some((
                            Id::try_from(prepared.life.info.draft_id.clone())?,
                            prepared
                                .pixels
                                .lossless
                                .then(|| prepared.pixels.asset.clone()),
                            prepared.life.info.frame.source_generation,
                        ));
                        prepared.life.lease.endpoint.wake.notify_one();
                    }
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
        if let Ok(mut prepared) = self.prepared.lock() {
            *prepared = None;
        }
    }
}
impl Drop for CaptureDraftSave {
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
