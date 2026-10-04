use super::*;
use vw_model::{DeviceId, OrderKey};

pub(super) fn admit_options(options: &SelectionEdit) -> SelectionResult<()> {
    budget(options.memory_budget_bytes)?;
    admit_binding(&options.binding)?;
    short_id(&options.transaction_id)?;
    if options.device_id.len() > 128 {
        return Err(SelectionError::Invalid);
    }
    if options.lamport == 0 || options.created_at_ms < 0 {
        return Err(SelectionError::Invalid);
    }
    Id::try_from(options.transaction_id.clone())?;
    DeviceId::try_from(options.device_id.clone())?;
    match &options.target {
        SelectionTarget::New {
            object_id,
            layer_id,
        } => {
            short_id(object_id)?;
            short_id(layer_id)?;
        }
        SelectionTarget::Existing { selection } => admit_version(selection)?,
    }
    if let SelectionOperation::Combine { other, .. } = &options.operation {
        admit_version(other)?;
    }
    match &options.operation {
        SelectionOperation::Lasso { points, .. }
            if points.len() < 3 || points.len() > vw_mask::MAX_PATH_POINTS =>
        {
            Err(SelectionError::Limit)
        }
        SelectionOperation::Paint { points, .. }
            if points.is_empty() || points.len() > vw_mask::MAX_PATH_POINTS =>
        {
            Err(SelectionError::Limit)
        }
        _ => Ok(()),
    }
}

