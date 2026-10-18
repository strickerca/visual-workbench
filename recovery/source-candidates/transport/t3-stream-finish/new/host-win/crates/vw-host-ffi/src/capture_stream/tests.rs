#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use std::{sync::{atomic::AtomicUsize, mpsc}, time::Duration};
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Fake {
    entered: Option<oneshot::Sender<()>>, release: Option<mpsc::Receiver<()>>,
    commands: Arc<Mutex<Vec<Command>>>, destroyed: Arc<AtomicUsize>, stopped: Arc<AtomicUsize>,
}
impl owner_thread::Producer for Fake {
    fn exchange(&mut self, command: Command, _cancel: &StreamCancellation) -> capture::Result<Packet> {
        let sequence = command.sequence().ok_or(capture::Error::Invalid)?;
        self.commands.lock().map_err(|_| capture::Error::Platform)?.push(command.clone());
        if matches!(&command, Command::Stop { .. }) {
            self.stopped.fetch_add(1, Ordering::AcqRel);
            return Ok(Packet { header: Header::Stopped { sequence }, payload: vec![] });
        }
        if let Some(entered) = self.entered.take() { let _ = entered.send(()); }
        if let Some(release) = self.release.take() { release.recv_timeout(Duration::from_secs(2)).map_err(|_| capture::Error::Timeout)?; }
        Ok(Packet { header: Header::Idle { sequence, retained_frames: 0, retained_bytes: 0 }, payload: vec![] })
    }
    fn terminate(&mut self) {}
}
impl Drop for Fake { fn drop(&mut self) { self.destroyed.fetch_add(1, Ordering::AcqRel); } }
struct Fixture { commands: Arc<Mutex<Vec<Command>>>, destroyed: Arc<AtomicUsize>, stopped: Arc<AtomicUsize> }
impl Fixture {
    fn new() -> Self { Self { commands: Arc::new(Mutex::new(vec![])), destroyed: Arc::new(AtomicUsize::new(0)), stopped: Arc::new(AtomicUsize::new(0)) } }
    fn fake(&self) -> Fake { Fake { entered: None, release: None, commands: self.commands.clone(), destroyed: self.destroyed.clone(), stopped: self.stopped.clone() } }
    async fn start(&self) -> Arc<HostCaptureStream> {
        let fake = self.fake(); let (stream, opened) = owner_thread::start(|_| Ok(fake)).unwrap();
        tokio::time::timeout(Duration::from_secs(2), opened).await.unwrap().unwrap().unwrap(); stream
    }
}
async fn settled(shared: &Arc<Shared>) -> capture::Result<()> {
    let mut done = shared.done.subscribe();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop { if let Some(value) = *done.borrow() { return value; } done.changed().await.unwrap(); }
    }).await.unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn normal_close_sends_stop_and_publishes_only_after_producer_destruction() {
    let _serial = SERIAL.lock().await; let fixture = Fixture::new(); let stream = fixture.start().await;
    let packet = stream.heartbeat().await.unwrap(); assert!(matches!(packet.info(), HostStreamPacketInfo::Idle { .. })); drop(packet);
    tokio::time::timeout(Duration::from_secs(2), stream.close_host_stream()).await.unwrap().unwrap();
    assert_eq!(fixture.stopped.load(Ordering::Acquire), 1); assert_eq!(fixture.destroyed.load(Ordering::Acquire), 1);
    assert!(stream.heartbeat().await.is_err());
    assert!(matches!(&fixture.commands.lock().unwrap()[..], [Command::Heartbeat { sequence: 1 }, Command::Stop { sequence: 2 }]));
}

#[tokio::test(flavor = "current_thread")]
async fn packet_owns_global_output_and_input_until_actual_handle_destruction() {
    let _serial = SERIAL.lock().await; let first = Fixture::new(); let stream = first.start().await;
    let packet = stream.heartbeat().await.unwrap(); assert_eq!(packet.take_payload().unwrap(), Vec::<u8>::new());
    assert!(packet.take_payload().is_err()); assert!(stream.heartbeat().await.is_err());
    stream.close_host_stream().await.unwrap();
    let next_fixture = Fixture::new(); let next = next_fixture.start().await;
    assert!(next.heartbeat().await.is_err()); // prior exported packet remains charged after helper exit
    drop(packet); drop(next.heartbeat().await.unwrap()); next.close_host_stream().await.unwrap();
    assert!(!OUTPUT_HELD.load(Ordering::Acquire));
}

