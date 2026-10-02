use crate::{AssetId, DeviceId, Id, ModelError, StateHash};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use vw_proto::v1;

/// Current editable project state. Histories, tombstones, conflicts, device-local
/// views, caches and transport queues are separate from this canonical hash.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub id: Id,
    pub title: String,
    pub created_by: DeviceId,
    pub schema_version: u32,
    pub documents: BTreeMap<Id, Document>,
    pub assets: BTreeMap<AssetId, v1::AddAsset>,
    pub layers: BTreeMap<Id, Layer>,
    pub objects: BTreeMap<Id, Object>,
    pub groups: BTreeMap<Id, Group>,
    pub instructions: BTreeMap<Id, Instruction>,
    pub semantic_snapshots: BTreeMap<Id, SemanticSnapshot>,
    pub results: BTreeMap<Id, ResultCandidate>,
    pub mask_versions: BTreeMap<Id, Vec<v1::MaskOp>>,
}

/// Versioned document definition; capture metadata remains attached to captures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub definition: v1::CreateDocument,
    pub pages: Vec<PdfPage>,
    pub created_at_ms: i64,
}

/// A PDF page uses a separate document space measured in PDF points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PdfPage {
    pub page_index: u32,
    pub crop_box: v1::RectD,
    pub rotation_degrees: i32,
}

/// Layer metadata with creation defaults made explicit in canonical state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub definition: v1::CreateLayer,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f64,
    pub blend: String,
}

/// One editable annotation; `state.shape` represents every specified object kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    pub document_id: Id,
    pub state: v1::ObjectState,
}

/// A logical grouping container; parent chains must be acyclic within a document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub id: Id,
    pub document_id: Id,
    pub parent: Option<Id>,
}

/// Instruction metadata, including the authoritative last write time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instruction {
    pub definition: v1::SetInstruction,
    pub updated_at_ms: i64,
}

/// Compressed semantic payload bound to its capture; captured text is always
/// untrusted. Decompression and accessibility redaction belong to capture import.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticSnapshot {
    pub definition: v1::AddSemanticSnapshot,
    pub capture_session_id: Option<Id>,
    pub frame_id: Option<u64>,
    pub created_at_ms: i64,
}

impl SemanticSnapshot {
    /// Captured element names/text can never become agent instructions implicitly.
    pub const fn text_is_untrusted(&self) -> bool {
        true
    }
}

/// A model result remains a candidate until explicitly accepted by a later op.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultCandidate {
    pub definition: v1::AddResult,
    pub status: String,
    pub acceptance_mask_asset_id: Option<AssetId>,
    pub created_at_ms: i64,
}

impl Project {
    /// Create an empty version-1 project with no device-local view state.
    pub fn new(id: Id, title: String, created_by: DeviceId) -> Self {
        Self {
            id,
            title,
            created_by,
            schema_version: 1,
            documents: BTreeMap::new(),
            assets: BTreeMap::new(),
            layers: BTreeMap::new(),
            objects: BTreeMap::new(),
            groups: BTreeMap::new(),
            instructions: BTreeMap::new(),
            semantic_snapshots: BTreeMap::new(),
            results: BTreeMap::new(),
            mask_versions: BTreeMap::new(),
        }
    }
    /// Canonical ordered JSON, schema-bound and domain-separated before hashing.
    /// Validation rejects nonfinite floats before serde could turn them into null.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ModelError> {
        self.validate()?;
        let mut value = serde_json::to_value(self)?;
        normalize(&mut value);
        Ok(serde_json::to_vec(&value)?)
    }
    /// Visible state hash excludes revision counters, so undo can restore a hash
    /// while its newly appended transaction still receives a new host sequence.
    pub fn state_hash(&self) -> Result<StateHash, ModelError> {
        let mut bytes = b"VisualWorkbench.Project.v1\0".to_vec();
        bytes.extend(self.canonical_bytes()?);
        Ok(StateHash::hash(&bytes))
    }
    /// Decode a bounded snapshot and validate all references and supported kinds.
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, ModelError> {
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(ModelError::Invalid("snapshot size"));
        }
        let value: Self = serde_json::from_slice(bytes)?;
        value.validate()?;
        Ok(value)
    }
}

// serde_json's default object map is ordered. Normalize signed floating zero,
// retaining integer precision; normal floating values use its stable formatter.
fn normalize(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Number(number) if number.is_f64() && number.as_f64() == Some(0.0) => {
            *value = serde_json::Value::Number(0.into());
        }
        serde_json::Value::Array(values) => {
            for value in values {
                normalize(value);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values_mut() {
                normalize(value);
            }
        }
        _ => {}
    }
}
