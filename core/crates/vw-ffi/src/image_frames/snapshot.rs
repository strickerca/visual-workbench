use super::*;
use crate::project::ProjectState;

pub(super) fn snapshot(
    state: &ProjectState,
    document: &Id,
    cancel: &Cancellation,
) -> WorkflowResult<ImageFramesSnapshot> {
    let project = state.project()?;
    let asset = bounds(project, document)?;
    let [source_width, source_height] = dimensions(asset);
    let frame = project
        .image_output(document)?
        .map(|value| {
            let layer = project
                .layers
                .get(value.layer_id)
                .ok_or(WorkflowError::Missing)?;
            let object = project
                .objects
                .get(value.object_id)
                .ok_or(WorkflowError::Missing)?;
            Ok::<_, WorkflowError>(ImageFrameEntry {
                layer_id: value.layer_id.to_string(),
                object_id: value.object_id.to_string(),
                enabled: value.enabled,
                locked: layer.locked || object.state.locked,
                settings: ImageFrameSettings::from_frame(value.frame),
            })
        })
        .transpose()?;
    cancel.check()?;
    Ok(ImageFramesSnapshot {
        binding: current(state, document.as_str())?,
        source_asset_id: asset.asset_id.clone(),
        source_width,
        source_height,
        source_bit_depth: asset.bit_depth as u8,
        frame,
    })
}
