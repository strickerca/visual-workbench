use crate::{json, png};
use std::{ffi::c_void, io::Write, mem::size_of, path::Path, ptr};
use windows::{
    Win32::{
        Devices::Display::*,
        Foundation::{HWND, LPARAM, RECT},
        Graphics::{Dwm::*, Gdi::*},
        Media::MediaFoundation::*,
        Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow},
        System::Com::*,
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
    core::{BOOL, GUID, PCWSTR},
};

type Result<T> = std::result::Result<T, String>;

fn api_error(name: &str, error: windows::core::Error) -> String {
    format!("{name} failed (HRESULT 0x{:08x})", error.code().0 as u32)
}

pub fn initialize_dpi() -> Result<()> {
    // SAFETY: Called on startup before any DPI-dependent API or created window.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
        .map_err(|error| api_error("SetProcessDpiAwarenessContext", error))
}

fn utf16(value: &[u16]) -> String {
    let count = value
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..count])
}

fn privacy(value: &str) -> String {
    let mut safe = value.to_owned();
    for key in ["USERNAME", "COMPUTERNAME", "USERPROFILE"] {
        if let Ok(identifier) = std::env::var(key)
            && !identifier.is_empty()
        {
            safe = safe.replace(&identifier, "[redacted]");
        }
    }
    safe
}

struct Monitor {
    name: [u16; 32],
    rect: RECT,
    dpi_x: u32,
    dpi_y: u32,
    primary: bool,
}

struct MonitorContext {
    monitors: Vec<Monitor>,
    error: Option<String>,
}

unsafe extern "system" fn monitor_callback(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // SAFETY: EnumDisplayMonitors synchronously invokes this callback with our live
    // exclusive MonitorContext pointer. All output structures are initialized/sized.
    let context = unsafe { &mut *(data.0 as *mut MonitorContext) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    let mut dpi_x = 0;
    let mut dpi_y = 0;
    // SAFETY: Valid callback monitor handle and writable structures, no retained pointers.
    unsafe {
        if !GetMonitorInfoW(monitor, (&mut info as *mut MONITORINFOEXW).cast()).as_bool() {
            context.error = Some("GetMonitorInfoW failed".into());
            return BOOL(0);
        }
        if let Err(error) = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) {
            context.error = Some(api_error("GetDpiForMonitor", error));
            return BOOL(0);
        }
    }
    context.monitors.push(Monitor {
        name: info.szDevice,
        rect: info.monitorInfo.rcMonitor,
        dpi_x,
        dpi_y,
        primary: info.monitorInfo.dwFlags & 1 != 0,
    });
    BOOL(1)
}

fn display_paths() -> Result<Vec<DISPLAYCONFIG_PATH_INFO>> {
    for _ in 0..3 {
        let (mut paths, mut modes) = (0, 0);
        // SAFETY: Output counts are valid, later arrays are sized from those counts.
        let status =
            unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut paths, &mut modes) };
        if status.0 != 0 {
            return Err(format!("GetDisplayConfigBufferSizes failed ({})", status.0));
        }
        if paths > 256 || modes > 1024 {
            return Err("Display topology exceeds diagnostic bounds".into());
        }
        let mut path_array = vec![DISPLAYCONFIG_PATH_INFO::default(); paths as usize];
        let mut mode_array = vec![DISPLAYCONFIG_MODE_INFO::default(); modes as usize];
        // SAFETY: Both arrays hold the requested initialized capacity; Windows updates counts.
        let status = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut paths,
                path_array.as_mut_ptr(),
                &mut modes,
                mode_array.as_mut_ptr(),
                None,
            )
        };
        if status.0 == 122 {
            continue;
        }
        if status.0 != 0 {
            return Err(format!("QueryDisplayConfig failed ({})", status.0));
        }
        path_array.truncate(paths as usize);
        return Ok(path_array);
    }
    Err("Display topology kept changing during three bounded reads".into())
}

