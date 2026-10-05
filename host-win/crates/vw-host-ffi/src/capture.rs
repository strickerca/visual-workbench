//! Additive WGC/UIA API. All foreign providers execute in a hash-pinned owned
//! process; dropped/cancelled callers signal that process without releasing its
//! admission slot or private-output ownership before actual termination.
use std::sync::{Arc, Mutex};
use vw_capture as capture;
#[path = "editor_identity.rs"]
pub(crate) mod editor_identity;
#[path = "capture_fixed.rs"]
mod fixed_target;
pub use editor_identity::*;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CaptureError {
    #[error("capture refused: {kind}")]
    Refused { kind: String },
}
type Result<T> = std::result::Result<T, CaptureError>;
fn failure(error: capture::Error) -> CaptureError {
    CaptureError::Refused {
        kind: format!("{error:?}"),
    }
}
#[derive(uniffi::Object)]
pub struct CaptureOperation {
    cancel: Arc<capture::Cancellation>,
}
#[uniffi::export]
impl CaptureOperation {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self {
            cancel: Arc::new(capture::Cancellation::default()),
        }
    }
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}
impl Default for CaptureOperation {
    fn default() -> Self {
        Self::new()
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct CaptureTarget {
    pub window: u64,
    pub process_id: u32,
    pub process_created: u64,
    pub client_x: i32,
    pub client_y: i32,
    pub client_width: u32,
    pub client_height: u32,
    pub frame_x: i32,
    pub frame_y: i32,
    pub frame_width: u32,
    pub frame_height: u32,
    pub dpi: u32,
    pub observed_ns: u64,
}
impl From<capture::WindowTarget> for CaptureTarget {
    fn from(v: capture::WindowTarget) -> Self {
        Self {
            window: v.window,
            process_id: v.process_id,
            process_created: v.process_created,
            client_x: v.client.x,
            client_y: v.client.y,
            client_width: v.client.width,
            client_height: v.client.height,
            frame_x: v.frame.x,
            frame_y: v.frame.y,
            frame_width: v.frame.width,
            frame_height: v.frame.height,
            dpi: v.dpi,
            observed_ns: v.observed_ns,
        }
    }
}
impl From<CaptureTarget> for capture::WindowTarget {
    fn from(v: CaptureTarget) -> Self {
        Self {
            window: v.window,
            process_id: v.process_id,
            process_created: v.process_created,
            client: capture::Rect {
                x: v.client_x,
                y: v.client_y,
                width: v.client_width,
                height: v.client_height,
            },
            frame: capture::Rect {
                x: v.frame_x,
                y: v.frame_y,
                width: v.frame_width,
                height: v.frame_height,
            },
            dpi: v.dpi,
            observed_ns: v.observed_ns,
        }
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct CaptureFrameInfo {
    pub capture_session_id: String,
    pub frame_id: u64,
    pub geometry_revision: u32,
    pub source_asset_id: String,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub physical_x: i32,
    pub physical_y: i32,
    pub dpi_scale: f64,
    pub timestamp_ns: u64,
    pub captured_at_ms: i64,
    pub window_handle: u64,
    pub border_visible: bool,
    pub png_bytes: u64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct CaptureElement {
    pub local_id: String,
    pub parent_local_id: Option<String>,
    pub name: String,
    pub role: String,
    pub automation_id: Option<String>,
    pub resource_id: Option<String>,
    pub html_id: Option<String>,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub text: String,
    pub enabled: bool,
    pub focused: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct CaptureTree {
    pub source_asset_id: String,
    pub frame_delta_ms: i32,
    pub collection_elapsed_ms: u64,
    pub platform: String,
    pub elements: Vec<CaptureElement>,
}
#[derive(uniffi::Object)]
pub struct CapturedFrame {
    receipt: capture::FrameReceipt,
    helper: Arc<capture::process::LockedHelper>,
}
#[uniffi::export]
impl CapturedFrame {
    pub fn info(&self) -> CaptureFrameInfo {
        let r = &self.receipt;
        let i = &r.identity;
        CaptureFrameInfo {
            capture_session_id: i.capture_session_id.clone(),
            frame_id: i.frame_id,
            geometry_revision: i.geometry_revision,
            source_asset_id: r.source_asset_id.clone(),
            width: r.width,
            height: r.height,
            bit_depth: r.bit_depth,
            physical_x: i.client_rect.x,
            physical_y: i.client_rect.y,
            dpi_scale: i.dpi_scale,
            timestamp_ns: i.monotonic_timestamp_ns,
            captured_at_ms: i.captured_at_ms,
            window_handle: i.window_handle,
            border_visible: r.border_visible,
            png_bytes: r.png_bytes,
        }
    }
    /// Call only after the canonical semantic ticket has been acquired. The
    /// receipt retains the exact HWND/process creation/physical rectangle/frame.
    pub async fn collect(&self, operation: Arc<CaptureOperation>) -> Result<CaptureTree> {
        let helper = self.helper.clone();
        let request = capture::Request::Tree(capture::TreeRequest {
            frame: self.receipt.clone(),
            owner_process_id: owner(),
            limits: capture::Limits::default(),
        });
        let cancel = operation.cancel.clone();
        owned(cancel.clone(), move || {
            match capture::process::run(&helper, request, cancel)? {
                capture::Response::Tree(v) => Ok(CaptureTree {
                    source_asset_id: v.source_asset_id,
                    frame_delta_ms: v.frame_delta_ms,
                    collection_elapsed_ms: v.collection_elapsed_ms,
                    platform: v.platform,
                    elements: v
                        .elements
                        .into_iter()
                        .map(|e| CaptureElement {
                            local_id: e.local_id,
                            parent_local_id: e.parent_local_id,
                            name: e.name,
                            role: e.role,
                            automation_id: e.automation_id,
                            resource_id: e.resource_id,
                            html_id: e.html_id,
                            x: e.bounds[0],
                            y: e.bounds[1],
                            width: e.bounds[2],
                            height: e.bounds[3],
                            text: e.text,
                            enabled: e.enabled,
                            focused: e.focused,
                        })
                        .collect(),
                }),
                _ => Err(capture::Error::Invalid),
            }
        })
        .await
    }
}
#[derive(uniffi::Object)]
pub struct CaptureService {
    helper: Arc<capture::process::LockedHelper>,
}
#[uniffi::export]
pub async fn create_capture_service(
    helper_path: String,
    packaged_sha256: String,
) -> Result<Arc<CaptureService>> {
    owned(Arc::new(capture::Cancellation::default()), move || {
        Ok(Arc::new(CaptureService {
            helper: Arc::new(capture::process::LockedHelper::open(
                std::path::Path::new(&helper_path),
                &packaged_sha256,
            )?),
        }))
    })
    .await
}
#[uniffi::export]
impl CaptureService {
    pub async fn foreground_target(&self) -> Result<CaptureTarget> {
        owned(Arc::new(capture::Cancellation::default()), move || {
            foreground().map(Into::into)
        })
        .await
    }
    pub async fn capture(
        &self,
        target: CaptureTarget,
        capture_session_id: String,
        private_output_directory: String,
        operation: Arc<CaptureOperation>,
    ) -> Result<Arc<CapturedFrame>> {
        let helper = self.helper.clone();
        let cancel = operation.cancel.clone();
        let request = capture::Request::Capture(capture::CaptureRequest {
            explicit_owner_action: true,
            owner_process_id: owner(),
            target: target.into(),
            capture_session_id,
            output_directory: private_output_directory,
            limits: capture::Limits::default(),
            diagnostic_color_stage: false,
            diagnostic_alpha_region: None,
        });
        owned(cancel.clone(), move || {
            match capture::process::run(&helper, request, cancel)? {
                capture::Response::Frame(receipt) => {
                    Ok(Arc::new(CapturedFrame { receipt, helper }))
                }
                _ => Err(capture::Error::Invalid),
            }
        })
        .await
    }
}
struct CancelOnDrop(Arc<capture::Cancellation>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
static WORKERS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
async fn owned<T: Send + 'static>(
    cancel: Arc<capture::Cancellation>,
    work: impl FnOnce() -> capture::Result<T> + Send + 'static,
) -> Result<T> {
    use std::sync::atomic::Ordering;
    struct Slot;
    impl Drop for Slot {
        fn drop(&mut self) {
            WORKERS.store(false, Ordering::Release);
        }
    }
    WORKERS
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| failure(capture::Error::Busy))?;
    let slot = Slot;
    let guard = CancelOnDrop(cancel);
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("vw-capture-owner".into())
        .spawn(move || {
            let value = work();
            drop(slot);
            let _ = tx.send(value);
        })
        .map_err(|_| failure(capture::Error::Platform))?;
    let result = rx
        .await
        .map_err(|_| failure(capture::Error::Platform))?
        .map_err(failure);
    drop(guard);
    result
}
fn owner() -> u32 {
    std::process::id()
}
#[cfg(windows)]
fn foreground() -> capture::Result<capture::WindowTarget> {
    capture::windows::foreground(owner())
}
#[cfg(not(windows))]
fn foreground() -> capture::Result<capture::WindowTarget> {
    Err(capture::Error::Unsupported)
}
/// Must be invoked for every new top-level HWND before showing it.
#[uniffi::export]
pub fn exclude_capture_windows(windows: Vec<u64>) -> Result<()> {
    #[cfg(windows)]
    {
        capture::windows::exclude_own_windows(&windows).map_err(failure)
    }
    #[cfg(not(windows))]
    {
        let _ = windows;
        Err(failure(capture::Error::Unsupported))
    }
}
#[derive(uniffi::Object)]
pub struct CaptureHotkey {
    #[cfg(windows)]
    watch: Mutex<Option<capture::windows::hotkey::Hotkey>>,
}
#[uniffi::export]
pub fn register_capture_hotkey(modifiers: u32, key: u32) -> Result<Arc<CaptureHotkey>> {
    #[cfg(windows)]
    {
        Ok(Arc::new(CaptureHotkey {
            watch: Mutex::new(Some(
                capture::windows::hotkey::Hotkey::start(modifiers, key).map_err(failure)?,
            )),
        }))
    }
    #[cfg(not(windows))]
    {
        let _ = (modifiers, key);
        Err(failure(capture::Error::Unsupported))
    }
}
#[uniffi::export]
impl CaptureHotkey {
    pub fn take(&self) -> Result<Option<CaptureTarget>> {
        #[cfg(windows)]
        {
            self.watch
                .lock()
                .map_err(|_| failure(capture::Error::Platform))?
                .as_ref()
                .ok_or_else(|| failure(capture::Error::Unavailable))?
                .take()
                .map(|v| v.map(Into::into))
                .map_err(failure)
        }
        #[cfg(not(windows))]
        {
            Err(failure(capture::Error::Unsupported))
        }
    }
    pub fn shutdown(&self) -> Result<()> {
        #[cfg(windows)]
        {
            if let Some(mut watch) = self
                .watch
                .lock()
                .map_err(|_| failure(capture::Error::Platform))?
                .take()
            {
                watch.shutdown();
            }
        }
        Ok(())
    }
}
