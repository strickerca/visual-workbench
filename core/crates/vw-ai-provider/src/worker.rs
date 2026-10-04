use crate::{Error, MAX_ACTIVE_ATTEMPTS, Result};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

static ACTIVE: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static IDLE: (std::sync::Mutex<()>, std::sync::Condvar) =
    (std::sync::Mutex::new(()), std::sync::Condvar::new());
pub(crate) const POLL: Duration = Duration::from_millis(20);

pub(crate) struct Slot;
impl Slot {
    pub(crate) fn acquire() -> Result<Self> {
        ACTIVE
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_ACTIVE_ATTEMPTS).then_some(count + 1)
            })
            .map(|_| Self)
            .map_err(|_| Error::Busy)
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
        #[cfg(test)]
        if let Ok(_guard) = IDLE.0.lock() {
            IDLE.1.notify_all();
        }
    }
}

/// Test isolation waits for actual resource retirement, not a signal from the
/// job or its result destructor (both happen before the owned slot is released).
#[cfg(test)]
pub(crate) fn wait_idle(timeout: Duration) -> bool {
    let Ok(guard) = IDLE.0.lock() else {
        return false;
    };
    let Ok((_guard, result)) = IDLE
        .1
        .wait_timeout_while(guard, timeout, |_| ACTIVE.load(Ordering::Acquire) != 0)
    else {
        return false;
    };
    !result.timed_out() || ACTIVE.load(Ordering::Acquire) == 0
}
struct StopOnDrop(Arc<AtomicBool>);
impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

pub(crate) fn run<T: Send + 'static>(
    slot: Slot,
    duration: Duration,
    cancel: &dyn vw_ai::Cancellation,
    job: impl FnOnce(Arc<AtomicBool>, Instant) -> Result<T> + Send + 'static,
) -> Result<T> {
    crate::check_cancel(cancel)?;
    let deadline = Instant::now()
        .checked_add(duration)
        .ok_or(Error::Deadline)?;
    let stop = Arc::new(AtomicBool::new(false));
    let guard = StopOnDrop(Arc::clone(&stop));
    let (send, receive) = mpsc::sync_channel(1);
    // No unbounded submission queue and no detached pointer into caller memory.
    // Worker owns the slot, request, key and runtime until its actual completion.
    let handle = std::thread::Builder::new()
        .name("vw-image-provider".into())
        .spawn(move || {
            let _slot = slot;
            let result = job(stop, deadline);
            let _ = send.send(result); // late result is dropped if caller left
        })
        .map_err(|_| Error::Worker)?;
    // Never join on UI/caller cancellation: OS DNS/trust/key stores can block.
    drop(handle);
    loop {
        crate::check_cancel(cancel)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Error::Deadline);
        }
        match receive.recv_timeout(POLL.min(remaining)) {
            Ok(result) => {
                crate::check_cancel(cancel)?;
                if Instant::now() >= deadline {
                    return Err(Error::Deadline);
                }
                drop(guard);
                return result;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(Error::Worker),
        }
    }
}
