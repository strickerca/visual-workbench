//! Typed marker/instruction adapters. OS recognition and authenticated remote
//! focus routing remain separate platform/session capabilities.
mod draft;
mod dto;
use crate::{Cancellation, ProjectSession, workflow::*};
pub use draft::*;
pub use dto::*;
use std::sync::Arc;
use vw_instructions::{DocumentView, InstructionExport};
use vw_model::Id;
use vw_proto::v1 as pb;

fn view<'a>(
    state: &'a crate::project::ProjectState,
    expected: &WorkflowBinding,
) -> WorkflowResult<DocumentView<'a>> {
    check_binding(state, expected)?;
    Ok(DocumentView::new(
        state.project()?,
        &state.view_revision()?,
        &Id::try_from(expected.document_id.clone())?,
    )?)
}
fn bounded_id(value: &str) -> WorkflowResult<()> {
    if value.len() != 36 {
        return Err(WorkflowError::Invalid);
    }
    Id::try_from(value.to_owned())?;
    Ok(())
}
fn bounded_text(text: &str, language: &str) -> WorkflowResult<()> {
    if text.len() > vw_instructions::MAX_TEXT_BYTES
        || text.contains('\0')
        || language.len() > 128
        || language.chars().any(char::is_control)
    {
        return Err(WorkflowError::Limit);
    }
    Ok(())
}
fn validate_command(command: &InstructionCommand) -> WorkflowResult<()> {
    match command {
        InstructionCommand::PlaceMarker {
            object_id,
            instruction_id,
            layer_id,
            element_eids,
            text,
            language,
            ..
        } => {
            bounded_id(object_id)?;
            bounded_id(instruction_id)?;
            bounded_id(layer_id)?;
            bounded_text(text, language)?;
            if element_eids.len() > vw_instructions::MAX_ELEMENT_REFS
                || element_eids
                    .iter()
                    .any(|e| e.len() > vw_instructions::MAX_ELEMENT_ID_BYTES || e.contains('\0'))
            {
                return Err(WorkflowError::Limit);
            }
        }
        InstructionCommand::SetInstruction {
            instruction_id,
            target_ids,
            text,
            language,
            ..
        } => {
            bounded_id(instruction_id)?;
            bounded_text(text, language)?;
            if target_ids.len() > vw_instructions::MAX_TARGETS {
                return Err(WorkflowError::Limit);
            }
            for id in target_ids {
                bounded_id(id)?;
            }
        }
        InstructionCommand::DeleteMarker { object_id } => bounded_id(object_id)?,
        InstructionCommand::DeleteInstruction { instruction_id } => bounded_id(instruction_id)?,
    }
    Ok(())
}
fn plan(
    view: &DocumentView<'_>,
    metadata: WorkflowMetadata,
    command: InstructionCommand,
) -> WorkflowResult<vw_instructions::EditPlan> {
    let meta = metadata.instruction()?;
    Ok(match command {
        InstructionCommand::PlaceMarker {
            object_id,
            instruction_id,
            layer_id,
            point,
            bounds,
            element_eids,
            style,
            role,
            text,
            entry_method,
            language,
        } => view.place_marker(
            meta,
            vw_instructions::MarkerPlacement {
                object_id: Id::try_from(object_id)?,
                instruction_id: Id::try_from(instruction_id)?,
                layer_id: Id::try_from(layer_id)?,
                point: pb::PointD {
                    x: point.x,
                    y: point.y,
                },
                bounds: bounds.map(|v| pb::RectD {
                    x: v.x,
                    y: v.y,
                    w: v.width,
                    h: v.height,
                }),
                element_eids,
                style: pb::Style {
                    stroke: Some(pb::Color { rgba: style.rgba }),
                    width: style.width,
                    screen_constant_width: style.screen_constant_width,
                    fill: style.fill.map(|rgba| pb::Color { rgba }),
                    has_fill: style.fill.is_some(),
                },
                role: role.core(),
                text,
                entry_method: entry_method.core(),
                language,
            },
        )?,
        InstructionCommand::SetInstruction {
            instruction_id,
            target_ids,
            role,
            text,
            entry_method,
            language,
        } => view.set_instruction(
            meta,
            vw_instructions::InstructionEdit {
                instruction_id: Id::try_from(instruction_id)?,
                targets: target_ids
                    .into_iter()
                    .map(Id::try_from)
                    .collect::<std::result::Result<_, _>>()?,
                role: role.core(),
                text,
                entry_method: entry_method.core(),
                language,
            },
        )?,
        InstructionCommand::DeleteMarker { object_id } => {
            view.delete_marker(meta, &Id::try_from(object_id)?)?
        }
        InstructionCommand::DeleteInstruction { instruction_id } => {
            view.delete_instruction(meta, &Id::try_from(instruction_id)?)?
        }
    })
}
#[uniffi::export]
impl ProjectSession {
    pub async fn instruction_document(
        &self,
        document_id: String,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<InstructionDocument> {
        self.check_open()?;
        bounded_id(&document_id)?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&closed, &cancellation)?;
                    admit(state, memory_budget_bytes, false, INSTRUCTION_WORK)?;
                    let binding = current(state, &document_id)?;
                    let view = view(state, &binding)?;
                    let detached = view.detached_instruction_ids()?;
                    let mut instructions = Vec::new();
                    for id in view.instruction_ids() {
                        cancellation.check()?;
                        let item = view.instruction(id)?;
                        let value = &item.definition;
                        instructions.push(InstructionRow {
                            instruction_id: id.to_string(),
                            target_ids: value
                                .target_object_ids
                                .iter()
                                .map(|id| Id::from_proto(Some(id)).map(|v| v.to_string()))
                                .collect::<std::result::Result<_, _>>()?,
                            role: InstructionRole::from_core(
                                vw_instructions::Role::from_canonical(value.role)?,
                            ),
                            text: value.text.clone(),
                            entry_method: InstructionEntryMethod::from_core(
                                vw_instructions::EntryMethod::from_canonical(&value.entry_method)?,
                            ),
                            language: value.language.clone(),
                            updated_at_ms: item.updated_at_ms,
                            detached: detached.contains(id),
                        });
                    }
                    let mut markers = Vec::new();
                    for id in view.marker_ids() {
                        cancellation.check()?;
                        let object = view.object(id)?;
                        let Some(pb::object_state::Shape::Marker(marker)) = &object.state.shape
                        else {
                            return Err(WorkflowError::Invalid);
                        };
                        let raw = object
                            .state
                            .transform
                            .as_ref()
                            .ok_or(WorkflowError::Invalid)?;
                        let transform =
                            vw_geom::Affine::new(raw.a, raw.b, raw.c, raw.d, raw.e, raw.f)?;
                        let point = marker.point.as_ref().ok_or(WorkflowError::Invalid)?;
                        let point = transform.map(vw_geom::Point::new(point.x, point.y)?)?;
                        let bounds = marker
                            .r#box
                            .as_ref()
                            .map(|b| -> WorkflowResult<crate::QueryRect> {
                                let b =
                                    transform.map_rect(vw_geom::Rect::new(b.x, b.y, b.w, b.h)?)?;
                                Ok(crate::QueryRect {
                                    x: b.left(),
                                    y: b.top(),
                                    width: b.width(),
                                    height: b.height(),
                                })
                            })
                            .transpose()?;
                        let layer = state
                            .project()?
                            .layers
                            .get(&Id::from_proto(object.state.layer_id.as_ref())?)
                            .ok_or(WorkflowError::Missing)?;
                        markers.push(MarkerRow {
                            object_id: id.to_string(),
                            instruction_id: view
                                .marker_instruction(id)
                                .ok()
                                .map(ToString::to_string),
                            number: marker.number,
                            point_document: crate::Point {
                                x: point.x(),
                                y: point.y(),
                            },
                            bounds_document: bounds,
                            element_eids: marker.element_eids.clone(),
                            hidden: object.state.hidden,
                            layer_visible: layer.visible,
                        });
                    }
                    let needs_reconciliation = match InstructionExport::from_view(&view) {
                        Ok(_) => false,
                        Err(
                            vw_instructions::Error::Reconciliation
                            | vw_instructions::Error::Detached,
                        ) => true,
                        Err(error) => return Err(error.into()),
                    };
                    cancellation.check()?;
                    Ok(InstructionDocument {
                        binding,
                        instructions,
                        markers,
                        needs_reconciliation,
                    })
                })())
            })
            .await?
    }
    pub async fn prepare_instruction(
        self: Arc<Self>,
        binding: WorkflowBinding,
        metadata: WorkflowMetadata,
        command: InstructionCommand,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<Arc<WorkflowPlan>> {
        self.check_open()?;
        binding.validate()?;
        metadata.validate()?;
        validate_command(&command)?;
        let permit = HandlePermit::acquire()?;
        let project = self.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&project.closed, &cancellation)?;
                    admit(state, memory_budget_bytes, true, INSTRUCTION_WORK)?;
                    check_metadata(state, &metadata)?;
                    let view = view(state, &binding)?;
                    let planned = plan(&view, metadata, command)?;
                    admit_transaction(
                        state,
                        planned.transaction(),
                        memory_budget_bytes,
                        INSTRUCTION_WORK,
                    )?;
                    cancellation.check()?;
                    WorkflowPlan::new(
                        &project,
                        state,
                        binding,
                        planned.transaction().clone(),
                        planned.next_lamport(),
                        memory_budget_bytes,
                        INSTRUCTION_WORK,
                        permit,
                    )
                })())
            })
            .await?
    }
    pub async fn export_instructions(
        &self,
        binding: WorkflowBinding,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<InstructionProjection> {
        self.check_open()?;
        binding.validate()?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&closed, &cancellation)?;
                    admit(state, memory_budget_bytes, false, INSTRUCTION_WORK)?;
                    let view = view(state, &binding)?;
                    let export = InstructionExport::from_view(&view)?;
                    let json = export.to_json()?;
                    let prompt_fragment = export.prompt_fragment()?;
                    cancellation.check()?;
                    Ok(InstructionProjection {
                        binding,
                        json,
                        prompt_fragment,
                    })
                })())
            })
            .await?
    }
}
