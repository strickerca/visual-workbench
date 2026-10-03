//! Native handles never leave this worker-thread module. All clipboard data is
//! allocated and validated before replacement; successful transfer relinquishes
//! the HGLOBAL exactly once to Windows.
use crate::{
    ClipboardImageFormat, ClipboardReadFormat, DpiInfo, HostError, HostResult, PhysicalRect,
    WindowDpiInfo,
    worker::{Backend, ClipboardSnapshot, RequestContext},
};
use std::{
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{GetLastError, GlobalFree, HGLOBAL, HWND, POINT, RECT, SetLastError},
    Graphics::Gdi::{
        ClientToScreen, GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
        MonitorFromPoint, MonitorFromWindow,
    },
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
            IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
        },
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
        Threading::GetCurrentProcess,
    },
    UI::{
        HiDpi::{
            AreDpiAwarenessContextsEqual, DPI_AWARENESS_CONTEXT,
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiAwarenessContextForProcess,
            GetDpiForWindow, GetThreadDpiAwarenessContext, GetWindowDpiAwarenessContext,
            SetProcessDpiAwarenessContext, SetThreadDpiAwarenessContext,
        },
        WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, DispatchMessageW, GetClientRect, GetWindowRect,
            IsWindow, MSG, PM_REMOVE, PeekMessageW, TranslateMessage, WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW, WS_POPUP,
        },
    },
};

const STATIC_CLASS: [u16; 7] = [83, 84, 65, 84, 73, 67, 0];
const EMPTY_TITLE: [u16; 1] = [0];
const PNG_FORMAT: [u16; 4] = [80, 78, 71, 0];
const CLIPBOARD_WAIT: Duration = Duration::from_millis(500);
const CF_DIB: u32 = 8;
const CF_DIBV5: u32 = 17;

pub(crate) fn set_process_per_monitor_v2() -> HostResult<crate::ProcessDpiInfo> {
    fn current_is_per_monitor_v2() -> bool {
        // SAFETY: Queries our own process through its documented pseudo-handle;
        // no external handle is borrowed, closed or dereferenced.
        let context = unsafe { GetDpiAwarenessContextForProcess(GetCurrentProcess()) };
        if context.is_null() {
            return false;
        }
        // SAFETY: Context values come from Windows and its documented constant.
        unsafe {
            AreDpiAwarenessContextsEqual(context, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) != 0
        }
    }
    if current_is_per_monitor_v2() {
        return Ok(crate::ProcessDpiInfo {
            per_monitor_v2: true,
            changed: false,
        });
    }
    // SAFETY: Explicit caller-requested startup configuration of this process,
    // before creating UI windows. Windows refuses an incompatible locked mode.
    let changed =
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } != 0;
    if !current_is_per_monitor_v2() {
        return Err(HostError::DpiUnavailable);
    }
    Ok(crate::ProcessDpiInfo {
        per_monitor_v2: true,
        changed,
    })
}

struct DpiContext(DPI_AWARENESS_CONTEXT);
impl DpiContext {
    fn enter() -> HostResult<Self> {
        // SAFETY: A documented pseudo-handle changes only this dedicated thread.
        // The previous context is retained and restored after its windows close.
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if previous.is_null() {
            return Err(HostError::DpiUnavailable);
        }
        let guard = Self(previous);
        if !is_per_monitor_v2() {
            return Err(HostError::DpiUnavailable);
        }
        Ok(guard)
    }
}
impl Drop for DpiContext {
    fn drop(&mut self) {
        // SAFETY: The guard is constructed and dropped on the same worker. The
        // non-null prior context came directly from the corresponding API.
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}
fn is_per_monitor_v2() -> bool {
    // SAFETY: Queries the current thread and compares documented context values.
    unsafe {
        AreDpiAwarenessContextsEqual(
            GetThreadDpiAwarenessContext(),
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        ) != 0
    }
}

struct Window(HWND);
impl Window {
    fn hidden_at(x: i32, y: i32) -> HostResult<Self> {
        // SAFETY: Built-in STATIC class and title are NUL-terminated static UTF16.
        // No custom pointer, parent, menu or callback is supplied. No WS_VISIBLE
        // is set; NOACTIVATE prevents these owned helper windows stealing focus.
        let window = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                STATIC_CLASS.as_ptr(),
                EMPTY_TITLE.as_ptr(),
                WS_POPUP,
                x,
                y,
                1,
                1,
                null_mut(),
                null_mut(),
                null_mut(),
                null(),
            )
        };
        if window.is_null() {
            return Err(HostError::WorkerUnavailable);
        }
        Ok(Self(window))
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: The handle was created by this same worker and is owned solely
        // by this guard. No Rust data is exposed through its built-in window proc.
        unsafe {
            DestroyWindow(self.0);
        }
    }
}

