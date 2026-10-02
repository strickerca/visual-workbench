//! All Win32 FFI for the guarded injector is confined to this module.
use crate::{
    Guard, Rect, Target,
    script::{Phase, Sample},
};
use std::mem::size_of;
use windows::Win32::{
    Foundation::{CloseHandle, FILETIME, HANDLE, HWND, LPARAM, POINT, RECT, WPARAM},
    Graphics::Gdi::ClientToScreen,
    Security::*,
    System::Threading::*,
    UI::{Controls::*, HiDpi::*, Input::Pointer::*, WindowsAndMessaging::*},
};

type Result<T> = std::result::Result<T, String>;
fn error(e: windows::core::Error) -> String {
    format!("Windows error 0x{:08x}", e.code().0 as u32)
}

pub fn initialize_dpi() -> Result<()> {
    // SAFETY: Called once before window, coordinates or injection APIs.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
        .map_err(error)
}
pub fn hwnd(value: usize) -> HWND {
    HWND(value as *mut std::ffi::c_void)
}
pub fn foreground() -> usize {
    // SAFETY: Query only; handle is treated as an opaque identity.
    unsafe { GetForegroundWindow().0 as usize }
}
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: Handle is uniquely owned and came from a successful Open* call.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn integrity(process: HANDLE) -> Result<u32> {
    let mut token = HANDLE::default();
    // SAFETY: Live process and writable token handle; TOKEN_QUERY is read-only.
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.map_err(error)?;
    let token = Handle(token);
    let mut length = 0;
    // SAFETY: Length-only query; no output buffer is supplied.
    let _ = unsafe { GetTokenInformation(token.0, TokenIntegrityLevel, None, 0, &mut length) };
    if !(size_of::<TOKEN_MANDATORY_LABEL>() as u32..=65536).contains(&length) {
        return Err("Cannot determine bounded token integrity size".into());
    }
    // usize storage supplies the alignment required by TOKEN_MANDATORY_LABEL.
    let mut storage = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
    // SAFETY: Storage is aligned, initialized and at least length bytes long.
    unsafe {
        GetTokenInformation(
            token.0,
            TokenIntegrityLevel,
            Some(storage.as_mut_ptr().cast()),
            length,
            &mut length,
        )
    }
    .map_err(error)?;
    // SAFETY: Successful API populated a TOKEN_MANDATORY_LABEL and its SID in this buffer.
    unsafe {
        let label = &*storage.as_ptr().cast::<TOKEN_MANDATORY_LABEL>();
        if !IsValidSid(label.Label.Sid).as_bool() {
            return Err("Invalid integrity SID".into());
        }
        let count = *GetSidSubAuthorityCount(label.Label.Sid);
        if count == 0 {
            return Err("Missing integrity RID".into());
        }
        Ok(*GetSidSubAuthority(label.Label.Sid, u32::from(count - 1)))
    }
}
pub fn caller_integrity() -> Result<u32> {
    // SAFETY: Pseudo-handle remains valid and is borrowed, never closed here.
    integrity(unsafe { GetCurrentProcess() })
}

/// Temporary input-thread priority only; no machine-wide timer or priority changes.
pub struct InputPriority {
    previous: THREAD_PRIORITY,
    _thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}
