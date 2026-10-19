//! Owner-scoped Android mirror and paired-capture producer.
//!
//! This module is deliberately not registered until root installs its exact-byte
//! runtime/license admission. No public value or hash string can enable launch.
mod capture;
mod device;
mod mirror;
mod session;
#[cfg(windows)]
mod windows;
#[cfg(test)]
mod tests;

pub use capture::{CapturePair, Hierarchy, PngImage, SemanticElement};
pub use device::{DeviceToken, UsbSelection};
pub use mirror::{CaptureSource, PhysicalRect};
pub use session::{AdmittedRuntime, AndroidTunnelOwner, Cancellation, OwnerAdb,
    RuntimeAdmission, retry_retirements};

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
