use std::{
    collections::{HashMap, HashSet},
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::Path,
    time::Instant,
};
use vw_pen_probe::platform as probe;
use windows::{
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Input::Pointer::*, WindowsAndMessaging::*},
    },
    core::w,
};

struct Segment {
    from: POINT,
    to: POINT,
    width: i32,
    color: COLORREF,
}
struct State {
    file: BufWriter<File>,
    start: Instant,
    seen: HashSet<(u32, u32, u64, u32)>,
    strokes: Vec<Segment>,
    previous: Option<POINT>,
    samples: usize,
    downs: usize,
    failure: Option<String>,
    canvas: Option<Canvas>,
    controller: Option<u32>,
}

struct Canvas {
    dc: HDC,
    bitmap: HBITMAP,
    previous_bitmap: HGDIOBJ,
    width: i32,
    height: i32,
    rendered: usize,
    pens: HashMap<(i32, u32), HPEN>,
}
impl Canvas {
    fn new(reference: HDC, width: i32, height: i32) -> Result<Self, String> {
        if width <= 0 || height <= 0 || i64::from(width) * i64::from(height) > 32_000_000 {
            return Err("Canvas exceeds pixel bound".into());
        }
        // SAFETY: Compatible GDI resources are owned here and released by Drop.
        unsafe {
            let dc = CreateCompatibleDC(Some(reference));
            if dc.is_invalid() {
                return Err("Cannot create canvas DC".into());
            }
            let bitmap = CreateCompatibleBitmap(reference, width, height);
            if bitmap.is_invalid() {
                let _ = DeleteDC(dc);
                return Err("Cannot create canvas bitmap".into());
            }
            let previous_bitmap = SelectObject(dc, bitmap.into());
            let bounds = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            FillRect(dc, &bounds, HBRUSH(GetStockObject(WHITE_BRUSH).0));
            Ok(Self {
                dc,
                bitmap,
                previous_bitmap,
                width,
                height,
                rendered: 0,
                pens: HashMap::new(),
            })
        }
    }
    fn draw(&mut self, strokes: &[Segment], destination: HDC) -> Result<(), String> {
        // SAFETY: Draw only newly appended segments to the owned bitmap. Restore each
        // selected pen; no borrowed handle is deleted, and all points are bounded native input.
        unsafe {
            for line in &strokes[self.rendered..] {
                let key = (line.width, line.color.0);
                let pen = if let Some(pen) = self.pens.get(&key) {
                    *pen
                } else {
                    let pen = CreatePen(PS_SOLID, line.width, line.color);
                    if pen.is_invalid() {
                        return Err("Cannot create pressure pen".into());
                    }
                    self.pens.insert(key, pen);
                    pen
                };
                let old = SelectObject(self.dc, pen.into());
                let _ = MoveToEx(self.dc, line.from.x, line.from.y, None);
                let end_x = if line.from == line.to {
                    line.to.x.saturating_add(1)
                } else {
                    line.to.x
                };
                let ok = LineTo(self.dc, end_x, line.to.y).as_bool();
                SelectObject(self.dc, old);
                if !ok {
                    return Err("Cannot draw pressure segment".into());
                }
                self.rendered += 1;
            }
            BitBlt(
                destination,
                0,
                0,
                self.width,
                self.height,
                Some(self.dc),
                0,
                0,
                SRCCOPY,
            )
            .map_err(win_error)
        }
    }
}
impl Drop for Canvas {
    fn drop(&mut self) {
        // SAFETY: Restore the borrowed bitmap before deleting owned resources. Cached
        // pens were restored after every segment and are never selected at teardown.
        unsafe {
            SelectObject(self.dc, self.previous_bitmap);
            let _ = DeleteObject(self.bitmap.into());
            for pen in self.pens.values() {
                let _ = DeleteObject((*pen).into());
            }
            let _ = DeleteDC(self.dc);
        }
    }
}
fn win_error(e: windows::core::Error) -> String {
    format!("Windows error 0x{:08x}", e.code().0 as u32)
}
fn new_file(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "Output already exists or is not writable".into())
}

