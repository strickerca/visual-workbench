use super::*;
use crate::{ProjectSession, project::ProjectState};
use std::sync::Arc;
use vw_model::Id;
pub(super) fn collect(
    state: &ProjectState,
    binding: &crate::WorkflowBinding,
) -> AiEditResult<(AiContext, Vec<vw_ai::Instruction>)> {
    crate::workflow::check_binding(state, binding)?;
    let project = state.project()?;
    let doc = Id::try_from(binding.document_id.clone())?;
    // AI prompt limits are narrower than an instruction package. Refuse before
    // allocating the complete export; no truncation or dropped role text.
    let mut count = 0usize;
    let mut bytes = 0usize;
    for value in project.instructions.values() {
        if value.definition.document_id == Some(doc.to_proto()) {
            count += 1;
            bytes = bytes
                .checked_add(value.definition.text.len())
                .ok_or(AiEditError::Limit)?;
            if count > 120 || bytes > 112 * 1024 {
                return Err(AiEditError::Limit);
            }
        }
    }
    let view = vw_instructions::DocumentView::new(project, &state.view_revision()?, &doc)
        .map_err(crate::WorkflowError::from)?;
    let export =
        vw_instructions::InstructionExport::from_view(&view).map_err(crate::WorkflowError::from)?;
    let mut summaries = Vec::new();
    let mut instructions = Vec::new();
    for item in export
        .global()
        .into_iter()
        .chain(export.instructions().iter())
    {
        let role = item.role.as_str();
        let model_role = match role {
            "change" => vw_ai::InstructionRole::Change,
            "preserve" => vw_ai::InstructionRole::Preserve,
            "reference" | "none" => vw_ai::InstructionRole::Reference,
            "explain" => vw_ai::InstructionRole::Explain,
            _ => return Err(AiEditError::Invalid),
        };
        let numbers = export
            .markers()
            .iter()
            .filter(|m| m.instruction_id == item.id)
            .map(|m| m.number)
            .collect::<Vec<_>>();
        let mut text = if role == "none" {
            format!("Context only, no requested edit: {}", item.text)
        } else {
            item.text.clone()
        };
        // Marker geometry remains numeric bound project context. Captured EIDs
        // and semantic text are not promoted into the authored instruction channel.
        for marker in export
            .markers()
            .iter()
            .filter(|m| m.instruction_id == item.id)
        {
            text.push_str(&format!(
                "\nMarker {} at D {:?}, bounds {:?}.",
                marker.number, marker.point_document, marker.bounds_document
            ));
        }
        if text.len() > 32768 {
            return Err(AiEditError::Limit);
        }
        instructions.push(vw_ai::Instruction {
            role: model_role,
            text,
        });
        summaries.push(AiInstructionSummary {
            id: item.id.to_string(),
            role: role.into(),
            text: item.text.clone(),
            target_object_ids: item.target_ids.iter().map(ToString::to_string).collect(),
            marker_numbers: numbers,
        });
    }
    let source = project
        .assets
        .get(&vw_model::AssetId::try_from(
            project
                .documents
                .get(&doc)
                .ok_or(AiEditError::Invalid)?
                .definition
                .primary_asset_id
                .clone(),
        )?)
        .ok_or(AiEditError::Proof)?;
    let (width, height) = if source.orientation >= 5 {
        (source.height, source.width)
    } else {
        (source.width, source.height)
    };
    Ok((
        AiContext {
            binding: binding.clone(),
            source_asset_id: source.asset_id.clone(),
            width,
            height,
            bit_depth: source.bit_depth,
            change_selections: crate::masks::ai_bridge::versions(state, &binding.document_id)?,
            instructions: summaries,
        },
        instructions,
    ))
}
#[uniffi::export]
impl ProjectSession {
    pub async fn ai_context(
        &self,
        binding: crate::WorkflowBinding,
        memory_budget_bytes: u64,
        cancellation: Arc<crate::Cancellation>,
    ) -> AiEditResult<AiContext> {
        self.check_open()?;
        binding.validate()?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check(&closed, &cancellation)?;
                    crate::workflow::admit(
                        state,
                        memory_budget_bytes.min(crate::workflow::MAX_MEMORY),
                        false,
                        32 * 1024 * 1024,
                    )?;
                    let (context, _) = collect(state, &binding)?;
                    check(&closed, &cancellation)?;
                    Ok(context)
                })())
            })
            .await?
    }
}
