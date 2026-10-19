// Compile isolated fixtures before root registers the tunnel in the host crate.
// Windows uses the exact owned Pins implementation through a local module alias;
// no test invokes adb, scrcpy, a device, or a GUI.
#![allow(dead_code, unused_imports)]
#[cfg(windows)]
use vw_remote_host::process::Cancellation;
#[cfg(windows)]
use vw_remote_host::{Error, Result};
#[cfg(windows)]
#[path = "../src/process/windows.rs"]
pub(crate) mod source_pins;
#[cfg(windows)]
#[path = "../src/startup.rs"]
mod startup;
#[cfg(windows)]
mod process {
    pub(crate) use crate::source_pins as windows;
}
#[path = "../src/android_tunnel.rs"]
mod android_tunnel;
