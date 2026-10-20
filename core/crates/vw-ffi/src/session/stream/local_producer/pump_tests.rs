//! Synthetic only at the helper boundary: all acquisition, packet admission,
//! publication jobs, freeze requests and retirement use the actual native owners.
#![allow(clippy::expect_used, clippy::panic)]
use super::super::{
    driver::Bridge,
    hub::Hub,
    publication_api::{CapturePublishSource, CapturePublisher},
    receiver_tests::{BUDGET, WORKERS, id, identity, packet, source, stamp},
    verified_tests::{fixture, root},
    wire::OriginalOffer,
};
use super::{
    CaptureLocalOptions, CaptureLocalProducer, CaptureLocalProgress, CaptureLocalRect,
    CaptureLocalSelection, CaptureLocalWindow,
    contract::{OpenProvider, Provider},
    worker,
};
use crate::session::SessionError;
use std::{
    future::{Future, poll_fn},
    io::{Cursor, Read, Seek, SeekFrom},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::Poll,
    time::Duration,
};
use vw_capture::{
    Result as CaptureResult,
    stream::{StreamRequest, stream_error},
    stream_wire::{self, Command, Header, Packet},
};
use vw_proto::{
    Message,
    v1::{self as pb, envelope::Body},
};
use vw_stream::{BudgetedBytes, Cancellation, MemoryKind, SessionBudget};

const DEADLINE: Duration = Duration::from_secs(5);
static OPEN: Mutex<Option<OpenProvider>> = Mutex::new(None);

pub(super) fn take_open() -> Option<OpenProvider> {
    OPEN.lock().expect("test provider lock").take()
}
struct Injection;
impl Injection {
    fn install(open: OpenProvider) -> Self {
        let mut pending = OPEN.lock().expect("test provider lock");
        assert!(pending.is_none(), "one helper factory per fixture");
        *pending = Some(open);
        Self
    }
}
impl Drop for Injection {
    fn drop(&mut self) {
        OPEN.lock().expect("test provider lock").take();
    }
}

