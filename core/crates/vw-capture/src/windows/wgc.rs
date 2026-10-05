use super::{DpiScope, api, fixed_target};
use crate::*;
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
struct Session {
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}
struct Frame(Direct3D11CaptureFrame);
impl Drop for Frame {
    fn drop(&mut self) {
        let _ = self.0.Close();
    }
}
pub fn capture(request: CaptureRequest, cancel: &Cancellation) -> Result<FrameReceipt> {
    let mut diagnostic = None;
    capture_diagnostic(request, cancel, &mut diagnostic)
}
pub fn capture_diagnostic(
    request: CaptureRequest,
    cancel: &Cancellation,
    diagnostic: &mut Option<ColorDepthDiagnostic>,
) -> Result<FrameReceipt> {
    let mut alpha_summary = None;
    capture_fixture_diagnostic(request, cancel, diagnostic, &mut alpha_summary)
}
pub fn capture_fixture_diagnostic(
    request: CaptureRequest,
    cancel: &Cancellation,
    diagnostic: &mut Option<ColorDepthDiagnostic>,
    alpha_summary: &mut Option<AlphaSummary>,
) -> Result<FrameReceipt> {
    match capture_selected(request, cancel, diagnostic, alpha_summary, None)? {
        Captured::FullClient(value) => Ok(value),
        Captured::Canvas(_) => Err(Error::Invalid),
    }
}
pub fn capture_canvas_fixture(
    request: CanvasCaptureRequest,
    cancel: &Cancellation,
    diagnostic: &mut Option<ColorDepthDiagnostic>,
) -> Result<CanvasFrameReceipt> {
    request.validate()?;
    let mut alpha_summary = None;
    match capture_selected(
        request.capture,
        cancel,
        diagnostic,
        &mut alpha_summary,
        Some(request.canvas_rect_host),
    )? {
        Captured::Canvas(value) => Ok(value),
        Captured::FullClient(_) => Err(Error::Invalid),
    }
}
enum Captured {
    FullClient(FrameReceipt),
    Canvas(CanvasFrameReceipt),
}
fn capture_selected(
    request: CaptureRequest,
    cancel: &Cancellation,
    diagnostic: &mut Option<ColorDepthDiagnostic>,
    alpha_summary: &mut Option<AlphaSummary>,
    canvas: Option<Rect>,
) -> Result<Captured> {
    *diagnostic = None;
    *alpha_summary = None;
    let output_rect = canvas.unwrap_or(request.target.client);
    if !request.explicit_owner_action {
        return Err(Error::Consent);
    }
    request.limits.validate()?;
    request.target.validate(request.owner_process_id)?;
    if let Some(region) = request.diagnostic_alpha_region {
        if !request.diagnostic_color_stage {
            return Err(Error::Invalid);
        }
        region.validate(request.target.client.width, request.target.client.height)?;
    }
    vw_model::Id::try_from(request.capture_session_id.clone()).map_err(|_| Error::Invalid)?;
    plain(Path::new(&request.output_directory), true)?;
    request
        .limits
        .image(request.target.frame.width, request.target.frame.height)?;
    let _dpi = DpiScope::enter()?;
    request.target.unchanged(
        &fixed_target(request.target.window, request.owner_process_id)?,
        request.owner_process_id,
    )?;
    cancel.check()?;
    if !api(GraphicsCaptureSession::IsSupported())? {
        return Err(Error::Unsupported);
    }
    let (mut device, mut context) = (None, None);
    // SAFETY: initialized COM output slots; no application-owned texture pointer.
    unsafe {
        api(D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        ))?;
    }
    let device = device.ok_or(Error::Platform)?;
    let context = context.ok_or(Error::Platform)?;
    let dxgi: IDXGIDevice = api(device.cast())?;
    admit_sdr(
        &dxgi,
        &request.target,
        request.diagnostic_color_stage,
        diagnostic,
    )?;
    let factory: IGraphicsCaptureItemInterop = api(::windows::core::factory::<
        GraphicsCaptureItem,
        IGraphicsCaptureItemInterop,
    >())?;
    // SAFETY: validated target HWND and live D3D device are borrowed synchronously.
    let (item, direct): (GraphicsCaptureItem, IDirect3DDevice) = unsafe {
        (
            api(factory.CreateForWindow(HWND(request.target.window as usize as *mut _)))?,
            api(api(CreateDirect3D11DeviceFromDXGIDevice(&dxgi))?.cast())?,
        )
    };
    let size = api(item.Size())?;
    if size.Width != request.target.frame.width as i32
        || size.Height != request.target.frame.height as i32
    {
        return Err(Error::Stale);
    }
    let pool = api(Direct3D11CaptureFramePool::CreateFreeThreaded(
        &direct,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        1,
        size,
    ))?;
    let session = api(pool.CreateCaptureSession(&item))?;
    // These Windows11 24H2 properties fail closed on an older API set.
    api(session.SetMinUpdateInterval(TimeSpan { Duration: 166_667 }))?;
    api(session.SetIncludeSecondaryWindows(true))?;
    api(session.SetIsCursorCaptureEnabled(false))?;
    // Keep the OS indicator: no permission request or borderless override occurs.
    api(session.SetIsBorderRequired(true))?;
    api(session.StartCapture())?;
    let capture = Session { session, pool };
    let start = Instant::now();
    let frame = loop {
        cancel.check()?;
        if start.elapsed() > Duration::from_millis(request.limits.capture_ms) {
            return Err(Error::Timeout);
        }
        match capture.pool.TryGetNextFrame() {
            Ok(value) => break Frame(value),
            Err(error) if error.code() == E_POINTER || error.code() == S_OK => {
                std::thread::sleep(Duration::from_millis(1))
            }
            Err(_) => return Err(Error::Platform),
        }
    };
    let actual = api(frame.0.ContentSize())?;
    if actual != size {
        return Err(Error::Stale);
    }
    let time = api(frame.0.SystemRelativeTime())?.Duration;
    if time <= 0 {
        return Err(Error::Platform);
    }
    let frame_ns = u64::try_from(time)
        .map_err(|_| Error::Limit)?
        .checked_mul(100)
        .ok_or(Error::Limit)?;
    if frame_ns < request.target.observed_ns {
        return Err(Error::Stale);
    }
    let access: IDirect3DDxgiInterfaceAccess = api(api(frame.0.Surface())?.cast())?;
    // SAFETY: captured frame retains ownership while readback acquires its texture.
    let texture: ID3D11Texture2D = unsafe { api(access.GetInterface())? };
    let pixels = readback(
        &device,
        &context,
        &texture,
        &request,
        cancel,
        (&mut *diagnostic, &mut *alpha_summary),
        ReadbackSelection {
            started: start,
            rect: output_rect,
            canvas_only: canvas.is_some(),
        },
    );
    if pixels.is_err() {
        let _ = crate::alpha_diagnostic::retain_after_target_check(alpha_summary, || {
            cancel.check()?;
            if start.elapsed() > Duration::from_millis(request.limits.capture_ms) {
                return Err(Error::Timeout);
            }
            request.target.unchanged(
                &fixed_target(request.target.window, request.owner_process_id)?,
                request.owner_process_id,
            )?;
            cancel.check()?;
            if start.elapsed() > Duration::from_millis(request.limits.capture_ms) {
                return Err(Error::Timeout);
            }
            Ok(())
        });
    }
    let pixels = pixels?;
    request.target.unchanged(
        &fixed_target(request.target.window, request.owner_process_id)?,
        request.owner_process_id,
    )?;
    cancel.check()?;
    let width = output_rect.width;
    let height = output_rect.height;
    let (asset, bytes) = if canvas.is_some() {
        crate::image::publish_canvas_png(
            Path::new(&request.output_directory),
            width,
            height,
            &pixels,
            request.limits,
            cancel,
        )?
    } else {
        publish_png(
            Path::new(&request.output_directory),
            width,
            height,
            &pixels,
            request.limits,
            cancel,
        )?
    };
    if canvas.is_some() {
        crate::canvas::final_target_check(
            cancel,
            || start.elapsed() <= Duration::from_millis(request.limits.capture_ms),
            || {
                request.target.unchanged(
                    &fixed_target(request.target.window, request.owner_process_id)?,
                    request.owner_process_id,
                )
            },
        )?;
    }
    let observed = super::clock_ns()?;
    let age = observed.checked_sub(frame_ns).ok_or(Error::Stale)? / 1_000_000;
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Platform)?
        .as_millis();
    let captured_at_ms = i64::try_from(wall.checked_sub(u128::from(age)).ok_or(Error::Stale)?)
        .map_err(|_| Error::Limit)?;
    let identity = FrameIdentity {
        capture_session_id: request.capture_session_id,
        frame_id: 1,
        geometry_revision: 1,
        source_kind: "window".into(),
        client_rect: request.target.client,
        dpi_scale: f64::from(request.target.dpi) / 96.0,
        monotonic_timestamp_ns: frame_ns,
        captured_at_ms,
        platform: "windows".into(),
        window_handle: request.target.window,
        monitor_id: String::new(),
    };
    identity.validate()?;
    if let Some(canvas_rect_host) = canvas {
        let (x, y) = canvas_rect_host.offset_inside(request.target.frame)?;
        crate::canvas::completion_check(cancel, &mut || {
            start.elapsed() <= Duration::from_millis(request.limits.capture_ms)
        })?;
        return Ok(Captured::Canvas(CanvasFrameReceipt {
            source_identity: identity,
            target: request.target,
            canvas_rect_host,
            crop_in_frame: AlphaRegion {
                x,
                y,
                width,
                height,
            },
            source_asset_id: asset,
            png_bytes: bytes,
            width,
            height,
            bit_depth: 8,
            border_visible: true,
            lossless: true,
            filename: "capture-canvas.png".into(),
        }));
    }
    Ok(Captured::FullClient(FrameReceipt {
        identity,
        target: request.target,
        source_asset_id: asset,
        png_bytes: bytes,
        width,
        height,
        bit_depth: 8,
        border_visible: true,
        lossless: true,
        filename: "capture.png".into(),
    }))
}
fn admit_sdr(
    device: &IDXGIDevice,
    target: &WindowTarget,
    enabled: bool,
    diagnostic: &mut Option<ColorDepthDiagnostic>,
) -> Result<()> {
    // SDR8 is admitted only when the actual target monitor reports RGB/P709.
    // HDR/wide-gamut captures need an explicit high-depth adapter, not clipping.
    unsafe {
        let monitor = MonitorFromWindow(
            HWND(target.window as usize as *mut _),
            MONITOR_DEFAULTTONULL,
        );
        if monitor.0.is_null() {
            return Err(Error::Unavailable);
        }
        let adapter = api(device.GetAdapter())?;
        let mut enumerated_outputs = 0;
        for index in 0..32 {
            let output = match adapter.EnumOutputs(index) {
                Ok(value) => value,
                Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(_) => return Err(Error::Platform),
            };
            enumerated_outputs += 1;
            let output: IDXGIOutput6 = api(output.cast())?;
            let desc = api(output.GetDesc1())?;
            if desc.Monitor == monitor {
                return if desc.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709 {
                    Ok(())
                } else {
                    if enabled {
                        *diagnostic = Some(ColorDepthDiagnostic::MonitorColorSpace {
                            color_space: desc.ColorSpace.0,
                        });
                    }
                    Err(Error::ColorDepth)
                };
            }
        }
        if enabled {
            *diagnostic = Some(ColorDepthDiagnostic::MonitorOutputMissing { enumerated_outputs });
        }
    }
    Err(Error::ColorDepth)
}
struct ReadbackSelection {
    started: Instant,
    rect: Rect,
    canvas_only: bool,
}
fn readback(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
    request: &CaptureRequest,
    cancel: &Cancellation,
    diagnostics: (&mut Option<ColorDepthDiagnostic>, &mut Option<AlphaSummary>),
    selection: ReadbackSelection,
) -> Result<Vec<u8>> {
    let (diagnostic, alpha_summary) = diagnostics;
    let capture_start = selection.started;
    let output_rect = selection.rect;
    let canvas_only = selection.canvas_only;
    let expected = &request.target;
    let (x, y) = output_rect.offset_inside(expected.frame)?;
    let count = request
        .limits
        .image(output_rect.width, output_rect.height)?;
    // SAFETY: texture metadata, stride, offsets and total allocation are checked
    // before mapped pointer reads. Unmap executes on every return from the closure.
    unsafe {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        texture.GetDesc(&mut desc);
        if desc.Width != expected.frame.width
            || desc.Height != expected.frame.height
            || desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
            || desc.ArraySize != 1
            || desc.MipLevels != 1
            || desc.SampleDesc.Count != 1
        {
            if request.diagnostic_color_stage {
                *diagnostic = Some(ColorDepthDiagnostic::TextureDescriptor {
                    width: desc.Width,
                    height: desc.Height,
                    format: desc.Format.0,
                    array_size: desc.ArraySize,
                    mip_levels: desc.MipLevels,
                    sample_count: desc.SampleDesc.Count,
                });
            }
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
            let mut scanner = request
                .diagnostic_alpha_region
                .map(|region| {
                    crate::alpha_diagnostic::AlphaScanner::new(
                        expected.client.width,
                        expected.client.height,
                        region,
                    )
                })
                .transpose()?;
            let mut first_bad_alpha = None;
            for row in y..y + output_rect.height {
                cancel.check()?;
                if (scanner.is_some() || canvas_only)
                    && capture_start.elapsed() > Duration::from_millis(request.limits.capture_ms)
                {
                    return Err(Error::Timeout);
                }
                let offset = (row as usize)
                    .checked_mul(mapped.RowPitch as usize)
                    .ok_or(Error::Limit)?;
                // Full mapped row length fits its checked pitch/texture. The pure
                // selector exposes only the exact full-client or verified canvas crop.
                let mapped_row = std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(offset),
                    desc.Width as usize * 4,
                );
                let data = crate::canvas::selected_row(mapped_row, x, output_rect.width)?;
                if canvas_only {
                    crate::canvas::canvas_rgba(
                        data,
                        request.diagnostic_color_stage,
                        diagnostic,
                        &mut rgba,
                    )?;
                    continue;
                }
                if let Some(scan) = scanner.as_mut() {
                    scan.row(row - y, data, cancel)?;
                }
                for pixel in data.as_chunks::<4>().0 {
                    if pixel[3] != 255 {
                        if first_bad_alpha.is_none() {
                            first_bad_alpha = Some(pixel[3]);
                            if request.diagnostic_color_stage {
                                *diagnostic =
                                    Some(ColorDepthDiagnostic::NonopaquePixel { alpha: pixel[3] });
                            }
                        }
                        if scanner.is_none() {
                            return Err(Error::ColorDepth);
                        }
                    }
                    if first_bad_alpha.is_none() {
                        rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                    }
                }
            }
            if let Some(scan) = scanner {
                cancel.check()?;
                if capture_start.elapsed() > Duration::from_millis(request.limits.capture_ms) {
                    return Err(Error::Timeout);
                }
                *alpha_summary = scan.finish()?;
            }
            if first_bad_alpha.is_some() {
                return Err(Error::ColorDepth);
            }
            Ok(rgba)
        })();
        context.Unmap(&staging, 0);
        result
    }
}
