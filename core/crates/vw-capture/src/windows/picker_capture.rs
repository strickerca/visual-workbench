//! Lossless selected-monitor/region capture. Window captures keep the reviewed
//! HWND/process identity path; all GPU work stays in the bounded helper process.
use super::{DpiScope, api};
use crate::{picker::*, *};
use ::windows::{
    Foundation::TimeSpan,
    Graphics::{
        Capture::*,
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
    },
    Win32::{
        Foundation::*,
        Graphics::{
            Direct3D::*,
            Direct3D11::*,
            Dxgi::{Common::*, *},
            Gdi::*,
        },
        System::WinRT::{Direct3D11::*, Graphics::Capture::IGraphicsCaptureItemInterop},
    },
    core::Interface,
};
use std::{
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Pool(Direct3D11CaptureFramePool);
impl Drop for Pool {
    fn drop(&mut self) {
        let _ = self.0.Close();
    }
}
struct Session(GraphicsCaptureSession);
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.0.Close();
    }
}
struct Frame(Direct3D11CaptureFrame);
impl Drop for Frame {
    fn drop(&mut self) {
        let _ = self.0.Close();
    }
}

pub fn capture(request: PickerCaptureRequest, cancel: &Cancellation) -> Result<PickerFrameReceipt> {
    let started = Instant::now();
    request.validate(cancel)?;
    if let PickerSource::Window { target } = &request.source {
        let mut frame = super::capture(
            CaptureRequest {
                explicit_owner_action: true,
                owner_process_id: request.owner_process_id,
                target: target.clone(),
                capture_session_id: request.capture_session_id.clone(),
                output_directory: request.output_directory.clone(),
                limits: request.limits,
                diagnostic_color_stage: false,
                diagnostic_alpha_region: None,
            },
            cancel,
        )?;
        frame.identity.frame_id = request.frame_id;
        frame.identity.geometry_revision = request.geometry_revision;
        crate::canvas::final_target_check(
            cancel,
            || started.elapsed() <= Duration::from_millis(request.limits.capture_ms),
            || {
                target.unchanged(
                    &super::fixed_target(target.window, request.owner_process_id)?,
                    request.owner_process_id,
                )
            },
        )?;
        let receipt = PickerFrameReceipt {
            identity: frame.identity.clone(),
            source: request.source.clone(),
            source_asset_id: frame.source_asset_id.clone(),
            png_bytes: frame.png_bytes,
            width: frame.width,
            height: frame.height,
            bit_depth: frame.bit_depth,
            border_visible: frame.border_visible,
            filename: frame.filename.clone(),
            window_receipt: Some(frame),
        };
        receipt.validate(&request)?;
        completion_check(&request, cancel, started)?;
        return Ok(receipt);
    }
    let target = match &request.source {
        PickerSource::Monitor { target } | PickerSource::Region { target, .. } => target,
        PickerSource::Window { .. } => return Err(Error::Invalid),
    };
    let output_rect = request.source.rect();
    plain(Path::new(&request.output_directory), true)?;
    let _dpi = DpiScope::enter()?;
    check_target(&request, cancel, started)?;
    if !api(GraphicsCaptureSession::IsSupported())? {
        return Err(Error::Unsupported);
    }
    let monitor = HMONITOR(target.monitor as usize as *mut _);
    let (device, context) = device_for_monitor(monitor)?;
    completion_check(&request, cancel, started)?;
    let dxgi: IDXGIDevice = api(device.cast())?;
    let factory: IGraphicsCaptureItemInterop = api(::windows::core::factory::<
        GraphicsCaptureItem,
        IGraphicsCaptureItemInterop,
    >())?;
    // SAFETY: the monitor was just validated; COM owns the item and device.
    let (item, direct): (GraphicsCaptureItem, IDirect3DDevice) = unsafe {
        (
            api(factory.CreateForMonitor(monitor))?,
            api(api(CreateDirect3D11DeviceFromDXGIDevice(&dxgi))?.cast())?,
        )
    };
    let size = api(item.Size())?;
    if size.Width != target.physical.width as i32 || size.Height != target.physical.height as i32 {
        return Err(Error::Stale);
    }
    let pool = Pool(api(Direct3D11CaptureFramePool::CreateFreeThreaded(
        &direct,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        1,
        size,
    ))?);
    let session = Session(api(pool.0.CreateCaptureSession(&item))?);
    api(session
        .0
        .SetMinUpdateInterval(TimeSpan { Duration: 166_667 }))?;
    api(session.0.SetIncludeSecondaryWindows(true))?;
    api(session.0.SetIsCursorCaptureEnabled(false))?;
    api(session.0.SetIsBorderRequired(true))?;
    check_target(&request, cancel, started)?;
    api(session.0.StartCapture())?;
    let frame = loop {
        completion_check(&request, cancel, started)?;
        match pool.0.TryGetNextFrame() {
            Ok(frame) => break Frame(frame),
            Err(error) if error.code() == E_POINTER || error.code() == S_OK => {
                std::thread::sleep(Duration::from_millis(1))
            }
            Err(_) => return Err(Error::Platform),
        }
    };
    if api(frame.0.ContentSize())? != size {
        return Err(Error::Stale);
    }
    let ticks = api(frame.0.SystemRelativeTime())?.Duration;
    let frame_ns = u64::try_from(ticks)
        .ok()
        .and_then(|value| value.checked_mul(100))
        .ok_or(Error::Limit)?;
    if frame_ns == 0 || frame_ns < target.observed_ns {
        return Err(Error::Stale);
    }
    let access: IDirect3DDxgiInterfaceAccess = api(api(frame.0.Surface())?.cast())?;
    // SAFETY: the frame lease remains live until texture readback finishes.
    let texture: ID3D11Texture2D = unsafe { api(access.GetInterface())? };
    require_sdr(&dxgi, monitor)?;
    let pixels = readback_rect(&device, &context, &texture, &request, cancel, started)?;
    // Release native frame/pool owners before the slower PNG publication. The
    // plain RGBA buffer is now the sole pixel source retained by this helper.
    drop(texture);
    drop(access);
    drop(frame);
    drop(session);
    drop(pool);
    drop(item);
    drop(direct);
    drop(context);
    drop(device);
    check_target(&request, cancel, started)?;
    let (asset, png_bytes) = publish_png(
        Path::new(&request.output_directory),
        output_rect.width,
        output_rect.height,
        &pixels,
        request.limits,
        cancel,
    )?;
    check_target(&request, cancel, started)?;
    require_sdr(&dxgi, monitor)?;
    let now = super::clock_ns()?;
    let age_ms = now.checked_sub(frame_ns).ok_or(Error::Stale)? / 1_000_000;
    let wall_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Platform)?
        .as_millis();
    let captured_at_ms = i64::try_from(
        wall_ms
            .checked_sub(u128::from(age_ms))
            .ok_or(Error::Stale)?,
    )
    .map_err(|_| Error::Limit)?;
    let identity = FrameIdentity {
        capture_session_id: request.capture_session_id.clone(),
        frame_id: request.frame_id,
        geometry_revision: request.geometry_revision,
        source_kind: request.source.kind().into(),
        client_rect: output_rect,
        dpi_scale: f64::from(target.dpi) / 96.0,
        monotonic_timestamp_ns: frame_ns,
        captured_at_ms,
        platform: "windows".into(),
        window_handle: 0,
        monitor_id: target.device.clone(),
    };
    let receipt = PickerFrameReceipt {
        identity,
        source: request.source.clone(),
        source_asset_id: asset,
        png_bytes,
        width: output_rect.width,
        height: output_rect.height,
        bit_depth: 8,
        border_visible: true,
        filename: "capture.png".into(),
        window_receipt: None,
    };
    receipt.validate(&request)?;
    completion_check(&request, cancel, started)?;
    Ok(receipt)
}
fn completion_check(
    request: &PickerCaptureRequest,
    cancel: &Cancellation,
    started: Instant,
) -> Result<()> {
    crate::canvas::completion_check(cancel, &mut || {
        started.elapsed() <= Duration::from_millis(request.limits.capture_ms)
    })
}
fn check_target(
    request: &PickerCaptureRequest,
    cancel: &Cancellation,
    started: Instant,
) -> Result<()> {
    let target = match &request.source {
        PickerSource::Monitor { target } | PickerSource::Region { target, .. } => target,
        PickerSource::Window { .. } => return Err(Error::Invalid),
    };
    crate::canvas::final_target_check(
        cancel,
        || started.elapsed() <= Duration::from_millis(request.limits.capture_ms),
        || {
            target.unchanged(&super::picker::monitor(target.monitor)?)?;
            super::picker::require_own_windows_excluded(request.owner_process_id)
        },
    )
}
// The current window WGC readback also serves diagnostic/owned-canvas requests.
// Keep this monitor-only variant private so those reviewed paths are unchanged;
// descriptor, crop, pitch and opacity admission are the same, with this request's
// single deadline additionally checked on each selected row.
fn readback_rect(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
    request: &PickerCaptureRequest,
    cancel: &Cancellation,
    started: Instant,
) -> Result<Vec<u8>> {
    let frame = match &request.source {
        PickerSource::Monitor { target } | PickerSource::Region { target, .. } => target.physical,
        PickerSource::Window { .. } => return Err(Error::Invalid),
    };
    let selected = request.source.rect();
    let (x, y) = selected.offset_inside(frame)?;
    let count = request.limits.image(selected.width, selected.height)?;
    completion_check(request, cancel, started)?;
    // SAFETY: the live frame owns this texture. Descriptor, full mapped extent,
    // row pitch and selected offsets are checked before any pointer read. Unmap
    // runs on every result from the closure, including deadline/cancel failure.
    unsafe {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        texture.GetDesc(&mut desc);
        if desc.Width != frame.width
            || desc.Height != frame.height
            || desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
            || desc.ArraySize != 1
            || desc.MipLevels != 1
            || desc.SampleDesc.Count != 1
        {
            return Err(Error::ColorDepth);
        }
        request.limits.image(desc.Width, desc.Height)?;
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;
        let mut staging = None;
        api(device.CreateTexture2D(&desc, None, Some(&mut staging)))?;
        let staging = staging.ok_or(Error::Platform)?;
        context.CopyResource(&staging, texture);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        api(context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)))?;
        let result = (|| {
            if mapped.pData.is_null()
                || mapped.RowPitch < desc.Width.checked_mul(4).ok_or(Error::Limit)?
                || u64::from(mapped.RowPitch) * u64::from(desc.Height) > request.limits.memory_bytes
            {
                return Err(Error::Limit);
            }
            let mut rgba = Vec::with_capacity(count);
            for row in y..y + selected.height {
                completion_check(request, cancel, started)?;
                let offset = (row as usize)
                    .checked_mul(mapped.RowPitch as usize)
                    .ok_or(Error::Limit)?;
                let mapped_row = std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(offset),
                    desc.Width as usize * 4,
                );
                let selected_row = crate::canvas::selected_row(mapped_row, x, selected.width)?;
                for pixel in selected_row.as_chunks::<4>().0 {
                    if pixel[3] != 255 {
                        return Err(Error::ColorDepth);
                    }
                    rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                }
            }
            completion_check(request, cancel, started)?;
            Ok(rgba)
        })();
        context.Unmap(&staging, 0);
        result
    }
}
fn require_sdr(device: &IDXGIDevice, monitor: HMONITOR) -> Result<()> {
    // SAFETY: the capture device and its bounded system-owned output interfaces
    // remain live. Querying current metadata never dereferences a monitor handle.
    unsafe {
        let adapter = api(device.GetAdapter())?;
        for index in 0..32 {
            let output = match adapter.EnumOutputs(index) {
                Ok(value) => value,
                Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(_) => return Err(Error::Platform),
            };
            let output: IDXGIOutput6 = api(output.cast())?;
            let desc = api(output.GetDesc1())?;
            if desc.Monitor == monitor {
                return if desc.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709 {
                    Ok(())
                } else {
                    Err(Error::ColorDepth)
                };
            }
        }
    }
    Err(Error::ColorDepth)
}
pub(super) fn device_for_monitor(monitor: HMONITOR) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    // SAFETY: bounded DXGI adapter/output enumeration. No raw pointer from an
    // untrusted process is used. D3D output options are initialized and owned.
    unsafe {
        let factory: IDXGIFactory1 = api(CreateDXGIFactory1())?;
        for adapter_index in 0..32 {
            let adapter = match factory.EnumAdapters1(adapter_index) {
                Ok(value) => value,
                Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(_) => return Err(Error::Platform),
            };
            for output_index in 0..32 {
                let output = match adapter.EnumOutputs(output_index) {
                    Ok(value) => value,
                    Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                    Err(_) => return Err(Error::Platform),
                };
                let output: IDXGIOutput6 = api(output.cast())?;
                let desc = api(output.GetDesc1())?;
                if desc.Monitor != monitor {
                    continue;
                }
                if desc.ColorSpace != DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709 {
                    return Err(Error::ColorDepth);
                }
                let adapter: IDXGIAdapter = api(adapter.cast())?;
                let (mut device, mut context) = (None, None);
                api(D3D11CreateDevice(
                    &adapter,
                    D3D_DRIVER_TYPE_UNKNOWN,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                ))?;
                return Ok((
                    device.ok_or(Error::Platform)?,
                    context.ok_or(Error::Platform)?,
                ));
            }
        }
    }
    Err(Error::Unavailable)
}
