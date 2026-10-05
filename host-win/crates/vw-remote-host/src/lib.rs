//! Production remote helpers. No MFT or UIA provider loads in the JVM.
#[cfg(any(windows, test))]
mod capture_recovery;
#[cfg(any(windows, test))]
mod contact;
#[cfg(windows)]
pub mod editor_effect;
#[cfg(windows)]
pub mod editor_probe;
#[cfg(windows)]
pub mod editor_probe_runner;
#[cfg(windows)]
pub mod editor_source;
#[cfg(any(windows, test))]
mod finite_state;
#[cfg(windows)]
pub mod hil;
#[cfg(windows)]
pub mod platform;
pub mod process;
mod retirement;
#[cfg(any(windows, test))]
mod startup;
pub use vw_remote::{Error, Result};

#[cfg(windows)]
pub mod picker;