pub fn input_priority() -> Result<InputPriority> {
    // SAFETY: Query/set only this thread's borrowed pseudo-handle.
    let previous = unsafe { GetThreadPriority(GetCurrentThread()) };
    if previous == i32::MAX {
        return Err("Cannot read input thread priority".into());
    }
    // SAFETY: A small, non-realtime priority increase for this sleeping input thread.
    unsafe {
        SetThreadPriority(
            GetCurrentThread(),
            THREAD_PRIORITY(previous.max(THREAD_PRIORITY_ABOVE_NORMAL.0)),
        )
    }
    .map_err(error)?;
    Ok(InputPriority {
        previous: THREAD_PRIORITY(previous),
        _thread_bound: std::marker::PhantomData,
    })
}
impl Drop for InputPriority {
    fn drop(&mut self) {
        // SAFETY: !Send/!Sync marker keeps the guard on its original thread; restore its saved priority.
        let _ = unsafe { SetThreadPriority(GetCurrentThread(), self.previous) };
    }
}
fn rect(r: RECT) -> Rect {
    Rect {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}

pub fn snapshot(value: usize, expected_pid: u32) -> Result<Target> {
    let h = hwnd(value);
    let mut pid = 0;
    let mut window = RECT::default();
    let mut client = RECT::default();
    // SAFETY: The OS validates opaque HWNDs; all output structures are initialized.
    unsafe {
        if !IsWindow(Some(h)).as_bool()
            || !IsWindowVisible(h).as_bool()
            || IsIconic(h).as_bool()
            || GetWindowLongW(h, GWL_STYLE) as u32 & WS_DISABLED.0 != 0
            || GetAncestor(h, GA_ROOT) != h
        {
            return Err(
                "Target is absent, hidden, disabled, minimized or not a root window".into(),
            );
        }
        let thread = GetWindowThreadProcessId(h, Some(&mut pid));
        if thread == 0 || pid != expected_pid {
            return Err("Target process identity changed".into());
        }
        GetWindowRect(h, &mut window).map_err(error)?;
        GetClientRect(h, &mut client).map_err(error)?;
        let mut origin = POINT {
            x: client.left,
            y: client.top,
        };
        let mut end = POINT {
            x: client.right,
            y: client.bottom,
        };
        if !ClientToScreen(h, &mut origin).as_bool() || !ClientToScreen(h, &mut end).as_bool() {
            return Err("Client-to-physical-screen conversion failed".into());
        }
        let dpi = GetDpiForWindow(h);
        let process =
            Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).map_err(error)?);
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        GetProcessTimes(process.0, &mut created, &mut exited, &mut kernel, &mut user)
            .map_err(error)?;
        let level = integrity(process.0)?;
        Ok(Target {
            hwnd: value,
            pid,
            thread,
            process_created: (u64::from(created.dwHighDateTime) << 32)
                | u64::from(created.dwLowDateTime),
            window: rect(window),
            client: Rect {
                left: origin.x,
                top: origin.y,
                right: end.x,
                bottom: end.y,
            },
            dpi,
            integrity: level,
        })
    }
}

pub fn pointer_flags(phase: Phase) -> u32 {
    let base = POINTER_FLAG_PRIMARY.0;
    base | match phase {
        Phase::Hover => POINTER_FLAG_INRANGE.0 | POINTER_FLAG_UPDATE.0,
        Phase::Down => {
            POINTER_FLAG_INRANGE.0
                | POINTER_FLAG_INCONTACT.0
                | POINTER_FLAG_DOWN.0
                | POINTER_FLAG_FIRSTBUTTON.0
        }
        Phase::Move => {
            POINTER_FLAG_INRANGE.0
                | POINTER_FLAG_INCONTACT.0
                | POINTER_FLAG_UPDATE.0
                | POINTER_FLAG_FIRSTBUTTON.0
        }
        Phase::Up => POINTER_FLAG_INRANGE.0 | POINTER_FLAG_UP.0,
        Phase::Leave => POINTER_FLAG_UPDATE.0,
    }
}

