use crate::{
    DocumentView, EntryMethod, Error, MAX_MARKERS, Result, Role, SourceBinding,
    view::{authenticate, validate_definition},
};
use vw_model::{DeviceId, Id, OrderKey, Project};
use vw_ops::{Acceptance, HostSequencer};
use vw_proto::v1::{self, object_state::Shape, op::Kind, property_value::Value};

#[derive(Clone, Debug)]
pub struct EditMetadata {
    pub transaction_id: Id,
    pub device: DeviceId,
    pub first_lamport: u64,
    pub created_at_ms: i64,
}
/// Ordinary canonical ops with a strict workflow precondition. Persist through
/// vw-store while holding the same project mutation lock as check_current.
#[derive(Clone)]
pub struct EditPlan {
    binding: SourceBinding,
    transaction: v1::Transaction,
    next_lamport: u64,
}
impl EditPlan {
    pub fn binding(&self) -> &SourceBinding {
        &self.binding
    }
    pub fn transaction(&self) -> &v1::Transaction {
        &self.transaction
    }
    pub fn next_lamport(&self) -> u64 {
        self.next_lamport
    }
    pub fn check_current(&self, project: &Project, revision: &v1::Revision) -> Result<()> {
        if self.binding
            != *DocumentView::new(project, revision, &self.binding.document_id)?.binding()
        {
            return Err(Error::Stale);
        }
        Ok(())
    }
    /// In-memory integration/reference path, not durable storage. Exact accepted
    /// retries bypass the stale guard; altered bytes remain rejected by vw-ops.
    pub fn submit(
        &self,
        host: &mut HostSequencer,
        authenticated: &DeviceId,
        now_ms: i64,
    ) -> Result<Acceptance> {
        authenticate(&self.transaction.device_id, authenticated)?;
        let id = Id::from_proto(self.transaction.txn_id.as_ref())?;
        if host.accepted_transaction(&id).is_none() {
            self.check_current(host.project(), &host.revision()?)?;
        }
        Ok(host.submit(self.transaction.clone(), authenticated, now_ms)?)
    }
}