#[derive(Default)]
struct Probe {
    commands: Mutex<Vec<Command>>,
    pool: Mutex<Option<SessionBudget>>,
    opens: AtomicUsize,
    closes: AtomicUsize,
    drops: AtomicUsize,
    unretired_drops: AtomicUsize,
}
struct Helper {
    probe: Arc<Probe>,
    request: StreamRequest,
    pool: SessionBudget,
    original: BudgetedBytes,
    frame: u64,
    retired: bool,
}
impl Provider for Helper {
    fn exchange(&mut self, command: Command, cancel: &Cancellation) -> CaptureResult<Packet> {
        cancel.check().map_err(stream_error)?;
        self.probe
            .commands
            .lock()
            .expect("command log")
            .push(command.clone());
        let sequence = command.sequence().ok_or(vw_capture::Error::Invalid)?;
        let (connection_epoch, source_generation) =
            command.binding().ok_or(vw_capture::Error::Invalid)?;
        let tiles;
        let (header, payload) = match &command {
            Command::Poll { .. } => {
                self.frame += 1;
                let mut frame = packet(self.frame, 0);
                frame.delivery_sequence = 0;
                frame.source_generation = 0;
                tiles = frame.encode_to_vec();
                (
                    Header::Tiles {
                        sequence,
                        connection_epoch,
                        source_generation,
                        frame_id: self.frame,
                        timestamp_ns: self.frame * 1000,
                        captured_at_ms: stamp(self.frame).captured_at_ms,
                        keyframe: true,
                        payload_bytes: tiles.len() as u32,
                    },
                    tiles.as_slice(),
                )
            }
            Command::InspectSource { .. } => (
                Header::Idle {
                    sequence,
                    connection_epoch,
                    source_generation,
                    retained_frames: self.frame as usize,
                    retained_bytes: self.frame * 4096,
                },
                &[][..],
            ),
            Command::Original { identity, .. } => (
                Header::Original {
                    sequence,
                    connection_epoch,
                    source_generation,
                    identity: identity.clone(),
                    asset_id: vw_model::AssetId::hash(self.original.bytes()).to_string(),
                    payload_bytes: self.original.bytes().len() as u32,
                },
                self.original.bytes(),
            ),
            _ => return Err(vw_capture::Error::Invalid),
        };
        let mut wire = Vec::new();
        stream_wire::write_packet(&mut wire, &header, payload, &self.pool, cancel)?;
        let reply = stream_wire::read_packet(&mut Cursor::new(wire), &self.pool, cancel)?;
        // Exercise G's actual packet validator as well as the worker's exact
        // pool/correlation checks; local protobuf authority fields stay zero.
        vw_capture::stream_process::validate_reply(&self.request, &command, &reply, &self.pool)?;
        assert!(reply.belongs_to(&self.pool));
        Ok(reply)
    }
    fn close(&mut self) -> CaptureResult<()> {
        self.probe.closes.fetch_add(1, Ordering::AcqRel);
        self.retired = true;
        Ok(())
    }
}
impl Drop for Helper {
    fn drop(&mut self) {
        self.probe.drops.fetch_add(1, Ordering::AcqRel);
        if !self.retired {
            self.probe.unretired_drops.fetch_add(1, Ordering::AcqRel);
        }
    }
}
fn open_provider(probe: Arc<Probe>, original: Vec<u8>) -> OpenProvider {
    Box::new(move |request, pool, cancel| {
        probe.opens.fetch_add(1, Ordering::AcqRel);
        *probe.pool.lock().expect("provider pool") = Some(pool.clone());
        let original = BudgetedBytes::copy_from(pool, MemoryKind::Transfer, &original, cancel)
            .map_err(stream_error)?;
        Ok(Box::new(Helper {
            probe,
            request: request.clone(),
            pool: pool.clone(),
            original,
            frame: 0,
            retired: false,
        }))
    })
}
fn approved() -> CapturePublishSource {
    let source = source(1);
    let geometry = source.geometry.wire();
    let rect = geometry
        .client_rect_host
        .as_ref()
        .expect("fixture geometry");
    CapturePublishSource {
        explicit_owner_action: true,
        capture_session_id: source.session.to_string(),
        source_kind: geometry.source_kind.clone(),
        window_handle: geometry.window_handle,
        monitor_id: geometry.monitor_id.clone(),
        host_x: rect.x,
        host_y: rect.y,
        width: source.geometry.width(),
        height: source.geometry.height(),
        dpi_scale: geometry.dpi_scale,
        geometry_revision: geometry.geometry_revision,
        observed_timestamp_ns: geometry.timestamp_ns,
        target_fps: source.fps,
        interaction: source.interaction,
        platform: source.platform,
        source_name: source.name,
    }
}
fn options() -> CaptureLocalOptions {
    let source = approved();
    let rect = CaptureLocalRect {
        x: source.host_x,
        y: source.host_y,
        width: source.width,
        height: source.height,
    };
    CaptureLocalOptions {
        explicit_owner_action: true,
        selection: CaptureLocalSelection::Window {
            target: CaptureLocalWindow {
                window: source.window_handle,
                process_id: if std::process::id() == 1 { 2 } else { 1 },
                process_created: 1,
                client: rect.clone(),
                frame: rect,
                dpi: (source.dpi_scale * 96.0) as u32,
                observed_ns: source.observed_timestamp_ns as u64,
            },
        },
        helper_path: "synthetic-helper-not-executed".into(),
        trusted_helper_sha256: "0".repeat(64),
        ring_bytes: 1024 * 1024,
    }
}
fn png() -> Vec<u8> {
    let directory = root();
    let (_, mut file) = fixture(directory.path());
    file.seek(SeekFrom::Start(0)).expect("fixture PNG rewind");
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).expect("fixture PNG bytes");
    bytes
}

