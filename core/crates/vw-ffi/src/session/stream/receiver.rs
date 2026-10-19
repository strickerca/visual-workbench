//! Authenticated driver's receiver owner. Network receipt, codec completion and
//! actual platform presentation are separate transitions with separate owners.
use super::{
    decoder::{DecodeJob, Decoder},
    memory::{ByteBudget, Bytes, OwnedFrame},
    order::{Arrival, MediaQueue, Source},
    presentation::{Draw, Freeze, Presentation},
    wire::frame_identity,
};
use crate::session::{SessionError, SessionResult};
use std::sync::Arc;
use vw_model::Id;
use vw_proto::v1 as pb;
use vw_stream::{FreezePhase, FreezeRequest, StreamMode, VerifiedOriginal};

pub(super) struct StreamReceiver {
    generation: u64,
    queue: MediaQueue,
    decoder: Decoder,
    job: Option<DecodeJob>,
    decoded: Option<Arc<OwnedFrame>>,
    shown: Option<Presentation>,
    frozen: Option<Freeze>,
    paused: bool,
    closed: bool,
    budget: ByteBudget,
    // Last field drops after queued/in-flight protobuf owners. The reservation
    // covers the sole queued packet plus one in the decoder; decoder scratch is
    // additionally admitted. Keep metadata headroom for all bounded strings.
    _packets: Bytes,
}
impl StreamReceiver {
    pub fn new(epoch: u64, budget_bytes: u64) -> SessionResult<Self> {
        let queue = MediaQueue::new(epoch)?;
        let budget = ByteBudget::new(budget_bytes)?;
        let packets = budget.reserve(2 * (vw_stream::MAX_TILE_BYTES as u64 + 64 * 1024))?;
        let decoder = Decoder::new(budget.clone())?;
        Ok(Self {
            generation: 0,
            queue,
            decoder,
            job: None,
            decoded: None,
            shown: None,
            frozen: None,
            paused: false,
            closed: false,
            budget,
            _packets: packets,
        })
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn open(&mut self, control: &pb::StreamControl) -> SessionResult<bool> {
        if self.closed {
            return Err(SessionError::Closed);
        }
        let source = Source::parse(control)?;
        // A remote source change can never silently discard the user's draft.
        if self
            .frozen
            .as_ref()
            .is_some_and(|v| v.phase() != FreezePhase::Live)
        {
            return Err(SessionError::Backpressure);
        }
        let mode = if source.interaction == "remote_edit" {
            StreamMode::RemoteEdit
        } else {
            StreamMode::LiveAnnotate
        };
        let generation = source.generation;
        if !self.queue.open(source)? {
            return Ok(false);
        }
        self.decoder.cancel_current();
        self.decoded = None;
        self.paused = false;
        self.generation = generation;
        // Driver publishes the source-generation change before exposing draws.
        // Platform must fence old-source work before drawing a fresh ticket.
        self.shown = Some(Presentation::new(generation, &self.budget)?);
        self.frozen = Some(Freeze::new(generation, mode, &self.budget)?);
        Ok(true)
    }
    pub fn control(&mut self, control: &pb::StreamControl) -> SessionResult<bool> {
        if self.closed {
            return Err(SessionError::Closed);
        }
        if !matches!(
            control.action.as_str(),
            "pause" | "resume" | "stop" | "source_closed"
        ) {
            return Err(SessionError::Invalid);
        }
        let Some(source) = self.queue.source() else {
            return Ok(false);
        };
        if !source.matches_control(control)? {
            return Ok(false);
        }
        match control.action.as_str() {
            "pause" => {
                self.paused = true;
                self.queue.pause();
                self.decoder.cancel_current();
            }
            "resume" => {
                if self
                    .frozen
                    .as_ref()
                    .is_some_and(|v| v.phase() != FreezePhase::Live)
                {
                    return Ok(false);
                }
                self.paused = false;
                self.queue.pause();
            }
            _ => {
                self.queue.close(control)?;
                self.paused = true;
                self.decoder.cancel_current();
                if let Some(frozen) = &mut self.frozen {
                    frozen.source_closed();
                }
                // Preserve acknowledged pixels and any draft on source loss.
            }
        }
        Ok(true)
    }
    pub fn media(&mut self, packet: pb::FrameTiles) -> SessionResult<Arrival> {
        if self.closed {
            return Err(SessionError::Closed);
        }
        if self.paused {
            frame_identity(&packet)?;
            return Ok(Arrival::Ignored);
        }
        self.queue.push(packet)
    }
    pub fn start_decode(&mut self) -> SessionResult<bool> {
        if self.closed || self.paused || self.job.is_some() {
            return Ok(false);
        }
        let Some(work) = self.queue.take()? else {
            return Ok(false);
        };
        let ticket = work.ticket.clone();
        match self.decoder.start(work, self.decoded.clone()) {
            Ok(job) => {
                self.job = Some(job);
                Ok(true)
            }
            Err(error) => {
                self.queue.settle(&ticket, false);
                Err(error)
            }
        }
    }
    pub fn has_decode(&self) -> bool {
        self.job.is_some()
    }
    /// Keep this receiver in the connected driver's select loop. Cancellation of
    /// this *waiting future* does not cancel or lose the underlying codec result.
    pub async fn complete_decode(&mut self) -> SessionResult<bool> {
        let job = self.job.as_mut().ok_or(SessionError::Closed)?;
        let ticket = job.ticket.clone();
        let result = job.receive().await;
        self.job = None;
        let admitted = self.queue.settle(&ticket, result.is_ok());
        if !admitted || self.closed || self.paused {
            return Ok(false);
        }
        let frame = result?;
        self.shown
            .as_mut()
            .ok_or(SessionError::Invalid)?
            .offer(frame.clone())?;
        self.decoded = Some(frame);
        Ok(true)
    }
    pub fn repair_control(&self) -> Option<pb::StreamControl> {
        if self.closed || self.paused || !self.queue.needs_repair() {
            return None;
        }
        self.queue.source().map(|v| v.control("keyframe"))
    }
    pub fn begin_draw(&mut self) -> SessionResult<Option<Draw>> {
        if self.closed
            || self
                .frozen
                .as_ref()
                .is_some_and(|v| v.phase() != FreezePhase::Live)
        {
            return Ok(None);
        }
        self.shown.as_mut().map_or(Ok(None), Presentation::begin)
    }
    pub fn draw_receipt(
        &mut self,
        generation: u64,
        token: &vw_stream::PresentationToken,
        shown: bool,
    ) -> SessionResult<()> {
        if self.closed || self.generation != generation {
            return Err(SessionError::Closed);
        }
        self.shown
            .as_mut()
            .ok_or(SessionError::Closed)?
            .settle(token, shown)
    }
    pub fn contact(
        &mut self,
        draft: Id,
        valid: bool,
        now_ms: u64,
    ) -> SessionResult<Option<FreezeRequest>> {
        if self.closed {
            return Err(SessionError::Closed);
        }
        let Some(frozen) = &mut self.frozen else {
            return Ok(None);
        };
        let request = frozen.contact(
            draft,
            self.shown.as_ref().ok_or(SessionError::Closed)?,
            valid,
            now_ms,
        )?;
        if request.is_some() {
            self.paused = true;
            self.queue.pause();
            self.decoder.cancel_current();
        }
        Ok(request)
    }
    pub fn frozen(&self) -> Option<&Freeze> {
        self.frozen.as_ref()
    }
    pub fn lossless(
        &mut self,
        request: &FreezeRequest,
        original: Arc<VerifiedOriginal>,
    ) -> SessionResult<bool> {
        self.frozen
            .as_mut()
            .map_or(Ok(false), |v| v.lossless(request, original))
    }
    pub fn unavailable(&mut self, request: &FreezeRequest) -> bool {
        self.frozen.as_mut().is_some_and(|v| v.unavailable(request))
    }
    pub fn tick(&mut self, now_ms: u64) -> bool {
        self.frozen.as_mut().is_some_and(|v| v.tick(now_ms))
    }
    pub fn saved(&mut self, request: &FreezeRequest) -> SessionResult<()> {
        self.frozen
            .as_mut()
            .ok_or(SessionError::Closed)?
            .saved(request)
    }
    /// Only after the app has settled/disposed the real unsaved gesture owner.
    pub fn cancel_resume_after_draft_disposed(&mut self) -> Option<Id> {
        let result = self.frozen.as_mut().and_then(Freeze::cancel_resume);
        if result.is_some() {
            self.paused = false;
            self.queue.pause();
        }
        result
    }
    pub fn retained_bytes(&self) -> u64 {
        self.budget.used()
    }
    pub fn budget(&self) -> &ByteBudget {
        &self.budget
    }
    pub fn verify_original(
        &mut self,
        offer: super::wire::OriginalOffer,
        scratch: tempfile::NamedTempFile,
    ) -> SessionResult<super::decoder::OriginalJob> {
        if self.closed || self.job.is_some() {
            return Err(SessionError::Backpressure);
        }
        let request = self
            .frozen
            .as_ref()
            .and_then(Freeze::request)
            .ok_or(SessionError::Invalid)?;
        if offer.frame != request.frame
            || !offer.scope.matches(&request.frame, &request.draft_id)
            || offer.scope.generation != self.generation
        {
            return Err(SessionError::Invalid);
        }
        if self
            .frozen
            .as_ref()
            .and_then(Freeze::frame)
            .is_none_or(|v| v.stamp() != &offer.stamp)
        {
            return Err(SessionError::Invalid);
        }
        self.decoder.original(offer, scratch)
    }
    pub async fn shutdown(&mut self) -> SessionResult<()> {
        self.closed = true;
        self.paused = true;
        self.queue.pause();
        self.decoder.cancel_current();
        // Worker completion precedes forgetting the decode receipt/old base.
        self.decoder.shutdown().await?;
        self.job = None;
        self.decoded = None;
        Ok(())
    }
}
