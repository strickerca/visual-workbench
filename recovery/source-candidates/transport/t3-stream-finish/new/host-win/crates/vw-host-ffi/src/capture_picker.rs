//! Child of capture.rs: all calls share the existing retained worker/helper.
use super::*;
use capture::picker as picker;
#[path="capture_picker_preview.rs"] mod preview;

#[derive(Clone, Debug, uniffi::Record)]
pub struct PickerMonitor { pub monitor: u64, pub device: String, pub x: i32, pub y: i32,
    pub width: u32, pub height: u32, pub dpi: u32, pub rotation: u32, pub primary: bool, pub observed_ns: u64 }
impl From<picker::MonitorTarget> for PickerMonitor { fn from(value: picker::MonitorTarget) -> Self {
    Self { monitor: value.monitor, device: value.device, x: value.physical.x, y: value.physical.y,
        width: value.physical.width, height: value.physical.height, dpi: value.dpi,
        rotation: value.rotation, primary: value.primary, observed_ns: value.observed_ns }
} }
impl From<PickerMonitor> for picker::MonitorTarget { fn from(value: PickerMonitor) -> Self {
    Self { monitor: value.monitor, device: value.device, physical: capture::Rect { x: value.x, y: value.y,
        width: value.width, height: value.height }, dpi: value.dpi, rotation: value.rotation,
        primary: value.primary, observed_ns: value.observed_ns }
} }
#[derive(Clone, Debug, uniffi::Record)]
pub struct PickerWindow { pub title: String, pub target: CaptureTarget }
#[derive(Clone, Debug, uniffi::Record)]
pub struct CapturePickerCatalog { pub windows: Vec<PickerWindow>, pub monitors: Vec<PickerMonitor>, pub truncated: bool }
#[derive(Clone, Debug, uniffi::Enum)]
pub enum CaptureSelection {
    Window { target: CaptureTarget }, Monitor { target: PickerMonitor },
    Region { target: PickerMonitor, x: i32, y: i32, width: u32, height: u32 },
}
impl From<CaptureSelection> for picker::PickerSource { fn from(value: CaptureSelection) -> Self {
    match value {
        CaptureSelection::Window { target } => Self::Window { target: target.into() },
        CaptureSelection::Monitor { target } => Self::Monitor { target: target.into() },
        CaptureSelection::Region { target, x, y, width, height } => Self::Region {
            target: target.into(), region: capture::Rect { x, y, width, height } },
    }
} }
#[derive(Clone, Debug, uniffi::Record)]
pub struct PickerFrameInfo { pub frame: CaptureFrameInfo, pub source_kind: String, pub monitor_id: String, pub semantic_window_available: bool }
#[derive(Clone, Debug, uniffi::Record)]
pub struct PickerPreview { pub source_asset_id: String, pub source_width: u32, pub source_height: u32,
    pub width: u32, pub height: u32, pub rgba: Vec<u8> }
