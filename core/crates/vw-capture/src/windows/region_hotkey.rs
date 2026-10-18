//! Explicitly enabled, coalesced region-picker notification. It does not capture.
use crate::{Error, Result};
use ::windows::Win32::UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
static REGISTERED: AtomicBool = AtomicBool::new(false);
struct Slot;
impl Drop for Slot {
    fn drop(&mut self) {
        REGISTERED.store(false, Ordering::Release);
    }
}
pub struct RegionHotkey {
    stop: Arc<AtomicBool>,
    event: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl RegionHotkey {
    pub fn start(modifiers: u32, key: u32) -> Result<Self> {
        if modifiers == 0
            || modifiers & !0xf != 0
            || !((0x30..=0x39).contains(&key) || (0x41..=0x5a).contains(&key))
        {
            return Err(Error::Invalid);
        }
        REGISTERED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::Busy)?;
        let slot = Slot;
        let stop = Arc::new(AtomicBool::new(false));
        let event = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let signal = event.clone();
        let (ready, receiver) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("vw-region-picker-hotkey".into())
            .spawn(move || {
                let _slot = slot;
                const ID: i32 = 0x5658;
                // SAFETY: registration/message queue/unregistration all belong to
                // this dedicated thread; no global hook or foreign HWND is used.
                unsafe {
                    let registration =
                        RegisterHotKey(None, ID, HOT_KEY_MODIFIERS(modifiers) | MOD_NOREPEAT, key)
                            .map_err(|_| Error::Unavailable);
                    if ready.send(registration).is_err() {
                        if registration.is_ok() {
                            let _ = UnregisterHotKey(None, ID);
                        }
                        return;
                    }
                    if registration.is_err() {
                        return;
                    }
                    while !stopped.load(Ordering::Acquire) {
                        let mut message = MSG::default();
                        let mut drained = 0;
                        while drained < 128
                            && PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool()
                        {
                            if message.message == WM_HOTKEY && message.wParam.0 == ID as usize {
                                signal.store(true, Ordering::Release);
                            }
                            drained += 1;
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                    let _ = UnregisterHotKey(None, ID);
                }
            })
            .map_err(|_| Error::Platform)?;
        match receiver.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => Ok(Self {
                stop,
                event,
                thread: Some(thread),
            }),
            _ => {
                stop.store(true, Ordering::Release);
                let _ = thread.join();
                Err(Error::Unavailable)
            }
        }
    }
    pub fn take(&self) -> bool {
        !self.stop.load(Ordering::Acquire) && self.event.swap(false, Ordering::AcqRel)
    }
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.event.store(false, Ordering::Release);
    }
}
impl Drop for RegionHotkey {
    fn drop(&mut self) {
        self.shutdown();
    }
}
