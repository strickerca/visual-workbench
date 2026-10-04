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
    if !request.explicit_owner_action {
        return Err(Error::Consent);
    }
    request.limits.validate()?;
    request.target.validate(request.owner_process_id)?;
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
    admit_sdr(&dxgi, &request.target)?;
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
    let pixels = readback(&device, &context, &texture, &request, cancel)?;
    request.target.unchanged(
        &fixed_target(request.target.window, request.owner_process_id)?,
        request.owner_process_id,
    )?;
    cancel.check()?;
    let (asset, bytes) = publish_png(
        Path::new(&request.output_directory),
        request.target.client.width,
        request.target.client.height,
        &pixels,
        request.limits,
        cancel,
    )?;
    let width = request.target.client.width;
    let height = request.target.client.height;
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
    Ok(FrameReceipt {
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
    })
}
fn admit_sdr(device: &IDXGIDevice, target: &WindowTarget) -> Result<()> {
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
fn readback(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
    request: &CaptureRequest,
    cancel: &Cancellation,
) -> Result<Vec<u8>> {
    let expected = &request.target;
    let (x, y) = expected.client.offset_inside(expected.frame)?;
    let count = request
        .limits
        .image(expected.client.width, expected.client.height)?;
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
            for row in y..y + expected.client.height {
                cancel.check()?;
                let offset = (row as usize)
                    .checked_mul(mapped.RowPitch as usize)
                    .and_then(|n| n.checked_add(x as usize * 4))
                    .ok_or(Error::Limit)?;
                let data = std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(offset),
                    expected.client.width as usize * 4,
                );
                for pixel in data.as_chunks::<4>().0 {
                    if pixel[3] != 255 {
                        return Err(Error::ColorDepth);
                    }
                    rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                }
            }
            Ok(rgba)
        })();
        context.Unmap(&staging, 0);
        result
    }
}
