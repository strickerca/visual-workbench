//! Bounded nondestructive tonal controls over canonical adjustment operations.
//! Read-only previews use the existing renderer and an isolated operation host.
mod dto;
mod editing_view;
mod plan;
mod preview;
mod snapshot;

use crate::{Cancellation, ProjectSession, workflow::*};
pub use dto::*;
pub(crate) use editing_view::is_editing_metadata;
use std::sync::Arc;
use vw_model::{AssetId, Id, Project};
use vw_proto::v1 as pb;

const TONE_WORK: u64 = 16 * 1024 * 1024;
const MAX_LAYERS: usize = 256;
const MAX_OBJECTS: usize = 4096;

fn id(value: &str) -> WorkflowResult<Id> {
    if value.len() != 36 {
        return Err(WorkflowError::Invalid);
    }
    Ok(Id::try_from(value.to_owned())?)
}
fn bounds<'a>(project: &'a Project, document: &Id) -> WorkflowResult<&'a pb::AddAsset> {
    let definition = &project
        .documents
        .get(document)
        .ok_or(WorkflowError::Missing)?
        .definition;
    if !matches!(
        pb::DocumentKind::try_from(definition.kind),
        Ok(pb::DocumentKind::Image | pb::DocumentKind::Capture)
    ) {
        return Err(WorkflowError::Invalid);
    }
    if project
        .layers
        .values()
        .filter(|v| v.definition.document_id.as_ref() == Some(&document.to_proto()))
        .count()
        > MAX_LAYERS
        || project
            .objects
            .values()
            .filter(|v| &v.document_id == document)
            .count()
            > MAX_OBJECTS
    {
        return Err(WorkflowError::Limit);
    }
    let asset = project
        .assets
        .get(&AssetId::try_from(definition.primary_asset_id.clone())?)
        .ok_or(WorkflowError::Missing)?;
    if !matches!(asset.bit_depth, 8 | 16)
        || asset.width == 0
        || asset.height == 0
        || !(1..=8).contains(&asset.orientation)
    {
        return Err(WorkflowError::Invalid);
    }
    Ok(asset)
}
fn dimensions(asset: &pb::AddAsset) -> [u32; 2] {
    if asset.orientation >= 5 {
        [asset.height, asset.width]
    } else {
        [asset.width, asset.height]
    }
}

#[uniffi::export]
impl ProjectSession {
    pub async fn tone_controls_snapshot(
        &self,
        document_id: String,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<ToneControlSnapshot> {
        self.check_open()?;
        let document = id(&document_id)?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&closed, &cancellation)?;
                    admit(state, memory_budget_bytes, false, TONE_WORK)?;
                    let result = snapshot::snapshot(state, &document, &cancellation)?;
                    check_live(&closed, &cancellation)?;
                    Ok(result)
                })())
            })
            .await?
    }
    pub async fn prepare_tone_control(
        self: Arc<Self>,
        binding: WorkflowBinding,
        metadata: WorkflowMetadata,
        command: ToneControlCommand,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<Arc<WorkflowPlan>> {
        self.check_open()?;
        binding.validate()?;
        metadata.validate()?;
        plan::validate(&command)?;
        cancellation.check()?;
        let permit = HandlePermit::acquire()?;
        let owner = self.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&owner.closed, &cancellation)?;
                    admit(state, memory_budget_bytes, true, TONE_WORK)?;
                    check_binding(state, &binding)?;
                    check_metadata(state, &metadata)?;
                    let document = id(&binding.document_id)?;
                    bounds(state.project()?, &document)?;
                    snapshot::tones(state.project()?, &document, &cancellation)?;
                    let (transaction, next) =
                        plan::transaction(state, &document, &metadata, command)?;
                    admit_transaction(state, &transaction, memory_budget_bytes, TONE_WORK)?;
                    if let Some(replica) = &state.replica {
                        let mut candidate = replica.clone();
                        candidate
                            .queue(transaction.clone())
                            .map_err(plan::operation_error)?;
                    } else {
                        state
                            .store()?
                            .validate_transaction(
                                &transaction,
                                &vw_model::DeviceId::try_from(metadata.device_id.clone())?,
                                metadata.created_at_ms,
                            )
                            .map_err(|e| match e {
                                vw_store::StoreError::Operation(e) => plan::operation_error(e),
                                _ => WorkflowError::Storage,
                            })?;
                    }
                    check_live(&owner.closed, &cancellation)?;
                    WorkflowPlan::new(
                        &owner,
                        state,
                        binding,
                        transaction,
                        next,
                        memory_budget_bytes,
                        TONE_WORK,
                        permit,
                    )
                })())
            })
            .await?
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn preview_tone_control(
        &self,
        binding: WorkflowBinding,
        command: Option<ToneControlCommand>,
        region: crate::SelectionRegion,
        assume_untagged_srgb: bool,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<ToneControlPreview> {
        self.check_open()?;
        binding.validate()?;
        if let Some(value) = &command {
            plan::validate(value)?;
        }
        if region.width == 0 || region.height == 0 || region.width > 512 || region.height > 512 {
            return Err(WorkflowError::Invalid);
        }
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&closed, &cancellation)?;
                    let value = preview::preview(
                        state,
                        binding,
                        command,
                        region,
                        assume_untagged_srgb,
                        memory_budget_bytes,
                        &cancellation,
                    )?;
                    check_live(&closed, &cancellation)?;
                    Ok(value)
                })())
            })
            .await?
    }
}
