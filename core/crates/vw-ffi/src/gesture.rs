use crate::{worker::Worker, *};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicU8, AtomicUsize, Ordering},
};
use vw_model::{DeviceId, Id, OrderKey};
use vw_proto::v1 as pb;

pub const MAX_BATCH_SAMPLES: usize = 512;
pub const MAX_GESTURE_SAMPLES: usize = 100_000;
const ACTIVE: u8 = 0;
const QUEUED: u8 = 1;
const COMMITTED: u8 = 2;
const CANCELLED: u8 = 3;
const PERSISTING: u8 = 4;

struct GesturePermit(Arc<AtomicUsize>);
impl Drop for GesturePermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct InkState {
    builder: vw_ink::StrokeBuilder,
    stroke: pb::Stroke,
    next_sequence: u64,
    previous: Option<(SampleBatch, InkUpdate)>,
    sealed: bool,
    _permit: GesturePermit,
}
#[derive(uniffi::Object)]
pub struct StrokeGesture {
    project: Weak<ProjectSession>,
    worker: Worker<InkState>,
    options: StrokeOptions,
    base_revision: pb::Revision,
    phase: Arc<AtomicU8>,
    receipt: Arc<Mutex<Option<ProjectInfo>>>,
    highlighter_layer: Option<pb::CreateLayer>,
}
impl StrokeGesture {
    pub(crate) fn session_preview_options(
        &self,
        project: &Arc<ProjectSession>,
    ) -> Result<StrokeOptions> {
        let owner = self.project.upgrade().ok_or(CoreError::Closed)?;
        if !Arc::ptr_eq(&owner, project) || self.phase.load(Ordering::Acquire) != ACTIVE {
            return Err(CoreError::Invalid);
        }
        Ok(self.options.clone())
    }
}
impl Drop for StrokeGesture {
    fn drop(&mut self) {
        let _ = self.cancel();
    }
}
struct CommitWait {
    phase: Arc<AtomicU8>,
}
impl Drop for CommitWait {
    fn drop(&mut self) {
        let _ = self
            .phase
            .compare_exchange(QUEUED, ACTIVE, Ordering::AcqRel, Ordering::Acquire);
    }
}

#[uniffi::export]
impl ProjectSession {
    pub async fn begin_stroke(
        self: Arc<Self>,
        mut options: StrokeOptions,
    ) -> Result<Arc<StrokeGesture>> {
        self.check_open()?;
        for value in [
            &options.gesture_id,
            &options.transaction_id,
            &options.object_id,
            &options.document_id,
            &options.layer_id,
        ] {
            Id::try_from(value.clone())?;
        }
        let device = DeviceId::try_from(options.device_id.clone())?;
        if options.created_at_ms < 0 || options.lamport == 0 {
            return Err(CoreError::Invalid);
        }
        let mut brush = vw_ink::Brush::new(
            vw_ink::BrushFamily::try_from(options.family.as_str())?,
            options.width,
        )?;
        if !options.pressure_curve.is_empty() {
            brush.pressure_curve = vw_ink::PressureCurve::new(
                options
                    .pressure_curve
                    .as_slice()
                    .try_into()
                    .map_err(|_| CoreError::Invalid)?,
            )?;
        }
        brush.stabilization = options.stabilization;
        brush.validate()?;
        self.active_gestures
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 4).then_some(n + 1)
            })
            .map_err(|_| CoreError::Backpressure)?;
        let permit = GesturePermit(self.active_gestures.clone());
        let requested = options.clone();
        let (base_revision, target_layer, highlighter_layer) = self
            .worker
            .call(move |state| {
                let store = state.store()?;
                let project = state.project()?;
                let layer = project
                    .layers
                    .get(&Id::try_from(requested.layer_id.clone())?)
                    .ok_or(CoreError::Invalid)?;
                if Id::from_proto(layer.definition.document_id.as_ref())?.as_str()
                    != requested.document_id
                    || layer.locked
                    || !layer.visible
                    || store.local_device()? != device
                    || project
                        .objects
                        .contains_key(&Id::try_from(requested.object_id)?)
                {
                    return Err(CoreError::Invalid);
                }
                let mut target = requested.layer_id.clone();
                let mut create = None;
                if requested.family == "highlighter" && layer.blend != "multiply" {
                    if let Some((id, _)) = project.layers.iter().find(|(_, l)| {
                        l.definition.document_id == layer.definition.document_id
                            && l.blend == "multiply"
                            && l.visible
                            && !l.locked
                    }) {
                        target = id.to_string();
                    } else {
                        target = requested.gesture_id.clone();
                        let left = project
                            .layers
                            .values()
                            .filter(|l| l.definition.document_id == layer.definition.document_id)
                            .map(|l| l.definition.order_key.as_str())
                            .max()
                            .map(|s| OrderKey::try_from(s.to_owned()))
                            .transpose()?;
                        create = Some(pb::CreateLayer {
                            layer_id: Some(Id::try_from(target.clone())?.to_proto()),
                            document_id: layer.definition.document_id.clone(),
                            page_index: -1,
                            name: "Highlighter".into(),
                            kind: "annotation".into(),
                            order_key: OrderKey::between(left.as_ref(), None)?.as_str().into(),
                        });
                    }
                }
                Ok((store.revision()?, target, create))
            })
            .await?;
        options.layer_id = target_layer;
        self.check_open()?;
        let state = InkState {
            stroke: pb::Stroke {
                brush: Some(brush.to_proto()),
                ..Default::default()
            },
            builder: vw_ink::StrokeBuilder::begin(brush)?,
            next_sequence: 1,
            previous: None,
            sealed: false,
            _permit: permit,
        };
        Ok(Arc::new(StrokeGesture {
            project: Arc::downgrade(&self),
            worker: Worker::new("vw-core-ink", state, 8)?,
            options,
            base_revision,
            highlighter_layer,
            phase: Arc::new(AtomicU8::new(ACTIVE)),
            receipt: Arc::new(Mutex::new(None)),
        }))
    }
}

