//! Latest-only stroke deltas carry a raw sample offset. Missing prefixes are
//! repaired on CONTROL; neither a datagram gap nor an obsolete replay can invent
//! a connecting segment. Minor 2 CONTROL openings/retirements bind monotonic
//! generations to at most four active gestures. Delayed datagrams/replays never
//! admit a gesture; no lifetime finished-ID tombstone collection is needed.
use super::{SessionError, SessionResult};
use crate::{ObjectStyle, SampleBatch, StrokeOptions, Transform};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};
use vw_model::{AssetId, DeviceId, Id};
use vw_proto::{
    Message,
    v1::{self as pb, envelope::Body},
};

#[derive(Clone, Copy, uniffi::Enum)]
pub enum PreviewKind {
    HandleDrag,
    Slider,
    Shape,
}
#[derive(Clone, uniffi::Record)]
pub struct ObjectPreview {
    pub gesture_id: String,
    pub document_id: String,
    pub object_id: String,
    pub sequence: u32,
    pub kind: PreviewKind,
    pub transform: Transform,
    pub style: ObjectStyle,
}
#[derive(Clone, uniffi::Record)]
pub struct NewObjectPreview {
    pub gesture_id: String,
    pub document_id: String,
    pub object_id: String,
    pub layer_id: String,
    pub sequence: u32,
    pub created_at_ms: i64,
    pub shape: crate::NewShape,
    pub transform: Transform,
    pub style: ObjectStyle,
}
#[derive(Clone, uniffi::Record)]
pub struct PeerPreviews {
    pub sequence: u64,
    pub gesture_ids: Vec<String>,
    pub items: Vec<crate::RenderItem>,
}
pub(super) struct Command {
    pub epoch: u64,
    pub kind: CommandKind,
}
impl Command {
    pub fn capture(epoch: u64, kind: CommandKind) -> Option<Self> {
        (epoch & 1 == 1).then_some(Self { epoch, kind })
    }
    pub fn belongs_to(&self, epoch: u64) -> bool {
        self.epoch == epoch && epoch & 1 == 1
    }
}
pub(super) enum CommandKind {
    Wire(Body),
    Stroke {
        options: StrokeOptions,
        batch: SampleBatch,
        offset: u32,
    },
    Object(ObjectPreview),
    NewObject(NewObjectPreview),
    Finish {
        gesture: Id,
        cancel: bool,
    },
}
struct Outgoing {
    generation: u64,
    announced: bool,
    options: StrokeOptions,
    stroke: pb::Stroke,
    last: Instant,
    dirty: bool,
}
struct ObjectOutgoing {
    generation: u64,
    announced: bool,
    value: ObjectPreview,
    template: Option<pb::ObjectState>,
    last: Instant,
    dirty: bool,
}
#[derive(Clone)]
pub(super) struct Preview {
    pub document: Id,
    pub state: pb::ObjectState,
    pub new_object: bool,
    last: Instant,
    seq: u32,
    total: usize,
    repairing: bool,
}
#[derive(Default)]
pub(super) struct Previews {
    pub sequence: u64,
    pub received: BTreeMap<Id, Preview>,
    sent: BTreeMap<Id, Outgoing>,
    objects: BTreeMap<Id, ObjectOutgoing>,
    next_generation: u64,
    remote_generation: u64,
    remote: BTreeMap<Id, Remote>,
}
struct Remote {
    generation: u64,
    closed: bool,
    opening_hash: AssetId,
    binding: pb::GestureUpdate,
}
/// Compact binding captured from a batch and used only after its exact journal
/// entries have passed authenticated verification and durable installation.
pub(super) struct AcceptedGesture {
    gesture: Id,
    transaction: Id,
    author: DeviceId,
    objects: BTreeSet<Id>,
}
impl AcceptedGesture {
    pub fn from_transaction(txn: &pb::Transaction) -> SessionResult<Option<Self>> {
        let Some(gesture) = txn.gesture_id.as_ref() else {
            return Ok(None);
        };
        let mut objects = BTreeSet::new();
        for op in &txn.ops {
            let object = match &op.kind {
                Some(pb::op::Kind::CreateObject(value)) => value
                    .state
                    .as_ref()
                    .and_then(|state| state.object_id.as_ref()),
                Some(pb::op::Kind::SetProperty(value)) => value.object_id.as_ref(),
                Some(pb::op::Kind::DeleteObject(value)) => value.object_id.as_ref(),
                _ => None,
            };
            if let Some(object) = object {
                objects.insert(Id::from_proto(Some(object)).map_err(|_| SessionError::Invalid)?);
            }
        }
        Ok(Some(Self {
            gesture: Id::from_proto(Some(gesture)).map_err(|_| SessionError::Invalid)?,
            transaction: Id::from_proto(txn.txn_id.as_ref()).map_err(|_| SessionError::Invalid)?,
            author: DeviceId::try_from(txn.device_id.clone()).map_err(|_| SessionError::Invalid)?,
            objects,
        }))
    }
}
impl Previews {
    pub fn clear(&mut self) {
        self.received.clear();
        self.sent.clear();
        self.objects.clear();
        self.remote.clear();
        self.next_generation = 0;
        self.remote_generation = 0;
        self.sequence = self.sequence.saturating_add(1);
    }
    pub fn finish(&mut self, id: Id, cancel: bool) -> SessionResult<Vec<Body>> {
        // Announce every preceding generation before closing one; ID sorting
        // must never reorder reliable openings assigned by local admission.
        let mut output = self.openings()?;
        let generation = self
            .sent
            .remove(&id)
            .map(|v| v.generation)
            .or_else(|| self.objects.remove(&id).map(|v| v.generation));
        if let Some(generation) = generation {
            output.push(Body::GestureAbort(pb::GestureCancel {
                gesture_id: Some(id.to_proto()),
                generation,
                committed: !cancel,
                reason: if cancel { "cancelled" } else { "finished" }.into(),
            }));
        }
        self.sequence = self.sequence.saturating_add(1);
        Ok(output)
    }
    pub fn finish_remote(&mut self, id: Id) -> SessionResult<()> {
        if self.sent.contains_key(&id) || self.objects.contains_key(&id) {
            return Err(SessionError::Authentication);
        }
        self.received.remove(&id);
        if let Some(remote) = self.remote.get_mut(&id) {
            remote.closed = true;
        }
        self.sequence = self.sequence.saturating_add(1);
        Ok(())
    }
    pub fn finish_accepted(
        &mut self,
        accepted: AcceptedGesture,
        local: &DeviceId,
    ) -> SessionResult<Vec<Body>> {
        if &accepted.author != local {
            self.finish_remote(accepted.gesture)?;
            return Ok(Vec::new());
        }
        if self.remote.contains_key(&accepted.gesture) {
            return Err(SessionError::Authentication);
        }
        if let Some(sent) = self.sent.get(&accepted.gesture)
            && (sent.options.transaction_id != accepted.transaction.as_str()
                || !accepted.objects.contains(
                    &Id::try_from(sent.options.object_id.clone())
                        .map_err(|_| SessionError::Invalid)?,
                ))
        {
            return Err(SessionError::Authentication);
        }
        if let Some(sent) = self.objects.get(&accepted.gesture)
            && !accepted.objects.contains(
                &Id::try_from(sent.value.object_id.clone()).map_err(|_| SessionError::Invalid)?,
            )
        {
            return Err(SessionError::Authentication);
        }
        self.finish(accepted.gesture, false)
    }
    /// Only reliable CONTROL can admit a generation. The durable lookup handles
    /// an OPS acceptance which overtook this opening on another channel.
    pub fn open(&mut self, update: &pb::GestureUpdate, closed: bool) -> SessionResult<bool> {
        let id = Id::from_proto(update.gesture_id.as_ref()).map_err(|_| SessionError::Invalid)?;
        if update.generation == 0 {
            return Err(SessionError::Invalid);
        }
        if update.encoded_len() > vw_net::payload_limit(pb::Channel::Control) {
            return Err(SessionError::Backpressure);
        }
        let opening_hash = AssetId::hash(&update.encode_to_vec());
        if update.generation <= self.remote_generation {
            if self.remote.get(&id).is_some_and(|v| {
                v.generation == update.generation && v.opening_hash != opening_hash
            }) || self
                .remote
                .iter()
                .any(|(prior, v)| v.generation == update.generation && prior != &id)
            {
                return Err(SessionError::Authentication);
            }
            return Ok(false);
        }
        if update.generation
            != self
                .remote_generation
                .checked_add(1)
                .ok_or(SessionError::Invalid)?
        {
            return Err(SessionError::Invalid);
        }
        if self.sent.contains_key(&id)
            || self.objects.contains_key(&id)
            || self.remote.contains_key(&id)
        {
            return Err(SessionError::Authentication);
        }
        if self.remote.len() >= 4 {
            return Err(SessionError::Backpressure);
        }
        self.remote_generation = update.generation;
        let binding = pb::GestureUpdate {
            document_id: update.document_id.clone(),
            kind: update.kind,
            target_object_id: update.target_object_id.clone(),
            preview_state: update.preview_state.clone(),
            ..Default::default()
        };
        self.remote.insert(
            id,
            Remote {
                generation: update.generation,
                closed,
                opening_hash,
                binding,
            },
        );
        Ok(!closed)
    }
    pub fn admitted(&self, update: &pb::GestureUpdate) -> SessionResult<bool> {
        let id = Id::from_proto(update.gesture_id.as_ref()).map_err(|_| SessionError::Invalid)?;
        if update.generation == 0 {
            return Err(SessionError::Invalid);
        }
        if self.sent.contains_key(&id) || self.objects.contains_key(&id) {
            return Err(SessionError::Authentication);
        }
        let Some(remote) = self
            .remote
            .get(&id)
            .filter(|v| v.generation == update.generation && !v.closed)
        else {
            return Ok(false);
        };
        if remote.binding.document_id != update.document_id
            || remote.binding.kind != update.kind
            || remote.binding.target_object_id != update.target_object_id
            || !(if update.kind == pb::GestureKind::Shape as i32 {
                same_template_binding(
                    remote.binding.preview_state.as_ref(),
                    update.preview_state.as_ref(),
                )
            } else {
                remote.binding.preview_state == update.preview_state
            })
        {
            return Err(SessionError::Authentication);
        }
        Ok(true)
    }
    /// A true result binds this exact authenticated ID/generation to its first
    /// retirement. Stale/duplicate closes cannot cancel another authored ID.
    pub fn retire(&mut self, close: &pb::GestureCancel) -> SessionResult<bool> {
        let id = Id::from_proto(close.gesture_id.as_ref()).map_err(|_| SessionError::Invalid)?;
        if close.generation == 0 || close.generation > self.remote_generation {
            return Err(SessionError::Invalid);
        }
        if self.sent.contains_key(&id) || self.objects.contains_key(&id) {
            return Err(SessionError::Authentication);
        }
        let Some(remote) = self.remote.get(&id) else {
            if self
                .remote
                .values()
                .any(|v| v.generation == close.generation)
            {
                return Err(SessionError::Authentication);
            }
            return Ok(false);
        };
        if remote.generation != close.generation {
            return Err(SessionError::Authentication);
        }
        self.remote.remove(&id);
        if close.committed {
            // Closing the volatile admission is not proof of a durable commit.
            // Keep a bounded visual grace until OPS swaps it, expiry, or a new
            // active preview needs this retired slot. No late packet refreshes it.
            if let Some(preview) = self.received.get_mut(&id) {
                preview.last = Instant::now();
            }
        } else {
            self.received.remove(&id);
        }
        self.sequence = self.sequence.saturating_add(1);
        Ok(true)
    }
    fn reserve_visual(&mut self, id: &Id) -> SessionResult<()> {
        if self.received.contains_key(id) || self.received.len() < 4 {
            return Ok(());
        }
        let retired = self
            .received
            .iter()
            .filter(|(id, _)| !self.remote.contains_key(*id))
            .min_by_key(|(_, preview)| preview.last)
            .map(|(id, _)| id.clone());
        if let Some(retired) = retired {
            self.received.remove(&retired);
            Ok(())
        } else {
            Err(SessionError::Backpressure)
        }
    }
    pub fn expire(&mut self) {
        let before = self.received.len();
        self.received
            .retain(|_, value| value.last.elapsed() < Duration::from_secs(1));
        if before != self.received.len() {
            self.sequence = self.sequence.saturating_add(1);
        }
    }
    pub fn record(
        &mut self,
        options: StrokeOptions,
        batch: SampleBatch,
        offset: u32,
        local: &DeviceId,
    ) -> SessionResult<()> {
        if options.device_id != local.as_str() {
            return Err(SessionError::Authentication);
        }
        let id = Id::try_from(options.gesture_id.clone()).map_err(|_| SessionError::Invalid)?;
        if batch.x.is_empty() || batch.x.len() > 512 {
            return Err(SessionError::Invalid);
        }
        let delta = pb::Stroke {
            x: batch.x,
            y: batch.y,
            t_ms: batch.t_ms,
            pressure: batch.pressure,
            tilt: batch.tilt,
            orientation: batch.orientation,
            brush: Some(brush(&options)?),
        };
        vw_model::validate_stroke(&delta).map_err(|_| SessionError::Invalid)?;
        if self.objects.contains_key(&id) || self.remote.contains_key(&id) {
            return Err(SessionError::Invalid);
        }
        if !self.sent.contains_key(&id) {
            // A reconnect discards volatile prefixes while the real native
            // StrokeGesture still owns its complete samples. Later suffixes
            // cannot invent that prefix or tear down the healthy new carrier.
            if offset != 0 {
                return Ok(());
            }
            if self.sent.len() + self.objects.len() >= 4 {
                return Err(SessionError::Backpressure);
            }
            self.sent.insert(
                id.clone(),
                Outgoing {
                    generation: self
                        .next_generation
                        .checked_add(1)
                        .ok_or(SessionError::Invalid)?,
                    announced: false,
                    options: options.clone(),
                    stroke: pb::Stroke {
                        brush: delta.brush.clone(),
                        ..Default::default()
                    },
                    last: Instant::now() - Duration::from_secs(1),
                    dirty: false,
                },
            );
            self.next_generation += 1;
        }
        let sent = self.sent.get_mut(&id).ok_or(SessionError::Invalid)?;
        if sent.options.transaction_id != options.transaction_id
            || sent.options.object_id != options.object_id
            || sent.options.document_id != options.document_id
            || sent.options.layer_id != options.layer_id
            || sent.options.rgba != options.rgba
            || sent.stroke.brush != delta.brush
        {
            return Err(SessionError::Invalid);
        }
        append(&mut sent.stroke, &delta, offset as usize)?;
        sent.dirty = true;
        Ok(())
    }
    pub fn record_object(&mut self, value: ObjectPreview) -> SessionResult<()> {
        self.record_object_template(value, None)
    }
    pub fn record_new_object(
        &mut self,
        value: NewObjectPreview,
        local: &DeviceId,
    ) -> SessionResult<()> {
        let template = new_object_template(&value, local)?;
        self.record_object_template(
            ObjectPreview {
                gesture_id: value.gesture_id,
                document_id: value.document_id,
                object_id: value.object_id,
                sequence: value.sequence,
                kind: PreviewKind::Shape,
                transform: value.transform,
                style: value.style,
            },
            Some(template),
        )
    }
    fn record_object_template(
        &mut self,
        value: ObjectPreview,
        template: Option<pb::ObjectState>,
    ) -> SessionResult<()> {
        let id = Id::try_from(value.gesture_id.clone()).map_err(|_| SessionError::Invalid)?;
        if value.sequence == 0 {
            return Err(SessionError::Invalid);
        }
        if self.sent.contains_key(&id) || self.remote.contains_key(&id) {
            return Err(SessionError::Invalid);
        }
        object_packet_with_template(value.clone(), template.as_ref())?;
        if let Some(prior) = self.objects.get_mut(&id) {
            if prior.value.object_id != value.object_id
                || prior.value.document_id != value.document_id
                || !same_template_binding(prior.template.as_ref(), template.as_ref())
                || std::mem::discriminant(&prior.value.kind) != std::mem::discriminant(&value.kind)
            {
                return Err(SessionError::Invalid);
            }
            if value.sequence > prior.value.sequence {
                prior.value = value;
                prior.template = template;
                prior.dirty = true;
            }
            return Ok(());
        }
        if self.sent.len() + self.objects.len() >= 4 {
            return Err(SessionError::Backpressure);
        }
        self.objects.insert(
            id,
            ObjectOutgoing {
                generation: self
                    .next_generation
                    .checked_add(1)
                    .ok_or(SessionError::Invalid)?,
                announced: false,
                value,
                template,
                last: Instant::now() - Duration::from_secs(1),
                dirty: true,
            },
        );
        self.next_generation += 1;
        Ok(())
    }
    fn openings(&mut self) -> SessionResult<Vec<Body>> {
        let mut openings = Vec::new();
        for (id, sent) in &mut self.sent {
            if !sent.announced {
                openings.push((
                    sent.generation,
                    packet(id, sent, 0, sent.stroke.x.len().min(8))?,
                ));
                sent.announced = true;
            }
        }
        for sent in self.objects.values_mut() {
            if !sent.announced {
                let Body::GestureUpdate(mut update) =
                    object_packet_with_template(sent.value.clone(), sent.template.as_ref())?
                else {
                    return Err(SessionError::Invalid);
                };
                update.generation = sent.generation;
                openings.push((sent.generation, *update));
                sent.announced = true;
            }
        }
        openings.sort_by_key(|(generation, _)| *generation);
        Ok(openings
            .into_iter()
            .map(|(_, update)| {
                Body::GestureReplay(Box::new(pb::GestureReplay {
                    update: Some(update),
                    open: true,
                }))
            })
            .collect())
    }
    pub fn updates(&mut self) -> SessionResult<Vec<Body>> {
        let mut output = self.openings()?;
        for (id, sent) in &mut self.sent {
            if !sent.dirty || sent.last.elapsed() < Duration::from_nanos(8_333_334) {
                continue;
            }
            let end = sent.stroke.x.len();
            let mut start = end.saturating_sub(8);
            loop {
                let update = packet(id, sent, start, end)?;
                let frame = pb::Envelope {
                    channel: pb::Channel::Ephemeral as i32,
                    seq: u64::MAX,
                    connection_id: Some(id.to_proto()),
                    body: Some(Body::GestureUpdate(Box::new(update.clone()))),
                };
                if frame.encoded_len() <= vw_net::payload_limit(pb::Channel::Ephemeral) {
                    output.push(Body::GestureUpdate(Box::new(update)));
                    break;
                }
                if start + 1 >= end {
                    return Err(SessionError::Backpressure);
                }
                start += 1;
            }
            sent.last = Instant::now();
            sent.dirty = false;
        }
        for sent in self.objects.values_mut() {
            if sent.dirty && sent.last.elapsed() >= Duration::from_nanos(8_333_334) {
                let Body::GestureUpdate(mut update) =
                    object_packet_with_template(sent.value.clone(), sent.template.as_ref())?
                else {
                    return Err(SessionError::Invalid);
                };
                update.generation = sent.generation;
                output.push(Body::GestureUpdate(update));
                sent.last = Instant::now();
                sent.dirty = false;
            }
        }
        Ok(output)
    }
    pub fn repair(&self, request: pb::GestureRepair) -> SessionResult<Option<Body>> {
        let id = Id::from_proto(request.gesture_id.as_ref()).map_err(|_| SessionError::Invalid)?;
        if request.generation == 0 || request.max_samples == 0 || request.max_samples > 512 {
            return Err(SessionError::Invalid);
        }
        let Some(sent) = self.sent.get(&id) else {
            return Ok(None);
        };
        if sent.generation != request.generation {
            return Ok(None);
        }
        let start = request.first_sample as usize;
        if start >= sent.stroke.x.len() {
            return Err(SessionError::Invalid);
        }
        let end = (start + request.max_samples as usize).min(sent.stroke.x.len());
        Ok(Some(Body::GestureReplay(Box::new(pb::GestureReplay {
            update: Some(packet(&id, sent, start, end)?),
            open: false,
        }))))
    }
    pub fn update(
        &mut self,
        update: pb::GestureUpdate,
        peer: &DeviceId,
        reliable: bool,
    ) -> SessionResult<Option<Body>> {
        let id = Id::from_proto(update.gesture_id.as_ref()).map_err(|_| SessionError::Invalid)?;
        let document =
            Id::from_proto(update.document_id.as_ref()).map_err(|_| SessionError::Invalid)?;
        if self.sent.contains_key(&id) || self.objects.contains_key(&id) {
            return Err(SessionError::Authentication);
        }
        if !self.admitted(&update)? {
            return Ok(None);
        }
        if update.seq == 0 || update.seq > 100_000 {
            return Err(SessionError::Invalid);
        }
        if update.kind != pb::GestureKind::Stroke as i32 {
            return Err(SessionError::Invalid);
        }
        let delta = update.stroke_delta.as_ref().ok_or(SessionError::Invalid)?;
        vw_model::validate_stroke(delta).map_err(|_| SessionError::Invalid)?;
        if delta.x.len() > 512 {
            return Err(SessionError::Invalid);
        }
        let end = (update.sample_offset as usize)
            .checked_add(delta.x.len())
            .ok_or(SessionError::Invalid)?;
        if end > 100_000 || (update.seq as usize) < end {
            return Err(SessionError::Invalid);
        }
        if !self.received.contains_key(&id) {
            self.reserve_visual(&id)?;
            if update.sample_offset > 0 {
                return Ok(Some(repair_request(&id, update.generation, 0)));
            }
            let mut state = update.preview_state.clone().ok_or(SessionError::Invalid)?;
            vw_model::validate_object_state(&state).map_err(|_| SessionError::Invalid)?;
            if state.created_by != peer.as_str() {
                return Err(SessionError::Authentication);
            }
            state.shape = Some(pb::object_state::Shape::Stroke(pb::Stroke {
                brush: delta.brush.clone(),
                ..Default::default()
            }));
            self.received.insert(
                id.clone(),
                Preview {
                    document: document.clone(),
                    state,
                    new_object: true,
                    last: Instant::now(),
                    seq: 0,
                    total: 0,
                    repairing: false,
                },
            );
        }
        let preview = self.received.get_mut(&id).ok_or(SessionError::Invalid)?;
        if preview.document != document {
            return Err(SessionError::Invalid);
        }
        let Some(pb::object_state::Shape::Stroke(stroke)) = &mut preview.state.shape else {
            return Err(SessionError::Invalid);
        };
        if stroke.brush != delta.brush {
            return Err(SessionError::Invalid);
        }
        if reliable && !preview.repairing && update.sample_offset as usize > stroke.x.len() {
            return Err(SessionError::Invalid);
        }
        preview.total = preview.total.max(update.seq as usize);
        preview.seq = preview.seq.max(update.seq);
        if update.sample_offset as usize > stroke.x.len() {
            if preview.repairing {
                return Ok(None);
            }
            preview.repairing = true;
            return Ok(Some(repair_request(
                &id,
                update.generation,
                stroke.x.len() as u32,
            )));
        }
        append(stroke, delta, update.sample_offset as usize)?;
        preview.last = Instant::now();
        preview.repairing = false;
        self.sequence = self.sequence.saturating_add(1);
        if stroke.x.len() < preview.total {
            preview.repairing = true;
            Ok(Some(repair_request(
                &id,
                update.generation,
                stroke.x.len() as u32,
            )))
        } else {
            Ok(None)
        }
    }
    pub fn object(
        &mut self,
        update: pb::GestureUpdate,
        mut state: pb::ObjectState,
        document: Id,
        new_object: bool,
    ) -> SessionResult<()> {
        let id = Id::from_proto(update.gesture_id.as_ref()).map_err(|_| SessionError::Invalid)?;
        if self.sent.contains_key(&id) || self.objects.contains_key(&id) {
            return Err(SessionError::Authentication);
        }
        if !self.admitted(&update)? {
            return Ok(());
        }
        if update.seq == 0
            || !matches!(
                pb::GestureKind::try_from(update.kind),
                Ok(pb::GestureKind::HandleDrag | pb::GestureKind::Slider | pb::GestureKind::Shape)
            )
        {
            return Err(SessionError::Invalid);
        }
        if self.received.get(&id).is_some_and(|p| p.seq >= update.seq) {
            return Ok(());
        }
        self.reserve_visual(&id)?;
        state.transform = update.preview_transform;
        state.style = update.preview_style;
        vw_model::validate_object_state(&state).map_err(|_| SessionError::Invalid)?;
        self.received.insert(
            id,
            Preview {
                document,
                state,
                new_object,
                last: Instant::now(),
                seq: update.seq,
                total: 0,
                repairing: false,
            },
        );
        self.sequence = self.sequence.saturating_add(1);
        Ok(())
    }
}
/// New shape geometry may evolve, while every authority-bearing field and the
/// tool kind remain fixed. Stroke templates still use exact comparison above.
fn same_template_binding(a: Option<&pb::ObjectState>, b: Option<&pb::ObjectState>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            let (Some(first), Some(second)) = (&a.shape, &b.shape) else {
                return false;
            };
            if std::mem::discriminant(first) != std::mem::discriminant(second) {
                return false;
            }
            let mut normalized = b.clone();
            normalized.shape = a.shape.clone();
            a == &normalized
        }
        _ => false,
    }
}
pub(super) fn new_object_template(
    value: &NewObjectPreview,
    local: &DeviceId,
) -> SessionResult<pb::ObjectState> {
    let template = pb::ObjectState {
        object_id: Some(
            Id::try_from(value.object_id.clone())
                .map_err(|_| SessionError::Invalid)?
                .to_proto(),
        ),
        layer_id: Some(
            Id::try_from(value.layer_id.clone())
                .map_err(|_| SessionError::Invalid)?
                .to_proto(),
        ),
        order_key: "V".into(),
        transform: Some(pb::Affine {
            a: 1.0,
            d: 1.0,
            ..Default::default()
        }),
        style: Some(pb::Style {
            stroke: Some(pb::Color { rgba: 0x000000ff }),
            width: 1.0,
            ..Default::default()
        }),
        role: pb::Role::None as i32,
        created_by: local.to_string(),
        created_at_ms: value.created_at_ms,
        shape: Some(crate::editor::shape(value.shape.clone())?),
        ..Default::default()
    };
    vw_model::validate_object_state(&template).map_err(|_| SessionError::Invalid)?;
    Ok(template)
}
pub(super) fn validate_new_preview(
    value: &NewObjectPreview,
    local: &DeviceId,
) -> SessionResult<()> {
    if value.sequence == 0 {
        return Err(SessionError::Invalid);
    }
    let template = new_object_template(value, local)?;
    let mut visible = template.clone();
    visible.transform = Some(crate::editor::affine(value.transform)?);
    visible.style = Some(style(value.style.clone()));
    vw_model::validate_object_state(&visible).map_err(|_| SessionError::Invalid)?;
    object_packet_with_template(
        ObjectPreview {
            gesture_id: value.gesture_id.clone(),
            document_id: value.document_id.clone(),
            object_id: value.object_id.clone(),
            sequence: value.sequence,
            kind: PreviewKind::Shape,
            transform: value.transform,
            style: value.style.clone(),
        },
        Some(&template),
    )?;
    Ok(())
}
fn object_packet_with_template(
    value: ObjectPreview,
    template: Option<&pb::ObjectState>,
) -> SessionResult<Body> {
    let Body::GestureUpdate(mut update) = object_packet(value)? else {
        return Err(SessionError::Invalid);
    };
    update.preview_state = template.cloned();
    // Charge the actual complete datagram plus worst-case counters before any
    // generation is consumed. Large shapes remain ordinary reliable edits.
    let mut probe = update.clone();
    probe.generation = u64::MAX;
    probe.seq = u32::MAX;
    let frame = pb::Envelope {
        channel: pb::Channel::Ephemeral as i32,
        seq: u64::MAX,
        connection_id: probe.gesture_id.clone(),
        body: Some(Body::GestureUpdate(probe)),
    };
    if frame.encoded_len() > vw_net::payload_limit(pb::Channel::Ephemeral) {
        return Err(SessionError::Backpressure);
    }
    Ok(Body::GestureUpdate(update))
}
pub(super) fn object_packet(value: ObjectPreview) -> SessionResult<Body> {
    let id = |value: String| {
        Id::try_from(value)
            .map(|id| id.to_proto())
            .map_err(|_| SessionError::Invalid)
    };
    Ok(Body::GestureUpdate(Box::new(pb::GestureUpdate {
        gesture_id: Some(id(value.gesture_id)?),
        document_id: Some(id(value.document_id)?),
        seq: value.sequence,
        kind: match value.kind {
            PreviewKind::HandleDrag => pb::GestureKind::HandleDrag,
            PreviewKind::Slider => pb::GestureKind::Slider,
            PreviewKind::Shape => pb::GestureKind::Shape,
        } as i32,
        target_object_id: Some(id(value.object_id)?),
        preview_transform: Some(crate::editor::affine(value.transform)?),
        preview_style: Some(style(value.style)),
        ..Default::default()
    })))
}
fn repair_request(id: &Id, generation: u64, offset: u32) -> Body {
    Body::GestureRepair(pb::GestureRepair {
        gesture_id: Some(id.to_proto()),
        first_sample: offset,
        max_samples: 512,
        generation,
    })
}
fn brush(options: &StrokeOptions) -> SessionResult<pb::Brush> {
    let mut brush = vw_ink::Brush::new(
        vw_ink::BrushFamily::try_from(options.family.as_str())
            .map_err(|_| SessionError::Invalid)?,
        options.width,
    )
    .map_err(|_| SessionError::Invalid)?;
    if !options.pressure_curve.is_empty() {
        brush.pressure_curve = vw_ink::PressureCurve::new(
            options
                .pressure_curve
                .as_slice()
                .try_into()
                .map_err(|_| SessionError::Invalid)?,
        )
        .map_err(|_| SessionError::Invalid)?;
    }
    brush.stabilization = options.stabilization;
    brush.validate().map_err(|_| SessionError::Invalid)?;
    Ok(brush.to_proto())
}
fn style(value: ObjectStyle) -> pb::Style {
    pb::Style {
        stroke: Some(pb::Color { rgba: value.rgba }),
        width: value.width,
        screen_constant_width: value.screen_constant_width,
        has_fill: value.fill.is_some(),
        fill: value.fill.map(|rgba| pb::Color { rgba }),
    }
}
fn slice(stroke: &pb::Stroke, start: usize, end: usize) -> pb::Stroke {
    pb::Stroke {
        brush: stroke.brush.clone(),
        x: stroke.x[start..end].to_vec(),
        y: stroke.y[start..end].to_vec(),
        t_ms: stroke.t_ms[start..end].to_vec(),
        pressure: stroke.pressure[start..end].to_vec(),
        tilt: if stroke.tilt.is_empty() {
            vec![]
        } else {
            stroke.tilt[start..end].to_vec()
        },
        orientation: if stroke.orientation.is_empty() {
            vec![]
        } else {
            stroke.orientation[start..end].to_vec()
        },
    }
}
fn packet(id: &Id, value: &Outgoing, start: usize, end: usize) -> SessionResult<pb::GestureUpdate> {
    let uuid = |value: &String| {
        Id::try_from(value.clone())
            .map(|id| id.to_proto())
            .map_err(|_| SessionError::Invalid)
    };
    let options = &value.options;
    let template = pb::ObjectState {
        object_id: Some(uuid(&options.object_id)?),
        layer_id: Some(uuid(&options.layer_id)?),
        order_key: "V".into(),
        transform: Some(pb::Affine {
            a: 1.0,
            d: 1.0,
            ..Default::default()
        }),
        style: Some(pb::Style {
            stroke: Some(pb::Color { rgba: options.rgba }),
            width: options.width,
            ..Default::default()
        }),
        role: pb::Role::None as i32,
        created_by: options.device_id.clone(),
        created_at_ms: options.created_at_ms,
        shape: Some(pb::object_state::Shape::Stroke(slice(&value.stroke, 0, 1))),
        ..Default::default()
    };
    Ok(pb::GestureUpdate {
        gesture_id: Some(id.to_proto()),
        generation: value.generation,
        seq: value.stroke.x.len() as u32,
        document_id: Some(uuid(&options.document_id)?),
        kind: pb::GestureKind::Stroke as i32,
        stroke_delta: Some(slice(&value.stroke, start, end)),
        preview_state: Some(template),
        sample_offset: start as u32,
        ..Default::default()
    })
}
fn append(destination: &mut pb::Stroke, delta: &pb::Stroke, offset: usize) -> SessionResult<()> {
    let end = offset
        .checked_add(delta.x.len())
        .ok_or(SessionError::Invalid)?;
    if end > 100_000 || offset > destination.x.len() {
        return Err(SessionError::Invalid);
    }
    let overlap = destination.x.len().min(end) - offset;
    if !destination.x.is_empty()
        && (destination.tilt.is_empty() != delta.tilt.is_empty()
            || destination.orientation.is_empty() != delta.orientation.is_empty())
    {
        return Err(SessionError::Invalid);
    }
    for i in 0..overlap {
        let j = offset + i;
        if destination.x[j].to_bits() != delta.x[i].to_bits()
            || destination.y[j].to_bits() != delta.y[i].to_bits()
            || destination.t_ms[j] != delta.t_ms[i]
            || destination.pressure[j].to_bits() != delta.pressure[i].to_bits()
            || (!delta.tilt.is_empty() && destination.tilt[j].to_bits() != delta.tilt[i].to_bits())
            || (!delta.orientation.is_empty()
                && destination.orientation[j].to_bits() != delta.orientation[i].to_bits())
        {
            return Err(SessionError::Invalid);
        }
    }
    if overlap < delta.x.len()
        && destination
            .t_ms
            .last()
            .is_some_and(|time| *time > delta.t_ms[overlap])
    {
        return Err(SessionError::Invalid);
    }
    destination.x.extend_from_slice(&delta.x[overlap..]);
    destination.y.extend_from_slice(&delta.y[overlap..]);
    destination.t_ms.extend_from_slice(&delta.t_ms[overlap..]);
    destination
        .pressure
        .extend_from_slice(&delta.pressure[overlap..]);
    if !delta.tilt.is_empty() {
        destination.tilt.extend_from_slice(&delta.tilt[overlap..]);
    }
    if !delta.orientation.is_empty() {
        destination
            .orientation
            .extend_from_slice(&delta.orientation[overlap..]);
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    fn id(n: u8) -> Id {
        Id::from_parts(1_700_000_000_000, [n; 10]).unwrap()
    }
    fn peer() -> DeviceId {
        DeviceId::from_bytes([8; 16])
    }
    fn options() -> StrokeOptions {
        StrokeOptions {
            gesture_id: id(1).to_string(),
            transaction_id: id(2).to_string(),
            object_id: id(3).to_string(),
            document_id: id(4).to_string(),
            layer_id: id(5).to_string(),
            device_id: peer().to_string(),
            lamport: 1,
            created_at_ms: 0,
            family: "pen".into(),
            width: 4.0,
            rgba: 0x102030ff,
            stabilization: 0.2,
            pressure_curve: vec![],
        }
    }
    fn batch(start: usize, len: usize) -> SampleBatch {
        SampleBatch {
            sequence: 1,
            x: (start..start + len).map(|i| i as f64 * 0.25).collect(),
            y: (start..start + len).map(|i| (i % 17) as f64).collect(),
            t_ms: (start..start + len).map(|i| i as u32).collect(),
            pressure: vec![0.7; len],
            tilt: vec![0.2; len],
            orientation: vec![1.3; len],
        }
    }
    fn stroke(previews: &Previews) -> &pb::Stroke {
        match previews.received[&id(1)].state.shape.as_ref().unwrap() {
            pb::object_state::Shape::Stroke(stroke) => stroke,
            _ => panic!("wrong fixture shape"),
        }
    }
    #[test]
    fn dropped_initial_delta_repairs_raw_prefix_and_canonical_geometry() {
        let mut sender = Previews::default();
        let mut receiver = Previews::default();
        sender.record(options(), batch(0, 512), 0, &peer()).unwrap();
        sender
            .record(options(), batch(512, 188), 512, &peer())
            .unwrap();
        let updates = sender.updates().unwrap();
        assert_eq!(updates.len(), 2);
        assert!(sender.updates().unwrap().is_empty());
        let Body::GestureReplay(open) = &updates[0] else {
            panic!("missing reliable opening")
        };
        assert!(open.open);
        let Body::GestureUpdate(update) = &updates[1] else {
            panic!("wrong update")
        };
        assert!(update.sample_offset > 0);
        let frame = pb::Envelope {
            channel: pb::Channel::Ephemeral as i32,
            seq: u64::MAX,
            connection_id: Some(id(9).to_proto()),
            body: Some(Body::GestureUpdate(update.clone())),
        };
        assert!(frame.encoded_len() <= 1100);
        // An EPHEMERAL suffix can overtake its reliable opening. It must not
        // allocate state or invent an admission on that independent channel.
        assert!(
            receiver
                .update(*update.clone(), &peer(), false)
                .unwrap()
                .is_none()
        );
        assert!(receiver.received.is_empty());
        let opening = open.update.clone().unwrap();
        assert!(receiver.open(&opening, false).unwrap());
        let mut repair = receiver.update(opening, &peer(), true).unwrap();
        let mut requests = 0;
        while let Some(Body::GestureRepair(request)) = repair {
            let Some(Body::GestureReplay(replay)) = sender.repair(request).unwrap() else {
                panic!("missing replay")
            };
            repair = receiver
                .update(replay.update.unwrap(), &peer(), true)
                .unwrap();
            requests += 1;
            assert!(requests <= 2);
        }
        assert_eq!(stroke(&receiver), &sender.sent[&id(1)].stroke);
        assert_eq!(
            vw_ink::geometry_from_stroke(stroke(&receiver)).unwrap(),
            vw_ink::geometry_from_stroke(&sender.sent[&id(1)].stroke).unwrap()
        );
        receiver.finish_remote(id(1)).unwrap();
        let replay = packet(&id(1), &sender.sent[&id(1)], 0, 8).unwrap();
        assert!(receiver.update(replay, &peer(), true).unwrap().is_none());
        assert!(receiver.received.is_empty());
    }
    #[test]
    fn conflicting_overlap_totals_and_cross_direction_ids_fail_closed() {
        let mut sender = Previews::default();
        sender.record(options(), batch(0, 8), 0, &peer()).unwrap();
        let update = packet(&id(1), &sender.sent[&id(1)], 0, 8).unwrap();
        let mut receiver = Previews::default();
        receiver.open(&update, false).unwrap();
        receiver.update(update.clone(), &peer(), false).unwrap();
        let before = stroke(&receiver).clone();
        let mut changed = update.clone();
        changed.stroke_delta.as_mut().unwrap().tilt[2] = 0.3;
        assert!(receiver.update(changed, &peer(), false).is_err());
        assert_eq!(stroke(&receiver), &before);
        let mut huge = update.clone();
        huge.seq = u32::MAX;
        assert!(receiver.update(huge, &peer(), false).is_err());
        assert_eq!(stroke(&receiver), &before);
        assert!(sender.update(update, &peer(), false).is_err());
        assert!(sender.finish_remote(id(1)).is_err());
        assert!(sender.sent.contains_key(&id(1)));
    }
    #[test]
    fn own_accepted_stroke_before_ui_finish_retires_only_the_exact_bound_gesture() {
        let mut sender = Previews::default();
        sender.record(options(), batch(0, 8), 0, &peer()).unwrap();
        let mut txn = pb::Transaction {
            txn_id: Some(id(9).to_proto()),
            device_id: peer().to_string(),
            gesture_id: Some(id(1).to_proto()),
            ops: vec![pb::Op {
                kind: Some(pb::op::Kind::CreateObject(pb::CreateObject {
                    document_id: Some(id(4).to_proto()),
                    state: Some(pb::ObjectState {
                        object_id: Some(id(3).to_proto()),
                        ..Default::default()
                    }),
                })),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(
            sender
                .finish_accepted(
                    AcceptedGesture::from_transaction(&txn).unwrap().unwrap(),
                    &peer()
                )
                .is_err()
        );
        assert!(sender.sent.contains_key(&id(1)));
        txn.txn_id = Some(id(2).to_proto());
        sender
            .finish_accepted(
                AcceptedGesture::from_transaction(&txn).unwrap().unwrap(),
                &peer(),
            )
            .unwrap();
        assert!(sender.sent.is_empty());
        assert!(sender.finish(id(1), false).unwrap().is_empty());
        assert!(sender.sent.is_empty());
        // A peer cancellation never gains the accepted-local-transaction path.
        let mut active = Previews::default();
        active.record(options(), batch(0, 8), 0, &peer()).unwrap();
        assert!(active.finish_remote(id(1)).is_err());
        assert!(active.sent.contains_key(&id(1)));
    }
    #[test]
    fn hundred_thousand_preview_samples_are_bounded_before_append() {
        let mut sender = Previews::default();
        for start in (0..100_000).step_by(512) {
            sender
                .record(
                    options(),
                    batch(start, (100_000 - start).min(512)),
                    start as u32,
                    &peer(),
                )
                .unwrap();
        }
        assert_eq!(sender.sent[&id(1)].stroke.x.len(), 100_000);
        assert!(
            sender
                .record(options(), batch(100_000, 1), 100_000, &peer())
                .is_err()
        );
        assert_eq!(sender.sent[&id(1)].stroke.x.len(), 100_000);
    }
    #[test]
    fn object_previews_coalesce_and_wait_for_the_next_send_interval() {
        let mut sender = Previews::default();
        let value = ObjectPreview {
            gesture_id: id(1).to_string(),
            document_id: id(4).to_string(),
            object_id: id(3).to_string(),
            sequence: 1,
            kind: PreviewKind::HandleDrag,
            transform: Transform {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                e: 1.0,
                f: 2.0,
            },
            style: ObjectStyle {
                rgba: 0x123456ff,
                width: 2.0,
                screen_constant_width: false,
                fill: None,
            },
        };
        sender.record_object(value.clone()).unwrap();
        let mut latest = value;
        latest.sequence = 2;
        latest.transform.e = 12.0;
        sender.record_object(latest.clone()).unwrap();
        let first = sender.updates().unwrap();
        assert_eq!(first.len(), 2);
        let Body::GestureUpdate(first) = &first[1] else {
            panic!("wrong preview")
        };
        assert_eq!(first.seq, 2);
        assert_eq!(first.preview_transform.as_ref().unwrap().e, 12.0);
        latest.sequence = 3;
        sender.record_object(latest).unwrap();
        assert!(sender.updates().unwrap().is_empty());
        sender.objects.get_mut(&id(1)).unwrap().last = Instant::now() - Duration::from_secs(1);
        assert_eq!(sender.updates().unwrap().len(), 1);
        sender.finish(id(1), false).unwrap();
        assert!(sender.objects.is_empty());
    }
    #[test]
    fn new_shape_keeps_kind_and_authority_while_geometry_changes_then_retires_without_resurrection()
    {
        let value = NewObjectPreview {
            gesture_id: id(1).to_string(),
            document_id: id(4).to_string(),
            object_id: id(3).to_string(),
            layer_id: id(5).to_string(),
            sequence: 1,
            created_at_ms: 1_700_000_000_000,
            shape: crate::NewShape::Rectangle {
                rectangle: crate::QueryRect {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                },
            },
            transform: Transform {
                a: 10.0,
                b: 0.0,
                c: 0.0,
                d: 20.0,
                e: 7.0,
                f: 9.0,
            },
            style: ObjectStyle {
                rgba: 0x123456ff,
                width: 2.0,
                screen_constant_width: false,
                fill: None,
            },
        };
        let mut sender = Previews::default();
        let mut receiver = Previews::default();
        validate_new_preview(&value, &peer()).unwrap();
        let template = new_object_template(&value, &peer()).unwrap();
        assert_eq!(template.role, pb::Role::None as i32);
        let mut unspecified = template;
        unspecified.role = pb::Role::Unspecified as i32;
        assert!(vw_model::validate_object_state(&unspecified).is_err());
        sender.record_new_object(value.clone(), &peer()).unwrap();
        let opening = sender
            .updates()
            .unwrap()
            .into_iter()
            .find_map(|body| {
                if let Body::GestureReplay(replay) = body {
                    replay.update
                } else {
                    None
                }
            })
            .unwrap();
        assert!(receiver.open(&opening, false).unwrap());
        receiver
            .object(
                opening.clone(),
                opening.preview_state.clone().unwrap(),
                id(4),
                true,
            )
            .unwrap();
        let mut changed = value.clone();
        changed.sequence = 2;
        changed.transform.a = 30.0;
        changed.style.width = 4.0;
        sender.record_new_object(changed.clone(), &peer()).unwrap();
        sender.objects.get_mut(&id(1)).unwrap().last = Instant::now() - Duration::from_secs(1);
        let Body::GestureUpdate(update) = sender.updates().unwrap().remove(0) else {
            panic!("wrong update");
        };
        assert_eq!(update.preview_state, opening.preview_state);
        assert!(receiver.admitted(&update).unwrap());
        receiver
            .object(
                *update.clone(),
                update.preview_state.clone().unwrap(),
                id(4),
                true,
            )
            .unwrap();
        assert!(receiver.received[&id(1)].new_object);
        assert_eq!(
            receiver.received[&id(1)]
                .state
                .transform
                .as_ref()
                .unwrap()
                .a,
            30.0
        );
        changed.sequence = 3;
        changed.shape = crate::NewShape::Rectangle {
            rectangle: crate::QueryRect {
                x: 4.0,
                y: 5.0,
                width: 12.0,
                height: 19.0,
            },
        };
        sender.record_new_object(changed.clone(), &peer()).unwrap();
        sender.objects.get_mut(&id(1)).unwrap().last = Instant::now() - Duration::from_secs(1);
        let Body::GestureUpdate(geometry) = sender.updates().unwrap().remove(0) else {
            panic!("wrong geometry");
        };
        assert_ne!(geometry.preview_state, opening.preview_state);
        assert!(receiver.admitted(&geometry).unwrap());
        let mut forged = geometry.clone();
        forged.preview_state.as_mut().unwrap().layer_id = Some(id(7).to_proto());
        assert!(receiver.admitted(&forged).is_err());
        let mut forged = geometry.clone();
        forged.preview_state.as_mut().unwrap().created_by =
            DeviceId::from_bytes([9; 16]).to_string();
        assert!(receiver.admitted(&forged).is_err());
        let mut forged = geometry.clone();
        forged.preview_state.as_mut().unwrap().role = pb::Role::Change as i32;
        assert!(receiver.admitted(&forged).is_err());
        changed.shape = crate::NewShape::Ellipse {
            rectangle: crate::QueryRect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
        };
        assert!(sender.record_new_object(changed, &peer()).is_err());
        let Body::GestureAbort(close) = sender.finish(id(1), true).unwrap().remove(0) else {
            panic!("wrong close");
        };
        assert!(receiver.retire(&close).unwrap());
        assert!(!receiver.admitted(&update).unwrap());
        assert!(receiver.received.is_empty());
        let mut large = value;
        let points: Vec<_> = (0..4096)
            .map(|n| crate::Point {
                x: f64::from(n),
                y: 1.0,
            })
            .collect();
        // A line is exactly two points. Only a valid multi-point arrow can
        // reach the datagram-size guard rather than fail shape validation.
        large.shape = crate::NewShape::Line {
            points: points.clone(),
        };
        assert!(matches!(
            validate_new_preview(&large, &peer()),
            Err(SessionError::Invalid)
        ));
        large.shape = crate::NewShape::Arrow { points };
        assert!(matches!(
            validate_new_preview(&large, &peer()),
            Err(SessionError::Backpressure)
        ));
    }

    fn numbered(n: u64) -> StrokeOptions {
        let mut value = options();
        value.gesture_id = Id::from_parts(1_700_000_000_000 + n, [1; 10])
            .unwrap()
            .to_string();
        value
    }
    fn announce(
        sender: &mut Previews,
        receiver: &mut Previews,
        value: StrokeOptions,
    ) -> pb::GestureUpdate {
        sender.record(value, batch(0, 8), 0, &peer()).unwrap();
        let updates = sender.updates().unwrap();
        let Body::GestureReplay(open) = &updates[0] else {
            panic!("opening")
        };
        let update = open.update.clone().unwrap();
        assert!(open.open);
        assert!(receiver.open(&update, false).unwrap());
        assert!(
            receiver
                .update(update.clone(), &peer(), true)
                .unwrap()
                .is_none()
        );
        update
    }
    fn close(
        sender: &mut Previews,
        receiver: &mut Previews,
        id: Id,
        cancel: bool,
    ) -> pb::GestureCancel {
        let bodies = sender.finish(id, cancel).unwrap();
        assert_eq!(bodies.len(), 1);
        let Body::GestureAbort(close) = &bodies[0] else {
            panic!("close")
        };
        assert!(receiver.retire(close).unwrap());
        close.clone()
    }
    #[test]
    fn ten_thousand_mixed_completions_and_cancels_keep_live_state_bounded_and_never_resurrect() {
        let mut sender = Previews::default();
        let mut receiver = Previews::default();
        let first = announce(&mut sender, &mut receiver, numbered(1));
        let first_id = Id::from_proto(first.gesture_id.as_ref()).unwrap();
        let late_replay = sender
            .repair(pb::GestureRepair {
                gesture_id: first.gesture_id.clone(),
                first_sample: 0,
                max_samples: 8,
                generation: first.generation,
            })
            .unwrap()
            .unwrap();
        let first_close = close(&mut sender, &mut receiver, first_id, true);
        for n in 2..=10_240 {
            let update = announce(&mut sender, &mut receiver, numbered(n));
            let id = Id::from_proto(update.gesture_id.as_ref()).unwrap();
            if n % 2 == 0 {
                receiver.finish_remote(id.clone()).unwrap();
            }
            close(&mut sender, &mut receiver, id, n % 2 != 0);
            assert!(receiver.remote.is_empty());
            assert!(receiver.received.len() <= 4);
            assert!(sender.sent.is_empty());
        }
        // Neither a never-consumed old datagram nor a reliable repair/open can
        // recreate the first identity after arbitrarily many other closures.
        assert!(!receiver.open(&first, false).unwrap());
        assert!(
            receiver
                .update(first.clone(), &peer(), false)
                .unwrap()
                .is_none()
        );
        let Body::GestureReplay(replay) = late_replay else {
            panic!("replay")
        };
        assert!(!replay.open);
        assert!(
            receiver
                .update(replay.update.unwrap(), &peer(), true)
                .unwrap()
                .is_none()
        );
        assert!(!receiver.retire(&first_close).unwrap());
        assert!(receiver.received.is_empty());
        assert_eq!(receiver.remote_generation, 10_240);
    }
    #[test]
    fn early_active_generation_survives_many_newer_closures_and_old_repairs_are_bound() {
        let mut sender = Previews::default();
        let mut receiver = Previews::default();
        let early = announce(&mut sender, &mut receiver, numbered(1));
        for n in 2..=1024 {
            let next = announce(&mut sender, &mut receiver, numbered(n));
            close(
                &mut sender,
                &mut receiver,
                Id::from_proto(next.gesture_id.as_ref()).unwrap(),
                true,
            );
        }
        sender.record(numbered(1), batch(8, 8), 8, &peer()).unwrap();
        let early_id = Id::from_proto(early.gesture_id.as_ref()).unwrap();
        let suffix = packet(&early_id, &sender.sent[&early_id], 8, 16).unwrap();
        receiver.update(suffix, &peer(), false).unwrap();
        assert_eq!(receiver.received[&early_id].total, 16);
        assert_eq!(receiver.remote.len(), 1);
        assert!(
            sender
                .repair(pb::GestureRepair {
                    gesture_id: early.gesture_id.clone(),
                    first_sample: 0,
                    max_samples: 8,
                    generation: 1024,
                })
                .unwrap()
                .is_none()
        );
        assert!(receiver.open(&early, false).is_ok()); // exact duplicate does not reset samples
        assert_eq!(receiver.received[&early_id].total, 16);
        let mut collision = early.clone();
        collision.document_id = Some(id(9).to_proto());
        assert!(receiver.open(&collision, false).is_err());
        assert!(receiver.update(collision, &peer(), false).is_err());
        let mut rebound = early.clone();
        rebound.target_object_id = Some(id(9).to_proto());
        assert!(receiver.update(rebound, &peer(), true).is_err());
        let mut unknown = early;
        unknown.generation = 9999;
        assert!(receiver.update(unknown, &peer(), true).unwrap().is_none());
        assert_eq!(receiver.remote_generation, 1024);
    }
    #[test]
    fn committed_close_before_ops_stops_replays_but_keeps_bounded_visual_grace() {
        let mut sender = Previews::default();
        let mut receiver = Previews::default();
        let opening = announce(&mut sender, &mut receiver, numbered(1));
        let id = Id::from_proto(opening.gesture_id.as_ref()).unwrap();
        let finished = close(&mut sender, &mut receiver, id.clone(), false);
        assert!(finished.committed);
        assert!(receiver.received.contains_key(&id));
        let time = receiver.received[&id].last;
        assert!(receiver.update(opening, &peer(), true).unwrap().is_none());
        assert_eq!(receiver.received[&id].last, time);
        receiver.finish_remote(id.clone()).unwrap(); // exact accepted OPS arrival
        assert!(!receiver.received.contains_key(&id));
        for n in 2..=20 {
            let next = announce(&mut sender, &mut receiver, numbered(n));
            close(
                &mut sender,
                &mut receiver,
                Id::from_proto(next.gesture_id.as_ref()).unwrap(),
                false,
            );
            assert!(receiver.received.len() <= 4);
        }
        for preview in receiver.received.values_mut() {
            preview.last = Instant::now() - Duration::from_secs(2);
        }
        receiver.expire();
        assert!(receiver.received.is_empty());
        assert!(receiver.remote.is_empty());
    }
    #[test]
    fn control_order_and_four_live_slots_are_enforced_without_mutating_on_refusal() {
        let mut sender = Previews::default();
        let mut receiver = Previews::default();
        for n in 1..=4 {
            announce(&mut sender, &mut receiver, numbered(n));
        }
        let first_id = Id::try_from(numbered(1).gesture_id).unwrap();
        let mut future = packet(&first_id, &sender.sent[&first_id], 0, 8).unwrap();
        future.gesture_id = Some(id(9).to_proto());
        future.generation = 5;
        assert!(receiver.open(&future, false).is_err());
        assert_eq!(receiver.remote_generation, 4);
        assert_eq!(receiver.remote.len(), 4);
        let future_close = pb::GestureCancel {
            gesture_id: future.gesture_id.clone(),
            generation: 5,
            ..Default::default()
        };
        assert!(receiver.retire(&future_close).is_err());
        assert_eq!(receiver.remote.len(), 4);
        let forged = pb::GestureCancel {
            generation: 1,
            ..future_close
        };
        assert!(receiver.retire(&forged).is_err());
        future.generation = 0;
        assert!(receiver.open(&future, false).is_err());
        assert!(receiver.update(future, &peer(), false).is_err());
    }
    #[test]
    fn ops_before_open_uses_durable_authority_including_cancelled_history_after_reopen() {
        let root = if cfg!(target_os = "android") {
            tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()
        } else {
            tempfile::tempdir().unwrap()
        };
        let path = root.path().join("authority");
        let mut store = vw_store::ProjectStore::create(
            &path,
            vw_model::Project::new(id(20), "preview authority".into(), peer()),
            peer(),
            0,
        )
        .unwrap();
        let txn = pb::Transaction {
            txn_id: Some(id(2).to_proto()),
            project_id: Some(id(20).to_proto()),
            device_id: peer().to_string(),
            base_revision: Some(store.revision().unwrap()),
            gesture_id: Some(id(1).to_proto()),
            ops: vec![pb::Op {
                op_id: Some(pb::OpId {
                    device_id: peer().to_string(),
                    lamport: 1,
                }),
                kind: Some(pb::op::Kind::CreateDocument(pb::CreateDocument {
                    document_id: Some(id(4).to_proto()),
                    kind: pb::DocumentKind::Image as i32,
                    schema_version: 1,
                    title: "synthetic".into(),
                    ..Default::default()
                })),
            }],
            ..Default::default()
        };
        store.commit(&txn, &peer(), 1).unwrap();
        store.cancel_gesture(peer(), id(7), 2).unwrap();
        drop(store);
        let store = vw_store::ProjectStore::open(&path).unwrap();
        assert!(store.gesture_closed(&peer(), &id(1)));
        assert!(store.gesture_closed(&peer(), &id(7)));
        assert!(!store.gesture_closed(&DeviceId::from_bytes([9; 16]), &id(1)));
        let mut sender = Previews::default();
        let mut receiver = Previews::default();
        sender.record(options(), batch(0, 8), 0, &peer()).unwrap();
        let opening = packet(&id(1), &sender.sent[&id(1)], 0, 8).unwrap();
        sender.updates().unwrap();
        assert!(
            !receiver
                .open(&opening, store.gesture_closed(&peer(), &id(1)))
                .unwrap()
        );
        assert!(receiver.update(opening, &peer(), true).unwrap().is_none());
        assert!(receiver.received.is_empty());
        assert_eq!(receiver.remote_generation, 1);
        let committed = close(&mut sender, &mut receiver, id(1), false);
        assert!(committed.committed);
        assert!(receiver.remote.is_empty());
        // Retiring a preview never writes an undo or cancellation transaction.
        assert_eq!(store.revision().unwrap().host_seq, 1);
        assert_eq!(store.accepted_transactions().count(), 1);
    }
    #[test]
    fn reconnect_suffix_does_not_reopen_partial_stroke_or_fail_the_new_connection() {
        let mut sender = Previews::default();
        sender.record(options(), batch(0, 8), 0, &peer()).unwrap();
        sender.updates().unwrap();
        sender.clear();
        sender.record(options(), batch(8, 8), 8, &peer()).unwrap();
        assert!(sender.sent.is_empty());
        assert!(sender.updates().unwrap().is_empty());
        assert_eq!(sender.next_generation, 0);
        // The real gesture commits through OPS independently. A subsequent new
        // gesture starts a valid generation in this authenticated epoch.
        sender.record(numbered(2), batch(0, 8), 0, &peer()).unwrap();
        let frames = sender.updates().unwrap();
        let Body::GestureReplay(open) = &frames[0] else {
            panic!("opening")
        };
        assert_eq!(open.update.as_ref().unwrap().generation, 1);
        assert!(open.open);
    }
}
