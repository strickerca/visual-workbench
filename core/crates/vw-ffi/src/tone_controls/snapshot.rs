use super::*;
use crate::project::ProjectState;

pub(super) fn tones(
    project: &Project,
    document: &Id,
    cancel: &Cancellation,
) -> WorkflowResult<Vec<ToneControlEntry>> {
    let mut ordered = project
        .layers
        .iter()
        .filter(|(_, v)| v.definition.document_id.as_ref() == Some(&document.to_proto()))
        .collect::<Vec<_>>();
    ordered.sort_by(|(a, x), (b, y)| {
        x.definition
            .order_key
            .cmp(&y.definition.order_key)
            .then(a.cmp(b))
    });
    let mut result = Vec::new();
    for (layer_id, layer) in ordered {
        cancel.check()?;
        for (object_id, object) in project.objects.iter().filter(|(_, v)| {
            &v.document_id == document && v.state.layer_id.as_ref() == Some(&layer_id.to_proto())
        }) {
            if let Some(pb::object_state::Shape::Adjustment(value)) = &object.state.shape {
                // Do not expose a partial catalog by silently skipping an adjustment
                // whose shape/placement is outside this typed control's contract.
                plan::tone(project, document, layer_id, object_id, false)?;
                result.push(ToneControlEntry {
                    layer_id: layer_id.to_string(),
                    object_id: object_id.to_string(),
                    enabled: layer.visible,
                    locked: layer.locked || object.state.locked,
                    opacity: layer.opacity,
                    settings: ImageToneSettings::from_wire(value)?,
                });
            }
        }
    }
    cancel.check()?;
    Ok(result)
}
pub(super) fn snapshot(
    state: &ProjectState,
    document: &Id,
    cancel: &Cancellation,
) -> WorkflowResult<ToneControlSnapshot> {
    let project = state.project()?;
    let asset = bounds(project, document)?;
    let [source_width, source_height] = dimensions(asset);
    let tones = tones(project, document, cancel)?;
    Ok(ToneControlSnapshot {
        binding: current(state, document.as_str())?,
        source_asset_id: asset.asset_id.clone(),
        source_width,
        source_height,
        source_bit_depth: asset.bit_depth as u8,
        tones,
    })
}
