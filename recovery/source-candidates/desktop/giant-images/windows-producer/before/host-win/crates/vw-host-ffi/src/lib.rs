//! Windows-only host services. UniFFI futures enqueue bounded work; PNG parsing,
//! Win32 calls and clipboard waits run on a dedicated native worker.
mod binding;
mod capture;
pub use capture::*;
mod connection_assist;
pub use connection_assist::*;
mod dib;
mod drag;
mod operation;
#[cfg(windows)]
mod platform;
#[cfg(not(windows))]
#[path = "unsupported.rs"]
mod platform;
mod worker;

pub use operation::{HostOperation, HostOperationStatus};
use std::sync::Arc;
pub use worker::HostService;

uniffi::setup_scaffolding!();

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum HostError {
    #[error("Windows host services are unavailable on this platform")]
    UnsupportedPlatform,
    #[error("the host service is closed")]
    Closed,
    #[error("the bounded host worker or byte budget is full; retry later")]
    Busy,
    #[error("PNG bytes are invalid or use unsupported color metadata")]
    InvalidPng,
    #[error("clipboard DIB is malformed")]
    InvalidDib,
    #[error("this DIB layout or compression is unsupported")]
    UnsupportedDib,
    #[error("sample-depth reduction requires explicit conversion or export permission")]
    DepthConversionRequired,
    #[error("convert this image to RGB/sRGB explicitly before creating a DIB companion")]
    ColorConversionRequired,
    #[error("untagged color requires an explicit sRGB assumption")]
    ColorAssumptionRequired,
    #[error("no requested image format is available on the clipboard")]
    ClipboardImageUnavailable,
    #[error("the PNG export hash, source or revision does not match its receipt")]
    ExportBindingMismatch,
    #[error("temporary handoff storage is unavailable or its ownership changed")]
    HandoffStorage,
    #[error("the drag-file lease is unknown or already released")]
    UnknownDragLease,
    #[error("PNG exceeds clipboard size or decoded-pixel limits")]
    SizeLimit,
    #[error("the operation was cancelled before publication")]
    Cancelled,
    #[error("the operation expired before publication")]
    Timeout,
    #[error("the operation token has already been used")]
    AlreadyUsed,
    #[error("the window handle is invalid or no longer exists")]
    InvalidWindow,
    #[error("physical DPI or window geometry is unavailable")]
    DpiUnavailable,
    #[error("the clipboard is busy or unavailable; its contents were not changed")]
    ClipboardUnavailable,
    #[error("clipboard publication failed after replacement began; its contents may have changed")]
    ClipboardPublicationFailed,
    #[error("the native host worker is unavailable")]
    WorkerUnavailable,
}
pub type HostResult<T> = Result<T, HostError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ProcessDpiInfo {
    pub per_monitor_v2: bool,
    pub changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct PhysicalRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DpiInfo {
    pub monitor_bounds: PhysicalRect,
    pub work_area: PhysicalRect,
    pub dpi: u32,
    pub scale_factor: f64,
    /// True only after Windows confirms the querying worker uses PMv2.
    pub per_monitor_v2: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct WindowDpiInfo {
    pub window_bounds: PhysicalRect,
    pub client_bounds: PhysicalRect,
    pub monitor: DpiInfo,
    /// Windows returns 96 for a DPI-unaware target. This is deliberately separate
    /// from the monitor's physical DPI obtained using our own PMv2 probe window.
    pub window_dpi: u32,
    pub window_per_monitor_v2: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ClipboardReceipt {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub png_bytes: u64,
    pub clipboard_sequence: u32,
    /// Windows registered "PNG" format; the exact caller bytes are published.
    pub png_format_published: bool,
}

/// Binds a handoff to a completed core export, never an editor preview. The
/// native worker independently verifies all three fields against the PNG.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct ExportBinding {
    pub png_blake3: String,
    pub source_asset: String,
    pub revision: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct DibOptions {
    /// The exact registered PNG remains at its original depth.
    pub allow_depth_reduction: bool,
    pub assume_untagged_srgb: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ClipboardImageReceipt {
    pub image: ClipboardReceipt,
    pub dibv5_bytes: u64,
    pub dib_depth_reduced: bool,
    pub dib_assumed_srgb: bool,
    pub binding: ExportBinding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ClipboardImageFormat {
    Png,
    DibV5,
    Dib,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ClipboardReadFormat {
    PreferPng,
    Png,
    DibV5,
    Dib,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ClipboardReadOptions {
    pub format: ClipboardReadFormat,
    pub assume_untagged_srgb: bool,
}

/// PNG is copied exactly; DIB input is losslessly normalized to PNG samples.
/// This is an import, so it has no invented project revision/export metadata.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct ClipboardImport {
    pub png: Vec<u8>,
    pub format: ClipboardImageFormat,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub clipboard_sequence: u32,
    pub original_payload_blake3: String,
    pub png_blake3: String,
    pub assumed_srgb: bool,
}
impl std::fmt::Debug for ClipboardImport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClipboardImport")
            .field("png_bytes", &self.png.len())
            .field("format", &self.format)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bit_depth", &self.bit_depth)
            .field("clipboard_sequence", &self.clipboard_sequence)
            .field("assumed_srgb", &self.assumed_srgb)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DragFileReceipt {
    pub lease_id: String,
    pub path: String,
    pub png_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub expires_at_unix_ms: u64,
    pub binding: ExportBinding,
}

/// Hold through the drag gesture. Release/drop merely retires the active lease;
/// the immutable file remains until its retention deadline for delayed readers.
#[derive(uniffi::Object)]
pub struct DragFile {
    receipt: DragFileReceipt,
    released: Arc<std::sync::atomic::AtomicBool>,
}
#[uniffi::export]
impl DragFile {
    pub fn receipt(&self) -> HostResult<DragFileReceipt> {
        if self.released.load(std::sync::atomic::Ordering::Acquire) {
            return Err(HostError::UnknownDragLease);
        }
        Ok(self.receipt.clone())
    }
    pub fn release(&self) {
        self.released
            .store(true, std::sync::atomic::Ordering::Release);
    }
}
impl Drop for DragFile {
    fn drop(&mut self) {
        self.released
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct DragCleanupReceipt {
    pub removed: u32,
    pub retained: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct HandoffLimits {
    pub clipboard_png_bytes: u64,
    pub clipboard_dib_bytes: u64,
    pub clipboard_pixels: u64,
    pub drag_png_bytes: u64,
    pub drag_pixels: u64,
    pub drag_active_leases: u32,
}

#[uniffi::export]
pub fn handoff_limits() -> HandoffLimits {
    HandoffLimits {
        clipboard_png_bytes: worker::MAX_PNG_BYTES as u64,
        clipboard_dib_bytes: dib::MAX_DIB_BYTES as u64,
        clipboard_pixels: dib::MAX_CLIPBOARD_PIXELS,
        drag_png_bytes: drag::MAX_DRAG_BYTES,
        drag_pixels: 50_000_000,
        drag_active_leases: drag::MAX_ACTIVE as u32,
    }
}

/// Call from a background Kotlin dispatcher when creating the service. This
/// factory awaits native initialization without blocking on Win32 or PNG work.
#[uniffi::export]
pub async fn create_host_service() -> HostResult<Arc<HostService>> {
    worker::start().await
}

/// Explicit desktop startup only, before any AWT/Compose window is created.
/// This bounded synchronous call performs no allocation-heavy or I/O work and
/// verifies the actual process context, including when a manifest already set it.
#[uniffi::export]
pub fn set_process_per_monitor_v2() -> HostResult<ProcessDpiInfo> {
    platform::set_process_per_monitor_v2()
}

/// Returns the original Windows host setup smoke ABI version only.
// SAFETY: This project-specific symbol has one definition in this library,
// takes no pointers, allocates nothing and cannot unwind across the C boundary.
#[unsafe(no_mangle)]
pub extern "C" fn vw_host_toolchain_smoke_version() -> u32 {
    1
}

#[cfg(test)]
mod handoff_tests;
#[cfg(test)]
mod tests;