pub(crate) struct Native {
    // Field order ensures the clipboard owner is destroyed before DPI restoration.
    owner: Window,
    _dpi: DpiContext,
}
impl Native {
    pub(crate) fn new() -> HostResult<Self> {
        let dpi = DpiContext::enter()?;
        let owner = Window::hidden_at(0, 0)?;
        Ok(Self { owner, _dpi: dpi })
    }

    fn monitor_dpi(&self, x: i32, y: i32) -> HostResult<DpiInfo> {
        // SAFETY: POINT is a value and the documented fallback selects the
        // nearest live monitor even when a requested point is outside all screens.
        let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
        self.monitor_dpi_for(monitor)
    }

    fn monitor_dpi_for(&self, monitor: HMONITOR) -> HostResult<DpiInfo> {
        if !is_per_monitor_v2() {
            return Err(HostError::DpiUnavailable);
        }
        if monitor.is_null() {
            return Err(HostError::DpiUnavailable);
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: Valid monitor handle and writable full-sized MONITORINFO.
        if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
            return Err(HostError::DpiUnavailable);
        }
        let bounds = physical_rect(info.rcMonitor)?;
        if bounds.right <= bounds.left || bounds.bottom <= bounds.top {
            return Err(HostError::DpiUnavailable);
        }
        let probe = Window::hidden_at(bounds.left, bounds.top)?;
        // SAFETY: Probe is a live, owned PMv2 window located on the selected
        // monitor. Unlike a foreign DPI-unaware HWND, it reports monitor DPI.
        let dpi = unsafe { GetDpiForWindow(probe.0) };
        if dpi == 0 {
            return Err(HostError::DpiUnavailable);
        }
        Ok(DpiInfo {
            monitor_bounds: bounds,
            work_area: physical_rect(info.rcWork)?,
            dpi,
            scale_factor: f64::from(dpi) / 96.0,
            per_monitor_v2: true,
        })
    }

