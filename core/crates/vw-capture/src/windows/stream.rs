//! One persistent WGC session in the private helper. No image or native frame
//! survives the producer's charged readback/publication boundary.
use super::{DpiScope, api};
use crate::{
    Error, Result,
    picker::PickerSource,
    stream::{CaptureStamp, StreamRequest, StreamSource, stream_error},
};
use ::windows::{
    Foundation::TimeSpan,
    Graphics::{
        Capture::*,
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
    },
    Win32::{
        Foundation::*,
        Graphics::{
            Direct3D11::*,
            Dxgi::{Common::*, *},
            Gdi::*,
        },
        System::WinRT::{Direct3D11::*, Graphics::Capture::IGraphicsCaptureItemInterop},
    },
    core::Interface,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use vw_stream::{Cancellation, MemoryCharge, MemoryKind, SessionBudget};

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

pub struct WgcStream {
    request: StreamRequest,
    budget: SessionBudget,
    _dpi: DpiScope,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    dxgi: IDXGIDevice,
    _item: GraphicsCaptureItem,
    _direct: IDirect3DDevice,
    pool: Pool,
    session: Session,
    size: ::windows::Graphics::SizeInt32,
    pending: Option<Frame>,
    _native_charge: MemoryCharge,
    _request_charge: MemoryCharge,
}
impl Drop for WgcStream {
    fn drop(&mut self) {
        self.pending = None;
        let _ = self.session.0.Close();
        let _ = self.pool.0.Close();
    }
}
impl WgcStream {
    pub fn open(
        request: StreamRequest,
        budget: SessionBudget,
        cancel: &Cancellation,
    ) -> Result<Self> {
        request.validate()?;
        cancel.check().map_err(stream_error)?;
        if !api(GraphicsCaptureSession::IsSupported())? {
            return Err(Error::Unsupported);
        }
        let _dpi = DpiScope::enter()?;
        check_source(&request)?;
        let full = full_rect(&request);
        let stride_cap = u64::from(full.width)
            .checked_mul(4)
            .and_then(|n| n.checked_add(4096))
            .ok_or(Error::Limit)?;
        let foreign_frame = stride_cap
            .checked_mul(u64::from(full.height))
            .ok_or(Error::Limit)?;
        // The WGC pool frame and its readback texture are foreign allocations.
        // Keep pool admission from before CreateFreeThreaded through session close.
        let native_charge = budget
            .reserve(
                MemoryKind::Foreign,
                foreign_frame.checked_mul(2).ok_or(Error::Limit)?,
            )
            .map_err(stream_error)?;
        let request_charge = budget
            .reserve(MemoryKind::Foreign, 16 * 1024)
            .map_err(stream_error)?;
        // Bind D3D to the adapter of the actually selected source output.
        // A default GPU may be on another adapter and cannot prove SDR/source
        // identity for this monitor or window.
        let monitor = selected_monitor(&request)?;
        let (device, context) = super::picker_capture::device_for_monitor(monitor)?;
        let dxgi: IDXGIDevice = api(device.cast())?;
        require_sdr(&dxgi, &request)?;
        let factory: IGraphicsCaptureItemInterop = api(::windows::core::factory::<
            GraphicsCaptureItem,
            IGraphicsCaptureItemInterop,
        >())?;
        let direct: IDirect3DDevice =
            unsafe { api(api(CreateDirect3D11DeviceFromDXGIDevice(&dxgi))?.cast())? };
        // SAFETY: source HWND/HMONITOR was rechecked; WGC retains an owned item.
        let item: GraphicsCaptureItem = unsafe {
            match &request.source {
                PickerSource::Window { target } => {
                    api(factory.CreateForWindow(HWND(target.window as usize as *mut _)))?
                }
                PickerSource::Monitor { target } | PickerSource::Region { target, .. } => {
                    api(factory.CreateForMonitor(HMONITOR(target.monitor as usize as *mut _)))?
                }
            }
        };
        let size = api(item.Size())?;
        if size.Width != full.width as i32 || size.Height != full.height as i32 {
            return Err(Error::Stale);
        }
        let pool = Pool(api(Direct3D11CaptureFramePool::CreateFreeThreaded(
            &direct,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            1,
            size,
        ))?);
        let session = Session(api(pool.0.CreateCaptureSession(&item))?);
        api(session.0.SetMinUpdateInterval(TimeSpan {
            Duration: i64::from(10_000_000u32.div_ceil(request.target_fps)),
        }))?;
        api(session.0.SetIncludeSecondaryWindows(true))?;
        api(session.0.SetIsCursorCaptureEnabled(false))?;
        if api(session.0.IsCursorCaptureEnabled())? {
            return Err(Error::Unavailable);
        }
        api(session.0.SetIsBorderRequired(true))?;
        check_source(&request)?;
        cancel.check().map_err(stream_error)?;
        api(session.0.StartCapture())?;
        Ok(Self {
            request,
            budget,
            _dpi,
            device,
            context,
            dxgi,
            _item: item,
            _direct: direct,
            pool,
            session,
            size,
            pending: None,
            _native_charge: native_charge,
            _request_charge: request_charge,
        })
    }
}
impl StreamSource for WgcStream {
    fn inspect(&mut self, cancel: &Cancellation) -> Result<()> {
        cancel.check().map_err(stream_error)?;
        check_source(&self.request)?;
        require_sdr(&self.dxgi, &self.request)?;
        if api(self.session.0.IsCursorCaptureEnabled())? {
            return Err(Error::Unavailable);
        }
        Ok(())
    }
    fn prepare(&mut self, timeout_ms: u64, cancel: &Cancellation) -> Result<Option<CaptureStamp>> {
        self.pending = None;
        check_source(&self.request)?;
        require_sdr(&self.dxgi, &self.request)?;
        if api(self.session.0.IsCursorCaptureEnabled())? {
            return Err(Error::Unavailable);
        }
        let started = Instant::now();
        let frame = loop {
            cancel.check().map_err(stream_error)?;
            if started.elapsed() >= Duration::from_millis(timeout_ms) {
                return Ok(None);
            }
            match self.pool.0.TryGetNextFrame() {
                Ok(value) => break Frame(value),
                Err(error) if error.code() == E_POINTER || error.code() == S_OK => {
                    std::thread::sleep(Duration::from_millis(1))
                }
                Err(_) => return Err(Error::Platform),
            }
        };
        if api(frame.0.ContentSize())? != self.size {
            return Err(Error::Stale);
        }
        let ticks = api(frame.0.SystemRelativeTime())?.Duration;
        let ns = u64::try_from(ticks)
            .ok()
            .and_then(|n| n.checked_mul(100))
            .ok_or(Error::Limit)?;
        let now = super::clock_ns()?;
        if ns < self.request.source.observed_ns() || ns > now {
            return Err(Error::Stale);
        }
        let age_ms = (now - ns) / 1_000_000;
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Platform)?
            .as_millis();
        let captured_at_ms =
            i64::try_from(wall.checked_sub(u128::from(age_ms)).ok_or(Error::Stale)?)
                .map_err(|_| Error::Limit)?;
        check_source(&self.request)?;
        self.pending = Some(frame);
        Ok(Some(CaptureStamp {
            timestamp_ns: ns,
            captured_at_ms,
        }))
    }
    fn read_into(&mut self, admitted_rgba: &mut [u8], cancel: &Cancellation) -> Result<()> {
        let frame = self.pending.as_ref().ok_or(Error::Invalid)?;
        let selected = self.request.source.rect();
        let full = full_rect(&self.request);
        let (x, y) = selected.offset_inside(full)?;
        let expected =
            vw_stream::rgba_bytes(selected.width, selected.height).map_err(stream_error)?;
        if admitted_rgba.len() != expected {
            return Err(Error::Invalid);
        }
        let access: IDirect3DDxgiInterfaceAccess = api(api(frame.0.Surface())?.cast())?;
        // SAFETY: the pending WGC frame remains owned throughout synchronous readback.
        let texture: ID3D11Texture2D = unsafe { api(access.GetInterface())? };
        // SAFETY: descriptor and mapped pitch are checked before every pointer row.
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            if desc.Width != full.width
                || desc.Height != full.height
                || desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
                || desc.ArraySize != 1
                || desc.MipLevels != 1
                || desc.SampleDesc.Count != 1
            {
                return Err(Error::ColorDepth);
            }
            let row_cap = u64::from(desc.Width)
                .checked_mul(4)
                .and_then(|n| n.checked_add(4096))
                .ok_or(Error::Limit)?;
            let staging_cap = row_cap
                .checked_mul(u64::from(desc.Height))
                .ok_or(Error::Limit)?;
            let _staging_charge = self
                .budget
                .reserve(MemoryKind::Foreign, staging_cap)
                .map_err(stream_error)?;
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            desc.MiscFlags = 0;
            let mut staging = None;
            api(self.device.CreateTexture2D(&desc, None, Some(&mut staging)))?;
            let staging = staging.ok_or(Error::Platform)?;
            self.context.CopyResource(&staging, &texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            api(self
                .context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)))?;
            let result = (|| {
                if mapped.pData.is_null()
                    || mapped.RowPitch < desc.Width.checked_mul(4).ok_or(Error::Limit)?
                {
                    return Err(Error::Limit);
                }
                // Row pitch can exceed the nominal GPU allocation; refuse if its
                // true footprint has not been admitted by the foreign reservation.
                if u64::from(mapped.RowPitch) * u64::from(desc.Height) > staging_cap {
                    return Err(Error::Limit);
                }
                for row in 0..selected.height {
                    cancel.check().map_err(stream_error)?;
                    let offset = (y as usize + row as usize)
                        .checked_mul(mapped.RowPitch as usize)
                        .and_then(|n| n.checked_add(x as usize * 4))
                        .ok_or(Error::Limit)?;
                    let source = std::slice::from_raw_parts(
                        (mapped.pData as *const u8).add(offset),
                        selected.width as usize * 4,
                    );
                    let destination = &mut admitted_rgba[row as usize * selected.width as usize * 4
                        ..(row as usize + 1) * selected.width as usize * 4];
                    for (from, to) in source
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(destination.as_chunks_mut::<4>().0)
                    {
                        if from[3] != 255 {
                            return Err(Error::ColorDepth);
                        }
                        to.copy_from_slice(&[from[2], from[1], from[0], 255]);
                    }
                }
                check_source(&self.request)?;
                require_sdr(&self.dxgi, &self.request)?;
                Ok(())
            })();
            self.context.Unmap(&staging, 0);
            result
        }
    }
    fn discard(&mut self) {
        self.pending = None;
    }
}

