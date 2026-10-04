//! Bounded local MCP IPC. OS-specific credentials and pipe ACLs are isolated.
#[cfg(windows)]
pub mod app_image;
#[cfg(windows)]
pub mod app_launch;
pub mod framing;
pub mod package_reader;
#[cfg(windows)]
pub mod windows;
pub mod write_deadline;
pub type Result<T> = std::result::Result<T, &'static str>;
pub const WIRE_BYTES: usize = 17 * 1024 * 1024;
pub const REQUEST_BYTES: usize = 6 * 1024 * 1024;
pub fn nonce() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| "entropy")?;
    Ok(bytes.iter().map(|x| format!("{x:02x}")).collect())
}
