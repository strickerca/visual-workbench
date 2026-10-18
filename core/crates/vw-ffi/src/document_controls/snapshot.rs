use super::*;
use crate::project::ProjectState;
use vw_model::{AssetId, Project};

pub(super) fn bounds(project: &Project, document: &Id) -> WorkflowResult<()> {
    if project.documents.len() > MAX_DOCUMENTS
        || project
            .layers
            .values()
            .filter(|v| v.definition.document_id.as_ref() == Some(&document.to_proto()))
            .count()
            > MAX_LAYERS
        || project
            .objects
            .values()
            .filter(|v| &v.document_id == document)
            .count()
            > MAX_OBJECTS
        || project
            .groups
            .values()
            .filter(|v| &v.document_id == document)
            .count()
            > MAX_GROUPS
    {
        return Err(WorkflowError::Limit);
    }
    Ok(())
}
pub(super) fn snapshot(
    state: &ProjectState,
    document: &Id,
    cancel: &Cancellation,
) -> WorkflowResult<DocumentControlSnapshot> {
    let project = state.project()?;
    bounds(project, document)?;
    if !project.documents.contains_key(document) {
        return Err(WorkflowError::Missing);
    }
    let mut documents = Vec::new();
    for (id, value) in &project.documents {
        cancel.check()?;
        let source = project
            .assets
            .get(&AssetId::try_from(
                value.definition.primary_asset_id.clone(),
            )?)
            .ok_or(WorkflowError::Missing)?;
        let (width, height) = if source.orientation >= 5 {
            (source.height, source.width)
        } else {
            (source.width, source.height)
        };
        let kind = pb::DocumentKind::try_from(value.definition.kind)
            .map_err(|_| WorkflowError::Invalid)?;
        documents.push(DocumentControlRow {
            document_id: id.to_string(),
            title: value.definition.title.clone(),
            kind: match kind {
                pb::DocumentKind::Image => "image",
                pb::DocumentKind::Capture => "capture",
                pb::DocumentKind::Pdf => "pdf",
                pb::DocumentKind::Svg => "svg",
                _ => return Err(WorkflowError::Invalid),
            }
            .into(),
            width,
            height,
        });
    }
    let mut layers = Vec::new();
    for (id, value) in &project.layers {
        cancel.check()?;
        if value.definition.document_id.as_ref() != Some(&document.to_proto()) {
            continue;
        }
        layers.push(LayerControlRow {
            layer_id: id.to_string(),
            name: value.definition.name.clone(),
            kind: value.definition.kind.clone(),
            order_key: value.definition.order_key.clone(),
            visible: value.visible,
            locked: value.locked,
            opacity: value.opacity,
            blend: value.blend.clone(),
            object_count: 0,
        });
    }
    layers.sort_by(|a, b| {
        a.order_key
            .cmp(&b.order_key)
            .then(a.layer_id.cmp(&b.layer_id))
    });
    let mut objects = Vec::new();
    for (id, value) in &project.objects {
        cancel.check()?;
        if &value.document_id != document {
            continue;
        }
        let s = &value.state;
        let layer_id = Id::from_proto(s.layer_id.as_ref())?.to_string();
        let layer = layers
            .iter_mut()
            .find(|v| v.layer_id == layer_id)
            .ok_or(WorkflowError::Missing)?;
        layer.object_count = layer
            .object_count
            .checked_add(1)
            .ok_or(WorkflowError::Limit)?;
        let a = s.transform.as_ref().ok_or(WorkflowError::Invalid)?;
        objects.push(ObjectControlRow {
            object_id: id.to_string(),
            layer_id,
            group_id: s
                .group_id
                .as_ref()
                .map(|v| Id::from_proto(Some(v)).map(|v| v.to_string()))
                .transpose()?,
            order_key: s.order_key.clone(),
            kind: shape_name(s.shape.as_ref().ok_or(WorkflowError::Invalid)?).into(),
            hidden: s.hidden,
            locked: s.locked,
            transform: crate::Transform {
                a: a.a,
                b: a.b,
                c: a.c,
                d: a.d,
                e: a.e,
                f: a.f,
            },
        });
    }
    // Layer order is already explicit. This order is canonical within each layer.
    objects.sort_by(|a, b| {
        a.layer_id
            .cmp(&b.layer_id)
            .then(a.order_key.cmp(&b.order_key))
            .then(a.object_id.cmp(&b.object_id))
    });
    let groups = project
        .groups
        .iter()
        .filter(|(_, v)| &v.document_id == document)
        .map(|(id, v)| GroupControlRow {
            group_id: id.to_string(),
            parent_id: v.parent.as_ref().map(ToString::to_string),
        })
        .collect();
    let binding = current(state, &document.to_string())?;
    let revision = state.info()?;
    cancel.check()?;
    Ok(DocumentControlSnapshot {
        binding,
        revision,
        documents,
        layers,
        objects,
        groups,
    })
}
pub(super) fn shape_name(shape: &pb::object_state::Shape) -> &'static str {
    use pb::object_state::Shape as S;
    match shape {
        S::Stroke(_) => "stroke",
        S::Line(_) => "line",
        S::Arrow(_) => "arrow",
        S::Rect(_) => "rectangle",
        S::Ellipse(_) => "ellipse",
        S::Polygon(_) => "polygon",
        S::Text(_) => "text",
        S::Marker(_) => "marker",
        S::SelectionVector(_) => "vector selection",
        S::SelectionRaster(_) => "pixel selection",
        S::Crop(_) => "crop",
        S::ResultId(_) => "result",
        S::Adjustment(_) => "adjustment",
    }
}
