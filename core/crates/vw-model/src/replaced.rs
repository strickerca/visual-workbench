//! What a set of edits replaced in a project.
//!
//! An edit is made on the project itself. `Replaced` holds the value every
//! entity had before the edit first touched it, which is enough to take the
//! edit back, to tell exactly which entities it touched, and to check and
//! hash the edited project without visiting the entities it left alone.
use crate::{
    AssetId, Document, Group, Id, Instruction, Layer, Object, Project, ResultCandidate,
    SemanticSnapshot,
};
use std::collections::BTreeMap;
use vw_proto::v1;

/// The value every touched entity had before a set of edits first changed
/// it; `None` for an entity the edits created. Look an ID up in the edited
/// project to tell a change from a removal.
#[derive(Debug, Default)]
pub struct Replaced {
    pub documents: BTreeMap<Id, Option<Document>>,
    pub assets: BTreeMap<AssetId, Option<v1::AddAsset>>,
    pub layers: BTreeMap<Id, Option<Layer>>,
    pub objects: BTreeMap<Id, Option<Object>>,
    pub groups: BTreeMap<Id, Option<Group>>,
    pub instructions: BTreeMap<Id, Option<Instruction>>,
    pub semantic_snapshots: BTreeMap<Id, Option<SemanticSnapshot>>,
    pub results: BTreeMap<Id, Option<ResultCandidate>>,
    pub mask_versions: BTreeMap<Id, Option<Vec<v1::MaskOp>>>,
}

impl Replaced {
    /// Whether nothing but objects was touched.
    pub fn only_objects(&self) -> bool {
        self.documents.is_empty()
            && self.assets.is_empty()
            && self.layers.is_empty()
            && self.groups.is_empty()
            && self.instructions.is_empty()
            && self.semantic_snapshots.is_empty()
            && self.results.is_empty()
            && self.mask_versions.is_empty()
    }

    /// Put every remembered value back, undoing the edits.
    pub fn restore(self, project: &mut Project) {
        macro_rules! restore {
            ($field:ident) => {
                for (id, original) in self.$field {
                    match original {
                        Some(value) => {
                            project.$field.insert(id, value);
                        }
                        None => {
                            project.$field.remove(&id);
                        }
                    }
                }
            };
        }
        restore!(documents);
        restore!(assets);
        restore!(layers);
        restore!(objects);
        restore!(groups);
        restore!(instructions);
        restore!(semantic_snapshots);
        restore!(results);
        restore!(mask_versions);
    }
}
