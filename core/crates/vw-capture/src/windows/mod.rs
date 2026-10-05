//! All foreign handles are queried, never dereferenced. Provider calls run only
//! in the owned helper; the editor uses process::run to enforce cancellation.
pub mod hotkey;
mod uia;
mod wgc;
use crate::*;
use ::windows::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::ClientToScreen},
    System::{Performance::*, Threading::*},
    UI::{HiDpi::*, WindowsAndMessaging::*},
};
pub use uia::collect;
pub use wgc::{capture, capture_canvas_fixture, capture_diagnostic, capture_fixture_diagnostic};
pub(crate) fn api<T>(value: ::windows::core::Result<T>) -> Result<T> {
    value.map_err(|_| Error::Platform)
}
pub(crate) struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        /* SAFETY: successful OpenProcess handle is uniquely owned. */
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
pub struct DpiScope(DPI_AWARENESS_CONTEXT);
impl DpiScope {
    pub fn enter() -> Result<Self> {
        /* SAFETY: changes only this helper thread and returns its prior context. */
        let old =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if old.0.is_null() {
            Err(Error::Platform)
        } else {
            Ok(Self(old))
        }
    }
}
impl Drop for DpiScope {
    fn drop(&mut self) {
        /* SAFETY: restore the exact prior thread context. */
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}
pub fn clock_ns() -> Result<u64> {
    let (mut value, mut frequency) = (0i64, 0i64); /* SAFETY: initialized output pointers are valid for the synchronous calls. */
    unsafe {
        api(QueryPerformanceCounter(&mut value))?;
        api(QueryPerformanceFrequency(&mut frequency))?;
    }
    if value < 0 || frequency <= 0 {
        return Err(Error::Platform);
    }
    u64::try_from(i128::from(value) * 1_000_000_000 / i128::from(frequency))
        .map_err(|_| Error::Limit)
}
fn rect(value: RECT) -> Result<Rect> {
    let width = i64::from(value.right) - i64::from(value.left);
    let height = i64::from(value.bottom) - i64::from(value.top);
    Rect {
        x: value.left,
        y: value.top,
        width: u32::try_from(width).map_err(|_| Error::Invalid)?,
        height: u32::try_from(height).map_err(|_| Error::Invalid)?,
    }
    .validate()
}
pub fn foreground(owner: u32) -> Result<WindowTarget> {
    let _dpi = DpiScope::enter()?; /* SAFETY: this obtains a system-owned handle; target() validates it. */
    let window = unsafe { GetForegroundWindow() };
    target(window.0 as usize as u64, owner)
}
pub fn target(value: u64, owner: u32) -> Result<WindowTarget> {
    query_target(value, owner, true)
}
/// Read-only same-HWND query for a previously owner-selected capture target.
pub fn fixed_target(value: u64, owner: u32) -> Result<WindowTarget> {
    query_target(value, owner, false)
}
fn query_target(value: u64, owner: u32, require_foreground: bool) -> Result<WindowTarget> {
    if value == 0 || owner == 0 {
        return Err(Error::Invalid);
    }
    let _dpi = DpiScope::enter()?;
    let window = HWND(value as usize as *mut _);
    // SAFETY: Win32 validates opaque HWNDs. No memory is dereferenced through one.
    unsafe {
        if !IsWindow(Some(window)).as_bool()
            || !IsWindowVisible(window).as_bool()
            || IsIconic(window).as_bool()
            || (require_foreground && GetForegroundWindow() != window)
            || GetAncestor(window, GA_ROOT) != window
        {
            return Err(Error::Stale);
        }
        let mut pid = 0;
        GetWindowThreadProcessId(window, Some(&mut pid));
        if pid == owner {
            return Err(Error::OwnWindow);
        }
        if pid == 0 {
            return Err(Error::Unavailable);
        }
        let process = Handle(api(OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            pid,
        ))?);
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        api(GetProcessTimes(
            process.0,
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        ))?;
        let mut client = RECT::default();
        api(GetClientRect(window, &mut client))?;
        let mut origin = POINT {
            x: client.left,
            y: client.top,
        };
        if !ClientToScreen(window, &mut origin).as_bool() {
            return Err(Error::Platform);
        }
        let w = client
            .right
            .checked_sub(client.left)
            .ok_or(Error::Invalid)?;
        let h = client
            .bottom
            .checked_sub(client.top)
            .ok_or(Error::Invalid)?;
        client = RECT {
            left: origin.x,
            top: origin.y,
            right: origin.x.checked_add(w).ok_or(Error::Invalid)?,
            bottom: origin.y.checked_add(h).ok_or(Error::Invalid)?,
        };
        let mut frame = RECT::default();
        api(DwmGetWindowAttribute(
            window,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut frame as *mut RECT).cast(),
            std::mem::size_of::<RECT>() as u32,
        ))?;
        let mut cloaked = 0u32;
        api(DwmGetWindowAttribute(
            window,
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            4,
        ))?;
        if cloaked != 0 {
            return Err(Error::Unavailable);
        }
        let result = WindowTarget {
            window: value,
            process_id: pid,
            process_created: (u64::from(created.dwHighDateTime) << 32)
                | u64::from(created.dwLowDateTime),
            client: rect(client)?,
            frame: rect(frame)?,
            dpi: GetDpiForWindow(window),
            observed_ns: clock_ns()?,
        };
        result.validate(owner)?;
        Ok(result)
    }
}
/// Called in the desktop process after each app HWND is created, before show.
pub fn exclude_own_windows(windows: &[u64]) -> Result<()> {
    if windows.is_empty() || windows.len() > 64 {
        return Err(Error::Limit);
    }
    // SAFETY: every HWND must belong to this process before changing affinity.
    unsafe {
        for value in windows {
            let window = HWND(*value as usize as *mut _);
            let mut pid = 0;
            GetWindowThreadProcessId(window, Some(&mut pid));
            if pid != GetCurrentProcessId() || !IsWindow(Some(window)).as_bool() {
                return Err(Error::OwnWindow);
            }
            api(SetWindowDisplayAffinity(window, WDA_EXCLUDEFROMCAPTURE))?;
            let mut affinity = WINDOW_DISPLAY_AFFINITY::default();
            api(GetWindowDisplayAffinity(window, &mut affinity.0))?;
            if affinity != WDA_EXCLUDEFROMCAPTURE {
                return Err(Error::Platform);
            }
        }
    }
    Ok(())
}