pub(super) fn apply(
    state: &mut ProjectState,
    options: SelectionEdit,
    cancel: &Cancellation,
) -> SelectionResult<SelectionReceipt> {
    let canonical = canonical_workspace(state, options.memory_budget_bytes, true)?;
    let doc = check_binding(state, &options.binding)?;
    let size = Size::new(doc.width, doc.height)?;
    let id = Id::try_from(options.transaction_id.clone())?.to_proto();
    if state.previous(&id)?.is_some() {
        return Err(SelectionError::ReusedTransaction);
    }
    let device = DeviceId::try_from(options.device_id.clone())?;
    if device != state.store()?.local_device()? {
        return Err(SelectionError::Invalid);
    }
    let input_bytes = match &options.operation {
        SelectionOperation::Lasso { points, .. } | SelectionOperation::Paint { points, .. } => {
            points.len() as u64 * 256
        }
        _ => 0,
    };
    memory(
        mask_estimate(
            size,
            input_bytes
                .checked_add(canonical)
                .ok_or(SelectionError::Limit)?,
        )?,
        options.memory_budget_bytes,
    )?;
    if let SelectionOperation::Expand { radius }
    | SelectionOperation::Shrink { radius }
    | SelectionOperation::Feather { radius } = &options.operation
    {
        admit_morphology_size(size, *radius)?;
    }

    let project = state.project()?;
    let document_id = Id::try_from(options.binding.document_id.clone())?;
    let mut kinds = Vec::new();
    let mut inputs = Vec::new();
    let (object_id, layer_id, prior_version, editable, visible, base, create) = match &options
        .target
    {
        SelectionTarget::Existing { selection } => {
            let (layer, editable, visible) =
                check_version(project, &options.binding, selection, size)?;
            if !editable {
                return Err(SelectionError::Locked);
            }
            let id = Id::try_from(selection.object_id.clone())?;
            inputs.push(id.to_proto());
            (
                id,
                Id::try_from(layer)?,
                selection.version,
                editable,
                visible,
                load_mask(state, selection, size, cancel)?,
                false,
            )
        }
        SelectionTarget::New {
            object_id,
            layer_id,
        } => {
            let object = Id::try_from(object_id.clone())?;
            let layer = Id::try_from(layer_id.clone())?;
            if project.objects.contains_key(&object)
                || project.groups.contains_key(&object)
                || project.mask_versions.contains_key(&object)
            {
                return Err(SelectionError::Conflict);
            }
            let visible = if let Some(existing) = project.layers.get(&layer) {
                if existing.definition.document_id != Some(document_id.to_proto())
                    || existing.definition.kind != "mask"
                    || existing.definition.page_index != -1
                {
                    return Err(SelectionError::Unsupported);
                }
                if existing.locked {
                    return Err(SelectionError::Locked);
                }
                existing.visible
            } else {
                let left = project
                    .layers
                    .values()
                    .filter(|value| value.definition.document_id == Some(document_id.to_proto()))
                    .map(|value| value.definition.order_key.as_str())
                    .max()
                    .map(|value| OrderKey::try_from(value.to_owned()))
                    .transpose()?;
                kinds.push(pb::op::Kind::CreateLayer(pb::CreateLayer {
                    layer_id: Some(layer.to_proto()),
                    document_id: Some(document_id.to_proto()),
                    page_index: -1,
                    name: "Selection".into(),
                    kind: "mask".into(),
                    order_key: OrderKey::between(left.as_ref(), None)?.as_str().into(),
                }));
                true
            };
            (object, layer, 0, true, visible, Mask::empty(size), true)
        }
    };
    let version = prior_version.checked_add(1).ok_or(SelectionError::Limit)?;
    cancel.check()?;
    let (mask, operation, amount) = apply_math(state, &options, &base, &mut inputs, cancel)?;
    drop(base);
    cancel.check()?;
    let encoded = mask.encode_mask_png(Region::full(size))?;
    if encoded.len() as u64 > png_limit(size)? {
        return Err(SelectionError::EncodedLimit);
    }
    let asset = AssetId::hash(&encoded);
    // A preexisting imported grayscale original can have these exact PNG bytes.
    // Its immutable content address must retain the existing asset metadata;
    // coverage semantics belong to SelectionRaster, not a conflicting relabel.
    let metadata = if let Some(existing) = project.assets.get(&asset) {
        if existing.format != "png"
            || existing.width != size.width()
            || existing.height != size.height()
            || existing.orientation != 1
            || existing.bit_depth != 8
            || existing.has_alpha
            || !existing.icc_profile.is_empty()
            || existing.byte_size != encoded.len() as u64
        {
            return Err(SelectionError::Corrupt);
        }
        existing.clone()
    } else {
        mask_asset(&mask, &asset, encoded.len() as u64)
    };
    kinds.push(pb::op::Kind::AddAsset(metadata));
    let shape = pb::object_state::Shape::SelectionRaster(pb::RasterMaskRef {
        mask_asset_id: asset.to_string(),
        bounds: Some(pb::RectD {
            x: 0.0,
            y: 0.0,
            w: f64::from(size.width()),
            h: f64::from(size.height()),
        }),
        // Coverage already contains all feathering; consumers must not apply it twice.
        feather: 0.0,
    });
    if create {
        let left = project
            .objects
            .values()
            .filter(|value| value.state.layer_id == Some(layer_id.to_proto()))
            .map(|value| value.state.order_key.as_str())
            .max()
            .map(|value| OrderKey::try_from(value.to_owned()))
            .transpose()?;
        let object = pb::ObjectState {
            object_id: Some(object_id.to_proto()),
            layer_id: Some(layer_id.to_proto()),
            order_key: OrderKey::between(left.as_ref(), None)?.as_str().into(),
            transform: Some(identity()),
            style: Some(pb::Style {
                stroke: Some(pb::Color { rgba: 0x2878ffff }),
                width: 1.0,
                screen_constant_width: true,
                fill: None,
                has_fill: false,
            }),
            role: pb::Role::None as i32,
            created_by: device.to_string(),
            created_at_ms: options.created_at_ms,
            shape: Some(shape),
            ..Default::default()
        };
        vw_model::validate_object_state(&object)?;
        kinds.push(pb::op::Kind::CreateObject(pb::CreateObject {
            document_id: Some(document_id.to_proto()),
            state: Some(object),
        }));
    } else {
        // This is the existing canonical "shape" property, not an invented
        // mask_asset_id property or a direct mutation outside the op journal.
        kinds.push(pb::op::Kind::SetProperty(pb::SetProperty {
            object_id: Some(object_id.to_proto()),
            property: "shape".into(),
            value: Some(pb::PropertyValue {
                value: Some(pb::property_value::Value::Raw(
                    serde_json::to_vec(&shape).map_err(|_| SelectionError::Invalid)?,
                )),
            }),
        }));
    }
    kinds.push(pb::op::Kind::MaskOp(pb::MaskOp {
        output_object_id: Some(object_id.to_proto()),
        op: operation.into(),
        input_object_ids: inputs,
        amount,
    }));
    let ops = kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            Ok(pb::Op {
                op_id: Some(pb::OpId {
                    device_id: device.to_string(),
                    lamport: options
                        .lamport
                        .checked_add(index as u64)
                        .ok_or(SelectionError::Limit)?,
                }),
                kind: Some(kind),
            })
        })
        .collect::<SelectionResult<Vec<_>>>()?;
    let txn = pb::Transaction {
        txn_id: Some(id),
        project_id: Some(project.id.to_proto()),
        device_id: device.to_string(),
        base_revision: Some(state.store()?.revision()?),
        created_at_wall_ms: options.created_at_ms,
        gesture_id: None,
        ops,
    };
    // Use the same authority as ProjectState::commit for a dry run before blob
    // admission, including optimistic pending objects on a connected client.
    if let Some(replica) = &state.replica {
        let mut candidate = replica.clone();
        // This is new work, not a recovered retry. Never return a prepared-mask
        // receipt for a transaction queued in the replica's blocked list.
        if txn.base_revision.as_ref() != Some(candidate.revision()) {
            return Err(SelectionError::Conflict);
        }
        candidate
            .queue(txn.clone())
            .map_err(|_| SelectionError::Invalid)?;
    } else {
        state
            .store()?
            .validate_transaction(&txn, &device, options.created_at_ms)
            .map_err(|_| SelectionError::Invalid)?;
    }
    cancel.check()?;
    let published = state.blobs.put(&encoded)?;
    if published != asset {
        return Err(SelectionError::Corrupt);
    }
    // Cancellation here may leave an unreferenced immutable blob. It never
    // changes the document or removes originals; normal blob GC may reclaim it.
    cancel.check()?;
    let revision = state.commit(&txn, &device, options.created_at_ms)?;
    // Commit is the point of no return. Do not turn an accepted transaction into
    // a Cancelled error after it has been journaled durably.
    let selection = SelectionVersion {
        object_id: object_id.to_string(),
        asset_id: asset.to_string(),
        version,
    };
    let result = snapshot(
        binding(
            &revision,
            &options.binding.document_id,
            &options.binding.source_asset_id,
        ),
        selection,
        layer_id.to_string(),
        editable,
        visible,
        &mask,
    );
    Ok(SelectionReceipt {
        transaction_id: options.transaction_id,
        snapshot: result,
        revision,
    })
}