fn monitors() -> Result<String> {
    let mut context = MonitorContext {
        monitors: Vec::new(),
        error: None,
    };
    // SAFETY: The synchronous callback receives a valid exclusive stack context.
    let success = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(monitor_callback),
            LPARAM((&mut context as *mut MonitorContext) as isize),
        )
    };
    if let Some(error) = context.error {
        return Err(error);
    }
    if !success.as_bool() || context.monitors.is_empty() {
        return Err("No active monitor result".into());
    }
    let paths = display_paths()?;
    let mut rows = Vec::new();
    for (index, monitor) in context.monitors.iter().enumerate() {
        let mut mode = DEVMODEW {
            dmSize: size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        // SAFETY: Monitor's GDI name is a fixed NUL-terminated Windows output buffer.
        let mode_ok = unsafe {
            EnumDisplaySettingsW(
                PCWSTR(monitor.name.as_ptr()),
                ENUM_CURRENT_SETTINGS,
                &mut mode,
            )
        }
        .as_bool();
        let mut matched = None;
        for path in &paths {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                    adapterId: path.sourceInfo.adapterId,
                    id: path.sourceInfo.id,
                },
                ..Default::default()
            };
            // SAFETY: Device packet starts with the correctly initialized header and full sized storage.
            if unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } == 0
                && utf16(&source.viewGdiDeviceName) == utf16(&monitor.name)
            {
                matched = Some(path);
                break;
            }
        }
        let mut fields = vec![
            ("monitor_index", (index + 1).to_string()),
            ("primary", monitor.primary.to_string()),
            (
                "physical_width",
                (monitor.rect.right - monitor.rect.left).to_string(),
            ),
            (
                "physical_height",
                (monitor.rect.bottom - monitor.rect.top).to_string(),
            ),
            ("dpi_x", monitor.dpi_x.to_string()),
            ("dpi_y", monitor.dpi_y.to_string()),
            ("scale_x", (f64::from(monitor.dpi_x) / 96.0).to_string()),
            ("scale_y", (f64::from(monitor.dpi_y) / 96.0).to_string()),
        ];
        if let Some(path) = matched {
            let refresh = &path.targetInfo.refreshRate;
            fields.push((
                "refresh_hz",
                if refresh.Denominator > 0 {
                    (f64::from(refresh.Numerator) / f64::from(refresh.Denominator)).to_string()
                } else {
                    "null".into()
                },
            ));
            fields.push((
                "rotation_degrees",
                match path.targetInfo.rotation.0 {
                    1 => "0",
                    2 => "90",
                    3 => "180",
                    4 => "270",
                    _ => "null",
                }
                .into(),
            ));
            let mut color = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
                    size: size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                ..Default::default()
            };
            // SAFETY: Packet layout/size matches the requested legacy advanced-color query.
            let color_status = unsafe { DisplayConfigGetDeviceInfo(&mut color.header) };
            if color_status == 0 {
                // SAFETY: Successful query initializes the bitfield union representation.
                let flags = unsafe { color.Anonymous.value };
                fields.extend([("advanced_color_supported", (flags & 1 != 0).to_string()),
                    ("advanced_color_enabled", (flags & 2 != 0).to_string()),
                    ("wide_color_enforced", (flags & 4 != 0).to_string()),
                    ("bits_per_color_channel", color.bitsPerColorChannel.to_string()),
                    ("color_encoding", color.colorEncoding.0.to_string()),
                    ("hdr_state", json::string("advanced-color flags reported; HDR vs WCG not distinguished by this legacy query"))]);
            } else {
                fields.push((
                    "hdr_state",
                    json::string(&format!(
                        "untestable: advanced-color query failed ({color_status})"
                    )),
                ));
            }
        } else {
            fields.push((
                "refresh_hz",
                if mode_ok {
                    mode.dmDisplayFrequency.to_string()
                } else {
                    "null".into()
                },
            ));
            fields.push(("rotation_degrees", "null".into()));
            fields.push((
                "hdr_state",
                json::string("untestable: active DisplayConfig path did not match monitor"),
            ));
        }
        fields.push((
            "color_gamut",
            json::string("untestable: no calibrated colorimeter/ICC gamut measurement performed"),
        ));
        rows.push(json::object(&fields));
    }
    Ok(json::array(&rows))
}

