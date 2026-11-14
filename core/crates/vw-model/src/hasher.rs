//! The canonical state hash, recomputed from where a project last changed.
//!
//! `Project::state_hash` serializes and hashes every byte of the project. The
//! canonical bytes are one JSON object whose `objects` member lists objects in
//! ID order, and BLAKE3 can resume from the state it had after any prefix, so
//! the bytes before the first changed object never need hashing twice. New
//! objects carry time-ordered IDs and therefore land at the end: saving one
//! more stroke hashes that stroke and the short remainder of the project.
use crate::{AssetId, DeviceId, Id, ModelError, Object, Project, StateHash, state::normalize};
use serde::Serialize;
use std::{collections::BTreeMap, ops::Bound};
use vw_proto::v1;

pub(crate) const DOMAIN: &[u8] = b"VisualWorkbench.Project.v1\0";
/// Objects between remembered hash states. An edit to an existing object
/// re-serializes at most this many objects before the one that changed.
const SPACING: usize = 64;

/// Remembers how far the canonical bytes of one project were hashed.
///
/// Give it the same project every time and report each change first:
/// `objects_changed_from` when only objects changed, `reset` otherwise. The
/// result is bit-identical to `Project::state_hash`. It does not validate.
#[derive(Debug, Clone, Default)]
pub struct StateHasher {
    outline: Option<Outline>,
    /// Ascending by object count; at most the last one is off the spacing grid.
    resume: Vec<Resume>,
}

/// The canonical bytes around the entries of the `objects` member.
#[derive(Debug, Clone)]
struct Outline {
    head: Vec<u8>,
    tail: Vec<u8>,
}

/// Hash state after the head and the first `count` object entries.
#[derive(Debug, Clone)]
struct Resume {
    last: Id,
    count: usize,
    bytes: usize,
    hasher: blake3::Hasher,
}

impl StateHasher {
    /// Objects with this ID or a later one were created, changed or removed;
    /// nothing else in the project changed.
    pub fn objects_changed_from(&mut self, first: &Id) {
        let keep = self.resume.partition_point(|resume| resume.last < *first);
        self.resume.truncate(keep);
    }

    /// Something other than an object changed, or the project was replaced.
    pub fn reset(&mut self) {
        self.outline = None;
        self.resume.clear();
    }

    /// The state hash of `project` and the length of its canonical bytes.
    pub fn hash(&mut self, project: &Project) -> Result<(StateHash, usize), ModelError> {
        let outline = match self.outline.take() {
            Some(outline) => outline,
            None => {
                self.resume.clear();
                Outline::of(project)?
            }
        };
        let result = self.resume_from(&outline, project);
        self.outline = Some(outline);
        result
    }

    fn resume_from(
        &mut self,
        outline: &Outline,
        project: &Project,
    ) -> Result<(StateHash, usize), ModelError> {
        let known = self.resume.len();
        let (mut hasher, mut bytes, mut count, after) = match self.resume.last() {
            Some(resume) => (
                resume.hasher.clone(),
                resume.bytes,
                resume.count,
                Some(resume.last.clone()),
            ),
            None => {
                let mut hasher = blake3::Hasher::new();
                hasher.update(DOMAIN);
                hasher.update(&outline.head);
                (hasher, outline.head.len(), 0, None)
            }
        };
        let rest = match &after {
            Some(id) => (Bound::Excluded(id), Bound::Unbounded),
            None => (Bound::Unbounded, Bound::Unbounded),
        };
        let mut entry = Vec::new();
        let mut last = None;
        for (id, object) in project.objects.range::<Id, _>(rest) {
            entry.clear();
            write_entry(&mut entry, id, object, count == 0)?;
            hasher.update(&entry);
            bytes += entry.len();
            count += 1;
            if count.is_multiple_of(SPACING) {
                self.resume.push(Resume {
                    last: id.clone(),
                    count,
                    bytes,
                    hasher: hasher.clone(),
                });
            }
            last = Some(id);
        }
        if let Some(id) = last {
            // The state after the newest object is where an append resumes.
            if known > 0 && !self.resume[known - 1].count.is_multiple_of(SPACING) {
                self.resume.remove(known - 1);
            }
            if !count.is_multiple_of(SPACING) {
                self.resume.push(Resume {
                    last: id.clone(),
                    count,
                    bytes,
                    hasher: hasher.clone(),
                });
            }
        }
        hasher.update(&outline.tail);
        Ok((
            StateHash::from_digest(hasher.finalize()),
            bytes + outline.tail.len(),
        ))
    }
}

/// `Project` with the `objects` member left empty. Listing every field makes a
/// new project field a compile error here rather than a silently wrong hash.
#[derive(Serialize)]
struct Shell<'a> {
    id: &'a Id,
    title: &'a str,
    created_by: &'a DeviceId,
    schema_version: u32,
    documents: &'a BTreeMap<Id, crate::Document>,
    assets: &'a BTreeMap<AssetId, v1::AddAsset>,
    layers: &'a BTreeMap<Id, crate::Layer>,
    objects: BTreeMap<Id, Object>,
    groups: &'a BTreeMap<Id, crate::Group>,
    instructions: &'a BTreeMap<Id, crate::Instruction>,
    semantic_snapshots: &'a BTreeMap<Id, crate::SemanticSnapshot>,
    results: &'a BTreeMap<Id, crate::ResultCandidate>,
    mask_versions: &'a BTreeMap<Id, Vec<v1::MaskOp>>,
}

impl Outline {
    fn of(project: &Project) -> Result<Self, ModelError> {
        let Project {
            id,
            title,
            created_by,
            schema_version,
            documents,
            assets,
            layers,
            objects: _,
            groups,
            instructions,
            semantic_snapshots,
            results,
            mask_versions,
        } = project;
        let mut value = serde_json::to_value(Shell {
            id,
            title,
            created_by,
            schema_version: *schema_version,
            documents,
            assets,
            layers,
            objects: BTreeMap::new(),
            groups,
            instructions,
            semantic_snapshots,
            results,
            mask_versions,
        })?;
        normalize(&mut value);
        let serde_json::Value::Object(members) = value else {
            return Err(ModelError::Invalid("canonical project outline"));
        };
        // Write the members exactly as serde_json writes the whole value, and
        // split inside the braces of `objects`.
        let mut head = vec![b'{'];
        let mut tail = Vec::new();
        let mut split = false;
        for (index, (name, member)) in members.iter().enumerate() {
            let out = if split { &mut tail } else { &mut head };
            if index > 0 {
                out.push(b',');
            }
            serde_json::to_writer(&mut *out, name)?;
            out.push(b':');
            if name == "objects" {
                out.push(b'{');
                tail.push(b'}');
                split = true;
            } else {
                serde_json::to_writer(&mut *out, member)?;
            }
        }
        if !split {
            return Err(ModelError::Invalid("canonical project outline"));
        }
        tail.push(b'}');
        Ok(Self { head, tail })
    }
}

/// One `"id":{object}` member, preceded by a comma unless it is the first.
fn write_entry(out: &mut Vec<u8>, id: &Id, object: &Object, first: bool) -> Result<(), ModelError> {
    if !first {
        out.push(b',');
    }
    serde_json::to_writer(&mut *out, id)?;
    out.push(b':');
    let mut value = serde_json::to_value(object)?;
    normalize(&mut value);
    serde_json::to_writer(&mut *out, &value)?;
    Ok(())
}
