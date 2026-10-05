//! WGC -> GPU client crop/padding -> NV12 -> hardware MFT. No CPU readback.
use super::{api, confine, encoder::Encoder, pump, required, unchanged};
use crate::{Error, Result};
use std::{
    io,
    time::{Duration, Instant},
};
use vw_remote::{
    Scope, Target, VideoConfig, annexb,
    wire::{self, Command, Header, Packet},
};
use windows::{
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
        Media::MediaFoundation::*,
        System::WinRT::{Direct3D11::*, Graphics::Capture::*, *},
    },
    core::Interface,
};
struct Runtime;
impl Runtime {
    fn start() -> Result<Self> {
        /* SAFETY: this owned helper thread initializes its apartment and MF, paired on all success/failure paths. */
        unsafe {
            api("RoInitialize", RoInitialize(RO_INIT_MULTITHREADED))?;
            if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                RoUninitialize();
                return api("MFStartup", Err(e));
            }
            Ok(Self)
        }
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        /* SAFETY: same thread that successfully initialized. */
        unsafe {
            let _ = MFShutdown();
            RoUninitialize();
        }
    }
}
pub struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub adapter_vendor: u32,
}
impl Gpu {
    fn new(target: &Target) -> Result<Self> {
        let (mut device, mut context) = (None, None);
        // SAFETY: initialized outputs; hardware only, no WARP fallback.
        unsafe {
            api(
                "D3D hardware",
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_HARDWARE,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                    Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                ),
            )?;
            let device = required(device)?;
            let context = required(context)?;
            let protection: ID3D11Multithread = api("D3D protection", context.cast())?;
            let _ = protection.SetMultithreadProtected(true);
            let dxgi: IDXGIDevice = api("DXGI", device.cast())?;
            let adapter = api("adapter", dxgi.GetAdapter())?;
            let adapter1: IDXGIAdapter1 = api("adapter descriptor", adapter.cast())?;
            let desc = api("adapter vendor", adapter1.GetDesc1())?;
            if desc.VendorId != 0x8086 {
                return Err(Error::Unavailable);
            }
            let monitor = MonitorFromWindow(
                HWND(target.window as usize as *mut _),
                MONITOR_DEFAULTTONULL,
            );
            if monitor.0.is_null() {
                return Err(Error::TargetChanged);
            }
            let mut admitted = false;
            for index in 0..32 {
                let output = match adapter.EnumOutputs(index) {
                    Ok(v) => v,
                    Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                    Err(e) => return api("monitor output", Err(e)),
                };
                let output6: IDXGIOutput6 = api("SDR descriptor", output.cast())?;
                let current = api("output descriptor", output6.GetDesc1())?;
                if current.Monitor == monitor {
                    if current.ColorSpace != DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709 {
                        return Err(Error::Unavailable);
                    }
                    admitted = true;
                    break;
                }
            }
            if !admitted {
                return Err(Error::Unavailable);
            }
            Ok(Self {
                device,
                context,
                adapter_vendor: desc.VendorId,
            })
        }
    }
}
struct Capture {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    width: u32,
    height: u32,
    last_qpc: u64,
}
struct Captured(Direct3D11CaptureFrame);
impl Drop for Captured {
    fn drop(&mut self) {
        let _ = self.0.Close();
    }
}
impl Capture {
    fn new(gpu: &Gpu, target: &Target) -> Result<Self> {
        if !api("WGC support", GraphicsCaptureSession::IsSupported())? {
            return Err(Error::Unavailable);
        }
        let factory: IGraphicsCaptureItemInterop = api(
            "WGC factory",
            windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>(),
        )?;
        // SAFETY: already pinned/revalidated HWND and live owned D3D device.
        let (item, direct): (GraphicsCaptureItem, IDirect3DDevice) = unsafe {
            let item = api(
                "selected window capture",
                factory.CreateForWindow(HWND(target.window as usize as *mut _)),
            )?;
            let dxgi: IDXGIDevice = api("capture DXGI", gpu.device.cast())?;
            (
                item,
                api(
                    "capture device",
                    api("WinRT device", CreateDirect3D11DeviceFromDXGIDevice(&dxgi))?.cast(),
                )?,
            )
        };
        let size = api("capture bounds", item.Size())?;
        if size.Width != target.frame_rect.width as i32
            || size.Height != target.frame_rect.height as i32
        {
            return Err(Error::TargetChanged);
        }
        let pool = api(
            "WGC frame pool",
            Direct3D11CaptureFramePool::CreateFreeThreaded(
                &direct,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                size,
            ),
        )?;
        let session = api("WGC session", pool.CreateCaptureSession(&item))?;
        let configure = (|| {
            api(
                "WGC 30Hz request",
                session.SetMinUpdateInterval(TimeSpan { Duration: 333_333 }),
            )?;
            api("capture cursor", session.SetIsCursorCaptureEnabled(false))?;
            api(
                "secondary windows",
                session.SetIncludeSecondaryWindows(true),
            )?;
            api("visible capture border", session.SetIsBorderRequired(true))?;
            api("start capture", session.StartCapture())
        })();
        if let Err(e) = configure {
            let _ = session.Close();
            let _ = pool.Close();
            return Err(e);
        }
        // The first static-window frame may precede the StartCapture return.
        // Exact target is revalidated around encoding; future injection receipts
        // remain fenced by this frame's real compositor timestamp.
        Ok(Self {
            pool,
            session,
            width: target.frame_rect.width,
            height: target.frame_rect.height,
            last_qpc: 0,
        })
    }
    fn next(&mut self) -> Result<Option<(Captured, u64)>> {
        let start = Instant::now();
        let mut latest = None;
        loop {
            pump();
            // Drain only the bounded two-frame pool, preserving the newest exact frame.
            for _ in 0..3 {
                match self.pool.TryGetNextFrame() {
                    Ok(frame) => {
                        let frame = Captured(frame);
                        let size = api("captured size", frame.0.ContentSize())?;
                        if size.Width != self.width as i32 || size.Height != self.height as i32 {
                            return Err(Error::TargetChanged);
                        }
                        let qpc = api("compositor QPC", frame.0.SystemRelativeTime())?.Duration;
                        if qpc <= 0 {
                            return Err(Error::Invalid);
                        }
                        let qpc = qpc as u64;
                        if qpc > self.last_qpc
                            && latest.as_ref().is_none_or(|(_, prior)| qpc > *prior)
                        {
                            latest = Some((frame, qpc))
                        }
                    }
                    Err(e) if e.code() == E_POINTER || e.code() == S_OK => break,
                    Err(e) => return api("capture next", Err(e)),
                }
            }
            if let Some((frame, qpc)) = latest.take() {
                self.last_qpc = qpc;
                return Ok(Some((frame, qpc)));
            }
            if start.elapsed() >= Duration::from_millis(40) {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}
fn texture(frame: &Captured) -> Result<ID3D11Texture2D> {
    let access: IDirect3DDxgiInterfaceAccess = api(
        "surface access",
        api("capture surface", frame.0.Surface())?.cast(),
    )?; /* SAFETY: frame retains this live surface through encoding, acquiring an owning COM reference. */
    unsafe { api("captured GPU texture", access.GetInterface()) }
}
struct Video {
    target: Target,
    scope: Scope,
    owner_pid: u32,
    encoder: Encoder,
    capture: Capture,
    _gpu: Gpu,
    _runtime: Runtime,
    config: Option<VideoConfig>,
    frame_id: u64,
    last_capture_restart: Instant,
    capture_recovery: crate::capture_recovery::CaptureRecovery,
}
impl Video {
    fn open(target: Target, scope: Scope, owner_pid: u32) -> Result<Self> {
        target.validate()?;
        scope.validate()?;
        if scope.target_token != target.token {
            return Err(Error::Invalid);
        }
        unchanged(&target, owner_pid)?;
        let runtime = Runtime::start()?;
        let gpu = Gpu::new(&target)?;
        let capture = Capture::new(&gpu, &target)?;
        let encoder = Encoder::new(&gpu, &target)?;
        Ok(Self {
            target,
            scope,
            owner_pid,
            encoder,
            capture,
            _gpu: gpu,
            _runtime: runtime,
            config: None,
            frame_id: 0,
            last_capture_restart: Instant::now(),
            capture_recovery: crate::capture_recovery::CaptureRecovery::default(),
        })
    }
    fn poll(&mut self, sequence: u64, force: bool) -> Result<Packet> {
        unchanged(&self.target, self.owner_pid)?;
        let force = force || self.config.is_none();
        let captured = self.capture.next()?;
        if captured.is_none()
            && self
                .capture_recovery
                .restart(force, self.last_capture_restart.elapsed())?
        {
            unchanged(&self.target, self.owner_pid)?;
            let prior_qpc = self.capture.last_qpc;
            // Close only this helper's owned capture session. No target input,
            // desktop invalidation, timestamp synthesis, or deadline extension.
            api(
                "restart capture session close",
                self.capture.session.Close(),
            )?;
            api("restart capture pool close", self.capture.pool.Close())?;
            let mut replacement = Capture::new(&self._gpu, &self.target)?;
            replacement.last_qpc = prior_qpc;
            self.capture = replacement;
            self.last_capture_restart = Instant::now();
            unchanged(&self.target, self.owner_pid)?;
            // Leave the original 40ms poll budget intact; the next normal
            // request observes the replacement pool with the retained QPC floor.
        }
        let Some((frame, qpc)) = captured else {
            return Ok(Packet {
                header: Header::Idle { sequence },
                payload: Vec::new(),
            });
        };
        self.capture_recovery.captured();
        if force {
            self.encoder.force_keyframe()?
        }
        let pts = i64::try_from(qpc).map_err(|_| Error::Limit)?;
        let output = self.encoder.encode(&texture(&frame)?, pts)?;
        if output.pts != pts {
            return Err(Error::Invalid);
        }
        let unit = annexb::access_unit(&output.bytes)?;
        if force && !unit.idr {
            return Err(Error::Unavailable);
        }
        if let Some(config) = &self.config
            && (unit.vps.is_some_and(|v| v != config.vps)
                || unit.sps.is_some_and(|v| v != config.sps)
                || unit.pps.is_some_and(|v| v != config.pps))
        {
            return Err(Error::TargetChanged);
        }
        if unit.idr {
            let current = self.config.as_ref();
            let vps = unit
                .vps
                .map(Vec::from)
                .or_else(|| current.map(|v| v.vps.clone()))
                .ok_or(Error::Unavailable)?;
            let sps = unit
                .sps
                .map(Vec::from)
                .or_else(|| current.map(|v| v.sps.clone()))
                .ok_or(Error::Unavailable)?;
            let pps = unit
                .pps
                .map(Vec::from)
                .or_else(|| current.map(|v| v.pps.clone()))
                .ok_or(Error::Unavailable)?;
            if current.is_some_and(|c| c.vps != vps || c.sps != sps || c.pps != pps) {
                return Err(Error::TargetChanged);
            }
            let (w, h) = self.target.coded_size()?;
            let config = VideoConfig {
                scope: self.scope.clone(),
                generation: 1,
                visible_width: self.target.client_rect.width,
                visible_height: self.target.client_rect.height,
                coded_width: w,
                coded_height: h,
                vps,
                sps,
                pps,
                encoder: self.encoder.capabilities(),
            };
            config.validate()?;
            self.config = Some(config);
        }
        let config = self.config.as_ref().ok_or(Error::Unavailable)?;
        unchanged(&self.target, self.owner_pid)?;
        self.frame_id = self.frame_id.checked_add(1).ok_or(Error::Limit)?;
        Ok(Packet {
            header: Header::Video {
                sequence,
                scope: self.scope.clone(),
                config_generation: config.generation,
                frame_id: self.frame_id,
                pts_100ns: pts,
                captured_qpc_100ns: qpc,
                keyframe: unit.idr,
                config: unit.idr.then(|| config.clone()),
                payload_bytes: output.bytes.len() as u32,
            },
            payload: output.bytes,
        })
    }
}
pub fn run() -> Result<()> {
    let _job = confine()?;
    super::native_guard::initialize_dpi().map_err(|_| Error::Unavailable)?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let command = wire::read_command(&mut input)?;
    let Command::OpenVideo {
        sequence: 1,
        target,
        scope,
        owner_pid,
    } = command
    else {
        return Err(Error::Invalid);
    };
    let mut video = match Video::open(target, scope, owner_pid) {
        Ok(v) => v,
        Err(error) => {
            wire::write_packet(
                &mut output,
                &Packet {
                    header: Header::Refused {
                        sequence: 1,
                        error: error.clone(),
                        finite_attempt: None,
                    },
                    payload: Vec::new(),
                },
            )?;
            return Err(error);
        }
    };
    wire::write_packet(
        &mut output,
        &Packet {
            header: Header::Ready {
                sequence: 1,
                capabilities: Some(video.encoder.capabilities()),
                input_authority: None,
            },
            payload: Vec::new(),
        },
    )?;
    let mut last = 1u64;
    loop {
        let command = wire::read_command(&mut input)?;
        let sequence = command.sequence();
        if sequence != last.checked_add(1).ok_or(Error::Limit)? {
            return Err(Error::Invalid);
        }
        last = sequence;
        let result = match command {
            Command::Poll { force_keyframe, .. } => video.poll(sequence, force_keyframe),
            Command::Stop { .. } => {
                drop(video);
                wire::write_packet(
                    &mut output,
                    &Packet {
                        header: Header::Stopped {
                            sequence,
                            pen_release: None,
                        },
                        payload: Vec::new(),
                    },
                )?;
                return Ok(());
            }
            _ => Err(Error::Invalid),
        };
        match result {
            Ok(packet) => wire::write_packet(&mut output, &packet)?,
            Err(error) => {
                wire::write_packet(
                    &mut output,
                    &Packet {
                        header: Header::Refused {
                            sequence,
                            error: error.clone(),
                            finite_attempt: None,
                        },
                        payload: Vec::new(),
                    },
                )?;
                return Err(error);
            }
        }
    }
}
