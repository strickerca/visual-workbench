use crate::OpsError;
use vw_model::{
    AssetId, Document, Group, Id, Instruction, Layer, Object, Project, ResultCandidate,
    SemanticSnapshot, validate_asset, validate_object_state,
};
use vw_proto::v1::{self, op::Kind, property_value::Value};

pub(crate) fn plan(project: &Project, transaction: &v1::Transaction) -> Result<Project, OpsError> {
    let mut next = project.clone();
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
        apply(&mut next, kind, transaction.created_at_wall_ms)?;
    }
    next.validate()?;
    Ok(next)
}

fn apply(project: &mut Project, kind: &Kind, time: i64) -> Result<(), OpsError> {
    match kind {
        Kind::CreateDocument(value) => {
            let id = Id::from_proto(value.document_id.as_ref())?;
            if project.documents.contains_key(&id) {
                return Err(OpsError::AlreadyExists);
            }
            project.documents.insert(
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
            project
                .documents
                .get_mut(&id)
                .ok_or(OpsError::NotFound)?
                .definition
                .title = value.title.clone();
        }
        Kind::AddAsset(value) => {
            validate_asset(value)?;
            let id = AssetId::try_from(value.asset_id.clone())?;
            if let Some(old) = project.assets.get(&id) {
                if old != value {
                    return Err(OpsError::AlreadyExists);
                }
            } else {
                project.assets.insert(id, value.clone());
            }
        }
        Kind::CreateLayer(value) => {
            let id = Id::from_proto(value.layer_id.as_ref())?;
            if project.layers.contains_key(&id) {
                return Err(OpsError::AlreadyExists);
            }
            project.layers.insert(
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
            let layer = project.layers.get_mut(&id).ok_or(OpsError::NotFound)?;
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
            let layer = project.layers.get(&id).ok_or(OpsError::NotFound)?;
            if layer.locked
                || project
                    .objects
                    .values()
                    .any(|o| o.state.layer_id == value.layer_id && o.state.locked)
            {
                return Err(OpsError::Locked);
            }
            project
                .objects
                .retain(|_, o| o.state.layer_id != value.layer_id);
            project.layers.remove(&id);
        }
        Kind::CreateObject(value) => {
            let state = value
                .state
                .clone()
                .ok_or(OpsError::Invalid("object state"))?;
            validate_object_state(&state)?;
            let id = Id::from_proto(state.object_id.as_ref())?;
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
            project.objects.insert(
                id,
                Object {
                    document_id: Id::from_proto(value.document_id.as_ref())?,
                    state,
                },
            );
        }
        Kind::SetProperty(value) => {
            let id = Id::from_proto(value.object_id.as_ref())?;
            ensure_editable(project, &id, value.property == "locked")?;
            let object = project.objects.get_mut(&id).ok_or(OpsError::NotFound)?;
            set_property(&mut object.state, value)?;
        }
        Kind::DeleteObject(value) => {
            let id = Id::from_proto(value.object_id.as_ref())?;
            ensure_editable(project, &id, false)?;
            project.objects.remove(&id);
        }
        Kind::Reorder(value) => {
            let id = Id::from_proto(value.object_id.as_ref())?;
            ensure_editable(project, &id, false)?;
            let destination = project
                .layers
                .get(&Id::from_proto(value.layer_id.as_ref())?)
                .ok_or(OpsError::NotFound)?;
            if destination.locked {
                return Err(OpsError::Locked);
            }
            let object = project.objects.get_mut(&id).ok_or(OpsError::NotFound)?;
            object.state.order_key = value.order_key.clone();
            object.state.layer_id = value.layer_id.clone();
        }
        Kind::Group(value) => {
            let id = Id::from_proto(value.group_id.as_ref())?;
            if project.objects.contains_key(&id) || value.object_ids.is_empty() {
                return Err(OpsError::Invalid("group members"));
            }
            let members = value
                .object_ids
                .iter()
                .map(|v| Id::from_proto(Some(v)))
                .collect::<Result<Vec<_>, _>>()?;
            let first = members.first().ok_or(OpsError::Invalid("group members"))?;
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
                project.groups.insert(
                    id.clone(),
                    Group {
                        id: id.clone(),
                        document_id: document,
                        parent: None,
                    },
                );
            }
            for member in members {
                if project.objects.contains_key(&member) {
                    ensure_editable(project, &member, false)?;
                    project
                        .objects
                        .get_mut(&member)
                        .ok_or(OpsError::NotFound)?
                        .state
                        .group_id = Some(id.to_proto());
                } else {
                    ensure_group_editable(project, &member)?;
                    project
                        .groups
                        .get_mut(&member)
                        .ok_or(OpsError::NotFound)?
                        .parent = Some(id.clone());
                }
            }
        }
        Kind::Ungroup(value) => {
            let id = Id::from_proto(value.group_id.as_ref())?;
            ensure_group_editable(project, &id)?;
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
            for member in member_ids {
                ensure_editable(project, &member, false)?;
                project
                    .objects
                    .get_mut(&member)
                    .ok_or(OpsError::NotFound)?
                    .state
                    .group_id = parent.as_ref().map(Id::to_proto);
            }
            for group in project.groups.values_mut() {
                if group.parent.as_ref() == Some(&id) {
                    group.parent = parent.clone();
                }
            }
            project.groups.remove(&id);
        }
        Kind::SetInstruction(value) => {
            let document = Id::from_proto(value.document_id.as_ref())?;
            for target in &value.target_object_ids {
                if project
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
            project.instructions.insert(
                id,
                Instruction {
                    definition: value.clone(),
                    updated_at_ms: time,
                },
            );
        }
        Kind::DeleteInstruction(value) => {
            if project
                .instructions
                .remove(&Id::from_proto(value.instruction_id.as_ref())?)
                .is_none()
            {
                return Err(OpsError::NotFound);
            }
        }
        Kind::AddSemanticSnapshot(value) => {
            let id = Id::from_proto(value.snapshot_id.as_ref())?;
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
            project.semantic_snapshots.insert(
                id,
                SemanticSnapshot {
                    definition: value.clone(),
                    capture_session_id,
                    frame_id: capture.map(|c| c.frame_id),
                    created_at_ms: time,
                },
            );
        }
        Kind::AddResult(value) => {
            let id = Id::from_proto(value.result_id.as_ref())?;
            if project.results.contains_key(&id) {
                return Err(OpsError::AlreadyExists);
            }
            project.results.insert(
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
            let result = project.results.get_mut(&id).ok_or(OpsError::NotFound)?;
            result.status = value.status.clone();
            result.acceptance_mask_asset_id = if value.acceptance_mask_asset_id.is_empty() {
                None
            } else {
                Some(AssetId::try_from(value.acceptance_mask_asset_id.clone())?)
            };
        }
        Kind::MaskOp(value) => {
            let id = Id::from_proto(value.output_object_id.as_ref())?;
            ensure_editable(project, &id, false)?;
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
            project
                .mask_versions
                .entry(id)
                .or_default()
                .push(value.clone());
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
