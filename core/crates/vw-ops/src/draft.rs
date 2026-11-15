//! Edits made to a project in place, with the way back.
//!
//! A transaction used to be tried on a copy of the whole project. A `Draft`
//! changes the project itself and keeps the value each entity had before its
//! first change, so a rejected transaction is undone by putting those values
//! back and an accepted one knows exactly which entities it touched.
use crate::OpsError;
use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};
use vw_model::{
    AssetId, Document, Group, Id, Instruction, Layer, ModelError, Object, Project, ResultCandidate,
    SemanticSnapshot, StateHasher,
};
use vw_proto::v1;

/// The journal's collections, in the order a project lists them.
pub(crate) const COLLECTIONS: [&str; 9] = [
    "documents",
    "assets",
    "layers",
    "objects",
    "groups",
    "instructions",
    "semantic_snapshots",
    "results",
    "mask_versions",
];

/// Entities one accepted transaction created, changed or removed, by ID.
/// Look an ID up in the project to tell which: absent means removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Touched {
    pub documents: BTreeSet<Id>,
    pub assets: BTreeSet<AssetId>,
    pub layers: BTreeSet<Id>,
    pub objects: BTreeSet<Id>,
    pub groups: BTreeSet<Id>,
    pub instructions: BTreeSet<Id>,
    pub semantic_snapshots: BTreeSet<Id>,
    pub results: BTreeSet<Id>,
    pub mask_versions: BTreeSet<Id>,
}

impl Touched {
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
}

/// The value every touched entity had before a draft first changed it;
/// `None` for an entity the draft created.
#[derive(Debug, Default)]
pub(crate) struct Originals {
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

/// Runs `$body!(field, key type, value type)` once per project collection.
macro_rules! for_each_collection {
    ($body:ident) => {
        $body!(documents, Id, Document);
        $body!(assets, AssetId, v1::AddAsset);
        $body!(layers, Id, Layer);
        $body!(objects, Id, Object);
        $body!(groups, Id, Group);
        $body!(instructions, Id, Instruction);
        $body!(semantic_snapshots, Id, SemanticSnapshot);
        $body!(results, Id, ResultCandidate);
        $body!(mask_versions, Id, Vec<v1::MaskOp>);
    };
}
pub(crate) use for_each_collection;

impl Originals {
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

    pub fn touched(&self) -> Touched {
        let mut touched = Touched::default();
        macro_rules! keys {
            ($field:ident, $key:ty, $value:ty) => {
                touched.$field = self.$field.keys().cloned().collect();
            };
        }
        for_each_collection!(keys);
        touched
    }

    /// Tell a hasher that follows the project which part these edits replaced.
    /// Call it once the edits are in place, and again after undoing them.
    pub fn forget_in(&self, hasher: &mut StateHasher) {
        if !self.only_objects() {
            hasher.reset();
        } else if let Some(first) = self.objects.keys().next() {
            hasher.objects_changed_from(first);
        }
    }

