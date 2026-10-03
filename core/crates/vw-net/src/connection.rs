//! Complete application handshake over an already authenticated carrier.
use crate::carrier::{CarrierReceiver, CarrierSender};
use crate::{
    AuthenticatedPeer, Frame, LocalHello, NetError, Receive, Result, Session,
    carrier::{QuicCarrier, TcpCarrier},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::{
    sync::Notify,
    task::{AbortHandle, JoinSet},
};
use vw_model::Id;
use vw_proto::v1::{self, envelope::Body};

pub enum CarrierIo {
    Tcp(TcpCarrier),
    Quic(QuicCarrier),
}
impl CarrierIo {
    pub fn peer(&self) -> &AuthenticatedPeer {
        match self {
            Self::Tcp(c) => c.peer(),
            Self::Quic(c) => c.peer(),
        }
    }
    pub async fn send(&mut self, frame: &Frame) -> Result<()> {
        match self {
            Self::Tcp(c) => c.send(frame).await,
            Self::Quic(c) => c.send(frame).await,
        }
    }
    pub async fn receive(&mut self) -> Result<Frame> {
        match self {
            Self::Tcp(c) => c.receive().await,
            Self::Quic(c) => c.receive().await,
        }
    }
    pub fn split(self) -> (CarrierSender, CarrierReceiver) {
        match self {
            Self::Tcp(c) => c.split(),
            Self::Quic(c) => c.split(),
        }
    }
}
pub struct SecureConnection {
    carrier: CarrierIo,
    session: Session,
}
impl SecureConnection {
    pub async fn client(mut carrier: CarrierIo, local: &LocalHello) -> Result<Self> {
        let unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| NetError::Invalid("connection clock"))?
            .as_millis();
        let nonce = Id::generate(
            u64::try_from(unix_ms).map_err(|_| NetError::Invalid("connection clock"))?,
        )?;
        carrier
            .send(&handshake_frame(Body::Hello(local.hello(&nonce)), &nonce))
            .await?;
        let frame = carrier.receive().await?;
        if frame.channel != v1::Channel::Control
            || frame.envelope.seq != 1
            || frame.envelope.connection_id.as_ref() != Some(&nonce.to_proto())
        {
            return Err(NetError::Invalid("handshake envelope"));
        }
        let Some(Body::HelloAck(ack)) = frame.envelope.body else {
            return Err(NetError::Invalid("expected HelloAck"));
        };
        let session = Session::finish(local, carrier.peer(), nonce, &ack)?;
        Ok(Self { carrier, session })
    }
    pub async fn server(mut carrier: CarrierIo, local: &LocalHello) -> Result<Self> {
        let frame = carrier.receive().await?;
        if frame.channel != v1::Channel::Control || frame.envelope.seq != 1 {
            return Err(NetError::Invalid("handshake envelope"));
        }
        let Some(Body::Hello(hello)) = frame.envelope.body else {
            return Err(NetError::Invalid("expected Hello"));
        };
        let nonce = Id::from_proto(hello.connection_id.as_ref())?;
        if frame.envelope.connection_id.as_ref() != Some(&nonce.to_proto()) {
            return Err(NetError::Invalid("handshake connection"));
        }
        let accepted = Session::accept(local, carrier.peer(), &hello);
        let (session, ack) = match accepted {
            Ok(value) => value,
            Err(error) => {
                let refusal = v1::HelloAck { accepted: false, protocol_minor: crate::PROTOCOL_MINOR, capabilities: Vec::new(), reason: "Protocol or identity mismatch; upgrade both applications and verify pairing".into(), connection_id: Some(nonce.to_proto()) };
                carrier
                    .send(&handshake_frame(Body::HelloAck(refusal), &nonce))
                    .await?;
                return Err(error);
            }
        };
        carrier
            .send(&handshake_frame(Body::HelloAck(ack), &nonce))
            .await?;
        Ok(Self { carrier, session })
    }
    pub const fn session(&self) -> &Session {
        &self.session
    }
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }
    /// Start a bounded duplex writer and independently own the receive half.
    /// At most one write per QUIC channel is in flight; TCP has one serialized
    /// frame writer. No session lock is held across either carrier I/O await.
    pub fn into_duplex(self) -> Result<(ConnectionSender, ConnectionReceiver)> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| NetError::Carrier)?;
        let (writer, reader) = self.carrier.split();
        let state = Arc::new(DuplexState {
            session: Mutex::new(self.session),
            ready: Notify::new(),
            closed: AtomicBool::new(false),
        });
        let task_state = state.clone();
        let task_writer = writer.clone();
        let task = runtime.spawn(async move {
            drive_writes(task_writer, task_state).await;
        });
        let task = Arc::new(DriverTask {
            abort: task.abort_handle(),
        });
        Ok((
            ConnectionSender {
                writer: writer.clone(),
                state: state.clone(),
                _task: task.clone(),
            },
            ConnectionReceiver {
                reader,
                writer,
                state,
                _task: task,
            },
        ))
    }
    /// Serial single-frame convenience for harnesses. Application sessions use
    /// into_duplex so waiting for a write cannot prevent reads or QUIC priorities.
    pub async fn send_next(&mut self) -> Result<bool> {
        let Some(frame) = self.session.next_frame()? else {
            return Ok(false);
        };
        if let Err(error) = self.carrier.send(&frame).await {
            self.session.disconnect();
            return Err(error);
        }
        Ok(true)
    }
    pub async fn receive(&mut self, monotonic_ms: u64) -> Result<Receive> {
        match self.carrier.receive().await {
            Ok(frame) => self.session.receive(frame, monotonic_ms),
            Err(NetError::KeyframeRequired) => Err(NetError::KeyframeRequired),
            Err(error) => {
                self.session.disconnect();
                Err(error)
            }
        }
    }
}
struct DuplexState {
    session: Mutex<Session>,
    ready: Notify,
    closed: AtomicBool,
}
struct DriverTask {
    abort: AbortHandle,
}
impl Drop for DriverTask {
    fn drop(&mut self) {
        self.abort.abort();
    }
}
#[derive(Clone)]
pub struct ConnectionSender {
    writer: CarrierSender,
    state: Arc<DuplexState>,
    _task: Arc<DriverTask>,
}
impl ConnectionSender {
    /// Accepted means admitted to the bounded session queue, not durably ACKed.
    /// Transactions must still be recorded by ProjectStore before enqueueing.
    pub fn enqueue(&self, body: Body) -> Result<()> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(NetError::Carrier);
        }
        self.state
            .session
            .lock()
            .map_err(|_| NetError::Carrier)?
            .enqueue(body)?;
        self.state.ready.notify_one();
        Ok(())
    }
    pub fn close(&self) {
        close_duplex(&self.writer, &self.state);
    }
    pub fn grant_input(&self, session: Id, capture: Id, geometry: u32, now_ms: u64) -> Result<()> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(NetError::Carrier);
        }
        self.state
            .session
            .lock()
            .map_err(|_| NetError::Carrier)?
            .input
            .grant(session, capture, geometry, now_ms)
    }
    pub fn revoke_input(&self) {
        if let Ok(mut session) = self.state.session.lock() {
            session.input.revoke();
        }
    }
}
pub struct ConnectionReceiver {
    reader: CarrierReceiver,
    writer: CarrierSender,
    state: Arc<DuplexState>,
    _task: Arc<DriverTask>,
}
impl ConnectionReceiver {
    pub async fn receive(&mut self, monotonic_ms: u64) -> Result<Receive> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(NetError::Carrier);
        }
        match self.reader.receive().await {
            Ok(frame) => self
                .state
                .session
                .lock()
                .map_err(|_| NetError::Carrier)?
                .receive(frame, monotonic_ms),
            Err(NetError::KeyframeRequired) => Err(NetError::KeyframeRequired),
            Err(error) => {
                close_duplex(&self.writer, &self.state);
                Err(error)
            }
        }
    }
    pub fn close(&self) {
        close_duplex(&self.writer, &self.state);
    }
}
impl Drop for ConnectionReceiver {
    fn drop(&mut self) {
        self.close();
    }
}
fn close_duplex(writer: &CarrierSender, state: &DuplexState) {
    state.closed.store(true, Ordering::Release);
    if let Ok(mut session) = state.session.lock() {
        session.disconnect();
    }
    writer.close();
    state.ready.notify_one();
}
async fn drive_writes(writer: CarrierSender, state: Arc<DuplexState>) {
    let mut writes = JoinSet::new();
    let mut busy = std::collections::BTreeSet::<i32>::new();
    loop {
        if state.closed.load(Ordering::Acquire) {
            break;
        }
        let notified = state.ready.notified();
        let frame = match state.session.lock() {
            Ok(mut session) => session.next_frame_where(|channel| {
                if writer.independent_channels() {
                    !busy.contains(&(channel as i32))
                } else {
                    busy.is_empty()
                }
            }),
            Err(_) => Err(NetError::Carrier),
        };
        match frame {
            Ok(Some(frame)) => {
                let channel = frame.channel as i32;
                busy.insert(channel);
                let lane = writer.clone();
                writes.spawn(async move { (channel, lane.send(&frame).await) });
                continue;
            }
            Err(_) => {
                close_duplex(&writer, &state);
                break;
            }
            Ok(None) => {}
        }
        tokio::select! {
            _ = notified => {},
            result = writes.join_next(), if !writes.is_empty() => {
                match result {
                    Some(Ok((channel, Ok(())))) => { busy.remove(&channel); }
                    _ => { close_duplex(&writer, &state); break; }
                }
            }
        }
    }
    writes.abort_all();
}
fn handshake_frame(body: Body, connection: &Id) -> Frame {
    Frame {
        channel: v1::Channel::Control,
        envelope: v1::Envelope {
            channel: v1::Channel::Control as i32,
            seq: 1,
            connection_id: Some(connection.to_proto()),
            body: Some(body),
        },
    }
}
