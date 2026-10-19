// Compile isolated fixtures on non-Windows before root registers the library.
// On Windows, android_tunnel's registered #[cfg(test)] module runs them in the
// actual host crate, where crate::process::windows::Pins exists.
#![cfg(not(windows))]
#![allow(dead_code, unused_imports)]
#[path = "../src/android_tunnel.rs"]
mod android_tunnel;
