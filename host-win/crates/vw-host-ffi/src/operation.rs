use crate::{HostError, HostResult};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HostOperationStatus {
    Ready,
    Queued,
    Validating,
    WaitingForClipboard,
    Publishing,
    Succeeded,
    Failed,
    Cancelled,
}
impl HostOperationStatus {
    const fn value(self) -> u8 {
        self as u8
    }
    fn from_value(value: u8) -> Self {
        match value {
            0 => Self::Ready,
            1 => Self::Queued,
            2 => Self::Validating,
            3 => Self::WaitingForClipboard,
            4 => Self::Publishing,
            5 => Self::Succeeded,
            6 => Self::Failed,
            _ => Self::Cancelled,
        }
    }
}

/// One token per clipboard request. Cancelling before the atomic Publishing
/// transition guarantees that this request does not replace clipboard contents.
#[derive(Debug, uniffi::Object)]
pub struct HostOperation {
    state: AtomicU8,
}

#[uniffi::export]
impl HostOperation {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: AtomicU8::new(HostOperationStatus::Ready.value()),
        })
    }

    pub fn status(&self) -> HostOperationStatus {
        HostOperationStatus::from_value(self.state.load(Ordering::Acquire))
    }

    /// False means publication has already begun or the request has completed.
    /// No attempt is made to undo a clipboard publication after its commit point.
    pub fn cancel(&self) -> bool {
        let mut state = self.state.load(Ordering::Acquire);
        loop {
            if state >= HostOperationStatus::Publishing.value() {
                return state == HostOperationStatus::Cancelled.value();
            }
            match self.state.compare_exchange_weak(
                state,
                HostOperationStatus::Cancelled.value(),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(next) => state = next,
            }
        }
    }
}
impl HostOperation {
    pub(crate) fn begin(&self) -> HostResult<()> {
        self.state
            .compare_exchange(
                HostOperationStatus::Ready.value(),
                HostOperationStatus::Queued.value(),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map(|_| ())
            .map_err(|value| {
                if value == HostOperationStatus::Cancelled.value() {
                    HostError::Cancelled
                } else {
                    HostError::AlreadyUsed
                }
            })
    }
    pub(crate) fn advance(&self, next: HostOperationStatus) -> HostResult<()> {
        let mut state = self.state.load(Ordering::Acquire);
        loop {
            if state == HostOperationStatus::Cancelled.value() {
                return Err(HostError::Cancelled);
            }
            if state >= next.value() || next.value() > HostOperationStatus::Publishing.value() {
                return Err(HostError::AlreadyUsed);
            }
            match self.state.compare_exchange_weak(
                state,
                next.value(),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(value) => state = value,
            }
        }
    }
    pub(crate) fn finish(&self, succeeded: bool) {
        let mut state = self.state.load(Ordering::Acquire);
        loop {
            if state >= HostOperationStatus::Succeeded.value() {
                return;
            }
            let next = if succeeded {
                HostOperationStatus::Succeeded
            } else {
                HostOperationStatus::Failed
            };
            match self.state.compare_exchange_weak(
                state,
                next.value(),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(value) => state = value,
            }
        }
    }
}

pub(crate) struct CancelOnDrop(pub Arc<HostOperation>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
