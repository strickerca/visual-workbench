//! Only recognized canonical tone metadata is omitted from vector editing.
use super::*;

pub(crate) fn is_editing_metadata(project: &Project, document: &Id, object: &Id) -> bool {
    let Some(value) = project.objects.get(object) else {
        return false;
    };
    let Some(pb::object_state::Shape::Adjustment(adjustment)) = &value.state.shape else {
        return false;
    };
    let Ok(layer) = Id::from_proto(value.state.layer_id.as_ref()) else {
        return false;
    };
    bounds(project, document).is_ok()
        && plan::tone(project, document, &layer, object, false).is_ok()
        && ImageToneSettings::from_wire(adjustment).is_ok()
}
