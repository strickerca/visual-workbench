//! Nondestructive output frames over existing canonical operations.
//! D remains the immutable source grid; O is the explicit export grid.
mod dto;
mod pixels;
mod plan;
mod snapshot;

use crate::{Cancellation, ProjectSession, workflow::*};
pub use dto::*;
pub(crate) use pixels::{PreparedOutput, admit_legacy_export, legacy_export, prepare_output};
use std::sync::Arc;
use vw_model::{AssetId, Id, Project};
use vw_proto::v1 as pb;

const FRAME_WORK: u64 = 16 * 1024 * 1024;
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
    pub async fn image_frames_snapshot(
        &self,
        document_id: String,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<ImageFramesSnapshot> {
        self.check_open()?;
        let document = id(&document_id)?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&closed, &cancellation)?;
                    admit(state, memory_budget_bytes, false, FRAME_WORK)?;
                    let result = snapshot::snapshot(state, &document, &cancellation)?;
                    check_live(&closed, &cancellation)?;
                    Ok(result)
                })())
            })
            .await?
    }
    pub async fn prepare_image_frame(
        self: Arc<Self>,
        binding: WorkflowBinding,
        metadata: WorkflowMetadata,
        command: ImageFrameCommand,
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
                    admit(state, memory_budget_bytes, true, FRAME_WORK)?;
                    check_binding(state, &binding)?;
                    check_metadata(state, &metadata)?;
                    let document = id(&binding.document_id)?;
                    bounds(state.project()?, &document)?;
                    state.project()?.image_output(&document)?;
                    let (transaction, next) =
                        plan::transaction(state, &document, &metadata, command)?;
                    admit_transaction(state, &transaction, memory_budget_bytes, FRAME_WORK)?;
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
                        FRAME_WORK,
                        permit,
                    )
                })())
            })
            .await?
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn image_frame_preview(
        &self,
        binding: WorkflowBinding,
        command: Option<ImageFrameCommand>,
        region: crate::SelectionRegion,
        marked: bool,
        assume_untagged_srgb: bool,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<ImageFramePreview> {
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
                    let value = pixels::preview(
                        state,
                        binding,
                        command,
                        region,
                        marked,
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
    pub async fn image_editing_preview(
        &self,
        binding: WorkflowBinding,
        region: crate::SelectionRegion,
        assume_untagged_srgb: bool,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<ImageEditingPreview> {
        self.check_open()?;
        binding.validate()?;
        if region.width == 0 || region.height == 0 || region.width > 512 || region.height > 512 {
            return Err(WorkflowError::Invalid);
        }
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&closed, &cancellation)?;
                    let result = pixels::editing(
                        state,
                        binding,
                        region,
                        assume_untagged_srgb,
                        memory_budget_bytes,
                        &cancellation,
                    )?;
                    check_live(&closed, &cancellation)?;
                    Ok(result)
                })())
            })
            .await?
    }
    /// Whole D-grid composition for the existing ordinary-image background
    /// owner. It includes annotations and tones; callers must not double-paint
    /// canonical vectors, and must fence the reply by its complete binding.
    pub async fn image_editing_background(
        &self,
        binding: WorkflowBinding,
        assume_untagged_srgb: bool,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<ImageEditingBackground> {
        self.check_open()?;
        binding.validate()?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&closed, &cancellation)?;
                    let result = pixels::background(
                        state,
                        binding,
                        assume_untagged_srgb,
                        memory_budget_bytes,
                        &cancellation,
                    )?;
                    check_live(&closed, &cancellation)?;
                    Ok(result)
                })())
            })
            .await?
    }
}

/// Only validated dedicated output metadata is omitted from the vector editing
/// query. Ordinary Crop objects retain their existing guide semantics.
pub(crate) fn is_editing_metadata(project: &Project, document: &Id, object: &Id) -> bool {
    project
        .image_output(document)
        .ok()
        .flatten()
        .is_some_and(|value| value.object_id == object)
}
