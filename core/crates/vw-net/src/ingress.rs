//! Bounded receive scheduling; readers never await an application FIFO slot.
//! Reliable overflow fails the connection so durable OPS can resume. Volatile
//! state coalesces per identity; broken predictive chains request a keyframe.
use crate::{Frame, NetError, Result};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;
use vw_proto::{Message, v1};

const MAX_KEYS: usize = 256;
const MAX_CHANNEL_BYTES: usize = 16 * 1024 * 1024;
const MAX_RELIABLE_FRAMES: usize = 128;
#[derive(Clone)]
pub(crate) struct Ingress {
    inner: Arc<Inner>,
}
struct Inner {
    state: Mutex<State>,
    ready: Notify,
}
struct State {
    connection: Option<vw_model::Id>,
    queues: [VecDeque<Frame>; 7],
    latest: BTreeMap<(i32, String), Frame>,
    watermarks: crate::replay::ReplayWindow,
    media_started: BTreeSet<String>,
    bytes: [usize; 7],
    error: Option<NetError>,
    failed: bool,
    keyframe_required: BTreeSet<String>,
    media_first: bool,
}
impl Ingress {
    pub fn is_failed(&self) -> bool {
        self.inner.state.lock().map_or(true, |state| state.failed)
    }
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    connection: None,
                    queues: std::array::from_fn(|_| VecDeque::new()),
                    latest: BTreeMap::new(),
                    watermarks: crate::replay::ReplayWindow::new(MAX_KEYS),
                    media_started: BTreeSet::new(),
                    bytes: [0; 7],
                    error: None,
                    failed: false,
                    keyframe_required: BTreeSet::new(),
                    media_first: false,
                }),
                ready: Notify::new(),
            }),
        }
    }
    pub fn fail(&self, error: NetError) {
        if let Ok(mut state) = self.inner.state.lock()
            && !state.failed
        {
            state.error = Some(error);
            state.failed = true;
            state.queues.iter_mut().for_each(VecDeque::clear);
            state.latest.clear();
            state.bytes = [0; 7];
        }
        self.inner.ready.notify_one();
    }
    pub fn push(&self, frame: Frame) -> Result<()> {
        let index = frame.channel as usize;
        let body = frame
            .envelope
            .body
            .as_ref()
            .ok_or(NetError::Invalid("missing ingress body"))?;
        if index == 0
            || index > 6
            || crate::channel_for(body) != frame.channel
            || frame.envelope.channel != frame.channel as i32
            || frame.envelope.seq == 0
        {
            return Err(NetError::Invalid("ingress channel/sequence"));
        }
        let size = frame.envelope.encoded_len() + crate::HEADER_BYTES;
        if size > crate::payload_limit(frame.channel) + crate::HEADER_BYTES {
            return Err(NetError::Invalid("ingress quota"));
        }
        let mut state = self.inner.state.lock().map_err(|_| NetError::Carrier)?;
        if state.failed {
            return Err(NetError::Carrier);
        }
        let connection = vw_model::Id::from_proto(frame.envelope.connection_id.as_ref())?;
        if state
            .connection
            .as_ref()
            .is_some_and(|known| *known != connection)
        {
            return Ok(());
        }
        state.connection = Some(connection);
        if matches!(frame.channel, v1::Channel::Media | v1::Channel::Ephemeral) {
            let key = crate::session::latest_key(body)?;
            let map_key = (frame.channel as i32, key.clone());
            let (frame_id, keyframe) = if frame.channel == v1::Channel::Media {
                crate::session::media_info(body)?
            } else {
                (0, true)
            };
            let previous_id = state.watermarks.get(&map_key).map(|mark| mark.value);
            if !state.watermarks.accepts(&map_key, frame.envelope.seq)
                || previous_id.is_some_and(|highest| {
                    frame_id > 0
                        && (highest > frame_id
                            || (highest == frame_id
                                && (!keyframe || state.media_started.contains(&key))))
                })
            {
                return Ok(());
            }
            let retired = {
                let State {
                    watermarks, latest, ..
                } = &mut *state;
                watermarks.make_room(&map_key, |candidate| latest.contains_key(candidate))?
            };
            if let Some((channel, retired)) = retired
                && channel == v1::Channel::Media as i32
            {
                state.media_started.remove(&retired);
                state.keyframe_required.remove(&retired);
            }
            // Eviction can raise this unknown identity's floor. Never insert a
            // delayed envelope that became retired while making room for it.
            if !state.watermarks.accepts(&map_key, frame.envelope.seq) {
                return Ok(());
            }
            let previous = state
                .latest
                .get(&map_key)
                .map_or(0, |old| old.envelope.encoded_len() + crate::HEADER_BYTES);
            state
                .watermarks
                .record(map_key.clone(), frame.envelope.seq, frame_id)?;
            if frame.channel == v1::Channel::Media
                && !keyframe
                && (previous > 0
                    || !state.media_started.contains(&key)
                    || previous_id.and_then(|id| id.checked_add(1)) != Some(frame_id))
            {
                state.latest.remove(&map_key);
                state.bytes[index] -= previous;
                state.media_started.remove(&key);
                state.keyframe_required.insert(key);
                drop(state);
                self.inner.ready.notify_one();
                return Ok(());
            }
            if state.bytes[index] - previous + size > MAX_CHANNEL_BYTES {
                return Err(NetError::Backpressure);
            }
            if frame.channel == v1::Channel::Media {
                state.keyframe_required.remove(&key);
                state.media_started.insert(key);
            }
            state.bytes[index] = state.bytes[index] - previous + size;
            state.latest.insert(map_key, frame);
        } else {
            if state.queues[index].len() >= MAX_RELIABLE_FRAMES
                || state.bytes[index] + size > MAX_CHANNEL_BYTES
            {
                return Err(NetError::Backpressure);
            }
            state.bytes[index] += size;
            state.queues[index].push_back(frame);
        }
        drop(state);
        self.inner.ready.notify_one();
        Ok(())
    }
    fn pop(&self) -> Result<Option<Frame>> {
        let mut state = self.inner.state.lock().map_err(|_| NetError::Carrier)?;
        if let Some(error) = state.error.take() {
            return Err(error);
        }
        if state.failed {
            return Err(NetError::Carrier);
        }
        let order = if state.media_first {
            [1, 6, 2, 3, 5, 4]
        } else {
            [1, 6, 2, 3, 4, 5]
        };
        for index in order {
            let next = if index == 3 || index == 5 {
                let key = state
                    .latest
                    .keys()
                    .find(|(channel, _)| *channel == index as i32)
                    .cloned();
                key.and_then(|key| state.latest.remove(&key))
            } else {
                state.queues[index].pop_front()
            };
            if let Some(frame) = next {
                state.bytes[index] -= frame.envelope.encoded_len() + crate::HEADER_BYTES;
                if index == 4 || index == 5 {
                    state.media_first = index == 4;
                }
                return Ok(Some(frame));
            }
        }
        if !state.keyframe_required.is_empty() {
            state.keyframe_required.clear();
            return Err(NetError::KeyframeRequired);
        }
        Ok(None)
    }
    pub async fn receive(&self) -> Result<Frame> {
        loop {
            let notified = self.inner.ready.notified();
            if let Some(frame) = self.pop()? {
                return Ok(frame);
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use vw_proto::v1::envelope::Body;
    fn id(n: u64) -> vw_model::Id {
        vw_model::Id::from_parts(1_700_000_000_000 + n, [9; 10]).unwrap()
    }
    fn frame(body: Body, seq: u64) -> Frame {
        let channel = crate::channel_for(&body);
        Frame {
            channel,
            envelope: v1::Envelope {
                channel: channel as i32,
                seq,
                connection_id: Some(id(99).to_proto()),
                body: Some(body),
            },
        }
    }
    fn media(n: u64) -> Body {
        Body::FrameTiles(v1::FrameTiles {
            capture_session_id: Some(id(44).to_proto()),
            frame_id: n,
            keyframe: true,
            ..Default::default()
        })
    }
    #[tokio::test(flavor = "current_thread")]
    async fn ingress_coalesces_received_media_and_cursor_before_control_input_ops() {
        let queue = Ingress::new();
        for seq in 1..=10_000 {
            queue.push(frame(media(seq), seq)).unwrap();
            queue
                .push(frame(
                    Body::CursorUpdate(v1::CursorUpdate {
                        document_id: Some(id(1).to_proto()),
                        tool: seq.to_string(),
                        ..Default::default()
                    }),
                    seq,
                ))
                .unwrap();
        }
        queue
            .push(frame(Body::SyncRequest(v1::SyncRequest::default()), 1))
            .unwrap();
        queue
            .push(frame(Body::InputStatus(v1::InputStatus::default()), 1))
            .unwrap();
        queue
            .push(frame(Body::Ping(v1::Ping::default()), 1))
            .unwrap();
        for channel in [v1::Channel::Control, v1::Channel::Input, v1::Channel::Ops] {
            assert_eq!(queue.receive().await.unwrap().channel, channel);
        }
        assert!(
            matches!(queue.receive().await.unwrap().envelope.body, Some(Body::CursorUpdate(v)) if v.tool == "10000")
        );
        assert!(
            matches!(queue.receive().await.unwrap().envelope.body, Some(Body::FrameTiles(v)) if v.frame_id == 10000)
        );
        assert!(queue.pop().unwrap().is_none());
        queue.push(frame(media(1), 20_000)).unwrap();
        assert!(queue.pop().unwrap().is_none());
    }
    #[test]
    fn ingress_does_not_deliver_predictive_media_after_dropping_its_parent() {
        let queue = Ingress::new();
        queue.push(frame(media(1), 1)).unwrap();
        let delta = |n| {
            frame(
                Body::FrameTiles(v1::FrameTiles {
                    capture_session_id: Some(id(44).to_proto()),
                    frame_id: n,
                    keyframe: false,
                    ..Default::default()
                }),
                n,
            )
        };
        queue.push(delta(2)).unwrap();
        assert!(matches!(queue.pop(), Err(NetError::KeyframeRequired)));
        queue.push(delta(3)).unwrap();
        assert!(matches!(queue.pop(), Err(NetError::KeyframeRequired)));
        queue.push(frame(media(4), 4)).unwrap();
        assert!(queue.pop().unwrap().is_some());
        queue.push(delta(5)).unwrap();
        assert!(queue.pop().unwrap().is_some());
        queue.push(delta(7)).unwrap();
        assert!(matches!(queue.pop(), Err(NetError::KeyframeRequired)));
        queue.push(frame(media(7), 8)).unwrap();
        assert!(queue.pop().unwrap().is_some());
    }
    #[test]
    fn reliable_overflow_is_explicit_and_does_not_consume_control_capacity() {
        let queue = Ingress::new();
        for n in 1..=MAX_RELIABLE_FRAMES {
            queue
                .push(frame(Body::BlobAck(v1::BlobAck::default()), n as u64))
                .unwrap();
        }
        assert!(matches!(
            queue.push(frame(Body::BlobAck(v1::BlobAck::default()), 500)),
            Err(NetError::Backpressure)
        ));
        queue
            .push(frame(Body::Ping(v1::Ping::default()), 1))
            .unwrap();
        assert_eq!(queue.pop().unwrap().unwrap().channel, v1::Channel::Control);
    }
    fn gesture(identity: u64, seq: u64) -> Frame {
        frame(
            Body::GestureUpdate(Box::new(v1::GestureUpdate {
                gesture_id: Some(id(identity).to_proto()),
                ..Default::default()
            })),
            seq,
        )
    }
    #[test]
    fn long_session_retires_consumed_history_without_accepting_old_replays() {
        let queue = Ingress::new();
        for seq in 1..=10_000 {
            queue.push(gesture(seq, seq)).unwrap();
            assert!(queue.pop().unwrap().is_some());
        }
        assert_eq!(queue.inner.state.lock().unwrap().watermarks.len(), MAX_KEYS);
        queue.push(gesture(1, 1)).unwrap();
        assert!(queue.pop().unwrap().is_none());
        let mut foreign = gesture(20_000, u64::MAX);
        foreign.envelope.connection_id = Some(id(100).to_proto());
        queue.push(foreign).unwrap();
        queue.push(gesture(10_001, 10_001)).unwrap();
        assert!(queue.pop().unwrap().is_some());
        assert!(queue.pop().unwrap().is_none());
    }
    #[test]
    fn history_retirement_preserves_queued_identity_and_its_delayed_update() {
        let queue = Ingress::new();
        queue.push(gesture(100_000, 1)).unwrap();
        for seq in 1000..2000 {
            queue.push(gesture(seq, seq)).unwrap();
            assert_eq!(queue.pop().unwrap().unwrap().envelope.seq, seq);
        }
        // Another identity's retirement must not discard this live identity's
        // valid delayed update, even below the channel's retired floor.
        queue.push(gesture(100_000, 2)).unwrap();
        assert_eq!(queue.pop().unwrap().unwrap().envelope.seq, 2);
        assert!(queue.pop().unwrap().is_none());
    }
    #[test]
    fn queued_volatile_capacity_remains_bounded_with_control_independent() {
        let queue = Ingress::new();
        for seq in 1..=MAX_KEYS as u64 {
            queue.push(gesture(seq, seq)).unwrap();
        }
        assert!(matches!(
            queue.push(gesture(1000, 1000)),
            Err(NetError::Backpressure)
        ));
        queue
            .push(frame(Body::Ping(v1::Ping::default()), 1))
            .unwrap();
        assert_eq!(queue.pop().unwrap().unwrap().channel, v1::Channel::Control);
        assert_eq!(queue.inner.state.lock().unwrap().latest.len(), MAX_KEYS);
    }
    #[test]
    fn retired_media_requires_keyframe_and_floors_stay_per_channel() {
        let queue = Ingress::new();
        queue.push(frame(media(1), 1)).unwrap();
        queue.pop().unwrap();
        for seq in 1000..2000 {
            queue.push(gesture(seq, seq)).unwrap();
            queue.pop().unwrap();
        }
        let mut delta = media(2);
        if let Body::FrameTiles(value) = &mut delta {
            value.keyframe = false;
        }
        queue.push(frame(delta, 2)).unwrap();
        assert!(matches!(queue.pop(), Err(NetError::KeyframeRequired)));
        queue.push(frame(media(2), 3)).unwrap();
        assert!(queue.pop().unwrap().is_some());
        queue.push(frame(media(1), 1)).unwrap();
        assert!(queue.pop().unwrap().is_none());
    }
}
