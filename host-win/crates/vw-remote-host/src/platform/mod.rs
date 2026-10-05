//! Windows boundary, executed only inside owned helper processes except query_target.
mod encoder;
pub mod finite_input;
pub mod guard;
mod input;
mod native_guard;
mod shortcuts;
pub(crate) use shortcuts::{
    Apartment as EffectApartment, ClickMeasurement, Measurement, measure, measure_click,
};
pub use shortcuts::{
    PreparedBatch, prepare, prepare_batch, validate_canvas, validate_current, validate_prepared,
};
pub(crate) use shortcuts::{krita_controls, paint_controls};
mod video;
use crate::{Error, Result};
use vw_remote::{Rect, Target};
use windows::Win32::{
    Foundation::*,
    System::{JobObjects::*, Threading::*},
    UI::WindowsAndMessaging::*,
};
pub fn api<T>(phase: &str, result: windows::core::Result<T>) -> Result<T> {
    result.map_err(|e| Error::Platform {
        phase: phase.into(),
        code: e.code().0 as u32,
    })
}
pub fn required<T>(value: Option<T>) -> Result<T> {
    value.ok_or(Error::Invalid)
}
pub fn pump() {
    let mut msg = MSG::default(); /* SAFETY: own helper thread message pump; initialized MSG, no foreign pointer retained. */
    unsafe {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
pub fn clock_100ns() -> Result<u64> {
    vw_capture::windows::clock_ns()
        .map(|ns| ns.div_ceil(100))
        .map_err(|_| Error::Unavailable)
}
fn remote_rect(r: guard::Rect) -> Rect {
    Rect {
        x: r.left,
        y: r.top,
        width: (i64::from(r.right) - i64::from(r.left)) as u32,
        height: (i64::from(r.bottom) - i64::from(r.top)) as u32,
    }
}
pub fn query_target(window: u64, owner_pid: u32, token: String) -> Result<Target> {
    vw_remote::id(&token)?;
    let selected =
        vw_capture::windows::fixed_target(window, owner_pid).map_err(|_| Error::TargetChanged)?;
    let native = native_guard::snapshot(window as usize, selected.process_id)
        .map_err(|_| Error::TargetChanged)?;
    let value = Target {
        token,
        window,
        process_id: native.pid,
        thread_id: native.thread,
        process_created: native.process_created,
        window_rect: remote_rect(native.window),
        frame_rect: Rect {
            x: selected.frame.x,
            y: selected.frame.y,
            width: selected.frame.width,
            height: selected.frame.height,
        },
        client_rect: remote_rect(native.client),
        dpi: native.dpi,
        integrity: native.integrity,
    };
    value.validate()?;
    if native.process_created != selected.process_created
        || value.client_rect
            != (Rect {
                x: selected.client.x,
                y: selected.client.y,
                width: selected.client.width,
                height: selected.client.height,
            })
        || value.dpi != selected.dpi
    {
        return Err(Error::TargetChanged);
    }
    Ok(value)
}
pub fn unchanged(target: &Target, owner_pid: u32) -> Result<()> {
    if query_target(target.window, owner_pid, target.token.clone())? != *target {
        return Err(Error::TargetChanged);
    }
    Ok(())
}
/// Called only for the owner's local PC Grant action. Windows may refuse focus;
/// that is pending focus, and never permission to arm or inject in the background.
pub fn focus_for_owner_grant(target: &Target, owner_pid: u32) -> Result<()> {
    unchanged(target, owner_pid)?;
    // SAFETY: exact revalidated owner-selected HWND. No attach-thread, synthetic
    // key, privilege escalation or focus-lock bypass is attempted.
    if !unsafe { SetForegroundWindow(HWND(target.window as usize as *mut std::ffi::c_void)) }
        .as_bool()
    {
        return Err(Error::Ungranted);
    }
    unchanged(target, owner_pid)?;
    // SAFETY: query-only confirmation; canvas proof is still independently required.
    if unsafe { GetForegroundWindow() }.0 as usize as u64 != target.window {
        return Err(Error::Ungranted);
    }
    Ok(())
}
pub struct Confinement(HANDLE);
impl Drop for Confinement {
    fn drop(&mut self) {
        /* SAFETY: uniquely owned Job handle. */
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
pub fn confine() -> Result<Confinement> {
    // SAFETY: the helper's own unnamed Job; maximum one process prevents plugin descendants.
    unsafe {
        let job = Confinement(api("helper Job", CreateJobObjectW(None, None))?);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        limits.BasicLimitInformation.ActiveProcessLimit = 1;
        limits.ProcessMemoryLimit = 768 * 1024 * 1024;
        api(
            "helper limits",
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            ),
        )?;
        api(
            "helper confinement",
            AssignProcessToJobObject(job.0, GetCurrentProcess()),
        )?;
        Ok(job)
    }
}
/// HIL-owned process initialization; no global settings change.
pub(crate) fn initialize_hil_dpi() -> Result<()> {
    native_guard::initialize_dpi().map_err(|_| Error::Unavailable)
}
pub use input::run as run_input;
pub use video::{Gpu, run as run_video};
