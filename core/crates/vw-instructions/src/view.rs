use crate::{
    EntryMethod, Error, MAX_ELEMENT_ID_BYTES, MAX_ELEMENT_REFS, MAX_INSTRUCTIONS, MAX_MARKERS,
    MAX_TARGETS, MAX_TOTAL_TEXT_BYTES, Result, Role, language, text,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io::Write};
use vw_model::{DeviceId, Id, Instruction, Object, Project, StateHash};
use vw_proto::v1;

/// Full visible-state binding, including project and document identity. Host
/// sequence alone cannot distinguish optimistic edits at the same accepted head.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBinding {
    pub project_id: Id,
    pub document_id: Id,
    pub host_seq: u64,
    pub state_hash: StateHash,
}
impl SourceBinding {
    pub fn revision(&self) -> v1::Revision {
        v1::Revision {
            host_seq: self.host_seq,
            state_hash: self.state_hash.bytes().to_vec(),
        }
    }
}

/// Borrowed current state; no second authoritative instruction/marker database.
pub struct DocumentView<'a> {
    pub(crate) project: &'a Project,
    pub(crate) binding: SourceBinding,
    pub(crate) markers: Vec<(&'a Id, &'a Object, &'a v1::Marker)>,
    pub(crate) instructions: Vec<(&'a Id, &'a Instruction)>,
}
impl<'a> DocumentView<'a> {
    pub fn new(project: &'a Project, revision: &v1::Revision, document: &Id) -> Result<Self> {
        if !project.documents.contains_key(document) {
            return Err(Error::Missing);
        }
        // Bound traversal/serialization before canonical hashing allocates its
        // JSON projection. The input project remains owned by model/store.
        json_admission(project, 64 * 1024 * 1024)?;
        let mut markers = Vec::new();
        let mut element_bytes = 0usize;
        let mut object_count = 0usize;
        for (id, object) in &project.objects {
            if &object.document_id == document {
                object_count += 1;
                if object_count > crate::MAX_DOCUMENT_OBJECTS {
                    return Err(Error::Limit("document objects"));
                }
            }
            if &object.document_id == document
                && let Some(v1::object_state::Shape::Marker(marker)) = &object.state.shape
            {
                if markers.len() == MAX_MARKERS {
                    return Err(Error::Limit("markers"));
                }
                if marker.element_eids.len() > MAX_ELEMENT_REFS
                    || marker
                        .element_eids
                        .iter()
                        .any(|v| v.len() > MAX_ELEMENT_ID_BYTES || v.contains('\0'))
                {
                    return Err(Error::Limit("element references"));
                }
                for eid in &marker.element_eids {
                    element_bytes = element_bytes
                        .checked_add(eid.len())
                        .ok_or(Error::Limit("element references"))?;
                    if element_bytes > MAX_TOTAL_TEXT_BYTES {
                        return Err(Error::Limit("element references"));
                    }
                }
                markers.push((id, object, marker));
            }
        }
        markers.sort_by(|a, b| (a.2.number, a.0).cmp(&(b.2.number, b.0)));
        let mut instructions = Vec::new();
        let mut total = 0usize;
        for (id, instruction) in &project.instructions {
            if Id::from_proto(instruction.definition.document_id.as_ref())? != *document {
                continue;
            }
            if instructions.len() == MAX_INSTRUCTIONS {
                return Err(Error::Limit("instructions"));
            }
            validate_definition(&instruction.definition)?;
            total = total
                .checked_add(instruction.definition.text.len())
                .ok_or(Error::Limit("total text"))?;
            if total > MAX_TOTAL_TEXT_BYTES {
                return Err(Error::Limit("total text"));
            }
            instructions.push((id, instruction));
        }
        let hash = project.state_hash()?;
        if revision.state_hash != hash.bytes() {
            return Err(Error::Stale);
        }
        Ok(Self {
            project,
            binding: SourceBinding {
                project_id: project.id.clone(),
                document_id: document.clone(),
                host_seq: revision.host_seq,
                state_hash: hash,
            },
            markers,
            instructions,
        })
    }
    pub fn binding(&self) -> &SourceBinding {
        &self.binding
    }
    pub fn marker_ids(&self) -> impl Iterator<Item = &Id> {
        self.markers.iter().map(|v| v.0)
    }
    pub fn instruction_ids(&self) -> impl Iterator<Item = &Id> {
        self.instructions.iter().map(|v| v.0)
    }
    pub fn instruction(&self, id: &Id) -> Result<&Instruction> {
        self.instructions
            .iter()
            .find(|v| v.0 == id)
            .map(|v| v.1)
            .ok_or(Error::Missing)
    }
    pub fn object(&self, id: &Id) -> Result<&Object> {
        let object = self.project.objects.get(id).ok_or(Error::Missing)?;
        if object.document_id != self.binding.document_id {
            return Err(Error::Invalid("target document"));
        }
        Ok(object)
    }
    pub fn marker_instruction(&self, marker: &Id) -> Result<&Id> {
        if !self.markers.iter().any(|v| v.0 == marker) {
            return Err(Error::Missing);
        }
        let mut found = None;
        let target = marker.to_proto();
        for (id, instruction) in &self.instructions {
            if instruction.definition.target_object_ids.contains(&target) {
                if found.is_some() {
                    return Err(Error::Reconciliation);
                }
                found = Some(*id);
            }
        }
        found.ok_or(Error::Reconciliation)
    }
    pub fn detached_instruction_ids(&self) -> Result<Vec<Id>> {
        let mut detached = Vec::new();
        for (id, instruction) in &self.instructions {
            if instruction
                .definition
                .target_object_ids
                .iter()
                .any(|target| {
                    Id::from_proto(Some(target))
                        .is_ok_and(|id| !self.project.objects.contains_key(&id))
                })
            {
                detached.push((*id).clone());
            }
        }
        Ok(detached)
    }
    pub(crate) fn numbering(&self) -> Result<()> {
        if self
            .markers
            .iter()
            .enumerate()
            .any(|(i, v)| v.2.number as usize != i + 1)
        {
            return Err(Error::Reconciliation);
        }
        Ok(())
    }
    pub(crate) fn editable(&self, id: &Id) -> Result<()> {
        let object = self.object(id)?;
        self.editable_layer(&Id::from_proto(object.state.layer_id.as_ref())?)?;
        if object.state.locked {
            return Err(Error::Locked);
        }
        Ok(())
    }
    pub(crate) fn editable_layer(&self, id: &Id) -> Result<()> {
        let layer = self.project.layers.get(id).ok_or(Error::Missing)?;
        if Id::from_proto(layer.definition.document_id.as_ref())? != self.binding.document_id {
            return Err(Error::Invalid("layer document"));
        }
        if layer.locked {
            return Err(Error::Locked);
        }
        Ok(())
    }
    pub(crate) fn fresh_id(&self, id: &Id) -> Result<()> {
        if self.project.objects.contains_key(id)
            || self.project.instructions.contains_key(id)
            || self.project.documents.contains_key(id)
            || self.project.layers.contains_key(id)
            || self.project.groups.contains_key(id)
            || self.project.results.contains_key(id)
            || self.project.semantic_snapshots.contains_key(id)
            || &self.project.id == id
        {
            return Err(Error::Invalid("identifier already exists"));
        }
        Ok(())
    }
}
pub(crate) fn validate_definition(value: &v1::SetInstruction) -> Result<()> {
    Id::from_proto(value.instruction_id.as_ref())?;
    Id::from_proto(value.document_id.as_ref())?;
    Role::from_canonical(value.role)?;
    EntryMethod::from_canonical(&value.entry_method)?;
    text(&value.text)?;
    language(&value.language)?;
    if value.target_object_ids.len() > MAX_TARGETS {
        return Err(Error::Limit("instruction targets"));
    }
    let mut targets = BTreeSet::new();
    for target in &value.target_object_ids {
        if !targets.insert(Id::from_proto(Some(target))?) {
            return Err(Error::Invalid("duplicate instruction target"));
        }
    }
    Ok(())
}
pub(crate) fn json_admission(value: &impl Serialize, limit: usize) -> Result<()> {
    let mut sink = CountWriter { bytes: 0, limit };
    serde_json::to_writer(&mut sink, value).map_err(|_| Error::Limit("serialized projection"))
}
struct CountWriter {
    bytes: usize,
    limit: usize,
}
impl Write for CountWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("size"))?;
        if self.bytes > self.limit {
            return Err(std::io::Error::other("size"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Host authentication is separate from authored instruction text.
pub(crate) fn authenticate(author: &str, authenticated: &DeviceId) -> Result<()> {
    if author != authenticated.as_str() {
        return Err(Error::Unauthorized);
    }
    Ok(())
}
