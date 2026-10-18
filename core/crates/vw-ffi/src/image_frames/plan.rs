use super::*;
use crate::project::ProjectState;
use pb::op::Kind as K;
use vw_model::{Layer, Object, OrderKey};

pub(super) fn operation_error(e: vw_ops::OpsError) -> WorkflowError {
    match e {
        vw_ops::OpsError::Locked => WorkflowError::Locked,
        vw_ops::OpsError::NotFound => WorkflowError::Missing,
        vw_ops::OpsError::RevisionMismatch => WorkflowError::Stale,
        vw_ops::OpsError::CounterExhausted => WorkflowError::Limit,
        _ => WorkflowError::Invalid,
    }
}
pub(super) fn validate(command: &ImageFrameCommand) -> WorkflowResult<()> {
    let (layer, object) = match command {
        ImageFrameCommand::Set {
            layer_id,
            object_id,
            settings,
            ..
        } => {
            settings.validate()?;
            (layer_id, object_id)
        }
        ImageFrameCommand::Remove {
            layer_id,
            object_id,
        } => (layer_id, object_id),
    };
    id(layer)?;
    id(object)?;
    if layer == object {
        return Err(WorkflowError::Invalid);
    }
    Ok(())
}
fn layer<'a>(project: &'a Project, document: &Id, layer: &Id) -> WorkflowResult<&'a Layer> {
    let v = project.layers.get(layer).ok_or(WorkflowError::Missing)?;
    if v.definition.document_id.as_ref() != Some(&document.to_proto()) {
        return Err(WorkflowError::Invalid);
    }
    Ok(v)
}
fn only_object<'a>(
    project: &'a Project,
    document: &Id,
    layer_id: &Id,
    object_id: &Id,
    write: bool,
) -> WorkflowResult<(&'a Layer, &'a Object)> {
    let layer = layer(project, document, layer_id)?;
    let object = project
        .objects
        .get(object_id)
        .ok_or(WorkflowError::Missing)?;
    if &object.document_id != document
        || object.state.layer_id.as_ref() != Some(&layer_id.to_proto())
        || project
            .objects
            .values()
            .filter(|v| v.state.layer_id.as_ref() == Some(&layer_id.to_proto()))
            .count()
            != 1
    {
        return Err(WorkflowError::Invalid);
    }
    if write && (layer.locked || object.state.locked) {
        return Err(WorkflowError::Locked);
    }
    Ok((layer, object))
}
fn existing(project: &Project, document: &Id, layer: &Id, object: &Id) -> WorkflowResult<()> {
    let frame = project
        .image_output(document)?
        .ok_or(WorkflowError::Missing)?;
    if frame.layer_id != layer || frame.object_id != object {
        return Err(WorkflowError::Invalid);
    }
    only_object(project, document, layer, object, true)?;
    Ok(())
}
fn fresh(project: &Project, id: &Id) -> bool {
    !project.documents.contains_key(id)
        && !project.layers.contains_key(id)
        && !project.objects.contains_key(id)
        && !project.groups.contains_key(id)
        && !project.instructions.contains_key(id)
        && !project.results.contains_key(id)
        && !project.semantic_snapshots.contains_key(id)
}
fn new_layer(
    project: &Project,
    document: &Id,
    layer: &Id,
    object: &Id,
    kind: &str,
    name: &str,
) -> WorkflowResult<K> {
    if !fresh(project, layer) || !fresh(project, object) {
        return Err(WorkflowError::Invalid);
    }
    if project
        .layers
        .values()
        .filter(|v| v.definition.document_id.as_ref() == Some(&document.to_proto()))
        .count()
        >= MAX_LAYERS
        || project
            .objects
            .values()
            .filter(|v| &v.document_id == document)
            .count()
            >= MAX_OBJECTS
    {
        return Err(WorkflowError::Limit);
    }
    let last = project
        .layers
        .values()
        .filter(|v| v.definition.document_id.as_ref() == Some(&document.to_proto()))
        .map(|v| v.definition.order_key.as_str())
        .max()
        .map(|v| OrderKey::try_from(v.to_owned()))
        .transpose()?;
    let order_key = OrderKey::between(last.as_ref(), None)
        .map_err(|_| WorkflowError::Reconciliation)?
        .into();
    Ok(K::CreateLayer(pb::CreateLayer {
        layer_id: Some(layer.to_proto()),
        document_id: Some(document.to_proto()),
        page_index: -1,
        kind: kind.into(),
        name: name.into(),
        order_key,
    }))
}
fn create(
    document: &Id,
    layer: &Id,
    object: &Id,
    meta: &WorkflowMetadata,
    shape: pb::object_state::Shape,
    transform: pb::Affine,
) -> K {
    K::CreateObject(pb::CreateObject {
        document_id: Some(document.to_proto()),
        state: Some(pb::ObjectState {
            object_id: Some(object.to_proto()),
            layer_id: Some(layer.to_proto()),
            order_key: "V".into(),
            transform: Some(transform),
            style: Some(pb::Style {
                stroke: Some(pb::Color { rgba: 0 }),
                width: 0.0,
                ..Default::default()
            }),
            role: pb::Role::None as i32,
            created_by: meta.device_id.clone(),
            created_at_ms: meta.created_at_ms,
            shape: Some(shape),
            ..Default::default()
        }),
    })
}
fn property(object: &Id, name: &str, value: pb::property_value::Value) -> K {
    K::SetProperty(pb::SetProperty {
        object_id: Some(object.to_proto()),
        property: name.into(),
        value: Some(pb::PropertyValue { value: Some(value) }),
    })
}
fn shape(object: &Id, shape: pb::object_state::Shape) -> WorkflowResult<K> {
    Ok(property(
        object,
        "shape",
        pb::property_value::Value::Raw(
            serde_json::to_vec(&shape).map_err(|_| WorkflowError::Invalid)?,
        ),
    ))
}
fn delete(kinds: &mut Vec<K>, layer: &Id, object: &Id) {
    kinds.push(K::DeleteObject(pb::DeleteObject {
        object_id: Some(object.to_proto()),
    }));
    kinds.push(K::DeleteLayer(pb::DeleteLayer {
        layer_id: Some(layer.to_proto()),
    }));
}
fn visibility(kinds: &mut Vec<K>, layer: &Id, enabled: bool) {
    kinds.push(K::UpdateLayer(pb::UpdateLayer {
        layer_id: Some(layer.to_proto()),
        visible: Some(enabled),
        ..Default::default()
    }));
}
pub(super) fn transaction(
    state: &ProjectState,
    document: &Id,
    meta: &WorkflowMetadata,
    command: ImageFrameCommand,
) -> WorkflowResult<(pb::Transaction, u64)> {
    validate(&command)?;
    let project = state.project()?;
    let mut kinds = Vec::new();
    match command {
        ImageFrameCommand::Set {
            layer_id,
            object_id,
            settings,
            enabled,
        } => {
            let layer = id(&layer_id)?;
            let object = id(&object_id)?;
            let frame = settings.frame(dimensions(bounds(project, document)?))?;
            let shape_value = pb::object_state::Shape::Crop(pb::RectD {
                x: f64::from(settings.crop_x),
                y: f64::from(settings.crop_y),
                w: f64::from(settings.crop_width),
                h: f64::from(settings.crop_height),
            });
            let [a, b, c, d, e, f] = frame.matrix()?.coefficients();
            let transform = pb::Affine { a, b, c, d, e, f };
            if project.image_output(document)?.is_some() {
                existing(project, document, &layer, &object)?;
                kinds.push(shape(&object, shape_value)?);
                kinds.push(property(
                    &object,
                    "transform",
                    pb::property_value::Value::Transform(transform),
                ));
            } else {
                kinds.push(new_layer(
                    project,
                    document,
                    &layer,
                    &object,
                    vw_model::IMAGE_OUTPUT_LAYER_KIND,
                    "Image output",
                )?);
                kinds.push(create(
                    document,
                    &layer,
                    &object,
                    meta,
                    shape_value,
                    transform,
                ));
            }
            visibility(&mut kinds, &layer, enabled);
        }
        ImageFrameCommand::Remove {
            layer_id,
            object_id,
        } => {
            let layer = id(&layer_id)?;
            let object = id(&object_id)?;
            existing(project, document, &layer, &object)?;
            delete(&mut kinds, &layer, &object);
        }
    }
    let next = meta
        .first_lamport
        .checked_add(kinds.len() as u64)
        .ok_or(WorkflowError::Limit)?;
    let ops = kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| pb::Op {
            op_id: Some(pb::OpId {
                device_id: meta.device_id.clone(),
                lamport: meta.first_lamport + index as u64,
            }),
            kind: Some(kind),
        })
        .collect();
    Ok((
        pb::Transaction {
            txn_id: Some(id(&meta.transaction_id)?.to_proto()),
            project_id: Some(project.id.to_proto()),
            device_id: meta.device_id.clone(),
            base_revision: Some(state.store()?.revision()?),
            created_at_wall_ms: meta.created_at_ms,
            gesture_id: None,
            ops,
        },
        next,
    ))
}
