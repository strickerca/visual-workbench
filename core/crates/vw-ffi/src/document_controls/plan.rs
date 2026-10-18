use super::*;
use crate::project::ProjectState;
use std::collections::BTreeSet;
use vw_model::{Layer, Object, OrderKey, Project};

pub(super) fn operation_error(value: vw_ops::OpsError) -> WorkflowError {
    match value {
        vw_ops::OpsError::Locked => WorkflowError::Locked,
        vw_ops::OpsError::NotFound => WorkflowError::Missing,
        vw_ops::OpsError::RevisionMismatch => WorkflowError::Stale,
        vw_ops::OpsError::CounterExhausted => WorkflowError::Limit,
        _ => WorkflowError::Invalid,
    }
}
fn id_list(values: &[String], minimum: usize) -> WorkflowResult<()> {
    if values.len() < minimum || values.len() > MAX_TARGETS {
        return Err(WorkflowError::Limit);
    }
    let mut seen = BTreeSet::new();
    for value in values {
        bounded_id(value)?;
        if !seen.insert(value) {
            return Err(WorkflowError::Invalid);
        }
    }
    Ok(())
}
fn target_input(value: &DocumentControlTargets) -> WorkflowResult<()> {
    match value {
        DocumentControlTargets::Objects { object_ids } => id_list(object_ids, 1),
        DocumentControlTargets::Group { group_id } => bounded_id(group_id).map(|_| ()),
    }
}
pub(super) fn validate(value: &DocumentControlCommand) -> WorkflowResult<()> {
    use DocumentControlCommand as C;
    match value {
        C::CreateAnnotationLayer {
            layer_id,
            name: value,
        } => {
            bounded_id(layer_id)?;
            name(value)?;
        }
        C::UpdateLayer { update } => {
            bounded_id(&update.layer_id)?;
            if update.name.is_none()
                && update.visible.is_none()
                && update.locked.is_none()
                && update.opacity.is_none()
                && update.blend.is_none()
            {
                return Err(WorkflowError::Invalid);
            }
            if let Some(value) = &update.name {
                name(value)?;
            }
            if update
                .opacity
                .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
                || update
                    .blend
                    .as_ref()
                    .is_some_and(|v| !matches!(v.as_str(), "normal" | "multiply"))
            {
                return Err(WorkflowError::Invalid);
            }
        }
        C::PlaceLayer {
            layer_id,
            before_layer_id,
        } => {
            bounded_id(layer_id)?;
            if let Some(other) = before_layer_id {
                bounded_id(other)?;
                if other == layer_id {
                    return Err(WorkflowError::Invalid);
                }
            }
        }
        C::DeleteEmptyLayer { layer_id } => {
            bounded_id(layer_id)?;
        }
        C::Group {
            group_id,
            member_ids,
        } => {
            bounded_id(group_id)?;
            id_list(member_ids, 2)?;
            if member_ids.contains(group_id) {
                return Err(WorkflowError::Invalid);
            }
        }
        C::Ungroup { group_id } => {
            bounded_id(group_id)?;
        }
        C::SetObjectLocks { targets, .. } | C::SetObjectHidden { targets, .. } => {
            target_input(targets)?
        }
        C::MoveObjects { targets, layer_id } => {
            target_input(targets)?;
            bounded_id(layer_id)?;
        }
        C::RenameDocument { title } => name(title)?,
        C::CreateDocumentFromSource {
            document_id,
            layer_id,
            title,
        } => {
            bounded_id(document_id)?;
            bounded_id(layer_id)?;
            name(title)?;
            if document_id == layer_id {
                return Err(WorkflowError::Invalid);
            }
        }
    }
    Ok(())
}
fn layer<'a>(project: &'a Project, document: &Id, id: &Id) -> WorkflowResult<&'a Layer> {
    let value = project.layers.get(id).ok_or(WorkflowError::Missing)?;
    if value.definition.document_id.as_ref() != Some(&document.to_proto()) {
        return Err(WorkflowError::Invalid);
    }
    Ok(value)
}
fn object<'a>(project: &'a Project, document: &Id, id: &Id) -> WorkflowResult<&'a Object> {
    let value = project.objects.get(id).ok_or(WorkflowError::Missing)?;
    if &value.document_id != document {
        return Err(WorkflowError::Invalid);
    }
    Ok(value)
}
fn group(project: &Project, document: &Id, id: &Id) -> WorkflowResult<()> {
    if &project
        .groups
        .get(id)
        .ok_or(WorkflowError::Missing)?
        .document_id
        != document
    {
        return Err(WorkflowError::Invalid);
    }
    Ok(())
}
fn unlocked(project: &Project, document: &Id, id: &Id, unlocking: bool) -> WorkflowResult<()> {
    let value = object(project, document, id)?;
    let layer = layer(
        project,
        document,
        &Id::from_proto(value.state.layer_id.as_ref())?,
    )?;
    if layer.locked || (value.state.locked && !unlocking) {
        return Err(WorkflowError::Locked);
    }
    Ok(())
}
fn ordered_layers<'a>(
    project: &'a Project,
    document: &Id,
    except: Option<&Id>,
) -> Vec<(&'a Id, &'a Layer)> {
    let mut values = project
        .layers
        .iter()
        .filter(|(id, v)| {
            except != Some(*id) && v.definition.document_id.as_ref() == Some(&document.to_proto())
        })
        .collect::<Vec<_>>();
    values.sort_by(|(a, x), (b, y)| {
        x.definition
            .order_key
            .cmp(&y.definition.order_key)
            .then(a.cmp(b))
    });
    values
}
fn between(left: Option<&str>, right: Option<&str>) -> WorkflowResult<String> {
    let left = left.map(|v| OrderKey::try_from(v.to_owned())).transpose()?;
    let right = right
        .map(|v| OrderKey::try_from(v.to_owned()))
        .transpose()?;
    // Concurrent equal keys are valid. Moving into their exact tied interval
    // cannot be represented by changing only one key; never silently reshuffle.
    Ok(OrderKey::between(left.as_ref(), right.as_ref())
        .map_err(|_| WorkflowError::Reconciliation)?
        .into())
}
fn resolve(
    project: &Project,
    document: &Id,
    targets: &DocumentControlTargets,
    cancel: &Cancellation,
) -> WorkflowResult<Vec<Id>> {
    let mut ids = Vec::new();
    match targets {
        DocumentControlTargets::Objects { object_ids } => {
            for id in object_ids {
                let id = bounded_id(id)?;
                object(project, document, &id)?;
                ids.push(id);
            }
        }
        DocumentControlTargets::Group { group_id } => {
            let target = bounded_id(group_id)?;
            group(project, document, &target)?;
            for (id, value) in &project.objects {
                cancel.check()?;
                if &value.document_id != document {
                    continue;
                }
                let mut parent = value
                    .state
                    .group_id
                    .as_ref()
                    .map(|v| Id::from_proto(Some(v)))
                    .transpose()?;
                let mut depth = 0;
                while let Some(current) = parent {
                    depth += 1;
                    if depth > MAX_GROUPS {
                        return Err(WorkflowError::Limit);
                    }
                    if current == target {
                        ids.push(id.clone());
                        break;
                    }
                    parent = project
                        .groups
                        .get(&current)
                        .ok_or(WorkflowError::Missing)?
                        .parent
                        .clone();
                }
                if ids.len() > MAX_TARGETS {
                    return Err(WorkflowError::Limit);
                }
            }
        }
    }
    if ids.is_empty() {
        return Err(WorkflowError::Missing);
    }
    Ok(ids)
}
fn flag(id: &Id, property: &str, value: bool) -> pb::op::Kind {
    pb::op::Kind::SetProperty(pb::SetProperty {
        object_id: Some(id.to_proto()),
        property: property.into(),
        value: Some(pb::PropertyValue {
            value: Some(pb::property_value::Value::Flag(value)),
        }),
    })
}

