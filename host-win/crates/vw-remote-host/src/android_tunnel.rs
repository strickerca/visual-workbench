//! Owner-scoped Android mirror and paired-capture producer.
//!
//! This module is deliberately not registered until root installs its exact-byte
//! runtime/license admission. No public value or hash string can enable launch.
#[path = "android_tunnel/capture.rs"]
mod capture;
#[path = "android_tunnel/control.rs"]
mod control;
#[path = "android_tunnel/device.rs"]
mod device;
#[path = "android_tunnel/mirror.rs"]
mod mirror;
#[path = "android_tunnel/session.rs"]
mod session;
#[cfg(test)]
#[path = "android_tunnel/tests.rs"]
mod tests;
#[cfg(windows)]
#[path = "android_tunnel/windows.rs"]
mod windows;

pub use capture::{CapturePair, Hierarchy, PngImage, SemanticElement};
pub use control::{
    ControlChallenge, InteractionMode, InteractionState, OwnerControlPermit, OwnerTargetDescriptor,
};
pub use device::{DeviceToken, UsbSelection};
pub use mirror::{CaptureSource, PhysicalRect};
pub use session::{
    AdmittedRuntime, AndroidTunnelOwner, Cancellation, OwnerAdb, RuntimeAdmission,
    retry_retirements,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Invalid,
    Consent,
    AmbiguousDevice,
    UnauthorizedDevice,
    RuntimeUnavailable,
    FeedbackLoop,
    Stale,
    Limit,
    Cancelled,
    Timeout,
    Storage,
    Platform,
    RetirementPending,
}

pub type Result<T> = std::result::Result<T, Error>;
