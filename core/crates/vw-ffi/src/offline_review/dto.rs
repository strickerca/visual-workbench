//! Proposed additive ABI. Root owns registration and generated bindings.
use crate::{ProjectInfo, WorkflowReceipt};

#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct OfflineReviewBinding {
    pub project_id: String,
    pub host_seq: u64,
    pub state_hash: String,
}
#[derive(Clone, uniffi::Record)]
pub struct OfflineReviewIndex {
    pub row: i64,
    pub conflict_id: String,
    pub property: String,
    pub delete_vs_edit: bool,
}
#[derive(Clone, uniffi::Record)]
pub struct OfflineReviewPage {
    pub binding: OfflineReviewBinding,
    pub entries: Vec<OfflineReviewIndex>,
}
#[derive(Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum OfflineReviewProgress {
    Unreviewed,
    Kept,
    PreviouslyResolved,
    Draft,
    Queued,
    Applied,
}
#[derive(Clone, uniffi::Enum)]
pub enum OfflineReviewChoice {
    SwapOther,
    RestoreCopy {
        document_id: String,
        object_id: String,
    },
}
/// Explicit private display text; never diagnostics. UTF-8 prefix, up to 4096
/// bytes, plus exact length/hash. A truncated value cannot authorize submission.
#[derive(Clone, uniffi::Record)]
pub struct OfflineReviewValue {
    pub preview: String,
    pub utf8_bytes: u64,
    pub blake3: Option<String>,
    pub truncated: bool,
    pub available: bool,
}
#[derive(Clone, uniffi::Record)]
pub struct OfflineReviewDetails {
    pub binding: OfflineReviewBinding,
    pub conflict_id: String,
    pub object_id: String,
    pub property: String,
    pub delete_vs_edit: bool,
    pub kept: OfflineReviewValue,
    pub other: OfflineReviewValue,
    pub current: OfflineReviewValue,
    pub operation: Option<OfflineReviewValue>,
    pub transaction_id: Option<String>,
    pub reviewed: Option<OfflineReviewBinding>,
    pub progress: OfflineReviewProgress,
    pub next_lamport: u64,
}
/// The normal commit receipt establishes durable state even if settling the
/// separate local review marker failed. Refresh reconciles this status.
#[derive(Clone, uniffi::Record)]
pub struct OfflineReviewSubmission {
    pub receipt: WorkflowReceipt,
    pub progress: OfflineReviewProgress,
    pub metadata_pending: bool,
}
#[derive(Clone, uniffi::Record)]
pub struct OfflineReviewMutation {
    pub details: OfflineReviewDetails,
    pub project: ProjectInfo,
}
