//! Private typed boundary between the FFI/driver owner and its blocking worker.
use std::sync::Arc;
use vw_capture::{
    FrameIdentity, Result,
    stream::{OfferFeedback, StreamRequest},
    stream_wire::{Command, Packet},
};
use vw_stream::{Cancellation, SessionBudget};

pub(in crate::session) type CurrentCheck = Arc<dyn Fn() -> Result<()> + Send + Sync>;

pub(in crate::session) enum Work {
    Poll {
        feedback: Option<OfferFeedback>,
        repair: bool,
    },
    Original {
        identity: FrameIdentity,
    },
    InspectSource,
}

/// close succeeds only after the exact process/Job/pipe work has retired.
pub(in crate::session) trait Provider: Send + 'static {
    fn exchange(&mut self, command: Command, cancel: &Cancellation) -> Result<Packet>;
    fn close(&mut self) -> Result<()>;
}

pub(in crate::session) type OpenProvider = Box<
    dyn FnOnce(&StreamRequest, &SessionBudget, &Cancellation) -> Result<Box<dyn Provider>> + Send,
>;
