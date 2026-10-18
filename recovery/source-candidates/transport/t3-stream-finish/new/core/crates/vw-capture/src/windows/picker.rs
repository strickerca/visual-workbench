//! Picker enumeration and monitor binding run in the existing bounded helper.
use crate::{picker::*, *};
use super::{DpiScope, api};
use ::windows::{core::{BOOL, PCWSTR, w}, Win32::{Foundation::*, Graphics::Gdi::*,
    UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*}, System::Threading::GetCurrentProcessId}};

struct Window(HWND);
impl Drop for Window { fn drop(&mut self) {
    // SAFETY: this is the uniquely owned, never-shown DPI query window.
    unsafe { let _ = DestroyWindow(self.0); }
} }

pub fn monitor(value: u64) -> Result<MonitorTarget> {
    if value == 0 { return Err(Error::Invalid); }
    let _dpi = DpiScope::enter()?;
    // SAFETY: Windows validates the opaque monitor handle. All output structures
    // and bounded UTF-16 arrays remain alive for each synchronous API call.
    unsafe {
        let handle = HMONITOR(value as usize as *mut _);
        let mut info = MONITORINFOEXW::default(); info.monitorInfo.cbSize = std::mem::size_of_val(&info) as u32;
        if !GetMonitorInfoW(handle, &mut info.monitorInfo).as_bool() { return Err(Error::Unavailable); }
        let length = info.szDevice.iter().position(|&v| v == 0).ok_or(Error::Invalid)?;
        let device = String::from_utf16(&info.szDevice[..length]).map_err(|_| Error::Invalid)?;
        let physical = super::rect(info.monitorInfo.rcMonitor)?;
        let mut mode = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
        if !EnumDisplaySettingsW(PCWSTR(info.szDevice.as_ptr()), ENUM_CURRENT_SETTINGS, &mut mode).as_bool()
            || !mode.dmFields.contains(DM_DISPLAYORIENTATION) { return Err(Error::Unavailable); }
        let rotation = mode.Anonymous1.Anonymous2.dmDisplayOrientation.0;
        // GetDpiForMonitor is not PMv2-aware. A 1px, hidden PMv2 HWND binds the
        // documented GetDpiForWindow result to this monitor without activation.
        let query = Window(api(CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, w!("STATIC"),
            w!(""), WS_POPUP, physical.x, physical.y, 1, 1, None, None, None, None))?);
        if MonitorFromWindow(query.0, MONITOR_DEFAULTTONULL) != handle { return Err(Error::Stale); }
        let result = MonitorTarget { monitor: value, device, physical, dpi: GetDpiForWindow(query.0),
            rotation, primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
            observed_ns: super::clock_ns()? };
        result.validate()?; Ok(result)
    }
}

struct Enumeration<'a> { owner: u32, cancel: &'a Cancellation, catalog: PickerCatalog,
    seen: usize, failure: Option<Error> }
