//! Explicit local WGC acquisition owned by the authenticated publisher. The
//! helper, parser and retained packets use the publisher's exact parented pool.
//! Pixels never round-trip through a foreign byte-array or application parser.
mod contract;
#[cfg(test)]
#[path = "local_producer/pump_tests.rs"]
mod pump_tests;
mod worker;
use super::{
    hub::Lease,
    order::Source,
    publication_api::{CaptureOriginalRequest, CapturePublisher},
};
use crate::session::{SessionError, SessionResult};
use contract::{OpenProvider, Work};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::Semaphore;
use vw_capture::{
    picker::{MonitorTarget, PickerSource},
    stream::{OfferFeedback, StreamRequest},
    stream_wire::Header,
};
use vw_stream::SessionBudget;
pub(in crate::session) use worker::Worker;

#[derive(Clone, Debug, uniffi::Record)]
pub struct CaptureLocalRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl CaptureLocalRect {
    fn native(self) -> vw_capture::Rect {
        vw_capture::Rect {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct CaptureLocalWindow {
    pub window: u64,
    pub process_id: u32,
    pub process_created: u64,
    pub client: CaptureLocalRect,
    pub frame: CaptureLocalRect,
    pub dpi: u32,
    pub observed_ns: u64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct CaptureLocalMonitor {
    pub monitor: u64,
    pub device: String,
    pub physical: CaptureLocalRect,
    pub dpi: u32,
    pub rotation: u32,
    pub primary: bool,
    pub observed_ns: u64,
}
impl CaptureLocalMonitor {
    fn native(self) -> MonitorTarget {
        MonitorTarget {
            monitor: self.monitor,
            device: self.device,
            physical: self.physical.native(),
            dpi: self.dpi,
            rotation: self.rotation,
            primary: self.primary,
            observed_ns: self.observed_ns,
        }
    }
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum CaptureLocalSelection {
    Window {
        target: CaptureLocalWindow,
    },
    Monitor {
        target: CaptureLocalMonitor,
    },
    Region {
        target: CaptureLocalMonitor,
        region: CaptureLocalRect,
    },
}
impl CaptureLocalSelection {
    fn native(self) -> PickerSource {
        match self {
            Self::Window { target } => PickerSource::Window {
                target: vw_capture::WindowTarget {
                    window: target.window,
                    process_id: target.process_id,
                    process_created: target.process_created,
                    client: target.client.native(),
                    frame: target.frame.native(),
                    dpi: target.dpi,
                    observed_ns: target.observed_ns,
                },
            },
            Self::Monitor { target } => PickerSource::Monitor {
                target: target.native(),
            },
            Self::Region { target, region } => PickerSource::Region {
                target: target.native(),
                region: region.native(),
            },
        }
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct CaptureLocalOptions {
    pub explicit_owner_action: bool,
    pub selection: CaptureLocalSelection,
    /// Path/hash come only from the admitted packaged runtime inventory.
    pub helper_path: String,
    pub trusted_helper_sha256: String,
    pub ring_bytes: u64,
}
#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum CaptureLocalProgress {
    Idle,
    Tiles { frame_id: u64, admitted: bool },
    OriginalProvided { frame_id: u64 },
    OriginalUnavailable,
}

pub(super) fn map_capture(error: vw_capture::Error) -> SessionError {
    use vw_capture::Error as E;
    match error {
        E::Invalid => SessionError::Invalid,
        E::Consent => SessionError::Declined,
        E::Limit | E::Busy => SessionError::Backpressure,
        E::Cancelled => SessionError::Cancelled,
        E::Timeout => SessionError::Timeout,
        E::Storage => SessionError::Storage,
        E::Stale
        | E::OwnWindow
        | E::Unavailable
        | E::ColorDepth
        | E::CodecMissing
        | E::Unsupported => SessionError::RemoteUnavailable,
        E::Platform => SessionError::Worker,
    }
}
pub(super) fn start(
    lease: Weak<Lease>,
    source: &Source,
    options: CaptureLocalOptions,
    pool: SessionBudget,
    cancellation: Arc<crate::Cancellation>,
) -> SessionResult<(Arc<Worker>, StreamRequest)> {
    if !options.explicit_owner_action {
        return Err(SessionError::Declined);
    }
    if options.helper_path.is_empty()
        || options.helper_path.len() > 4096
        || options.helper_path.contains('\0')
        || options.trusted_helper_sha256.len() != 64
        || !options
            .trusted_helper_sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return Err(SessionError::Invalid);
    }
    let owner = lease.upgrade().ok_or(SessionError::Closed)?;
    owner.check()?;
    let request = StreamRequest {
        explicit_owner_action: true,
        owner_process_id: std::process::id(),
        capture_session_id: source.session.to_string(),
        connection_epoch: owner.endpoint.epoch,
        source_generation: source.generation,
        source: options.selection.native(),
        geometry_revision: source.geometry.wire().geometry_revision,
        target_fps: source.fps,
        endpoint_bytes: vw_capture::stream::PROCESS_MEMORY_BYTES,
        ring_bytes: options.ring_bytes,
    };
    request.validate().map_err(map_capture)?;
    let actual = request
        .stream_geometry(request.source.observed_ns())
        .map_err(map_capture)?;
    if source.platform != "windows"
        || actual.wire() != source.geometry.wire()
        || actual.width() != source.geometry.width()
        || actual.height() != source.geometry.height()
    {
        return Err(SessionError::Invalid);
    }
    let guard_source = match &request.source {
        PickerSource::Window { target } => crate::AndroidTunnelSource::Window {
            window: target.window,
        },
        PickerSource::Monitor { target } => crate::AndroidTunnelSource::Monitor {
            monitor: target.monitor,
        },
        PickerSource::Region { target, region } => crate::AndroidTunnelSource::Region {
            monitor: target.monitor,
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
        },
    };
    let mirror_guard = crate::begin_android_tunnel_capture_guard(guard_source)
        .map_err(|_| SessionError::RemoteUnavailable)?;
    let current: contract::CurrentCheck = Arc::new(move || {
        if cancellation.is_cancelled() {
            return Err(vw_capture::Error::Cancelled);
        }
        mirror_guard.check().map_err(|error| match error {
            crate::AndroidTunnelError::FeedbackLoop => vw_capture::Error::OwnWindow,
            crate::AndroidTunnelError::Stale => vw_capture::Error::Stale,
            _ => vw_capture::Error::Unavailable,
        })?;
        lease
            .upgrade()
            .ok_or(vw_capture::Error::Cancelled)?
            .check()
            .map_err(|_| vw_capture::Error::Cancelled)
    });
    let open = platform_open(options.helper_path, options.trusted_helper_sha256);
    #[cfg(test)]
    let open = pump_tests::take_open().unwrap_or(open);
    let worker =
        Worker::start(request.clone(), pool.clone(), current, open).map_err(map_capture)?;
    if !worker.pool().same_pool(&pool) {
        worker.cancel();
        return Err(SessionError::Worker);
    }
    Ok((worker, request))
}
#[cfg(windows)]
fn platform_open(path: String, hash: String) -> OpenProvider {
    struct WindowsProvider(vw_capture::stream_process::StreamProcess);
    impl contract::Provider for WindowsProvider {
        fn exchange(
            &mut self,
            command: vw_capture::stream_wire::Command,
            cancel: &vw_stream::Cancellation,
        ) -> vw_capture::Result<vw_capture::stream_wire::Packet> {
            self.0.exchange(command, cancel)
        }
        fn close(&mut self) -> vw_capture::Result<()> {
            self.0.close()
        }
    }
    Box::new(move |request, pool, cancel| {
        let helper = Arc::new(vw_capture::stream_process::PinnedStreamHelper::open(
            std::path::Path::new(&path),
            &hash,
        )?);
        // This is the production caller: no independent host-library pool.
        Ok(Box::new(WindowsProvider(
            vw_capture::stream_process::StreamProcess::open_budgeted(
                helper,
                request.clone(),
                pool.clone(),
                cancel,
            )?,
        )))
    })
}
#[cfg(not(windows))]
fn platform_open(_path: String, _hash: String) -> OpenProvider {
    Box::new(|_, _, _| Err(vw_capture::Error::Unsupported))
}

struct PumpState {
    feedback: Option<OfferFeedback>,
    repair: u64,
    original: u64,
}
#[derive(uniffi::Object)]
pub struct CaptureLocalProducer {
    worker: Arc<Worker>,
    request: StreamRequest,
    publisher: Arc<CapturePublisher>,
    slot: Semaphore,
    state: Mutex<PumpState>,
}
impl CaptureLocalProducer {
    pub(super) fn new(
        worker: Arc<Worker>,
        request: StreamRequest,
        publisher: Arc<CapturePublisher>,
    ) -> Self {
        Self {
            worker,
            request,
            publisher,
            slot: Semaphore::new(1),
            state: Mutex::new(PumpState {
                feedback: None,
                repair: 0,
                original: 0,
            }),
        }
    }
    pub(super) async fn ready(&self) -> SessionResult<()> {
        self.worker.ready().await.map_err(map_capture)
    }
    fn original_identity(
        &self,
        request: &CaptureOriginalRequest,
    ) -> SessionResult<vw_capture::FrameIdentity> {
        let frame = &request.frame;
        let identity = self.request.capture_identity(
            frame.frame_id,
            u64::try_from(frame.timestamp_ns).map_err(|_| SessionError::Invalid)?,
            frame.captured_at_ms,
        );
        if frame.capture_session_id != identity.capture_session_id
            || frame.source_generation != self.request.source_generation
            || frame.geometry_revision != identity.geometry_revision
            || frame.host_x != identity.client_rect.x
            || frame.host_y != identity.client_rect.y
            || frame.width != identity.client_rect.width
            || frame.height != identity.client_rect.height
            || frame.dpi_scale != identity.dpi_scale
            || frame.platform != identity.platform
        {
            return Err(SessionError::Invalid);
        }
        Ok(identity)
    }
}
// Dropping an entered native future seals the producer; no lost feedback can
// turn into a successor Poll or reuse a now-ambiguous local offer.
struct PumpGuard<'a> {
    worker: &'a Worker,
    settled: bool,
}
impl Drop for PumpGuard<'_> {
    fn drop(&mut self) {
        if !self.settled {
            self.worker.cancel();
        }
    }
}
#[uniffi::export]
impl CaptureLocalProducer {
    /// One actual helper exchange and native publication. Frozen sessions only
    /// inspect the approved source, preserving the original ring without capture.
    pub async fn pump_once(&self) -> SessionResult<CaptureLocalProgress> {
        let _slot = self
            .slot
            .try_acquire()
            .map_err(|_| SessionError::Backpressure)?;
        self.publisher.lease.check()?;
        let signal = self.publisher.signal();
        if signal.closed
            || signal.connection_epoch != self.request.connection_epoch
            || signal.source_generation != self.request.source_generation
        {
            return Err(SessionError::Closed);
        }
        let mut guard = PumpGuard {
            worker: &self.worker,
            settled: false,
        };
        let requested = signal
            .original
            .filter(|r| self.state.lock().is_ok_and(|s| s.original != r.sequence));
        let work = if let Some(ref original) = requested {
            Work::Original {
                identity: self.original_identity(original)?,
            }
        } else if signal.paused {
            Work::InspectSource
        } else {
            let mut state = self.state.lock().map_err(|_| SessionError::Worker)?;
            Work::Poll {
                feedback: state.feedback.take(),
                repair: state.repair != signal.repair_sequence,
            }
        };
        let packet = self.worker.exchange(work).await.map_err(map_capture)?;
        let result = match &packet.header {
            Header::Idle { .. } => CaptureLocalProgress::Idle,
            Header::Tiles {
                sequence, frame_id, ..
            } => {
                // Publisher's sole input permit reserves its complete incoming
                // allocation before copying. Packet keeps its parser/bytes charge
                // until native admission settles; no foreign pixel copy exists.
                let admitted = self
                    .publisher
                    .publish_local_frame(packet.payload.bytes())
                    .await?;
                let mut state = self.state.lock().map_err(|_| SessionError::Worker)?;
                state.feedback = Some(OfferFeedback {
                    offer_sequence: *sequence,
                    admitted_to_media: admitted,
                });
                if admitted {
                    state.repair = signal.repair_sequence;
                }
                CaptureLocalProgress::Tiles {
                    frame_id: *frame_id,
                    admitted,
                }
            }
            Header::Original {
                identity, asset_id, ..
            } => {
                let request = requested.as_ref().ok_or(SessionError::Invalid)?;
                if *identity != self.original_identity(request)? {
                    return Err(SessionError::Invalid);
                }
                let d = crate::CaptureImportDescriptor {
                    explicit_owner_action: true,
                    capture_session_id: identity.capture_session_id.clone(),
                    frame_id: identity.frame_id,
                    geometry_revision: identity.geometry_revision,
                    platform: identity.platform.clone(),
                    source_kind: identity.source_kind.clone(),
                    physical_x: identity.client_rect.x,
                    physical_y: identity.client_rect.y,
                    width: identity.client_rect.width,
                    height: identity.client_rect.height,
                    dpi_scale: identity.dpi_scale,
                    monotonic_timestamp_ns: identity.monotonic_timestamp_ns,
                    captured_at_ms: identity.captured_at_ms,
                    window_handle: identity.window_handle,
                    monitor_id: identity.monitor_id.clone(),
                    expected_source_asset_id: Some(asset_id.clone()),
                };
                self.publisher
                    .provide_local_original(request.sequence, d, packet.payload.bytes())
                    .await?;
                self.state
                    .lock()
                    .map_err(|_| SessionError::Worker)?
                    .original = request.sequence;
                CaptureLocalProgress::OriginalProvided {
                    frame_id: identity.frame_id,
                }
            }
            Header::Refused {
                error: vw_capture::Error::Unavailable,
                ..
            } if requested.is_some() => {
                let sequence = requested.as_ref().ok_or(SessionError::Invalid)?.sequence;
                self.publisher.original_unavailable(sequence).await?;
                self.state
                    .lock()
                    .map_err(|_| SessionError::Worker)?
                    .original = sequence;
                CaptureLocalProgress::OriginalUnavailable
            }
            Header::Refused { error, .. } => return Err(map_capture(*error)),
            _ => return Err(SessionError::Invalid),
        };
        self.publisher.lease.check()?;
        guard.settled = true;
        Ok(result)
    }
    pub fn cancel(&self) {
        self.worker.cancel();
    }
    /// Completion includes the actual blocking thread/process retirement.
    pub async fn close_producer(&self) -> SessionResult<()> {
        self.worker.close().await.map_err(map_capture)
    }
}
impl Drop for CaptureLocalProducer {
    fn drop(&mut self) {
        self.worker.cancel();
    }
}
