//! Lossless selected-monitor/region capture. Window captures keep the reviewed
//! HWND/process identity path; all GPU work stays in the bounded helper process.
use crate::{picker::*, *};
use super::{api, DpiScope};
use std::{path::Path, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};
use ::windows::{core::Interface, Foundation::TimeSpan, Graphics::{Capture::*, DirectX::{
    Direct3D11::IDirect3DDevice, DirectXPixelFormat}}, Win32::{Foundation::*, Graphics::{
    Direct3D::*, Direct3D11::*, Dxgi::{*, Common::*}, Gdi::*}, System::WinRT::{
    Direct3D11::*, Graphics::Capture::IGraphicsCaptureItemInterop}}};

struct Session { session: GraphicsCaptureSession, pool: Direct3D11CaptureFramePool }
impl Drop for Session { fn drop(&mut self) { let _ = self.session.Close(); let _ = self.pool.Close(); } }
struct Frame(Direct3D11CaptureFrame);
impl Drop for Frame { fn drop(&mut self) { let _ = self.0.Close(); } }

pub fn capture(request: PickerCaptureRequest, cancel: &Cancellation) -> Result<PickerFrameReceipt> {
    request.validate(cancel)?;
    if let PickerSource::Window { target } = &request.source {
        let mut frame = super::capture(CaptureRequest { explicit_owner_action: true,
            owner_process_id: request.owner_process_id, target: target.clone(),
            capture_session_id: request.capture_session_id, output_directory: request.output_directory,
            limits: request.limits }, cancel)?;
        frame.identity.frame_id = request.frame_id; frame.identity.geometry_revision = request.geometry_revision;
        return Ok(PickerFrameReceipt { identity: frame.identity.clone(), source: request.source,
            source_asset_id: frame.source_asset_id.clone(), png_bytes: frame.png_bytes,
            width: frame.width, height: frame.height, bit_depth: frame.bit_depth,
            border_visible: frame.border_visible, filename: frame.filename.clone(), window_receipt: Some(frame) });
    }
    let target = match &request.source {
        PickerSource::Monitor { target } | PickerSource::Region { target, .. } => target,
        PickerSource::Window { .. } => return Err(Error::Invalid),
    };
    let output_rect = request.source.rect();
    plain(Path::new(&request.output_directory), true)?;
    let _dpi = DpiScope::enter()?;
    check_target(target, request.owner_process_id, cancel)?;
    if !api(GraphicsCaptureSession::IsSupported())? { return Err(Error::Unsupported); }
    let monitor = HMONITOR(target.monitor as usize as *mut _);
    let (device, context) = device_for_monitor(monitor)?;
    let dxgi: IDXGIDevice = api(device.cast())?;
    let factory: IGraphicsCaptureItemInterop = api(::windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>())?;
    // SAFETY: the monitor was just validated; COM owns the item and device.
    let (item, direct): (GraphicsCaptureItem, IDirect3DDevice) = unsafe {
        (api(factory.CreateForMonitor(monitor))?, api(api(CreateDirect3D11DeviceFromDXGIDevice(&dxgi))?.cast())?)
    };
    let size = api(item.Size())?;
    if size.Width != target.physical.width as i32 || size.Height != target.physical.height as i32 { return Err(Error::Stale); }
    let pool = api(Direct3D11CaptureFramePool::CreateFreeThreaded(&direct, DirectXPixelFormat::B8G8R8A8UIntNormalized, 1, size))?;
    let session = Session { session: api(pool.CreateCaptureSession(&item))?, pool };
    api(session.session.SetMinUpdateInterval(TimeSpan { Duration: 166_667 }))?;
    api(session.session.SetIncludeSecondaryWindows(true))?; api(session.session.SetIsCursorCaptureEnabled(false))?;
    api(session.session.SetIsBorderRequired(true))?; api(session.session.StartCapture())?;
    let started = Instant::now();
    let frame = loop {
        cancel.check()?;
        if started.elapsed() > Duration::from_millis(request.limits.capture_ms) { return Err(Error::Timeout); }
        match session.pool.TryGetNextFrame() {
            Ok(frame) => break Frame(frame),
            Err(error) if error.code() == E_POINTER || error.code() == S_OK => std::thread::sleep(Duration::from_millis(1)),
            Err(_) => return Err(Error::Platform),
        }
    };
    if api(frame.0.ContentSize())? != size { return Err(Error::Stale); }
    let ticks = api(frame.0.SystemRelativeTime())?.Duration;
    let frame_ns = u64::try_from(ticks).ok().and_then(|value| value.checked_mul(100)).ok_or(Error::Limit)?;
    if frame_ns == 0 || frame_ns < target.observed_ns { return Err(Error::Stale); }
    let access: IDirect3DDxgiInterfaceAccess = api(api(frame.0.Surface())?.cast())?;
    // SAFETY: the frame lease remains live until texture readback finishes.
    let texture: ID3D11Texture2D = unsafe { api(access.GetInterface())? };
    let pixels = super::wgc::readback_rect(&device, &context, &texture, target.physical,
        output_rect, request.limits, cancel)?;
    check_target(target, request.owner_process_id, cancel)?;
    let (asset, png_bytes) = publish_png(Path::new(&request.output_directory), output_rect.width,
        output_rect.height, &pixels, request.limits, cancel)?;
    let now = super::clock_ns()?;
    let age_ms = now.checked_sub(frame_ns).ok_or(Error::Stale)? / 1_000_000;
    let wall_ms = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| Error::Platform)?.as_millis();
    let captured_at_ms = i64::try_from(wall_ms.checked_sub(u128::from(age_ms)).ok_or(Error::Stale)?).map_err(|_| Error::Limit)?;
    let identity = FrameIdentity { capture_session_id: request.capture_session_id, frame_id: request.frame_id,
        geometry_revision: request.geometry_revision, source_kind: request.source.kind().into(),
        client_rect: output_rect, dpi_scale: f64::from(target.dpi) / 96.0,
        monotonic_timestamp_ns: frame_ns, captured_at_ms, platform: "windows".into(),
        window_handle: 0, monitor_id: target.device.clone() };
    identity.validate()?; cancel.check()?;
    Ok(PickerFrameReceipt { identity, source: request.source, source_asset_id: asset, png_bytes,
        width: output_rect.width, height: output_rect.height, bit_depth: 8, border_visible: true,
        filename: "capture.png".into(), window_receipt: None })
}
fn check_target(target: &MonitorTarget, owner: u32, cancel: &Cancellation) -> Result<()> {
    cancel.check()?; target.unchanged(&super::picker::monitor(target.monitor)?)?;
    super::picker::require_own_windows_excluded(owner)
}
pub(super) fn device_for_monitor(monitor: HMONITOR) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    // SAFETY: bounded DXGI adapter/output enumeration. No raw pointer from an
    // untrusted process is used. D3D output options are initialized and owned.
    unsafe {
        let factory: IDXGIFactory1 = api(CreateDXGIFactory1())?;
        for adapter_index in 0..32 {
            let adapter = match factory.EnumAdapters1(adapter_index) {
                Ok(value) => value, Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(_) => return Err(Error::Platform),
            };
            for output_index in 0..32 {
                let output = match adapter.EnumOutputs(output_index) {
                    Ok(value) => value, Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                    Err(_) => return Err(Error::Platform),
                };
                let output: IDXGIOutput6 = api(output.cast())?; let desc = api(output.GetDesc1())?;
                if desc.Monitor != monitor { continue; }
                if desc.ColorSpace != DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709 { return Err(Error::ColorDepth); }
                let adapter: IDXGIAdapter = api(adapter.cast())?;
                let (mut device, mut context) = (None, None);
                api(D3D11CreateDevice(&adapter, D3D_DRIVER_TYPE_UNKNOWN, HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT, Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                    D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context)))?;
                return Ok((device.ok_or(Error::Platform)?, context.ok_or(Error::Platform)?));
            }
        }
    }
    Err(Error::Unavailable)
}