/// Device cannot be used without the latched guard; raw handle never escapes.
pub struct Injector {
    device: HSYNTHETICPOINTERDEVICE,
    target: Target,
    guard: Guard,
    session: u64,
    last_contact: Option<std::time::Instant>,
    allow_no_refresh: bool,
}
impl Injector {
    pub fn arm(target: Target, session: u64, allow_no_refresh: bool) -> Result<Self> {
        let guard = Guard::new(target, session, caller_integrity()?).map_err(|e| e.to_string())?;
        if foreground() != target.hwnd {
            return Err("Target must already be foreground".into());
        }
        // SAFETY: A single PT_PEN device; creation itself injects no input.
        let device = unsafe { CreateSyntheticPointerDevice(PT_PEN, 1, POINTER_FEEDBACK_DEFAULT) }
            .map_err(error)?;
        Ok(Self {
            device,
            target,
            guard,
            session,
            last_contact: None,
            allow_no_refresh,
        })
    }
    pub fn send(&mut self, s: Sample) -> Result<()> {
        if s.pressure > 1024
            || s.rotation > 359
            || !(-90..=90).contains(&s.tilt_x)
            || !(-90..=90).contains(&s.tilt_y)
            || s.pen_flags & !7 != 0
        {
            self.guard.suspend();
            return Err("Sample outside native pen bounds".into());
        }
        let current = match snapshot(self.target.hwnd, self.target.pid) {
            Ok(current) => current,
            Err(e) => {
                self.guard.suspend();
                return Err(e);
            }
        };
        let point = POINT { x: s.x, y: s.y };
        // SAFETY: Read-only hit test, including children; an occluding popup fails.
        let hit = unsafe { GetAncestor(WindowFromPoint(point), GA_ROOT).0 as usize };
        self.guard
            .check(current, foreground(), hit, self.session, (s.x, s.y))
            .map_err(|e| e.to_string())?;
        let pen = POINTER_PEN_INFO {
            pointerInfo: POINTER_INFO {
                pointerType: PT_PEN,
                pointerId: 1,
                pointerFlags: POINTER_FLAGS(pointer_flags(s.phase)),
                ptPixelLocation: point,
                ..Default::default()
            },
            penFlags: s.pen_flags,
            penMask: 15,
            pressure: s.pressure,
            rotation: s.rotation,
            tiltX: s.tilt_x,
            tiltY: s.tilt_y,
        };
        let input = POINTER_TYPE_INFO {
            r#type: PT_PEN,
            Anonymous: POINTER_TYPE_INFO_0 { penInfo: pen },
        };
        // SAFETY: Fully initialized single-pen input; every call is preceded by the guard.
        // Windows offers no atomic guard-and-inject API; the remaining race is measured by HIL.
        if !self.allow_no_refresh
            && self
                .last_contact
                .is_some_and(|last| last.elapsed() > std::time::Duration::from_millis(50))
        {
            self.guard.suspend();
            return Err(format!(
                "Input paused: guard/scheduler exceeded the 50 ms contact deadline (gap {:.3} ms)",
                self.last_contact
                    .map_or(0.0, |last| last.elapsed().as_secs_f64() * 1000.0)
            ));
        }
        let submitted = std::time::Instant::now();
        if let Err(e) = unsafe { InjectSyntheticPointerInput(self.device, &[input]) } {
            self.guard.suspend();
            return Err(error(e));
        }
        self.last_contact = if matches!(s.phase, Phase::Down | Phase::Move) {
            Some(submitted)
        } else {
            None
        };
        Ok(())
    }
}
impl Drop for Injector {
    fn drop(&mut self) {
        // SAFETY: Device is uniquely owned. Never send an unguarded UP after focus/geometry loss.
        unsafe { DestroySyntheticPointerDevice(self.device) };
    }
}

/// HIL support: window mutations are restricted to explicitly selected probe windows.
pub fn require_harness(value: usize, pid: u32) -> Result<()> {
    snapshot(value, pid)?;
    let mut class = [0u16; 128];
    // SAFETY: Bounded output buffer; read-only class lookup on the selected window.
    let count = unsafe { GetClassNameW(hwnd(value), &mut class) };
    if count <= 0
        || String::from_utf16_lossy(&class[..count as usize]) != "VisualWorkbenchPenHarnessT005"
    {
        return Err("Guard trials may manipulate only Visual Workbench harness windows".into());
    }
    Ok(())
}

pub const FOREGROUND_HANDOFF: u32 = WM_APP + 77;
pub const NATIVE_DOWN_COUNT: u32 = WM_APP + 78;

/// Read the recorder's delivered-contact count, not the sender's API-call count.
pub fn native_down_count(target: Target) -> Result<usize> {
    verify_harness_identity(target)?;
    let mut count = 0;
    // SAFETY: Pointer-free, bounded query to the validated probe window. A failed
    // query never becomes a zero count or successful delivery acknowledgment.
    let status = unsafe {
        SendMessageTimeoutW(
            hwnd(target.hwnd),
            NATIVE_DOWN_COUNT,
            WPARAM(0),
            LPARAM(0),
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            100,
            Some(&mut count),
        )
    };
    if status.0 == 0 {
        return Err("Native contact acknowledgment unavailable".into());
    }
    Ok(count)
}