#[tokio::test(flavor = "current_thread")]
async fn canceled_request_keeps_slots_until_actual_blocked_exchange_settles() {
    let _serial = SERIAL.lock().await; let fixture = Fixture::new(); let mut fake = fixture.fake();
    let (entered, ready) = oneshot::channel(); let (release, held) = mpsc::sync_channel(1);
    fake.entered = Some(entered); fake.release = Some(held);
    let (stream, opened) = owner_thread::start(|_| Ok(fake)).unwrap(); opened.await.unwrap().unwrap();
    let job = tokio::spawn({ let stream = stream.clone(); async move { stream.heartbeat().await } });
    tokio::time::timeout(Duration::from_secs(2), ready).await.unwrap().unwrap(); job.abort(); let _ = job.await;
    assert!(OUTPUT_HELD.load(Ordering::Acquire)); assert_eq!(stream.shared.input.available_permits(), 0);
    assert_eq!(fixture.destroyed.load(Ordering::Acquire), 0);
    assert!(owner_thread::start({ let fake = fixture.fake(); move |_| Ok(fake) }).is_err());
    release.send(()).unwrap(); assert_eq!(settled(&stream.shared).await, Err(capture::Error::Cancelled));
    assert_eq!(fixture.destroyed.load(Ordering::Acquire), 2); // rejected opener's fake and actual owner
    assert!(!OUTPUT_HELD.load(Ordering::Acquire)); assert_eq!(stream.shared.input.available_permits(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn normal_close_during_exchange_refuses_late_frame_then_stops_normally() {
    let _serial = SERIAL.lock().await; let fixture = Fixture::new(); let mut fake = fixture.fake();
    let (entered, ready) = oneshot::channel(); let (release, held) = mpsc::sync_channel(1);
    fake.entered = Some(entered); fake.release = Some(held);
    let (stream, opened) = owner_thread::start(|_| Ok(fake)).unwrap(); opened.await.unwrap().unwrap();
    let read = tokio::spawn({ let stream = stream.clone(); async move { stream.heartbeat().await } });
    tokio::time::timeout(Duration::from_secs(2), ready).await.unwrap().unwrap();
    let close = tokio::spawn({ let stream = stream.clone(); async move { stream.close_host_stream().await } });
    tokio::task::yield_now().await; assert!(stream.shared.closing.load(Ordering::Acquire));
    release.send(()).unwrap(); assert!(read.await.unwrap().is_err()); close.await.unwrap().unwrap();
    assert_eq!(fixture.stopped.load(Ordering::Acquire), 1); assert_eq!(fixture.destroyed.load(Ordering::Acquire), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn canceled_start_retains_owner_until_late_producer_is_destroyed() {
    let _serial = SERIAL.lock().await; let fixture = Fixture::new(); let fake = fixture.fake();
    let (entered, ready) = oneshot::channel(); let (release, held) = mpsc::sync_channel(1);
    let (stream, opened) = owner_thread::start(move |_| {
        let _ = entered.send(()); held.recv_timeout(Duration::from_secs(2)).map_err(|_| capture::Error::Timeout)?; Ok(fake)
    }).unwrap();
    let shared = stream.shared.clone(); tokio::time::timeout(Duration::from_secs(2), ready).await.unwrap().unwrap();
    drop(opened); drop(stream); assert_eq!(fixture.destroyed.load(Ordering::Acquire), 0);
    release.send(()).unwrap(); assert_eq!(settled(&shared).await, Err(capture::Error::Cancelled));
    assert_eq!(fixture.destroyed.load(Ordering::Acquire), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn feedback_retains_exact_helper_offer_and_monotonic_command_sequence() {
    let _serial = SERIAL.lock().await; let fixture = Fixture::new(); let stream = fixture.start().await;
    assert!(stream.poll(0, true, false).await.is_err());
    drop(stream.poll(0, false, true).await.unwrap());
    drop(stream.poll(1, false, false).await.unwrap()); stream.close_host_stream().await.unwrap();
    let commands = fixture.commands.lock().unwrap();
    assert!(matches!(&commands[0], Command::Poll { sequence: 1, feedback: None, repair: true }));
    assert!(matches!(&commands[1], Command::Poll { sequence: 2, feedback: Some(Feedback { offer_sequence: 1, admitted_to_media: false }), repair: false }));
}

#[tokio::test(flavor = "current_thread")]
async fn canceled_close_waiter_does_not_cancel_or_abandon_normal_stop() {
    let _serial = SERIAL.lock().await; let fixture = Fixture::new(); let mut fake = fixture.fake();
    let (entered, ready) = oneshot::channel(); let (release, held) = mpsc::sync_channel(1);
    fake.entered = Some(entered); fake.release = Some(held);
    let (stream, opened) = owner_thread::start(|_| Ok(fake)).unwrap(); opened.await.unwrap().unwrap();
    let read = tokio::spawn({ let stream = stream.clone(); async move { stream.heartbeat().await } });
    tokio::time::timeout(Duration::from_secs(2), ready).await.unwrap().unwrap();
    let close = tokio::spawn({ let stream = stream.clone(); async move { stream.close_host_stream().await } });
    tokio::task::yield_now().await; close.abort(); let _ = close.await;
    release.send(()).unwrap(); assert!(read.await.unwrap().is_err()); assert_eq!(settled(&stream.shared).await, Ok(()));
    assert_eq!(fixture.stopped.load(Ordering::Acquire), 1); assert_eq!(fixture.destroyed.load(Ordering::Acquire), 1);
}

fn options() -> HostCaptureStreamOptions {
    HostCaptureStreamOptions { explicit_owner_action: true,
        source: super::super::PickerMonitor { monitor: 1, device: "synthetic-monitor".into(),
            x: -64, y: 32, width: 64, height: 64, dpi: 144, rotation: 0,
            primary: false, observed_ns: 1 }.into_selection(),
        capture_session_id: "01900000-0000-7000-8000-000000000001".into(),
        geometry_revision: 2, target_fps: 30, ring_frames: 8, ring_bytes: 300_000_000,
        helper_memory_bytes: 640 * 1024 * 1024, host_buffer_bytes: HOST_STREAM_BUFFER_BYTES }
}
// Keep the test-only convenience outside the production DTO contract.
trait MonitorSelection { fn into_selection(self) -> super::super::CaptureSelection; }
impl MonitorSelection for super::super::PickerMonitor {
    fn into_selection(self) -> super::super::CaptureSelection { super::super::CaptureSelection::Monitor { target: self } }
}
#[test]
fn explicit_source_and_both_process_budgets_are_admitted_before_start() {
    let value = options().request().unwrap();
    assert_eq!(value.source.rect().x, -64); assert_eq!(value.geometry_revision, 2);
    let mut bad = options(); bad.explicit_owner_action = false;
    assert_eq!(bad.request().unwrap_err(), capture::Error::Consent);
    let mut bad = options(); bad.host_buffer_bytes = HOST_STREAM_BUFFER_BYTES - 1;
    assert_eq!(bad.request().unwrap_err(), capture::Error::Limit);
    let mut bad = options(); bad.helper_memory_bytes = 1;
    assert_eq!(bad.request().unwrap_err(), capture::Error::Limit);
}
#[test]
fn original_mapping_preserves_physical_and_wall_clock_provenance() {
    let input = capture::FrameIdentity { capture_session_id: "01900000-0000-7000-8000-000000000001".into(),
        frame_id: 42, geometry_revision: 9, source_kind: "region".into(),
        client_rect: capture::Rect { x: -1200, y: 37, width: 1280, height: 720 }, dpi_scale: 1.75,
        monotonic_timestamp_ns: 123_456_789, captured_at_ms: 1_800_000_000_000,
        platform: "windows".into(), window_handle: 0, monitor_id: "synthetic-monitor".into() };
    let value = HostStreamOriginalIdentity::new(input.clone(), "a".repeat(64));
    assert_eq!(value.capture_session_id, input.capture_session_id); assert_eq!(value.frame_id, input.frame_id);
    assert_eq!(value.geometry_revision, input.geometry_revision); assert_eq!(value.source_kind, input.source_kind);
    assert_eq!((value.physical_x, value.physical_y, value.width, value.height), (-1200, 37, 1280, 720));
    assert_eq!(value.dpi_scale, 1.75); assert_eq!(value.timestamp_ns, input.monotonic_timestamp_ns);
    assert_eq!(value.captured_at_ms, input.captured_at_ms); assert_eq!(value.platform, "windows");
    assert_eq!(value.monitor_id, input.monitor_id); assert_eq!(value.window_handle, input.window_handle);
    assert_eq!(value.source_asset_id, "a".repeat(64));
}
