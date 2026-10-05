//! Owner-ready, task-owned sink evidence for the production Process/input helper.
//! No old injector is called. Ordinary tests never launch this harness.
use crate::process::windows::Pins;
use crate::{
    Error, Result, platform,
    process::{self, Cancellation, Kind, LockedHelper, Process},
};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use vw_remote::{
    Binding, Scope, Target,
    wire::{Command, FiniteAction, Header, PenSample, Phase},
};
use windows::{
    Win32::{
        Foundation::*,
        System::{LibraryLoader::GetModuleHandleW, Threading::*},
        UI::{
            Input::{
                GetCurrentInputMessageSource, INPUT_MESSAGE_SOURCE, KeyboardAndMouse::*, Pointer::*,
            },
            WindowsAndMessaging::*,
        },
    },
    core::{PWSTR, w},
};
#[path = "hil/controller_surface.rs"]
mod controller_surface;
const CLASS: &str = "VisualWorkbenchRemoteGuardHarnessM4";
const HANDOFF: u32 = WM_APP + 101;
const COUNTS: u32 = WM_APP + 102;
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        /* SAFETY: one owned process query handle. */
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
fn image(pid: u32) -> Result<PathBuf> {
    let mut text = [0u16; 32768];
    let mut size = text.len() as u32;
    // SAFETY: read-only process lookup and bounded initialized UTF-16 output.
    unsafe {
        let p = Handle(platform::api(
            "HIL process",
            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid),
        )?);
        platform::api(
            "HIL process image",
            QueryFullProcessImageNameW(
                p.0,
                PROCESS_NAME_WIN32,
                PWSTR(text.as_mut_ptr()),
                &mut size,
            ),
        )?;
    }
    if size == 0 || size as usize >= text.len() {
        return Err(Error::Invalid);
    }
    Ok(PathBuf::from(
        String::from_utf16(&text[..size as usize]).map_err(|_| Error::Invalid)?,
    ))
}
fn digest(pins: &Pins) -> Result<String> {
    let mut file = pins.executable.file().try_clone()?;
    if file.metadata()?.len() > 128 * 1024 * 1024 {
        return Err(Error::Limit);
    }
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = [0; 65536];
    let mut count = 0usize;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        count += n;
        if count > 128 * 1024 * 1024 {
            return Err(Error::Limit);
        }
        context.update(&buffer[..n]);
    }
    pins.verify()?;
    Ok(context
        .finish()
        .as_ref()
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect())
}
struct Image {
    path: PathBuf,
    pins: Pins,
}
impl Image {
    fn own(expected: Option<&str>) -> Result<Self> {
        let path = std::env::current_exe()?;
        let pins = Pins::open(&path)?;
        if let Some(expected) = expected
            && (!vw_remote::profile::digest(expected) || digest(&pins)? != expected)
        {
            return Err(Error::Invalid);
        }
        Ok(Self { path, pins })
    }
    fn matches(&self, pid: u32) -> Result<()> {
        self.pins.verify()?;
        if image(pid)? != self.path {
            return Err(Error::TargetChanged);
        }
        self.pins.verify()
    }
}
struct SurfaceState {
    record: File,
    image: Image,
    counts: [u32; 6],
    diagnostic_records: u32,
    failed: bool,
    controller: Option<u32>,
    controller_fixture: Option<controller_surface::ControllerSurface>,
    release_marker: Option<(PathBuf, String)>,
    published: bool,
}
impl SurfaceState {
    fn report_counts(&mut self) {
        if self.controller_fixture.is_none() || self.diagnostic_records >= 32 {
            return;
        }
        // File::metadata queries the opened writer handle. Directory-entry
        // length can lag while this journal remains open; it is not evidence.
        let bytes = self
            .record
            .metadata()
            .map(|v| v.len().to_string())
            .unwrap_or_else(|_| "unknown".into());
        let [down, update, up, mouse_down, mouse_up, wheel] = self.counts;
        self.diagnostic_records += 1;
        // Bounded observation only. Output failure cannot release this HWND.
        let _ = writeln!(
            std::io::stdout(),
            "REMOTE_RECEIVER_COUNTS:down={down};update={update};up={up};mouse_down={mouse_down};mouse_up={mouse_up};wheel={wheel};journal_bytes={bytes}"
        );
    }

