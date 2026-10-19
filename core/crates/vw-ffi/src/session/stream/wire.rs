//! Pure capture-only wire admission. Calling this module cannot capture pixels,
//! read arbitrary assets, publish project history, or grant remote input.
use crate::session::{SessionError, SessionResult};
use vw_model::{AssetId, Id};
use vw_proto::{Message, v1 as pb};
use vw_stream::{FrameIdentity, MAX_LOSSLESS_BYTES, StreamGeometry};

pub(super) const MAX_CAPTURED_AT_MS: i64 = 253_402_300_799_999;
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CaptureStamp {
    pub captured_at_ms: i64,
    pub platform: String,
}
impl CaptureStamp {
    pub fn new(captured_at_ms: i64, platform: &str) -> SessionResult<Self> {
        if !(1..=MAX_CAPTURED_AT_MS).contains(&captured_at_ms)
            || !matches!(platform, "windows" | "android")
        {
            return Err(SessionError::Invalid);
        }
        Ok(Self {
            captured_at_ms,
            platform: platform.into(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Scope {
    pub session: Id,
    pub frame: u64,
    pub geometry_revision: u32,
    pub request: Id,
    pub transfer: Id,
    pub generation: u64,
}
impl Scope {
    pub fn parse(value: &pb::CaptureTransferKey) -> SessionResult<Self> {
        if value.frame_id == 0 || value.geometry_revision == 0 || value.generation == 0 {
            return Err(SessionError::Invalid);
        }
        Ok(Self {
            session: id(value.capture_session_id.as_ref())?,
            frame: value.frame_id,
            geometry_revision: value.geometry_revision,
            request: id(value.request_id.as_ref())?,
            transfer: id(value.transfer_id.as_ref())?,
            generation: value.generation,
        })
    }
    pub fn wire(&self) -> pb::CaptureTransferKey {
        pb::CaptureTransferKey {
            capture_session_id: Some(self.session.to_proto()),
            frame_id: self.frame,
            geometry_revision: self.geometry_revision,
            request_id: Some(self.request.to_proto()),
            transfer_id: Some(self.transfer.to_proto()),
            generation: self.generation,
        }
    }
    pub fn matches(&self, frame: &FrameIdentity, request: &Id) -> bool {
        self.session == frame.session
            && self.frame == frame.frame_id
            && self.geometry_revision == frame.geometry.revision()
            && &self.request == request
    }
    pub fn envelope(&self, body: pb::stream_blob::Body) -> pb::StreamBlob {
        pb::StreamBlob {
            key: Some(self.wire()),
            body: Some(body),
        }
    }
    pub fn check<'a>(&self, value: &'a pb::StreamBlob) -> SessionResult<&'a pb::stream_blob::Body> {
        if value.encoded_len() > 128 * 1024 - 1024
            || Self::parse(value.key.as_ref().ok_or(SessionError::Invalid)?)? != *self
        {
            return Err(SessionError::Invalid);
        }
        value.body.as_ref().ok_or(SessionError::Invalid)
    }
}

/// Admitted metadata for an exact transient original; not a canonical Asset.
/// It is constructed only after matching a pending locally admitted request.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct OriginalOffer {
    pub scope: Scope,
    pub frame: FrameIdentity,
    pub asset: AssetId,
    pub bytes: u64,
    pub stamp: CaptureStamp,
}
impl OriginalOffer {
    pub fn parse(
        value: &pb::LosslessFrame,
        expected: &FrameIdentity,
        stamp: &CaptureStamp,
        request: &Id,
        generation: u64,
    ) -> SessionResult<Option<Self>> {
        if id(value.capture_session_id.as_ref())? != expected.session
            || value.frame_id != expected.frame_id
            || id(value.request_id.as_ref())? != *request
            || !value.data.is_empty()
            || value.encoded_len() > 1024
        {
            return Err(SessionError::Invalid);
        }
        if !value.available {
            if value.geometry.is_some()
                || !value.format.is_empty()
                || !value.asset_id.is_empty()
                || value.transfer.is_some()
                || value.total_size != 0
                || value.captured_at_ms != 0
            {
                return Err(SessionError::Invalid);
            }
            return Ok(None);
        }
        let geometry = StreamGeometry::new(
            value.geometry.as_ref().ok_or(SessionError::Invalid)?,
            expected.geometry.width(),
            expected.geometry.height(),
        )
        .map_err(|_| SessionError::Invalid)?;
        if geometry != expected.geometry
            || value.captured_at_ms != stamp.captured_at_ms
            || CaptureStamp::new(value.captured_at_ms, &stamp.platform)? != *stamp
            || value.format != "png"
            || value.total_size == 0
            || value.total_size > MAX_LOSSLESS_BYTES as u64
        {
            return Err(SessionError::Invalid);
        }
        let scope = Scope::parse(value.transfer.as_ref().ok_or(SessionError::Invalid)?)?;
        if generation == 0 || scope.generation != generation || !scope.matches(expected, request) {
            return Err(SessionError::Invalid);
        }
        Ok(Some(Self {
            scope,
            frame: expected.clone(),
            asset: AssetId::try_from(value.asset_id.clone()).map_err(|_| SessionError::Invalid)?,
            bytes: value.total_size,
            stamp: stamp.clone(),
        }))
    }
    pub fn wire(&self) -> pb::LosslessFrame {
        pb::LosslessFrame {
            capture_session_id: Some(self.frame.session.to_proto()),
            frame_id: self.frame.frame_id,
            geometry: Some(self.frame.geometry.wire().clone()),
            available: true,
            format: "png".into(),
            data: Vec::new(),
            asset_id: self.asset.to_string(),
            transfer: Some(self.scope.wire()),
            total_size: self.bytes,
            request_id: Some(self.scope.request.to_proto()),
            captured_at_ms: self.stamp.captured_at_ms,
        }
    }
    pub fn request(&self) -> pb::StreamBlob {
        self.scope
            .envelope(pb::stream_blob::Body::Request(pb::BlobRequest {
                asset_id: self.asset.to_string(),
                offset: 0,
                length: self.bytes,
                priority: 0,
            }))
    }
}

pub(super) fn unavailable(frame: &FrameIdentity, request: &Id) -> pb::LosslessFrame {
    pb::LosslessFrame {
        capture_session_id: Some(frame.session.to_proto()),
        frame_id: frame.frame_id,
        request_id: Some(request.to_proto()),
        ..Default::default()
    }
}
pub(super) fn id(value: Option<&pb::Uuid>) -> SessionResult<Id> {
    Id::from_proto(value).map_err(|_| SessionError::Invalid)
}

/// Metadata and shape admission before queueing a frame. Exact JPEG checks and
/// reconstruction happen on the bounded codec worker, never the network loop.
pub(super) fn frame_identity(value: &pb::FrameTiles) -> SessionResult<FrameIdentity> {
    if value.delivery_sequence == 0
        || value.source_generation == 0
        || !(1..=MAX_CAPTURED_AT_MS).contains(&value.captured_at_ms)
    {
        return Err(SessionError::Invalid);
    }
    if value.tiles.len() > vw_stream::MAX_TILES
        || value.encoded_len() > vw_stream::MAX_TILE_BYTES + 32 * 1024
    {
        return Err(SessionError::Backpressure);
    }
    packet_capacity(value)?;
    let frame = FrameIdentity {
        session: id(value.capture_session_id.as_ref())?,
        frame_id: value.frame_id,
        geometry: StreamGeometry::new(
            value.geometry.as_ref().ok_or(SessionError::Invalid)?,
            value.width,
            value.height,
        )
        .map_err(|_| SessionError::Invalid)?,
        timestamp_ns: value.timestamp_ns,
    };
    frame.validate().map_err(|_| SessionError::Invalid)?;
    let mut encoded = 0usize;
    for tile in &value.tiles {
        let r = tile.rect.as_ref().ok_or(SessionError::Invalid)?;
        if tile.format != "jpeg"
            || r.x < 0
            || r.y < 0
            || r.w <= 0
            || r.h <= 0
            || r.w > 512
            || r.h > 512
            || r.x.checked_add(r.w).is_none_or(|v| v > value.width as i32)
            || r.y.checked_add(r.h).is_none_or(|v| v > value.height as i32)
        {
            return Err(SessionError::Invalid);
        }
        encoded = encoded
            .checked_add(tile.data.len())
            .ok_or(SessionError::Backpressure)?;
        if encoded > vw_stream::MAX_TILE_BYTES {
            return Err(SessionError::Backpressure);
        }
    }
    Ok(frame)
}

/// Retained packet capacity, including in-process spare capacity. This check
/// applies before the packet enters the fixed queued-packet reservation.
pub(super) fn packet_capacity(packet: &pb::FrameTiles) -> SessionResult<u64> {
    if packet.tiles.len() > vw_stream::MAX_TILES {
        return Err(SessionError::Backpressure);
    }
    let mut size = 4096u64
        .checked_add(
            (packet.tiles.capacity() as u64)
                .checked_mul(std::mem::size_of::<pb::TileImage>() as u64)
                .ok_or(SessionError::Backpressure)?,
        )
        .ok_or(SessionError::Backpressure)?;
    if let Some(id) = &packet.capture_session_id {
        size = size
            .checked_add(id.value.capacity() as u64)
            .ok_or(SessionError::Backpressure)?;
    }
    if let Some(geometry) = &packet.geometry {
        size = size
            .checked_add(geometry.source_kind.capacity() as u64)
            .and_then(|v| v.checked_add(geometry.monitor_id.capacity() as u64))
            .ok_or(SessionError::Backpressure)?;
    }
    for tile in &packet.tiles {
        size = size
            .checked_add(tile.data.capacity() as u64)
            .and_then(|v| v.checked_add(tile.format.capacity() as u64))
            .ok_or(SessionError::Backpressure)?;
    }
    if size > vw_stream::MAX_TILE_BYTES as u64 + 64 * 1024 {
        return Err(SessionError::Backpressure);
    }
    Ok(size)
}