#[derive(Clone, Copy)]
enum MediaAdmission {
    Accept,
    Decline,
}
fn flush(bridge: &mut Bridge, output: &mut Vec<Body>, admission: MediaAdmission) {
    if let Some(publication) = bridge.test_publication() {
        publication
            .flush(|body| {
                if matches!(admission, MediaAdmission::Decline)
                    && matches!(&body, Body::FrameTiles(_))
                {
                    return Err(vw_net::NetError::Backpressure);
                }
                output.push(body);
                Ok(())
            })
            .expect("actual publication flush");
    }
}
async fn drive<F: Future>(
    bridge: &mut Bridge,
    mut future: Pin<&mut F>,
    output: &mut Vec<Body>,
    admission: MediaAdmission,
) -> F::Output {
    tokio::time::timeout(DEADLINE, async {
        loop {
            tokio::select! {
                result = future.as_mut() => return result,
                event = bridge.next() => {
                    bridge.handle(event, 0).expect("actual bridge dispatch");
                    flush(bridge, output, admission);
                }
            }
        }
    })
    .await
    .expect("native fixture deadline")
}
async fn opening(
    hub: &Hub,
    bridge: &mut Bridge,
    probe: Arc<Probe>,
    original: Vec<u8>,
    output: &mut Vec<Body>,
) -> (Arc<CapturePublisher>, Arc<CaptureLocalProducer>) {
    let mut publish = Box::pin(hub.publish(1, approved(), BUDGET));
    let publisher = drive(bridge, publish.as_mut(), output, MediaAdmission::Accept)
        .await
        .expect("native publisher admission");
    let injection = Injection::install(open_provider(probe.clone(), original));
    let mut acquire = Box::pin(
        publisher
            .clone()
            .open_local_capture(options(), Arc::new(crate::Cancellation::default())),
    );
    let producer = drive(bridge, acquire.as_mut(), output, MediaAdmission::Accept)
        .await
        .expect("actual local producer acquisition");
    assert!(OPEN.lock().expect("test provider lock").is_none());
    drop(injection);
    assert_eq!(probe.opens.load(Ordering::Acquire), 1);
    let pool = probe
        .pool
        .lock()
        .expect("provider pool")
        .clone()
        .expect("injected pool");
    assert!(producer.worker.pool().same_pool(&pool));
    (publisher, producer)
}
async fn pump(
    producer: &CaptureLocalProducer,
    bridge: &mut Bridge,
    output: &mut Vec<Body>,
    admission: MediaAdmission,
) -> CaptureLocalProgress {
    let mut future = Box::pin(producer.pump_once());
    drive(bridge, future.as_mut(), output, admission)
        .await
        .expect("native pump")
}