impl State {
    fn record(&mut self, h: HWND, message: u32, id: u32) -> Result<(), String> {
        let mut kind = POINTER_INPUT_TYPE::default();
        // SAFETY: Called synchronously while handling the pointer message with its pointer ID.
        unsafe { GetPointerType(id, &mut kind) }.map_err(win_error)?;
        if kind != PT_PEN {
            return Ok(());
        }
        let mut count = 0;
        // SAFETY: Length-only history query for the currently dispatched message.
        unsafe { GetPointerPenInfoHistory(id, &mut count, None) }.map_err(win_error)?;
        if !(1..=4096).contains(&count) {
            return Err("Pointer history exceeds recorder bound".into());
        }
        let mut history = vec![POINTER_PEN_INFO::default(); count as usize];
        // SAFETY: History has count writable initialized elements and stays alive for the call.
        unsafe { GetPointerPenInfoHistory(id, &mut count, Some(history.as_mut_ptr())) }
            .map_err(win_error)?;
        if count as usize > history.len() {
            return Err("Pointer history grew beyond buffer".into());
        }
        for pen in history[..count as usize].iter().rev() {
            let p = &pen.pointerInfo;
            if !self
                .seen
                .insert((p.pointerId, p.frameId, p.PerformanceCount, p.pointerFlags.0))
            {
                continue;
            }
            if self.samples >= 30000 {
                return Err("Recorder sample bound reached".into());
            }
            writeln!(
                self.file,
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{:.3}",
                self.samples,
                message,
                p.pointerId,
                p.frameId,
                p.dwTime,
                p.PerformanceCount,
                p.ptPixelLocation.x,
                p.ptPixelLocation.y,
                pen.pressure,
                pen.tiltX,
                pen.tiltY,
                pen.rotation,
                pen.penFlags,
                pen.penMask,
                p.pointerFlags.0,
                p.historyCount,
                self.start.elapsed().as_secs_f64() * 1000.0
            )
            .map_err(|_| "Cannot write native sample")?;
            self.samples += 1;
            if p.pointerFlags.0 & POINTER_FLAG_DOWN.0 != 0 {
                self.downs += 1;
            }
            let mut point = p.ptPixelLocation;
            // SAFETY: Physical screen point converted for this PMv2 window's drawing surface.
            if !unsafe { ScreenToClient(h, &mut point) }.as_bool() {
                return Err("ScreenToClient failed".into());
            }
            if p.pointerFlags.0 & POINTER_FLAG_INCONTACT.0 != 0 {
                if p.pointerFlags.0 & POINTER_FLAG_DOWN.0 != 0 {
                    self.previous = None;
                }
                let color = if pen.penFlags & PEN_FLAG_ERASER != 0 {
                    COLORREF(0xffffff)
                } else if pen.penFlags & PEN_FLAG_BARREL != 0 {
                    COLORREF(0xc06020)
                } else {
                    COLORREF(0x303030)
                };
                self.strokes.push(Segment {
                    from: self.previous.unwrap_or(point),
                    to: point,
                    width: 1 + (pen.pressure.min(1024) * 24 / 1024) as i32,
                    color,
                });
                self.previous = Some(point);
            } else {
                self.previous = None;
            }
        }
        // SAFETY: Invalidates only the owned harness; no synchronous callback is requested.
        let _ = unsafe { InvalidateRect(Some(h), None, false) };
        Ok(())
    }
    fn paint(&mut self, h: HWND) -> Result<(), String> {
        let mut paint = PAINTSTRUCT::default();
        // SAFETY: Begin/EndPaint are paired even on errors; the buffer owns its GDI resources.
        unsafe {
            let dc = BeginPaint(h, &mut paint);
            let mut client = RECT::default();
            let result = (|| -> Result<(), String> {
                GetClientRect(h, &mut client).map_err(win_error)?;
                if client.right == 0 || client.bottom == 0 {
                    return Ok(());
                }
                if self
                    .canvas
                    .as_ref()
                    .is_none_or(|c| c.width != client.right || c.height != client.bottom)
                {
                    self.canvas = Some(Canvas::new(dc, client.right, client.bottom)?);
                }
                self.canvas
                    .as_mut()
                    .ok_or("Canvas unavailable")?
                    .draw(&self.strokes, dc)
            })();
            let _ = EndPaint(h, &paint);
            result
        }
    }
}

