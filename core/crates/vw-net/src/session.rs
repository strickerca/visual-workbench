use crate::{
    AuthenticatedPeer, Frame, NetError, PROTOCOL_MAJOR, PROTOCOL_MINOR, Result, channel_for,
    encode_frame,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use vw_model::{DeviceId, Id};
use vw_proto::v1::{self, envelope::Body};

const MAX_QUEUE_BYTES: usize = 16 * 1024 * 1024;
const MAX_KEYS: usize = 256;
const IDLE_INPUT_MS: u64 = 600_000;

#[derive(Debug, Clone)]
pub struct LocalHello {
    pub device: DeviceId,
    pub platform: v1::Platform,
    pub app_version: String,
    pub label: String,
    pub capabilities: BTreeSet<String>,
}
impl LocalHello {
    pub fn hello(&self, connection: &Id) -> v1::Hello {
        v1::Hello {
            protocol_major: PROTOCOL_MAJOR,
            protocol_minor: PROTOCOL_MINOR,
            device_id: self.device.to_string(),
            platform: self.platform as i32,
            app_version: self.app_version.clone(),
            device_label: self.label.clone(),
            capabilities: self.capabilities.iter().cloned().collect(),
            connection_id: Some(connection.to_proto()),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Discovering,
    Handshaking,
    Connected(v1::Carrier),
    Reconnecting { deadline_ms: u64 },
    Offline { pending: usize },
}
#[derive(Debug, Clone, Copy)]
pub enum ConnectionEvent {
    Discover,
    Handshake,
    Connected(v1::Carrier),
    Lost { now_ms: u64 },
    Tick { now_ms: u64, pending: usize },
    Disconnect,
}
pub struct ConnectionMachine {
    state: ConnectionState,
}
impl Default for ConnectionMachine {
    fn default() -> Self {
        Self {
            state: ConnectionState::Disconnected,
        }
    }
}
impl ConnectionMachine {
    pub const fn state(&self) -> &ConnectionState {
        &self.state
    }
    /// Returns true when callers must discard volatile state and input grants.
    pub fn transition(&mut self, event: ConnectionEvent) -> Result<bool> {
        let (state, clear) = match (&self.state, event) {
            (_, ConnectionEvent::Disconnect) => (ConnectionState::Disconnected, true),
            (
                ConnectionState::Disconnected | ConnectionState::Offline { .. },
                ConnectionEvent::Discover,
            ) => (ConnectionState::Discovering, false),
            (
                ConnectionState::Discovering | ConnectionState::Reconnecting { .. },
                ConnectionEvent::Handshake,
            ) => (ConnectionState::Handshaking, true),
            (ConnectionState::Handshaking, ConnectionEvent::Connected(carrier))
                if carrier != v1::Carrier::Unspecified =>
            {
                (ConnectionState::Connected(carrier), true)
            }
            (
                ConnectionState::Connected(_) | ConnectionState::Handshaking,
                ConnectionEvent::Lost { now_ms },
            ) => (
                ConnectionState::Reconnecting {
                    deadline_ms: now_ms.saturating_add(2000),
                },
                true,
            ),
            (
                ConnectionState::Reconnecting { deadline_ms },
                ConnectionEvent::Tick { now_ms, pending },
            ) if now_ms >= *deadline_ms => (ConnectionState::Offline { pending }, false),
            (ConnectionState::Reconnecting { .. }, ConnectionEvent::Tick { .. }) => {
                (self.state.clone(), false)
            }
            _ => return Err(NetError::Invalid("connection transition")),
        };
        self.state = state;
        Ok(clear)
    }
}
#[derive(Debug, Default)]
pub struct InputGuard {
    grant: Option<(Id, Id, u32)>,
    last_seq: u64,
    last_activity_ms: u64,
    previous: BTreeSet<Id>,
}
impl InputGuard {
    /// Called only by the local, explicit user grant path, never by InputEvent.
    pub fn grant(&mut self, session: Id, capture: Id, geometry: u32, now_ms: u64) -> Result<()> {
        if geometry == 0 || self.previous.contains(&session) || self.previous.len() >= 4096 {
            return Err(NetError::Invalid("fresh input grant"));
        }
        self.previous.insert(session.clone());
        self.grant = Some((session, capture, geometry));
        self.last_seq = 0;
        self.last_activity_ms = now_ms;
        Ok(())
    }
    pub fn revoke(&mut self) {
        self.grant = None;
        self.last_seq = 0;
    }
    pub const fn last_input_seq(&self) -> u64 {
        self.last_seq
    }
    pub fn accept(&mut self, event: &v1::InputEvent, now_ms: u64) -> Result<bool> {
        let Some((session, capture, geometry)) = &self.grant else {
            return Ok(false);
        };
        if now_ms < self.last_activity_ms || now_ms - self.last_activity_ms >= IDLE_INPUT_MS {
            self.revoke();
            return Ok(false);
        }
        if event.input_session_id.as_ref() != Some(&session.to_proto())
            || event.capture_session_id.as_ref() != Some(&capture.to_proto())
            || event.geometry_revision != *geometry
            || event.input_seq <= self.last_seq
            || event.event.is_none()
        {
            return Ok(false);
        }
        self.last_seq = event.input_seq;
        self.last_activity_ms = now_ms;
        Ok(true)
    }
}
#[derive(Debug, PartialEq)]
// Body is already size-bounded and boxes its largest wire payloads. Keep the
// delivered body owned inline; another box would allocate on every receive.
#[allow(clippy::large_enum_variant)]
pub enum Receive {
    Deliver(Body),
    Discard,
}
pub struct Session {
    peer: DeviceId,
    local: DeviceId,
    connection: Id,
    capabilities: BTreeSet<String>,
    active: bool,
    next: [u64; 7],
    received: [u64; 7],
    queues: [VecDeque<Frame>; 7],
    queued_bytes: [usize; 7],
    latest: BTreeMap<(i32, String), Frame>,
    watermarks: crate::replay::ReplayWindow,
    media_started: BTreeSet<String>,
    media_received: BTreeMap<String, u64>,
    media_offered: crate::replay::ReplayWindow,
    media_recovery: BTreeSet<String>,
    bulk_media_first: bool,
    pub input: InputGuard,
}
impl Session {
    pub fn accept(
        local: &LocalHello,
        peer: &AuthenticatedPeer,
        hello: &v1::Hello,
    ) -> Result<(Self, v1::HelloAck)> {
        if hello.device_id != peer.device().as_str()
            || hello.protocol_major != PROTOCOL_MAJOR
            || hello.protocol_minor < PROTOCOL_MINOR
            || hello.capabilities.len() > 64
            || hello.capabilities.iter().any(|s| s.len() > 64)
            || hello.device_label.len() > 256
            || hello.app_version.len() > 64
            || hello.platform == v1::Platform::Unspecified as i32
        {
            return Err(NetError::Invalid(
                "Hello: identity/version/limits; upgrade may be required",
            ));
        }
        let connection = Id::from_proto(hello.connection_id.as_ref())?;
        let capabilities: BTreeSet<_> = hello
            .capabilities
            .iter()
            .filter(|c| local.capabilities.contains(*c))
            .cloned()
            .collect();
        let ack = v1::HelloAck {
            accepted: true,
            protocol_minor: hello.protocol_minor.min(PROTOCOL_MINOR),
            capabilities: capabilities.iter().cloned().collect(),
            reason: String::new(),
            connection_id: Some(connection.to_proto()),
        };
        Ok((
            Self::new(
                local.device.clone(),
                peer.device().clone(),
                connection,
                capabilities,
            ),
            ack,
        ))
    }
    pub fn finish(
        local: &LocalHello,
        peer: &AuthenticatedPeer,
        connection: Id,
        ack: &v1::HelloAck,
    ) -> Result<Self> {
        if !ack.accepted
            || ack.protocol_minor != PROTOCOL_MINOR
            || ack.connection_id.as_ref() != Some(&connection.to_proto())
            || ack.capabilities.len() > 64
            || ack
                .capabilities
                .iter()
                .any(|c| !local.capabilities.contains(c))
        {
            return Err(NetError::Invalid("HelloAck"));
        }
        Ok(Self::new(
            local.device.clone(),
            peer.device().clone(),
            connection,
            ack.capabilities.iter().cloned().collect(),
        ))
    }
    fn new(
        local: DeviceId,
        peer: DeviceId,
        connection: Id,
        capabilities: BTreeSet<String>,
    ) -> Self {
        let mut sequences = [0; 7];
        sequences[v1::Channel::Control as usize] = 1;
        Self {
            peer,
            local,
            connection,
            capabilities,
            active: true,
            next: sequences,
            received: sequences,
            queues: std::array::from_fn(|_| VecDeque::new()),
            queued_bytes: [0; 7],
            latest: BTreeMap::new(),
            watermarks: crate::replay::ReplayWindow::new(4096),
            media_started: BTreeSet::new(),
            media_received: BTreeMap::new(),
            media_offered: crate::replay::ReplayWindow::new(MAX_KEYS),
            media_recovery: BTreeSet::new(),
            bulk_media_first: false,
            input: InputGuard::default(),
        }
    }
    pub fn capabilities(&self) -> &BTreeSet<String> {
        &self.capabilities
    }
    pub const fn peer(&self) -> &DeviceId {
        &self.peer
    }
    pub fn disconnect(&mut self) {
        self.active = false;
        self.queues.iter_mut().for_each(VecDeque::clear);
        self.queued_bytes = [0; 7];
        self.latest.clear();
        self.watermarks.clear();
        self.media_started.clear();
        self.media_received.clear();
        self.media_offered.clear();
        self.media_recovery.clear();
        self.input.revoke();
    }
    pub fn enqueue_transaction(
        &mut self,
        store: &mut vw_store::ProjectStore,
        transaction: v1::Transaction,
    ) -> Result<()> {
        if transaction.device_id != self.local.as_str() {
            return Err(NetError::Authentication);
        }
        store.enqueue_pending(&transaction)?;
        self.enqueue(Body::Txn(transaction))
    }
    /// Retrying is safe: each accepted transaction returns its original receipt.
    /// Backpressure leaves every remaining transaction in the durable journal.
    pub fn resend_pending(&mut self, store: &vw_store::ProjectStore) -> Result<()> {
        for transaction in store.pending()? {
            if transaction.device_id == self.local.as_str() {
                self.enqueue(Body::Txn(transaction))?;
            }
        }
        Ok(())
    }
    pub fn enqueue(&mut self, body: Body) -> Result<()> {
        if !self.active {
            return Err(NetError::Invalid("disconnected session"));
        }
        let channel = channel_for(&body);
        let index = channel as usize;
        let seq = self.next[index]
            .checked_add(1)
            .ok_or(NetError::Invalid("sequence exhausted"))?;
        let media = if channel == v1::Channel::Media {
            let key = latest_key(&body)?;
            let map_key = (channel as i32, key.clone());
            let (frame_id, keyframe) = media_info(&body)?;
            let previous = self.media_offered.get(&map_key).map(|mark| mark.value);
            if previous.is_some_and(|last| {
                frame_id < last
                    || (frame_id == last && (!keyframe || self.media_started.contains(&key)))
            }) {
                return Ok(());
            }
            if let Some((_, retired)) = self
                .media_offered
                .make_room(&map_key, |candidate| self.latest.contains_key(candidate))?
            {
                self.media_started.remove(&retired);
            }
            self.media_offered.record(map_key, seq, frame_id)?;
            Some((
                key,
                keyframe,
                previous.is_some_and(|last| last.checked_add(1) != Some(frame_id)),
            ))
        } else {
            None
        };
        let frame = Frame {
            channel,
            envelope: v1::Envelope {
                channel: channel as i32,
                seq,
                connection_id: Some(self.connection.to_proto()),
                body: Some(body),
            },
        };
        let size = match encode_frame(&frame) {
            Ok(bytes) => bytes.len(),
            Err(error) => {
                if let Some((key, _, _)) = &media {
                    self.media_started.remove(key);
                }
                return Err(error);
            }
        };
        if matches!(channel, v1::Channel::Ephemeral | v1::Channel::Media) {
            let key = latest_key(
                frame
                    .envelope
                    .body
                    .as_ref()
                    .ok_or(NetError::Invalid("body"))?,
            )?;
            let map_key = (channel as i32, key);
            let prior = self
                .latest
                .get(&map_key)
                .map(encode_frame)
                .transpose()?
                .map_or(0, |b| b.len());
            if channel == v1::Channel::Media {
                let (_, keyframe, gap) =
                    media.as_ref().ok_or(NetError::Invalid("media routing"))?;
                if !keyframe && (prior > 0 || *gap || !self.media_started.contains(&map_key.1)) {
                    self.latest.remove(&map_key);
                    self.queued_bytes[index] -= prior;
                    self.media_started.remove(&map_key.1);
                    return Err(NetError::KeyframeRequired);
                }
                if !self.media_started.contains(&map_key.1) && self.media_started.len() >= MAX_KEYS
                {
                    return Err(NetError::Backpressure);
                }
            }
            if (!self.latest.contains_key(&map_key) && self.latest.len() >= MAX_KEYS)
                || self.queued_bytes[index] - prior + size > MAX_QUEUE_BYTES
            {
                if channel == v1::Channel::Media {
                    self.media_started.remove(&map_key.1);
                }
                return Err(NetError::Backpressure);
            }
            if channel == v1::Channel::Media {
                self.media_started.insert(map_key.1.clone());
            }
            self.queued_bytes[index] = self.queued_bytes[index] - prior + size;
            self.latest.insert(map_key, frame);
        } else {
            if self.queues[index].len() >= 1024 || self.queued_bytes[index] + size > MAX_QUEUE_BYTES
            {
                return Err(NetError::Backpressure);
            }
            self.queued_bytes[index] += size;
            if channel == v1::Channel::Blob {
                let priority = blob_priority(&frame);
                let position = self.queues[index]
                    .iter()
                    .position(|other| blob_priority(other) > priority)
                    .unwrap_or(self.queues[index].len());
                self.queues[index].insert(position, frame);
            } else {
                self.queues[index].push_back(frame);
            }
        }
        self.next[index] = seq;
        Ok(())
    }
    /// One bounded write at a time. CONTROL/INPUT/OPS preempt bulk traffic;
    /// latest-only queues retain at most one unsent item per capture/gesture.
    pub fn next_frame(&mut self) -> Result<Option<Frame>> {
        self.next_frame_where(|_| true)
    }
    /// Select only channels whose bounded carrier write lane is currently free.
    /// Disallowed latest slots stay replaceable while another frame is in flight.
    pub(crate) fn next_frame_where(
        &mut self,
        allowed: impl Fn(v1::Channel) -> bool,
    ) -> Result<Option<Frame>> {
        let order = if self.bulk_media_first {
            [1, 6, 2, 3, 5, 4]
        } else {
            [1, 6, 2, 3, 4, 5]
        };
        for channel in order {
            let kind = v1::Channel::try_from(channel as i32)
                .map_err(|_| NetError::Invalid("scheduled channel"))?;
            if !allowed(kind) {
                continue;
            }
            let frame = if channel == 3 || channel == 5 {
                let key = self
                    .latest
                    .keys()
                    .find(|(c, _)| *c == channel as i32)
                    .cloned();
                key.and_then(|key| self.latest.remove(&key))
            } else {
                self.queues[channel].pop_front()
            };
            if let Some(frame) = frame {
                self.queued_bytes[channel] -= encode_frame(&frame)?.len();
                if channel == 4 || channel == 5 {
                    self.bulk_media_first = channel == 4;
                }
                return Ok(Some(frame));
            }
        }
        Ok(None)
    }
    pub fn receive(&mut self, frame: Frame, now_ms: u64) -> Result<Receive> {
        encode_frame(&frame)?;
        if !self.active
            || frame.envelope.connection_id.as_ref() != Some(&self.connection.to_proto())
            || frame.envelope.seq == 0
        {
            return Ok(Receive::Discard);
        }
        let body = frame.envelope.body.ok_or(NetError::Invalid("body"))?;
        let index = frame.channel as usize;
        if matches!(frame.channel, v1::Channel::Ephemeral | v1::Channel::Media) {
            let key = (frame.channel as i32, latest_key(&body)?);
            if !self.watermarks.accepts(&key, frame.envelope.seq) {
                return Ok(Receive::Discard);
            }
            if let Some((channel, retired)) = self.watermarks.make_room(&key, |_| false)?
                && channel == v1::Channel::Media as i32
            {
                self.media_received.remove(&retired);
                self.media_recovery.remove(&retired);
            }
            if !self.watermarks.accepts(&key, frame.envelope.seq) {
                return Ok(Receive::Discard);
            }
            if frame.channel == v1::Channel::Media {
                let (frame_id, keyframe) = media_info(&body)?;
                let prior = self.media_received.get(&key.1).copied();
                if prior.is_some_and(|prior| {
                    prior > frame_id
                        || (prior == frame_id
                            && (!keyframe || !self.media_recovery.contains(&key.1)))
                }) {
                    return Ok(Receive::Discard);
                }
                if !keyframe
                    && (self.media_recovery.contains(&key.1)
                        || prior.and_then(|prior| prior.checked_add(1)) != Some(frame_id))
                {
                    self.media_received.insert(key.1.clone(), frame_id);
                    self.media_recovery.insert(key.1.clone());
                    self.watermarks.record(key, frame.envelope.seq, 0)?;
                    return Err(NetError::KeyframeRequired);
                }
                self.media_recovery.remove(&key.1);
                self.media_received.insert(key.1.clone(), frame_id);
            }
            self.watermarks.record(key, frame.envelope.seq, 0)?;
        } else if frame.channel != v1::Channel::Blob {
            if frame.envelope.seq <= self.received[index] {
                return Ok(Receive::Discard);
            }
            self.received[index] = frame.envelope.seq;
        }
        if let Body::Txn(txn) = &body
            && txn.device_id != self.peer.as_str()
        {
            return Err(NetError::Authentication);
        }
        if let Body::InputEvent(event) = &body
            && !self.input.accept(event, now_ms)?
        {
            return Ok(Receive::Discard);
        }
        Ok(Receive::Deliver(body))
    }
}
fn blob_priority(frame: &Frame) -> u32 {
    match frame.envelope.body.as_ref() {
        Some(Body::BlobRequest(v)) => v.priority,
        Some(Body::TileRequest(v)) => v.priority,
        Some(Body::BlobAck(_)) => 0,
        _ => u32::MAX,
    }
}
pub(crate) fn media_info(body: &Body) -> Result<(u64, bool)> {
    let value = match body {
        Body::FrameTiles(v) => (v.frame_id, v.keyframe),
        Body::VideoFrame(v) => (v.frame_id, v.keyframe),
        _ => return Err(NetError::Invalid("media body")),
    };
    if value.0 == 0 {
        return Err(NetError::Invalid("media frame ID"));
    }
    Ok(value)
}
pub(crate) fn latest_key(body: &Body) -> Result<String> {
    let (prefix, id) = match body {
        Body::GestureUpdate(v) => ("gesture", v.gesture_id.as_ref()),
        Body::GestureCancel(v) => ("gesture", v.gesture_id.as_ref()),
        Body::CursorUpdate(v) => ("cursor", v.document_id.as_ref()),
        Body::ViewportOutline(v) => ("viewport", v.document_id.as_ref()),
        Body::FrameTiles(v) => ("media", v.capture_session_id.as_ref()),
        Body::VideoFrame(v) => ("media", v.capture_session_id.as_ref()),
        _ => return Err(NetError::Invalid("latest key")),
    };
    Ok(format!("{prefix}:{}", Id::from_proto(id)?))
}
