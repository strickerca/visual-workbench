//! Bounded connection/source/transmission ordering. Captured identity never
//! changes when the same original is retransmitted as a repair keyframe.
use super::wire::{CaptureStamp, frame_identity, id};
use crate::session::{SessionError, SessionResult};
use vw_model::Id;
use vw_proto::{Message, v1 as pb};
use vw_stream::{FrameIdentity, StreamGeometry};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Source {
    pub session: Id,
    pub generation: u64,
    pub geometry: StreamGeometry,
    pub fps: u32,
    pub interaction: String,
    pub platform: String,
    pub name: String,
}
impl Source {
    pub fn parse(value: &pb::StreamControl) -> SessionResult<Self> {
        if value.source_name.is_empty()
            || value.source_name.len() > 256
            || value.source_name.chars().any(char::is_control)
            || value.encoded_len() > 1024
            || value.source_generation == 0
            || value.action != "start"
            || value.mode != "frame_tiles"
            || !(15..=60).contains(&value.target_fps)
            || !matches!(value.interaction.as_str(), "live_annotate" | "remote_edit")
            || !matches!(value.platform.as_str(), "windows" | "android")
        {
            return Err(SessionError::Invalid);
        }
        let wire = value
            .source_geometry
            .as_ref()
            .ok_or(SessionError::Invalid)?;
        if wire.source_kind != value.source_kind
            || wire.window_handle != value.window_handle
            || wire.monitor_id != value.monitor_id
        {
            return Err(SessionError::Invalid);
        }
        Ok(Self {
            session: id(value.capture_session_id.as_ref())?,
            generation: value.source_generation,
            geometry: StreamGeometry::new(wire, value.max_width, value.max_height)
                .map_err(|_| SessionError::Invalid)?,
            fps: value.target_fps,
            interaction: value.interaction.clone(),
            platform: value.platform.clone(),
            name: value.source_name.clone(),
        })
    }
    pub fn control(&self, action: &str) -> pb::StreamControl {
        let g = self.geometry.wire();
        pb::StreamControl {
            capture_session_id: Some(self.session.to_proto()),
            action: action.into(),
            source_kind: g.source_kind.clone(),
            window_handle: g.window_handle,
            monitor_id: g.monitor_id.clone(),
            max_width: self.geometry.width(),
            max_height: self.geometry.height(),
            target_fps: self.fps,
            mode: "frame_tiles".into(),
            source_geometry: Some(g.clone()),
            interaction: self.interaction.clone(),
            source_generation: self.generation,
            platform: self.platform.clone(),
            source_name: self.name.clone(),
        }
    }
    pub fn matches_control(&self, value: &pb::StreamControl) -> SessionResult<bool> {
        // Reuse exact metadata validation, preserving the bounded action field.
        if !matches!(
            value.action.as_str(),
            "start" | "stop" | "pause" | "resume" | "keyframe" | "source_closed"
        ) || value.encoded_len() > 1024
        {
            return Err(SessionError::Invalid);
        }
        let mut start = value.clone();
        start.action = "start".into();
        Ok(Self::parse(&start)? == *self)
    }
    pub fn admits(&self, frame: &FrameIdentity) -> bool {
        frame.session == self.session && self.geometry.same_source(&frame.geometry)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct DecodeTicket {
    pub epoch: u64,
    pub serial: u64,
    pub delivery: u64,
    pub generation: u64,
    pub frame: FrameIdentity,
    pub stamp: CaptureStamp,
}
pub(super) struct DecodeWork {
    pub ticket: DecodeTicket,
    pub packet: pb::FrameTiles,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Arrival {
    Queued,
    Ignored,
    Repair,
}

/// One owned decoder job plus one queued packet. A predictive packet cannot
/// replace its own predecessor. All failure paths request a keyframe rather
/// than exposing partially reconstructed pixels or disconnecting OPS.
pub(super) struct MediaQueue {
    epoch: u64,
    highest_source: u64,
    source: Option<Source>,
    delivery: u64,
    serial: u64,
    pending: Option<(FrameIdentity, CaptureStamp, pb::FrameTiles)>,
    decoding: Option<DecodeTicket>,
    decoded: Option<(FrameIdentity, CaptureStamp)>,
    repair: bool,
}
impl MediaQueue {
    pub fn new(epoch: u64) -> SessionResult<Self> {
        if epoch & 1 == 0 {
            return Err(SessionError::Invalid);
        }
        Ok(Self {
            epoch,
            highest_source: 0,
            source: None,
            delivery: 0,
            serial: 0,
            pending: None,
            decoding: None,
            decoded: None,
            repair: true,
        })
    }
    pub fn source(&self) -> Option<&Source> {
        self.source.as_ref()
    }
    pub fn open(&mut self, value: Source) -> SessionResult<bool> {
        if value.generation < self.highest_source {
            return Ok(false);
        }
        if value.generation == self.highest_source {
            // Duplicates never reset watermarks, queued pixels or decoded base.
            return match &self.source {
                Some(prior) if prior == &value => Ok(false),
                None => Ok(false),
                _ => Err(SessionError::Invalid),
            };
        }
        self.highest_source = value.generation;
        self.source = Some(value);
        self.delivery = 0;
        // An old decoder still owns its memory/worker until exact settlement.
        self.pending = None;
        self.decoded = None;
        self.repair = true;
        Ok(true)
    }
    pub fn close(&mut self, value: &pb::StreamControl) -> SessionResult<bool> {
        let Some(source) = &self.source else {
            return Ok(false);
        };
        if !source.matches_control(value)? {
            return Ok(false);
        }
        self.source = None;
        self.pending = None;
        self.decoded = None;
        self.repair = true;
        Ok(true)
    }
    pub fn push(&mut self, packet: pb::FrameTiles) -> SessionResult<Arrival> {
        let frame = frame_identity(&packet)?; // Validate even ignored data first.
        let Some(source) = &self.source else {
            return Ok(Arrival::Ignored);
        };
        if packet.source_generation != source.generation || !source.admits(&frame) {
            return Ok(Arrival::Ignored);
        }
        let stamp = CaptureStamp::new(packet.captured_at_ms, &source.platform)?;
        if packet.delivery_sequence <= self.delivery {
            return Ok(Arrival::Ignored);
        }
        let consecutive = self.delivery.checked_add(1) == Some(packet.delivery_sequence);
        self.delivery = packet.delivery_sequence;
        // Same-original repair is useful only if the original was never decoded.
        // It must never overwrite newer pixels or fabricate a new capture time.
        if let Some((decoded, prior_stamp)) = &self.decoded {
            if frame.frame_id == decoded.frame_id {
                if &frame != decoded || &stamp != prior_stamp {
                    return Err(SessionError::Invalid);
                }
                return Ok(Arrival::Ignored);
            }
            if frame.frame_id < decoded.frame_id {
                if frame.timestamp_ns >= decoded.timestamp_ns {
                    return Err(SessionError::Invalid);
                }
                return Ok(Arrival::Ignored);
            }
        }
        if let Some(decoding) = &self.decoding
            && decoding.generation == packet.source_generation
        {
            if decoding.frame.frame_id == frame.frame_id {
                if decoding.frame != frame || decoding.stamp != stamp {
                    return Err(SessionError::Invalid);
                }
                return Ok(Arrival::Ignored);
            }
            if frame.frame_id < decoding.frame.frame_id {
                if frame.timestamp_ns >= decoding.frame.timestamp_ns {
                    return Err(SessionError::Invalid);
                }
                return Ok(Arrival::Ignored);
            }
        }
        if let Some((pending, prior_stamp, _)) = &self.pending {
            if frame.frame_id == pending.frame_id && (&frame != pending || &stamp != prior_stamp) {
                return Err(SessionError::Invalid);
            }
            if frame.frame_id < pending.frame_id {
                if frame.timestamp_ns >= pending.timestamp_ns {
                    return Err(SessionError::Invalid);
                }
                return Ok(Arrival::Ignored);
            }
        }
        if !packet.keyframe {
            let predecessor = self
                .decoding
                .as_ref()
                .map(|v| &v.frame)
                .or(self.decoded.as_ref().map(|v| &v.0));
            if self.repair
                || !consecutive
                || self.pending.is_some()
                || predecessor.is_none_or(|p| {
                    p.frame_id.checked_add(1) != Some(frame.frame_id)
                        || p.timestamp_ns >= frame.timestamp_ns
                })
            {
                self.pending = None;
                self.repair = true;
                return Ok(Arrival::Repair);
            }
        }
        // A keyframe can supersede pending data. A decoder still using an older
        // base retains its owned bytes until completion; it is never abandoned.
        self.pending = Some((frame, stamp, packet));
        Ok(Arrival::Queued)
    }
    pub fn take(&mut self) -> SessionResult<Option<DecodeWork>> {
        if self.decoding.is_some() {
            return Ok(None);
        }
        let Some((frame, stamp, packet)) = self.pending.take() else {
            return Ok(None);
        };
        self.serial = self
            .serial
            .checked_add(1)
            .ok_or(SessionError::Backpressure)?;
        let ticket = DecodeTicket {
            epoch: self.epoch,
            serial: self.serial,
            delivery: packet.delivery_sequence,
            generation: packet.source_generation,
            frame,
            stamp,
        };
        self.decoding = Some(ticket.clone());
        Ok(Some(DecodeWork { ticket, packet }))
    }
    /// Caller must settle even an undelivered/cancelled codec result. true admits
    /// immutable pixels for subsequent presentation; it is not a draw receipt.
    pub fn settle(&mut self, ticket: &DecodeTicket, success: bool) -> bool {
        if self.decoding.as_ref() != Some(ticket) {
            return false;
        }
        self.decoding = None;
        if self
            .source
            .as_ref()
            .is_none_or(|s| s.generation != ticket.generation || !s.admits(&ticket.frame))
        {
            return false;
        }
        if !success {
            self.pending = None;
            self.repair = true;
            return false;
        }
        self.decoded = Some((ticket.frame.clone(), ticket.stamp.clone()));
        self.repair = false;
        true
    }
    pub fn needs_repair(&self) -> bool {
        self.source.is_some() && self.repair && self.pending.is_none() && self.decoding.is_none()
    }
    pub fn pause(&mut self) {
        self.pending = None;
        self.repair = true;
    }
}

/// Only local publication stamps this order; caller-supplied order is ignored.
/// A failed send still consumes its number, forcing a subsequent keyframe.
#[cfg(test)]
pub(super) struct Delivery {
    source: Source,
    next: u64,
}
#[cfg(test)]
impl Delivery {
    pub fn new(source: Source) -> Self {
        Self { source, next: 0 }
    }
    pub fn stamp(&mut self, packet: &mut pb::FrameTiles) -> SessionResult<()> {
        // Admission uses a provisional nonzero stamp but binds real geometry.
        packet.delivery_sequence = 1;
        packet.source_generation = self.source.generation;
        if !self.source.admits(&frame_identity(packet)?) {
            return Err(SessionError::Invalid);
        }
        self.next = self.next.checked_add(1).ok_or(SessionError::Backpressure)?;
        packet.delivery_sequence = self.next;
        Ok(())
    }
}
