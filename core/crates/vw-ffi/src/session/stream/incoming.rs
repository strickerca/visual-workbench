//! Private scoped BLOB I/O never runs on the session's network executor. One
//! actual write owns the worker admission and staging file through cancellation.
//! PNG decoding remains on the separately bounded codec worker after final hash.
use super::{
    memory::{ByteBudget, Bytes},
    transfer,
    wire::OriginalOffer,
};
use crate::{
    session::{SessionError, SessionResult},
    worker::Worker,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::oneshot;
use vw_proto::{Message, v1 as pb};

static FILE_WORKERS: AtomicUsize = AtomicUsize::new(0);
struct Permit;
impl Drop for Permit {
    fn drop(&mut self) {
        FILE_WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Busy(Arc<AtomicBool>);
impl Drop for Busy {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
struct State {
    transfer: Option<transfer::Receiver<tempfile::NamedTempFile>>,
    initial: Option<(OriginalOffer, tempfile::NamedTempFile)>,
    _bytes: Bytes,
    // File/scope destruction must precede capacity becoming available again.
    _permit: Permit,
}
pub(super) struct Incoming {
    worker: Worker<State>,
    offer: OriginalOffer,
    budget: ByteBudget,
    busy: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    closing: bool,
}
pub(super) struct ChunkJob {
    receive: Option<oneshot::Receiver<SessionResult<ChunkResult>>>,
}
pub(super) struct ChunkResult {
    pub ack: pb::StreamBlob,
    pub original: Option<(OriginalOffer, tempfile::NamedTempFile)>,
}

impl Incoming {
    pub fn new(
        offer: OriginalOffer,
        scratch: tempfile::NamedTempFile,
        budget: ByteBudget,
    ) -> SessionResult<Self> {
        FILE_WORKERS
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 2).then_some(n + 1)
            })
            .map_err(|_| SessionError::Backpressure)?;
        let permit = Permit;
        let bytes = budget.reserve(8192)?;
        // Even the initial seek/empty-file check belongs to this worker.
        let worker = Worker::new(
            "vw-stream-file",
            State {
                transfer: None,
                initial: Some((offer.clone(), scratch)),
                _bytes: bytes,
                _permit: permit,
            },
            1,
        )
        .map_err(SessionError::from)?;
        Ok(Self {
            worker,
            offer,
            budget,
            busy: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            closing: false,
        })
    }
    pub fn start(&self, chunk: pb::StreamBlob) -> SessionResult<ChunkJob> {
        self.start_with(chunk, || Ok(()))
    }
    fn start_with(
        &self,
        chunk: pb::StreamBlob,
        before: impl FnOnce() -> SessionResult<()> + Send + 'static,
    ) -> SessionResult<ChunkJob> {
        if self.closing || self.cancel.load(Ordering::Acquire) {
            return Err(SessionError::Closed);
        }
        let bytes = admit_chunk(&self.offer, &chunk)?;
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| SessionError::Backpressure)?;
        let busy = Busy(self.busy.clone());
        let bytes = self.budget.reserve(bytes)?;
        let cancel = self.cancel.clone();
        let (send, receive) = oneshot::channel();
        self.worker
            .enqueue(Box::new(move |state| {
                let result = (|| {
                    if cancel.load(Ordering::Acquire) {
                        return Err(SessionError::Cancelled);
                    }
                    before()?;
                    if cancel.load(Ordering::Acquire) {
                        return Err(SessionError::Cancelled);
                    }
                    if let Some((offer, scratch)) = state.initial.take() {
                        state.transfer = Some(transfer::Receiver::new(offer, scratch)?);
                    }
                    let transfer = state.transfer.as_mut().ok_or(SessionError::Closed)?;
                    let ack = transfer.receive(&chunk)?;
                    let original = if transfer.complete() {
                        Some(
                            state
                                .transfer
                                .take()
                                .ok_or(SessionError::Worker)?
                                .finish()?,
                        )
                    } else {
                        None
                    };
                    Ok(ChunkResult { ack, original })
                })();
                drop(chunk);
                drop(bytes);
                drop(busy);
                let _ = send.send(result);
            }))
            .map_err(SessionError::from)?;
        Ok(ChunkJob {
            receive: Some(receive),
        })
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn start_queued(&self, queued: QueuedChunk) -> SessionResult<ChunkJob> {
        let QueuedChunk { value, _bytes } = queued;
        self.start(value)
    }
    pub async fn shutdown(&mut self) -> SessionResult<()> {
        self.closing = true;
        self.cancel();
        self.worker
            .shutdown(|_| Ok(()))
            .await
            .map_err(SessionError::from)
    }
}
impl Drop for Incoming {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl ChunkJob {
    /// Cancel-safe wait: the connected select loop retains this receiver until
    /// exact settlement, even if another channel wins a poll.
    pub async fn receive(&mut self) -> SessionResult<ChunkResult> {
        let result = self
            .receive
            .as_mut()
            .ok_or(SessionError::Closed)?
            .await
            .map_err(|_| SessionError::Worker);
        self.receive = None;
        result?
    }
}
/// Admission covers spare protobuf Vec capacity, not just encoded length.
pub(super) fn admit_chunk(offer: &OriginalOffer, value: &pb::StreamBlob) -> SessionResult<u64> {
    let pb::stream_blob::Body::Chunk(chunk) = offer.scope.check(value)? else {
        return Err(SessionError::Invalid);
    };
    if chunk.asset_id != offer.asset.as_str()
        || chunk.total_size != offer.bytes
        || value.encoded_len() > 128 * 1024 - 1024
    {
        return Err(SessionError::Invalid);
    }
    let retained = (chunk.data.capacity() as u64)
        .checked_add(chunk.asset_id.capacity() as u64)
        .and_then(|v| v.checked_add(4096))
        .ok_or(SessionError::Backpressure)?;
    if retained > 128 * 1024 {
        return Err(SessionError::Backpressure);
    }
    Ok(retained)
}
/// Queued chunks retain their reservation until moved into the actual writer.
pub(super) struct QueuedChunk {
    pub value: pb::StreamBlob,
    _bytes: Bytes,
}
impl QueuedChunk {
    pub fn new(
        offer: &OriginalOffer,
        value: pb::StreamBlob,
        budget: &ByteBudget,
    ) -> SessionResult<Self> {
        let bytes = budget.reserve(admit_chunk(offer, &value)?)?;
        Ok(Self {
            value,
            _bytes: bytes,
        })
    }
}

#[cfg(test)]
#[path = "incoming_tests.rs"]
mod tests;
