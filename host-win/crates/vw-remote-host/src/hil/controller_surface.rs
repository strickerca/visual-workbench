//! Paint-only opt-in owned controller fixture. No input or grant authority.
use crate::{Error, Result};
use windows::Win32::{
    Foundation::{COLORREF, HWND, RECT},
    Graphics::Gdi::*,
    UI::WindowsAndMessaging::*,
};
pub(super) const TIMER_ID: usize = 2;
pub(super) struct ControllerSurface {
    generation: u32,
}
impl ControllerSurface {
    pub(super) fn new() -> Self {
        Self { generation: 0 }
    }
    fn advance(&mut self) -> Result<()> {
        if self.generation >= 3000 {
            return Err(Error::Limit);
        }
        self.generation += 1;
        Ok(())
    }
    pub(super) fn tick(&mut self, hwnd: HWND) -> Result<()> {
        self.advance()?;
        // SAFETY: paint invalidation only on our own UI-thread window; no input is posted.
        if !unsafe { InvalidateRect(Some(hwnd), None, false) }.as_bool() {
            return Err(Error::Unavailable);
        }
        Ok(())
    }
    pub(super) fn generation(&self) -> u32 {
        self.generation
    }
    // Copy generation before BeginPaint: GDI can synchronously send another
    // window message, so no SurfaceState/fixture reference is held across it.
    pub(super) fn paint(hwnd: HWND, generation: u32) -> Result<()> {
        let mut state = PAINTSTRUCT::default();
        // SAFETY: initialized bounded PAINTSTRUCT for the owning UI thread/window.
        let dc = unsafe { BeginPaint(hwnd, &mut state) };
        if dc.0.is_null() {
            return Err(Error::Unavailable);
        }
        let paint = Paint { hwnd, state };
        let mut rect = RECT::default();
        // SAFETY: initialized client rect; borrowed stock brush/DC live through Paint drop.
        unsafe {
            GetClientRect(hwnd, &mut rect).map_err(|_| Error::Unavailable)?;
            let brush = GetStockObject(DC_BRUSH);
            if brush.0.is_null() {
                return Err(Error::Unavailable);
            }
            let value = 32 + generation % 192;
            if SetDCBrushColor(dc, COLORREF(value | (64 << 8) | (96 << 16))).0 == u32::MAX {
                return Err(Error::Unavailable);
            }
            if FillRect(dc, &rect, HBRUSH(brush.0)) == 0 {
                return Err(Error::Unavailable);
            }
        }
        drop(paint);
        Ok(())
    }
}
struct Paint {
    hwnd: HWND,
    state: PAINTSTRUCT,
}
impl Drop for Paint {
    fn drop(&mut self) {
        // SAFETY: exactly the successful BeginPaint owned by this UI-thread guard.
        let _ = unsafe { EndPaint(self.hwnd, &self.state) };
    }
}
/// Actual receiver-side host QPC. Separate from POINTER_INFO.PerformanceCount,
/// which remains independently reported and may be unavailable (zero).
pub(super) fn received_clock() -> Result<(u64, u64)> {
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
    let mut counter = 0i64;
    let mut frequency = 0i64;
    // SAFETY: initialized local output buffers, read-only monotonic clock calls.
    unsafe {
        QueryPerformanceCounter(&mut counter).map_err(|_| Error::Unavailable)?;
        QueryPerformanceFrequency(&mut frequency).map_err(|_| Error::Unavailable)?;
    }
    if counter <= 0 || frequency <= 0 {
        return Err(Error::Unavailable);
    }
    Ok((counter as u64, frequency as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn animation_is_bounded_and_does_not_wrap_or_renew_the_surface_deadline() {
        let mut fixture = ControllerSurface::new();
        for _ in 0..3000 {
            assert!(fixture.advance().is_ok());
        }
        assert!(matches!(fixture.advance(), Err(Error::Limit)));
        assert_eq!(fixture.generation, 3000);
    }
}