struct MediaSession;
impl MediaSession {
    fn start() -> Result<Self> {
        // SAFETY: Fresh process/thread; matched CoUninitialize and MFShutdown on ownership drop.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }
            .map_err(|error| api_error("CoInitializeEx", error))?;
        // SAFETY: Supported MF version and full initialization flag; no media objects yet.
        if let Err(error) = unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) } {
            // SAFETY: Exactly balances successful CoInitializeEx on this thread.
            unsafe { CoUninitialize() };
            return Err(api_error("MFStartup", error));
        }
        Ok(Self)
    }
}
impl Drop for MediaSession {
    fn drop(&mut self) {
        // SAFETY: All enumeration objects are already dropped; balances this thread's session.
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}

struct Activations {
    pointer: *mut Option<IMFActivate>,
    count: usize,
}
impl Drop for Activations {
    fn drop(&mut self) {
        if !self.pointer.is_null() {
            // SAFETY: MFTEnumEx supplied count initialized Option<COM> entries and task-allocated array.
            unsafe {
                for index in 0..self.count {
                    ptr::drop_in_place(self.pointer.add(index));
                }
                CoTaskMemFree(Some(self.pointer.cast()));
            }
        }
    }
}

fn codec(category: GUID, subtype: GUID, role: &str, name: &str) -> Result<String> {
    let info = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: subtype,
    };
    let mut pointer = ptr::null_mut();
    let mut count = 0;
    // SAFETY: Valid initialized type filter and output pointers; returned array owned below.
    unsafe {
        MFTEnumEx(
            category,
            MFT_ENUM_FLAG_HARDWARE,
            if role == "decoder" { Some(&info) } else { None },
            if role == "encoder" { Some(&info) } else { None },
            &mut pointer,
            &mut count,
        )
    }
    .map_err(|error| api_error("MFTEnumEx", error))?;
    let activations = Activations {
        pointer,
        count: count as usize,
    };
    if count > 0 && activations.pointer.is_null() {
        return Err("MFT returned a positive count without an array".into());
    }
    let mut names = Vec::new();
    for index in 0..activations.count {
        // SAFETY: index is inside the initialized MFTEnumEx array, kept alive by owner.
        let activation = unsafe { &*activations.pointer.add(index) };
        if let Some(activation) = activation {
            // SAFETY: Live IMFActivate implements IMFAttributes; output buffers sized below.
            let length = unsafe { activation.GetStringLength(&MFT_FRIENDLY_NAME_Attribute) }
                .map_err(|error| api_error("MFT friendly-name length", error))?;
            if length > 4096 {
                return Err("MFT friendly name exceeds diagnostic bounds".into());
            }
            let mut buffer = vec![0u16; length as usize + 1];
            // SAFETY: Capacity covers reported length plus terminator; valid live interface/key.
            unsafe { activation.GetString(&MFT_FRIENDLY_NAME_Attribute, &mut buffer, None) }
                .map_err(|error| api_error("MFT friendly name", error))?;
            names.push(json::string(&privacy(&utf16(&buffer))));
        }
    }
    Ok(json::object(&[
        ("codec", json::string(name)),
        ("role", json::string(role)),
        ("hardware_flag", "true".into()),
        ("count", count.to_string()),
        ("names", json::array(&names)),
        ("processing_tested", "false".into()),
    ]))
}

pub fn diagnostics() -> Result<String> {
    let monitors = monitors()?;
    let _session = MediaSession::start()?;
    let mut codecs = Vec::new();
    for (name, subtype) in [("H.264", MFVideoFormat_H264), ("HEVC", MFVideoFormat_HEVC)] {
        codecs.push(codec(MFT_CATEGORY_VIDEO_ENCODER, subtype, "encoder", name)?);
        codecs.push(codec(MFT_CATEGORY_VIDEO_DECODER, subtype, "decoder", name)?);
    }
    Ok(json::object(&[
        ("schema_version", "1".into()),
        ("per_monitor_v2", "true".into()),
        ("monitors", monitors),
        ("media_foundation", json::array(&codecs)),
        ("capability_only", "true".into()),
        ("codec_latency_throughput_untested", "true".into()),
    ]))
}