pub struct MarkerPlacement {
    pub object_id: Id,
    pub instruction_id: Id,
    pub layer_id: Id,
    pub point: v1::PointD,
    pub bounds: Option<v1::RectD>,
    pub element_eids: Vec<String>,
    pub style: v1::Style,
    pub role: Role,
    pub text: String,
    pub entry_method: EntryMethod,
    pub language: String,
}
#[derive(Clone)]
pub struct InstructionEdit {
    pub instruction_id: Id,
    /// Empty targets mean the document's one optional global instruction.
    pub targets: Vec<Id>,
    pub role: Role,
    pub text: String,
    pub entry_method: EntryMethod,
    pub language: String,
}
impl DocumentView<'_> {
    pub fn place_marker(&self, meta: EditMetadata, placement: MarkerPlacement) -> Result<EditPlan> {
        self.numbering()?;
        if self.markers.len() == MAX_MARKERS {
            return Err(Error::Limit("markers"));
        }
        if self
            .project
            .objects
            .values()
            .filter(|v| v.document_id == self.binding.document_id)
            .count()
            == crate::MAX_DOCUMENT_OBJECTS
        {
            return Err(Error::Limit("document objects"));
        }
        if placement.object_id == placement.instruction_id {
            return Err(Error::Invalid("marker identity"));
        }
        self.fresh_id(&placement.object_id)?;
        self.fresh_id(&placement.instruction_id)?;
        self.editable_layer(&placement.layer_id)?;
        let last = self
            .project
            .objects
            .values()
            .filter(|v| v.state.layer_id == Some(placement.layer_id.to_proto()))
            .map(|v| v.state.order_key.as_str())
            .max();
        let last = last.map(|v| OrderKey::try_from(v.to_owned())).transpose()?;
        let order = OrderKey::between(last.as_ref(), None)?;
        let kind = self
            .project
            .documents
            .get(&self.binding.document_id)
            .ok_or(Error::Missing)?
            .definition
            .kind;
        let pixels = matches!(
            v1::DocumentKind::try_from(kind),
            Ok(v1::DocumentKind::Image | v1::DocumentKind::Capture)
        );
        if kind == v1::DocumentKind::Pdf as i32
            && self
                .project
                .layers
                .get(&placement.layer_id)
                .ok_or(Error::Missing)?
                .definition
                .page_index
                < 0
        {
            return Err(Error::Invalid("PDF marker page"));
        }
        let point = if pixels {
            v1::PointD {
                x: vw_geom::snap_pixel_center(placement.point.x)?,
                y: vw_geom::snap_pixel_center(placement.point.y)?,
            }
        } else {
            placement.point
        };
        let bounds = placement
            .bounds
            .map(|rect| -> Result<v1::RectD> {
                let checked = vw_geom::Rect::new(rect.x, rect.y, rect.w, rect.h)?;
                if pixels {
                    let x = vw_geom::snap_edge(checked.left())?;
                    let y = vw_geom::snap_edge(checked.top())?;
                    let right = vw_geom::snap_edge(checked.right())?;
                    let bottom = vw_geom::snap_edge(checked.bottom())?;
                    Ok(v1::RectD {
                        x,
                        y,
                        w: right - x,
                        h: bottom - y,
                    })
                } else {
                    Ok(rect)
                }
            })
            .transpose()?;
        if placement.element_eids.len() > crate::MAX_ELEMENT_REFS
            || placement
                .element_eids
                .iter()
                .any(|v| v.len() > crate::MAX_ELEMENT_ID_BYTES || v.contains('\0'))
        {
            return Err(Error::Limit("element references"));
        }
        let existing_eids = self
            .markers
            .iter()
            .flat_map(|v| &v.2.element_eids)
            .map(String::len)
            .sum::<usize>();
        let added_eids = placement
            .element_eids
            .iter()
            .map(String::len)
            .sum::<usize>();
        if existing_eids
            .checked_add(added_eids)
            .is_none_or(|bytes| bytes > crate::MAX_TOTAL_TEXT_BYTES)
        {
            return Err(Error::Limit("element references"));
        }
        let state = v1::ObjectState {
            object_id: Some(placement.object_id.to_proto()),
            layer_id: Some(placement.layer_id.to_proto()),
            order_key: order.as_str().into(),
            transform: Some(v1::Affine {
                a: 1.0,
                d: 1.0,
                ..Default::default()
            }),
            style: Some(placement.style),
            role: placement.role.canonical(),
            created_by: meta.device.to_string(),
            created_at_ms: meta.created_at_ms,
            shape: Some(Shape::Marker(v1::Marker {
                number: self.markers.len() as u32 + 1,
                point: Some(point),
                r#box: bounds,
                element_eids: placement.element_eids,
            })),
            ..Default::default()
        };
        vw_model::validate_object_state(&state)?;
        let instruction = self.definition(InstructionEdit {
            instruction_id: placement.instruction_id,
            targets: vec![placement.object_id],
            role: placement.role,
            text: placement.text,
            entry_method: placement.entry_method,
            language: placement.language,
        })?;
        self.plan(
            meta,
            vec![
                Kind::CreateObject(v1::CreateObject {
                    document_id: Some(self.binding.document_id.to_proto()),
                    state: Some(state),
                }),
                Kind::SetInstruction(instruction),
            ],
        )
    }
    pub fn delete_marker(&self, meta: EditMetadata, marker: &Id) -> Result<EditPlan> {
        self.numbering()?;
        self.editable(marker)?;
        let instruction_id = self.marker_instruction(marker)?;
        let instruction = self.instruction(instruction_id)?;
        if instruction.definition.target_object_ids.len() != 1 {
            return Err(Error::Reconciliation);
        }
        let deleted = self
            .markers
            .iter()
            .find(|v| v.0 == marker)
            .ok_or(Error::Missing)?
            .2
            .number;
        let mut kinds = vec![
            Kind::DeleteInstruction(v1::DeleteInstruction {
                instruction_id: Some(instruction_id.to_proto()),
            }),
            Kind::DeleteObject(v1::DeleteObject {
                object_id: Some(marker.to_proto()),
            }),
        ];
        for (id, _, value) in &self.markers {
            if value.number > deleted {
                self.editable(id)?;
                kinds.push(property(
                    id,
                    "marker.number",
                    Value::Integer(i64::from(value.number - 1)),
                ));
            }
        }
        self.plan(meta, kinds)
    }
    pub fn set_instruction(&self, meta: EditMetadata, edit: InstructionEdit) -> Result<EditPlan> {
        let value = self.definition(edit)?;
        let id = Id::from_proto(value.instruction_id.as_ref())?;
        if let Some(old) = self.project.instructions.get(&id) {
            if Id::from_proto(old.definition.document_id.as_ref())? != self.binding.document_id {
                return Err(Error::Invalid("instruction document"));
            }
            // Retargeting may not strand an existing marker without its record.
            for target in &old.definition.target_object_ids {
                let target_id = Id::from_proto(Some(target))?;
                // Retargeting removes a binding from the old object, so its
                // object/layer lock applies just as it does to deletion.
                if self.project.objects.contains_key(&target_id) {
                    self.editable(&target_id)?;
                }
                if self.markers.iter().any(|v| v.0 == &target_id)
                    && !value.target_object_ids.contains(target)
                {
                    return Err(Error::Reconciliation);
                }
            }
        } else {
            self.fresh_id(&id)?;
        }
        let mut kinds = Vec::new();
        for target in &value.target_object_ids {
            let target_id = Id::from_proto(Some(target))?;
            self.editable(&target_id)?;
            for (other_id, other) in &self.instructions {
                if *other_id != &id && other.definition.target_object_ids.contains(target) {
                    return Err(Error::Reconciliation);
                }
            }
            if self.object(&target_id)?.state.role != value.role {
                kinds.push(property(&target_id, "role", Value::Role(value.role)));
            }
        }
        if value.target_object_ids.is_empty()
            && self.instructions.iter().any(|(other_id, other)| {
                *other_id != &id && other.definition.target_object_ids.is_empty()
            })
        {
            return Err(Error::Reconciliation);
        }
        kinds.push(Kind::SetInstruction(value));
        self.plan(meta, kinds)
    }
    pub fn delete_instruction(&self, meta: EditMetadata, instruction: &Id) -> Result<EditPlan> {
        let value = &self.instruction(instruction)?.definition;
        for target in &value.target_object_ids {
            let id = Id::from_proto(Some(target))?;
            if self.markers.iter().any(|v| v.0 == &id) {
                return Err(Error::Reconciliation);
            }
            if self.project.objects.contains_key(&id) {
                self.editable(&id)?;
            }
        }
        self.plan(
            meta,
            vec![Kind::DeleteInstruction(v1::DeleteInstruction {
                instruction_id: Some(instruction.to_proto()),
            })],
        )
    }
    fn definition(&self, edit: InstructionEdit) -> Result<v1::SetInstruction> {
        if edit.targets.len() > crate::MAX_TARGETS {
            return Err(Error::Limit("instruction targets"));
        }
        let value = v1::SetInstruction {
            instruction_id: Some(edit.instruction_id.to_proto()),
            document_id: Some(self.binding.document_id.to_proto()),
            target_object_ids: edit.targets.iter().map(Id::to_proto).collect(),
            role: edit.role.canonical(),
            text: edit.text,
            entry_method: edit.entry_method.as_str().into(),
            language: edit.language,
        };
        validate_definition(&value)?;
        let id = Id::from_proto(value.instruction_id.as_ref())?;
        let replacing = self.instructions.iter().any(|v| v.0 == &id);
        if !replacing && self.instructions.len() == crate::MAX_INSTRUCTIONS {
            return Err(Error::Limit("instructions"));
        }
        let mut bytes = value.text.len();
        for (other_id, other) in &self.instructions {
            if *other_id != &id {
                bytes = bytes
                    .checked_add(other.definition.text.len())
                    .ok_or(Error::Limit("total text"))?;
            }
        }
        if bytes > crate::MAX_TOTAL_TEXT_BYTES {
            return Err(Error::Limit("total text"));
        }
        Ok(value)
    }
    fn plan(&self, meta: EditMetadata, kinds: Vec<Kind>) -> Result<EditPlan> {
        if meta.first_lamport == 0
            || meta.created_at_ms < 0
            || meta.created_at_ms as u64 >= 1u64 << 48
        {
            return Err(Error::Invalid("edit metadata"));
        }
        let next_lamport = meta
            .first_lamport
            .checked_add(kinds.len() as u64)
            .ok_or(Error::Exhausted)?;
        let transaction = v1::Transaction {
            txn_id: Some(meta.transaction_id.to_proto()),
            device_id: meta.device.to_string(),
            project_id: Some(self.binding.project_id.to_proto()),
            base_revision: Some(self.binding.revision()),
            gesture_id: None,
            created_at_wall_ms: meta.created_at_ms,
            ops: kinds
                .into_iter()
                .enumerate()
                .map(|(i, kind)| v1::Op {
                    op_id: Some(v1::OpId {
                        device_id: meta.device.to_string(),
                        lamport: meta.first_lamport + i as u64,
                    }),
                    kind: Some(kind),
                })
                .collect(),
        };
        Ok(EditPlan {
            binding: self.binding.clone(),
            transaction,
            next_lamport,
        })
    }
}
fn property(id: &Id, name: &str, value: Value) -> Kind {
    Kind::SetProperty(v1::SetProperty {
        object_id: Some(id.to_proto()),
        property: name.into(),
        value: Some(v1::PropertyValue { value: Some(value) }),
    })
}
