use crate::{ProjectInfo, SelectionVersion, WorkflowBinding, WorkflowMetadata};

#[derive(Clone, Debug, thiserror::Error, uniffi::Error)]
pub enum AiEditError {
    #[error("invalid AI edit request or configuration")]
    Invalid,
    #[error("the document, mask, configuration or review changed")]
    Stale,
    #[error("AI work exceeds its memory or work budget")]
    Limit,
    #[error("source depth reduction needs explicit permission")]
    Depth,
    #[error("source color handling needs explicit permission or is unsupported")]
    Color,
    #[error("a current, provenance-bound token estimate is required before Send")]
    Estimate,
    #[error("the daily soft budget needs explicit acknowledgement")]
    SoftBudget,
    #[error("an uncertain earlier attempt blocks a new request; do not reset history")]
    Unresolved,
    #[error("this request has already consumed its Send action")]
    AttemptConsumed,
    #[error("protected credentials or platform trust are unavailable")]
    Credentials,
    #[error("the provider did not return a complete valid response; charge may be uncertain")]
    Provider,
    #[error("immutable image data or proof does not match")]
    Proof,
    #[error("private storage or publication failed")]
    Storage,
    #[error("this source or platform is unsupported")]
    Unsupported,
    #[error("AI work was cancelled; an attempted request may still be billed")]
    Cancelled,
    #[error("the AI owner or project is closed")]
    Closed,
    #[error("bounded AI worker capacity is occupied")]
    Busy,
}
pub type AiEditResult<T> = Result<T, AiEditError>;

#[derive(Clone, Debug, uniffi::Record)]
pub struct AiConfiguration {
    /// Versioned JSON configuration, including dated provenance. No credentials.
    pub json: String,
    pub fingerprint: String,
    pub verified_on: String,
    pub expires_on: String,
    pub model: String,
    pub quality: String,
    pub daily_soft_budget_microusd: u64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AiTokenEstimate {
    pub schema: u32,
    pub text_input: u64,
    pub image_input: u64,
    pub image_output: u64,
    /// Owner supplied estimate provenance, never a claimed invoice or cap.
    pub provenance: String,
    pub verified_on: String,
    pub expires_on: String,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AiPrepareOptions {
    pub binding: WorkflowBinding,
    pub selections: Vec<SelectionVersion>,
    pub intent_id: String,
    pub instruction: String,
    pub configuration_fingerprint: String,
    pub estimate: Option<AiTokenEstimate>,
    pub feather_px: u32,
    pub allow_16bit_provider_copy: bool,
    pub assume_untagged_srgb: bool,
    pub memory_budget_bytes: u64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AiReview {
    pub request_id: String,
    pub binding: WorkflowBinding,
    pub source_asset_id: String,
    pub source_file_sha256: String,
    pub mask_sha256: String,
    pub configuration_fingerprint: String,
    pub model: String,
    pub quality: String,
    pub source_width: u32,
    pub source_height: u32,
    pub source_bit_depth: u8,
    pub model_width: u32,
    pub model_height: u32,
    pub crop_x: i32,
    pub crop_y: i32,
    pub crop_width: u32,
    pub crop_height: u32,
    pub feather_px: u32,
    pub estimated_microusd: Option<u64>,
    pub estimate_provenance: Option<String>,
    pub estimate_expires_on: Option<String>,
    pub configuration_expires_on: String,
    pub provider_copy_reduces_depth: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AiReadiness {
    pub configured: bool,
    pub trust_ready: bool,
    pub configuration_current: bool,
    pub spent_today_microusd: u64,
    pub daily_soft_budget_microusd: u64,
}
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum AiSettlement {
    UsagePriced,
    UsageMissing,
    LedgerUnavailable,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AiProofInfo {
    pub changed_outside: u64,
    pub outside_sha256_before: String,
    pub outside_sha256_after: String,
    pub changed_unaccepted: Option<u64>,
    pub unaccepted_sha256_before: Option<String>,
    pub unaccepted_sha256_after: Option<String>,
    pub delta_e2000_mean: f64,
    pub delta_e2000_max: f64,
    pub ssim_inside_mask: f64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AiCandidateInfo {
    pub request_id: String,
    pub candidate_id: String,
    pub binding: WorkflowBinding,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub partial: bool,
    pub settlement: AiSettlement,
    pub actual_microusd: Option<u64>,
    pub proof: AiProofInfo,
}
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum AiCompareMode {
    Before,
    After,
    Wipe {
        vertical: bool,
        cut: u32,
        result_before: bool,
    },
    Split,
    Difference {
        gain: u8,
    },
}
#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct AiRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AiPixels {
    pub candidate_id: String,
    pub region: AiRegion,
    pub canvas_width: u32,
    pub canvas_height: u32,
    pub rgba_srgb: Vec<u8>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AiAcceptanceBrush {
    pub expected_candidate_id: String,
    pub next_candidate_id: String,
    pub points: Vec<crate::Point>,
    pub radius: f64,
    pub opacity: u8,
    pub subtract: bool,
    pub clear_first: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AiResultPixels {
    pub binding: WorkflowBinding,
    pub result_id: String,
    pub source_asset_id: String,
    pub composite_asset_id: String,
    pub region: AiRegion,
    pub canvas_width: u32,
    pub canvas_height: u32,
    pub rgba_srgb: Vec<u8>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AiSaveOptions {
    pub expected: WorkflowBinding,
    pub metadata: WorkflowMetadata,
    pub result_id: String,
    pub layer_id: String,
    pub object_id: String,
    /// Replaces only this prior AI Result object, atomically. Its immutable
    /// candidate/assets remain in history and Undo restores it.
    pub replace_object_id: Option<String>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AiResultReceipt {
    pub transaction_id: String,
    pub result_id: String,
    pub object_id: String,
    pub layer_id: String,
    pub output_asset_id: String,
    pub composite_asset_id: String,
    pub acceptance_mask_asset_id: Option<String>,
    pub revision: ProjectInfo,
    pub proof: AiProofInfo,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct AiInstructionSummary {
    pub id: String,
    pub role: String,
    pub text: String,
    pub target_object_ids: Vec<String>,
    pub marker_numbers: Vec<u32>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AiContext {
    pub binding: WorkflowBinding,
    pub source_asset_id: String,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    pub change_selections: Vec<SelectionVersion>,
    pub instructions: Vec<AiInstructionSummary>,
}
