//! All Win32 FFI for the guarded injector is confined to this module.
use super::guard::{Guard, Rect, Target};
use std::mem::size_of;
use vw_remote::wire::{PenSample as Sample, Phase};
use windows::Win32::{
    Foundation::{CloseHandle, FILETIME, HANDLE, HWND, POINT, RECT},
    Graphics::Gdi::ClientToScreen,
    Security::*,
    System::Threading::*,
    UI::{Controls::*, HiDpi::*, Input::Pointer::*, WindowsAndMessaging::*},
};

pub type GuardResult<T> = std::result::Result<T, String>;
type Result<T> = GuardResult<T>;
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
#[cfg(feature = "integration-carrier-fault")]
mod release_witness;
#[cfg(feature = "integration-carrier-fault")]
pub(super) use release_witness::Record as ReleaseRecord;
/// Only a completed noncontact send can permit one guarded departure. Clearing
/// before every attempt preserves direct destruction after any ambiguous error.
#[derive(Default)]
struct HoverRetirement {
    eligible: Option<Sample>,
}
impl HoverRetirement {
    fn begin_attempt(&mut self) {
        self.eligible = None;
    }
    fn accepted(&mut self, sample: Sample) {
        self.eligible = (matches!(sample.phase, Phase::Up | Phase::Hover)
            && sample.pressure == 0
            && sample.pen_flags == 0)
            .then_some(sample);
    }
    fn take_leave(&mut self, guard_suspended: bool) -> Option<Sample> {
        let sample = self.eligible.take()?;
        (!guard_suspended).then_some(Sample {
            phase: Phase::Leave,
            x: sample.x,
            y: sample.y,
            pressure: 0,
            tilt_x: 0,
            tilt_y: 0,
            rotation: 0,
            pen_flags: 0,
        })
    }
}

pub struct Injector {
    device: HSYNTHETICPOINTERDEVICE,
    target: Target,
    guard: Guard,
    session: u64,
    last_contact: Option<std::time::Instant>,
    allow_no_refresh: bool,
    hover_retirement: HoverRetirement,
    #[cfg(feature = "integration-carrier-fault")]
    release_record: Option<std::sync::Arc<ReleaseRecord>>,
    #[cfg(feature = "integration-carrier-fault")]
    actual_held: bool,
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
            hover_retirement: HoverRetirement::default(),
            #[cfg(feature = "integration-carrier-fault")]
            release_record: None,
            #[cfg(feature = "integration-carrier-fault")]
            actual_held: false,
        })
    }
    #[cfg(feature = "integration-carrier-fault")]
    pub(super) fn with_release_record(mut self, record: std::sync::Arc<ReleaseRecord>) -> Self {
        self.release_record = Some(record);
        self
    }
    pub fn contact_started(&self) -> Option<std::time::Instant> {
        self.last_contact
    }
    pub fn validate_point(&mut self, x: i32, y: i32) -> Result<()> {
        let current = match snapshot(self.target.hwnd, self.target.pid) {
            Ok(v) => v,
            Err(e) => {
                self.guard.suspend();
                return Err(e);
            }
        };
        // SAFETY: query-only root hit test on the already bounded physical point.
        let hit = unsafe { GetAncestor(WindowFromPoint(POINT { x, y }), GA_ROOT).0 as usize };
        self.guard
            .check(current, foreground(), hit, self.session, (x, y))
            .map_err(|e| e.to_string())
    }
    pub fn send(&mut self, s: Sample) -> Result<()> {
        self.hover_retirement.begin_attempt();
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
        #[cfg(feature = "integration-carrier-fault")]
        {
            self.actual_held = matches!(s.phase, Phase::Down | Phase::Move);
        }
        let completed = std::time::Instant::now();
        self.last_contact = match crate::contact::next(
            self.last_contact,
            submitted,
            completed,
            matches!(s.phase, Phase::Down | Phase::Move),
        ) {
            Ok(next) => next,
            Err(_) => {
                self.guard.suspend();
                return Err("Input paused: blocking injection missed the contact deadline".into());
            }
        };
        self.hover_retirement.accepted(s);
        Ok(())
    }
}
impl Drop for Injector {
    fn drop(&mut self) {
        // Preserve the pre-retirement held state for the actual destruction
        // witness. A graceful departure must never manufacture a release.
        #[cfg(feature = "integration-carrier-fault")]
        let held_before_release = self.actual_held;
        if let Some(leave) = self.hover_retirement.take_leave(self.guard.suspended()) {
            // UP intentionally leaves a pen in hover range. Retire that range
            // through the same exact target/foreground/hit-test guard, after UP
            // has already returned to its caller. Never retry or bypass a guard.
            // Held, unknown, failed-send and suspended states have no candidate.
            let _ = self.send(leave);
        }
        // SAFETY: Device is uniquely owned. Never send an unguarded UP after focus/geometry loss.
        #[cfg(feature = "integration-carrier-fault")]
        if let Some(record) = &self.release_record {
            record.release(held_before_release, || unsafe {
                DestroySyntheticPointerDevice(self.device)
            });
            return;
        }
        unsafe { DestroySyntheticPointerDevice(self.device) };
    }
}

