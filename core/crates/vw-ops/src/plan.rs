use crate::{OpsError, draft::Draft};
use vw_model::{
    AssetId, Document, Group, Id, Instruction, Layer, Object, Project, ResultCandidate,
    SemanticSnapshot, validate_asset, validate_object_state,
};
use vw_proto::v1::{self, op::Kind, property_value::Value};

/// Apply a transaction's operations to a draft in order. The caller validates
/// the edited project and discards the draft when anything here fails.
pub(crate) fn apply(draft: &mut Draft<'_>, transaction: &v1::Transaction) -> Result<(), OpsError> {
    for op in &transaction.ops {
        let kind = op
            .kind
            .as_ref()
            .ok_or(OpsError::Invalid("operation kind"))?;
        if let Kind::CreateObject(value) = kind
            && value
                .state
                .as_ref()
                .is_some_and(|state| state.created_by != transaction.device_id)
        {
            return Err(OpsError::DeviceMismatch);
        }
        apply_kind(draft, kind, transaction.created_at_wall_ms)?;
    }
    Ok(())
}

fn apply_kind(draft: &mut Draft<'_>, kind: &Kind, time: i64) -> Result<(), OpsError> {
    match kind {
        Kind::CreateDocument(value) => {
            let id = Id::from_proto(value.document_id.as_ref())?;
            if draft.project().documents.contains_key(&id) {
                return Err(OpsError::AlreadyExists);
            }
            draft.insert_document(
                id,
                Document {
                    definition: value.clone(),
                    pages: Vec::new(),
                    created_at_ms: time,
                },
            );
        }
        Kind::UpdateDocument(value) => {
            let id = Id::from_proto(value.document_id.as_ref())?;
            draft
                .document_mut(&id)
                .ok_or(OpsError::NotFound)?
                .definition
                .title = value.title.clone();
        }
        Kind::AddAsset(value) => {
            validate_asset(value)?;
            let id = AssetId::try_from(value.asset_id.clone())?;
            if let Some(old) = draft.project().assets.get(&id) {
                if old != value {
                    return Err(OpsError::AlreadyExists);
                }
            } else {
                draft.insert_asset(id, value.clone());
            }
        }
        Kind::CreateLayer(value) => {
            let id = Id::from_proto(value.layer_id.as_ref())?;
            if draft.project().layers.contains_key(&id) {
                return Err(OpsError::AlreadyExists);
            }
            draft.insert_layer(
                id,
                Layer {
                    definition: value.clone(),
                    visible: true,
                    locked: false,
                    opacity: 1.0,
                    blend: "normal".into(),
                },
            );
        }
        Kind::UpdateLayer(value) => {
            let id = Id::from_proto(value.layer_id.as_ref())?;
            let layer = draft.layer_mut(&id).ok_or(OpsError::NotFound)?;
            if layer.locked
                && (value.name.is_some()
                    || value.order_key.is_some()
                    || value.visible.is_some()
                    || value.opacity.is_some()
                    || value.blend.is_some())
            {
                return Err(OpsError::Locked);
            }
            if let Some(v) = &value.name {
                layer.definition.name = v.clone();
            }
            if let Some(v) = &value.order_key {
                layer.definition.order_key = v.clone();
            }
            if let Some(v) = value.visible {
                layer.visible = v;
            }
            if let Some(v) = value.locked {
                layer.locked = v;
            }
            if let Some(v) = value.opacity {
                layer.opacity = f64::from(v);
            }
            if let Some(v) = &value.blend {
                layer.blend = v.clone();
            }
        }
        Kind::DeleteLayer(value) => {
            let id = Id::from_proto(value.layer_id.as_ref())?;
            let project = draft.project();
            let layer = project.layers.get(&id).ok_or(OpsError::NotFound)?;
            if layer.locked
                || project
                    .objects
                    .values()
                    .any(|o| o.state.layer_id == value.layer_id && o.state.locked)
            {
                return Err(OpsError::Locked);
            }
            let members = project
                .objects
                .iter()
                .filter(|(_, o)| o.state.layer_id == value.layer_id)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            for member in &members {
                draft.remove_object(member);
            }
            draft.remove_layer(&id);
        }
        Kind::CreateObject(value) => {
            let state = value
                .state
                .clone()
                .ok_or(OpsError::Invalid("object state"))?;
            validate_object_state(&state)?;
            let id = Id::from_proto(state.object_id.as_ref())?;
            let project = draft.project();
            if project.objects.contains_key(&id) || project.groups.contains_key(&id) {
                return Err(OpsError::AlreadyExists);
            }
            let layer = project
                .layers
                .get(&Id::from_proto(state.layer_id.as_ref())?)
                .ok_or(OpsError::NotFound)?;
            if layer.locked {
                return Err(OpsError::Locked);
            }
            draft.insert_object(
                id,
                Object {
                    document_id: Id::from_proto(value.document_id.as_ref())?,
                    state,
                },
            );
        }
        Kind::SetProperty(value) => {
            let id = Id::from_proto(value.object_id.as_ref())?;
            ensure_editable(draft.project(), &id, value.property == "locked")?;
            let object = draft.object_mut(&id).ok_or(OpsError::NotFound)?;
            set_property(&mut object.state, value)?;
        }
        Kind::DeleteObject(value) => {
            let id = Id::from_proto(value.object_id.as_ref())?;
            ensure_editable(draft.project(), &id, false)?;
            draft.remove_object(&id);
        }
        Kind::Reorder(value) => {
            let id = Id::from_proto(value.object_id.as_ref())?;
            ensure_editable(draft.project(), &id, false)?;
            let destination = draft
                .project()
                .layers
                .get(&Id::from_proto(value.layer_id.as_ref())?)
                .ok_or(OpsError::NotFound)?;
            if destination.locked {
                return Err(OpsError::Locked);
            }
            let object = draft.object_mut(&id).ok_or(OpsError::NotFound)?;
            object.state.order_key = value.order_key.clone();
            object.state.layer_id = value.layer_id.clone();
        }
        Kind::Group(value) => {
            let id = Id::from_proto(value.group_id.as_ref())?;
            if draft.project().objects.contains_key(&id) || value.object_ids.is_empty() {
                return Err(OpsError::Invalid("group members"));
            }
            let members = value
                .object_ids
                .iter()
                .map(|v| Id::from_proto(Some(v)))
                .collect::<Result<Vec<_>, _>>()?;
            let first = members.first().ok_or(OpsError::Invalid("group members"))?;
            let project = draft.project();
            let document = if let Some(object) = project.objects.get(first) {
                object.document_id.clone()
            } else {
                project
                    .groups
                    .get(first)
                    .ok_or(OpsError::NotFound)?
                    .document_id
                    .clone()
            };
            if let Some(existing) = project.groups.get(&id) {
                if existing.document_id != document {
                    return Err(OpsError::Invalid("group document"));
                }
            } else {
                draft.insert_group(
                    id.clone(),
                    Group {
                        id: id.clone(),
                        document_id: document,
                        parent: None,
                    },
                );
            }
            for member in members {
                if draft.project().objects.contains_key(&member) {
                    ensure_editable(draft.project(), &member, false)?;
                    draft
                        .object_mut(&member)
                        .ok_or(OpsError::NotFound)?
                        .state
                        .group_id = Some(id.to_proto());
                } else {
                    ensure_group_editable(draft.project(), &member)?;
                    draft.group_mut(&member).ok_or(OpsError::NotFound)?.parent = Some(id.clone());
                }
            }
        }
        Kind::Ungroup(value) => {
            let id = Id::from_proto(value.group_id.as_ref())?;
            ensure_group_editable(draft.project(), &id)?;
            let project = draft.project();
            let parent = project
                .groups
                .get(&id)
                .ok_or(OpsError::NotFound)?
                .parent
                .clone();
            let member_ids = project
                .objects
                .iter()
                .filter(|(_, o)| o.state.group_id == value.group_id)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            let child_ids = project
                .groups
                .iter()
                .filter(|(_, group)| group.parent.as_ref() == Some(&id))
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            for member in member_ids {
                ensure_editable(draft.project(), &member, false)?;
                draft
                    .object_mut(&member)
                    .ok_or(OpsError::NotFound)?
                    .state
                    .group_id = parent.as_ref().map(Id::to_proto);
            }
            for child in child_ids {
                draft.group_mut(&child).ok_or(OpsError::NotFound)?.parent = parent.clone();
            }
            draft.remove_group(&id);
        }
        Kind::SetInstruction(value) => {
            let document = Id::from_proto(value.document_id.as_ref())?;
            for target in &value.target_object_ids {
                if draft
                    .project()
                    .objects
                    .get(&Id::from_proto(Some(target))?)
                    .ok_or(OpsError::NotFound)?
                    .document_id
                    != document
                {
                    return Err(OpsError::Invalid("instruction target document"));
                }
            }
            let id = Id::from_proto(value.instruction_id.as_ref())?;
            draft.insert_instruction(
                id,
                Instruction {
                    definition: value.clone(),
                    updated_at_ms: time,
                },
            );
        }
        Kind::DeleteInstruction(value) => {
            if !draft.remove_instruction(&Id::from_proto(value.instruction_id.as_ref())?) {
                return Err(OpsError::NotFound);
            }
        }
        Kind::AddSemanticSnapshot(value) => {
            let id = Id::from_proto(value.snapshot_id.as_ref())?;
            let project = draft.project();
            if project.semantic_snapshots.contains_key(&id) {
                return Err(OpsError::AlreadyExists);
            }
            let document = project
                .documents
                .get(&Id::from_proto(value.document_id.as_ref())?)
                .ok_or(OpsError::NotFound)?;
            let capture = document.definition.capture.as_ref();
            let capture_session_id = capture
                .map(|c| Id::from_proto(c.capture_session_id.as_ref()))
                .transpose()?;
            let frame_id = capture.map(|c| c.frame_id);
            draft.insert_semantic_snapshot(
                id,
                SemanticSnapshot {
                    definition: value.clone(),
                    capture_session_id,
                    frame_id,
                    created_at_ms: time,
                },
            );
        }
        Kind::AddResult(value) => {
            let id = Id::from_proto(value.result_id.as_ref())?;
            if draft.project().results.contains_key(&id) {
                return Err(OpsError::AlreadyExists);
            }
            draft.insert_result(
                id,
                ResultCandidate {
                    definition: value.clone(),
                    status: "pending".into(),
                    acceptance_mask_asset_id: None,
                    created_at_ms: time,
                },
            );
        }
        Kind::UpdateResult(value) => {
            let id = Id::from_proto(value.result_id.as_ref())?;
            let result = draft.result_mut(&id).ok_or(OpsError::NotFound)?;
            result.status = value.status.clone();
            result.acceptance_mask_asset_id = if value.acceptance_mask_asset_id.is_empty() {
                None
            } else {
                Some(AssetId::try_from(value.acceptance_mask_asset_id.clone())?)
            };
        }
        Kind::MaskOp(value) => {
            let id = Id::from_proto(value.output_object_id.as_ref())?;
            ensure_editable(draft.project(), &id, false)?;
            let project = draft.project();
            let output = project.objects.get(&id).ok_or(OpsError::NotFound)?;
            for input in &value.input_object_ids {
                let input = project
                    .objects
                    .get(&Id::from_proto(Some(input))?)
                    .ok_or(OpsError::NotFound)?;
                if input.document_id != output.document_id || !is_mask(&input.state) {
                    return Err(OpsError::Invalid("mask input"));
                }
            }
            if !is_mask(&output.state) {
                return Err(OpsError::Invalid("mask output"));
            }
            match draft.mask_versions_mut(&id) {
                Some(versions) => versions.push(value.clone()),
                None => draft.insert_mask_versions(id, vec![value.clone()]),
            }
        }
        Kind::UndoTransaction(_) => {
            return Err(OpsError::Invalid("undo must be a standalone transaction"));
        }
    }
    Ok(())
}