fn full_rect(request: &StreamRequest) -> crate::Rect {
    match &request.source {
        PickerSource::Window { target } => target.frame,
        PickerSource::Monitor { target } | PickerSource::Region { target, .. } => target.physical,
    }
}
fn check_source(request: &StreamRequest) -> Result<()> {
    match &request.source {
        PickerSource::Window { target } => {
            target.unchanged(
                &super::fixed_target(target.window, request.owner_process_id)?,
                request.owner_process_id,
            )?;
            super::picker::require_own_windows_excluded(request.owner_process_id)
        }
        PickerSource::Monitor { target } | PickerSource::Region { target, .. } => {
            target.unchanged(&super::picker::monitor(target.monitor)?)?;
            super::picker::require_own_windows_excluded(request.owner_process_id)
        }
    }
}
fn require_sdr(device: &IDXGIDevice, request: &StreamRequest) -> Result<()> {
    let monitor = selected_monitor(request)?;
    // SAFETY: adapter/output interfaces are owned COM values, not raw display pointers.
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
    Err(Error::Unavailable)
}
fn selected_monitor(request: &StreamRequest) -> Result<HMONITOR> {
    let monitor = match &request.source {
        PickerSource::Window { target } => {
            // SAFETY: the selected HWND is revalidated before this query.
            unsafe {
                MonitorFromWindow(
                    HWND(target.window as usize as *mut _),
                    MONITOR_DEFAULTTONULL,
                )
            }
        }
        PickerSource::Monitor { target } | PickerSource::Region { target, .. } => {
            HMONITOR(target.monitor as usize as *mut _)
        }
    };
    if monitor.0.is_null() {
        return Err(Error::Unavailable);
    }
    Ok(monitor)
}
