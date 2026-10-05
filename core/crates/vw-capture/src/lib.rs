//! Explicit-action platform collection. No capture or global hook starts on load.
mod admission;
mod alpha_diagnostic;
pub use alpha_diagnostic::{AlphaRegion, AlphaSummary};
mod canvas;
pub use canvas::{CanvasCaptureRequest, CanvasFrameReceipt};
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
    // Harness-only, explicit opt-in. False preserves the exact legacy request.
    #[serde(default, skip_serializing_if = "is_false")]
    pub diagnostic_color_stage: bool,
    // Separate owned-fixture opt-in; only a relative freshly bound canvas region.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic_alpha_region: Option<AlphaRegion>,
}
fn is_false(value: &bool) -> bool {
    !*value
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
    CanvasFixture(CanvasCaptureRequest),
    Tree(TreeRequest),
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    Frame(FrameReceipt),
    CanvasFixture(CanvasFrameReceipt),
    Tree(TreeReceipt),
    Refused {
        error: Error,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color_depth: Option<ColorDepthDiagnostic>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alpha_summary: Option<AlphaSummary>,
    },
}
/// Closed, bounded numeric facts only. No monitor/device/window identifiers,
/// pixel RGB, coordinates, arbitrary provider text or acceptance authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case", deny_unknown_fields)]
pub enum ColorDepthDiagnostic {
    MonitorOutputMissing {
        enumerated_outputs: u32,
    },
    MonitorColorSpace {
        color_space: i32,
    },
    TextureDescriptor {
        width: u32,
        height: u32,
        format: i32,
        array_size: u32,
        mip_levels: u32,
        sample_count: u32,
    },
    NonopaquePixel {
        alpha: u8,
    },
}
#[cfg(test)]
mod color_diagnostic_tests {
    use super::*;
    #[test]
    fn legacy_refusal_bytes_and_request_optout_stay_exact() -> Result<()> {
        let value = Response::Refused {
            error: Error::ColorDepth,
            color_depth: None,
            alpha_summary: None,
        };
        let bytes = serde_json::to_vec(&value).map_err(|_| Error::Invalid)?;
        assert_eq!(bytes, br#"{"result":"refused","error":"color_depth"}"#);
        let baseline = br#"{"explicit_owner_action":true,"owner_process_id":99,"target":{"window":1,"process_id":7,"process_created":1,"client":{"x":0,"y":0,"width":80,"height":80},"frame":{"x":0,"y":0,"width":80,"height":80},"dpi":96,"observed_ns":1000},"capture_session_id":"00000000-0000-7000-8000-000000000001","output_directory":"C:\\Temp","limits":{"memory_bytes":268435456,"png_bytes":67108864,"max_pixels":50000000,"max_elements":4096,"max_text_bytes":2097152,"tree_ms":300,"capture_ms":3000}}"#;
        let request: CaptureRequest =
            serde_json::from_slice(baseline).map_err(|_| Error::Invalid)?;
        assert!(!request.diagnostic_color_stage);
        assert!(request.diagnostic_alpha_region.is_none());
        request.target.validate(request.owner_process_id)?;
        assert_eq!(
            serde_json::to_vec(&request).map_err(|_| Error::Invalid)?,
            baseline
        );
        Ok(())
    }
    #[test]
    fn opted_in_closed_numeric_stage_roundtrips_without_changing_refusal() -> Result<()> {
        for stage in [
            ColorDepthDiagnostic::MonitorOutputMissing {
                enumerated_outputs: 2,
            },
            ColorDepthDiagnostic::MonitorColorSpace { color_space: 12 },
            ColorDepthDiagnostic::TextureDescriptor {
                width: 800,
                height: 600,
                format: 87,
                array_size: 1,
                mip_levels: 1,
                sample_count: 1,
            },
            ColorDepthDiagnostic::NonopaquePixel { alpha: 0 },
        ] {
            let bytes = serde_json::to_vec(&Response::Refused {
                error: Error::ColorDepth,
                color_depth: Some(stage),
                alpha_summary: None,
            })
            .map_err(|_| Error::Invalid)?;
            assert!(bytes.len() < 256);
            let actual: Response = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
            assert!(
                matches!(actual, Response::Refused { error: Error::ColorDepth, color_depth: Some(value), alpha_summary: None } if value == stage)
            );
        }
        Ok(())
    }
    #[test]
    fn diagnostic_cannot_import_text_rgb_coordinates_or_unknown_stages() {
        for value in [
            r#"{"stage":"nonopaque_pixel","alpha":0,"rgb":"secret"}"#,
            r#"{"stage":"monitor_color_space","color_space":12,"monitor":"private"}"#,
            r#"{"stage":"nonopaque_pixel","alpha":0,"x":1}"#,
            r#"{"stage":"arbitrary_failure","message":"text"}"#,
            r#"{"stage":"nonopaque_pixel","alpha":256}"#,
        ] {
            assert!(serde_json::from_str::<ColorDepthDiagnostic>(value).is_err());
        }
    }
    #[test]
    fn r25_stage_bytes_are_unchanged_when_alpha_census_absent() -> Result<()> {
        let value = Response::Refused {
            error: Error::ColorDepth,
            color_depth: Some(ColorDepthDiagnostic::NonopaquePixel { alpha: 228 }),
            alpha_summary: None,
        };
        assert_eq!(serde_json::to_vec(&value).map_err(|_|Error::Invalid)?,br#"{"result":"refused","error":"color_depth","color_depth":{"stage":"nonopaque_pixel","alpha":228}}"#);
        Ok(())
    }
    #[test]
    fn relative_region_roundtrips_and_refuses_private_fields() -> Result<()> {
        let region = AlphaRegion {
            x: 2,
            y: 3,
            width: 4,
            height: 5,
        };
        let bytes = serde_json::to_vec(&region).map_err(|_| Error::Invalid)?;
        assert_eq!(bytes, br#"{"x":2,"y":3,"width":4,"height":5}"#);
        assert!(
            serde_json::from_str::<AlphaRegion>(
                r#"{"x":2,"y":3,"width":4,"height":5,"window":99}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<AlphaRegion>(r#"{"x":-1,"y":3,"width":4,"height":5}"#).is_err()
        );
        Ok(())
    }
}