fn is_mask(state: &v1::ObjectState) -> bool {
    matches!(
        state.shape,
        Some(
            v1::object_state::Shape::SelectionRaster(_)
                | v1::object_state::Shape::SelectionVector(_)
        )
    )
}

// Changing a group's parent moves every descendant, including nested groups.
// Check all descendant objects so nesting cannot bypass layer or object locks.
fn ensure_group_editable(project: &Project, id: &Id) -> Result<(), OpsError> {
    if !project.groups.contains_key(id) {
        return Err(OpsError::NotFound);
    }
    for (object_id, object) in &project.objects {
        let mut parent = object
            .state
            .group_id
            .as_ref()
            .map(|value| Id::from_proto(Some(value)))
            .transpose()?;
        let mut seen = std::collections::BTreeSet::new();
        while let Some(group_id) = parent {
            if !seen.insert(group_id.clone()) {
                return Err(OpsError::Invalid("group cycle"));
            }
            if &group_id == id {
                ensure_editable(project, object_id, false)?;
                break;
            }
            parent = project
                .groups
                .get(&group_id)
                .ok_or(OpsError::NotFound)?
                .parent
                .clone();
        }
    }
    Ok(())
}

fn ensure_editable(project: &Project, id: &Id, unlocking: bool) -> Result<(), OpsError> {
    let object = project.objects.get(id).ok_or(OpsError::NotFound)?;
    let layer = project
        .layers
        .get(&Id::from_proto(object.state.layer_id.as_ref())?)
        .ok_or(OpsError::NotFound)?;
    if layer.locked || (object.state.locked && !unlocking) {
        return Err(OpsError::Locked);
    }
    Ok(())
}

