//! Local approved-source authority, distinct from a remote source announcement.
//! Captured frame identity is immutable; only delivery_sequence is assigned by
//! this owner. No method selects an OS window/monitor or grants remote input.
use super::{
    memory::{ByteBudget, Bytes},
    order::Source,
    wire::{CaptureStamp, frame_identity, id},
};
use crate::session::{SessionError, SessionResult};
use std::collections::VecDeque;
use vw_model::Id;
use vw_proto::{
    Message,
    v1::{self as pb, envelope::Body},
};
use vw_stream::FrameIdentity;

const HISTORY: usize = 32;
const HISTORY_BYTES: u64 = 128 * 1024;
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PublishedFrame {
    pub frame: FrameIdentity,
    pub stamp: CaptureStamp,
}
#[derive(Clone, Debug, PartialEq)]
pub(super) struct OriginalRequest {
    pub sequence: u64,
    pub draft: Id,
    pub frame: PublishedFrame,
}
pub(super) enum FreezeAdmission {
    Ignored,
    Unavailable(pb::LosslessFrame),
    Request(OriginalRequest),
}
pub(super) struct PreparedFrame {
    packet: pb::FrameTiles,
    _bytes: Bytes,
}
impl PreparedFrame {
    /// Run on the producer's owned worker. Repeated fields are admitted before
    /// prost allocates; caller-supplied spare capacity is charged as well.
    pub fn decode(encoded: Vec<u8>, budget: &ByteBudget) -> SessionResult<Self> {
        if encoded.len() > vw_proto::capture_admission::MAX_MEDIA_BYTES
            || encoded.capacity() > vw_proto::capture_admission::MAX_MEDIA_BYTES
        {
            return Err(SessionError::Backpressure);
        }
        vw_proto::capture_admission::frame_tiles(&encoded).map_err(|_| SessionError::Invalid)?;
        let encoded_charge = budget.reserve(encoded.capacity() as u64)?;
        let packet_charge = budget.reserve(vw_stream::MAX_TILE_BYTES as u64 + 64 * 1024)?;
        let packet =
            pb::FrameTiles::decode(encoded.as_slice()).map_err(|_| SessionError::Invalid)?;
        // A local helper has no authenticated network-generation authority.
        if packet.source_generation != 0
            || packet.delivery_sequence != 0
            || packet.last_input_seq_applied != 0
        {
            return Err(SessionError::Invalid);
        }
        super::wire::packet_capacity(&packet)?;
        drop(encoded);
        drop(encoded_charge);
        Ok(Self {
            packet,
            _bytes: packet_charge,
        })
    }
}