#[uniffi::export]
impl StrokeGesture {
    /// Seal input and await destruction of the ink worker and its capacity permit.
    /// Unpublished gestures are cancelled. A durable commit already in progress
    /// keeps running on the project worker; shutdown does not roll it back.
    pub async fn shutdown(&self) -> Result<()> {
        match self.cancel() {
            Ok(()) | Err(CoreError::Closed) => {}
            Err(error) => return Err(error),
        }
        self.worker.shutdown(|_| Ok(())).await
    }
    /// Disposable predicted tail. A stale base sequence is rejected and nothing
    /// here changes raw samples, geometry, sequence, or project history.
    pub async fn preview_samples(&self, batch: SampleBatch) -> Result<InkUpdate> {
        self.project
            .upgrade()
            .ok_or(CoreError::Closed)?
            .check_open()?;
        if self.phase.load(Ordering::Acquire) != ACTIVE {
            return Err(CoreError::Closed);
        }
        validate_batch(&batch)?;
        if batch.x.len() > 32 {
            return Err(CoreError::Invalid);
        }
        let phase = self.phase.clone();
        self.worker
            .call(move |state| {
                if phase.load(Ordering::Acquire) != ACTIVE {
                    return Err(CoreError::Closed);
                }
                if state.sealed {
                    return Err(CoreError::Closed);
                }
                if batch.sequence != state.next_sequence {
                    return Err(CoreError::Invalid);
                }
                if state
                    .builder
                    .geometry()
                    .polygons()
                    .iter()
                    .map(|p| p.points.len())
                    .sum::<usize>()
                    > 131072
                {
                    return Err(CoreError::Backpressure);
                }
                let mut prediction = state.builder.clone();
                let samples = (0..batch.x.len())
                    .map(|i| vw_ink::Sample {
                        x: batch.x[i],
                        y: batch.y[i],
                        t_ms: batch.t_ms[i],
                        pressure: f64::from(batch.pressure[i]),
                        tilt: batch.tilt.get(i).map(|v| f64::from(*v)),
                        orientation: batch.orientation.get(i).map(|v| f64::from(*v)),
                    })
                    .collect::<Vec<_>>();
                let range = prediction.append(&samples)?;
                Ok(InkUpdate {
                    sequence: batch.sequence,
                    first_polygon: range.start as u64,
                    sample_count: state.stroke.x.len() as u64,
                    contours: contours(&prediction.geometry().polygons()[range.start..range.end])?,
                })
            })
            .await
    }
    pub async fn append_samples(&self, batch: SampleBatch) -> Result<InkUpdate> {
        self.project
            .upgrade()
            .ok_or(CoreError::Closed)?
            .check_open()?;
        if self.phase.load(Ordering::Acquire) != ACTIVE {
            return Err(CoreError::Closed);
        }
        validate_batch(&batch)?;
        let phase = self.phase.clone();
        self.worker
            .call(move |state| {
                if phase.load(Ordering::Acquire) == CANCELLED {
                    return Err(CoreError::Cancelled);
                }
                if let Some((old, receipt)) = &state.previous
                    && old.sequence == batch.sequence
                {
                    return if *old == batch {
                        Ok(receipt.clone())
                    } else {
                        Err(CoreError::Invalid)
                    };
                }
                if state.sealed {
                    return Err(CoreError::Closed);
                }
                let count = state.stroke.x.len();
                if batch.sequence != state.next_sequence
                    || count
                        .checked_add(batch.x.len())
                        .is_none_or(|n| n > MAX_GESTURE_SAMPLES)
                    || (count > 0
                        && (state.stroke.tilt.is_empty() != batch.tilt.is_empty()
                            || state.stroke.orientation.is_empty() != batch.orientation.is_empty()))
                {
                    return Err(CoreError::Invalid);
                }
                let samples = (0..batch.x.len())
                    .map(|i| vw_ink::Sample {
                        x: batch.x[i],
                        y: batch.y[i],
                        t_ms: batch.t_ms[i],
                        pressure: f64::from(batch.pressure[i]),
                        tilt: batch.tilt.get(i).map(|v| f64::from(*v)),
                        orientation: batch.orientation.get(i).map(|v| f64::from(*v)),
                    })
                    .collect::<Vec<_>>();
                let range = state.builder.append(&samples)?;
                state.stroke.x.extend_from_slice(&batch.x);
                state.stroke.y.extend_from_slice(&batch.y);
                state.stroke.t_ms.extend_from_slice(&batch.t_ms);
                state.stroke.pressure.extend_from_slice(&batch.pressure);
                state.stroke.tilt.extend_from_slice(&batch.tilt);
                state
                    .stroke
                    .orientation
                    .extend_from_slice(&batch.orientation);
                state.next_sequence += 1;
                let output = contours(&state.builder.geometry().polygons()[range.start..range.end])
                    .inspect_err(|_| {
                        phase.store(CANCELLED, Ordering::Release);
                    })?;
                let update = InkUpdate {
                    sequence: batch.sequence,
                    first_polygon: range.start as u64,
                    sample_count: state.stroke.x.len() as u64,
                    contours: output,
                };
                state.previous = Some((batch, update.clone()));
                Ok(update)
            })
            .await
    }
    /// Cancellation succeeds only before the durable commit starts. It discards
    /// all queued input; no cancelled gesture enters project history.
    pub fn cancel(&self) -> Result<()> {
        loop {
            let phase = self.phase.load(Ordering::Acquire);
            match phase {
                CANCELLED => return Ok(()),
                COMMITTED | PERSISTING => return Err(CoreError::Closed),
                _ => {
                    if self
                        .phase
                        .compare_exchange(phase, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                    {
                        return Ok(());
                    }
                }
            }
        }
    }
    pub async fn commit(&self, cancellation: Arc<Cancellation>) -> Result<ProjectInfo> {
        if self.phase.load(Ordering::Acquire) == COMMITTED {
            return self
                .receipt
                .lock()
                .map_err(|_| CoreError::Worker)?
                .clone()
                .ok_or(CoreError::Worker);
        }
        let project = self.project.upgrade().ok_or(CoreError::Closed)?;
        project.check_open()?;
        cancellation.check()?;
        self.phase
            .compare_exchange(ACTIVE, QUEUED, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|phase| {
                if phase == CANCELLED {
                    CoreError::Cancelled
                } else {
                    CoreError::Closed
                }
            })?;
        let _wait = CommitWait {
            phase: self.phase.clone(),
        };
        let stroke = self
            .worker
            .call(|state| {
                if state.stroke.x.is_empty() {
                    Err(CoreError::Invalid)
                } else {
                    state.sealed = true;
                    Ok(state.stroke.clone())
                }
            })
            .await?;
        cancellation.check()?;
        let options = self.options.clone();
        let base = self.base_revision.clone();
        let highlighter_layer = self.highlighter_layer.clone();
        let phase = self.phase.clone();
        let receipt = self.receipt.clone();
        let closed = project.closed.clone();
        project
            .worker
            .call(move |state| {
                cancellation.check()?;
                if closed.load(Ordering::Acquire) {
                    return Err(CoreError::Closed);
                }
                let model = state.project()?;
                let layer_id = Id::try_from(options.layer_id)?;
                let left = model
                    .objects
                    .values()
                    .filter(|o| o.state.layer_id.as_ref() == Some(&layer_id.to_proto()))
                    .map(|o| o.state.order_key.as_str())
                    .max()
                    .map(|key| OrderKey::try_from(key.to_owned()))
                    .transpose()?;
                let order = OrderKey::between(left.as_ref(), None)?;
                let object = pb::ObjectState {
                    object_id: Some(Id::try_from(options.object_id)?.to_proto()),
                    layer_id: Some(layer_id.to_proto()),
                    order_key: order.as_str().into(),
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
                    shape: Some(pb::object_state::Shape::Stroke(stroke)),
                    ..Default::default()
                };
                let device = DeviceId::try_from(options.device_id)?;
                let mut kinds = Vec::new();
                if let Some(layer) = highlighter_layer {
                    kinds.push(pb::op::Kind::CreateLayer(layer.clone()));
                    kinds.push(pb::op::Kind::UpdateLayer(pb::UpdateLayer {
                        layer_id: layer.layer_id,
                        blend: Some("multiply".into()),
                        ..Default::default()
                    }));
                }
                kinds.push(pb::op::Kind::CreateObject(pb::CreateObject {
                    document_id: Some(Id::try_from(options.document_id)?.to_proto()),
                    state: Some(object),
                }));
                let ops = kinds
                    .into_iter()
                    .enumerate()
                    .map(|(offset, kind)| {
                        Ok(pb::Op {
                            op_id: Some(pb::OpId {
                                device_id: device.to_string(),
                                lamport: options
                                    .lamport
                                    .checked_add(offset as u64)
                                    .ok_or(CoreError::Invalid)?,
                            }),
                            kind: Some(kind),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let txn = pb::Transaction {
                    txn_id: Some(Id::try_from(options.transaction_id)?.to_proto()),
                    project_id: Some(model.id.to_proto()),
                    device_id: device.to_string(),
                    base_revision: Some(base),
                    created_at_wall_ms: options.created_at_ms,
                    gesture_id: Some(Id::try_from(options.gesture_id)?.to_proto()),
                    ops,
                };
                cancellation.check()?;
                phase
                    .compare_exchange(QUEUED, PERSISTING, Ordering::AcqRel, Ordering::Acquire)
                    .map_err(|_| CoreError::Cancelled)?;
                match state.commit(&txn, &device, options.created_at_ms) {
                    Ok(info) => {
                        *receipt.lock().map_err(|_| CoreError::Worker)? = Some(info.clone());
                        phase.store(COMMITTED, Ordering::Release);
                        Ok(info)
                    }
                    Err(error) => {
                        phase.store(ACTIVE, Ordering::Release);
                        Err(error)
                    }
                }
            })
            .await
    }
}
fn validate_batch(batch: &SampleBatch) -> Result<()> {
    let n = batch.x.len();
    if batch.sequence == 0
        || batch.sequence == u64::MAX
        || n == 0
        || n > MAX_BATCH_SAMPLES
        || batch.y.len() != n
        || batch.t_ms.len() != n
        || batch.pressure.len() != n
        || (!batch.tilt.is_empty() && batch.tilt.len() != n)
        || (!batch.orientation.is_empty() && batch.orientation.len() != n)
    {
        return Err(CoreError::Invalid);
    }
    Ok(())
}
pub(crate) fn contours(polygons: &[vw_ink::Polygon]) -> Result<Contours> {
    let count = polygons.iter().try_fold(0usize, |n, polygon| {
        n.checked_add(polygon.points.len())
            .ok_or(CoreError::Invalid)
    })?;
    if count > vw_ink::MAX_VERTICES {
        return Err(CoreError::Invalid);
    }
    let mut result = Contours::default();
    result
        .x
        .try_reserve_exact(count)
        .map_err(|_| CoreError::Worker)?;
    result
        .y
        .try_reserve_exact(count)
        .map_err(|_| CoreError::Worker)?;
    for polygon in polygons {
        for point in &polygon.points {
            result.x.push(point.x);
            result.y.push(point.y);
        }
        result
            .ends
            .push(u32::try_from(result.x.len()).map_err(|_| CoreError::Invalid)?);
    }
    Ok(result)
}