    /// Put every remembered value back, undoing the draft that produced this.
    pub fn restore(self, project: &mut Project) {
        macro_rules! restore {
            ($field:ident, $key:ty, $value:ty) => {
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
        for_each_collection!(restore);
    }

    /// What `Project::validate` would say about `project`, which was valid
    /// when these originals were current. A change confined to objects is
    /// checked without visiting the rest of the project.
    pub fn validate(&self, project: &Project) -> Result<(), ModelError> {
        if !self.only_objects() {
            return project.validate();
        }
        let result = project.validate_changed_objects(
            self.objects
                .iter()
                .map(|(id, original)| (id, original.as_ref())),
        );
        #[cfg(feature = "cross-check")]
        assert_eq!(
            result.as_ref().err().map(ToString::to_string),
            project.validate().err().map(|error| error.to_string()),
            "changed-object validation differs from whole-project validation"
        );
        result
    }
}

/// A project being edited in place. Reads see the edits made so far.
pub(crate) struct Draft<'a> {
    project: &'a mut Project,
    originals: Originals,
}

macro_rules! accessors {
    ($field:ident, $key:ty, $value:ty, $insert:ident, $remove:ident) => {
        /// Create or replace an entry.
        pub fn $insert(&mut self, id: $key, value: $value) {
            let previous = self.project.$field.insert(id.clone(), value);
            if let Entry::Vacant(slot) = self.originals.$field.entry(id) {
                slot.insert(previous);
            }
        }
        /// Remove an entry; false when there was none.
        pub fn $remove(&mut self, id: &$key) -> bool {
            let Some(previous) = self.project.$field.remove(id) else {
                return false;
            };
            if let Entry::Vacant(slot) = self.originals.$field.entry(id.clone()) {
                slot.insert(Some(previous));
            }
            true
        }
    };
}
macro_rules! mutable {
    ($field:ident, $key:ty, $value:ty, $get_mut:ident) => {
        /// Mutable access to an existing entry, remembering its current value.
        pub fn $get_mut(&mut self, id: &$key) -> Option<&mut $value> {
            let value = self.project.$field.get_mut(id)?;
            if let Entry::Vacant(slot) = self.originals.$field.entry(id.clone()) {
                slot.insert(Some(value.clone()));
            }
            Some(value)
        }
    };
}

impl<'a> Draft<'a> {
    pub fn new(project: &'a mut Project) -> Self {
        Self {
            project,
            originals: Originals::default(),
        }
    }
    pub fn project(&self) -> &Project {
        self.project
    }
    pub fn originals(&self) -> &Originals {
        &self.originals
    }
    /// Validate the edited project; see `Originals::validate`.
    pub fn validate(&self) -> Result<(), ModelError> {
        self.originals.validate(self.project)
    }
    /// Keep the edits and hand back what they replaced.
    pub fn keep(self) -> Originals {
        self.originals
    }
    /// Undo every edit.
    pub fn discard(self) {
        self.originals.restore(self.project);
    }

    accessors!(documents, Id, Document, insert_document, remove_document);
    accessors!(assets, AssetId, v1::AddAsset, insert_asset, remove_asset);
    accessors!(layers, Id, Layer, insert_layer, remove_layer);
    accessors!(objects, Id, Object, insert_object, remove_object);
    accessors!(groups, Id, Group, insert_group, remove_group);
    accessors!(
        instructions,
        Id,
        Instruction,
        insert_instruction,
        remove_instruction
    );
    accessors!(
        semantic_snapshots,
        Id,
        SemanticSnapshot,
        insert_semantic_snapshot,
        remove_semantic_snapshot
    );
    accessors!(results, Id, ResultCandidate, insert_result, remove_result);
    accessors!(
        mask_versions,
        Id,
        Vec<v1::MaskOp>,
        insert_mask_versions,
        remove_mask_versions
    );
    mutable!(documents, Id, Document, document_mut);
    mutable!(layers, Id, Layer, layer_mut);
    mutable!(objects, Id, Object, object_mut);
    mutable!(groups, Id, Group, group_mut);
    mutable!(results, Id, ResultCandidate, result_mut);
    mutable!(mask_versions, Id, Vec<v1::MaskOp>, mask_versions_mut);
}

/// Run `edit` on `project` in place. On success the edits stay and what they
/// replaced is returned; on any error, including failed validation, the
/// project is exactly as it was.
pub(crate) fn edit(
    project: &mut Project,
    edit: impl FnOnce(&mut Draft<'_>) -> Result<(), OpsError>,
) -> Result<Originals, OpsError> {
    let mut draft = Draft::new(project);
    let outcome = edit(&mut draft).and_then(|()| Ok(draft.validate()?));
    match outcome {
        Ok(()) => Ok(draft.keep()),
        Err(error) => {
            draft.discard();
            Err(error)
        }
    }
}
