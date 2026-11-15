//! Edits made to a project in place, with the way back.
//!
//! A transaction used to be tried on a copy of the whole project. A `Draft`
//! changes the project itself and keeps the value each entity had before its
//! first change, so a rejected transaction is undone by putting those values
//! back and an accepted one knows exactly which entities it touched.
use crate::OpsError;
use std::collections::{BTreeSet, btree_map::Entry};
use vw_model::{
    AssetId, Document, Group, Id, Instruction, Layer, ModelError, Object, Project, ResultCandidate,
    SemanticSnapshot,
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
    /// How many of all these entities the transaction created. Deciding a
    /// new entity again costs less than deciding a change to an existing one.
    pub created: usize,
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
pub(crate) type Originals = vw_model::Replaced;

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

impl Touched {
    /// Add everything `other` names.
    pub fn extend(&mut self, other: &Self) {
        macro_rules! union {
            ($field:ident, $key:ty, $value:ty) => {
                self.$field.extend(other.$field.iter().cloned());
            };
        }
        for_each_collection!(union);
        self.created = self.created.saturating_add(other.created);
    }

    /// The entities a kept draft touched.
    pub(crate) fn of(originals: &Originals) -> Self {
        let mut touched = Self::default();
        macro_rules! keys {
            ($field:ident, $key:ty, $value:ty) => {
                touched.$field = originals.$field.keys().cloned().collect();
                touched.created += originals
                    .$field
                    .values()
                    .filter(|original| original.is_none())
                    .count();
            };
        }
        for_each_collection!(keys);
        touched
    }
}

/// What `Project::validate` would say about `project`, which was valid when
/// `originals` were current. Only what the edits can have affected is
/// checked, never the entities they left alone.
pub(crate) fn validate(originals: &Originals, project: &Project) -> Result<(), ModelError> {
    let result = project.validate_replaced(originals);
    #[cfg(feature = "cross-check")]
    assert_eq!(
        result.as_ref().err().map(ToString::to_string),
        project.validate().err().map(|error| error.to_string()),
        "validation of what an edit replaced differs from whole-project validation"
    );
    result
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
    /// Validate the edited project; see `validate`.
    pub fn validate(&self) -> Result<(), ModelError> {
        validate(&self.originals, self.project)
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
