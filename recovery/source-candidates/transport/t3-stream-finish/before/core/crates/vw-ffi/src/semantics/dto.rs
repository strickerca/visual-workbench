use crate::{Point, QueryRect, WorkflowBinding};
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum SemanticPlatform {
    Uia,
    AndroidAx,
    ChromiumUia,
}
impl SemanticPlatform {
    pub(super) fn core(self) -> vw_semantics::Platform {
        match self {
            Self::Uia => vw_semantics::Platform::Uia,
            Self::AndroidAx => vw_semantics::Platform::AndroidAx,
            Self::ChromiumUia => vw_semantics::Platform::ChromiumUia,
        }
    }
    pub(super) fn from_core(value: vw_semantics::Platform) -> Self {
        match value {
            vw_semantics::Platform::Uia => Self::Uia,
            vw_semantics::Platform::AndroidAx => Self::AndroidAx,
            vw_semantics::Platform::ChromiumUia => Self::ChromiumUia,
        }
    }
}
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum SemanticBoundsSpace {
    HostPhysical,
    CapturePixels,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct CapturedSemanticElement {
    pub local_id: String,
    pub parent_local_id: Option<String>,
    pub name: String,
    pub role: String,
    pub automation_id: Option<String>,
    pub resource_id: Option<String>,
    pub html_id: Option<String>,
    pub bounds: QueryRect,
    pub text: String,
    pub enabled: bool,
    pub focused: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct SemanticElement {
    pub eid: String,
    pub parent: Option<String>,
    pub name: String,
    pub role: String,
    pub automation_id: Option<String>,
    pub resource_id: Option<String>,
    pub html_id: Option<String>,
    pub bounds_document: QueryRect,
    pub bounds_clipped: bool,
    pub text: String,
    pub enabled: bool,
    pub focused: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct SemanticCaptureInfo {
    pub binding: WorkflowBinding,
    pub capture_session_id: String,
    pub frame_id: u64,
    pub geometry_revision: u32,
    pub source_asset_id: String,
    pub captured_at_ms: i64,
    pub width: f64,
    pub height: f64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct SemanticDocument {
    pub binding: WorkflowBinding,
    pub snapshot_id: String,
    pub platform: SemanticPlatform,
    pub frame_delta_ms: i32,
    pub collection_elapsed_ms: u64,
    pub elements: Vec<SemanticElement>,
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum SemanticSnapQuery {
    Point { point: Point },
    Box { bounds: QueryRect },
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct SemanticSnap {
    pub snapshot_id: String,
    pub binding: WorkflowBinding,
    pub eid: String,
    pub bounds_document: QueryRect,
    pub distance_screen_pixels: f64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct SemanticReference {
    pub platform: SemanticPlatform,
    pub eid: String,
    pub name: String,
    pub role: String,
    pub automation_id: Option<String>,
    pub resource_id: Option<String>,
    pub html_id: Option<String>,
    pub bounds_document: QueryRect,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct SemanticProjection {
    pub binding: WorkflowBinding,
    pub snapshot_id: String,
    pub references: Vec<SemanticReference>,
    pub semantic_json: Vec<u8>,
    pub quoted_prompt_data: String,
}
