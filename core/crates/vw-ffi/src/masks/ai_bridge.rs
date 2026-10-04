//! Narrow borrowed bridge; all calls are on the owning project worker.
use super::*;
pub(crate) struct AiInputs {
    pub source: Vec<u8>,
    pub source_asset: pb::AddAsset,
    pub masks: Vec<Mask>,
}
pub(crate) fn inputs(
    state: &ProjectState,
    expected: &crate::WorkflowBinding,
    selections: &[SelectionVersion],
    budget: u64,
    cancel: &Cancellation,
) -> SelectionResult<AiInputs> {
    let doc = document(state, &expected.document_id)?;
    if doc.binding.project_id != expected.project_id
        || doc.binding.host_seq != expected.host_seq
        || doc.binding.state_hash != expected.state_hash
    {
        return Err(SelectionError::Conflict);
    }
    if selections.is_empty() || selections.len() > 64 {
        return Err(SelectionError::Limit);
    }
    let complete = versions(state, &expected.document_id)?;
    if selections.len() != complete.len() || complete.iter().any(|v| !selections.contains(v)) {
        return Err(SelectionError::Conflict);
    }
    let size = Size::new(doc.width, doc.height)?;
    let project = state.project()?;
    let source_id = AssetId::try_from(doc.binding.source_asset_id.clone())?;
    let source = project
        .assets
        .get(&source_id)
        .ok_or(SelectionError::Corrupt)?;
    let retained = (size.pixels() as u64)
        .checked_mul(selections.len() as u64 * 2 + 16)
        .and_then(|n| n.checked_add(source.byte_size))
        .and_then(|n| n.checked_add(CODEC_RESERVE))
        .ok_or(SelectionError::Limit)?;
    memory(retained, budget)?;
    let mut seen = std::collections::BTreeSet::new();
    for selection in selections {
        admit_version(selection)?;
        if !seen.insert(&selection.object_id) {
            return Err(SelectionError::Invalid);
        }
        let (_, _, visible) = check_version(project, &doc.binding, selection, size)?;
        let object = project
            .objects
            .get(&Id::try_from(selection.object_id.clone())?)
            .ok_or(SelectionError::Conflict)?;
        if !visible || object.state.role != pb::Role::Change as i32 {
            return Err(SelectionError::Invalid);
        }
    }
    let bytes = export::read_blob(state, &source_id, source.byte_size, MAX_ENCODED, cancel)?;
    let mut masks = Vec::with_capacity(selections.len());
    for selection in selections {
        cancel.check()?;
        masks.push(load_mask(state, selection, size, cancel)?);
    }
    Ok(AiInputs {
        source: bytes,
        source_asset: source.clone(),
        masks,
    })
}
pub(crate) fn blob(
    state: &ProjectState,
    id: &AssetId,
    length: u64,
    limit: u64,
    cancel: &Cancellation,
) -> SelectionResult<Vec<u8>> {
    export::read_blob(state, id, length, limit, cancel)
}

pub(crate) fn versions(
    state: &ProjectState,
    document_id: &str,
) -> SelectionResult<Vec<SelectionVersion>> {
    let doc = document(state, document_id)?;
    let size = Size::new(doc.width, doc.height)?;
    let project = state.project()?;
    let mut result = Vec::new();
    for (id, value) in &project.objects {
        if value.document_id.as_str() != document_id
            || value.state.role != pb::Role::Change as i32
            || value.state.hidden
        {
            continue;
        }
        let layer = project
            .layers
            .get(&Id::from_proto(value.state.layer_id.as_ref())?)
            .ok_or(SelectionError::Corrupt)?;
        if !layer.visible || layer.opacity == 0.0 {
            continue;
        }
        match &value.state.shape {
            Some(pb::object_state::Shape::SelectionVector(_)) => {
                return Err(SelectionError::Unsupported);
            }
            Some(pb::object_state::Shape::SelectionRaster(_)) => {}
            _ => continue,
        }
        if result.len() == 64 {
            return Err(SelectionError::Limit);
        }
        result.push(describe(project, &doc.binding, id.as_str(), size)?.0);
    }
    Ok(result)
}