struct WindowSearch {
    needle: String,
    handles: Vec<HWND>,
}
unsafe extern "system" fn window_callback(window: HWND, data: LPARAM) -> BOOL {
    // SAFETY: EnumWindows callback pointer comes from the live exclusive WindowSearch context.
    let context = unsafe { &mut *(data.0 as *mut WindowSearch) };
    let mut title = vec![0u16; 2048];
    // SAFETY: Valid enumerated HWND and fixed writable title buffer; raw titles never logged.
    unsafe {
        if IsWindowVisible(window).as_bool() {
            let length = GetWindowTextW(window, &mut title);
            if length > 0
                && utf16(&title[..length as usize])
                    .to_lowercase()
                    .contains(&context.needle)
            {
                context.handles.push(window);
            }
        }
    }
    BOOL(1)
}

struct CaptureSurface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut c_void,
}
impl Drop for CaptureSurface {
    fn drop(&mut self) {
        // SAFETY: Restore borrowed object before deleting our bitmap/DC; no remaining pixel borrows.
        unsafe {
            let _ = SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

pub fn screenshot(title: &str, output: &Path) -> Result<String> {
    if title.trim().is_empty() {
        return Err("Empty window selection is forbidden".into());
    }
    let parent = output
        .parent()
        .ok_or("Screenshot output needs an existing parent directory")?
        .canonicalize()
        .map_err(|_| "Screenshot output parent unavailable")?;
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|_| "Repository containment unavailable")?;
    if parent.starts_with(&repository) {
        return Err("Verification screenshots must stay outside the repository".into());
    }
    let mut search = WindowSearch {
        needle: title.to_lowercase(),
        handles: Vec::new(),
    };
    // SAFETY: Callback receives exclusive live search storage; enumeration completes before inspection.
    unsafe {
        EnumWindows(
            Some(window_callback),
            LPARAM((&mut search as *mut WindowSearch) as isize),
        )
    }
    .map_err(|error| api_error("EnumWindows", error))?;
    if search.handles.len() != 1 {
        return Err(format!(
            "Screenshot requires one visible matching window; found {}",
            search.handles.len()
        ));
    }
    let window = search.handles[0];
    // SAFETY: Live matching HWND; minimized windows cannot provide trustworthy current content.
    if unsafe { IsIconic(window) }.as_bool() {
        return Err("Selected window is minimized; capture not verified".into());
    }
    let mut bounds = RECT::default();
    // SAFETY: Writable physical RECT and live HWND, PMv2 already initialized.
    unsafe { GetWindowRect(window, &mut bounds) }
        .map_err(|error| api_error("GetWindowRect", error))?;
    let width = u32::try_from(bounds.right - bounds.left).map_err(|_| "Invalid window width")?;
    let height = u32::try_from(bounds.bottom - bounds.top).map_err(|_| "Invalid window height")?;
    if width == 0 || height == 0 || (width as usize) * (height as usize) > png::MAX_PIXELS {
        return Err("Window exceeds bounded screenshot dimensions".into());
    }
    // SAFETY: Independent memory DC, no screen image is read or copied.
    let dc = unsafe { CreateCompatibleDC(None) };
    if dc.is_invalid() {
        return Err("CreateCompatibleDC failed".into());
    }
    let header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = ptr::null_mut();
    // SAFETY: Sized top-down 32-bit DIB, initialized info, task-owned memory/DC.
    let bitmap =
        match unsafe { CreateDIBSection(Some(dc), &header, DIB_RGB_COLORS, &mut bits, None, 0) } {
            Ok(bitmap) => bitmap,
            Err(error) => {
                // SAFETY: DC was created above and no bitmap is selected.
                unsafe {
                    let _ = DeleteDC(dc);
                }
                return Err(api_error("CreateDIBSection", error));
            }
        };
    // SAFETY: Live owned bitmap/DC; previous object is borrowed and restored on drop.
    let previous = unsafe { SelectObject(dc, bitmap.into()) };
    let surface = CaptureSurface {
        dc,
        bitmap,
        previous,
        bits,
    };
    if surface.bits.is_null() || previous.is_invalid() {
        return Err("DIB selection failed".into());
    }
    // SAFETY: Live exclusively owned DIB storage, bounded dimensions; initialize
    // before capture so an incomplete renderer cannot expose unwritten bytes.
    unsafe {
        ptr::write_bytes(
            surface.bits.cast::<u8>(),
            0,
            width as usize * height as usize * 4,
        )
    };
    // SAFETY: Captures only the selected HWND into bounded private DIB storage. This
    // synchronous operation must be invoked through the project's bounded runner.
    if !unsafe { PrintWindow(window, surface.dc, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)) }
        .as_bool()
    {
        return Err("PrintWindow failed; no screen-copy fallback allowed".into());
    }
    // SAFETY: Complete this thread's GDI batch before directly reading DIB bits,
    // as required by CreateDIBSection's synchronization contract.
    if !unsafe { GdiFlush() }.as_bool() {
        return Err("GDI flush failed; capture completion unverified".into());
    }
    // SAFETY: DIB memory is valid for width*height*4 until surface drops; GDI write is complete.
    let bgra = unsafe {
        std::slice::from_raw_parts(
            surface.bits.cast::<u8>(),
            width as usize * height as usize * 4,
        )
    };
    let mut crop = bounds;
    // SAFETY: Same live HWND and correctly sized output RECT; unsuccessful DWM query falls back to whole window rectangle.
    if unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut crop as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        )
    }
    .is_err()
    {
        crop = bounds;
    }
    if crop.left < bounds.left
        || crop.top < bounds.top
        || crop.right > bounds.right
        || crop.bottom > bounds.bottom
        || crop.right <= crop.left
        || crop.bottom <= crop.top
    {
        return Err("DWM crop not contained in selected window".into());
    }
    let cropped_width = (crop.right - crop.left) as u32;
    let cropped_height = (crop.bottom - crop.top) as u32;
    let (offset_x, offset_y) = (
        (crop.left - bounds.left) as usize,
        (crop.top - bounds.top) as usize,
    );
    let mut rgb = Vec::with_capacity(cropped_width as usize * cropped_height as usize * 3);
    for y in 0..cropped_height as usize {
        for x in 0..cropped_width as usize {
            let pixel = ((offset_y + y) * width as usize + offset_x + x) * 4;
            rgb.extend_from_slice(&[bgra[pixel + 2], bgra[pixel + 1], bgra[pixel]]);
        }
    }
    if rgb.iter().all(|byte| *byte == 0) {
        return Err("Captured image is entirely black; renderer compatibility unverified".into());
    }
    // SAFETY: Live HWND; validate scale before creating the output file.
    let dpi = unsafe { GetDpiForWindow(window) };
    if dpi == 0 {
        return Err("Window DPI unavailable; screenshot verification incomplete".into());
    }
    let image = png::rgb(cropped_width, cropped_height, &rgb)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|_| "Screenshot output unavailable or already exists")?;
    file.write_all(&image)
        .map_err(|_| "Screenshot write failed")?;
    Ok(json::object(&[
        (
            "capture",
            json::string("selected-window PrintWindow; DWM physical bounds crop"),
        ),
        ("physical_width", cropped_width.to_string()),
        ("physical_height", cropped_height.to_string()),
        ("window_dpi", dpi.to_string()),
        ("scale", (f64::from(dpi) / 96.0).to_string()),
        ("per_monitor_v2", "true".into()),
        ("raw_window_title_retained", "false".into()),
    ]))
}