    fn release_confirmed(&self) -> bool {
        let Some((path, expected)) = &self.release_marker else {
            return true;
        };
        std::fs::symlink_metadata(path)
            .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink() && m.len() <= 64)
            && std::fs::read_to_string(path).is_ok_and(|value| value == *expected)
    }
}
fn retain_controller_destination(controller: bool, published: bool, settled: bool) -> bool {
    controller && published && !settled
}
fn consume_controller_pointer_lifecycle(controller: bool, message: u32) -> bool {
    controller
        && matches!(
            message,
            WM_POINTERENTER | WM_POINTERLEAVE | WM_POINTERCAPTURECHANGED
        )
}
fn input_provenance(source: Option<(i32, i32)>, extra_info: u64) -> serde_json::Value {
    // These are raw current-message observations, never proof that mouse input
    // was promoted from pen or that an injected pointer was a physical device.
    serde_json::json!({
        "schema": 1,
        "source_available": source.is_some(),
        "device_type": source.map(|v| v.0),
        "origin_id": source.map(|v| v.1),
        "message_extra_info": format!("{extra_info:016x}")
    })
}
fn current_input_provenance() -> serde_json::Value {
    let mut source = INPUT_MESSAGE_SOURCE::default();
    // SAFETY: initialized bounded output, read-only queries on the actual
    // receiver UI thread during dispatch; neither query pumps or injects input.
    let (available, extra_info) = unsafe {
        (
            GetCurrentInputMessageSource(&mut source).is_ok(),
            GetMessageExtraInfo().0 as usize as u64,
        )
    };
    input_provenance(
        available.then_some((source.deviceType.0, source.originId.0)),
        extra_info,
    )
}
unsafe extern "system" fn surface_proc(
    hwnd: HWND,
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        // SAFETY: CREATESTRUCTW contains our live boxed UI-thread state.
        unsafe {
            let c = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, c.lpCreateParams as isize);
        }
    }
    // SAFETY: pointer is supplied by this binary and cleared on NCDESTROY; one UI thread owns it.
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut SurfaceState;
    if message == WM_NCDESTROY {
        /* SAFETY: end state exposure before dropping the Box. */
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        }
    }
    if !pointer.is_null() && message == WM_PAINT {
        // SAFETY: one owned UI thread, state lives through NCDESTROY. The
        // immutable read ends before BeginPaint can re-enter this procedure.
        let generation = unsafe { &*pointer }
            .controller_fixture
            .as_ref()
            .map(controller_surface::ControllerSurface::generation);
        if let Some(generation) = generation {
            if controller_surface::ControllerSurface::paint(hwnd, generation).is_err() {
                // SAFETY: no state reference crossed the native paint call.
                unsafe { (*pointer).failed = true };
                // Retain this exact input destination while retirement may be
                // Pending; a paint failure is not permission to destroy it.
                // SAFETY: stop only this fixture's paint work, not its HWND.
                let _ = unsafe { KillTimer(Some(hwnd), controller_surface::TIMER_ID) };
            }
            return LRESULT(0);
        }
    }
    if !pointer.is_null() {
        // SAFETY: no nested pump while this reference is borrowed.
        let state = unsafe { &mut *pointer };
        // A separate opt-in integration surface keeps actual WGC output live;
        // the default guard/input-free surfaces retain their original behavior.
        if let Some(fixture) = &mut state.controller_fixture {
            let result = if message == WM_TIMER && wp.0 == controller_surface::TIMER_ID {
                fixture.tick(hwnd).map(|()| true)
            } else {
                Ok(false)
            };
            match result {
                Ok(true) => return LRESULT(0),
                Ok(false) => {}
                Err(_) => {
                    state.failed = true;
                    // SAFETY: bounded work stops, exact release destination lives.
                    let _ = unsafe { KillTimer(Some(hwnd), controller_surface::TIMER_ID) };
                    return LRESULT(0);
                }
            }
        }
        if message == COUNTS {
            return LRESULT(state.counts.get(wp.0).copied().unwrap_or(u32::MAX) as isize);
        }
        if message == HANDOFF || message == WM_ACTIVATE {
            if message == HANDOFF {
                let Ok(pid) = u32::try_from(wp.0) else {
                    return LRESULT(0);
                };
                if state.image.matches(pid).is_err() {
                    return LRESULT(0);
                }
                state.controller = Some(pid);
            }
            if let Some(pid) = state.controller
                && state.image.matches(pid).is_ok()
            {
                // SAFETY: explicit owner-ready handoff only to the same retained exact task image.
                let accepted = unsafe { AllowSetForegroundWindow(pid) }.is_ok();
                if message == HANDOFF {
                    return LRESULT(isize::from(accepted));
                }
            }
        }
        if consume_controller_pointer_lifecycle(state.controller_fixture.is_some(), message) {
            // This fixture consumes DOWN/UPDATE/UP below. Microsoft documents
            // selective pointer consumption with DefWindowProc as undefined;
            // consume its ENTER/LEAVE/CAPTURECHANGED notifications consistently.
            // No contact/retirement inference: capture loss need not pair with UP.
            return LRESULT(0);
        }
        let counter = match message {
            WM_POINTERDOWN => Some(0),
            WM_POINTERUPDATE => Some(1),
            WM_POINTERUP => Some(2),
            WM_LBUTTONDOWN | WM_RBUTTONDOWN => Some(3),
            WM_LBUTTONUP | WM_RBUTTONUP => Some(4),
            WM_MOUSEWHEEL => Some(5),
            _ => None,
        };
        if let Some(index) = counter {
            if state.counts.iter().copied().sum::<u32>() >= 20000 {
                state.failed = true;
            } else {
                state.counts[index] += 1;
                // Capture provenance before other per-message native queries.
                // Legacy default surfaces preserve their existing journal schema.
                let provenance = state
                    .controller_fixture
                    .as_ref()
                    .map(|_| current_input_provenance());
                let mut pen = POINTER_PEN_INFO::default();
                // SAFETY: bounded pointer ID from the actual window message, initialized output.
                let pen_ok = index < 3
                    && unsafe { GetPointerPenInfo((wp.0 & 0xffff) as u32, &mut pen) }.is_ok();
                let mut record = serde_json::json!({"message":message,"index":index,"count":state.counts[index],"pointer_available":pen_ok,
                    "pressure":if pen_ok{Some(pen.pressure)}else{None},"pen_flags":if pen_ok{Some(pen.penFlags)}else{None},
                    "performance_count":if pen_ok{Some(pen.pointerInfo.PerformanceCount)}else{None},
                    "pointer_flags":if pen_ok{Some(pen.pointerInfo.pointerFlags.0)}else{None},
                    "pointer_id":if pen_ok{Some(pen.pointerInfo.pointerId)}else{None}});
                if let Some(provenance) = provenance {
                    record["input_provenance"] = provenance;
                }
                if state.controller_fixture.is_some() {
                    match controller_surface::received_clock() {
                        Ok((counter, frequency)) => {
                            record["received_qpc"] = serde_json::json!({"schema":1,"counter":counter,"frequency":frequency})
                        }
                        Err(_) => state.failed = true,
                    }
                }
                if serde_json::to_writer(&mut state.record, &record).is_err()
                    || writeln!(&mut state.record).is_err()
                {
                    state.failed = true;
                }
                state.report_counts();
            }
            if state.failed {
                if state.controller_fixture.is_some() {
                    // SAFETY: stop failed animation but retain the release HWND.
                    let _ = unsafe { KillTimer(Some(hwnd), controller_surface::TIMER_ID) };
                } else {
                    /* SAFETY: original default-surface failure close. */
                    let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
                }
            }
            return LRESULT(0);
        }
    }
    if !pointer.is_null() && message == WM_TIMER {
        // SAFETY: read ends before timer calls. The opt-in receiver's bounded
        // work deadline never authorizes destruction during Pending retirement.
        let retain_destination = unsafe { &*pointer }.controller_fixture.is_some();
        if retain_destination {
            // SAFETY: both timers belong only to this exact retained HWND.
            unsafe {
                let _ = KillTimer(Some(hwnd), 1);
                let _ = KillTimer(Some(hwnd), controller_surface::TIMER_ID);
            }
            return LRESULT(0);
        }
    }
    if !pointer.is_null() && (message == WM_CLOSE || message == WM_KEYDOWN && wp.0 == 27) {
        // SAFETY: one UI thread owns the state. A work deadline or Escape is
        // only a stop request; only the correlated settlement marker permits close.
        let state = unsafe { &*pointer };
        if retain_controller_destination(
            state.controller_fixture.is_some(),
            state.published,
            state.release_confirmed(),
        ) {
            return LRESULT(0);
        }
    }
    if message == WM_TIMER || message == WM_KEYDOWN && wp.0 == 27 {
        /* SAFETY: owned deadline/Escape close. */
        let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
        return LRESULT(0);
    }
    if message == WM_DESTROY {
        /* SAFETY: this process owns one UI window. */
        unsafe {
            PostQuitMessage(0);
        }
        return LRESULT(0);
    }
    // SAFETY: forward unchanged native message parameters.
    unsafe { DefWindowProcW(hwnd, message, wp, lp) }
}
fn new_file(path: &Path) -> Result<File> {
    Ok(OpenOptions::new().write(true).create_new(true).open(path)?)
}
fn publish_target(output: &Path, target: &Target) -> Result<()> {
    let pending = output.join("target.json.pending");
    {
        let mut file = new_file(&pending)?;
        serde_json::to_writer(&mut file, target).map_err(|_| Error::Io)?;
        file.sync_all()?;
    }
    // Atomic no-replace publication on the same owned Windows volume. A reader
    // observing target.json can never race an open/partial JSON writer.
    std::fs::hard_link(&pending, output.join("target.json"))?;
    // Final publication is authoritative. Do not destroy the now-published
    // release destination merely because private staged-link cleanup failed.
    let _ = std::fs::remove_file(pending);
    Ok(())
}
fn surface(output: &Path, position: i32, controller_fixture: bool) -> Result<()> {
    platform::initialize_hil_dpi()?;
    std::fs::create_dir(output)?;
    let mut state = Box::new(SurfaceState {
        record: new_file(&output.join("received.jsonl"))?,
        image: Image::own(None)?,
        counts: [0; 6],
        diagnostic_records: 0,
        failed: false,
        controller: None,
        controller_fixture: controller_fixture.then(controller_surface::ControllerSurface::new),
        release_marker: if controller_fixture {
            let parent = output.parent().ok_or(Error::Invalid)?;
            let run = parent
                .file_name()
                .and_then(|v| v.to_str())
                .and_then(|v| v.strip_prefix("VisualWorkbench-remote-integration-"))
                .ok_or(Error::Invalid)?;
            if run.len() != 32
                || !run
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                return Err(Error::Invalid);
            }
            Some((output.join("input-settled"), run.to_owned()))
        } else {
            None
        },
        published: false,
    });
    // SAFETY: one owned window/class, boxed state outlives DestroyWindow and native message loop.
    let hwnd = unsafe {
        let instance = HINSTANCE(platform::api("HIL module", GetModuleHandleW(None))?.0);
        let class = WNDCLASSW {
            hInstance: instance,
            lpfnWndProc: Some(surface_proc),
            lpszClassName: w!("VisualWorkbenchRemoteGuardHarnessM4"),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(Error::Unavailable);
        }
        let hwnd = platform::api(
            "HIL window",
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class.lpszClassName,
                if position == 80 {
                    w!(
                        "TARGET: click window to start; no buttons; deliberate movements — Escape closes"
                    )
                } else {
                    w!("SINK: receiver; deliberate focus switches; no buttons — Escape closes")
                },
                WS_OVERLAPPEDWINDOW,
                position,
                120,
                640,
                560,
                None,
                None,
                Some(instance),
                Some((&mut *state as *mut SurfaceState).cast()),
            ),
        )?;
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        hwnd
    };
    let result = (|| {
        // Read-only fixture receipt. The injection driver supplies its actual PID
        // later; this surface never injects or creates a control grant itself.
        let target = platform::query_target(hwnd.0 as usize as u64, u32::MAX, id(1)?)?;
        if controller_fixture {
            // Ordinary exact owned-window foreground acquisition, followed by
            // full HWND/PID/thread/creation/geometry and actual foreground proof.
            // No global key, thread attachment or foreground-lock bypass.
            platform::focus_for_owner_grant(&target, u32::MAX)?;
        }
        // SAFETY: deadline timer belongs to our own window.
        if unsafe { SetTimer(Some(hwnd), 1, 300000, None) } == 0 {
            return Err(Error::Unavailable);
        }
        if controller_fixture {
            // SAFETY: bounded paint-only timer on this exact owned fixture HWND.
            if unsafe { SetTimer(Some(hwnd), controller_surface::TIMER_ID, 100, None) } == 0 {
                return Err(Error::Unavailable);
            }
        }
        publish_target(output, &target)?;
        // No grant-capable identity is published until all readiness work passed.
        state.published = true;
        println!(
            "surface ready; bounded300s; actual receiver messages; no physical provenance inferred"
        );
        let mut msg = MSG::default();
        loop {
            /* SAFETY: initialized buffer for the owning UI thread. */
            let status = unsafe { GetMessageW(&mut msg, None, 0, 0) }.0;
            if status < 0 {
                return Err(Error::Unavailable);
            }
            if status == 0 {
                break;
            }
            // SAFETY: original parameters from GetMessageW; Box remains alive.
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if state.failed {
            return Err(Error::Io);
        }
        state.record.sync_all()?;
        serde_json::to_writer(new_file(&output.join("counts.json"))?, &state.counts)
            .map_err(|_| Error::Io)?;
        Ok(())
    })();
    if retain_controller_destination(
        controller_fixture,
        state.published,
        state.release_confirmed(),
    ) {
        // GetMessage/paint/IO failure cannot drop a published release destination.
        // Keep servicing this exact UI thread until actual settlement is correlated.
        let mut report = Instant::now();
        while !state.release_confirmed() {
            let mut message = MSG::default();
            // SAFETY: initialized message and owned UI thread; no state borrow crosses dispatch.
            unsafe {
                if PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool()
                    && message.message != WM_QUIT
                {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            if report.elapsed() >= Duration::from_secs(2) {
                println!("REMOTE_SURFACE_PHASE:waiting_actual_input_retirement");
                report = Instant::now();
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
    // SAFETY: destroy only the task window while its state still lives.
    if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        let _ = unsafe { DestroyWindow(hwnd) };
    }
    result
}
fn id(counter: u64) -> Result<String> {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Invalid)?
        .as_nanos();
    let value = format!(
        "{:08x}-{:04x}-7{:03x}-8{:03x}-{:012x}",
        (ns as u64 >> 32) as u32,
        (ns as u64 >> 16) as u16,
        ns as u64 & 0xfff,
        std::process::id() & 0xfff,
        counter
    );
    vw_remote::id(&value)?;
    Ok(value)
}
struct OwnedSurface {
    target: Target,
    image: Arc<Image>,
}
impl OwnedSurface {
    fn open(window: u64, pid: u32, image: Arc<Image>) -> Result<Self> {
        if pid == 0 || pid == std::process::id() {
            return Err(Error::Invalid);
        }
        image.matches(pid)?;
        let target =
            platform::query_target(window, std::process::id(), id(window & 0xffffffffffff)?)?;
        if target.process_id != pid {
            return Err(Error::TargetChanged);
        }
        let v = Self { target, image };
        v.verify()?;
        Ok(v)
    }
    fn verify(&self) -> Result<()> {
        self.image.matches(self.target.process_id)?;
        let mut pid = 0;
        let mut class = [0u16; 128];
        let h = HWND(self.target.window as usize as *mut std::ffi::c_void);
        // SAFETY: read-only class/process checks on the exact task-owned window; works during restore.
        unsafe {
            let thread = GetWindowThreadProcessId(h, Some(&mut pid));
            let n = GetClassNameW(h, &mut class);
            if thread != self.target.thread_id
                || pid != self.target.process_id
                || n <= 0
                || String::from_utf16_lossy(&class[..n as usize]) != CLASS
            {
                return Err(Error::TargetChanged);
            }
            let p = Handle(platform::api(
                "HIL target process",
                OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid),
            )?);
            let (mut c, mut e, mut k, mut u) = (
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
            );
            platform::api(
                "HIL target creation",
                GetProcessTimes(p.0, &mut c, &mut e, &mut k, &mut u),
            )?;
            if ((u64::from(c.dwHighDateTime) << 32) | u64::from(c.dwLowDateTime))
                != self.target.process_created
            {
                return Err(Error::TargetChanged);
            }
        }
        self.image.pins.verify()
    }
    fn message(&self, message: u32, index: usize) -> Result<usize> {
        self.verify()?;
        let mut value = 0;
        // SAFETY: pointer-free bounded message only to the exact retained task surface.
        let result = unsafe {
            SendMessageTimeoutW(
                HWND(self.target.window as usize as *mut std::ffi::c_void),
                message,
                WPARAM(index),
                LPARAM(0),
                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                100,
                Some(&mut value),
            )
        };
        if result.0 == 0 {
            return Err(Error::Timeout);
        }
        Ok(value)
    }
    fn count(&self, index: usize) -> Result<usize> {
        self.message(COUNTS, index)
    }
    fn snapshot(&self) -> Result<[usize; 6]> {
        Ok([
            self.count(0)?,
            self.count(1)?,
            self.count(2)?,
            self.count(3)?,
            self.count(4)?,
            self.count(5)?,
        ])
    }
    fn drained_snapshot(&self) -> Result<[usize; 6]> {
        // Actual producer retirement precedes callers' final observations. Native
        // posted input can lag; require two stable six-category observations.
        let deadline = Instant::now() + Duration::from_millis(250);
        thread::sleep(Duration::from_millis(25));
        let mut previous = self.snapshot()?;
        loop {
            thread::sleep(Duration::from_millis(25));
            let current = self.snapshot()?;
            if current == previous {
                return Ok(current);
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            previous = current;
        }
    }
    fn wait_count(&self, index: usize, expected: usize) -> Result<()> {
        let started = Instant::now();
        loop {
            if self.count(index)? == expected {
                return Ok(());
            }
            if started.elapsed() >= Duration::from_millis(250) {
                return Err(Error::Timeout);
            }
            thread::sleep(Duration::from_millis(2));
        }
    }
    fn restore(&self) -> Result<()> {
        self.verify()?;
        let r = self.target.window_rect;
        // SAFETY: restore only the verified task HWND's original physical rectangle.
        unsafe {
            let h = HWND(self.target.window as usize as *mut std::ffi::c_void);
            let _ = ShowWindow(h, SW_RESTORE);
            platform::api(
                "HIL restore",
                SetWindowPos(
                    h,
                    None,
                    r.x,
                    r.y,
                    r.width as i32,
                    r.height as i32,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                ),
            )?;
        }
        Ok(())
    }
    fn fresh(&self) -> Result<Target> {
        self.verify()?;
        platform::query_target(
            self.target.window,
            std::process::id(),
            self.target.token.clone(),
        )
    }
    fn wait_initial_owner_click(
        &self,
        peer: &Self,
        software: Option<Arc<LockedHelper>>,
    ) -> Result<()> {
        // Observe a NEW balanced receiver pair after this readiness baseline.
        // Automatic foreground activation is insufficient. This is receiver
        // evidence, not independent physical/hardware mouse provenance.
        // Interactive default sends no input. Explicit unattended administration
        // uses one production-helper guarded software pair, never fabricated
        // window messages or physical/hardware provenance.
        let started = Instant::now();
        let deadline = started + Duration::from_secs(45);
        let mut next_report = started;
        let counts = self.snapshot()?;
        let mut baseline = (counts[3], counts[4]);
        let mut settling_initial = !mouse_released() || baseline.0 != baseline.1;
        let mut software_sent = false;
        loop {
            self.verify()?;
            peer.verify()?;
            let counts = self.snapshot()?;
            let observed = (counts[3], counts[4]);
            let released = mouse_released();
            if settling_initial && released && observed.0 == observed.1 {
                // A held pre-baseline mouse is allowed to release, but that
                // release cannot start input. Require a subsequent NEW pair.
                baseline = observed;
                settling_initial = false;
            }
            if !settling_initial
                && !software_sent
                && let Some(helper) = &software
            {
                // Baseline precedes this one actual finite pair. Production
                // focus/arm/SendInput independently guard the exact target and
                // point; unrelated foreground and any partial batch refuse.
                let before = peer.drained_snapshot()?;
                let current = self.focus(Some(peer))?;
                if !mouse_released() {
                    return Err(Error::Ungranted);
                }
                let (mut owner, binding) = open(helper.clone(), &current, 202)?;
                let sequence = owner.next_sequence()?;
                let x = current
                    .client_rect
                    .x
                    .checked_add(
                        i32::try_from(current.client_rect.width / 2).map_err(|_| Error::Invalid)?,
                    )
                    .ok_or(Error::Invalid)?;
                let y = current
                    .client_rect
                    .y
                    .checked_add(
                        i32::try_from(current.client_rect.height / 2)
                            .map_err(|_| Error::Invalid)?,
                    )
                    .ok_or(Error::Invalid)?;
                // Helper startup/preparation may block. Recheck the SAME
                // readiness deadline before sending any finite input, and
                // observe this actual owner close even when it expired.
                if Instant::now() >= deadline {
                    let _retirement = owner.close();
                    // Pending stays retained by Process Drop; no successful
                    // readiness/retirement receipt is published on timeout.
                    return Err(Error::Timeout);
                }
                let accepted = injected(
                    &mut owner,
                    &binding,
                    1,
                    Command::Finite {
                        sequence,
                        input_seq: 1,
                        action: FiniteAction::Click { x, y, button: 1 },
                    },
                );
                // Observe real retirement even on injection refusal. Preserve
                // its primary error; Pending remains retained by Process Drop.
                let closed = owner.close();
                let accepted_qpc_100ns = accepted?;
                closed?;
                if before != peer.drained_snapshot()? {
                    return Err(Error::Invalid);
                }
                if Instant::now() >= deadline {
                    return Err(Error::Timeout);
                }
                software_sent = true;
                println!(
                    "{}",
                    serde_json::json!({"phase":"owned_software_readiness_click",
                    "target":current,"binding":binding,"accepted_qpc_100ns":accepted_qpc_100ns,
                    "actual_helper_retired":true,"all_six_peer_counts_unchanged":true,
                    "software_only":true,"automation_input":true,"physical_human_provenance":false,"physical_mouse_provenance":false})
                );
                // Re-observe receiver counts/release/foreground next iteration;
                // an Injected header alone never satisfies readiness.
                continue;
            }
            // SAFETY: read-only current foreground and initialized PID output.
            let foreground = unsafe { GetForegroundWindow() };
            let mut pid = 0;
            unsafe {
                GetWindowThreadProcessId(foreground, Some(&mut pid));
            }
            let window = foreground.0 as usize as u64;
            if !settling_initial
                && readiness_click(
                    baseline,
                    observed,
                    released,
                    window,
                    pid,
                    (self.target.window, self.target.process_id),
                )
            {
                self.verify()?;
                // No blocking call follows the final held-button/foreground
                // observation. Production focus/arm still make their guards.
                if mouse_released()
                    && unsafe { GetForegroundWindow() }.0 as usize as u64 == self.target.window
                {
                    println!(
                        "{}",
                        serde_json::json!({"phase":"initial_receiver_click_ready",
                        "target_window":self.target.window,"target_pid":self.target.process_id,
                        "baseline_mouse_down":baseline.0,"baseline_mouse_up":baseline.1,
                        "observed_mouse_down":observed.0,"observed_mouse_up":observed.1,
                        "balanced_new_receiver_pair":true,"buttons_released":true,
                        "unattended":software.is_some(),"software_click_sent":software_sent,"automation_input":software_sent,
                        "physical_mouse_provenance":false,"elapsed_ms":started.elapsed().as_millis()})
                    );
                    return Ok(());
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(Error::Timeout);
            }
            if now >= next_report {
                println!(
                    "{}",
                    serde_json::json!({"phase":"awaiting_initial_receiver_click",
                    "instruction":"Click and release inside the empty TARGET area; no buttons. Then leave both windows untouched during deliberate rapid movements.",
                    "target_window":self.target.window,"target_pid":self.target.process_id,
                    "foreground_window":window,"foreground_pid":pid,
                    "baseline_mouse_down":baseline.0,"baseline_mouse_up":baseline.1,
                    "observed_mouse_down":observed.0,"observed_mouse_up":observed.1,
                    "buttons_released":released,"settling_prebaseline_mouse":settling_initial,
                    "unattended":software.is_some(),"software_click_sent":software_sent,"automation_input":software_sent,
                    "elapsed_ms":started.elapsed().as_millis(),"deadline_ms":45000})
                );
                next_report = now + Duration::from_secs(1);
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
    fn focus(&self, peer: Option<&Self>) -> Result<Target> {
        let started = Instant::now();
        let deadline = started + Duration::from_millis(250);
        let mut attempt = 0u32;
        loop {
            if Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            self.verify()?;
            if let Some(peer) = peer {
                peer.verify()?;
            }
            attempt += 1;
            // SAFETY: read-only current native foreground and initialized PID output.
            let foreground = unsafe { GetForegroundWindow() };
            let mut foreground_pid = 0;
            unsafe {
                GetWindowThreadProcessId(foreground, Some(&mut foreground_pid));
            }
            let foreground_window = foreground.0 as usize as u64;
            let h = HWND(self.target.window as usize as *mut std::ffi::c_void);
            let mut rect = RECT::default();
            // SAFETY: query-only diagnostics on the exact verified task-owned HWND.
            let (visible, minimized, root, rect_result) = unsafe {
                (
                    IsWindowVisible(h).as_bool(),
                    IsIconic(h).as_bool(),
                    GetAncestor(h, GA_ROOT).0 as usize as u64,
                    GetWindowRect(h, &mut rect),
                )
            };
            println!(
                "{}",
                serde_json::json!({"phase":"bounded_owned_focus_attempt",
                "attempt":attempt,"elapsed_ms":started.elapsed().as_millis(),
                "target_window":self.target.window,"target_pid":self.target.process_id,
                "peer_window":peer.map(|p|p.target.window),"peer_pid":peer.map(|p|p.target.process_id),
                "foreground_window":foreground_window,"foreground_pid":foreground_pid,
                "target_visible":visible,"target_minimized":minimized,"target_root":root,
                "target_rect":if rect_result.is_ok(){Some([rect.left,rect.top,rect.right,rect.bottom])}else{None},
                "target_rect_error":rect_result.err().map(|e|e.code().0)})
            );
            let authority = match owned_foreground(
                foreground_window,
                foreground_pid,
                (self.target.window, self.target.process_id),
                peer.map(|p| (p.target.window, p.target.process_id)),
            )? {
                Some(true) => self,
                Some(false) => peer.ok_or(Error::Ungranted)?,
                None => {
                    // Null is an observed asynchronous transition, never permission to focus or inject.
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
            };
            // Revalidate retained image/PID creation/class and current foreground after diagnostics.
            authority.verify()?;
            if unsafe { GetForegroundWindow() }.0 as usize as u64 != authority.target.window {
                continue;
            }
            let returned = authority.message(HANDOFF, std::process::id() as usize)?;
            println!(
                "{}",
                serde_json::json!({"phase":"exact_foreground_handoff",
                "attempt":attempt,"authority_window":authority.target.window,
                "authority_pid":authority.target.process_id,"driver_pid":std::process::id(),"returned":returned})
            );
            if returned != 1 {
                return Err(Error::Ungranted);
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            let target = self.fresh()?;
            // No potentially blocking provider call follows this final owned-foreground query.
            // The unchanged production operation still makes its own exact target/foreground checks.
            authority.verify()?;
            if unsafe { GetForegroundWindow() }.0 as usize as u64 != authority.target.window {
                continue;
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            let focused = platform::focus_for_owner_grant(&target, std::process::id());
            let after = unsafe { GetForegroundWindow() };
            println!(
                "{}",
                serde_json::json!({"phase":"production_focus_result",
                "attempt":attempt,"elapsed_ms":started.elapsed().as_millis(),
                "target_window":target.window,"target_pid":target.process_id,
                "foreground_window":after.0 as usize as u64,"accepted":focused.is_ok(),
                "error":focused.as_ref().err().map(|e|format!("{e:?}"))})
            );
            match focused {
                Ok(()) => return Ok(target),
                Err(Error::Ungranted) => {
                    // Retry only after the next iteration freshly proves an owned foreground.
                    // Unrelated nonzero foreground refuses immediately; no input has been sent.
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => return Err(error),
            }
        }
    }
    fn mutate(&self, sink: &Self, mode: u32) -> Result<()> {
        self.verify()?;
        sink.verify()?;
        // SAFETY: explicit owner-ready mutation restricted to the two retained task surfaces.
        unsafe {
            let h = HWND(self.target.window as usize as *mut std::ffi::c_void);
            let r = self.target.window_rect;
            match mode {
                0 => platform::api(
                    "HIL move",
                    SetWindowPos(
                        h,
                        None,
                        r.x + 12,
                        r.y,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                    ),
                )?,
                1 => platform::api(
                    "HIL resize",
                    SetWindowPos(
                        h,
                        None,
                        0,
                        0,
                        r.width as i32 - 12,
                        r.height as i32,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    ),
                )?,
                2 => {
                    let _ = ShowWindow(h, SW_MINIMIZE);
                }
                3 => {
                    sink.focus(Some(self))?;
                }
                _ => return Err(Error::Invalid),
            }
        }
        Ok(())
    }
}
fn binding(target: &Target, counter: u64) -> Result<Binding> {
    Ok(Binding {
        scope: Scope {
            connection_epoch: 1,
            capture_session_id: id(counter * 2)?,
            source_generation: counter,
            target_token: target.token.clone(),
            geometry_revision: 1,
        },
        input_session_id: id(counter * 2 + 1)?,
    })
}
fn open(helper: Arc<LockedHelper>, target: &Target, counter: u64) -> Result<(Process, Binding)> {
    let binding = binding(target, counter)?;
    let (owner, ready) = Process::open(
        helper,
        Command::OpenInput {
            sequence: 1,
            target: target.clone(),
            binding: binding.clone(),
            owner_pid: std::process::id(),
            profile: None,
        },
        &Cancellation::default(),
    )?;
    if !matches!(
        ready.header,
        Header::Ready {
            input_authority: None,
            ..
        }
    ) {
        return Err(Error::Invalid);
    }
    Ok((owner, binding))
}
fn pen(target: &Target, phase: Phase, flags: u32) -> PenSample {
    PenSample {
        phase,
        x: target.client_rect.x + 100,
        y: target.client_rect.y + 100,
        pressure: 512,
        tilt_x: 12,
        tilt_y: -9,
        rotation: 0,
        pen_flags: flags,
    }
}
fn injected(
    owner: &mut Process,
    binding: &Binding,
    input_seq: u64,
    request: Command,
) -> Result<u64> {
    let reply = owner.exchange(request, &Cancellation::default())?;
    match reply.header {
        Header::Injected {
            binding: got,
            input_seq: got_seq,
            accepted_qpc_100ns,
            ..
        } if got == *binding && got_seq == input_seq && accepted_qpc_100ns > 0 => {
            Ok(accepted_qpc_100ns)
        }
        _ => Err(Error::Invalid),
    }
}
fn submit_pen(owner: &mut Process, binding: &Binding, seq: u64, sample: PenSample) -> Result<u64> {
    let sequence = owner.next_sequence()?;
    injected(
        owner,
        binding,
        seq,
        Command::Pen {
            sequence,
            input_seq: seq,
            sample,
        },
    )
}
fn write_case(log: &mut File, name: &str, value: serde_json::Value) -> Result<()> {
    serde_json::to_writer(
        &mut *log,
        &serde_json::json!({"case":name,"evidence":value}),
    )
    .map_err(|_| Error::Io)?;
    writeln!(log)?;
    log.sync_all()?;
    println!("remote guard case: {name}");
    Ok(())
}
// Diagnostic observations are collected only after the unchanged operation
// refuses. They cannot identify the original failure-time OS snapshot, create
// a grant, retry a producer or convert any refusal into successful admission.
fn target_differences(expected: &Target, observed: &Target) -> Vec<&'static str> {
    let mut values = Vec::new();
    macro_rules! field {
        ($name:ident) => {
            if expected.$name != observed.$name {
                values.push(stringify!($name));
            }
        };
    }
    field!(token);
    field!(window);
    field!(process_id);
    field!(thread_id);
    field!(process_created);
    field!(window_rect);
    field!(frame_rect);
    field!(client_rect);
    field!(dpi);
    field!(integrity);
    values
}
fn diagnosed<T>(
    trial: u32,
    mode: u32,
    stage: &'static str,
    surface: &OwnedSurface,
    expected: Option<&Target>,
    work: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let result = work();
    if let Err(error) = &result {
        let observed = surface.fresh();
        // SAFETY: read-only current foreground, initialized PID output, no input.
        let foreground = unsafe { GetForegroundWindow() };
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(foreground, Some(&mut pid));
        }
        println!(
            "{}",
            serde_json::json!({"phase":"guard_trial_refused",
            "trial":trial,"mode":mode,"stage":stage,"error":format!("{error:?}"),
            "expected":expected,"post_failure_observed":observed.as_ref().ok(),
            "observation_error":observed.as_ref().err().map(|e|format!("{e:?}")),
            "post_failure_scalar_mismatches":expected.zip(observed.as_ref().ok()).map(|(a,b)|target_differences(a,b)),
            "foreground_window":foreground.0 as usize as u64,"foreground_pid":pid,
            "observed_after_refusal":true,"original_failure_snapshot":false,
            "retry":false,"predicate_changed":false})
        );
    }
    result
}
fn drive(args: &[String]) -> Result<()> {
    if args.len() != 9 || !matches!(args[0].as_str(), "--owner-ready" | "--owner-unattended") {
        return Err(Error::Invalid);
    }
    platform::initialize_hil_dpi()?;
    let image = Arc::new(Image::own(Some(&args[2]))?);
    let helper = LockedHelper::open(Path::new(&args[1]), &args[3], Kind::Input)?;
    let number = |index: usize| args[index].parse::<u64>().map_err(|_| Error::Invalid);
    let target = OwnedSurface::open(
        number(4)?,
        u32::try_from(number(5)?).map_err(|_| Error::Invalid)?,
        image.clone(),
    )?;
    let sink = OwnedSurface::open(
        number(6)?,
        u32::try_from(number(7)?).map_err(|_| Error::Invalid)?,
        image,
    )?;
    if target.target.process_id == sink.target.process_id {
        return Err(Error::Invalid);
    }
    let mut log = new_file(Path::new(&args[8]))?;
    let result = (|| {
        target.wait_initial_owner_click(
            &sink,
            (args[0] == "--owner-unattended").then(|| helper.clone()),
        )?;
        let current = target.focus(Some(&sink))?;
        let (mut owner, binding) = open(helper.clone(), &current, 1)?;
        let downs = target.count(0)?;
        let qpc = submit_pen(&mut owner, &binding, 1, pen(&current, Phase::Down, 0))?;
        target.wait_count(0, downs + 1)?;
        submit_pen(&mut owner, &binding, 2, pen(&current, Phase::Up, 0))?;
        let check = owner.next_sequence()?;
        if !matches!(
            owner
                .exchange(Command::Check { sequence: check }, &Cancellation::default())?
                .header,
            Header::Idle { .. }
        ) {
            return Err(Error::Invalid);
        }
        let clicks = target.count(3)?;
        let releases = target.count(4)?;
        let wheels = target.count(5)?;
        let sink_before = sink.snapshot()?;
        let sequence = owner.next_sequence()?;
        injected(
            &mut owner,
            &binding,
            3,
            Command::Finite {
                sequence,
                input_seq: 3,
                action: FiniteAction::Click {
                    x: current.client_rect.x + 140,
                    y: current.client_rect.y + 160,
                    button: 1,
                },
            },
        )?;
        target.wait_count(3, clicks + 1)?;
        target.wait_count(4, releases + 1)?;
        let sequence = owner.next_sequence()?;
        injected(
            &mut owner,
            &binding,
            4,
            Command::Finite {
                sequence,
                input_seq: 4,
                action: FiniteAction::Wheel {
                    x: current.client_rect.x + 140,
                    y: current.client_rect.y + 160,
                    delta: 120,
                },
            },
        )?;
        target.wait_count(5, wheels + 1)?;
        if sink_before != sink.drained_snapshot()? {
            return Err(Error::Invalid);
        }
        write_case(
            &mut log,
            "delivered_pen_click_wheel",
            serde_json::json!({"binding":binding,"successful_injection_qpc_100ns":qpc,"target_downs":downs+1,"paired_click":true,"wheel":true,"sink_unchanged":true,"renderer_effect":false}),
        )?;
        let sequence = owner.next_sequence()?;
        let refused = owner.exchange(
            Command::Finite {
                sequence,
                input_seq: 5,
                action: FiniteAction::Wheel {
                    x: current.client_rect.x + 140,
                    y: current.client_rect.y + 160,
                    delta: i32::MIN,
                },
            },
            &Cancellation::default(),
        );
        if !matches!(refused, Err(Error::Invalid)) {
            return Err(Error::Invalid);
        }
        if !matches!(
            owner.exchange(
                Command::Check {
                    sequence: owner.next_sequence()?
                },
                &Cancellation::default()
            ),
            Err(Error::Unavailable)
        ) {
            return Err(Error::Invalid);
        }
        owner.close()?;
        if sink_before != sink.drained_snapshot()? {
            return Err(Error::Invalid);
        }
        write_case(
            &mut log,
            "min_wheel_refused_and_sealed",
            serde_json::json!({"returned":"Invalid","subsequent_owner":"Unavailable","retired":true}),
        )?;
        for trial in 0..100u32 {
            let mode = trial % 4;
            diagnosed(trial, mode, "old_target_restore", &target, None, || {
                target.restore()
            })?;
            diagnosed(trial, mode, "peer_restore", &sink, None, || sink.restore())?;
            let current = diagnosed(trial, mode, "old_target_focus_query", &target, None, || {
                target.focus(Some(&sink))
            })?;
            let (mut old, old_binding) = diagnosed(
                trial,
                mode,
                "old_helper_start_and_arm",
                &target,
                Some(&current),
                || open(helper.clone(), &current, u64::from(trial) + 2),
            )?;
            let downs = diagnosed(trial, mode, "receiver_down_census", &target, None, || {
                target.count(0)
            })?;
            diagnosed(
                trial,
                mode,
                "old_helper_pen_down",
                &target,
                Some(&current),
                || submit_pen(&mut old, &old_binding, 1, pen(&current, Phase::Down, 0)),
            )?;
            diagnosed(trial, mode, "receiver_wait_down", &target, None, || {
                target.wait_count(0, downs + 1)
            })?;
            let sink_before = diagnosed(trial, mode, "peer_before_all_six", &sink, None, || {
                sink.snapshot()
            })?;
            diagnosed(
                trial,
                mode,
                "deliberate_target_mutation",
                &target,
                Some(&current),
                || target.mutate(&sink, mode),
            )?;
            let result = old.exchange(
                Command::Pen {
                    sequence: old.next_sequence()?,
                    input_seq: 2,
                    sample: pen(&current, Phase::Move, 0),
                },
                &Cancellation::default(),
            );
            if !matches!(
                result,
                Err(Error::TargetChanged | Error::Unavailable | Error::Io)
            ) {
                return Err(Error::Invalid);
            }
            diagnosed(
                trial,
                mode,
                "old_helper_actual_retirement",
                &target,
                None,
                || old.close(),
            )?;
            diagnosed(trial, mode, "fresh_target_restore", &target, None, || {
                target.restore()
            })?;
            diagnosed(trial, mode, "peer_restore", &sink, None, || sink.restore())?;
            let new_target = diagnosed(
                trial,
                mode,
                "fresh_target_focus_query",
                &target,
                None,
                || target.focus(Some(&sink)),
            )?;
            if !matches!(
                old.exchange(
                    Command::Check {
                        sequence: old.next_sequence()?
                    },
                    &Cancellation::default()
                ),
                Err(Error::Unavailable)
            ) {
                return Err(Error::Invalid);
            }
            let (mut fresh, fresh_binding) = diagnosed(
                trial,
                mode,
                "fresh_helper_start_and_arm",
                &target,
                Some(&new_target),
                || open(helper.clone(), &new_target, u64::from(trial) + 1000),
            )?;
            if fresh_binding == old_binding {
                return Err(Error::Invalid);
            }
            let downs = diagnosed(trial, mode, "receiver_down_census", &target, None, || {
                target.count(0)
            })?;
            diagnosed(
                trial,
                mode,
                "fresh_helper_pen_down",
                &target,
                Some(&new_target),
                || {
                    submit_pen(
                        &mut fresh,
                        &fresh_binding,
                        1,
                        pen(&new_target, Phase::Down, 0),
                    )
                },
            )?;
            diagnosed(trial, mode, "receiver_wait_down", &target, None, || {
                target.wait_count(0, downs + 1)
            })?;
            diagnosed(
                trial,
                mode,
                "fresh_helper_pen_up",
                &target,
                Some(&new_target),
                || {
                    submit_pen(
                        &mut fresh,
                        &fresh_binding,
                        2,
                        pen(&new_target, Phase::Up, 0),
                    )
                },
            )?;
            diagnosed(
                trial,
                mode,
                "fresh_helper_actual_retirement",
                &target,
                None,
                || fresh.close(),
            )?;
            if sink_before
                != diagnosed(trial, mode, "peer_drained_all_six", &sink, None, || {
                    sink.drained_snapshot()
                })?
            {
                return Err(Error::Invalid);
            }
            write_case(
                &mut log,
                "target_mutation_latched_fresh_grant",
                serde_json::json!({"trial":trial,"mode":mode,"old_binding":old_binding,"fresh_binding":fresh_binding,"old_owner_retired":true,"sink_unchanged":true}),
            )?;
        }
        let current = target.focus(Some(&sink))?;
        let (mut owner, binding) = open(helper.clone(), &current, 3000)?;
        let cancel = Cancellation::default();
        cancel.cancel();
        if !matches!(
            owner.exchange(
                Command::Check {
                    sequence: owner.next_sequence()?
                },
                &cancel
            ),
            Err(Error::Cancelled)
        ) {
            return Err(Error::Invalid);
        }
        owner.close()?;
        write_case(
            &mut log,
            "explicit_cancel_retires_actual_helper",
            serde_json::json!({"binding":binding,"retired":true}),
        )?;
        if process::retry_retirements()? != 0 {
            return Err(Error::RetirementPending);
        }
        Ok(())
    })();
    let restored = target.restore().and_then(|()| sink.restore());
    result.and(restored)
}
// Pure classification supports regressions without launching or injecting anything.
// None means observe null only; a nonzero unrelated foreground always refuses.
fn owned_foreground(
    window: u64,
    pid: u32,
    target: (u64, u32),
    peer: Option<(u64, u32)>,
) -> Result<Option<bool>> {
    if window == 0 && pid == 0 {
        return Ok(None);
    }
    if (window, pid) == target {
        return Ok(Some(true));
    }
    if peer == Some((window, pid)) {
        return Ok(Some(false));
    }
    Err(Error::Ungranted)
}
fn initial_owner_foreground(window: u64, pid: u32, target: (u64, u32)) -> bool {
    window != 0 && pid != 0 && (window, pid) == target
}
fn mouse_released() -> bool {
    // SAFETY: query only global button state; no key/button injection or clearing.
    [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON, VK_XBUTTON1, VK_XBUTTON2]
        .iter()
        .all(|button| unsafe { GetAsyncKeyState(i32::from(button.0)) } >= 0)
}
fn readiness_click(
    baseline: (usize, usize),
    observed: (usize, usize),
    released: bool,
    window: u64,
    pid: u32,
    target: (u64, u32),
) -> bool {
    let down = observed.0.checked_sub(baseline.0);
    let up = observed.1.checked_sub(baseline.1);
    released
        && initial_owner_foreground(window, pid, target)
        && matches!((down,up),(Some(d),Some(u)) if d>0 && d==u)
}
pub fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() == 3 && matches!(args[0].as_str(), "--surface" | "--surface-controller") {
        surface(
            Path::new(&args[1]),
            args[2].parse().map_err(|_| Error::Invalid)?,
            args[0] == "--surface-controller",
        )
    } else {
        drive(&args)
    }
}
#[cfg(test)]
mod receiver_provenance_tests {
    use super::*;

    #[test]
    fn failed_source_query_keeps_enum_fields_unknown() {
        assert_eq!(
            input_provenance(None, 0),
            serde_json::json!({"schema":1,"source_available":false,
                "device_type":null,"origin_id":null,"message_extra_info":"0000000000000000"})
        );
    }

    #[test]
    fn unavailable_and_unknown_native_enums_are_not_reclassified() {
        for source in [(0, 0), (8, 2), (2, 4), (i32::MIN, i32::MAX)] {
            let observed = input_provenance(Some(source), 0);
            assert_eq!(observed["source_available"], true);
            assert_eq!(observed["device_type"], source.0);
            assert_eq!(observed["origin_id"], source.1);
            assert_eq!(observed.as_object().map(|v| v.len()), Some(5));
        }
    }

    #[test]
    fn extra_info_preserves_all_bits_without_promotion_inference() {
        for (raw, expected) in [
            (0, "0000000000000000"),
            (0xff515700, "00000000ff515700"),
            (u64::MAX, "ffffffffffffffff"),
        ] {
            let observed = input_provenance(Some((2, 2)), raw);
            assert_eq!(observed["message_extra_info"], expected);
            assert_eq!(observed["device_type"], 2);
            assert_eq!(observed["origin_id"], 2);
            assert_eq!(observed.as_object().map(|v| v.len()), Some(5));
        }
    }

    #[test]
    fn controller_lifecycle_consumption_does_not_change_mouse_or_close_dispatch() {
        for message in [WM_POINTERENTER, WM_POINTERLEAVE, WM_POINTERCAPTURECHANGED] {
            assert!(consume_controller_pointer_lifecycle(true, message));
            assert!(!consume_controller_pointer_lifecycle(false, message));
        }
        for message in [
            WM_POINTERDOWN,
            WM_POINTERUPDATE,
            WM_POINTERUP,
            WM_LBUTTONDOWN,
            WM_LBUTTONUP,
            WM_RBUTTONDOWN,
            WM_RBUTTONUP,
            WM_MOUSEWHEEL,
            WM_CLOSE,
            WM_DESTROY,
            WM_TIMER,
            WM_ACTIVATE,
        ] {
            assert!(!consume_controller_pointer_lifecycle(true, message));
            assert!(!consume_controller_pointer_lifecycle(false, message));
        }
    }
}
#[cfg(test)]
mod focus_transition_tests {
    use super::*;
    #[test]
    fn initial_owner_click_is_exact_target_only() {
        assert!(initial_owner_foreground(10, 20, (10, 20)));
        for observed in [(0, 0), (10, 0), (0, 20), (30, 40), (10, 40), (30, 20)] {
            assert!(!initial_owner_foreground(observed.0, observed.1, (10, 20)));
        }
    }

    #[test]
    fn null_transition_never_authorizes_focus() -> Result<()> {
        assert!(owned_foreground(0, 0, (10, 20), Some((30, 40)))?.is_none());
        assert!(matches!(
            owned_foreground(0, 20, (10, 20), Some((30, 40))),
            Err(Error::Ungranted)
        ));
        Ok(())
    }
    #[test]
    fn owned_foreground_requires_exact_window_and_process() -> Result<()> {
        assert_eq!(
            owned_foreground(10, 20, (10, 20), Some((30, 40)))?,
            Some(true)
        );
        assert_eq!(
            owned_foreground(30, 40, (10, 20), Some((30, 40)))?,
            Some(false)
        );
        for observed in [(10, 40), (30, 20), (99, 20), (99, 99)] {
            assert!(matches!(
                owned_foreground(observed.0, observed.1, (10, 20), Some((30, 40))),
                Err(Error::Ungranted)
            ));
        }
        Ok(())
    }

    #[test]
    fn failure_differences_preserve_exact_geometry_and_creation_fields() -> Result<()> {
        let rect = vw_remote::Rect {
            x: 80,
            y: 120,
            width: 640,
            height: 560,
        };
        let expected = Target {
            token: id(77)?,
            window: 10,
            process_id: 20,
            thread_id: 30,
            process_created: 40,
            window_rect: rect,
            frame_rect: rect,
            client_rect: rect,
            dpi: 96,
            integrity: 0x2000,
        };
        assert!(target_differences(&expected, &expected).is_empty());
        let mut observed = expected.clone();
        observed.client_rect.x += 1;
        observed.process_created += 1;
        assert_eq!(
            target_differences(&expected, &observed),
            vec!["process_created", "client_rect"]
        );
        observed = expected.clone();
        observed.frame_rect.width -= 1;
        observed.dpi = 120;
        assert_eq!(
            target_differences(&expected, &observed),
            vec!["frame_rect", "dpi"]
        );
        Ok(())
    }

    #[test]
    fn automatic_foreground_without_new_mouse_pair_never_starts() {
        assert!(!readiness_click((0, 0), (0, 0), true, 10, 20, (10, 20)));
        assert!(!readiness_click((4, 4), (4, 4), true, 10, 20, (10, 20)));
    }
    #[test]
    fn new_pair_requires_release_and_exact_owned_foreground() {
        assert!(readiness_click((4, 4), (5, 5), true, 10, 20, (10, 20)));
        for (counts, released, window, pid) in [
            ((5, 4), true, 10, 20),
            ((4, 5), true, 10, 20),
            ((5, 5), false, 10, 20),
            ((5, 5), true, 30, 40),
            ((5, 5), true, 10, 40),
            ((3, 3), true, 10, 20),
            ((5, 6), true, 10, 20),
        ] {
            assert!(!readiness_click(
                (4, 4),
                counts,
                released,
                window,
                pid,
                (10, 20)
            ));
        }
    }
}
