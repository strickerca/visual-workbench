use crate::Target;
use serde::{Deserialize, Serialize};
use vw_model::Id;

pub use crate::geometry::ImageMapping;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: String,
    pub package_id: Id,
    pub created_at: String,
    pub source: Source,
    pub compiled_for: CompiledFor,
    pub images: Vec<PackageImage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_instruction: Option<String>,
    pub constraints: Vec<String>,
    pub markers: Vec<PackageMarker>,
    pub files: Vec<FileEntry>,
    pub semantic_file: String,
    pub prompt_file: String,
    pub untrusted_text_notice: String,
    pub redactions: Vec<Redaction>,
    pub extensions: Extensions,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub project_id: Id,
    pub document_id: Id,
    pub document_kind: String,
    pub revision: String,
    pub asset_hash: String,
    pub width: u32,
    pub height: u32,
    pub units: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture: Option<Capture>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    pub platform: String,
    pub session_id: Id,
    pub frame_id: u64,
    pub app_name: String,
    pub window_title: Option<String>,
    pub captured_at: String,
    pub lossless: bool,
    pub degraded: bool,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledFor {
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub max_long_edge: u32,
    pub coordinate_convention: String,
    pub scale: f64,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageImage {
    pub id: String,
    pub role: String,
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub sha256: String,
    pub color_space: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker: Option<u32>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageMarker {
    pub number: u32,
    pub kind: String,
    pub role: String,
    pub instruction: String,
    pub bbox_compiled: [f64; 4],
    pub bbox_document: [f64; 4],
    pub point_compiled: [f64; 2],
    pub point_document: [f64; 2],
    pub element_refs: Vec<ElementReference>,
    pub crop_image: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElementReference {
    pub platform: String,
    pub eid: String,
    pub name: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html_id: Option<String>,
    pub bounds_document: [f64; 4],
}
impl From<&vw_semantics::ElementReference> for ElementReference {
    fn from(v: &vw_semantics::ElementReference) -> Self {
        Self {
            platform: v.platform.as_str().into(),
            eid: v.eid.clone(),
            name: v.name.clone(),
            role: v.role.clone(),
            automation_id: v.automation_id.clone(),
            resource_id: v.resource_id.clone(),
            html_id: v.html_id.clone(),
            bounds_document: v.bounds_document,
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Redaction {
    pub field: String,
    pub reason: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extensions {
    pub compiler: String,
    pub state_hash: String,
    pub host_seq: u64,
    pub target_profile: Target,
    pub source_bit_depth: u8,
    pub derivative_bit_depth: u8,
    pub color_conversion: String,
    pub overview_mapping: ImageMapping,
    pub crops: Vec<CropMapping>,
    pub semantic_snapshot: Option<Id>,
    pub instructions: Vec<InstructionBinding>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CropMapping {
    pub marker: u32,
    pub image: String,
    pub mapping: ImageMapping,
    pub bbox_crop_pixels: [f64; 4],
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionBinding {
    pub instruction_id: Id,
    pub target_ids: Vec<Id>,
    pub entry_method: String,
    pub language: String,
    pub updated_at_ms: i64,
}
