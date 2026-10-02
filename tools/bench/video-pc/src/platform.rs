//! Windows API boundary. Only windows created by this process are captured.
mod encoder;
use encoder::Encoder;
use image::ImageFormat;
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    mem::size_of,
    path::Path,
    time::{Duration, Instant},
};
use vw_video_bench::{Error, Result, byte_count, changed_regions, encode, measure_raster, stats};
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
            Dxgi::{Common::*, IDXGIDevice},
            Gdi::*,
        },
        Media::MediaFoundation::*,
        System::{
            LibraryLoader::GetModuleHandleW,
            WinRT::{Direct3D11::*, Graphics::Capture::*, *},
        },
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
    core::{Interface, w},
};

pub(super) fn api<T>(phase: &'static str, r: windows::core::Result<T>) -> Result<T> {
    r.map_err(|e| Error::Platform {
        phase,
        code: e.code().0 as u32,
    })
}
fn required<T>(v: Option<T>) -> Result<T> {
    v.ok_or(Error::Invalid)
}
fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer_pretty(file, value)?;
    Ok(())
}
struct Runtime;
impl Drop for Runtime {
    fn drop(&mut self) {
        // SAFETY: Balances successful MFStartup/RoInitialize on this same thread.
        unsafe {
            let _ = MFShutdown();
            RoUninitialize();
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_GETMINMAXINFO {
        // SAFETY: Windows supplies a valid writable MINMAXINFO for this message.
        // Allow the owned portrait fixture to extend beyond the desktop height.
        unsafe {
            let limits = &mut *(lparam.0 as *mut MINMAXINFO);
            limits.ptMaxSize = POINT { x: 8192, y: 8192 };
            limits.ptMaxTrackSize = POINT { x: 8192, y: 8192 };
        }
        return LRESULT(0);
    }
    if msg == WM_PAINT {
        // SAFETY: The callback HWND is alive; BeginPaint/EndPaint and created GDI
        // objects are paired. Text is generated fixture data, never a window title.
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut ps);
            if !dc.is_invalid() {
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                let brush = CreateSolidBrush(COLORREF(0x00292320));
                FillRect(dc, &rect, brush);
                let _ = DeleteObject(brush.into());
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(dc, COLORREF(0x00e8eee9));
                let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as u32;
                let sequence = state >> 8;
                let percent = if state == 0 { 100 } else { state & 255 };
                let changed = RECT {
                    left: 0,
                    top: 0,
                    right: rect.right,
                    bottom: rect.bottom * percent as i32 / 100,
                };
                let brush = CreateSolidBrush(COLORREF(0x00302020 + (sequence % 16) * 0x010101));
                FillRect(dc, &changed, brush);
                let _ = DeleteObject(brush.into());
                for y in (220..rect.bottom).step_by(68) {
                    let mut row: Vec<u16> = format!("Layer {:02}      Generated canvas text      0123456789      Aa Bb Cc      Properties", y / 68).encode_utf16().collect();
                    let mut bounds = RECT {
                        left: 32,
                        top: y,
                        right: rect.right - 32,
                        bottom: y + 48,
                    };
                    DrawTextW(dc, &mut row, &mut bounds, DT_LEFT | DT_TOP | DT_NOPREFIX);
                }
                let mut text:Vec<u16>=format!("Visual Workbench synthetic frame {sequence:06}\n\nWindow capture / HEVC / raster benchmark\nGenerated UI fixture - no user content\n\nLayers     Canvas     Properties\n\nToolbar     Select    Draw    Export").encode_utf16().collect();
                let mut text_rect = RECT {
                    left: 32,
                    top: 24,
                    right: rect.right - 32,
                    bottom: rect.bottom - 24,
                };
                DrawTextW(
                    dc,
                    &mut text,
                    &mut text_rect,
                    DT_LEFT | DT_TOP | DT_NOPREFIX,
                );
                let stripe = RECT {
                    left: 32,
                    top: 150,
                    right: (rect.right - 32).max(33),
                    bottom: 195,
                };
                let brush = CreateSolidBrush(COLORREF(0x00408000 | ((sequence * 17) & 255)));
                FillRect(dc, &stripe, brush);
                let _ = DeleteObject(brush.into());
            }
            let _ = EndPaint(hwnd, &ps);
        }
        return LRESULT(0);
    }
    // SAFETY: Forward unhandled native window messages with their original values.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

struct SourceWindow {
    hwnd: HWND,
    width: u32,
    height: u32,
}
impl SourceWindow {
    fn new(width: u32, height: u32) -> Result<Self> {
        byte_count(width, height, 4)?;
        // SAFETY: Static class strings/procedure remain valid. The window is
        // process-owned and created in physical pixels after PMv2 initialization.
        unsafe {
            let instance = api("GetModuleHandle", GetModuleHandleW(None))?;
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                lpszClassName: w!("VWVideoSyntheticSource"),
                ..Default::default()
            };
            let _ = RegisterClassExW(&class);
            let hwnd = api(
                "CreateWindow",
                CreateWindowExW(
                    WS_EX_TOOLWINDOW,
                    w!("VWVideoSyntheticSource"),
                    w!("Visual Workbench generated video fixture"),
                    WS_POPUP,
                    0,
                    0,
                    width as i32,
                    height as i32,
                    None,
                    None,
                    Some(instance.into()),
                    None,
                ),
            )?;
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(hwnd);
            Ok(Self {
                hwnd,
                width,
                height,
            })
        }
    }
    fn tick(&self, sequence: u32, percent: u32) {
        // SAFETY: Only our live window is invalidated. No focus or input injection.
        unsafe {
            SetWindowLongPtrW(
                self.hwnd,
                GWLP_USERDATA,
                ((sequence << 8) | percent) as isize,
            );
            let rect = RECT {
                left: 0,
                top: 0,
                right: self.width as i32,
                bottom: (self.height * percent / 100) as i32,
            };
            let _ = InvalidateRect(Some(self.hwnd), Some(&rect), false);
            let _ = UpdateWindow(self.hwnd);
        }
    }
}
impl Drop for SourceWindow {
    fn drop(&mut self) {
        // SAFETY: Destroy only the window created and owned by this object.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
fn pump() {
    // SAFETY: Pump this thread's messages without synthesizing input.
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

pub(super) struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
}
impl Gpu {
    fn new() -> Result<Self> {
        let (mut device, mut context) = (None, None);
        // SAFETY: API outputs are initialized and owned by COM wrappers. Hardware
        // is required; there is no silent WARP/software fallback for this spike.
        unsafe {
            api(
                "D3D11CreateDevice",
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
            let protected: ID3D11Multithread = api("D3D multithread", context.cast())?;
            let _ = protected.SetMultithreadProtected(true);
            Ok(Self { device, context })
        }
    }
    fn read_rgb(&self, texture: &ID3D11Texture2D, width: u32, height: u32) -> Result<Vec<u8>> {
        let count = byte_count(width, height, 3)?;
        // SAFETY: Texture metadata and mapped bounds are checked before access;
        // only the raster branch maps GPU data. HEVC never calls this method.
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            if desc.Width < width
                || desc.Height < height
                || desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
            {
                return Err(Error::Invalid);
            }
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            desc.MiscFlags = 0;
            let mut staging = None;
            api(
                "readback texture",
                self.device.CreateTexture2D(&desc, None, Some(&mut staging)),
            )?;
            let staging = required(staging)?;
            self.context.CopyResource(&staging, texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            api(
                "raster Map",
                self.context
                    .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)),
            )?;
            let result = (|| {
                if mapped.pData.is_null() || mapped.RowPitch < width * 4 {
                    return Err(Error::Invalid);
                }
                let mut rgb = Vec::with_capacity(count);
                for y in 0..height as usize {
                    let row = std::slice::from_raw_parts(
                        (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                        width as usize * 4,
                    );
                    for p in row.as_chunks::<4>().0 {
                        rgb.extend_from_slice(&[p[2], p[1], p[0]]);
                    }
                }
                Ok(rgb)
            })();
            self.context.Unmap(&staging, 0);
            result
        }
    }
}

struct Capture {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    width: u32,
    height: u32,
}
impl Capture {
    fn new(gpu: &Gpu, window: &SourceWindow) -> Result<Self> {
        if !api("WGC support", GraphicsCaptureSession::IsSupported())? {
            return Err(Error::Unsupported("Windows.Graphics.Capture"));
        }
        let factory: IGraphicsCaptureItemInterop = api(
            "WGC factory",
            windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>(),
        )?;
        // SAFETY: The interop receives only our own live window and D3D device.
        let (item, device): (GraphicsCaptureItem, IDirect3DDevice) = unsafe {
            let item = api("WGC CreateForWindow", factory.CreateForWindow(window.hwnd))?;
            let dxgi: IDXGIDevice = api("DXGI device", gpu.device.cast())?;
            let device = api(
                "WinRT D3D device",
                CreateDirect3D11DeviceFromDXGIDevice(&dxgi),
            )?;
            (item, api("WinRT D3D cast", device.cast())?)
        };
        let size = api("capture size", item.Size())?;
        if size.Width != window.width as i32 || size.Height != window.height as i32 {
            return Err(Error::Unsupported(
                "native source size differs from requested dimensions",
            ));
        }
        let pool = api(
            "WGC pool",
            Direct3D11CaptureFramePool::CreateFreeThreaded(
                &device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                size,
            ),
        )?;
        let session = api("WGC session", pool.CreateCaptureSession(&item))?;
        api(
            "WGC MinUpdateInterval",
            session.SetMinUpdateInterval(TimeSpan { Duration: 333_333 }),
        )?;
        api(
            "WGC dirty regions",
            session.SetDirtyRegionMode(GraphicsCaptureDirtyRegionMode::ReportOnly),
        )?;
        api("WGC cursor", session.SetIsCursorCaptureEnabled(false))?;
        api(
            "WGC secondary windows",
            session.SetIncludeSecondaryWindows(true),
        )?;
        // Leave the visible capture border on: no borderless permission request.
        api("WGC start", session.StartCapture())?;
        Ok(Self {
            pool,
            session,
            width: window.width,
            height: window.height,
        })
    }
    fn frame(&self) -> Result<Direct3D11CaptureFrame> {
        let start = Instant::now();
        loop {
            pump();
            match self.pool.TryGetNextFrame() {
                Ok(frame) => {
                    let size = api("frame size", frame.ContentSize())?;
                    if size.Width != self.width as i32 || size.Height != self.height as i32 {
                        let _ = frame.Close();
                        return Err(Error::Invalid);
                    }
                    return Ok(frame);
                }
                // An empty WinRT interface is represented by windows-rs as an
                // error with S_OK (observed on this OS) or E_POINTER. Both mean
                // the nonblocking frame pool has no frame yet, not a failed API.
                Err(e) if e.code() == E_POINTER || e.code() == S_OK => {}
                Err(e) => {
                    return Err(Error::Platform {
                        phase: "WGC next frame",
                        code: e.code().0 as u32,
                    });
                }
            }
            if start.elapsed() > Duration::from_secs(10) {
                return Err(Error::Timeout("WGC frame arrival"));
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
fn texture(frame: &Direct3D11CaptureFrame) -> Result<ID3D11Texture2D> {
    let access: IDirect3DDxgiInterfaceAccess = api(
        "frame surface",
        api("frame Surface", frame.Surface())?.cast(),
    )?;
    // SAFETY: The capture frame owns the live surface while the COM texture is acquired.
    unsafe { api("frame texture", access.GetInterface()) }
}
fn dirty(
    frame: &Direct3D11CaptureFrame,
    width: u32,
    height: u32,
) -> Result<Vec<(u32, u32, u32, u32)>> {
    let regions = api("frame dirty regions", frame.DirtyRegions())?;
    let count = api("dirty count", regions.Size())?;
    if count > 1024 {
        return Err(Error::Invalid);
    }
    let mut result = Vec::new();
    for i in 0..count {
        let r = api("dirty rectangle", regions.GetAt(i))?;
        if r.X < 0
            || r.Y < 0
            || r.Width <= 0
            || r.Height <= 0
            || r.X as u64 + r.Width as u64 > width as u64
            || r.Y as u64 + r.Height as u64 > height as u64
        {
            return Err(Error::Invalid);
        }
        result.push((r.X as u32, r.Y as u32, r.Width as u32, r.Height as u32));
    }
    Ok(result)
}

fn video(root: &Path, width: u32, height: u32, count: u32) -> Result<Value> {
    let window = SourceWindow::new(width, height)?;
    let gpu = Gpu::new()?;
    let capture = Capture::new(&gpu, &window)?;
    let mut encoder = Encoder::new(&gpu, width, height, 24_000_000)?;
    let mut file = File::create(root.join("stream.h265"))?;
    let mut samples = Vec::new();
    let mut arrivals = Vec::new();
    let mut dirty_counts = Vec::new();
    let mut latencies = Vec::new();
    let mut previous = None;
    let mut offset = 0u64;
    for i in 0..count + 10 {
        window.tick(i, 100);
        let frame = capture.frame()?;
        let stamp = api("frame timestamp", frame.SystemRelativeTime())?.Duration;
        let tex = texture(&frame)?;
        let regions = dirty(&frame, width, height)?;
        let encoded = encoder.encode(&tex, i as i64 * 333_333)?;
        api("frame close", frame.Close())?;
        if i >= 10 {
            if let Some(last) = previous {
                arrivals.push((stamp - last) as f64 / 10_000.0);
            }
            latencies.push(encoded.latency_ms);
            dirty_counts.push(regions.len() as f64);
        }
        previous = Some(stamp);
        use std::io::Write;
        file.write_all(&encoded.bytes)?;
        samples.push(json!({"offset":offset,"length":encoded.bytes.len(),"pts_us":encoded.pts/10,"warmup":i<10}));
        offset += encoded.bytes.len() as u64;
        if offset > 256 * 1024 * 1024 {
            return Err(Error::Invalid);
        }
        if i % 30 == 0 {
            eprintln!("HEVC {width}x{height}: frame {i}/{}", count + 10);
        }
    }
    let report = json!({"completed":true,"width":width,"height":height,"fps":30,"bitrate_bps":24_000_000,
        "encoder":encoder.description(),"samples":samples,"encode_submit_to_output_ms":stats(&latencies)?,
        "wgc_arrival_interval_ms":stats(&arrivals)?,"dirty_region_count":stats(&dirty_counts)?,
        "capture_min_update_interval_100ns":333333,"gpu_input":true,"cpu_pixel_copies_for_hevc":0,
        "async_inflight_limit":1,"stream_file":"stream.h265","warmup_frames":10});
    write_json(&root.join("index.json"), &report)?;
    Ok(report)
}

fn raster(root: &Path, width: u32, height: u32, percent: u32, count: u32) -> Result<Value> {
    let window = SourceWindow::new(width, height)?;
    let gpu = Gpu::new()?;
    let capture = Capture::new(&gpu, &window)?;
    let (mut times, mut bytes, mut fractions, mut copies, mut reported_fractions, mut diff_times) = (
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let mut transfer_frames = Vec::new();
    let mut transfer_base = None;
    let mut last_rgb: Option<Vec<u8>> = None;
    let mut unchanged_frames = 0u32;
    let mut captured_frames = 0u32;
    for i in 0..(count + 5) * 5 {
        captured_frames += 1;
        window.tick(i, percent);
        let frame = capture.frame()?;
        let reported = dirty(&frame, width, height)?;
        let reported_area: u64 = reported.iter().map(|r| r.2 as u64 * r.3 as u64).sum();
        let start = Instant::now();
        let rgb = gpu.read_rgb(&texture(&frame)?, width, height)?;
        let readback_ms = start.elapsed().as_secs_f64() * 1000.0;
        api("raster frame close", frame.Close())?;
        let diff_start = Instant::now();
        let rectangles = match &last_rgb {
            Some(previous) => changed_regions(previous, &rgb, width, height)?,
            None => vec![(0, 0, width, height)],
        };
        let diff_ms = diff_start.elapsed().as_secs_f64() * 1000.0;
        if rectangles.is_empty() {
            unchanged_frames += 1;
            last_rgb = Some(rgb);
            continue;
        }
        let start = Instant::now();
        let (mut encoded_bytes, mut area) = (0usize, 0u64);
        let mut transfer_frame = Vec::new();
        transfer_frame.extend_from_slice(&(rectangles.len() as u32).to_be_bytes());
        for (x, y, w, h) in &rectangles {
            let mut tile = Vec::with_capacity(byte_count(*w, *h, 3)?);
            for row in *y..y + h {
                let offset = (row as usize * width as usize + *x as usize) * 3;
                tile.extend_from_slice(&rgb[offset..offset + *w as usize * 3]);
            }
            let encoded = encode(&tile, *w, *h, ImageFormat::Jpeg)?;
            encoded_bytes += encoded.len();
            for value in [*x, *y, *w, *h, encoded.len() as u32] {
                transfer_frame.extend_from_slice(&value.to_be_bytes());
            }
            transfer_frame.extend_from_slice(&encoded);
            area += *w as u64 * *h as u64;
        }
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        if i >= 5 {
            times.push(elapsed);
            bytes.push(encoded_bytes as f64);
            fractions.push(area as f64 / (width as f64 * height as f64));
            reported_fractions.push(reported_area as f64 / (width as f64 * height as f64));
            diff_times.push(diff_ms);
            copies.push(readback_ms);
            if width == 2560 {
                if transfer_base.is_none() {
                    // Full initial state is transferred once outside measured
                    // dirty frames; static pixels must not remain uninitialized.
                    let encoded = encode(&rgb, width, height, ImageFormat::Jpeg)?;
                    let mut baseline = Vec::new();
                    for value in [1, 0, 0, width, height, encoded.len() as u32] {
                        baseline.extend_from_slice(&value.to_be_bytes());
                    }
                    baseline.extend_from_slice(&encoded);
                    transfer_base = Some(baseline);
                }
                transfer_frames.push(transfer_frame);
            }
        }
        last_rgb = Some(rgb);
        if times.len() == count as usize {
            break;
        }
        if i % 15 == 0 {
            eprintln!(
                "raster {width}x{height} {percent}% invalidation: frame {i}/{}",
                count + 5
            );
        }
    }
    if times.len() != count as usize {
        return Err(Error::Timeout("distinct raster source changes"));
    }
    let lossless = if width == 3840 {
        let rgb = required(last_rgb)?;
        // One task-owned inspection image outside Git; the HIL runner inventories
        // and removes it after inspection. It never contains another window.
        std::fs::write(
            root.join("generated-ui-preview.png"),
            encode(&rgb, width, height, ImageFormat::Png)?,
        )?;
        vec![
            measure_raster(&rgb, width, height, ImageFormat::Png, 10)?,
            measure_raster(&rgb, width, height, ImageFormat::Qoi, 10)?,
        ]
    } else {
        Vec::new()
    };
    if width == 2560 {
        use std::io::Write;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(format!("tiles-{percent}.vwt")))?;
        file.write_all(b"VWJT0002")?;
        for value in [width, height, transfer_frames.len() as u32] {
            file.write_all(&value.to_be_bytes())?;
        }
        let baseline = required(transfer_base)?;
        file.write_all(&(baseline.len() as u32).to_be_bytes())?;
        file.write_all(&baseline)?;
        for frame in transfer_frames {
            file.write_all(&(frame.len() as u32).to_be_bytes())?;
            file.write_all(&frame)?;
        }
    }
    Ok(
        json!({"completed":true,"width":width,"height":height,"requested_dirty_percent":percent,
        "jpeg_q85_tiles_encode_and_copy_ms":stats(&times)?,"encoded_bytes_per_frame":stats(&bytes)?,
        "reported_dirty_area_fraction_sum":stats(&reported_fractions)?,"encoded_dirty_area_fraction_sum":stats(&fractions)?,
        "dirty_strategy":"CPU pixel difference with tight row-span coalescing; WGC regions retained separately",
        "pixel_difference_ms":stats(&diff_times)?,"captured_frames":captured_frames,"unchanged_frames_skipped":unchanged_frames,
        "gpu_readback_ms":stats(&copies)?,
        "lossless":lossless,"network_fps_measured":false,"source":"generated UI-like window; no user content"}),
    )
}

pub fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !(2..=3).contains(&args.len()) {
        return Err(Error::Invalid);
    }
    let full = match args[1].to_str() {
        Some("smoke") => false,
        Some("full") => true,
        _ => return Err(Error::Invalid),
    };
    let phase = args.get(2).and_then(|v| v.to_str()).unwrap_or("all");
    if ![
        "all",
        "portrait",
        "4k",
        "raster-10",
        "raster-25",
        "raster-4k",
    ]
    .contains(&phase)
    {
        return Err(Error::Invalid);
    }
    let root = Path::new(&args[0]);
    if !root.is_absolute() || root.starts_with(std::env::current_dir()?) {
        return Err(Error::Unsupported(
            "media output must be a new owned temporary directory outside the repository",
        ));
    }
    std::fs::create_dir(root)?;
    // SAFETY: Initialization precedes window creation; runtime guard balances both APIs.
    unsafe {
        api(
            "PMv2",
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2),
        )?;
        api("RoInitialize", RoInitialize(RO_INIT_MULTITHREADED))?;
        if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
            RoUninitialize();
            return api("MFStartup", Err(e));
        }
    }
    let _runtime = Runtime;
    let mut results = Vec::new();
    let mut failed = false;
    for (name, width, height) in [("portrait", 1440, 3088), ("4k", 3840, 2160)] {
        if phase != "all" && phase != name {
            continue;
        }
        let directory = if phase == "all" {
            root.join(name)
        } else {
            root.to_path_buf()
        };
        if phase == "all" {
            std::fs::create_dir(&directory)?;
        }
        match video(&directory, width, height, if full { 120 } else { 12 }) {
            Ok(value) => results.push(json!({"phase":name,"result":value})),
            Err(error) => {
                failed = true;
                eprintln!("{name}: {error}");
                results.push(json!({"phase":name,"error":error.to_string()}));
            }
        }
    }
    for (width, height, percent) in [(2560, 1440, 10), (2560, 1440, 25), (3840, 2160, 25)] {
        let name = if width == 3840 {
            "raster-4k"
        } else if percent == 10 {
            "raster-10"
        } else {
            "raster-25"
        };
        if phase != "all" && phase != name {
            continue;
        }
        match raster(root, width, height, percent, if full { 30 } else { 5 }) {
            Ok(value) => results.push(json!({"phase":"raster","result":value})),
            Err(error) => {
                failed = true;
                eprintln!("raster: {error}");
                results.push(json!({"phase":"raster","width":width,"percent":percent,"error":error.to_string()}));
            }
        }
    }
    write_json(
        &root.join("report.json"),
        &json!({"schema":1,"completed":!failed,"profile":if full{"full"}else{"smoke"},"results":results}),
    )?;
    if failed {
        return Err(Error::Unsupported(
            "one or more measured phases failed; inspect numeric report",
        ));
    }
    Ok(())
}
