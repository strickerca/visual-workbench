//! One actual codec call per receiver, owned through cancellation and shutdown.
use super::{
    memory::{ByteBudget, OwnedFrame},
    order::DecodeWork,
};
use crate::{
    CoreError,
    session::{SessionError, SessionResult},
    worker::Worker,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::oneshot;

static DECODERS: AtomicUsize = AtomicUsize::new(0);
struct DecoderPermit;
impl Drop for DecoderPermit {
    fn drop(&mut self) {
        DECODERS.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Busy(Arc<AtomicBool>);
impl Drop for Busy {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(super) struct Decoder {
    worker: Worker<DecoderPermit>,
    budget: ByteBudget,
    busy: Arc<AtomicBool>,
    active: Option<Arc<vw_stream::Cancellation>>,
    closing: bool,
}
pub(super) struct DecodeJob {
    pub ticket: super::order::DecodeTicket,
    cancel: Arc<vw_stream::Cancellation>,
    receive: Option<oneshot::Receiver<SessionResult<Arc<OwnedFrame>>>>,
}
pub(super) struct OriginalJob {
    cancel: Arc<vw_stream::Cancellation>,
    receive: Option<oneshot::Receiver<SessionResult<super::verified::Original>>>,
}
impl OriginalJob {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    pub async fn receive(&mut self) -> SessionResult<super::verified::Original> {
        let value = self
            .receive
            .as_mut()
            .ok_or(SessionError::Closed)?
            .await
            .map_err(|_| SessionError::Worker);
        self.receive = None;
        value?
    }
}
impl Drop for OriginalJob {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl DecodeJob {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    /// Cancellation-safe across select branches: dropping this waiting future
    /// retains the receiver. The driver must keep the job until actual receipt.
    pub async fn receive(&mut self) -> SessionResult<Arc<OwnedFrame>> {
        let value = self
            .receive
            .as_mut()
            .ok_or(SessionError::Closed)?
            .await
            .map_err(|_| SessionError::Worker);
        self.receive = None;
        value?
    }
}
impl Drop for DecodeJob {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl Decoder {
    pub fn new(budget: ByteBudget) -> SessionResult<Self> {
        DECODERS
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 2).then_some(n + 1)
            })
            .map_err(|_| SessionError::Backpressure)?;
        let worker = Worker::new("vw-stream-decode", DecoderPermit, 1).map_err(map_worker)?;
        Ok(Self {
            worker,
            budget,
            busy: Arc::new(AtomicBool::new(false)),
            active: None,
            closing: false,
        })
    }
    pub fn start(
        &mut self,
        work: DecodeWork,
        previous: Option<Arc<OwnedFrame>>,
    ) -> SessionResult<DecodeJob> {
        self.start_with(work, previous, |packet, previous, budget, cancel| {
            vw_stream::decode_tiles_budgeted(packet, previous, budget, cancel).map_err(map_stream)
        })
    }
    fn start_with(
        &mut self,
        work: DecodeWork,
        previous: Option<Arc<OwnedFrame>>,
        decode: impl FnOnce(
            &vw_stream::EncodedTiles,
            Option<&vw_stream::DisplayFrame>,
            &vw_stream::SessionBudget,
            &vw_stream::Cancellation,
        ) -> SessionResult<vw_stream::DisplayFrame>
        + Send
        + 'static,
    ) -> SessionResult<DecodeJob> {
        if self.closing {
            return Err(SessionError::Closed);
        }
        if super::wire::frame_identity(&work.packet)? != work.ticket.frame
            || work.packet.captured_at_ms != work.ticket.stamp.captured_at_ms
            || super::wire::CaptureStamp::new(
                work.ticket.stamp.captured_at_ms,
                &work.ticket.stamp.platform,
            )? != work.ticket.stamp
            || previous
                .as_ref()
                .is_some_and(|frame| !frame.belongs_to(&self.budget))
        {
            return Err(SessionError::Invalid);
        }
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| SessionError::Backpressure)?;
        let busy = Busy(self.busy.clone());
        let packet = work.packet;
        // Reservation precedes packet adoption; the canonical decoder retains
        // actual output/workspace charges in the same endpoint/process pool.
        let packet = vw_stream::EncodedTiles::reserve(
            self.budget.pool(),
            &vw_stream::Cancellation::default(),
        )
        .and_then(|owner| owner.adopt(packet, &vw_stream::Cancellation::default()))
        .map_err(map_stream)?;
        let output = self.budget.reserve(0)?;
        let budget = self.budget.clone();
        let cancel = Arc::new(vw_stream::Cancellation::default());
        let signal = cancel.clone();
        let (send, receive) = oneshot::channel();
        let ticket = work.ticket;
        let stamp = ticket.stamp.clone();
        self.worker
            .enqueue(Box::new(move |_| {
                let result = (|| {
                    signal.check().map_err(map_stream)?;
                    let frame = decode(
                        &packet,
                        previous.as_ref().map(|v| v.frame.as_ref()),
                        budget.pool(),
                        &signal,
                    )?;
                    signal.check().map_err(map_stream)?;
                    OwnedFrame::new(frame, stamp, output)
                })();
                // Destroy temporary input/old-base/scratch before admitting another
                // codec call, even if the consumer abandoned this result.
                drop(packet);
                drop(previous);
                drop(busy);
                let _ = send.send(result);
            }))
            .map_err(map_worker)?;
        self.active = Some(cancel.clone());
        Ok(DecodeJob {
            ticket,
            cancel,
            receive: Some(receive),
        })
    }
    pub fn cancel_current(&self) {
        if let Some(cancel) = &self.active {
            cancel.cancel();
        }
    }
    pub fn original(
        &mut self,
        offer: super::wire::OriginalOffer,
        scratch: tempfile::NamedTempFile,
    ) -> SessionResult<OriginalJob> {
        if self.closing {
            return Err(SessionError::Closed);
        }
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| SessionError::Backpressure)?;
        let busy = Busy(self.busy.clone());
        let budget = self.budget.clone();
        let cancel = Arc::new(vw_stream::Cancellation::default());
        let signal = cancel.clone();
        let (send, receive) = oneshot::channel();
        self.worker
            .enqueue(Box::new(move |_| {
                let result = super::verified::verify(offer, scratch, budget, &signal);
                drop(busy);
                let _ = send.send(result);
            }))
            .map_err(map_worker)?;
        self.active = Some(cancel.clone());
        Ok(OriginalJob {
            cancel,
            receive: Some(receive),
        })
    }
    pub async fn shutdown(&mut self) -> SessionResult<()> {
        self.closing = true;
        self.cancel_current();
        self.worker.shutdown(|_| Ok(())).await.map_err(map_worker)
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        self.cancel_current();
    }
}

pub(super) fn map_stream(error: vw_stream::StreamError) -> SessionError {
    use vw_stream::StreamError as E;
    match error {
        E::Limit => SessionError::Backpressure,
        E::Cancelled => SessionError::Cancelled,
        E::Unavailable => SessionError::Closed,
        _ => SessionError::Invalid,
    }
}
fn map_worker(error: CoreError) -> SessionError {
    match error {
        CoreError::Backpressure => SessionError::Backpressure,
        CoreError::Closed => SessionError::Closed,
        CoreError::Cancelled => SessionError::Cancelled,
        _ => SessionError::Worker,
    }
}

#[cfg(test)]
#[path = "decoder_tests.rs"]
mod tests;