#[tokio::test]
async fn native_pump_carries_exact_queue_feedback_and_paused_original_binding() {
    let _codec = WORKERS.lock().await;
    let _helper = worker::TESTS.lock().await;
    let hub = Hub::new();
    let (endpoint, commands) = hub.activate(1).expect("connection");
    let mut bridge = Bridge::new(endpoint, commands);
    let probe = Arc::new(Probe::default());
    let original = png();
    let mut output = Vec::new();
    let (publisher, producer) = opening(
        &hub,
        &mut bridge,
        probe.clone(),
        original.clone(),
        &mut output,
    )
    .await;

    assert_eq!(
        pump(&producer, &mut bridge, &mut output, MediaAdmission::Accept).await,
        CaptureLocalProgress::Tiles {
            frame_id: 1,
            admitted: true
        }
    );
    assert_eq!(
        pump(&producer, &mut bridge, &mut output, MediaAdmission::Decline).await,
        CaptureLocalProgress::Tiles {
            frame_id: 2,
            admitted: false
        }
    );
    assert_eq!(
        pump(&producer, &mut bridge, &mut output, MediaAdmission::Accept).await,
        CaptureLocalProgress::Tiles {
            frame_id: 3,
            admitted: true
        }
    );
    let emitted: Vec<_> = output
        .iter()
        .filter_map(|body| match body {
            Body::FrameTiles(frame) => Some((
                frame.frame_id,
                frame.delivery_sequence,
                frame.source_generation,
            )),
            _ => None,
        })
        .collect();
    let generation = publisher.signal().source_generation;
    assert_eq!(emitted, vec![(1, 1, generation), (3, 3, generation)]);

    bridge
        .control(source(generation).control("pause"))
        .expect("authenticated pause");
    assert!(publisher.signal().paused);
    let before = output.len();
    assert_eq!(
        pump(&producer, &mut bridge, &mut output, MediaAdmission::Accept).await,
        CaptureLocalProgress::Idle
    );
    assert_eq!(output.len(), before);

    // Freeze the older admitted frame, not the latest source frame or the
    // declined frame. Native history supplies the exact original ticket.
    bridge
        .freeze_request(
            pb::FreezeRequest {
                capture_session_id: Some(id(1).to_proto()),
                frame_id: 1,
                request_id: Some(id(8).to_proto()),
                geometry_revision: 1,
                source_generation: generation,
            },
            0,
        )
        .expect("exact freeze request");
    let ticket = publisher.signal().original.expect("original ticket");
    assert_eq!(ticket.frame.frame_id, 1);
    assert_eq!(ticket.frame.timestamp_ns, identity(1).timestamp_ns);
    assert_eq!(ticket.frame.captured_at_ms, stamp(1).captured_at_ms);
    assert_eq!(
        pump(&producer, &mut bridge, &mut output, MediaAdmission::Accept).await,
        CaptureLocalProgress::OriginalProvided { frame_id: 1 }
    );
    flush(&mut bridge, &mut output, MediaAdmission::Accept);
    let offer = output
        .iter()
        .find_map(|body| match body {
            Body::LosslessFrame(value) if value.available => {
                OriginalOffer::parse(value, &identity(1), &stamp(1), &id(8), generation)
                    .expect("native original offer")
            }
            _ => None,
        })
        .expect("exact original offered");
    assert_eq!(offer.asset, vw_model::AssetId::hash(&original));
    bridge
        .stream_blob(offer.request())
        .expect("scoped original request");
    output.clear();
    flush(&mut bridge, &mut output, MediaAdmission::Accept);
    let transferred = output
        .iter()
        .find_map(|body| match body {
            Body::StreamBlob(value) => match value.body.as_ref() {
                Some(pb::stream_blob::Body::Chunk(chunk)) => Some(chunk.data.as_slice()),
                _ => None,
            },
            _ => None,
        })
        .expect("actual original transfer chunk");
    assert_eq!(transferred, original);
    assert_eq!(
        pump(&producer, &mut bridge, &mut output, MediaAdmission::Accept).await,
        CaptureLocalProgress::Idle
    );

    bridge
        .control(source(generation).control("resume"))
        .expect("authenticated resume");
    assert_eq!(
        pump(&producer, &mut bridge, &mut output, MediaAdmission::Accept).await,
        CaptureLocalProgress::Tiles {
            frame_id: 4,
            admitted: true
        }
    );

    let commands = probe.commands.lock().expect("command log").clone();
    assert_eq!(commands.len(), 7);
    assert!(matches!(
        &commands[0],
        Command::Poll {
            sequence: 1,
            feedback: None,
            repair_keyframe: true,
            ..
        }
    ));
    assert!(
        matches!(&commands[1], Command::Poll { sequence: 2, feedback: Some(value), repair_keyframe: false, .. }
        if value.offer_sequence == 1 && value.admitted_to_media)
    );
    assert!(
        matches!(&commands[2], Command::Poll { sequence: 3, feedback: Some(value), .. }
        if value.offer_sequence == 2 && !value.admitted_to_media)
    );
    assert!(matches!(
        &commands[3],
        Command::InspectSource { sequence: 4, .. }
    ));
    let Command::Original {
        identity: requested,
        sequence: 5,
        ..
    } = &commands[4]
    else {
        panic!("missing original command")
    };
    assert_eq!(
        requested,
        &producer
            .request
            .capture_identity(1, 1000, stamp(1).captured_at_ms)
    );
    assert!(matches!(
        &commands[5],
        Command::InspectSource { sequence: 6, .. }
    ));
    assert!(
        matches!(&commands[6], Command::Poll { sequence: 7, feedback: Some(value), repair_keyframe: true, .. }
        if value.offer_sequence == 3 && value.admitted_to_media)
    );
    assert!(
        commands
            .iter()
            .all(|value| value.binding() == Some((1, generation)))
    );

    bridge
        .shutdown()
        .await
        .expect("actor and real worker joined");
    producer
        .close_producer()
        .await
        .expect("same retirement receipt");
    publisher
        .close_publisher()
        .await
        .expect("publication retirement receipt");
    hub.close().await.expect("connection retirement receipt");
    assert_eq!(probe.closes.load(Ordering::Acquire), 1);
    assert_eq!(probe.drops.load(Ordering::Acquire), 1);
    assert_eq!(probe.unretired_drops.load(Ordering::Acquire), 0);
    let pool = probe
        .pool
        .lock()
        .expect("provider pool")
        .clone()
        .expect("pool");
    drop(producer);
    drop(publisher);
    assert_eq!(pool.used_bytes(), 0);
}

