//! Bounded typed metadata controls over existing canonical ops. No pixel assets
//! are rewritten; mutation plans share the reviewed visible-CAS/undo/replica path.
mod dto;
mod plan;
mod snapshot;
use crate::{Cancellation, ProjectSession, workflow::*};
pub use dto::*;
use std::sync::Arc;
use vw_model::Id;
use vw_proto::v1 as pb;

const MAX_DOCUMENTS: usize = 128;
const MAX_LAYERS: usize = 256;
const MAX_OBJECTS: usize = 4096;
const MAX_GROUPS: usize = 256;
const MAX_TARGETS: usize = 256;
// Covers bounded source DTO + staging + generated/common DTO copies: <=384
// 4096-byte titles with eight representations, <=4096 1KiB object row envelopes,
// group/ID collections and command construction. Canonical/new-txn clones are
// separately charged by the reviewed workflow meter before hashing/validation.
const CONTROL_WORK: u64 = 32 * 1024 * 1024;

fn bounded_id(value: &str) -> WorkflowResult<Id> {
    if value.len() != 36 {
        return Err(WorkflowError::Invalid);
    }
    Ok(Id::try_from(value.to_owned())?)
}
fn name(value: &str) -> WorkflowResult<()> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(WorkflowError::Invalid);
    }
    Ok(())
}

#[uniffi::export]
impl ProjectSession {
    pub async fn document_controls_snapshot(
        &self,
        document_id: String,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<DocumentControlSnapshot> {
        self.check_open()?;
        let document = bounded_id(&document_id)?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&closed, &cancellation)?;
                    admit(state, memory_budget_bytes, false, CONTROL_WORK)?;
                    snapshot::snapshot(state, &document, &cancellation)
                })())
            })
            .await?
    }
    pub async fn prepare_document_control(
        self: Arc<Self>,
        binding: WorkflowBinding,
        metadata: WorkflowMetadata,
        command: DocumentControlCommand,
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
                    admit(state, memory_budget_bytes, true, CONTROL_WORK)?;
                    check_binding(state, &binding)?;
                    check_metadata(state, &metadata)?;
                    let document = bounded_id(&binding.document_id)?;
                    snapshot::bounds(state.project()?, &document)?;
                    let (transaction, next) =
                        plan::transaction(state, &document, &metadata, command, &cancellation)?;
                    admit_transaction(state, &transaction, memory_budget_bytes, CONTROL_WORK)?;
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
                        CONTROL_WORK,
                        permit,
                    )
                })())
            })
            .await?
    }
}
