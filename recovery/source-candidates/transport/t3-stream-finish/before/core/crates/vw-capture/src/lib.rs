//! Explicit-action platform collection. No capture or global hook starts on load.
mod admission;
mod fixed_target;
pub use fixed_target::refreshed_target;
mod image;
pub mod process;
#[cfg(windows)]
pub mod windows;
pub use admission::*;
pub use image::*;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Error {
    #[error("invalid capture request")]
    Invalid,
    #[error("capture requires an explicit owner action")]
    Consent,
    #[error("capture source changed")]
    Stale,
    #[error("the app's own window cannot be captured")]
    OwnWindow,
    #[error("capture source is unavailable or protected")]
    Unavailable,
    #[error("capture exceeds its resource budget")]
    Limit,
    #[error("unsupported source color or pixel depth")]
    ColorDepth,
    #[error("platform codec is not installed")]
    CodecMissing,
    #[error("capture timed out")]
    Timeout,
    #[error("capture cancelled")]
    Cancelled,
    #[error("capture capacity exhausted")]
    Busy,
    #[error("capture storage failed")]
    Storage,
    #[error("platform capture API failed")]
    Platform,
    #[error("platform is unsupported")]
    Unsupported,
}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Default)]
pub struct Cancellation(AtomicBool);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn check(&self) -> Result<()> {
        if self.0.load(Ordering::Acquire) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowTarget {
    pub window: u64,
    pub process_id: u32,
    pub process_created: u64,
    pub client: Rect,
    pub frame: Rect,
    pub dpi: u32,
    pub observed_ns: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameIdentity {
    pub capture_session_id: String,
    pub frame_id: u64,
    pub geometry_revision: u32,
    pub source_kind: String,
    pub client_rect: Rect,
    pub dpi_scale: f64,
    pub monotonic_timestamp_ns: u64,
    pub captured_at_ms: i64,
    pub platform: String,
    pub window_handle: u64,
    pub monitor_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameReceipt {
    pub identity: FrameIdentity,
    pub target: WindowTarget,
    pub source_asset_id: String,
    pub png_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub border_visible: bool,
    pub lossless: bool,
    pub filename: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Element {
    pub local_id: String,
    pub parent_local_id: Option<String>,
    pub name: String,
    pub role: String,
    pub automation_id: Option<String>,
    pub resource_id: Option<String>,
    pub html_id: Option<String>,
    pub bounds: [f64; 4],
    pub text: String,
    pub enabled: bool,
    pub focused: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeReceipt {
    pub identity: FrameIdentity,
    pub source_asset_id: String,
    pub platform: String,
    pub frame_delta_ms: i32,
    pub collection_elapsed_ms: u64,
    pub elements: Vec<Element>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureRequest {
    pub explicit_owner_action: bool,
    pub owner_process_id: u32,
    pub target: WindowTarget,
    pub capture_session_id: String,
    pub output_directory: String,
    pub limits: Limits,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeRequest {
    pub frame: FrameReceipt,
    pub owner_process_id: u32,
    pub limits: Limits,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Capture(CaptureRequest),
    Tree(TreeRequest),
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    Frame(FrameReceipt),
    Tree(TreeReceipt),
    Refused { error: Error },
}
