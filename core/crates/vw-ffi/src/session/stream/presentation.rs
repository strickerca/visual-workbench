//! Byte-accounted adapter around the canonical actual-presentation contract.
use super::{decoder::map_stream, memory::OwnedFrame};
use crate::session::{SessionError, SessionResult};
use std::sync::Arc;
use vw_model::Id;
use vw_stream::{
    FreezeMachine, FreezePhase, FreezeQuality, FreezeRequest, PresentationToken, PresentedFrames,
    StreamMode, VerifiedOriginal,
};

pub(super) struct Draw {
    pub token: PresentationToken,
    pub frame: Arc<OwnedFrame>,
}
pub(super) struct Presentation {
    // Drop canonical unmetered Arc clones BEFORE their matching byte owners.
    core: PresentedFrames,
    queued: Option<Arc<OwnedFrame>>,
    in_flight: Option<Draw>,
    displayed: Option<Arc<OwnedFrame>>,
}
impl Presentation {
    pub fn new(epoch: u64, budget: &super::memory::ByteBudget) -> SessionResult<Self> {
        Ok(Self {
            core: PresentedFrames::new_budgeted(epoch, budget.pool().clone())
                .map_err(map_stream)?,
            queued: None,
            in_flight: None,
            displayed: None,
        })
    }
    pub fn offer(&mut self, frame: Arc<OwnedFrame>) -> SessionResult<()> {
        self.core.offer(frame.frame.clone()).map_err(map_stream)?;
        self.queued = Some(frame);
        Ok(())
    }
    pub fn begin(&mut self) -> SessionResult<Option<Draw>> {
        if self.in_flight.is_some() || self.queued.is_none() {
            return Ok(None);
        }
        let Some((token, temporary)) = self.core.begin_presentation().map_err(map_stream)? else {
            return Ok(None);
        };
        drop(temporary);
        let frame = self.queued.take().ok_or(SessionError::Worker)?;
        self.in_flight = Some(Draw {
            token: token.clone(),
            frame: frame.clone(),
        });
        Ok(Some(Draw { token, frame }))
    }
    pub fn settle(
        &mut self,
        token: &PresentationToken,
        actually_presented: bool,
    ) -> SessionResult<()> {
        if self
            .in_flight
            .as_ref()
            .is_none_or(|draw| &draw.token != token)
        {
            return Err(SessionError::Invalid);
        }
        if actually_presented {
            self.core.presented(token).map_err(map_stream)?
        } else {
            self.core.not_presented(token).map_err(map_stream)?
        }
        let draw = self.in_flight.take().ok_or(SessionError::Worker)?;
        if actually_presented {
            self.displayed = Some(draw.frame)
        }
        Ok(())
    }
    pub fn has_in_flight(&self) -> bool {
        self.in_flight.is_some()
    }
}

pub(super) struct Freeze {
    core: FreezeMachine,
    // The canonical FrozenDraft has an Arc clone; this owner must outlive it.
    retained: Option<Arc<OwnedFrame>>,
    mode: StreamMode,
    budget: super::memory::ByteBudget,
}
impl Freeze {
    pub fn new(
        epoch: u64,
        mode: StreamMode,
        budget: &super::memory::ByteBudget,
    ) -> SessionResult<Self> {
        Ok(Self {
            core: FreezeMachine::new_budgeted(epoch, mode, budget.pool().clone())
                .map_err(map_stream)?,
            retained: None,
            mode,
            budget: budget.clone(),
        })
    }
    pub fn contact(
        &mut self,
        id: Id,
        shown: &Presentation,
        valid: bool,
        now_ms: u64,
    ) -> SessionResult<Option<FreezeRequest>> {
        if self.mode == StreamMode::LiveAnnotate && valid && shown.has_in_flight() {
            return Err(SessionError::Backpressure);
        }
        let request = self
            .core
            .first_contact(id, &shown.core, valid, now_ms)
            .map_err(map_stream)?;
        if request.is_some() {
            self.retained = shown.displayed.clone();
        }
        Ok(request)
    }
    pub fn request(&self) -> Option<&FreezeRequest> {
        self.core.draft().map(|v| &v.request)
    }
    pub fn phase(&self) -> FreezePhase {
        self.core.phase()
    }
    pub fn quality(&self) -> Option<&FreezeQuality> {
        self.core.draft().map(|v| &v.quality)
    }
    pub fn frame(&self) -> Option<&Arc<OwnedFrame>> {
        self.retained.as_ref()
    }
    pub fn lossless(
        &mut self,
        request: &FreezeRequest,
        original: Arc<VerifiedOriginal>,
    ) -> SessionResult<bool> {
        self.core
            .lossless_budgeted(request, original)
            .map_err(map_stream)
    }
    pub fn unavailable(&mut self, request: &FreezeRequest) -> bool {
        self.core.unavailable(request)
    }
    pub fn tick(&mut self, now_ms: u64) -> bool {
        self.core.tick(now_ms)
    }
    pub fn saved(&mut self, request: &FreezeRequest) -> SessionResult<()> {
        self.core.saved(request).map_err(map_stream)
    }
    pub fn cancel_resume(&mut self) -> Option<Id> {
        if self.core.phase() == FreezePhase::Saved {
            let id = self.core.draft()?.request.draft_id.clone();
            let epoch = self.core.draft()?.request.epoch;
            let available = self.core.source_available();
            self.core =
                FreezeMachine::new_budgeted(epoch, self.mode, self.budget.pool().clone()).ok()?;
            if !available {
                self.core.source_closed();
            }
            self.retained = None;
            return Some(id);
        }
        let result = self.core.cancel_resume();
        if self.core.draft().is_none() {
            self.retained = None
        }
        result
    }
    pub fn source_closed(&mut self) {
        self.core.source_closed();
    }
}