#[derive(uniffi::Object)]
pub struct PickerFrame { receipt: picker::PickerFrameReceipt, helper: Arc<capture::process::LockedHelper>, path: std::path::PathBuf }
#[uniffi::export]
impl PickerFrame {
    pub fn info(&self) -> PickerFrameInfo {
        let r = &self.receipt; let i = &r.identity;
        PickerFrameInfo { frame: CaptureFrameInfo { capture_session_id: i.capture_session_id.clone(), frame_id: i.frame_id,
            geometry_revision: i.geometry_revision, source_asset_id: r.source_asset_id.clone(), width: r.width,
            height: r.height, bit_depth: r.bit_depth, physical_x: i.client_rect.x, physical_y: i.client_rect.y,
            dpi_scale: i.dpi_scale, timestamp_ns: i.monotonic_timestamp_ns, captured_at_ms: i.captured_at_ms,
            window_handle: i.window_handle, border_visible: r.border_visible, png_bytes: r.png_bytes },
            source_kind: i.source_kind.clone(), monitor_id: i.monitor_id.clone(), semantic_window_available: r.window_receipt.is_some() }
    }
    pub async fn collect(&self, operation: Arc<CaptureOperation>) -> Result<CaptureTree> {
        let receipt = self.receipt.window_receipt.clone().ok_or_else(|| failure(capture::Error::Unsupported))?;
        CapturedFrame { receipt, helper: self.helper.clone() }.collect(operation).await
    }
    pub async fn preview(&self, max_side: u32, operation: Arc<CaptureOperation>) -> Result<PickerPreview> {
        let receipt = self.receipt.clone(); let path = self.path.clone(); let cancel = operation.cancel.clone();
        owned(cancel.clone(), move || preview::read(&path, &receipt, max_side, &cancel)).await
    }
    /// Crop the exact displayed lossless monitor snapshot. Never recapture a
    /// newer unseen frame when the owner accepts the drawn region.
    pub async fn crop_region(&self, edges: Vec<f64>, private_output_directory: String,
        operation: Arc<CaptureOperation>) -> Result<Arc<PickerFrame>> {
        let values: [f64; 4] = edges.try_into().map_err(|_| failure(capture::Error::Invalid))?;
        let target = match &self.receipt.source {
            picker::PickerSource::Monitor { target } => target.clone(),
            _ => return Err(failure(capture::Error::Invalid)),
        };
        let region = self.receipt.source.region_from_frame_edges(values, self.receipt.identity.geometry_revision).map_err(failure)?;
        if private_output_directory.len() > 4096 { return Err(failure(capture::Error::Limit)); }
        let output = std::path::PathBuf::from(private_output_directory);
        let path = self.path.clone(); let receipt = self.receipt.clone(); let helper = self.helper.clone(); let cancel = operation.cancel.clone();
        owned(cancel.clone(), move || {
            capture::plain(&output, true)?;
            let (source_asset_id, png_bytes) = preview::crop(&path, &receipt, region, &output, &cancel)?;
            let mut identity = receipt.identity; identity.source_kind = "region".into(); identity.client_rect = region;
            let receipt = picker::PickerFrameReceipt { identity, source: picker::PickerSource::Region { target, region },
                source_asset_id, png_bytes, width: region.width, height: region.height, bit_depth: 8,
                border_visible: receipt.border_visible, filename: "capture.png".into(), window_receipt: None };
            Ok(Arc::new(PickerFrame { receipt, helper, path: output.join("capture.png") }))
        }).await
    }
}
#[uniffi::export]
impl CaptureService {
    pub async fn picker_catalog(&self, operation: Arc<CaptureOperation>) -> Result<CapturePickerCatalog> {
        let helper = self.helper.clone(); let cancel = operation.cancel.clone();
        owned(cancel.clone(), move || match capture::process::run(&helper,
            capture::Request::PickerCatalog { owner_process_id: owner() }, cancel)? {
            capture::Response::PickerCatalog(value) => { value.validate(owner())?;
                Ok(CapturePickerCatalog { windows: value.windows.into_iter().map(|v| PickerWindow { title: v.title, target: v.target.into() }).collect(),
                    monitors: value.monitors.into_iter().map(Into::into).collect(), truncated: value.truncated }) },
            _ => Err(capture::Error::Invalid),
        }).await
    }
    pub async fn capture_selection(&self, source: CaptureSelection, capture_session_id: String,
        frame_id: u64, geometry_revision: u32, private_output_directory: String,
        operation: Arc<CaptureOperation>) -> Result<Arc<PickerFrame>> {
        let helper = self.helper.clone(); let cancel = operation.cancel.clone();
        let request = picker::PickerCaptureRequest { explicit_owner_action: true, owner_process_id: owner(),
            source: source.into(), capture_session_id, frame_id, geometry_revision,
            output_directory: private_output_directory, limits: capture::Limits::default() };
        request.validate(&cancel).map_err(failure)?;
        let path = std::path::Path::new(&request.output_directory).join("capture.png");
        owned(cancel.clone(), move || match capture::process::run(&helper, capture::Request::PickerCapture(request.clone()), cancel)? {
            capture::Response::PickerFrame(receipt) => { receipt.validate(&request)?; Ok(Arc::new(PickerFrame { receipt, helper, path })) },
            _ => Err(capture::Error::Invalid),
        }).await
    }
}
/// Pure bounded conversion; the capture call independently revalidates source
/// geometry before any GPU work. This does not refresh or broaden a selection.
#[uniffi::export]
pub fn picker_region(target: PickerMonitor, edges: Vec<f64>, geometry_revision: u32) -> Result<CaptureSelection> {
    let values: [f64; 4] = edges.try_into().map_err(|_| failure(capture::Error::Invalid))?;
    let source = picker::PickerSource::Monitor { target: target.clone().into() };
    source.validate(owner(), capture::Limits::default()).map_err(failure)?;
    let region = source.region_from_frame_edges(values, geometry_revision).map_err(failure)?;
    Ok(CaptureSelection::Region { target, x: region.x, y: region.y, width: region.width, height: region.height })
}
#[derive(uniffi::Object)]
pub struct RegionPickerHotkey { #[cfg(windows)] watch: Mutex<Option<capture::windows::region_hotkey::RegionHotkey>> }
#[uniffi::export]
pub async fn register_region_picker_hotkey(modifiers: u32, key: u32) -> Result<Arc<RegionPickerHotkey>> {
    owned(Arc::new(capture::Cancellation::default()), move || {
        #[cfg(windows)] { Ok(Arc::new(RegionPickerHotkey { watch: Mutex::new(Some(capture::windows::region_hotkey::RegionHotkey::start(modifiers, key)?)) })) }
        #[cfg(not(windows))] { let _ = (modifiers, key); Err(capture::Error::Unsupported) }
    }).await
}
#[uniffi::export]
impl RegionPickerHotkey {
    pub fn take(&self) -> Result<bool> {
        #[cfg(windows)] { Ok(self.watch.lock().map_err(|_| failure(capture::Error::Platform))?.as_ref().ok_or_else(|| failure(capture::Error::Unavailable))?.take()) }
        #[cfg(not(windows))] { Err(failure(capture::Error::Unsupported)) }
    }
    pub fn shutdown(&self) -> Result<()> {
        #[cfg(windows)] { if let Some(mut value) = self.watch.lock().map_err(|_| failure(capture::Error::Platform))?.take() { value.shutdown(); } }
        Ok(())
    }
}