pub fn verified_injector(pid: u32) -> Result<()> {
    if pid == 0 || pid == u32::MAX {
        return Err("Invalid foreground controller".into());
    }
    let mut name = [0u16; 32768];
    let mut length = name.len() as u32;
    // SAFETY: Read-only query on a uniquely owned handle; output buffer is bounded.
    unsafe {
        let process =
            Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).map_err(error)?);
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(name.as_mut_ptr()),
            &mut length,
        )
        .map_err(error)?;
    }
    let actual = std::fs::canonicalize(String::from_utf16_lossy(&name[..length as usize]))
        .map_err(|_| "Cannot resolve controller image")?;
    let current = std::env::current_exe().map_err(|_| "Cannot resolve harness image")?;
    let expected = std::fs::canonicalize(
        current
            .parent()
            .ok_or("Missing executable parent")?
            .join("pen-inject.exe"),
    )
    .map_err(|_| "Cannot resolve sibling injector")?;
    if !actual
        .to_string_lossy()
        .eq_ignore_ascii_case(&expected.to_string_lossy())
    {
        return Err("Foreground handoff is restricted to the sibling pen-inject executable".into());
    }
    Ok(())
}
pub fn request_foreground_handoff(target: Target) -> Result<()> {
    verify_harness_identity(target)?;
    let mut accepted = 0;
    // SAFETY: A bounded, pointer-free message to the validated owned harness. The
    // receiver verifies this process image before granting foreground rights.
    let result = unsafe {
        SendMessageTimeoutW(
            hwnd(target.hwnd),
            FOREGROUND_HANDOFF,
            WPARAM(GetCurrentProcessId() as usize),
            LPARAM(0),
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            500,
            Some(&mut accepted),
        )
    };
    if result.0 == 0 || accepted != 1 {
        return Err("Harness foreground handoff unavailable".into());
    }
    Ok(())
}

pub fn restore_harness(target: Target) -> Result<()> {
    verify_harness_identity(target)?;
    // SAFETY: Caller validated this task-owned harness. Restore only its original geometry.
    unsafe {
        let _ = ShowWindow(hwnd(target.hwnd), SW_RESTORE);
        SetWindowPos(
            hwnd(target.hwnd),
            None,
            target.window.left,
            target.window.top,
            target.window.right - target.window.left,
            target.window.bottom - target.window.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .map_err(error)
    }
}
pub fn focus_harness(target: Target) -> Result<()> {
    verify_harness_identity(target)?;
    if foreground() == target.hwnd {
        return Ok(());
    }
    // SAFETY: Only used by the explicitly owner-ready trial, with a validated probe HWND.
    if !unsafe { SetForegroundWindow(hwnd(target.hwnd)) }.as_bool() {
        return Err("INCONCLUSIVE: Windows denied harness activation; no focus bypass used".into());
    }
    Ok(())
}
pub fn mutate_harness(target: Target, sink: Target, mode: u32) -> Result<()> {
    verify_harness_identity(target)?;
    // SAFETY: Caller validates both owned harness windows and restores geometry after the trial.
    unsafe {
        match mode {
            0 => SetWindowPos(
                hwnd(target.hwnd),
                None,
                target.window.left + 12,
                target.window.top,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .map_err(error),
            1 => SetWindowPos(
                hwnd(target.hwnd),
                None,
                0,
                0,
                target.window.right - target.window.left - 12,
                target.window.bottom - target.window.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .map_err(error),
            2 => {
                let _ = ShowWindow(hwnd(target.hwnd), SW_MINIMIZE);
                Ok(())
            }
            3 => {
                // Injected input is delivered to the harness and can revoke the
                // controller's old foreground grant. Renew it after that input.
                request_foreground_handoff(target)?;
                focus_harness(sink)
            }
            _ => Err("Unknown guard trial".into()),
        }
    }
}

fn verify_harness_identity(target: Target) -> Result<()> {
    let mut class = [0u16; 128];
    let mut pid = 0;
    // SAFETY: Read-only identity checks with initialized outputs. Works while minimized,
    // allowing cleanup without weakening snapshot()'s injection guard.
    unsafe {
        let thread = GetWindowThreadProcessId(hwnd(target.hwnd), Some(&mut pid));
        let count = GetClassNameW(hwnd(target.hwnd), &mut class);
        if pid != target.pid
            || thread != target.thread
            || count <= 0
            || String::from_utf16_lossy(&class[..count as usize]) != "VisualWorkbenchPenHarnessT005"
        {
            return Err("Harness identity changed; refusing window mutation".into());
        }
        let process =
            Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).map_err(error)?);
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        GetProcessTimes(process.0, &mut created, &mut exited, &mut kernel, &mut user)
            .map_err(error)?;
        let stamp = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        if stamp != target.process_created {
            return Err("Harness process was replaced".into());
        }
    }
    Ok(())
}