fn admit_morphology_size(size: Size, radius: u32) -> SelectionResult<()> {
    // Empty masks skip morphology legitimately; they cannot admit an unknown
    // input before decode. A canonical nonempty one-pixel probe retains <= one
    // 256x256 tile, already covered by admitted CODEC_RESERVE. The library owns
    // the work/dense-buffer formula, so these checks cannot drift from it.
    let probe = Mask::rectangle(
        size,
        vw_mask::Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
    )?;
    probe.admit_morphology(radius)?;
    Ok(())
}

fn apply_math(
    state: &ProjectState,
    options: &SelectionEdit,
    base: &Mask,
    inputs: &mut Vec<pb::Uuid>,
    cancel: &Cancellation,
) -> SelectionResult<(Mask, &'static str, f64)> {
    let combine =
        |other: Mask, how: SelectionCombine| -> SelectionResult<(Mask, &'static str, f64)> {
            let (op, name) = match how {
                SelectionCombine::Add => (vw_mask::Combine::Add, "add"),
                SelectionCombine::Subtract => (vw_mask::Combine::Subtract, "subtract"),
                SelectionCombine::Intersect => (vw_mask::Combine::Intersect, "intersect"),
            };
            cancel.check()?;
            Ok((base.combine(&other, op)?, name, 0.0))
        };
    let points = |values: &[crate::Point]| {
        values
            .iter()
            .map(|p| vw_mask::Point { x: p.x, y: p.y })
            .collect::<Vec<_>>()
    };
    match &options.operation {
        SelectionOperation::Rectangle {
            rectangle: r,
            combine: how,
        } => combine(
            Mask::rectangle(
                base.size(),
                vw_mask::Rect {
                    x: r.x,
                    y: r.y,
                    width: r.width,
                    height: r.height,
                },
            )?,
            *how,
        ),
        SelectionOperation::Lasso {
            points: p,
            combine: how,
        } => combine(Mask::lasso(base.size(), &points(p))?, *how),
        SelectionOperation::Paint {
            points: p,
            radius,
            opacity,
            combine: how,
        } => combine(
            Mask::paint(base.size(), &points(p), *radius, *opacity)?,
            *how,
        ),
        SelectionOperation::Combine {
            other,
            combine: how,
        } => {
            check_version(state.project()?, &options.binding, other, base.size())?;
            let id = Id::try_from(other.object_id.clone())?.to_proto();
            if !inputs.contains(&id) {
                inputs.push(id);
            }
            combine(load_mask(state, other, base.size(), cancel)?, *how)
        }
        SelectionOperation::Invert => Ok((base.invert()?, "invert", 0.0)),
        SelectionOperation::Expand { radius } => {
            Ok((base.expand(*radius)?, "expand", f64::from(*radius)))
        }
        SelectionOperation::Shrink { radius } => {
            Ok((base.shrink(*radius)?, "shrink", f64::from(*radius)))
        }
        SelectionOperation::Feather { radius } => {
            Ok((base.feather(*radius)?, "feather", f64::from(*radius)))
        }
    }
}
