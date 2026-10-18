use super::*;
use std::{sync::mpsc, thread, time::Duration};

static WORKER_HELD: AtomicBool = AtomicBool::new(false);
struct WorkerPermit;
impl WorkerPermit {
    fn acquire() -> capture::Result<Self> {
        WORKER_HELD.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| capture::Error::Busy)?;
        Ok(Self)
    }
}
impl Drop for WorkerPermit { fn drop(&mut self) { WORKER_HELD.store(false, Ordering::Release); } }

pub(super) trait Producer: Send + 'static {
    fn exchange(&mut self, command: Command, cancel: &StreamCancellation) -> capture::Result<Packet>;
    /// Returns only after the exact child and its pipe worker are gone.
    fn terminate(&mut self);
}
impl Producer for capture::stream_process::StreamProcess {
    fn exchange(&mut self, command: Command, cancel: &StreamCancellation) -> capture::Result<Packet> {
        self.exchange(command, cancel)
    }
    fn terminate(&mut self) { self.terminate(); }
}
type Opened = oneshot::Receiver<capture::Result<()>>;

pub(super) fn start<P: Producer>(open: impl FnOnce(&StreamCancellation) -> capture::Result<P> + Send + 'static)
    -> capture::Result<(Arc<HostCaptureStream>, Opened)> {
    let permit = WorkerPermit::acquire()?;
    let (done, _) = watch::channel(None);
    let shared = Arc::new(Shared { cancel: Arc::new(StreamCancellation::default()),
        closing: AtomicBool::new(false), done, input: Arc::new(Semaphore::new(1)) });
    let (send, receive) = mpsc::sync_channel(1);
    let stream = Arc::new(HostCaptureStream { shared: shared.clone(), send });
    let (opened, ready) = oneshot::channel();
    thread::Builder::new().name("vw-host-capture-stream".into()).spawn(move || {
        let mut opened = Some(opened);
        // No application panic is manufactured here. A provider/library panic
        // must still unwind the owned process before reporting completion.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let producer = open(&shared.cancel)?;
            run(producer, &shared, receive, &mut opened)
        })).unwrap_or(Err(capture::Error::Platform));
        if let Some(reply) = opened.take() { let _ = reply.send(outcome); }
        shared.closing.store(true, Ordering::Release);
        drop(permit);
        shared.done.send_replace(Some(outcome));
    }).map_err(|_| capture::Error::Platform)?;
    Ok((stream, ready))
}
struct Owned<P: Producer>(P);
impl<P: Producer> Drop for Owned<P> { fn drop(&mut self) { self.0.terminate(); } }

fn run<P: Producer>(producer: P, shared: &Shared, receive: mpsc::Receiver<Request>,
    opened: &mut Option<oneshot::Sender<capture::Result<()>>>) -> capture::Result<()> {
    let mut producer = Owned(producer);
    shared.check()?;
    if opened.take().ok_or(capture::Error::Platform)?.send(Ok(())).is_err() {
        shared.cancel(); return Err(capture::Error::Cancelled);
    }
    let mut sequence = 0u64;
    let outcome = loop {
        if shared.cancel.check().is_err() { break Err(capture::Error::Cancelled); }
        if shared.closing.load(Ordering::Acquire) {
            while let Ok(request) = receive.try_recv() { let _ = request.reply.send(Err(capture::Error::Unavailable)); }
            sequence = sequence.checked_add(1).ok_or(capture::Error::Limit)?;
            break match producer.0.exchange(Command::Stop { sequence }, &shared.cancel) {
                Ok(Packet { header: Header::Stopped { sequence: actual }, payload }) if actual == sequence && payload.is_empty() => Ok(()),
                Ok(_) => Err(capture::Error::Invalid), Err(error) => Err(error),
            };
        }
        let request = match receive.recv_timeout(Duration::from_millis(5)) {
            Ok(request) => request,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => { shared.cancel(); break Err(capture::Error::Cancelled); }
        };
        if shared.check().is_err() || request.reply.is_closed() {
            let _ = request.reply.send(Err(capture::Error::Cancelled));
            continue;
        }
        sequence = sequence.checked_add(1).ok_or(capture::Error::Limit)?;
        let command = match request.work {
            Work::Poll { feedback, repair } => Command::Poll { sequence, feedback, repair },
            Work::Original { capture_session_id, frame_id } => Command::Original { sequence, capture_session_id, frame_id },
            Work::Heartbeat => Command::Heartbeat { sequence },
        };
        let result = producer.0.exchange(command, &shared.cancel);
        if result.is_ok() && shared.closing.load(Ordering::Acquire) {
            // A normal close during an exchange discards its late payload but
            // still sends Stop. The caller cannot receive an after-close frame.
            let _ = request.reply.send(Err(capture::Error::Unavailable));
            drop(result);
            continue;
        }
        let error = result.as_ref().err().copied();
        let result = result.and_then(|packet| {
            shared.check()?;
            HostStreamPacket::new(packet, request.input, request.output)
        });
        let failed_header = result.as_ref().err().copied();
        let _ = request.reply.send(result);
        if let Some(error) = error.or(failed_header) { break Err(error); }
    };
    // Both normal Stop and failure pass this exact destruction boundary before
    // start() releases worker admission and publishes its completion receipt.
    drop(producer);
    outcome
}