unsafe extern "system" fn window_callback(window: HWND, raw: LPARAM) -> BOOL {
    // SAFETY: EnumWindows invokes synchronously with this live stack context.
    let state = unsafe { &mut *(raw.0 as *mut Enumeration<'_>) };
    if let Err(error) = state.cancel.check() { state.failure = Some(error); return BOOL(0); }
    state.seen += 1;
    if state.seen > 4096 || state.catalog.windows.len() >= MAX_PICKER_WINDOWS {
        state.catalog.truncated = true; return BOOL(0);
    }
    // The helper is a distinct process; exclude both it and the actual app owner.
    let mut pid = 0;
    // SAFETY: querying an opaque HWND does not dereference application memory.
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)); }
    if pid == state.owner || pid == unsafe { GetCurrentProcessId() } { return BOOL(1); }
    if let Ok(target) = super::fixed_target(window.0 as usize as u64, state.owner) {
        let mut title = [0u16; 1025];
        // SAFETY: bounded title buffer. GetWindowText does not request UIA data.
        let length = unsafe { GetWindowTextW(window, &mut title) };
        let title = if length > 0 { String::from_utf16_lossy(&title[..length as usize]) } else { String::new() };
        state.catalog.windows.push(PickerWindow { title, target });
    }
    BOOL(1)
}
unsafe extern "system" fn monitor_callback(handle: HMONITOR, _: HDC, _: *mut RECT, raw: LPARAM) -> BOOL {
    // SAFETY: EnumDisplayMonitors invokes synchronously with this live context.
    let state = unsafe { &mut *(raw.0 as *mut Enumeration<'_>) };
    if let Err(error) = state.cancel.check() { state.failure = Some(error); return BOOL(0); }
    if state.catalog.monitors.len() >= MAX_PICKER_MONITORS { state.catalog.truncated = true; return BOOL(0); }
    match monitor(handle.0 as usize as u64) {
        Ok(value) => state.catalog.monitors.push(value),
        Err(error) => { state.failure = Some(error); return BOOL(0); }
    }
    BOOL(1)
}
pub fn enumerate(owner: u32, cancel: &Cancellation) -> Result<PickerCatalog> {
    if owner == 0 { return Err(Error::Invalid); } cancel.check()?;
    let _dpi = DpiScope::enter()?;
    let mut state = Enumeration { owner, cancel, catalog: PickerCatalog { windows: Vec::new(), monitors: Vec::new(), truncated: false }, seen: 0, failure: None };
    let pointer = LPARAM((&mut state as *mut Enumeration<'_>) as isize);
    // SAFETY: callbacks and state do not escape either bounded synchronous call.
    unsafe {
        let windows = EnumWindows(Some(window_callback), pointer);
        if windows.is_err() && !state.catalog.truncated && state.failure.is_none() { return Err(Error::Platform); }
        if let Some(error) = state.failure.take() { return Err(error); }
        let monitors = EnumDisplayMonitors(None, None, Some(monitor_callback), pointer);
        if !monitors.as_bool() && !state.catalog.truncated && state.failure.is_none() { return Err(Error::Platform); }
    }
    if let Some(error) = state.failure { return Err(error); }
    state.catalog.validate(owner)?; cancel.check()?; Ok(state.catalog)
}

struct OwnWindows { owner: u32, seen: usize, failure: Option<Error> }
unsafe extern "system" fn own_window_callback(window: HWND, raw: LPARAM) -> BOOL {
    // SAFETY: synchronous EnumWindows stack context, no borrowed memory escapes.
    let state = unsafe { &mut *(raw.0 as *mut OwnWindows) };
    state.seen += 1;
    if state.seen > 4096 { state.failure = Some(Error::Limit); return BOOL(0); }
    let mut pid = 0;
    // SAFETY: only attributes of validated owner HWNDs are queried; never changed.
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut pid));
        if pid == state.owner && IsWindowVisible(window).as_bool() {
            let mut affinity = 0;
            if GetWindowDisplayAffinity(window, &mut affinity).is_err() || affinity != WDA_EXCLUDEFROMCAPTURE.0 {
                state.failure = Some(Error::OwnWindow); return BOOL(0);
            }
        }
    }
    BOOL(1)
}
/// A monitor capture refuses if any visible owner top-level window lacks the
/// exclusion flag. The app must still set exclusion before showing every new
/// HWND: two observations cannot prove a transient dialog never appeared.
pub fn require_own_windows_excluded(owner: u32) -> Result<()> {
    if owner == 0 { return Err(Error::Invalid); }
    let mut state = OwnWindows { owner, seen: 0, failure: None };
    // SAFETY: callback only borrows this live, synchronous stack context.
    let result = unsafe { EnumWindows(Some(own_window_callback), LPARAM((&mut state as *mut OwnWindows) as isize)) };
    if let Some(error) = state.failure { return Err(error); }
    api(result)
}
