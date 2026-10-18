use crate::{ProjectInfo, Transform, WorkflowBinding};

#[derive(Clone, Debug, uniffi::Record)]
pub struct DocumentControlRow {
    pub document_id: String,
    pub title: String,
    pub kind: String,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct LayerControlRow {
    pub layer_id: String,
    pub name: String,
    pub kind: String,
    pub order_key: String,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f64,
    pub blend: String,
    pub object_count: u32,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ObjectControlRow {
    pub object_id: String,
    pub layer_id: String,
    pub group_id: Option<String>,
    pub order_key: String,
    pub kind: String,
    pub hidden: bool,
    pub locked: bool,
    pub transform: Transform,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct GroupControlRow {
    pub group_id: String,
    pub parent_id: Option<String>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct DocumentControlSnapshot {
    pub binding: WorkflowBinding,
    pub revision: ProjectInfo,
    pub documents: Vec<DocumentControlRow>,
    /// Back to front; equal fractional indexes use canonical stable-ID order.
    pub layers: Vec<LayerControlRow>,
    /// Includes hidden objects, so explicit Show/Unlock remains available.
    pub objects: Vec<ObjectControlRow>,
    pub groups: Vec<GroupControlRow>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct LayerControlUpdate {
    pub layer_id: String,
    pub name: Option<String>,
    pub visible: Option<bool>,
    pub locked: Option<bool>,
    pub opacity: Option<f64>,
    pub blend: Option<String>,
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum DocumentControlTargets {
    Objects { object_ids: Vec<String> },
    /// Resolved once from canonical nested membership under the visible fence.
    Group { group_id: String },
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum DocumentControlCommand {
    CreateAnnotationLayer { layer_id: String, name: String },
    UpdateLayer { update: LayerControlUpdate },
    /// None means top/front. A tied/exhausted interval fails without reordering peers.
    PlaceLayer { layer_id: String, before_layer_id: Option<String> },
    /// Deleting contents requires the existing linked-object deletion workflow.
    DeleteEmptyLayer { layer_id: String },
    Group { group_id: String, member_ids: Vec<String> },
    Ungroup { group_id: String },
    SetObjectLocks { targets: DocumentControlTargets, locked: bool },
    SetObjectHidden { targets: DocumentControlTargets, hidden: bool },
    /// Only ordinary annotation objects may move into an annotation layer.
    MoveObjects { targets: DocumentControlTargets, layer_id: String },
    RenameDocument { title: String },
    /// Independent image/capture document sharing the unchanged immutable source.
    CreateDocumentFromSource { document_id: String, layer_id: String, title: String },
}
