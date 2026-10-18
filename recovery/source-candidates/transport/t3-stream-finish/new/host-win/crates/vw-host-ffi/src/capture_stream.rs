//! Child of capture.rs. One explicitly selected persistent producer; no OS
//! source is inferred from a peer request. The worker owns the helper until it
//! has exited, and a returned packet keeps the sole output admission alive.
use super::*;
use capture::{stream::{StreamCancellation, StreamRequest}, stream_wire::{Command, Feedback, Header, Packet}};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot, watch};

#[path = "capture_stream/owner.rs"] mod owner_thread;
#[cfg(test)] #[path = "capture_stream/tests.rs"] mod tests;

// A helper response owns at most 64 MiB. Admission also reserves two copies
// while UniFFI serializes/transfers it, plus bounded header/owner overhead.
// JVM/GPU copies retained by the caller are a separate platform budget.
pub const HOST_STREAM_BUFFER_BYTES: u64 = 193 * 1024 * 1024;
static OUTPUT_HELD: AtomicBool = AtomicBool::new(false);
struct OutputPermit;
impl OutputPermit {
    fn acquire() -> capture::Result<Self> {
        OUTPUT_HELD.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| capture::Error::Busy)?;
        Ok(Self)
    }
}
impl Drop for OutputPermit { fn drop(&mut self) { OUTPUT_HELD.store(false, Ordering::Release); } }