#[tokio::test]
async fn abandoned_native_pump_retains_actual_admission_and_forbids_lost_feedback_successor() {
    let _codec = WORKERS.lock().await;
    let _helper = worker::TESTS.lock().await;
    let hub = Hub::new();
    let (endpoint, commands) = hub.activate(1).expect("connection");
    let mut bridge = Bridge::new(endpoint, commands);
    let probe = Arc::new(Probe::default());
    let mut output = Vec::new();
    let (publisher, producer) = opening(&hub, &mut bridge, probe.clone(), png(), &mut output).await;
    let mut waiting = Box::pin(producer.pump_once());

    // Dispatch the real Frame command, then hold the actual codec completion
    // outside its actor. The pump has no queue receipt or helper feedback yet.
    tokio::time::timeout(DEADLINE, async {
        loop {
            assert!(poll_fn(|cx| Poll::Ready(waiting.as_mut().poll(cx).is_pending())).await);
            if bridge.test_publication().is_some_and(|value| value.has_work()) {
                break;
            }
            tokio::select! {
                event = bridge.next() => bridge.handle(event, 0).expect("publication command dispatch"),
                _ = tokio::task::yield_now() => (),
            }
        }
    }).await.expect("actual publication admission deadline");
    let completion = tokio::time::timeout(DEADLINE, bridge.next())
        .await
        .expect("actual codec completion");
    assert!(
        bridge
            .test_publication()
            .expect("publisher owner")
            .has_work()
    );
    drop(waiting);

    // The abandoned pump seals its G owner before another offer can consume
    // missing feedback. Its native input permit remains held by the real job.
    assert!(matches!(
        producer.pump_once().await,
        Err(SessionError::Cancelled)
    ));
    let mut next = packet(2, 0);
    next.delivery_sequence = 0;
    assert!(matches!(
        publisher.publish_frame(next.encode_to_vec()).await,
        Err(SessionError::Backpressure)
    ));
    assert_eq!(probe.commands.lock().expect("command log").len(), 1);
    bridge
        .handle(completion, 0)
        .expect("actual canceled publication completion");
    flush(&mut bridge, &mut output, MediaAdmission::Accept);
    assert!(
        output
            .iter()
            .all(|body| !matches!(body, Body::FrameTiles(_)))
    );
    assert!(
        !bridge
            .test_publication()
            .expect("publisher owner")
            .has_work()
    );
    assert!(matches!(
        producer.pump_once().await,
        Err(SessionError::Cancelled)
    ));
    assert_eq!(probe.commands.lock().expect("command log").len(), 1);

    bridge
        .shutdown()
        .await
        .expect("publication and G owner actually joined");
    publisher
        .close_publisher()
        .await
        .expect("publication joined");
    producer.close_producer().await.expect("G worker joined");
    hub.close().await.expect("connection joined");
    assert_eq!(probe.closes.load(Ordering::Acquire), 1);
    assert_eq!(probe.drops.load(Ordering::Acquire), 1);
    assert_eq!(probe.unretired_drops.load(Ordering::Acquire), 0);
    let pool = probe
        .pool
        .lock()
        .expect("provider pool")
        .clone()
        .expect("pool");
    drop(producer);
    drop(publisher);
    assert_eq!(pool.used_bytes(), 0);
}