unsafe extern "system" fn window_proc(h: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if message == WM_NCCREATE {
        // SAFETY: WM_NCCREATE supplies CREATESTRUCTW with our live State pointer.
        unsafe {
            let create = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(h, GWLP_USERDATA, create.lpCreateParams as isize);
        }
    }
    if message == WM_NCDESTROY {
        // SAFETY: Stop exposing the borrowed State before window teardown completes.
        unsafe {
            SetWindowLongPtrW(h, GWLP_USERDATA, 0);
        }
    }
    if matches!(
        message,
        WM_POINTERDOWN
            | WM_POINTERUPDATE
            | WM_POINTERUP
            | WM_POINTERENTER
            | WM_POINTERLEAVE
            | WM_PAINT
    ) {
        // SAFETY: State is boxed in run(), lives past DestroyWindow, and is accessed only on this UI thread.
        let data = unsafe { GetWindowLongPtrW(h, GWLP_USERDATA) as *mut State };
        if !data.is_null() {
            // SAFETY: No reentrant message pump is called while this exclusive reference is held.
            let state = unsafe { &mut *data };
            let result = if message == WM_PAINT {
                state.paint(h)
            } else {
                state.record(h, message, (wp.0 & 0xffff) as u32)
            };
            if let Err(e) = result {
                state.failure = Some(e);
                // SAFETY: Queues closure on our own window without synchronous reentry.
                let _ = unsafe { PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)) };
            }
            return LRESULT(0);
        }
    }
    if message == probe::NATIVE_DOWN_COUNT {
        // SAFETY: Read only the boxed State on its owning UI thread; no message dispatch.
        let data = unsafe { GetWindowLongPtrW(h, GWLP_USERDATA) as *const State };
        if !data.is_null() {
            // SAFETY: Same State lifetime as the recorder; count is bounded by 30,000.
            return LRESULT(unsafe { (*data).downs } as isize);
        }
        return LRESULT(0);
    }
    if message == probe::FOREGROUND_HANDOFF || message == WM_ACTIVATE {
        // SAFETY: Same boxed State lifetime as the pointer-message path, on the UI thread.
        let data = unsafe { GetWindowLongPtrW(h, GWLP_USERDATA) as *mut State };
        if !data.is_null() {
            // SAFETY: This path does not dispatch nested messages while State is borrowed.
            let state = unsafe { &mut *data };
            if message == probe::FOREGROUND_HANDOFF {
                if let Ok(pid) = u32::try_from(wp.0)
                    && probe::verified_injector(pid).is_ok()
                {
                    state.controller = Some(pid);
                } else {
                    return LRESULT(0);
                }
            }
            if let Some(pid) = state.controller
                && probe::verified_injector(pid).is_ok()
            {
                // SAFETY: Grant only the verified sibling injector, never ASFW_ANY.
                // Windows still requires the granting process to own foreground rights.
                let _ = unsafe { AllowSetForegroundWindow(pid) };
            }
            if message == probe::FOREGROUND_HANDOFF {
                return LRESULT(1);
            }
        }
    }
    if message == WM_TIMER || message == WM_KEYDOWN && wp.0 == 27 {
        // SAFETY: Close only the owned window after its deadline or Escape.
        let _ = unsafe { PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)) };
        return LRESULT(0);
    }
    if message == WM_DESTROY {
        // SAFETY: This binary owns one top-level window and this UI thread's message loop.
        unsafe {
            PostQuitMessage(0);
        }
        return LRESULT(0);
    }
    // SAFETY: Unhandled messages forwarded with their original parameters.
    unsafe { DefWindowProcW(h, message, wp, lp) }
}

