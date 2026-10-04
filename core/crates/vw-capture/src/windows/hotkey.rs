//! Explicit owner-enabled, remappable foreground capture hook. No injection.
use crate::*;
use ::windows::Win32::{
    System::Threading::GetCurrentProcessId,
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
pub struct Hotkey {
    stop: Arc<AtomicBool>,
    latest: Arc<Mutex<Option<Result<WindowTarget>>>>,
    worker: Option<JoinHandle<()>>,
}
static REGISTERED: AtomicBool = AtomicBool::new(false);
struct Slot;
impl Drop for Slot {
    fn drop(&mut self) {
        REGISTERED.store(false, Ordering::Release);
    }
}
impl Hotkey {
    pub fn start(modifiers: u32, key: u32) -> Result<Self> {
        if modifiers == 0 || modifiers & !0xf != 0 || !(0x30..=0x5a).contains(&key) {
            return Err(Error::Invalid);
        }
        REGISTERED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::Busy)?;
        let slot = Slot;
        let stop = Arc::new(AtomicBool::new(false));
        let latest = Arc::new(Mutex::new(None));
        let closed = stop.clone();
        let event = latest.clone();
        let (ready, started) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("vw-capture-hotkey".into())
            .spawn(move || {
                let _slot = slot;
                const ID: i32 = 0x5657;
                // SAFETY: registration belongs only to this dedicated thread; no HWND
                // is borrowed. The matching Unregister always runs on the same thread.
                unsafe {
                    let result =
                        RegisterHotKey(None, ID, HOT_KEY_MODIFIERS(modifiers) | MOD_NOREPEAT, key)
                            .map_err(|_| Error::Unavailable);
                    if ready.send(result).is_err() {
                        let _ = UnregisterHotKey(None, ID);
                        return;
                    }
                    if result.is_err() {
                        return;
                    }
                    while !closed.load(Ordering::Acquire) {
                        let mut message = MSG::default();
                        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                            if message.message == WM_HOTKEY && message.wParam.0 == ID as usize {
                                let target = super::foreground(GetCurrentProcessId());
                                if let Ok(mut slot) = event.lock() {
                                    *slot = Some(target);
                                }
                            }
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                    let _ = UnregisterHotKey(None, ID);
                }
            })
            .map_err(|_| Error::Platform)?;
        match started.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => Ok(Self {
                stop,
                latest,
                worker: Some(worker),
            }),
            _ => {
                stop.store(true, Ordering::Release);
                let _ = worker.join();
                Err(Error::Unavailable)
            }
        }
    }
    pub fn take(&self) -> Result<Option<WindowTarget>> {
        self.latest
            .lock()
            .map_err(|_| Error::Platform)?
            .take()
            .transpose()
    }
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Hotkey {
    fn drop(&mut self) {
        self.shutdown();
    }
}
