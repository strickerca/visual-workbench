use super::*;
use crate::{
    ProjectSession, WorkflowBinding, WorkflowMetadata, WorkflowReceipt, project::ProjectState,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use vw_model::{AssetId, DeviceId, Id, OrderKey};
use vw_proto::v1 as pb;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MaterializedProof {
    schema: u32,
    materialization: String,
    source_asset_id: String,
    composite_asset_id: String,
    acceptance_mask_asset_id: Option<String>,
    changed_outside: u64,
    proof: vw_ai::proof::Proof,
}
pub(super) fn validate_materialized(
    json: &str,
    source: &AssetId,
    composite: &AssetId,
    acceptance: Option<&str>,
) -> AiEditResult<()> {
    if json.len() > 16 * 1024 {
        return Err(AiEditError::Proof);
    }
    let p: MaterializedProof = serde_json::from_str(json).map_err(|_| AiEditError::Proof)?;
    if p.schema != 1
        || p.materialization != "vw-ai-composite-v1"
        || p.source_asset_id != source.as_str()
        || p.composite_asset_id != composite.as_str()
        || p.acceptance_mask_asset_id.as_deref() != acceptance
        || p.changed_outside != 0
        || p.proof.changed_outside != 0
        || p.proof.outside_sha256_before != p.proof.outside_sha256_after
        || p.proof
            .acceptance
            .as_ref()
            .is_some_and(|p| p.changed_unaccepted != 0 || p.before_sha256 != p.after_sha256)
        || p.proof.acceptance.is_some() != acceptance.is_some()
    {
        return Err(AiEditError::Proof);
    }
    Ok(())
}
pub(super) struct SavePayload {
    options: AiSaveOptions,
    raw: Arc<Vec<u8>>,
    composite: Vec<u8>,
    acceptance: Option<Vec<u8>>,
    assets: Vec<pb::AddAsset>,
    result: pb::AddResult,
    proof: AiProofInfo,
    budget: u64,
    resident: u64,
    transaction: Mutex<Option<pb::Transaction>>,
}
fn same(a: &AiSaveOptions, b: &AiSaveOptions) -> bool {
    a.expected == b.expected
        && a.result_id == b.result_id
        && a.layer_id == b.layer_id
        && a.object_id == b.object_id
        && a.replace_object_id == b.replace_object_id
        && a.metadata.transaction_id == b.metadata.transaction_id
        && a.metadata.device_id == b.metadata.device_id
        && a.metadata.first_lamport == b.metadata.first_lamport
        && a.metadata.created_at_ms == b.metadata.created_at_ms
}
pub(super) fn prepare(
    s: &mut request::RequestState,
    options: AiSaveOptions,
    stop: &request::Stop,
) -> AiEditResult<Arc<SavePayload>> {
    if let Some(old) = &s.save {
        if !same(&old.options, &options) {
            return Err(AiEditError::Stale);
        }
        return Ok(old.clone());
    }
    options.expected.validate()?;
    options.metadata.validate()?;
    for id in [&options.result_id, &options.layer_id, &options.object_id] {
        Id::try_from(id.clone())?;
    }
    if let Some(id) = &options.replace_object_id {
        Id::try_from(id.clone())?;
    }
    if options.expected != s.options.binding {
        return Err(AiEditError::Stale);
    }
    let done = s.current()?;
    let n = u64::from(done.image().width()) * u64::from(done.image().height());
    // Candidate state stays resident while immutable output payloads are encoded
    // and later admitted by the project worker. Raw bytes are shared, not cloned.
    let resident = s.resident();
    memory(
        resident + n * 40 + RAW_RESERVE + n * 2 + 33 * 1024 * 1024,
        s.options.memory_budget_bytes,
    )?;
    stop.check()?;
    let composite = done.image().encode_png(
        vw_ai::Limits {
            memory_bytes: s.options.memory_budget_bytes - resident - n * 2 - 1024 * 1024,
        },
        stop,
    )?;
    let acceptance = done
        .acceptance_mask()
        .map(|m| m.encode_mask_png(vw_mask::Region::full(m.size())))
        .transpose()?;
    let output_id = AssetId::hash(s.raw.as_slice());
    let composite_id = AssetId::hash(&composite);
    let acceptance_id = acceptance.as_deref().map(AssetId::hash);
    let mut assets = vec![
        s.raw_asset.clone().ok_or(AiEditError::Proof)?,
        pb::AddAsset {
            asset_id: composite_id.to_string(),
            format: "png".into(),
            width: done.image().width(),
            height: done.image().height(),
            orientation: 1,
            bit_depth: u32::from(done.image().bit_depth()),
            has_alpha: done.image().pixels().has_alpha(),
            color_space: "ICC".into(),
            icc_profile: done.image().icc().to_vec(),
            byte_size: composite.len() as u64,
            source: "ai_result".into(),
            captured_at_ms: 0,
            metadata_json: "{}".into(),
        },
    ];
    if let (Some(bytes), Some(id)) = (&acceptance, &acceptance_id) {
        assets.push(pb::AddAsset {
            asset_id: id.to_string(),
            format: "png".into(),
            width: done.image().width(),
            height: done.image().height(),
            orientation: 1,
            bit_depth: 8,
            has_alpha: false,
            color_space: "coverage".into(),
            icc_profile: vec![],
            byte_size: bytes.len() as u64,
            source: "ai_result".into(),
            captured_at_ms: 0,
            metadata_json: "{}".into(),
        });
    }
    let proof = MaterializedProof {
        schema: 1,
        materialization: "vw-ai-composite-v1".into(),
        source_asset_id: s.source.asset_id.clone(),
        composite_asset_id: composite_id.to_string(),
        acceptance_mask_asset_id: acceptance_id.as_ref().map(ToString::to_string),
        changed_outside: 0,
        proof: done.proof().clone(),
    };
    let proof_json = serde_json::to_string(&proof).map_err(|_| AiEditError::Proof)?;
    validate_materialized(
        &proof_json,
        &AssetId::try_from(s.source.asset_id.clone())?,
        &composite_id,
        acceptance_id.as_ref().map(AssetId::as_str),
    )?;
    // No key or ledger data enters the project. Request text is deliberate
    // project content; the bound config and estimate retain their provenance.
    let request_json = admission::metadata_json(
        &s.prepared,
        &s.source.asset_id,
        &s.configuration,
        s.options.estimate.as_ref().map(|e| e.provenance.as_str()),
        match s.settlement {
            AiSettlement::UsagePriced => "usage_priced",
            AiSettlement::UsageMissing => "usage_missing",
            AiSettlement::LedgerUnavailable => "ledger_unavailable",
        },
        done.actual_microusd(),
        match done.origin() {
            vw_ai::ResponseOrigin::ProviderPayload => "provider_payload",
            vw_ai::ResponseOrigin::OfflineMock => "offline_mock",
        },
    )?;
    let result = pb::AddResult {
        result_id: Some(Id::try_from(options.result_id.clone())?.to_proto()),
        document_id: Some(Id::try_from(options.expected.document_id.clone())?.to_proto()),
        provider: match done.origin() {
            vw_ai::ResponseOrigin::ProviderPayload => "openai".into(),
            vw_ai::ResponseOrigin::OfflineMock => "offline_mock".into(),
        },
        model: s.prepared.review().model().into(),
        request_json,
        output_asset_id: output_id.to_string(),
        composite_asset_id: composite_id.to_string(),
        proof_json,
        metrics_json: serde_json::to_string(&done.proof().metrics)
            .map_err(|_| AiEditError::Proof)?,
        cost_estimate_usd: s
            .prepared
            .review()
            .estimated_microusd()
            .ok_or(AiEditError::Estimate)? as f64
            / 1_000_000.0,
        package_id: None,
    };
    let payload = Arc::new(SavePayload {
        options,
        raw: s.raw.clone(),
        composite,
        acceptance,
        assets,
        result,
        proof: request::proof(done.proof()),
        budget: s.options.memory_budget_bytes,
        resident,
        transaction: Mutex::new(None),
    });
    stop.check()?;
    s.save = Some(payload.clone());
    Ok(payload)
}
pub(super) fn asset(image: &vw_raster::DecodedImage, id: &AssetId, length: u64) -> pb::AddAsset {
    let (width, height) = if image.orientation_applied >= 5 {
        (image.height, image.width)
    } else {
        (image.width, image.height)
    };
    pb::AddAsset {
        asset_id: id.to_string(),
        format: "png".into(),
        width,
        height,
        orientation: u32::from(image.orientation_applied),
        bit_depth: u32::from(image.pixels.bit_depth()),
        has_alpha: image.pixels.has_alpha(),
        color_space: if image.icc.is_some() {
            "ICC".into()
        } else {
            "sRGB".into()
        },
        icc_profile: image.icc.clone().unwrap_or_default(),
        byte_size: length,
        source: "ai_result".into(),
        captured_at_ms: 0,
        metadata_json: "{}".into(),
    }
}

fn transaction(
    state: &ProjectState,
    expected: &WorkflowBinding,
    meta: &WorkflowMetadata,
    kinds: Vec<pb::op::Kind>,
) -> AiEditResult<pb::Transaction> {
    let ops = kinds
        .into_iter()
        .enumerate()
        .map(|(i, kind)| {
            Ok(pb::Op {
                op_id: Some(pb::OpId {
                    device_id: meta.device_id.clone(),
                    lamport: meta
                        .first_lamport
                        .checked_add(i as u64)
                        .ok_or(AiEditError::Limit)?,
                }),
                kind: Some(kind),
            })
        })
        .collect::<AiEditResult<Vec<_>>>()?;
    Ok(pb::Transaction {
        txn_id: Some(Id::try_from(meta.transaction_id.clone())?.to_proto()),
        project_id: Some(Id::try_from(expected.project_id.clone())?.to_proto()),
        device_id: meta.device_id.clone(),
        base_revision: Some(state.store()?.revision()?),
        created_at_wall_ms: meta.created_at_ms,
        gesture_id: None,
        ops,
    })
}
pub(super) fn save(
    state: &mut ProjectState,
    p: &SavePayload,
    cancel: &crate::Cancellation,
    stop: &request::Stop,
) -> AiEditResult<AiResultReceipt> {
    stop.check()?;
    let options = &p.options;
    let device = DeviceId::try_from(options.metadata.device_id.clone())?;
    let output = AssetId::hash(p.raw.as_slice());
    let composite = AssetId::hash(&p.composite);
    let acceptance = p.acceptance.as_deref().map(AssetId::hash);
    let extra =
        p.resident + p.composite.len() as u64 + p.acceptance.as_ref().map_or(0, |p| p.len() as u64);
    memory(extra, p.budget)?;
    let mut retained = p.transaction.lock().map_err(|_| AiEditError::Closed)?;
    if retained.is_none() {
        crate::workflow::admit(
            state,
            (p.budget - extra).min(crate::workflow::MAX_MEMORY),
            true,
            0,
        )?;
        crate::workflow::check_binding(state, &options.expected)?;
        crate::workflow::check_metadata(state, &options.metadata)?;
        let project = state.project()?;
        let doc = Id::try_from(options.expected.document_id.clone())?;
        let layer = Id::try_from(options.layer_id.clone())?;
        let object = Id::try_from(options.object_id.clone())?;
        let result_id = Id::try_from(options.result_id.clone())?;
        if project.results.contains_key(&result_id)
            || project.objects.contains_key(&object)
            || project.groups.contains_key(&object)
        {
            return Err(AiEditError::Stale);
        }
        let original = &project
            .documents
            .get(&doc)
            .ok_or(AiEditError::Stale)?
            .definition
            .primary_asset_id;
        validate_materialized(
            &p.result.proof_json,
            &AssetId::try_from(original.clone())?,
            &composite,
            acceptance.as_ref().map(AssetId::as_str),
        )?;
        let occupied = project
            .objects
            .iter()
            .filter(|(_, o)| o.state.layer_id == Some(layer.to_proto()) && !o.state.hidden)
            .collect::<Vec<_>>();
        if occupied
            .iter()
            .any(|(id, _)| options.replace_object_id.as_deref() != Some(id.as_str()))
        {
            return Err(AiEditError::Invalid);
        }
        let mut kinds = Vec::new();
        for added in &p.assets {
            let id = AssetId::try_from(added.asset_id.clone())?;
            if let Some(old) = project.assets.get(&id) {
                if old.width != added.width
                    || old.height != added.height
                    || old.bit_depth != added.bit_depth
                    || old.orientation != added.orientation
                    || old.icc_profile != added.icc_profile
                    || old.byte_size != added.byte_size
                {
                    return Err(AiEditError::Proof);
                }
            } else {
                kinds.push(pb::op::Kind::AddAsset(added.clone()));
            }
        }
        if let Some(existing) = project.layers.get(&layer) {
            if existing.locked
                || !existing.visible
                || existing.definition.kind != "result"
                || existing.definition.document_id != Some(doc.to_proto())
                || existing.blend != "normal"
                || existing.opacity != 1.0
            {
                return Err(AiEditError::Invalid);
            }
        } else {
            let left = project
                .layers
                .values()
                .filter(|l| l.definition.document_id == Some(doc.to_proto()))
                .map(|l| l.definition.order_key.as_str())
                .max()
                .map(|s| OrderKey::try_from(s.to_owned()))
                .transpose()?;
            kinds.push(pb::op::Kind::CreateLayer(pb::CreateLayer {
                layer_id: Some(layer.to_proto()),
                document_id: Some(doc.to_proto()),
                page_index: -1,
                name: "AI Result".into(),
                kind: "result".into(),
                order_key: OrderKey::between(left.as_ref(), None)?.as_str().into(),
            }));
        }
        if let Some(replace) = &options.replace_object_id {
            let id = Id::try_from(replace.clone())?;
            let prior = project.objects.get(&id).ok_or(AiEditError::Stale)?;
            let prior_layer = project
                .layers
                .get(&Id::from_proto(prior.state.layer_id.as_ref())?)
                .ok_or(AiEditError::Invalid)?;
            if prior.document_id != doc
                || prior.state.locked
                || prior_layer.locked
                || !matches!(
                    prior.state.shape,
                    Some(pb::object_state::Shape::ResultId(_))
                )
            {
                return Err(AiEditError::Invalid);
            }
            kinds.push(pb::op::Kind::DeleteObject(pb::DeleteObject {
                object_id: Some(id.to_proto()),
            }));
        }
        kinds.push(pb::op::Kind::AddResult(p.result.clone()));
        kinds.push(pb::op::Kind::UpdateResult(pb::UpdateResult {
            result_id: Some(result_id.to_proto()),
            status: if acceptance.is_some() {
                "partial".into()
            } else {
                "ready".into()
            },
            acceptance_mask_asset_id: acceptance
                .as_ref()
                .map_or_else(String::new, ToString::to_string),
        }));
        let left = project
            .objects
            .values()
            .filter(|o| o.state.layer_id == Some(layer.to_proto()))
            .map(|o| o.state.order_key.as_str())
            .max()
            .map(|s| OrderKey::try_from(s.to_owned()))
            .transpose()?;
        kinds.push(pb::op::Kind::CreateObject(pb::CreateObject {
            document_id: Some(doc.to_proto()),
            state: Some(pb::ObjectState {
                object_id: Some(object.to_proto()),
                layer_id: Some(layer.to_proto()),
                order_key: OrderKey::between(left.as_ref(), None)?.as_str().into(),
                transform: Some(pb::Affine {
                    a: 1.0,
                    d: 1.0,
                    ..Default::default()
                }),
                style: Some(pb::Style {
                    stroke: Some(pb::Color { rgba: 0 }),
                    width: 0.0,
                    screen_constant_width: false,
                    fill: None,
                    has_fill: false,
                }),
                role: pb::Role::None as i32,
                created_by: device.to_string(),
                created_at_ms: options.metadata.created_at_ms,
                shape: Some(pb::object_state::Shape::ResultId(result_id.to_proto())),
                ..Default::default()
            }),
        }));
        *retained = Some(transaction(
            state,
            &options.expected,
            &options.metadata,
            kinds,
        )?);
    }
    let txn = retained.as_ref().ok_or(AiEditError::Invalid)?;
    crate::workflow::admit_transaction(
        state,
        txn,
        (p.budget - extra).min(crate::workflow::MAX_MEMORY),
        0,
    )?;
    let duplicate =
        if let Some(previous) = state.previous(txn.txn_id.as_ref().ok_or(AiEditError::Invalid)?)? {
            if previous != txn {
                return Err(AiEditError::Stale);
            }
            true
        } else {
            crate::workflow::check_binding(state, &options.expected)?;
            false
        };
    if !duplicate {
        if let Some(replica) = &state.replica {
            let mut trial = replica.clone();
            trial.queue(txn.clone()).map_err(|_| AiEditError::Invalid)?;
        } else {
            state
                .store()?
                .validate_transaction(txn, &device, options.metadata.created_at_ms)
                .map_err(|_| AiEditError::Invalid)?;
        }
        stop.check()?;
        if state.blobs.put(p.raw.as_slice())? != output
            || state.blobs.put(&p.composite)? != composite
        {
            return Err(AiEditError::Proof);
        }
        if let (Some(bytes), Some(id)) = (&p.acceptance, &acceptance)
            && state.blobs.put(bytes)? != *id
        {
            return Err(AiEditError::Proof);
        }
        cancel.check()?;
        stop.check()?;
    }
    let revision = if duplicate {
        state.info()?
    } else {
        state.commit(txn, &device, options.metadata.created_at_ms)?
    };
    Ok(AiResultReceipt {
        transaction_id: options.metadata.transaction_id.clone(),
        result_id: options.result_id.clone(),
        object_id: options.object_id.clone(),
        layer_id: options.layer_id.clone(),
        output_asset_id: output.to_string(),
        composite_asset_id: composite.to_string(),
        acceptance_mask_asset_id: acceptance.map(|a| a.to_string()),
        revision,
        proof: p.proof.clone(),
    })
}
#[uniffi::export]
impl ProjectSession {
    pub async fn ai_result_status(
        &self,
        binding: WorkflowBinding,
        result_id: String,
        accepted: bool,
        metadata: WorkflowMetadata,
        cancellation: Arc<crate::Cancellation>,
    ) -> AiEditResult<WorkflowReceipt> {
        self.check_open()?;
        binding.validate()?;
        metadata.validate()?;
        let result = Id::try_from(result_id)?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check(&closed, &cancellation)?;
                    crate::workflow::canonical_workspace(state, crate::workflow::MAX_MEMORY, true)?;
                    crate::workflow::check_binding(state, &binding)?;
                    crate::workflow::check_metadata(state, &metadata)?;
                    let project = state.project()?;
                    let value = project.results.get(&result).ok_or(AiEditError::Invalid)?;
                    if value.definition.document_id
                        != Some(Id::try_from(binding.document_id.clone())?.to_proto())
                    {
                        return Err(AiEditError::Invalid);
                    }
                    let mut kinds = vec![pb::op::Kind::UpdateResult(pb::UpdateResult {
                        result_id: Some(result.to_proto()),
                        status: if accepted {
                            "accepted".into()
                        } else {
                            "rejected".into()
                        },
                        acceptance_mask_asset_id: value
                            .acceptance_mask_asset_id
                            .as_ref()
                            .map_or_else(String::new, ToString::to_string),
                    })];
                    for (id, object) in &project.objects {
                        if object.state.shape
                            == Some(pb::object_state::Shape::ResultId(result.to_proto()))
                        {
                            let layer = project
                                .layers
                                .get(&Id::from_proto(object.state.layer_id.as_ref())?)
                                .ok_or(AiEditError::Invalid)?;
                            if object.state.locked || layer.locked {
                                return Err(AiEditError::Invalid);
                            }
                            kinds.push(pb::op::Kind::SetProperty(pb::SetProperty {
                                object_id: Some(id.to_proto()),
                                property: "hidden".into(),
                                value: Some(pb::PropertyValue {
                                    value: Some(pb::property_value::Value::Flag(!accepted)),
                                }),
                            }));
                        }
                    }
                    if kinds.len() > 512 {
                        return Err(AiEditError::Limit);
                    }
                    let txn = transaction(state, &binding, &metadata, kinds)?;
                    crate::workflow::admit_transaction(
                        state,
                        &txn,
                        crate::workflow::MAX_MEMORY,
                        0,
                    )?;
                    check(&closed, &cancellation)?;
                    let revision = state.commit(
                        &txn,
                        &DeviceId::try_from(metadata.device_id.clone())?,
                        metadata.created_at_ms,
                    )?;
                    Ok(WorkflowReceipt {
                        transaction_id: metadata.transaction_id,
                        revision,
                        duplicate: false,
                    })
                })())
            })
            .await?
    }
}