pub fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 3 || args[0] != "--out" {
        return Err("Usage: pen-harness --out <new-directory> <duration-seconds:10..300>".into());
    }
    let seconds = args[2].parse::<u32>().map_err(|_| "Invalid duration")?;
    if !(10..=300).contains(&seconds) {
        return Err("Duration must be 10..300 seconds".into());
    }
    let output = Path::new(&args[1]);
    std::fs::create_dir(output)
        .map_err(|_| "Choose a new harness directory with an existing parent")?;
    let mut file = new_file(&output.join("received.csv"))?;
    writeln!(file, "sequence,message,pointer_id,frame_id,time_ms,performance_count,x,y,pressure,tilt_x,tilt_y,rotation,pen_flags,pen_mask,pointer_flags,history_count,received_ms").map_err(|_| "Cannot write CSV header")?;
    probe::initialize_dpi()?;
    let mut state = Box::new(State {
        file: BufWriter::with_capacity(256 * 1024, file),
        start: Instant::now(),
        seen: HashSet::new(),
        strokes: Vec::new(),
        previous: None,
        samples: 0,
        downs: 0,
        failure: None,
        canvas: None,
        controller: None,
    });
    // SAFETY: Class, title and state storage stay alive through the message loop; one UI thread owns the window.
    let h = unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None).map_err(win_error)?.0);
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: w!("VisualWorkbenchPenHarnessT005"),
            hCursor: LoadCursorW(None, IDC_CROSS).map_err(win_error)?,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err("Cannot register harness window".into());
        }
        let h = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class.lpszClassName,
            w!("Visual Workbench pen harness - Escape closes - physical input evidence"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1000,
            800,
            None,
            None,
            Some(instance),
            Some((&mut *state as *mut State).cast()),
        )
        .map_err(win_error)?;
        let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
        h
    };
    let result = (|| -> Result<(), String> {
        let target = probe::snapshot(h.0 as usize, std::process::id())?;
        writeln!(new_file(&output.join("target.json"))?, "{{\"schema\":1,\"hwnd\":{},\"pid\":{},\"dpi\":{},\"client\":[{},{},{},{}],\"source\":\"native_wm_pointer\"}}",
            target.hwnd, target.pid, target.dpi, target.client.left, target.client.top, target.client.right, target.client.bottom).map_err(|_| "Cannot write target receipt")?;
        // SAFETY: Timer belongs to the owned window and WM_TIMER closes it after the bounded run.
        if unsafe { SetTimer(Some(h), 1, seconds * 1000, None) } == 0 {
            return Err("Cannot set harness deadline".into());
        }
        println!(
            "Harness ready; target receipt written. No foreground activation requested. Deadline: {seconds}s."
        );
        let mut message = MSG::default();
        loop {
            // SAFETY: Initialized message buffer and the current UI thread's queue.
            let status = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
            if status == -1 {
                return Err("GetMessageW failed".into());
            }
            if status == 0 {
                break;
            }
            // SAFETY: Dispatch the message returned by GetMessageW; State remains live.
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        Ok(())
    })();
    // SAFETY: Destroy the owned window (if still alive) before dropping its borrowed State.
    unsafe {
        if IsWindow(Some(h)).as_bool() {
            let _ = DestroyWindow(h);
        }
    }
    state
        .file
        .flush()
        .map_err(|_| "Cannot flush receiver journal")?;
    let healthy = result.is_ok() && state.failure.is_none();
    writeln!(new_file(&output.join("receiver.json"))?, "{{\"schema\":1,\"native_samples\":{},\"recorder_healthy\":{},\"measurement_complete\":false}}", state.samples, healthy).map_err(|_| "Cannot write receiver receipt")?;
    result?;
    if let Some(e) = state.failure {
        return Err(e);
    }
    println!(
        "Harness closed: {} native samples. Completion requires comparison with the command journal.",
        state.samples
    );
    Ok(())
}
