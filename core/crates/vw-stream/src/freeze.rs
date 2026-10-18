use crate::{DisplayFrame, FrameIdentity, Result, StreamError, VerifiedOriginal};
use std::sync::Arc;
use vw_model::{AssetId, Id};

pub const FREEZE_TIMEOUT_MS: u64 = 2_000;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamMode {
    LiveAnnotate,
    RemoteEdit,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreezePhase {
    Live,
    Freezing,
    FrozenDraft,
    Saved,
}
#[derive(Debug, Clone, PartialEq)]
pub struct FreezeRequest {
    pub draft_id: Id,
    pub frame: FrameIdentity,
    pub epoch: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FreezeQuality {
    Waiting,
    Lossless { asset_id: AssetId },
    DegradedCompressed,
}
#[derive(Debug, Clone)]
pub struct FrozenDraft {
    pub request: FreezeRequest,
    pub displayed: Arc<DisplayFrame>,
    pub quality: FreezeQuality,
    started_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentationToken {
    epoch: u64,
    sequence: u64,
}
/// A frame becomes eligible for first-contact freeze only after the platform's
/// actual draw/presentation callback acknowledges this exact ticket. One queued
/// latest frame may coalesce; the sole in-flight frame cannot be replaced.
/// The platform must serialize actual draw + receipt before accepted contact.
pub struct PresentedFrames {
    epoch: u64,
    next: u64,
    queued: Option<Arc<DisplayFrame>>,
    in_flight: Option<(PresentationToken, Arc<DisplayFrame>)>,
    displayed: Option<Arc<DisplayFrame>>,
}
impl PresentedFrames {
    pub fn new(epoch: u64) -> Result<Self> {
        if epoch == 0 {
            return Err(StreamError::Invalid);
        }
        Ok(Self {
            epoch,
            next: 0,
            queued: None,
            in_flight: None,
            displayed: None,
        })
    }
    /// Enqueue only. This never grants permission to draw the offered frame.
    pub fn offer(&mut self, frame: Arc<DisplayFrame>) -> Result<()> {
        let last = self
            .queued
            .as_ref()
            .or(self.in_flight.as_ref().map(|(_, v)| v))
            .or(self.displayed.as_ref());
        if last.is_some_and(|old| {
            old.identity().session != frame.identity().session
                || !old
                    .identity()
                    .geometry
                    .same_source(&frame.identity().geometry)
                || old.identity().frame_id >= frame.identity().frame_id
                || old.identity().timestamp_ns >= frame.identity().timestamp_ns
        }) {
            return Err(StreamError::Stale);
        }
        self.queued = Some(frame);
        Ok(())
    }
    /// The sole frame the platform may next draw. Retain this owner through its
    /// actual presentation result; later offers cannot revoke or replace it.
    pub fn begin_presentation(&mut self) -> Result<Option<(PresentationToken, Arc<DisplayFrame>)>> {
        if self.in_flight.is_some() || self.queued.is_none() {
            return Ok(None);
        }
        self.next = self.next.checked_add(1).ok_or(StreamError::Limit)?;
        let frame = self.queued.take().ok_or(StreamError::Invalid)?;
        let token = PresentationToken {
            epoch: self.epoch,
            sequence: self.next,
        };
        self.in_flight = Some((token.clone(), frame.clone()));
        Ok(Some((token, frame)))
    }
    pub fn presented(&mut self, token: &PresentationToken) -> Result<()> {
        self.check_in_flight(token)?;
        self.displayed = self.in_flight.take().map(|(_, frame)| frame);
        Ok(())
    }
    /// Explicit platform receipt that this frame was never drawn. Cancellation
    /// alone is not sufficient if a draw may have occurred.
    pub fn not_presented(&mut self, token: &PresentationToken) -> Result<()> {
        self.check_in_flight(token)?;
        self.in_flight = None;
        Ok(())
    }
    fn check_in_flight(&self, token: &PresentationToken) -> Result<()> {
        if token.epoch != self.epoch
            || self
                .in_flight
                .as_ref()
                .is_none_or(|(expected, _)| expected != token)
        {
            return Err(StreamError::Stale);
        }
        Ok(())
    }
    pub fn displayed(&self) -> Option<&Arc<DisplayFrame>> {
        self.displayed.as_ref()
    }
    /// Caller must first prevent any late platform draw from the old source.
    pub fn invalidate(&mut self) {
        self.queued = None;
        self.in_flight = None;
        self.displayed = None;
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}

/// Pure state machine. Samples remain in the existing real gesture owner; a
/// lossless swap changes only its background. Cancel returns the exact draft
/// identity that the app must dispose before resuming capture.
pub struct FreezeMachine {
    epoch: u64,
    mode: StreamMode,
    phase: FreezePhase,
    draft: Option<FrozenDraft>,
    source_available: bool,
}
impl FreezeMachine {
    pub fn new(epoch: u64, mode: StreamMode) -> Result<Self> {
        if epoch == 0 {
            return Err(StreamError::Invalid);
        }
        Ok(Self {
            epoch,
            mode,
            phase: FreezePhase::Live,
            draft: None,
            source_available: true,
        })
    }
    pub fn phase(&self) -> FreezePhase {
        self.phase
    }
    pub fn draft(&self) -> Option<&FrozenDraft> {
        self.draft.as_ref()
    }
    pub fn source_available(&self) -> bool {
        self.source_available
    }
    /// `valid_pen_contact` is supplied only by the app's real accepted DOWN path
    /// (inside viewport, one pen contact, no hover/palm/prediction/tool refusal).
    /// Remote-edit mode never freezes, even for a valid pen contact.
    pub fn first_contact(
        &mut self,
        draft_id: Id,
        shown: &PresentedFrames,
        valid_pen_contact: bool,
        now_ms: u64,
    ) -> Result<Option<FreezeRequest>> {
        if self.mode == StreamMode::RemoteEdit || !valid_pen_contact {
            return Ok(None);
        }
        if self.phase != FreezePhase::Live {
            return Ok(None);
        }
        if !self.source_available {
            return Err(StreamError::Unavailable);
        }
        if shown.epoch() != self.epoch {
            return Err(StreamError::Stale);
        }
        let displayed = shown.displayed().cloned().ok_or(StreamError::Unavailable)?;
        let request = FreezeRequest {
            draft_id,
            frame: displayed.identity().clone(),
            epoch: self.epoch,
        };
        self.draft = Some(FrozenDraft {
            request: request.clone(),
            displayed,
            quality: FreezeQuality::Waiting,
            started_ms: now_ms,
        });
        self.phase = FreezePhase::Freezing;
        Ok(Some(request))
    }
    /// Only the complete verified BLOB can settle a lossless swap. The request
    /// identity also fences late responses after cancel/new draft.
    pub fn lossless(
        &mut self,
        request: &FreezeRequest,
        original: &VerifiedOriginal,
    ) -> Result<bool> {
        let Some(draft) = &mut self.draft else {
            return Ok(false);
        };
        if self.phase == FreezePhase::Saved
            || &draft.request != request
            || request.epoch != self.epoch
        {
            return Ok(false);
        }
        if original.identity() != &request.frame {
            return Err(StreamError::Stale);
        }
        draft.quality = FreezeQuality::Lossless {
            asset_id: original.asset_id().clone(),
        };
        self.phase = FreezePhase::FrozenDraft;
        Ok(true)
    }
    pub fn unavailable(&mut self, request: &FreezeRequest) -> bool {
        let Some(draft) = &mut self.draft else {
            return false;
        };
        if &draft.request != request
            || self.phase == FreezePhase::Saved
            || matches!(draft.quality, FreezeQuality::Lossless { .. })
        {
            return false;
        }
        draft.quality = FreezeQuality::DegradedCompressed;
        self.phase = FreezePhase::FrozenDraft;
        true
    }
    pub fn tick(&mut self, now_ms: u64) -> bool {
        if self.phase != FreezePhase::Freezing {
            return false;
        }
        let Some(draft) = &mut self.draft else {
            return false;
        };
        if now_ms.saturating_sub(draft.started_ms) < FREEZE_TIMEOUT_MS {
            return false;
        }
        draft.quality = FreezeQuality::DegradedCompressed;
        self.phase = FreezePhase::FrozenDraft;
        true
    }
    /// Called only after the canonical new capture document is durably saved.
    /// Degraded exports must retain the explicit quality flag in their metadata.
    pub fn saved(&mut self, request: &FreezeRequest) -> Result<()> {
        if self.phase != FreezePhase::FrozenDraft
            || self.draft.as_ref().is_none_or(|d| &d.request != request)
        {
            return Err(StreamError::Stale);
        }
        self.phase = FreezePhase::Saved;
        Ok(())
    }
    pub fn cancel_resume(&mut self) -> Option<Id> {
        if self.phase == FreezePhase::Saved {
            return None;
        }
        let id = self.draft.take().map(|d| d.request.draft_id);
        self.phase = FreezePhase::Live;
        id
    }
    /// Preserve shown/frozen data; never silently choose another source.
    pub fn source_closed(&mut self) {
        self.source_available = false;
    }
}