pub(super) fn transaction(
    state: &ProjectState,
    document: &Id,
    meta: &WorkflowMetadata,
    command: DocumentControlCommand,
    cancel: &Cancellation,
) -> WorkflowResult<(pb::Transaction, u64)> {
    use DocumentControlCommand as C;
    use pb::op::Kind as K;
    let project = state.project()?;
    let mut kinds = Vec::new();
    match command {
        C::CreateAnnotationLayer { layer_id, name } => {
            if project
                .layers
                .values()
                .filter(|v| v.definition.document_id.as_ref() == Some(&document.to_proto()))
                .count()
                >= MAX_LAYERS
            {
                return Err(WorkflowError::Limit);
            }
            let id = bounded_id(&layer_id)?;
            if project.layers.contains_key(&id) {
                return Err(WorkflowError::Invalid);
            }
            let ordered = ordered_layers(project, document, None);
            let order_key = between(
                ordered.last().map(|(_, v)| v.definition.order_key.as_str()),
                None,
            )?;
            kinds.push(K::CreateLayer(pb::CreateLayer {
                layer_id: Some(id.to_proto()),
                document_id: Some(document.to_proto()),
                page_index: -1,
                name,
                kind: "annotation".into(),
                order_key,
            }));
        }
        C::UpdateLayer { update } => {
            let id = bounded_id(&update.layer_id)?;
            let value = layer(project, document, &id)?;
            if value.locked
                && (update.name.is_some()
                    || update.visible.is_some()
                    || update.opacity.is_some()
                    || update.blend.is_some())
            {
                return Err(WorkflowError::Locked);
            }
            if value.definition.kind != "annotation"
                && update.blend.as_ref().is_some_and(|v| v != "normal")
            {
                return Err(WorkflowError::Invalid);
            }
            kinds.push(K::UpdateLayer(pb::UpdateLayer {
                layer_id: Some(id.to_proto()),
                name: update.name,
                order_key: None,
                visible: update.visible,
                locked: update.locked,
                opacity: update.opacity.map(|v| v as f32),
                blend: update.blend,
            }));
        }
        C::PlaceLayer {
            layer_id,
            before_layer_id,
        } => {
            let id = bounded_id(&layer_id)?;
            if layer(project, document, &id)?.locked {
                return Err(WorkflowError::Locked);
            }
            let ordered = ordered_layers(project, document, Some(&id));
            let index = if let Some(next) = before_layer_id {
                let next = bounded_id(&next)?;
                ordered
                    .iter()
                    .position(|(id, _)| *id == &next)
                    .ok_or(WorkflowError::Missing)?
            } else {
                ordered.len()
            };
            let left = index
                .checked_sub(1)
                .and_then(|i| ordered.get(i))
                .map(|(_, v)| v.definition.order_key.as_str());
            let right = ordered
                .get(index)
                .map(|(_, v)| v.definition.order_key.as_str());
            kinds.push(K::UpdateLayer(pb::UpdateLayer {
                layer_id: Some(id.to_proto()),
                order_key: Some(between(left, right)?),
                ..Default::default()
            }));
        }
        C::DeleteEmptyLayer { layer_id } => {
            let id = bounded_id(&layer_id)?;
            let value = layer(project, document, &id)?;
            if value.locked {
                return Err(WorkflowError::Locked);
            }
            if project
                .objects
                .values()
                .any(|o| o.state.layer_id.as_ref() == Some(&id.to_proto()))
            {
                return Err(WorkflowError::Invalid);
            }
            kinds.push(K::DeleteLayer(pb::DeleteLayer {
                layer_id: Some(id.to_proto()),
            }));
        }
        C::Group {
            group_id,
            member_ids,
        } => {
            if project
                .groups
                .values()
                .filter(|g| &g.document_id == document)
                .count()
                >= MAX_GROUPS
            {
                return Err(WorkflowError::Limit);
            }
            let id = bounded_id(&group_id)?;
            if project.groups.contains_key(&id) || project.objects.contains_key(&id) {
                return Err(WorkflowError::Invalid);
            }
            let mut members = Vec::new();
            for member in member_ids {
                cancel.check()?;
                let member = bounded_id(&member)?;
                if project.objects.contains_key(&member) {
                    unlocked(project, document, &member, false)?;
                } else {
                    group(project, document, &member)?;
                }
                members.push(member.to_proto());
            }
            kinds.push(K::Group(pb::Group {
                group_id: Some(id.to_proto()),
                object_ids: members,
            }));
        }
        C::Ungroup { group_id } => {
            let id = bounded_id(&group_id)?;
            group(project, document, &id)?;
            kinds.push(K::Ungroup(pb::Ungroup {
                group_id: Some(id.to_proto()),
            }));
        }
        C::SetObjectLocks { targets, locked } => {
            for id in resolve(project, document, &targets, cancel)? {
                cancel.check()?;
                unlocked(project, document, &id, true)?;
                kinds.push(flag(&id, "locked", locked));
            }
        }
        C::SetObjectHidden { targets, hidden } => {
            for id in resolve(project, document, &targets, cancel)? {
                cancel.check()?;
                unlocked(project, document, &id, false)?;
                kinds.push(flag(&id, "hidden", hidden));
            }
        }
        C::MoveObjects { targets, layer_id } => {
            let layer_id = bounded_id(&layer_id)?;
            let destination = layer(project, document, &layer_id)?;
            if destination.locked {
                return Err(WorkflowError::Locked);
            }
            if destination.definition.kind != "annotation" {
                return Err(WorkflowError::Invalid);
            }
            let mut ids = resolve(project, document, &targets, cancel)?;
            for id in &ids {
                unlocked(project, document, id, false)?;
                let value = object(project, document, id)?;
                if matches!(
                    value.state.shape,
                    Some(
                        pb::object_state::Shape::SelectionVector(_)
                            | pb::object_state::Shape::SelectionRaster(_)
                            | pb::object_state::Shape::Crop(_)
                            | pb::object_state::Shape::ResultId(_)
                            | pb::object_state::Shape::Adjustment(_)
                    )
                ) {
                    return Err(WorkflowError::Invalid);
                }
            }
            // Keep the same back-to-front order when a group spans layers.
            let order: std::collections::BTreeMap<Id, usize> =
                ordered_layers(project, document, None)
                    .into_iter()
                    .enumerate()
                    .map(|(n, (id, _))| (id.clone(), n))
                    .collect();
            let mut keyed = ids
                .into_iter()
                .map(|id| {
                    let value = &object(project, document, &id)?.state;
                    let layer = Id::from_proto(value.layer_id.as_ref())?;
                    let layer_order = *order.get(&layer).ok_or(WorkflowError::Missing)?;
                    Ok((layer_order, value.order_key.clone(), id))
                })
                .collect::<WorkflowResult<Vec<_>>>()?;
            keyed.sort();
            ids = keyed.into_iter().map(|(_, _, id)| id).collect();
            let selected: BTreeSet<_> = ids.iter().collect();
            let mut key = project
                .objects
                .iter()
                .filter(|(id, v)| {
                    v.state.layer_id.as_ref() == Some(&layer_id.to_proto())
                        && !selected.contains(id)
                })
                .map(|(_, v)| v.state.order_key.clone())
                .max();
            for id in ids {
                cancel.check()?;
                let next = between(key.as_deref(), None)?;
                kinds.push(K::Reorder(pb::Reorder {
                    object_id: Some(id.to_proto()),
                    order_key: next.clone(),
                    layer_id: Some(layer_id.to_proto()),
                }));
                key = Some(next);
            }
        }
        C::RenameDocument { title } => kinds.push(K::UpdateDocument(pb::UpdateDocument {
            document_id: Some(document.to_proto()),
            title,
        })),
        C::CreateDocumentFromSource {
            document_id,
            layer_id,
            title,
        } => {
            if project.documents.len() >= MAX_DOCUMENTS {
                return Err(WorkflowError::Limit);
            }
            let new_id = bounded_id(&document_id)?;
            let layer_id = bounded_id(&layer_id)?;
            if project.documents.contains_key(&new_id) || project.layers.contains_key(&layer_id) {
                return Err(WorkflowError::Invalid);
            }
            let old = project
                .documents
                .get(document)
                .ok_or(WorkflowError::Missing)?;
            if !matches!(
                pb::DocumentKind::try_from(old.definition.kind),
                Ok(pb::DocumentKind::Image | pb::DocumentKind::Capture)
            ) || !old.pages.is_empty()
            {
                return Err(WorkflowError::Invalid);
            }
            let mut definition = old.definition.clone();
            definition.document_id = Some(new_id.to_proto());
            definition.title = title;
            kinds.push(K::CreateDocument(definition));
            kinds.push(K::CreateLayer(pb::CreateLayer {
                layer_id: Some(layer_id.to_proto()),
                document_id: Some(new_id.to_proto()),
                page_index: -1,
                name: "Annotations".into(),
                kind: "annotation".into(),
                order_key: between(None, None)?,
            }));
        }
    }
    cancel.check()?;
    if kinds.is_empty() || kinds.len() > MAX_TARGETS {
        return Err(WorkflowError::Limit);
    }
    let next = meta
        .first_lamport
        .checked_add(kinds.len() as u64)
        .ok_or(WorkflowError::Limit)?;
    let ops = kinds
        .into_iter()
        .enumerate()
        .map(|(i, kind)| pb::Op {
            op_id: Some(pb::OpId {
                device_id: meta.device_id.clone(),
                lamport: meta.first_lamport + i as u64,
            }),
            kind: Some(kind),
        })
        .collect();
    Ok((
        pb::Transaction {
            txn_id: Some(bounded_id(&meta.transaction_id)?.to_proto()),
            device_id: meta.device_id.clone(),
            base_revision: Some(state.store()?.revision()?),
            ops,
            gesture_id: None,
            created_at_wall_ms: meta.created_at_ms,
            project_id: Some(project.id.to_proto()),
        },
        next,
    ))
}
