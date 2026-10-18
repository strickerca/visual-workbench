use crate::{project::ProjectState, *};
use std::{collections::BTreeMap, sync::Arc};
use vw_model::{DeviceId, Id, OrderKey};
use vw_ops::{Acceptance, UndoManager};
use vw_proto::v1 as pb;

#[derive(Clone)]
pub(crate) struct History {
    pub undo: UndoManager,
    pub next_lamport: u64,
    directions: BTreeMap<Id, bool>,
    device: DeviceId,
}
impl History {
    pub fn restore(store: &vw_store::ProjectStore) -> Result<Self> {
        let device = store.local_device()?;
        let mut result = Self {
            undo: UndoManager::new(device.clone()),
            next_lamport: 1,
            directions: BTreeMap::new(),
            device,
        };
        let mut rows = store.accepted_transactions().collect::<Vec<_>>();
        rows.sort_by_key(|(_, ack)| ack.host_seq);
        for (txn, ack) in rows {
            result.accept(
                txn,
                &Acceptance {
                    ack: ack.clone(),
                    conflicts: vec![],
                    duplicate: false,
                },
            )?;
        }
        for txn in store.pending()? {
            result.pending(&txn)?;
        }
        Ok(result)
    }
    pub fn accept(&mut self, txn: &pb::Transaction, receipt: &Acceptance) -> Result<()> {
        for op in &txn.ops {
            self.next_lamport = self.next_lamport.max(
                op.op_id
                    .as_ref()
                    .ok_or(CoreError::Invalid)?
                    .lamport
                    .checked_add(1)
                    .ok_or(CoreError::Invalid)?,
            );
        }
        if txn.device_id != self.device.as_str() {
            return Ok(());
        }
        let id = Id::from_proto(txn.txn_id.as_ref())?;
        if let [
            pb::Op {
                kind: Some(pb::op::Kind::UndoTransaction(inverse)),
                ..
            },
        ] = txn.ops.as_slice()
        {
            let target = Id::from_proto(inverse.target_txn_id.as_ref())?;
            let redo = !self
                .directions
                .get(&target)
                .copied()
                .ok_or(CoreError::Invalid)?;
            self.undo.acknowledge(redo, txn, receipt)?;
            self.directions.insert(id, redo);
        } else {
            self.undo.record(txn, receipt)?;
            self.directions.insert(id, true);
        }
        Ok(())
    }
    pub(crate) fn pending(&mut self, txn: &pb::Transaction) -> Result<()> {
        if txn.device_id != self.device.as_str() {
            return Err(CoreError::Invalid);
        }
        for op in &txn.ops {
            self.next_lamport = self.next_lamport.max(
                op.op_id
                    .as_ref()
                    .ok_or(CoreError::Invalid)?
                    .lamport
                    .checked_add(1)
                    .ok_or(CoreError::Invalid)?,
            );
        }
        let id = Id::from_proto(txn.txn_id.as_ref())?;
        if let [
            pb::Op {
                kind: Some(pb::op::Kind::UndoTransaction(inverse)),
                ..
            },
        ] = txn.ops.as_slice()
        {
            let target = Id::from_proto(inverse.target_txn_id.as_ref())?;
            let redo = !self
                .directions
                .get(&target)
                .copied()
                .ok_or(CoreError::Invalid)?;
            self.undo.record_pending_inverse(redo, txn)?;
            self.directions.insert(id, redo);
        } else {
            self.undo.record_pending(txn)?;
            self.directions.insert(id, true);
        }
        Ok(())
    }
}
impl ProjectState {
    pub(crate) fn commit(
        &mut self,
        txn: &pb::Transaction,
        device: &DeviceId,
        time: i64,
    ) -> Result<ProjectInfo> {
        if &self.store()?.local_device()? != device {
            return Err(CoreError::Invalid);
        }
        if let Some(replica) = &self.replica {
            if let Some((old, _)) = self
                .store()?
                .accepted_transactions()
                .find(|(old, _)| old.txn_id == txn.txn_id)
            {
                if old != txn {
                    return Err(CoreError::Invalid);
                }
                return self.info();
            }
            if let Some(old) = replica
                .pending()
                .iter()
                .find(|p| p.transaction.txn_id == txn.txn_id)
            {
                if &old.transaction != txn {
                    return Err(CoreError::Invalid);
                }
                return self.info();
            }
            let mut staged = replica.clone();
            if txn.base_revision.as_ref() == Some(staged.revision()) {
                staged.queue(txn.clone())?;
            } else {
                staged.queue_recovered(txn.clone())?;
            }
            let mut history = self.history.clone();
            history.pending(txn)?;
            self.store
                .as_mut()
                .ok_or(CoreError::Closed)?
                .enqueue_pending(txn)?;
            self.replica = Some(staged);
            self.history = history;
            let info = self.info()?;
            self.events.publish(ChangeKind::Committed, info.clone());
            self.changed.notify_one();
            return Ok(info);
        }
        let receipt = self
            .store
            .as_mut()
            .ok_or(CoreError::Closed)?
            .commit(txn, device, time)?;
        self.history.accept(txn, &receipt)?;
        let info = self.info()?;
        self.events.publish(ChangeKind::Committed, info.clone());
        self.changed.notify_one();
        Ok(info)
    }
}
pub(crate) fn affine(value: Transform) -> Result<pb::Affine> {
    vw_geom::Affine::new(value.a, value.b, value.c, value.d, value.e, value.f)?.inverse()?;
    Ok(pb::Affine {
        a: value.a,
        b: value.b,
        c: value.c,
        d: value.d,
        e: value.e,
        f: value.f,
    })
}
pub(crate) fn shape(value: NewShape) -> Result<pb::object_state::Shape> {
    use pb::object_state::Shape;
    let points = |points: Vec<Point>| -> Result<pb::Polyline> {
        if points.len() < 2 || points.len() > 4096 {
            return Err(CoreError::Invalid);
        }
        Ok(pb::Polyline {
            points: points
                .into_iter()
                .map(|p| pb::PointD { x: p.x, y: p.y })
                .collect(),
            closed: false,
        })
    };
    let rect = |r: QueryRect| pb::RectD {
        x: r.x,
        y: r.y,
        w: r.width,
        h: r.height,
    };
    Ok(match value {
        NewShape::Line { points: p } => Shape::Line(points(p)?),
        NewShape::Arrow { points: p } => Shape::Arrow(points(p)?),
        NewShape::Rectangle { rectangle } => Shape::Rect(rect(rectangle)),
        NewShape::Ellipse { rectangle } => Shape::Ellipse(rect(rectangle)),
        NewShape::Text {
            anchor,
            text,
            font,
            size,
        } => {
            if text.len() > 65536 {
                return Err(CoreError::Invalid);
            }
            vw_raster::layout_text(&text, vw_raster::FontFamily::parse(&font)?, size as f32)?;
            Shape::Text(pb::TextObject {
                text,
                font_family: font,
                font_size: size,
                anchor: Some(pb::PointD {
                    x: anchor.x,
                    y: anchor.y,
                }),
            })
        }
    })
}
fn transaction(
    state: &ProjectState,
    options: &EditOptions,
    ops: Vec<pb::Op>,
    gesture_id: Option<pb::Uuid>,
) -> Result<pb::Transaction> {
    let store = state.store()?;
    let device = DeviceId::try_from(options.device_id.clone())?;
    if device != store.local_device()? || options.lamport == 0 || options.created_at_ms < 0 {
        return Err(CoreError::Invalid);
    }
    let id = Id::try_from(options.transaction_id.clone())?.to_proto();
    let previous = state.previous(&id)?;
    let value = pb::Transaction {
        txn_id: Some(id),
        project_id: Some(store.project().id.to_proto()),
        device_id: device.to_string(),
        base_revision: if let Some(old) = previous {
            old.base_revision.clone()
        } else {
            Some(store.revision()?)
        },
        created_at_wall_ms: options.created_at_ms,
        gesture_id,
        ops,
    };
    if previous.is_some_and(|old| old != &value) {
        return Err(CoreError::Invalid);
    }
    Ok(value)
}
#[uniffi::export]
impl ProjectSession {
    pub async fn apply_edit(
        &self,
        options: EditOptions,
        commands: Vec<EditCommand>,
        cancellation: Arc<Cancellation>,
    ) -> Result<ProjectInfo> {
        self.apply_edit_at(options, commands, None, None, cancellation)
            .await
    }
    /// Absolute edits can atomically require the visible revision captured when
    /// the gesture began. Exact transaction retries remain idempotent.
    pub async fn apply_edit_at(
        &self,
        options: EditOptions,
        commands: Vec<EditCommand>,
        expected: Option<EditPrecondition>,
        gesture_id: Option<String>,
        cancellation: Arc<Cancellation>,
    ) -> Result<ProjectInfo> {
        self.check_open()?;
        if commands.is_empty() || commands.len() > 512 {
            return Err(CoreError::Invalid);
        }
        let gesture_id = gesture_id
            .map(Id::try_from)
            .transpose()?
            .map(|id| id.to_proto());
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                cancellation.check()?;
                if closed.load(std::sync::atomic::Ordering::Acquire) {
                    return Err(CoreError::Closed);
                }
                let project = state.project()?;
                let document = Id::try_from(options.document_id.clone())?;
                let txn_id = Id::try_from(options.transaction_id.clone())?.to_proto();
                let previous = state.previous(&txn_id)?;
                if previous.is_none()
                    && let Some(expected) = expected
                {
                    let current = state.info()?;
                    if expected.host_seq != current.host_seq
                        || expected.state_hash != current.state_hash
                    {
                        return Err(CoreError::Invalid);
                    }
                }
                if !project.documents.contains_key(&document) {
                    return Err(CoreError::Invalid);
                }
                let mut order_by_layer = BTreeMap::<Id, OrderKey>::new();
                let mut ops = Vec::new();
                for command in commands {
                    let index = ops.len();
                    let kind = match command {
                        EditCommand::Create {
                            object_id,
                            layer_id,
                            shape: s,
                            style,
                            transform,
                        } => {
                            let layer = Id::try_from(layer_id)?;
                            let left = order_by_layer.get(&layer).cloned().or_else(|| {
                                project
                                    .objects
                                    .values()
                                    .filter(|o| {
                                        o.state.layer_id.as_ref() == Some(&layer.to_proto())
                                    })
                                    .map(|o| o.state.order_key.as_str())
                                    .max()
                                    .and_then(|v| OrderKey::try_from(v.to_owned()).ok())
                            });
                            let old_order =
                                previous
                                    .and_then(|txn| txn.ops.get(index))
                                    .and_then(|op| match op.kind.as_ref() {
                                        Some(pb::op::Kind::CreateObject(c)) => {
                                            c.state.as_ref().map(|s| s.order_key.clone())
                                        }
                                        _ => None,
                                    });
                            let order = if let Some(value) = old_order {
                                OrderKey::try_from(value)?
                            } else {
                                OrderKey::between(left.as_ref(), None)?
                            };
                            order_by_layer.insert(layer.clone(), order.clone());
                            pb::op::Kind::CreateObject(pb::CreateObject {
                                document_id: Some(document.to_proto()),
                                state: Some(pb::ObjectState {
                                    object_id: Some(Id::try_from(object_id)?.to_proto()),
                                    layer_id: Some(layer.to_proto()),
                                    order_key: order.as_str().into(),
                                    transform: Some(affine(transform)?),
                                    style: Some(pb::Style {
                                        stroke: Some(pb::Color { rgba: style.rgba }),
                                        width: style.width,
                                        screen_constant_width: style.screen_constant_width,
                                        fill: style.fill.map(|rgba| pb::Color { rgba }),
                                        has_fill: style.fill.is_some(),
                                    }),
                                    role: pb::Role::None as i32,
                                    created_by: options.device_id.clone(),
                                    created_at_ms: options.created_at_ms,
                                    shape: Some(shape(s)?),
                                    ..Default::default()
                                }),
                            })
                        }
                        EditCommand::Delete { object_id } => {
                            let id = Id::try_from(object_id)?;
                            if previous.is_none()
                                && project
                                    .objects
                                    .get(&id)
                                    .is_none_or(|o| o.document_id != document)
                            {
                                return Err(CoreError::Invalid);
                            }
                            pb::op::Kind::DeleteObject(pb::DeleteObject {
                                object_id: Some(id.to_proto()),
                            })
                        }
                        EditCommand::Transform {
                            object_id,
                            transform,
                        } => {
                            let id = Id::try_from(object_id)?;
                            if previous.is_none()
                                && project
                                    .objects
                                    .get(&id)
                                    .is_none_or(|o| o.document_id != document)
                            {
                                return Err(CoreError::Invalid);
                            }
                            pb::op::Kind::SetProperty(pb::SetProperty {
                                object_id: Some(id.to_proto()),
                                property: "transform".into(),
                                value: Some(pb::PropertyValue {
                                    value: Some(pb::property_value::Value::Transform(affine(
                                        transform,
                                    )?)),
                                }),
                            })
                        }
                        EditCommand::SetStyle { object_id, style } => {
                            let id = Id::try_from(object_id)?;
                            if previous.is_none()
                                && project
                                    .objects
                                    .get(&id)
                                    .is_none_or(|o| o.document_id != document)
                            {
                                return Err(CoreError::Invalid);
                            }
                            let prior_shape = previous.and_then(|txn| txn.ops.get(index)).and_then(
                                |op| match &op.kind {
                                    Some(pb::op::Kind::SetProperty(p)) if p.property == "shape" => {
                                        p.value.as_ref().and_then(|v| match &v.value {
                                            Some(pb::property_value::Value::Raw(bytes)) => {
                                                Some(bytes)
                                            }
                                            _ => None,
                                        })
                                    }
                                    _ => None,
                                },
                            );
                            let mut shape = if let Some(bytes) = prior_shape {
                                Some(
                                    serde_json::from_slice::<pb::object_state::Shape>(bytes)
                                        .map_err(|_| CoreError::Invalid)?,
                                )
                            } else if previous.is_some() {
                                None
                            } else {
                                project.objects.get(&id).and_then(|o| o.state.shape.clone())
                            };
                            let change_width =
                                if let Some(pb::object_state::Shape::Stroke(stroke)) = &mut shape {
                                    let brush = stroke.brush.as_mut().ok_or(CoreError::Invalid)?;
                                    let changed =
                                        brush.base_width != style.width || prior_shape.is_some();
                                    brush.base_width = style.width;
                                    changed
                                } else {
                                    false
                                };
                            if change_width {
                                let raw = serde_json::to_vec(&shape.ok_or(CoreError::Invalid)?)
                                    .map_err(|_| CoreError::Invalid)?;
                                ops.push(pb::Op {
                                    op_id: Some(pb::OpId {
                                        device_id: options.device_id.clone(),
                                        lamport: options
                                            .lamport
                                            .checked_add(ops.len() as u64)
                                            .ok_or(CoreError::Invalid)?,
                                    }),
                                    kind: Some(pb::op::Kind::SetProperty(pb::SetProperty {
                                        object_id: Some(id.to_proto()),
                                        property: "shape".into(),
                                        value: Some(pb::PropertyValue {
                                            value: Some(pb::property_value::Value::Raw(raw)),
                                        }),
                                    })),
                                });
                            }
                            pb::op::Kind::SetProperty(pb::SetProperty {
                                object_id: Some(id.to_proto()),
                                property: "style".into(),
                                value: Some(pb::PropertyValue {
                                    value: Some(pb::property_value::Value::Style(pb::Style {
                                        stroke: Some(pb::Color { rgba: style.rgba }),
                                        width: style.width,
                                        screen_constant_width: style.screen_constant_width,
                                        fill: style.fill.map(|rgba| pb::Color { rgba }),
                                        has_fill: style.fill.is_some(),
                                    })),
                                }),
                            })
                        }
                        EditCommand::SetText {
                            object_id,
                            text,
                            font,
                            size,
                        } => {
                            let id = Id::try_from(object_id)?;
                            if previous.is_none()
                                && project.objects.get(&id).is_none_or(|o| {
                                    o.document_id != document
                                        || !matches!(
                                            o.state.shape,
                                            Some(pb::object_state::Shape::Text(_))
                                        )
                                })
                            {
                                return Err(CoreError::Invalid);
                            }
                            if text.len() > 65536 {
                                return Err(CoreError::Invalid);
                            }
                            vw_raster::layout_text(
                                &text,
                                vw_raster::FontFamily::parse(&font)?,
                                size as f32,
                            )?;
                            for (property, value) in [
                                ("text.text", pb::property_value::Value::Text(text)),
                                ("text.font_family", pb::property_value::Value::Text(font)),
                            ] {
                                ops.push(pb::Op {
                                    op_id: Some(pb::OpId {
                                        device_id: options.device_id.clone(),
                                        lamport: options
                                            .lamport
                                            .checked_add(ops.len() as u64)
                                            .ok_or(CoreError::Invalid)?,
                                    }),
                                    kind: Some(pb::op::Kind::SetProperty(pb::SetProperty {
                                        object_id: Some(id.to_proto()),
                                        property: property.into(),
                                        value: Some(pb::PropertyValue { value: Some(value) }),
                                    })),
                                });
                            }
                            pb::op::Kind::SetProperty(pb::SetProperty {
                                object_id: Some(id.to_proto()),
                                property: "text.font_size".into(),
                                value: Some(pb::PropertyValue {
                                    value: Some(pb::property_value::Value::Number(size)),
                                }),
                            })
                        }
                    };
                    ops.push(pb::Op {
                        op_id: Some(pb::OpId {
                            device_id: options.device_id.clone(),
                            lamport: options
                                .lamport
                                .checked_add(ops.len() as u64)
                                .ok_or(CoreError::Invalid)?,
                        }),
                        kind: Some(kind),
                    });
                }
                let txn = transaction(state, &options, ops, gesture_id)?;
                cancellation.check()?;
                state.commit(
                    &txn,
                    &DeviceId::try_from(options.device_id)?,
                    options.created_at_ms,
                )
            })
            .await
    }
    /// Undo and redo target the device's accepted cursor. Reopening reconstructs
    /// the same cursor from the durable accepted transaction journal.
    pub async fn undo_redo(
        &self,
        options: EditOptions,
        redo: bool,
        cancellation: Arc<Cancellation>,
    ) -> Result<ProjectInfo> {
        self.check_open()?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                cancellation.check()?;
                if closed.load(std::sync::atomic::Ordering::Acquire) {
                    return Err(CoreError::Closed);
                }
                let id = Id::try_from(options.transaction_id.clone())?;
                let previous = state.previous(&id.to_proto())?;
                let op = if let Some(previous) = previous {
                    if state.history.directions.get(&id) != Some(&redo)
                        || previous.ops.len() != 1
                        || !matches!(previous.ops[0].kind, Some(pb::op::Kind::UndoTransaction(_)))
                    {
                        return Err(CoreError::Invalid);
                    }
                    let mut op = previous.ops[0].clone();
                    op.op_id = Some(pb::OpId {
                        device_id: options.device_id.clone(),
                        lamport: options.lamport,
                    });
                    op
                } else {
                    state.history.undo.operation(redo, options.lamport)?
                };
                let txn = transaction(state, &options, vec![op], None)?;
                cancellation.check()?;
                state.commit(
                    &txn,
                    &DeviceId::try_from(options.device_id)?,
                    options.created_at_ms,
                )
            })
            .await
    }
}