pub(crate) fn set_property(
    state: &mut v1::ObjectState,
    property: &v1::SetProperty,
) -> Result<(), OpsError> {
    let value = property
        .value
        .as_ref()
        .and_then(|v| v.value.as_ref())
        .ok_or(OpsError::Invalid("property value"))?;
    match (property.property.as_str(), value) {
        ("transform", Value::Transform(v)) => state.transform = Some(*v),
        ("style", Value::Style(v)) => state.style = Some(*v),
        ("role", Value::Role(v)) => state.role = *v,
        ("locked", Value::Flag(v)) => state.locked = *v,
        ("hidden", Value::Flag(v)) => state.hidden = *v,
        ("order_key", Value::Text(v)) => state.order_key = v.clone(),
        ("shape", Value::Raw(v)) if v.len() <= 16 * 1024 * 1024 => {
            state.shape = Some(serde_json::from_slice(v)?)
        }
        ("style.width", Value::Number(v)) => {
            state
                .style
                .as_mut()
                .ok_or(OpsError::Invalid("style"))?
                .width = *v
        }
        ("style.stroke", Value::Color(v)) => {
            state
                .style
                .as_mut()
                .ok_or(OpsError::Invalid("style"))?
                .stroke = Some(*v)
        }
        ("style.fill", Value::Color(v)) => {
            let style = state.style.as_mut().ok_or(OpsError::Invalid("style"))?;
            style.fill = Some(*v);
            style.has_fill = true;
        }
        ("style.has_fill", Value::Flag(v)) => {
            state
                .style
                .as_mut()
                .ok_or(OpsError::Invalid("style"))?
                .has_fill = *v
        }
        ("style.screen_constant_width", Value::Flag(v)) => {
            state
                .style
                .as_mut()
                .ok_or(OpsError::Invalid("style"))?
                .screen_constant_width = *v
        }
        ("text.text", Value::Text(v)) => text_shape(state)?.text = v.clone(),
        ("text.font_family", Value::Text(v)) => text_shape(state)?.font_family = v.clone(),
        ("text.font_size", Value::Number(v)) => text_shape(state)?.font_size = *v,
        ("marker.number", Value::Integer(v)) => {
            let Some(v1::object_state::Shape::Marker(marker)) = state.shape.as_mut() else {
                return Err(OpsError::Invalid("marker shape"));
            };
            marker.number = u32::try_from(*v).map_err(|_| OpsError::Invalid("marker number"))?;
        }
        _ => return Err(OpsError::Invalid("property name/type")),
    }
    validate_object_state(state)?;
    Ok(())
}

fn text_shape(state: &mut v1::ObjectState) -> Result<&mut v1::TextObject, OpsError> {
    match state.shape.as_mut() {
        Some(v1::object_state::Shape::Text(value)) => Ok(value),
        _ => Err(OpsError::Invalid("text shape")),
    }
}
