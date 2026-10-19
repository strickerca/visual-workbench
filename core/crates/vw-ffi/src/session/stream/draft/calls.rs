//! Retain admission until actual worker/plan construction settles, not until its
//! awaiter disappears. Close seals before waiting and never blocks a UI thread.
use crate::workflow::{WorkflowError, WorkflowResult};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;
#[derive(Default)]
struct State {
    active: usize,
    closed: bool,
}
#[derive(Default)]
pub(super) struct Calls {
    state: Mutex<State>,
    changed: Notify,
}
pub(super) struct Call {
    owner: Arc<Calls>,
}
impl Calls {
    pub fn enter(self: &Arc<Self>) -> WorkflowResult<Arc<Call>> {
        let mut state = self.state.lock().map_err(|_| WorkflowError::Closed)?;
        if state.closed {
            return Err(WorkflowError::Closed);
        }
        if state.active >= 4 {
            return Err(WorkflowError::Backpressure);
        }
        state.active += 1;
        Ok(Arc::new(Call {
            owner: self.clone(),
        }))
    }
    pub fn seal(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
        }
        self.changed.notify_waiters();
    }
    pub async fn join(&self) -> WorkflowResult<()> {
        self.seal();
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.state.lock().map_err(|_| WorkflowError::Closed)?.active == 0 {
                return Ok(());
            }
            changed.await;
        }
    }
}
impl Drop for Call {
    fn drop(&mut self) {
        if let Ok(mut state) = self.owner.state.lock() {
            state.active -= 1;
        }
        self.owner.changed.notify_waiters();
    }
}
