//! Local Android tunnel. Root's compiled package policy decides availability;
//! USB selection and control confirmation are explicit local-owner actions.
mod api;
mod contract;
mod platform;
pub use api::*;
mod worker;
use std::sync::Arc;

#[derive(Clone, Debug, thiserror::Error, uniffi::Error)]
pub enum AndroidTunnelError {
    #[error("invalid tunnel input")]
    Invalid,
    #[error("explicit local owner action required")]
    Consent,
    #[error("USB selection is ambiguous")]
    AmbiguousDevice,
    #[error("selected USB device is unauthorized")]
    UnauthorizedDevice,
    #[error("Android tunnel runtime is not admitted in this package")]
    RuntimeUnavailable,
    #[error("capture would include an owned Android mirror")]
    FeedbackLoop,
    #[error("tunnel source changed")]
    Stale,
    #[error("tunnel capacity exhausted")]
    Limit,
    #[error("tunnel operation cancelled")]
    Cancelled,
    #[error("tunnel operation timed out")]
    Timeout,
    #[error("tunnel storage failed")]
    Storage,
    #[error("tunnel platform operation failed")]
    Platform,
    #[error("tunnel retirement remains pending")]
    RetirementPending,
    #[error("tunnel closed")]
    Closed,
}
pub type AndroidTunnelResult<T> = std::result::Result<T, AndroidTunnelError>;
#[derive(Clone, Debug, uniffi::Record)]
pub struct AndroidTunnelOptions {
    pub explicit_owner_action: bool,
    pub runtime_directory: String,
    pub owner_adb_path: String,
    pub selected_model: String,
    pub selected_transport_id: u32,
    pub session_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum AndroidTunnelMode {
    ViewOnly,
    OwnerControl,
    RetirementPending,
    Closed,
}
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AndroidTunnelState {
    pub mode: AndroidTunnelMode,
    pub mirror_revision: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AndroidTunnelTarget {
    pub model: String,
    pub transport_id: u32,
    pub mirror_revision: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum AndroidTunnelHierarchy {
    Present,
    Missing,
    Stale,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AndroidTunnelElement {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub class_name: String,
    pub resource_id: String,
    pub description: String,
    pub text: String,
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum AndroidTunnelSource {
    Window {
        window: u64,
    },
    Monitor {
        monitor: u64,
    },
    Region {
        monitor: u64,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AndroidTunnelCaptureInfo {
    pub capture_session_id: String,
    pub source_generation: u64,
    pub frame_id: u64,
    pub width: u32,
    pub height: u32,
    pub encoded_bytes: u64,
    pub asset_id: String,
    pub image_started_ms: u64,
    pub image_finished_ms: u64,
    pub image_finished_timestamp_ns: u64,
    pub hierarchy: AndroidTunnelHierarchy,
    pub hierarchy_age_ms: Option<u64>,
    pub hierarchy_started_ms: Option<u64>,
    pub hierarchy_finished_ms: Option<u64>,
}