#[derive(Clone, Debug, uniffi::Record)]
pub struct HostCaptureStreamOptions {
    pub explicit_owner_action: bool,
    pub source: super::CaptureSelection,
    pub capture_session_id: String,
    pub geometry_revision: u32,
    pub target_fps: u32,
    pub ring_frames: u32,
    pub ring_bytes: u64,
    pub helper_memory_bytes: u64,
    pub host_buffer_bytes: u64,
}
impl HostCaptureStreamOptions {
    fn request(self) -> capture::Result<StreamRequest> {
        if self.host_buffer_bytes < HOST_STREAM_BUFFER_BYTES || self.host_buffer_bytes > 256 * 1024 * 1024 {
            return Err(capture::Error::Limit);
        }
        let request = StreamRequest { explicit_owner_action: self.explicit_owner_action,
            owner_process_id: super::owner(), source: self.source.into(), capture_session_id: self.capture_session_id,
            geometry_revision: self.geometry_revision, target_fps: self.target_fps,
            ring_frames: usize::try_from(self.ring_frames).map_err(|_| capture::Error::Limit)?,
            ring_bytes: self.ring_bytes, memory_bytes: self.helper_memory_bytes };
        request.validate()?;
        Ok(request)
    }
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct HostStreamOriginalIdentity {
    pub capture_session_id: String, pub frame_id: u64, pub geometry_revision: u32,
    pub source_kind: String, pub physical_x: i32, pub physical_y: i32,
    pub width: u32, pub height: u32, pub dpi_scale: f64,
    pub timestamp_ns: u64, pub captured_at_ms: i64, pub platform: String,
    pub window_handle: u64, pub monitor_id: String, pub source_asset_id: String,
}
impl HostStreamOriginalIdentity {
    fn new(value: capture::FrameIdentity, source_asset_id: String) -> Self {
        Self { capture_session_id: value.capture_session_id, frame_id: value.frame_id,
            geometry_revision: value.geometry_revision, source_kind: value.source_kind,
            physical_x: value.client_rect.x, physical_y: value.client_rect.y,
            width: value.client_rect.width, height: value.client_rect.height, dpi_scale: value.dpi_scale,
            timestamp_ns: value.monotonic_timestamp_ns, captured_at_ms: value.captured_at_ms,
            platform: value.platform, window_handle: value.window_handle, monitor_id: value.monitor_id,
            source_asset_id }
    }
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum HostStreamPacketInfo {
    Tiles { offer_sequence: u64, frame_id: u64, timestamp_ns: i64, encoded_bytes: u32 },
    Original { identity: HostStreamOriginalIdentity, encoded_bytes: u32 },
    OriginalUnavailable,
    Idle { retained_frames: u32, retained_bytes: u64 },
}
#[derive(uniffi::Object)]
pub struct HostStreamPacket {
    info: HostStreamPacketInfo,
    payload: Mutex<Option<Vec<u8>>>,
    _input: OwnedSemaphorePermit,
    _output: OutputPermit,
}
impl HostStreamPacket {
    fn new(packet: Packet, input: OwnedSemaphorePermit, output: OutputPermit) -> capture::Result<Arc<Self>> {
        let info = match packet.header {
            Header::Tiles { sequence, frame_id, timestamp_ns, payload_bytes } =>
                HostStreamPacketInfo::Tiles { offer_sequence: sequence, frame_id, timestamp_ns, encoded_bytes: payload_bytes },
            Header::Original { identity, asset_id, payload_bytes, .. } =>
                HostStreamPacketInfo::Original { identity: HostStreamOriginalIdentity::new(identity, asset_id), encoded_bytes: payload_bytes },
            Header::Idle { retained_frames, retained_bytes, .. } => HostStreamPacketInfo::Idle {
                retained_frames: u32::try_from(retained_frames).map_err(|_| capture::Error::Limit)?, retained_bytes },
            Header::Refused { error: capture::Error::Unavailable, .. } => HostStreamPacketInfo::OriginalUnavailable,
            Header::Refused { error, .. } => return Err(error),
            _ => return Err(capture::Error::Invalid),
        };
        Ok(Arc::new(Self { info, payload: Mutex::new(Some(packet.payload)), _input: input, _output: output }))
    }
}
#[uniffi::export]
impl HostStreamPacket {
    pub fn info(&self) -> HostStreamPacketInfo { self.info.clone() }
    /// One transfer, not a duplicating read. Keep this native handle alive
    /// through publication in vw-core, then destroy it in joined cleanup.
    /// The foreign recipient must release its own ByteArray before next poll.
    pub fn take_payload(&self) -> Result<Vec<u8>> {
        self.payload.try_lock().map_err(|_| failure(capture::Error::Busy))?
            .take().ok_or_else(|| failure(capture::Error::Unavailable))
    }
}

struct Shared {
    cancel: Arc<StreamCancellation>, closing: AtomicBool,
    done: watch::Sender<Option<capture::Result<()>>>, input: Arc<Semaphore>,
}
impl Shared {
    fn check(&self) -> capture::Result<()> {
        self.cancel.check()?;
        if self.closing.load(Ordering::Acquire) || self.done.borrow().is_some() { return Err(capture::Error::Unavailable); }
        Ok(())
    }
    fn cancel(&self) { self.closing.store(true, Ordering::Release); self.cancel.cancel(); }
}
enum Work {
    Poll { feedback: Option<Feedback>, repair: bool },
    Original { capture_session_id: String, frame_id: u64 },
    Heartbeat,
}
struct Request {
    work: Work, input: OwnedSemaphorePermit, output: OutputPermit,
    reply: oneshot::Sender<capture::Result<Arc<HostStreamPacket>>>,
}
struct CancelOnDrop { shared: Arc<Shared>, armed: bool }
impl Drop for CancelOnDrop { fn drop(&mut self) { if self.armed { self.shared.cancel(); } } }

#[derive(uniffi::Object)]
pub struct HostCaptureStream { shared: Arc<Shared>, send: std::sync::mpsc::SyncSender<Request> }
impl Drop for HostCaptureStream { fn drop(&mut self) { self.shared.cancel(); } }
impl HostCaptureStream {
    async fn request(&self, work: Work) -> Result<Arc<HostStreamPacket>> {
        self.shared.check().map_err(failure)?;
        let input = self.shared.input.clone().try_acquire_owned().map_err(|_| failure(capture::Error::Busy))?;
        let output = OutputPermit::acquire().map_err(failure)?;
        let (reply, receive) = oneshot::channel();
        self.shared.check().map_err(failure)?;
        self.send.try_send(Request { work, input, output, reply }).map_err(|_| failure(capture::Error::Busy))?;
        let mut guard = CancelOnDrop { shared: self.shared.clone(), armed: true };
        let result = receive.await.map_err(|_| failure(capture::Error::Platform))?;
        guard.armed = false;
        result.map_err(failure)
    }
}
#[uniffi::export]
impl HostCaptureStream {
    pub async fn poll(&self, last_offer_sequence: u64, admitted_to_media: bool, repair: bool) -> Result<Arc<HostStreamPacket>> {
        let feedback = if last_offer_sequence == 0 {
            if admitted_to_media { return Err(failure(capture::Error::Invalid)); }
            None
        } else { Some(Feedback { offer_sequence: last_offer_sequence, admitted_to_media }) };
        self.request(Work::Poll { feedback, repair }).await
    }
    pub async fn original(&self, capture_session_id: String, frame_id: u64) -> Result<Arc<HostStreamPacket>> {
        if capture_session_id.len() != 36 || frame_id == 0 { return Err(failure(capture::Error::Invalid)); }
        vw_model::Id::try_from(capture_session_id.clone()).map_err(|_| failure(capture::Error::Invalid))?;
        self.request(Work::Original { capture_session_id, frame_id }).await
    }
    /// Paused sources still check liveness without generating frames.
    pub async fn heartbeat(&self) -> Result<Arc<HostStreamPacket>> { self.request(Work::Heartbeat).await }
    /// Signals only this owned helper. It does not claim cleanup has finished.
    pub fn cancel(&self) { self.shared.cancel(); }
    /// Seal admission, stop the helper normally when possible, and join actual
    /// process/pipe cleanup. Cancellation of this waiter never abandons cleanup.
    pub async fn close_host_stream(&self) -> Result<()> {
        self.shared.closing.store(true, Ordering::Release);
        let mut done = self.shared.done.subscribe();
        loop {
            if let Some(result) = *done.borrow() { return result.map_err(failure); }
            done.changed().await.map_err(|_| failure(capture::Error::Platform))?;
        }
    }
}
#[uniffi::export]
impl CaptureService {
    pub async fn open_capture_stream(&self, options: HostCaptureStreamOptions) -> Result<Arc<HostCaptureStream>> {
        let request = options.request().map_err(failure)?;
        let helper = self.helper.clone();
        let (stream, opened) = owner_thread::start(move |cancel| {
            capture::stream_process::StreamProcess::open(helper, request, cancel)
        }).map_err(failure)?;
        // The handle already exists locally: dropping this suspended open seals
        // the worker even if the helper becomes ready at the same instant.
        opened.await.map_err(|_| failure(capture::Error::Platform))?.map_err(failure)?;
        stream.shared.check().map_err(failure)?;
        Ok(stream)
    }
}
