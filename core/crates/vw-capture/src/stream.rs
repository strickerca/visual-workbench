//! Persistent capture producer. The endpoint owns this actor and its budget;
//! it makes no presentation, draft, freeze, or save decision.
use crate::{Error, FrameIdentity as CaptureIdentity, Result, picker::PickerSource};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, time::Instant};
use vw_proto::v1;
use vw_stream::{
    BudgetedBytes, BudgetedEncodedOriginal, Cancellation, FrameRing, FrameWriter, MemoryCharge,
    MemoryKind, OriginalFrame, SessionBudget, StreamError, StreamGeometry, StreamPacer,
};

pub const PROCESS_MEMORY_BYTES: u64 = 512 * 1024 * 1024;
pub const STREAM_RING_FRAMES: usize = 8;
pub const MAX_STREAM_PACKET_BYTES: usize = vw_stream::MAX_LOSSLESS_BYTES;

pub fn stream_error(error: StreamError) -> Error {
    match error {
        StreamError::Invalid => Error::Invalid,
        StreamError::Stale => Error::Stale,
        StreamError::Limit => Error::Limit,
        StreamError::Unavailable | StreamError::KeyframeRequired => Error::Unavailable,
        StreamError::Consent => Error::Consent,
        StreamError::Cancelled => Error::Cancelled,
        StreamError::Codec => Error::Platform,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamRequest {
    pub explicit_owner_action: bool,
    pub owner_process_id: u32,
    pub capture_session_id: String,
    pub connection_epoch: u64,
    pub source_generation: u64,
    pub source: PickerSource,
    pub geometry_revision: u32,
    pub target_fps: u32,
    pub endpoint_bytes: u64,
    pub ring_bytes: u64,
}
impl StreamRequest {
    pub fn validate(&self) -> Result<()> {
        if !self.explicit_owner_action {
            return Err(Error::Consent);
        }
        if self.owner_process_id == 0
            || self.connection_epoch == 0
            || self.source_generation == 0
            || self.geometry_revision == 0
            || self.ring_bytes == 0
            || self.ring_bytes > vw_stream::MAX_RING_BYTES
            || self.endpoint_bytes == 0
            || self.endpoint_bytes > PROCESS_MEMORY_BYTES
        {
            return Err(Error::Invalid);
        }
        vw_model::Id::try_from(self.capture_session_id.clone()).map_err(|_| Error::Invalid)?;
        self.source
            .validate(self.owner_process_id, crate::Limits::default())?;
        self.source.geometry(self.geometry_revision)?;
        StreamPacer::new(self.target_fps).map_err(stream_error)?;
        let rect = self.source.rect();
        let original =
            vw_stream::rgba_bytes(rect.width, rect.height).map_err(stream_error)? as u64 + 4096;
        if original
            .checked_mul(STREAM_RING_FRAMES as u64)
            .is_none_or(|n| n > self.ring_bytes)
            || self.endpoint_bytes < self.ring_bytes
            || rect.width.div_ceil(vw_stream::TILE_SIDE) as u64
                * rect.height.div_ceil(vw_stream::TILE_SIDE) as u64
                > vw_stream::MAX_TILES as u64
        {
            return Err(Error::Limit);
        }
        self.stream_geometry(self.source.observed_ns())?;
        Ok(())
    }
    pub fn stream_geometry(&self, timestamp_ns: u64) -> Result<StreamGeometry> {
        let rect = self.source.rect();
        let (window_handle, monitor_id) = match &self.source {
            PickerSource::Window { target } => (target.window, String::new()),
            PickerSource::Monitor { target } | PickerSource::Region { target, .. } => {
                (0, target.device.clone())
            }
        };
        let wire = v1::CaptureGeometry {
            source_kind: self.source.kind().into(),
            window_handle,
            monitor_id,
            client_rect_host: Some(v1::RectI {
                x: rect.x,
                y: rect.y,
                w: i32::try_from(rect.width).map_err(|_| Error::Limit)?,
                h: i32::try_from(rect.height).map_err(|_| Error::Limit)?,
            }),
            dpi_scale: f64::from(self.source.dpi()) / 96.0,
            geometry_revision: self.geometry_revision,
            timestamp_ns: i64::try_from(timestamp_ns).map_err(|_| Error::Limit)?,
        };
        StreamGeometry::new(&wire, rect.width, rect.height).map_err(stream_error)
    }
    pub fn capture_identity(
        &self,
        frame_id: u64,
        timestamp_ns: u64,
        captured_at_ms: i64,
    ) -> CaptureIdentity {
        let (window_handle, monitor_id) = match &self.source {
            PickerSource::Window { target } => (target.window, String::new()),
            PickerSource::Monitor { target } | PickerSource::Region { target, .. } => {
                (0, target.device.clone())
            }
        };
        CaptureIdentity {
            capture_session_id: self.capture_session_id.clone(),
            frame_id,
            geometry_revision: self.geometry_revision,
            source_kind: self.source.kind().into(),
            client_rect: self.source.rect(),
            dpi_scale: f64::from(self.source.dpi()) / 96.0,
            monotonic_timestamp_ns: timestamp_ns,
            captured_at_ms,
            platform: "windows".into(),
            window_handle,
            monitor_id,
        }
    }
}

/// A native frame is acquired first to learn its actual clock value. The ring
/// reserves its full original before `read_into` can allocate a CPU readback.
pub trait StreamSource {
    /// Liveness/geometry/color/cursor validation only; never obtains a frame.
    fn inspect(&mut self, cancel: &Cancellation) -> Result<()>;
    fn prepare(&mut self, timeout_ms: u64, cancel: &Cancellation) -> Result<Option<CaptureStamp>>;
    fn read_into(&mut self, admitted_rgba: &mut [u8], cancel: &Cancellation) -> Result<()>;
    fn discard(&mut self);
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureStamp {
    pub timestamp_ns: u64,
    pub captured_at_ms: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfferFeedback {
    pub offer_sequence: u64,
    pub admitted_to_media: bool,
}

// One stack-held reply in a bounded worker, not a collection. Its larger
// original owns a linear allocation charge; boxing adds a separate allocation.
#[allow(clippy::large_enum_variant)]
pub enum ProducerReply {
    Idle {
        retained_frames: usize,
        retained_bytes: u64,
    },
    Tiles {
        frame_id: u64,
        timestamp_ns: u64,
        captured_at_ms: i64,
        keyframe: bool,
        payload: BudgetedBytes,
    },
    Original {
        identity: CaptureIdentity,
        encoded: BudgetedEncodedOriginal,
    },
}

pub struct StreamProducer {
    request: StreamRequest,
    budget: SessionBudget,
    cancel: Cancellation,
    ring: FrameRing,
    pacer: StreamPacer,
    previous: Option<OriginalFrame>,
    offered: Option<(u64, Instant, Option<u64>)>,
    metadata: VecDeque<CaptureIdentity>,
    _dto_charge: MemoryCharge,
    last_frame: u64,
    last_timestamp: u64,
}
impl StreamProducer {
    pub fn open(request: StreamRequest, budget: SessionBudget) -> Result<Self> {
        request.validate()?;
        if budget.limit_bytes() != request.endpoint_bytes {
            return Err(Error::Invalid);
        }
        // Request, fixed source geometry, eight small frame identities and
        // the actor's control metadata remain admitted for its whole lifetime.
        let dto_charge = budget
            .reserve(MemoryKind::Foreign, 64 * 1024)
            .map_err(stream_error)?;
        let id = vw_model::Id::try_from(request.capture_session_id.clone())
            .map_err(|_| Error::Invalid)?;
        let geometry = request.stream_geometry(request.source.observed_ns())?;
        let ring = if request.ring_bytes == vw_stream::MAX_RING_BYTES {
            FrameRing::default_budgeted_for(id, geometry, budget.clone())
        } else {
            FrameRing::new_budgeted(
                id,
                geometry,
                STREAM_RING_FRAMES,
                request.ring_bytes,
                budget.clone(),
            )
        }
        .map_err(stream_error)?;
        let pacer = StreamPacer::new(request.target_fps).map_err(stream_error)?;
        Ok(Self {
            request,
            budget,
            cancel: Cancellation::default(),
            ring,
            pacer,
            previous: None,
            offered: None,
            metadata: VecDeque::with_capacity(STREAM_RING_FRAMES),
            _dto_charge: dto_charge,
            last_frame: 0,
            last_timestamp: 0,
        })
    }
    pub fn request(&self) -> &StreamRequest {
        &self.request
    }
    pub fn budget(&self) -> &SessionBudget {
        &self.budget
    }
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    pub fn retained_frames(&self) -> usize {
        self.ring.retained_frames()
    }
    pub fn retained_bytes(&self) -> u64 {
        self.ring.used_bytes()
    }
    fn idle(&self) -> ProducerReply {
        ProducerReply::Idle {
            retained_frames: self.retained_frames(),
            retained_bytes: self.retained_bytes(),
        }
    }

    /// Frozen periods keep exact originals without taking new screenshots.
    pub fn inspect(&self, source: &mut impl StreamSource) -> Result<ProducerReply> {
        self.cancel.check().map_err(stream_error)?;
        source.inspect(&self.cancel)?;
        Ok(self.idle())
    }

    pub fn poll(
        &mut self,
        source: &mut impl StreamSource,
        sequence: u64,
        feedback: Option<OfferFeedback>,
        repair_keyframe: bool,
        now_ms: u64,
    ) -> Result<ProducerReply> {
        self.cancel.check().map_err(stream_error)?;
        match (self.offered.take(), feedback) {
            (Some((expected, _, Some(published_ms))), Some(reply))
                if expected == reply.offer_sequence =>
            {
                self.pacer
                    .complete(published_ms, reply.admitted_to_media)
                    .map_err(stream_error)?
            }
            (None, None) => (),
            _ => return Err(Error::Invalid),
        }
        if repair_keyframe {
            self.pacer.keyframe_required();
        }
        if !self.pacer.can_begin(now_ms) {
            return Ok(self.idle());
        }
        let Some(stamp) = source.prepare(100, &self.cancel)? else {
            return Ok(self.idle());
        };
        if stamp.timestamp_ns <= self.last_timestamp
            || stamp.timestamp_ns < self.request.source.observed_ns()
            || stamp.timestamp_ns > i64::MAX as u64
            || stamp.captured_at_ms < 0
        {
            source.discard();
            return Err(Error::Stale);
        }
        let next = self.last_frame.checked_add(1).ok_or(Error::Limit)?;
        let geometry = self.request.stream_geometry(stamp.timestamp_ns)?;
        let identity = vw_stream::FrameIdentity {
            session: vw_model::Id::try_from(self.request.capture_session_id.clone())
                .map_err(|_| Error::Invalid)?,
            frame_id: next,
            geometry,
            timestamp_ns: i64::try_from(stamp.timestamp_ns).map_err(|_| Error::Limit)?,
        };
        let mut writer: FrameWriter = match self.ring.begin_frame(identity, &self.cancel) {
            Ok(value) => value,
            Err(error) => {
                source.discard();
                return Err(stream_error(error));
            }
        };
        let read = source.read_into(writer.rgba_mut(), &self.cancel);
        source.discard();
        read?;
        let frame = writer.finish(&self.cancel).map_err(stream_error)?;
        self.ring.publish(frame.clone()).map_err(stream_error)?;
        self.last_frame = next;
        self.last_timestamp = stamp.timestamp_ns;
        let capture_identity =
            self.request
                .capture_identity(next, stamp.timestamp_ns, stamp.captured_at_ms);
        capture_identity.validate()?;
        if self.metadata.len() == STREAM_RING_FRAMES {
            self.metadata.pop_front();
        }
        self.metadata.push_back(capture_identity);
        let keyframe = self.pacer.begin(now_ms).ok_or(Error::Busy)?;
        let began = Instant::now();
        let encoded = match vw_stream::encode_tiles_budgeted(
            &frame,
            self.previous.as_ref(),
            keyframe,
            self.pacer.quality(),
            &self.budget,
            &self.cancel,
        )
        .and_then(|tiles| tiles.with_captured_at_ms(stamp.captured_at_ms))
        {
            Ok(value) => value,
            Err(error) => {
                self.pacer.cancelled();
                return Err(stream_error(error));
            }
        };
        let payload = match encoded.to_transfer_bytes(&self.budget, &self.cancel) {
            Ok(value) => value,
            Err(error) => {
                self.pacer.cancelled();
                return Err(stream_error(error));
            }
        };
        self.previous = Some(frame);
        self.offered = Some((sequence, began, None));
        Ok(ProducerReply::Tiles {
            frame_id: next,
            timestamp_ns: stamp.timestamp_ns,
            captured_at_ms: stamp.captured_at_ms,
            keyframe,
            payload,
        })
    }
    /// The helper calls this only after the entire charged MEDIA payload has
    /// actually left its pipe. Feedback later reports endpoint admission.
    pub fn published(&mut self, sequence: u64) -> Result<()> {
        let Some((offered, began, elapsed)) = self.offered.as_mut() else {
            return Err(Error::Invalid);
        };
        if *offered != sequence || elapsed.is_some() {
            return Err(Error::Invalid);
        }
        *elapsed = Some(u64::try_from(began.elapsed().as_millis()).map_err(|_| Error::Limit)?);
        Ok(())
    }
    pub fn original(&self, requested: &CaptureIdentity) -> Result<ProducerReply> {
        self.cancel.check().map_err(stream_error)?;
        requested.validate()?;
        if requested.capture_session_id != self.request.capture_session_id
            || requested.geometry_revision != self.request.geometry_revision
            || !self.metadata.iter().any(|m| same_identity(m, requested))
        {
            return Err(Error::Unavailable);
        }
        let id = vw_model::Id::try_from(self.request.capture_session_id.clone())
            .map_err(|_| Error::Invalid)?;
        let frame = self
            .ring
            .pin(&id, requested.frame_id)
            .map_err(stream_error)?;
        if u64::try_from(frame.identity().timestamp_ns).map_err(|_| Error::Invalid)?
            != requested.monotonic_timestamp_ns
        {
            return Err(Error::Unavailable);
        }
        let encoded = vw_stream::encode_original_budgeted(
            &frame,
            MAX_STREAM_PACKET_BYTES,
            &self.budget,
            &self.cancel,
        )
        .map_err(stream_error)?;
        Ok(ProducerReply::Original {
            identity: requested.clone(),
            encoded,
        })
    }
}

fn same_identity(a: &CaptureIdentity, b: &CaptureIdentity) -> bool {
    a.capture_session_id == b.capture_session_id
        && a.frame_id == b.frame_id
        && a.geometry_revision == b.geometry_revision
        && a.source_kind == b.source_kind
        && a.client_rect == b.client_rect
        && a.dpi_scale == b.dpi_scale
        && a.monotonic_timestamp_ns == b.monotonic_timestamp_ns
        && a.captured_at_ms == b.captured_at_ms
        && a.platform == b.platform
        && a.window_handle == b.window_handle
        && a.monitor_id == b.monitor_id
}