    fn open_clipboard(&mut self, context: &RequestContext) -> HostResult<ClipboardOpen> {
        let deadline = Instant::now() + CLIPBOARD_WAIT;
        loop {
            context.checkpoint()?;
            // SAFETY: Non-null live owner window was created on this worker.
            // Passing its handle also makes EmptyClipboard/SetClipboardData valid.
            if unsafe { OpenClipboard(self.owner.0) } != 0 {
                return Ok(ClipboardOpen { open: true });
            }
            if Instant::now() >= deadline {
                return Err(HostError::ClipboardUnavailable);
            }
            self.pump_messages();
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Backend for Native {
    fn dpi_at_point(&mut self, x: i32, y: i32) -> HostResult<DpiInfo> {
        self.monitor_dpi(x, y)
    }

    fn window_dpi_info(&mut self, window: u64) -> HostResult<WindowDpiInfo> {
        let value = usize::try_from(window).map_err(|_| HostError::InvalidWindow)?;
        if value == 0 {
            return Err(HostError::InvalidWindow);
        }
        let window = value as HWND;
        // SAFETY: This is an opaque Windows handle, never dereferenced by Rust.
        // IsWindow and every subsequent API validate it; windows may close between calls.
        if unsafe { IsWindow(window) } == 0 {
            return Err(HostError::InvalidWindow);
        }
        let mut bounds = RECT::default();
        let mut client = RECT::default();
        // SAFETY: Non-dereferenced HWND and writable RECT values. PMv2 on this
        // worker keeps GetWindowRect and client-to-screen mapping in physical pixels.
        if unsafe { GetWindowRect(window, &mut bounds) } == 0
            || unsafe { GetClientRect(window, &mut client) } == 0
        {
            return Err(HostError::InvalidWindow);
        }
        let mut top_left = POINT {
            x: client.left,
            y: client.top,
        };
        let mut bottom_right = POINT {
            x: client.right,
            y: client.bottom,
        };
        // SAFETY: Both points are writable client coordinates for this live HWND.
        if unsafe { ClientToScreen(window, &mut top_left) } == 0
            || unsafe { ClientToScreen(window, &mut bottom_right) } == 0
        {
            return Err(HostError::InvalidWindow);
        }
        // SAFETY: Read-only HWND queries. A concurrently destroyed window returns
        // zero DPI/null context and is rejected below.
        let dpi = unsafe { GetDpiForWindow(window) };
        let context = unsafe { GetWindowDpiAwarenessContext(window) };
        if dpi == 0 || context.is_null() {
            return Err(HostError::InvalidWindow);
        }
        // SAFETY: The system selects the monitor containing the largest part of
        // this HWND. A center-point heuristic is wrong for some spanning windows.
        let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
        // SAFETY: Both contexts came from Windows or its documented constant.
        let per_monitor_v2 = unsafe {
            AreDpiAwarenessContextsEqual(context, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) != 0
        };
        Ok(WindowDpiInfo {
            window_bounds: physical_rect(bounds)?,
            client_bounds: physical_rect(RECT {
                left: top_left.x,
                top: top_left.y,
                right: bottom_right.x,
                bottom: bottom_right.y,
            })?,
            monitor: self.monitor_dpi_for(monitor)?,
            window_dpi: dpi,
            window_per_monitor_v2: per_monitor_v2,
        })
    }

    fn publish_png(&mut self, bytes: &[u8], context: &RequestContext) -> HostResult<u32> {
        publish_parts(self, bytes, None, context)
    }

    fn publish_image(
        &mut self,
        png: &[u8],
        dib: &[u8],
        context: &RequestContext,
    ) -> HostResult<u32> {
        publish_parts(self, png, Some(dib), context)
    }

    fn read_image(
        &mut self,
        requested: ClipboardReadFormat,
        context: &RequestContext,
    ) -> HostResult<ClipboardSnapshot> {
        context.checkpoint()?;
        // SAFETY: Static NUL-terminated registered name; registration is not a
        // clipboard read or mutation and does not enumerate any unrelated format.
        let png = unsafe { RegisterClipboardFormatW(PNG_FORMAT.as_ptr()) };
        if png == 0 {
            return Err(HostError::ClipboardUnavailable);
        }
        let clipboard = self.open_clipboard(context)?;
        let candidates = match requested {
            ClipboardReadFormat::PreferPng => vec![
                (png, ClipboardImageFormat::Png),
                (CF_DIBV5, ClipboardImageFormat::DibV5),
                (CF_DIB, ClipboardImageFormat::Dib),
            ],
            ClipboardReadFormat::Png => vec![(png, ClipboardImageFormat::Png)],
            ClipboardReadFormat::DibV5 => vec![(CF_DIBV5, ClipboardImageFormat::DibV5)],
            ClipboardReadFormat::Dib => vec![(CF_DIB, ClipboardImageFormat::Dib)],
        };
        let mut selected = None;
        for (id, format) in candidates {
            context.checkpoint()?;
            // SAFETY: Queries only the requested numeric image format while the
            // clipboard is open on this thread. No text or file-list access.
            if unsafe { IsClipboardFormatAvailable(id) } != 0 {
                selected = Some((id, format));
                break;
            }
        }
        let (id, format) = selected.ok_or(HostError::ClipboardImageUnavailable)?;
        // SAFETY: Clipboard is held open. The borrowed memory is copied before
        // closing and is never freed or retained as a native handle by this app.
        let handle = unsafe { GetClipboardData(id) };
        if handle.is_null() {
            return Err(HostError::ClipboardUnavailable);
        }
        let cap = if format == ClipboardImageFormat::Png {
            crate::worker::MAX_PNG_BYTES
        } else {
            crate::dib::MAX_DIB_BYTES
        };
        let bytes = copy_borrowed_global(handle, cap)?;
        context.checkpoint()?;
        // SAFETY: Same open clipboard snapshot as the copied image.
        let sequence = unsafe { GetClipboardSequenceNumber() };
        clipboard
            .close()
            .map_err(|_| HostError::ClipboardUnavailable)?;
        Ok(ClipboardSnapshot {
            bytes,
            format,
            sequence,
        })
    }

    fn pump_messages(&mut self) {
        // Bounded draining prevents hostile posted messages monopolizing the worker.
        for _ in 0..64 {
            let mut message = MSG::default();
            // SAFETY: MSG is writable, null HWND means this thread's own queue.
            if unsafe { PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) } == 0 {
                break;
            }
            // SAFETY: Message was produced by PeekMessageW for our worker queue;
            // helper windows use the system STATIC class, with no Rust callbacks.
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

fn publish_parts(
    native: &mut Native,
    png: &[u8],
    dib: Option<&[u8]>,
    context: &RequestContext,
) -> HostResult<u32> {
    context.checkpoint()?;
    // SAFETY: Static NUL-terminated UTF16 registered format name.
    let format = unsafe { RegisterClipboardFormatW(PNG_FORMAT.as_ptr()) };
    if format == 0 {
        return Err(HostError::ClipboardUnavailable);
    }
    let mut png_memory = GlobalBlock::copy_from(png)?;
    let mut dib_memory = dib.map(GlobalBlock::copy_from).transpose()?;
    let clipboard = native.open_clipboard(context)?;
    // Every codec, binding check and allocation finishes before this commit point.
    context.begin_publication()?;
    // SAFETY: This worker's owned HWND holds the open clipboard.
    if unsafe { EmptyClipboard() } == 0 {
        return Err(HostError::ClipboardPublicationFailed);
    }
    // SAFETY: A complete unlocked GMEM_MOVEABLE block. Relinquish only after a
    // successful transfer; later failures preserve that OS ownership exactly once.
    if unsafe { SetClipboardData(format, png_memory.handle) }.is_null() {
        return Err(HostError::ClipboardPublicationFailed);
    }
    png_memory.relinquish();
    if let Some(memory) = dib_memory.as_mut() {
        // SAFETY: This independent preallocated DIBV5 block uses the same owner.
        if unsafe { SetClipboardData(CF_DIBV5, memory.handle) }.is_null() {
            return Err(HostError::ClipboardPublicationFailed);
        }
        memory.relinquish();
    }
    // SAFETY: Snapshot the sequence before releasing this publication's lock.
    let sequence = unsafe { GetClipboardSequenceNumber() };
    clipboard.close()?;
    Ok(sequence)
}

fn copy_borrowed_global(handle: HGLOBAL, limit: usize) -> HostResult<Vec<u8>> {
    // SAFETY: GetClipboardData returned this non-null memory-format handle and
    // the calling worker retains the open clipboard until this copy returns.
    let size = unsafe { GlobalSize(handle) };
    if size == 0 {
        return Err(HostError::ClipboardImageUnavailable);
    }
    if size > limit {
        return Err(HostError::SizeLimit);
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| HostError::SizeLimit)?;
    // SAFETY: Live borrowed memory; paired with unlock on all exits below.
    let pointer = unsafe { GlobalLock(handle) };
    if pointer.is_null() {
        return Err(HostError::ClipboardUnavailable);
    }
    // SAFETY: GlobalSize bounds this readable allocation while locked; the Vec
    // owns separate reserved storage. No pointer or borrowed slice escapes.
    bytes.extend_from_slice(unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), size) });
    // SAFETY: Clear last-error; a zero final lock count is normal. Other readers
    // may retain locks, so a nonzero return is also successful for borrowed data.
    unsafe {
        SetLastError(0);
    }
    // SAFETY: Balances this function's successful GlobalLock on borrowed data.
    let result = unsafe { GlobalUnlock(handle) };
    // SAFETY: Reads only this thread's last-error state after GlobalUnlock.
    let error = unsafe { GetLastError() };
    if result == 0 && error != 0 {
        return Err(HostError::ClipboardUnavailable);
    }
    Ok(bytes)
}

fn physical_rect(rect: RECT) -> HostResult<PhysicalRect> {
    if rect.right < rect.left || rect.bottom < rect.top {
        return Err(HostError::DpiUnavailable);
    }
    Ok(PhysicalRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    })
}

struct ClipboardOpen {
    open: bool,
}
impl ClipboardOpen {
    fn close(mut self) -> HostResult<()> {
        // SAFETY: This guard is created only following successful OpenClipboard
        // on the same worker. Failed close is retried once by Drop.
        if unsafe { CloseClipboard() } == 0 {
            return Err(HostError::ClipboardPublicationFailed);
        }
        self.open = false;
        Ok(())
    }
}
impl Drop for ClipboardOpen {
    fn drop(&mut self) {
        if self.open {
            // SAFETY: Same ownership contract as close; handles every early return.
            unsafe {
                CloseClipboard();
            }
        }
    }
}

struct GlobalBlock {
    handle: HGLOBAL,
    locked: bool,
}
impl GlobalBlock {
    fn copy_from(bytes: &[u8]) -> HostResult<Self> {
        if bytes.is_empty() {
            return Err(HostError::InvalidPng);
        }
        // SAFETY: Bounded positive allocation size; GMEM_MOVEABLE is required for
        // clipboard ownership transfer. Failure returns null and is checked.
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) };
        if handle.is_null() {
            return Err(HostError::ClipboardUnavailable);
        }
        let mut memory = Self {
            handle,
            locked: false,
        };
        // SAFETY: Live solely owned HGLOBAL; successful locking yields at least
        // the allocated number of writable bytes and is paired with GlobalUnlock.
        let pointer = unsafe { GlobalLock(handle) };
        if pointer.is_null() {
            return Err(HostError::ClipboardUnavailable);
        }
        memory.locked = true;
        // SAFETY: The destination allocation is disjoint from the input slice and
        // has its exact length. No native pointer escapes this synchronous copy.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
        }
        // SAFETY: Clear last-error to distinguish a successful final unlock's
        // zero return from an API failure. This HGLOBAL was locked once above.
        unsafe {
            SetLastError(0);
        }
        let still_locked = unsafe { GlobalUnlock(handle) } != 0;
        let error = unsafe { GetLastError() };
        if still_locked || error != 0 {
            return Err(HostError::ClipboardUnavailable);
        }
        memory.locked = false;
        Ok(memory)
    }
    fn relinquish(&mut self) {
        self.handle = null_mut();
    }
}
impl Drop for GlobalBlock {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: A non-null handle remains Rust-owned because clipboard
            // transfer did not succeed. Balance an early-return lock, then free.
            unsafe {
                if self.locked {
                    GlobalUnlock(self.handle);
                }
                GlobalFree(self.handle);
            }
        }
    }
}
