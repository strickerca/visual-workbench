//! Explicit local publication handles. These APIs transfer pixels from an
//! already-approved local producer; they never select or capture an OS source.
use super::{api::StreamFrameInfo, hub::Lease, order::Source};
use crate::session::{SessionError, SessionResult};
use std::sync::Arc;
use tokio::sync::{Semaphore, oneshot, watch};
use vw_model::Id;
use vw_proto::v1 as pb;

#[derive(Clone, uniffi::Record)]
pub struct CapturePublishSource {
    pub explicit_owner_action: bool,
    pub capture_session_id: String,
    pub source_kind: String,
    pub window_handle: u64,
    pub monitor_id: String,
    pub host_x: i32,
    pub host_y: i32,
    pub width: u32,
    pub height: u32,
    pub dpi_scale: f64,
    pub geometry_revision: u32,
    pub observed_timestamp_ns: i64,
    pub target_fps: u32,
    pub interaction: String,
    pub platform: String,
    pub source_name: String,
}
impl CapturePublishSource {
    pub(super) fn source(&self, generation: u64) -> SessionResult<Source> {
        if !self.explicit_owner_action {
            return Err(SessionError::Declined);
        }
        if self.capture_session_id.len() != 36
            || self.source_kind.len() > 16
            || self.monitor_id.len() > 128
            || self.interaction.len() > 16
            || self.platform.len() > 8
        {
            return Err(SessionError::Invalid);
        }
        let session =
            Id::try_from(self.capture_session_id.clone()).map_err(|_| SessionError::Invalid)?;
        let geometry = pb::CaptureGeometry {
            source_kind: self.source_kind.clone(),
            window_handle: self.window_handle,
            monitor_id: self.monitor_id.clone(),
            client_rect_host: Some(pb::RectI {
                x: self.host_x,
                y: self.host_y,
                w: i32::try_from(self.width).map_err(|_| SessionError::Invalid)?,
                h: i32::try_from(self.height).map_err(|_| SessionError::Invalid)?,
            }),
            dpi_scale: self.dpi_scale,
            geometry_revision: self.geometry_revision,
            timestamp_ns: self.observed_timestamp_ns,
        };
        Source::parse(&pb::StreamControl {
            capture_session_id: Some(session.to_proto()),
            action: "start".into(),
            source_kind: self.source_kind.clone(),
            window_handle: self.window_handle,
            monitor_id: self.monitor_id.clone(),
            max_width: self.width,
            max_height: self.height,
            target_fps: self.target_fps,
            mode: "frame_tiles".into(),
            source_geometry: Some(geometry),
            interaction: self.interaction.clone(),
            source_generation: generation,
            platform: self.platform.clone(),
            source_name: self.source_name.clone(),
        })
    }
}
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct CaptureOriginalRequest {
    pub sequence: u64,
    pub draft_id: String,
    pub frame: StreamFrameInfo,
}
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct CapturePublicationSignal {
    pub sequence: u64,
    pub connection_epoch: u64,
    pub source_generation: u64,
    pub paused: bool,
    pub closed: bool,
    pub repair_sequence: u64,
    pub original: Option<CaptureOriginalRequest>,
    pub failure: Option<String>,
}
impl CapturePublicationSignal {
    pub(super) fn empty(epoch: u64, generation: u64) -> Self {
        Self {
            sequence: 1,
            connection_epoch: epoch,
            source_generation: generation,
            paused: false,
            closed: false,
            repair_sequence: 1,
            original: None,
            failure: None,
        }
    }
}
#[derive(uniffi::Object)]
pub struct CapturePublisher {
    pub(super) lease: Arc<Lease>,
    signal: watch::Receiver<CapturePublicationSignal>,
    input: Arc<Semaphore>,
}
impl CapturePublisher {
    pub(super) fn new(
        lease: Arc<Lease>,
        signal: watch::Receiver<CapturePublicationSignal>,
    ) -> Self {
        Self {
            lease,
            signal,
            input: Arc::new(Semaphore::new(1)),
        }
    }
}
#[uniffi::export]
impl CapturePublisher {
    pub fn signal(&self) -> CapturePublicationSignal {
        let mut value = self.signal.borrow().clone();
        if self.lease.check().is_err() {
            value.closed = true;
        }
        value
    }
    pub async fn wait_signal(
        &self,
        after_sequence: u64,
    ) -> SessionResult<CapturePublicationSignal> {
        self.lease.check()?;
        let _slot = self
            .lease
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| SessionError::Backpressure)?;
        let mut changed = self.signal.clone();
        let mut stopped = self.lease.stopped.subscribe();
        let mut done = self.lease.endpoint.done.subscribe();
        loop {
            self.lease.check()?;
            let value = changed.borrow().clone();
            if value.sequence > after_sequence {
                return Ok(value);
            }
            tokio::select! {result=changed.changed()=>result.map_err(|_|SessionError::Closed)?,_=stopped.changed()=>return Err(SessionError::Closed),_=done.changed()=>return Err(SessionError::Closed),}
        }
    }
    /// The exact bounded helper payload; no Kotlin protobuf parsing or editing.
    /// Returns queue admission feedback for the helper's next Poll. Cancellation
    /// does not release the input permit until the actual native job settles.
    pub async fn publish_frame(&self, encoded: Vec<u8>) -> SessionResult<bool> {
        self.lease.check()?;
        if encoded.is_empty()
            || encoded.len() > vw_proto::capture_admission::MAX_MEDIA_BYTES
            || encoded.capacity() > vw_proto::capture_admission::MAX_MEDIA_BYTES
        {
            return Err(SessionError::Backpressure);
        }
        let permit = self
            .input
            .clone()
            .try_acquire_owned()
            .map_err(|_| SessionError::Backpressure)?;
        let (reply, receive) = oneshot::channel();
        self.lease.endpoint.send(super::hub::Command::Publication(
            super::publication::Command::Frame {
                lease: self.lease.id,
                encoded,
                permit,
                reply,
            },
        ))?;
        receive.await.map_err(|_| SessionError::Closed)?
    }
    /// Submit only the exact helper result requested by signal.original. The
    /// supplied descriptor must match every source/time field before bytes can
    /// be offered to the authenticated peer. The encoded PNG is never altered.
    pub async fn provide_original(
        &self,
        request_sequence: u64,
        identity: crate::CaptureImportDescriptor,
        encoded: Vec<u8>,
    ) -> SessionResult<()> {
        self.lease.check()?;
        if encoded.is_empty()
            || encoded.len() > vw_stream::MAX_LOSSLESS_BYTES
            || encoded.capacity() > vw_stream::MAX_LOSSLESS_BYTES
        {
            return Err(SessionError::Backpressure);
        }
        let permit = self
            .input
            .clone()
            .try_acquire_owned()
            .map_err(|_| SessionError::Backpressure)?;
        let (reply, receive) = oneshot::channel();
        self.lease.endpoint.send(super::hub::Command::Publication(
            super::publication::Command::Original {
                lease: self.lease.id,
                request_sequence,
                identity,
                encoded,
                permit,
                reply,
            },
        ))?;
        receive.await.map_err(|_| SessionError::Closed)?
    }
    pub async fn original_unavailable(&self, request_sequence: u64) -> SessionResult<()> {
        self.lease.check()?;
        let _slot = self
            .lease
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| SessionError::Backpressure)?;
        let (reply, receive) = oneshot::channel();
        self.lease.endpoint.send(super::hub::Command::Publication(
            super::publication::Command::Unavailable {
                lease: self.lease.id,
                request_sequence,
                reply,
            },
        ))?;
        receive.await.map_err(|_| SessionError::Closed)?
    }
    /// Call after the platform producer has stopped/settled. This joins native
    /// admission/verification, not an independently-owned OS capture process.
    pub async fn close_publisher(&self) -> SessionResult<()> {
        self.lease.close().await
    }
}
impl Drop for CapturePublisher {
    fn drop(&mut self) {
        self.lease.stop();
    }
}
