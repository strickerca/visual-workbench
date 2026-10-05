//! Shared finite live owner. Private trusted producers only; no peer raw INPUT.
//! Drop cannot destroy a known down. Pending keeps the real source/target owner.
use super::{clock_100ns, native_guard};
use crate::{
    Error, Result,
    finite_state::{Button, Event, Held, Ledger},
};
use std::{mem::size_of, thread, time::Duration};
use vw_remote::Target;
pub use vw_remote::wire::{FiniteAttempt, FiniteRetirement};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE, POINT},
    System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
struct ProcessLease(HANDLE);
impl Drop for ProcessLease {
    fn drop(&mut self) {
        // SAFETY: uniquely owned query-only process handle.
        let _ = unsafe { CloseHandle(self.0) };
    }
}
pub struct FiniteInputOwner<S: 'static> {
    source: S,
    target: Target,
    owner_pid: u32,
    _process: ProcessLease,
    ledger: Ledger,
    sealed: bool,
    destination: Option<(i32, i32)>,
    key_destination: Option<KeyboardDestination>,
    retirement_error: Option<Error>,
}
fn native_target(target: &Target) -> native_guard::GuardResult<super::guard::Target> {
    let current = native_guard::snapshot(target.window as usize, target.process_id)?;
    let r = |v: super::guard::Rect| vw_remote::Rect {
        x: v.left,
        y: v.top,
        width: (i64::from(v.right) - i64::from(v.left)) as u32,
        height: (i64::from(v.bottom) - i64::from(v.top)) as u32,
    };
    if current.process_created != target.process_created
        || current.thread != target.thread_id
        || current.dpi != target.dpi
        || current.integrity != target.integrity
        || r(current.window) != target.window_rect
        || r(current.client) != target.client_rect
    {
        return Err("Finite target identity changed".into());
    }
    Ok(current)
}
fn guard(
    target: &Target,
    owner_pid: u32,
    mut point: (i32, i32),
    key_destination: Option<KeyboardDestination>,
    mouse_release: bool,
) -> Result<()> {
    super::unchanged(target, owner_pid)?;
    let current = native_target(target).map_err(|_| Error::TargetChanged)?;
    let mut guard = super::guard::Guard::new(
        current,
        1,
        native_guard::caller_integrity().map_err(|_| Error::Unavailable)?,
    )
    .map_err(|_| Error::TargetChanged)?;
    if let Some(original) = key_destination
        && keyboard_destination(target)? != original
    {
        return Err(Error::TargetChanged);
    }
    if mouse_release {
        let mut current = POINT::default();
        // SAFETY: actual cursor destination sampled after other native identity
        // queries. Never reuse an assumed requested point for button release.
        unsafe { GetCursorPos(&mut current) }.map_err(|_| Error::TargetChanged)?;
        point = (current.x, current.y);
    }
    // SAFETY: read-only root hit test on bounded physical destination.
    let hit = unsafe {
        GetAncestor(
            WindowFromPoint(POINT {
                x: point.0,
                y: point.1,
            }),
            GA_ROOT,
        )
        .0 as usize
    };
    guard
        .check(current, native_guard::foreground(), hit, 1, point)
        .map_err(|_| Error::TargetChanged)
}
fn event(input: &INPUT) -> Result<Event> {
    match input.r#type {
        INPUT_KEYBOARD => {
            // SAFETY: discriminant proves the initialized trusted producer keyboard union.
            let k = unsafe { input.Anonymous.ki };
            if k.wVk.0 == 0
                || k.wVk.0 > 254
                || k.wScan != 0
                || k.time != 0
                || k.dwExtraInfo != 0
                || k.dwFlags.0 & !(KEYEVENTF_KEYUP.0 | KEYEVENTF_EXTENDEDKEY.0) != 0
                || matches!(k.wVk, VK_LWIN | VK_RWIN | VK_MENU | VK_LMENU | VK_RMENU)
            {
                return Err(Error::Invalid);
            }
            let key = Held::Key {
                code: k.wVk.0,
                extended: k.dwFlags.contains(KEYEVENTF_EXTENDEDKEY),
            };
            Ok(if k.dwFlags.contains(KEYEVENTF_KEYUP) {
                Event::Up(key)
            } else {
                Event::Down(key)
            })
        }
        INPUT_MOUSE => {
            // SAFETY: discriminant proves the initialized trusted producer mouse union.
            let m = unsafe { input.Anonymous.mi };
            if m.time != 0 || m.dwExtraInfo != 0 {
                return Err(Error::Invalid);
            }
            let f = m.dwFlags;
            if f == (MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK) {
                if !(0..=65535).contains(&m.dx) || !(0..=65535).contains(&m.dy) || m.mouseData != 0
                {
                    return Err(Error::Invalid);
                }
                return Ok(Event::Neutral);
            }
            if m.dx != 0 || m.dy != 0 {
                return Err(Error::Invalid);
            }
            if f == MOUSEEVENTF_WHEEL {
                let d = m.mouseData as i32;
                if d == 0 || d.unsigned_abs() > 1200 || d % 120 != 0 {
                    return Err(Error::Invalid);
                };
                return Ok(Event::Neutral);
            }
            if m.mouseData != 0 {
                return Err(Error::Invalid);
            }
            if f == MOUSEEVENTF_LEFTDOWN {
                Ok(Event::Down(Held::Button(Button::Left)))
            } else if f == MOUSEEVENTF_LEFTUP {
                Ok(Event::Up(Held::Button(Button::Left)))
            } else if f == MOUSEEVENTF_RIGHTDOWN {
                Ok(Event::Down(Held::Button(Button::Right)))
            } else if f == MOUSEEVENTF_RIGHTUP {
                Ok(Event::Up(Held::Button(Button::Right)))
            } else {
                Err(Error::Invalid)
            }
        }
        _ => Err(Error::Invalid),
    }
}
fn release_input(value: Held) -> INPUT {
    match value {
        Held::Button(button) => INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: match button {
                        Button::Left => MOUSEEVENTF_LEFTUP,
                        Button::Right => MOUSEEVENTF_RIGHTUP,
                    },
                    ..Default::default()
                },
            },
        },
        Held::Key { code, extended } => INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(code),
                    dwFlags: KEYEVENTF_KEYUP
                        | if extended {
                            KEYEVENTF_EXTENDEDKEY
                        } else {
                            KEYBD_EVENT_FLAGS(0)
                        },
                    ..Default::default()
                },
            },
        },
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct KeyboardDestination {
    window: usize,
    pid: u32,
    thread: u32,
}
fn keyboard_destination(target: &Target) -> Result<KeyboardDestination> {
    let mut info = GUITHREADINFO {
        cbSize: size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    let mut pid = 0;
    // SAFETY: initialized actual selected GUI-thread query; no assumed virtual focus.
    unsafe { GetGUIThreadInfo(target.thread_id, &mut info) }.map_err(|_| Error::TargetChanged)?;
    if info.hwndFocus.0.is_null() {
        return Err(Error::Ungranted);
    }
    // SAFETY: read-only current native focus identity/root membership.
    let thread = unsafe { GetWindowThreadProcessId(info.hwndFocus, Some(&mut pid)) };
    if pid != target.process_id
        || thread != target.thread_id
        || unsafe { GetAncestor(info.hwndFocus, GA_ROOT) }
            != native_guard::hwnd(target.window as usize)
    {
        return Err(Error::TargetChanged);
    }
    Ok(KeyboardDestination {
        window: info.hwndFocus.0 as usize,
        pid,
        thread,
    })
}
fn neutral(events: &[Event]) -> Result<()> {
    let mut keys = vec![
        VK_LBUTTON.0,
        VK_RBUTTON.0,
        VK_MBUTTON.0,
        VK_XBUTTON1.0,
        VK_XBUTTON2.0,
        VK_SHIFT.0,
        VK_CONTROL.0,
        VK_MENU.0,
        VK_LWIN.0,
        VK_RWIN.0,
    ];
    for event in events {
        if let Event::Down(Held::Key { code, .. }) = event {
            keys.push(*code)
        }
    }
    for key in keys {
        // SAFETY: read-only high-bit state; never release owner-held input.
        if unsafe { GetAsyncKeyState(i32::from(key)) } as u16 & 0x8000 != 0 {
            return Err(Error::Ungranted);
        }
    }
    Ok(())
}
impl<S: 'static> FiniteInputOwner<S> {
    pub fn new(target: Target, owner_pid: u32, source: S) -> Result<Self> {
        target.validate()?;
        super::unchanged(&target, owner_pid)?;
        // SAFETY: retain exact process identity; no injection or write access requested.
        let process = ProcessLease(
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, target.process_id) }
                .map_err(|_| Error::Unavailable)?,
        );
        native_target(&target).map_err(|_| Error::TargetChanged)?;
        Ok(Self {
            source,
            target,
            owner_pid,
            _process: process,
            ledger: Ledger::default(),
            sealed: false,
            destination: None,
            key_destination: None,
            retirement_error: None,
        })
    }
    pub fn source(&self) -> &S {
        &self.source
    }
    pub fn send(
        &mut self,
        batch: &[INPUT],
        destination: (i32, i32),
        validate: impl FnOnce(&S) -> Result<()>,
    ) -> FiniteAttempt {
        let expected_count = batch.len() as u32;
        let mut attempt = FiniteAttempt {
            expected_count,
            accepted_count: 0,
            accepted_qpc_100ns: None,
            error: None,
            retirement: self.state(),
        };
        let prepared = (|| -> Result<Vec<Event>> {
            if self.sealed || self.ledger.pending() {
                return Err(Error::RetirementPending);
            }
            if batch.is_empty() || batch.len() > 16 {
                return Err(Error::Limit);
            }
            let events = batch.iter().map(event).collect::<Result<Vec<_>>>()?;
            Ledger::plan(&events)?;
            validate(&self.source)?;
            // Source/provider work is finished before neutral-state/final cheap guard.
            neutral(&events)?;
            self.key_destination = if events
                .iter()
                .any(|v| matches!(v, Event::Down(Held::Key { .. })))
            {
                Some(keyboard_destination(&self.target)?)
            } else {
                None
            };
            guard(
                &self.target,
                self.owner_pid,
                destination,
                self.key_destination,
                false,
            )?;
            Ok(events)
        })();
        let events = match prepared {
            Ok(v) => v,
            Err(error) => {
                self.sealed = true;
                attempt.error = Some(error);
                attempt.retirement = self.state();
                return attempt;
            }
        };
        self.destination = Some(destination);
        // SAFETY: validated generated balanced batch; no provider after final guard.
        // Windows inserts this array serially. Atomic guard/inject remains impossible.
        let count = unsafe { SendInput(batch, size_of::<INPUT>() as i32) };
        attempt.accepted_count = count;
        let ownership = self.ledger.accepted(&events, count);
        // Accepted-prefix ownership is committed BEFORE any fallible post-call clock.
        let clock = clock_100ns();
        if count > 0 {
            attempt.accepted_qpc_100ns = clock.as_ref().ok().copied()
        }
        let error = ownership
            .err()
            .or_else(|| (count != expected_count).then_some(Error::PartialInput))
            .or_else(|| clock.err());
        if let Some(error) = error {
            self.sealed = true;
            attempt.error = Some(error);
            attempt.retirement = self.retire()
        } else {
            attempt.retirement = self.state()
        }
        attempt
    }
    fn state(&self) -> FiniteRetirement {
        if !self.ledger.pending() {
            FiniteRetirement::Complete
        } else {
            FiniteRetirement::Pending {
                held_count: self.ledger.count(),
                uncertain: self.ledger.unknown(),
                error: self
                    .retirement_error
                    .clone()
                    .unwrap_or(Error::RetirementPending),
            }
        }
    }
    pub fn retire(&mut self) -> FiniteRetirement {
        self.sealed = true;
        if self.ledger.unknown() {
            self.retirement_error = Some(Error::Invalid);
            return self.state();
        }
        while let Some(value) = self.ledger.last() {
            let point = match value {
                Held::Button(_) => {
                    let mut point = POINT::default(); // SAFETY: initialized current cursor position; UP is guarded at its real destination.
                    if unsafe { GetCursorPos(&mut point) }.is_err() {
                        return self.state();
                    }
                    (point.x, point.y)
                }
                Held::Key { .. } => {
                    let Some(original) = self.key_destination else {
                        return self.state();
                    };
                    if keyboard_destination(&self.target).ok() != Some(original) {
                        return self.state();
                    }
                    match self.destination {
                        Some(p) => p,
                        None => return self.state(),
                    }
                }
            };
            let input = release_input(value);
            // SAFETY: exactly one UP for a down in this actual accepted-prefix ledger;
            // current exact original target/foreground/point was revalidated immediately.
            let result = self.ledger.release_one(
                value,
                || {
                    guard(
                        &self.target,
                        self.owner_pid,
                        point,
                        if matches!(value, Held::Key { .. }) {
                            self.key_destination
                        } else {
                            None
                        },
                        matches!(value, Held::Button(_)),
                    )
                },
                || unsafe { SendInput(&[input], size_of::<INPUT>() as i32) },
            );
            if let Err(error) = result {
                self.retirement_error = Some(error);
                return self.state();
            }
        }
        self.state()
    }
    /// Pending retains this actual thread/source/process. No new commands are
    /// admitted; only known-owned release attempts may occur. Parent observes a
    /// bounded Pending and must retain its Job/IO/permit, never kill as UP proof.
    pub fn wait_retired(&mut self) {
        while !matches!(self.retire(), FiniteRetirement::Complete) {
            thread::sleep(Duration::from_millis(20))
        }
    }
}
impl<S: 'static> Drop for FiniteInputOwner<S> {
    fn drop(&mut self) {
        if self.ledger.pending() {
            self.wait_retired()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_RIGHT,
                    dwFlags: flags,
                    ..Default::default()
                },
            },
        }
    }
    #[test]
    fn generated_extended_arrow_preserves_key_up_identity() -> Result<()> {
        let down = event(&key(KEYEVENTF_EXTENDEDKEY))?;
        let up = event(&key(KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP))?;
        Ledger::plan(&[down, up])?;
        assert_eq!(
            down,
            Event::Down(Held::Key {
                code: VK_RIGHT.0,
                extended: true
            })
        );
        Ok(())
    }
    #[test]
    fn unicode_scan_shell_key_or_foreign_extra_refused() {
        assert_eq!(event(&key(KEYEVENTF_UNICODE)), Err(Error::Invalid));
        let keyboard = |vk: VIRTUAL_KEY, extra: usize| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    dwExtraInfo: extra,
                    ..Default::default()
                },
            },
        };
        assert_eq!(event(&keyboard(VK_LWIN, 0)), Err(Error::Invalid));
        assert_eq!(event(&keyboard(VK_RIGHT, 1)), Err(Error::Invalid));
    }
    #[test]
    fn each_generated_release_is_only_the_exact_owned_up() -> Result<()> {
        for value in [
            Held::Button(Button::Left),
            Held::Button(Button::Right),
            Held::Key {
                code: VK_RIGHT.0,
                extended: true,
            },
        ] {
            assert_eq!(event(&release_input(value))?, Event::Up(value))
        }
        Ok(())
    }
    #[test]
    fn malformed_mouse_mixed_flags_or_i32_min_wheel_refused() {
        let mouse = |flags: MOUSE_EVENT_FLAGS, data: u32| INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: flags,
                    mouseData: data,
                    ..Default::default()
                },
            },
        };
        assert_eq!(
            event(&mouse(MOUSEEVENTF_LEFTUP | MOUSEEVENTF_RIGHTUP, 0)),
            Err(Error::Invalid)
        );
        assert_eq!(
            event(&mouse(MOUSEEVENTF_WHEEL, i32::MIN as u32)),
            Err(Error::Invalid)
        );
    }
}
