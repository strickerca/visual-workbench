//! Private root-owned actor boundary; no public runtime or approval constructor.
use super::{
    AndroidTunnelElement, AndroidTunnelError as Error, AndroidTunnelHierarchy,
    AndroidTunnelResult as Result, AndroidTunnelState, AndroidTunnelTarget,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use vw_stream::SessionBudget;

pub(super) struct Stop {
    pub(super) flag: AtomicBool,
    signal: Arc<dyn Fn() + Send + Sync>,
}
impl Stop {
    pub(super) fn new(signal: Arc<dyn Fn() + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            flag: AtomicBool::new(false),
            signal,
        })
    }
    pub(super) fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
        (self.signal)();
    }
    pub(super) fn check(&self) -> Result<()> {
        if self.flag.load(Ordering::Acquire) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}
pub(super) struct Captured {
    pub(super) png: Vec<u8>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) image_started_ms: u64,
    pub(super) image_finished_ms: u64,
    pub(super) image_finished_timestamp_ns: u64,
    pub(super) hierarchy: AndroidTunnelHierarchy,
    pub(super) hierarchy_age_ms: Option<u64>,
    pub(super) hierarchy_started_ms: Option<u64>,
    pub(super) hierarchy_finished_ms: Option<u64>,
    pub(super) elements: Vec<AndroidTunnelElement>,
}
pub(super) trait Provider: Send + 'static {
    fn state(&mut self) -> Result<AndroidTunnelState>;
    fn begin_confirmation(&mut self) -> Result<AndroidTunnelTarget>;
    fn confirm(&mut self, target: AndroidTunnelTarget) -> Result<AndroidTunnelState>;
    fn cancel_confirmation(&mut self) -> Result<()>;
    fn revoke_control(&mut self) -> Result<AndroidTunnelState>;
    fn capture(&mut self) -> Result<Captured>;
    fn check_source(&mut self, source: super::AndroidTunnelSource, revision: u64) -> Result<()>;
    fn close(&mut self) -> Result<()>;
}
pub(super) type OpenProvider = Box<dyn FnOnce(&SessionBudget) -> Result<Box<dyn Provider>> + Send>;
pub(super) enum Work {
    State,
    BeginConfirmation,
    Confirm(AndroidTunnelTarget),
    CancelConfirmation,
    RevokeControl,
    Capture,
}
pub(super) enum Response {
    State(AndroidTunnelState),
    Target(AndroidTunnelTarget),
    Captured(Captured),
    Done,
}