pub(super) struct PublishOrder {
    source: Source,
    delivery: u64,
    last: Option<PublishedFrame>,
    history: VecDeque<PublishedFrame>,
    paused: bool,
    closed: bool,
    repair: u64,
    request_sequence: u64,
    request: Option<OriginalRequest>,
    last_request_at: Option<u64>,
    pending_open: bool,
    _history: Bytes,
}
impl PublishOrder {
    pub fn new(source: Source, budget: &ByteBudget) -> SessionResult<Self> {
        Source::parse(&source.control("start"))?;
        Ok(Self {
            source,
            delivery: 0,
            last: None,
            history: VecDeque::new(),
            paused: false,
            closed: false,
            repair: 1,
            request_sequence: 0,
            request: None,
            last_request_at: None,
            pending_open: true,
            _history: budget.reserve(HISTORY_BYTES)?,
        })
    }
    pub fn source(&self) -> &Source {
        &self.source
    }
    pub fn paused(&self) -> bool {
        self.paused
    }
    pub fn closed(&self) -> bool {
        self.closed
    }
    pub fn repair(&self) -> u64 {
        self.repair
    }
    pub fn request(&self) -> Option<&OriginalRequest> {
        self.request.as_ref()
    }
    pub fn current_request(&self, request: &OriginalRequest) -> bool {
        !self.closed && self.request.as_ref() == Some(request)
    }
    pub fn announce(
        &mut self,
        mut enqueue: impl FnMut(Body) -> vw_net::Result<()>,
    ) -> SessionResult<bool> {
        if self.closed {
            return Err(SessionError::Closed);
        }
        if self.pending_open {
            match enqueue(Body::StreamControl(self.source.control("start"))) {
                Ok(()) => self.pending_open = false,
                Err(vw_net::NetError::Backpressure) => return Ok(false),
                Err(error) => return Err(error.into()),
            }
        }
        Ok(true)
    }
    /// Queue admission is the only feedback supplied to the persistent helper;
    /// it is never a claim that the peer rendered or durably saved these pixels.
    pub fn publish(
        &mut self,
        prepared: PreparedFrame,
        mut enqueue: impl FnMut(Body) -> vw_net::Result<()>,
    ) -> SessionResult<bool> {
        if self.closed {
            return Err(SessionError::Closed);
        }
        let PreparedFrame { mut packet, _bytes } = prepared;
        let next = self
            .delivery
            .checked_add(1)
            .ok_or(SessionError::Backpressure)?;
        packet.delivery_sequence = next;
        packet.source_generation = self.source.generation;
        let frame = frame_identity(&packet)?;
        if !self.source.admits(&frame)
            || frame.timestamp_ns < self.source.geometry.wire().timestamp_ns
        {
            return Err(SessionError::Invalid);
        }
        let value = PublishedFrame {
            frame,
            stamp: CaptureStamp::new(packet.captured_at_ms, &self.source.platform)?,
        };
        if let Some(prior) = &self.last {
            if value.frame.frame_id < prior.frame.frame_id {
                return Err(SessionError::Invalid);
            }
            if value.frame.frame_id == prior.frame.frame_id {
                if &value != prior || !packet.keyframe {
                    return Err(SessionError::Invalid);
                }
            } else if value.frame.timestamp_ns <= prior.frame.timestamp_ns {
                return Err(SessionError::Invalid);
            }
        }
        self.last = Some(value.clone());
        if self.paused {
            return Ok(false);
        }
        if !self.announce(&mut enqueue)? {
            return Ok(false);
        }
        // Gaps explicitly communicate a declined MEDIA admission, so a peer
        // cannot accidentally apply a later predictive delta to an old base.
        self.delivery = next;
        match enqueue(Body::FrameTiles(packet)) {
            Ok(()) => {
                if self.history.back() != Some(&value) {
                    if self.history.len() == HISTORY {
                        self.history.pop_front();
                    }
                    self.history.push_back(value);
                }
                Ok(true)
            }
            Err(vw_net::NetError::Backpressure) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
    /// Only matching authenticated CONTROL changes the already-approved source.
    /// It cannot broaden selection or alter capture metadata. Resume invalidates
    /// the exact prior freeze ticket and requests a real repair keyframe.
    pub fn control(&mut self, value: &pb::StreamControl) -> SessionResult<bool> {
        if !self.source.matches_control(value)? {
            return Ok(false);
        }
        match value.action.as_str() {
            "pause" => self.paused = true,
            "resume" => {
                if !self.closed {
                    self.paused = false;
                    self.request = None;
                    self.bump_repair()?;
                }
            }
            "keyframe" => {
                if !self.closed {
                    self.bump_repair()?;
                }
            }
            "stop" => {
                self.closed = true;
                self.paused = true;
                self.request = None;
            }
            _ => return Err(SessionError::Invalid),
        }
        Ok(true)
    }
    fn bump_repair(&mut self) -> SessionResult<()> {
        self.repair = self
            .repair
            .checked_add(1)
            .ok_or(SessionError::Backpressure)?;
        Ok(())
    }
    pub fn freeze(
        &mut self,
        value: &pb::FreezeRequest,
        now_ms: u64,
    ) -> SessionResult<FreezeAdmission> {
        admit_freeze(value)?;
        let session = id(value.capture_session_id.as_ref())?;
        let draft = id(value.request_id.as_ref())?;
        if self.closed
            || session != self.source.session
            || value.source_generation != self.source.generation
            || value.geometry_revision != self.source.geometry.revision()
        {
            return Ok(FreezeAdmission::Ignored);
        }
        if let Some(request) = &self.request {
            return Ok(
                if request.draft == draft && request.frame.frame.frame_id == value.frame_id {
                    FreezeAdmission::Request(request.clone())
                } else {
                    FreezeAdmission::Unavailable(unavailable(value))
                },
            );
        }
        // Four new original requests/s maximum. Delayed lawful traffic refuses
        // only the transient request, never tears down OPS or widens authority.
        if self
            .last_request_at
            .is_some_and(|prior| now_ms.saturating_sub(prior) < 250)
        {
            return Ok(FreezeAdmission::Unavailable(unavailable(value)));
        }
        let Some(frame) = self
            .history
            .iter()
            .find(|item| item.frame.frame_id == value.frame_id)
            .cloned()
        else {
            return Ok(FreezeAdmission::Unavailable(unavailable(value)));
        };
        self.request_sequence = self
            .request_sequence
            .checked_add(1)
            .ok_or(SessionError::Backpressure)?;
        self.last_request_at = Some(now_ms);
        self.paused = true;
        let request = OriginalRequest {
            sequence: self.request_sequence,
            draft,
            frame,
        };
        self.request = Some(request.clone());
        Ok(FreezeAdmission::Request(request))
    }
    pub fn stop(&mut self) {
        self.closed = true;
        self.paused = true;
        self.request = None;
    }
}
pub(super) fn admit_freeze(value: &pb::FreezeRequest) -> SessionResult<()> {
    if value.encoded_len() > 128
        || value.frame_id == 0
        || value.source_generation == 0
        || value.geometry_revision == 0
    {
        return Err(SessionError::Invalid);
    }
    id(value.capture_session_id.as_ref())?;
    id(value.request_id.as_ref())?;
    Ok(())
}
fn unavailable(value: &pb::FreezeRequest) -> pb::LosslessFrame {
    pb::LosslessFrame {
        capture_session_id: value.capture_session_id.clone(),
        frame_id: value.frame_id,
        request_id: value.request_id.clone(),
        ..Default::default()
    }
}

#[cfg(test)]
#[path = "publish_order_tests.rs"]
mod tests;
