//! Recognized global tone metadata is edited through the typed output panel.
//! It remains canonical and participates in native preview/export composition,
//! but is not a paintable vector object in the source D-space editing canvas.
use super::{ImageToneSettings, bounds, plan};
use vw_model::{Id, Project};
use vw_proto::v1 as pb;

pub(crate) fn is_metadata(project: &Project, document: &Id, layer: &Id, object: &Id) -> bool {
    // Ordinary strokes/objects must not trigger repeated whole-project scans.
    let Some(state) = project.objects.get(object) else { return false; };
    let Some(pb::object_state::Shape::Adjustment(value)) = &state.state.shape else { return false; };
    // Match the typed reader's complete document/source and output-frame
    // admission before omitting any object from its source editing view.
    if bounds(project, document).is_err() || project.image_output(document).is_err() { return false; }
    let Some(definition) = project.documents.get(document) else { return false; };
    if !matches!(pb::DocumentKind::try_from(definition.definition.kind), Ok(pb::DocumentKind::Image | pb::DocumentKind::Capture)) {
        return false;
    }
    if plan::tone(project, document, layer, object, false).is_err() { return false; }
    ImageToneSettings::from_wire(value).is_ok()
}
