//! A foreign codec owner may outlive the JNI invocation (including failed
//! attachment/exception/cancellation). Keep its exact native staging owner until
//! the pinned Java runtime confirms actual retirement, potentially on a retry.
use super::{Error, Result, input};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

struct Pending {
    id: u64,
    _source: Arc<input::PreparedSource>,
    retired: Arc<AtomicBool>,
}
/// Read-only native receipt: dropping it cannot credit foreign retirement.
pub(super) struct Retirement(Arc<AtomicBool>);
impl Retirement {
    pub(super) fn require_confirmed(&self) -> Result<()> {
        if self.0.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(Error::Storage)
        }
    }
}
pub(super) struct ForeignSources(Mutex<Vec<Pending>>);
impl ForeignSources {
    pub(super) fn new() -> Self {
        Self(Mutex::new(Vec::with_capacity(2)))
    }
    pub(super) fn retain(&self, id: u64, source: Arc<input::PreparedSource>) -> Result<Retirement> {
        let mut entries = self.0.lock().map_err(|_| Error::Storage)?;
        if id == 0 || entries.iter().any(|entry| entry.id == id) {
            return Err(Error::Invalid);
        }
        // The same two native input permits remain held through these sources.
        // No queue growth, replacement, timeout or cancellation evicts a pin.
        if entries.len() >= 2 {
            return Err(Error::Backpressure);
        }
        let retired = Arc::new(AtomicBool::new(false));
        entries.push(Pending {
            id,
            _source: source,
            retired: retired.clone(),
        });
        Ok(Retirement(retired))
    }
    /// Only the fixed Java runtime's post-retirement callback calls this in
    /// production. It can run on the joined cleanup thread, after ACTIVE is gone.
    pub(super) fn confirm_retired(&self, id: u64) -> Result<()> {
        let pending = {
            let mut entries = self.0.lock().map_err(|_| Error::Storage)?;
            let Some(index) = entries.iter().position(|entry| entry.id == id) else {
                // Idempotent retry, without crediting any other operation.
                return Ok(());
            };
            entries.swap_remove(index)
        };
        pending.retired.store(true, Ordering::Release);
        // Drop outside the registry lock: actual native directory cleanup may
        // quarantine its own input permit/charge on failure.
        drop(pending);
        Ok(())
    }
}