#[cfg(test)]
mod retirement_tests {
    use super::*;

    fn sample(phase: Phase) -> Sample {
        Sample {
            phase,
            x: 101,
            y: 202,
            pressure: 0,
            tilt_x: 10,
            tilt_y: -10,
            rotation: 90,
            pen_flags: 0,
        }
    }

    #[test]
    fn accepted_up_retires_only_the_same_noncontact_point() {
        let mut state = HoverRetirement::default();
        state.accepted(sample(Phase::Up));
        assert_eq!(
            state.take_leave(false),
            Some(Sample {
                phase: Phase::Leave,
                x: 101,
                y: 202,
                pressure: 0,
                tilt_x: 0,
                tilt_y: 0,
                rotation: 0,
                pen_flags: 0,
            })
        );
        assert!(state.take_leave(false).is_none());
    }

    #[test]
    fn accepted_hover_can_depart_without_contact() {
        let mut state = HoverRetirement::default();
        state.accepted(sample(Phase::Hover));
        assert!(state.take_leave(false).is_some());
    }

    #[test]
    fn held_down_and_move_never_synthesize_retirement_input() {
        for phase in [Phase::Down, Phase::Move] {
            let mut state = HoverRetirement::default();
            state.accepted(sample(Phase::Up));
            state.begin_attempt();
            state.accepted(sample(phase));
            assert!(state.take_leave(false).is_none());
        }
    }

    #[test]
    fn failed_attempt_clears_earlier_hover_eligibility() {
        let mut state = HoverRetirement::default();
        state.accepted(sample(Phase::Up));
        state.begin_attempt();
        // Validation, snapshot, guard, OS or completion-deadline failure returns
        // before accepted(), even if an earlier UP established hover range.
        assert!(state.take_leave(false).is_none());
    }

    #[test]
    fn new_device_has_no_pointer_to_retire() {
        assert!(HoverRetirement::default().take_leave(false).is_none());
    }

    #[test]
    fn suspended_guard_consumes_candidate_without_input() {
        let mut state = HoverRetirement::default();
        state.accepted(sample(Phase::Up));
        assert!(state.take_leave(true).is_none());
        assert!(state.take_leave(false).is_none());
    }

    #[test]
    fn successful_leave_cannot_generate_a_second_leave() -> Result<()> {
        let mut state = HoverRetirement::default();
        state.accepted(sample(Phase::Up));
        let leave = state.take_leave(false).ok_or("completed UP candidate")?;
        state.begin_attempt();
        state.accepted(leave);
        assert!(state.take_leave(false).is_none());
        Ok(())
    }

    #[test]
    fn pressure_or_pen_buttons_refuse_graceful_retirement() {
        for (pressure, pen_flags) in [(1, 0), (0, 1), (0, 2), (0, 4)] {
            let mut state = HoverRetirement::default();
            let mut value = sample(Phase::Up);
            value.pressure = pressure;
            value.pen_flags = pen_flags;
            state.accepted(value);
            assert!(state.take_leave(false).is_none());
        }
    }

    #[test]
    fn departure_flags_never_include_contact_buttons_or_range() {
        let flags = pointer_flags(Phase::Leave);
        assert_eq!(flags, POINTER_FLAG_PRIMARY.0 | POINTER_FLAG_UPDATE.0);
        assert_eq!(
            flags
                & (POINTER_FLAG_INRANGE.0
                    | POINTER_FLAG_INCONTACT.0
                    | POINTER_FLAG_FIRSTBUTTON.0
                    | POINTER_FLAG_CANCELED.0
                    | POINTER_FLAG_UP.0),
            0
        );
    }

    #[test]
    fn retirement_refusal_does_not_restore_a_candidate() {
        let mut state = HoverRetirement::default();
        state.accepted(sample(Phase::Up));
        assert!(state.take_leave(false).is_some());
        // send() clears before snapshot/hit-test/injection; refusal has no
        // accepted() call and destruction proceeds without a second attempt.
        state.begin_attempt();
        assert!(state.take_leave(false).is_none());
    }
}
