//! Additive source candidate: root registers this module and its proposed ABI.
//! Every mutation stays on the existing project worker and normal commit path.
mod dto;
use crate::{
    Cancellation, ProjectSession, WorkflowError, WorkflowMetadata, WorkflowReceipt,
    project::ProjectState,
    workflow::{self, WorkflowResult},
};
pub use dto::*;
use std::sync::{Arc, atomic::Ordering};
use vw_model::{AssetId, Id, StateHash};
use vw_proto::v1 as pb;
use vw_store::{ConflictReviewStatus as Status, StoreError};

// One bounded conflict/draft and their JSON/protobuf representations coexist
// with the existing speculative transaction/replica/history clone admission.
const REVIEW_WORK: u64 = 160 * 1024 * 1024;
const PREVIEW_BYTES: usize = 4096;
const VALUE_BYTES: usize = 16 * 1024 * 1024;

fn error(value: StoreError) -> WorkflowError {
    use vw_ops::OpsError;
    match value {
        StoreError::Operation(OpsError::Locked) => WorkflowError::Locked,
        StoreError::Operation(OpsError::NotFound) => WorkflowError::Missing,
        StoreError::Operation(OpsError::RevisionMismatch) => WorkflowError::Stale,
        StoreError::Operation(OpsError::IdCollision) => WorkflowError::ReusedTransaction,
        StoreError::Invalid("review changed or pending edits") => WorkflowError::Stale,
        StoreError::Invalid("review identity reuse" | "review already started") => {
            WorkflowError::ReusedTransaction
        }
        StoreError::Invalid(_) | StoreError::Model(_) | StoreError::Operation(_) => {
            WorkflowError::Invalid
        }
        _ => WorkflowError::Storage,
    }
}
fn id(value: &str) -> WorkflowResult<Id> {
    if value.len() != 36 {
        return Err(WorkflowError::Invalid);
    }
    Ok(Id::try_from(value.to_owned())?)
}
fn binding(state: &ProjectState) -> WorkflowResult<OfflineReviewBinding> {
    let revision = state.view_revision()?;
    Ok(OfflineReviewBinding {
        project_id: state.project()?.id.to_string(),
        host_seq: revision.host_seq,
        state_hash: hash_text(&revision.state_hash)?,
    })
}
fn hash_text(bytes: &[u8]) -> WorkflowResult<String> {
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| WorkflowError::Invalid)?;
    Ok(blake3::Hash::from_bytes(bytes).to_hex().to_string())
}
fn check_binding(state: &ProjectState, expected: &OfflineReviewBinding) -> WorkflowResult<()> {
    id(&expected.project_id)?;
    if expected.state_hash.len() != 64 {
        return Err(WorkflowError::Invalid);
    }
    StateHash::try_from(expected.state_hash.clone())?;
    if &binding(state)? != expected {
        return Err(WorkflowError::Stale);
    }
    Ok(())
}
fn progress(value: Status) -> OfflineReviewProgress {
    match value {
        Status::Unreviewed => OfflineReviewProgress::Unreviewed,
        Status::Kept => OfflineReviewProgress::Kept,
        Status::PreviouslyResolved => OfflineReviewProgress::PreviouslyResolved,
        Status::Draft => OfflineReviewProgress::Draft,
        Status::Queued => OfflineReviewProgress::Queued,
        Status::Applied => OfflineReviewProgress::Applied,
    }
}
/// Root also calls this at the end of History::restore, before initial project
/// info and before admitting ordinary edits. Do not register intents as undo.
pub(crate) fn restore_counter(state: &mut ProjectState) -> WorkflowResult<()> {
    let floor = state
        .store()?
        .conflict_review_lamport_floor()
        .map_err(error)?;
    state.history.next_lamport = state.history.next_lamport.max(floor);
    Ok(())
}
fn now_ms() -> WorkflowResult<i64> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| WorkflowError::Storage)?;
    i64::try_from(elapsed.as_millis()).map_err(|_| WorkflowError::Storage)
}
fn preview(bytes: &[u8]) -> WorkflowResult<OfflineReviewValue> {
    if bytes.len() > VALUE_BYTES {
        return Err(WorkflowError::Limit);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| WorkflowError::Invalid)?;
    let mut end = text.len().min(PREVIEW_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Ok(OfflineReviewValue {
        preview: text[..end].to_owned(),
        utf8_bytes: bytes.len() as u64,
        blake3: Some(AssetId::hash(bytes).to_string()),
        truncated: end != text.len(),
        available: true,
    })
}
fn historical(value: &Option<pb::PropertyValue>) -> WorkflowResult<OfflineReviewValue> {
    let Some(pb::property_value::Value::Raw(bytes)) = value.as_ref().and_then(|v| v.value.as_ref())
    else {
        return Err(WorkflowError::Invalid);
    };
    preview(bytes)
}
fn current_value(
    state: &ProjectState,
    record: &pb::ConflictRecord,
) -> WorkflowResult<OfflineReviewValue> {
    let entity = Id::from_proto(record.object_id.as_ref())?;
    let (collection, path) = record
        .property
        .split_once('.')
        .ok_or(WorkflowError::Invalid)?;
    let project = state.project()?;
    let root = match collection {
        "objects" => project.objects.get(&entity).map(serde_json::to_value),
        "documents" => project.documents.get(&entity).map(serde_json::to_value),
        "layers" => project.layers.get(&entity).map(serde_json::to_value),
        _ => {
            return Ok(OfflineReviewValue {
                preview: String::new(),
                utf8_bytes: 0,
                blake3: None,
                truncated: false,
                available: false,
            });
        }
    }
    .transpose()
    .map_err(|_| WorkflowError::Invalid)?;
    let mut cursor = root.as_ref();
    if path != "$entity" {
        for segment in path.split('.') {
            cursor = cursor.and_then(|v| v.get(segment));
        }
    }
    preview(&serde_json::to_vec(&cursor).map_err(|_| WorkflowError::Invalid)?)
}
fn details(state: &ProjectState, conflict: &Id) -> WorkflowResult<OfflineReviewDetails> {
    let review = state.store()?.conflict_review(conflict).map_err(error)?;
    let record = &review.record;
    let operation = review
        .transaction
        .as_ref()
        .map(|t| preview(&serde_json::to_vec(&t.ops[0].kind).map_err(|_| WorkflowError::Invalid)?))
        .transpose()?;
    let reviewed = review
        .transaction
        .as_ref()
        .map(|t| -> WorkflowResult<OfflineReviewBinding> {
            let r = t.base_revision.as_ref().ok_or(WorkflowError::Invalid)?;
            Ok(OfflineReviewBinding {
                project_id: state.project()?.id.to_string(),
                host_seq: r.host_seq,
                state_hash: hash_text(&r.state_hash)?,
            })
        })
        .transpose()?;
    Ok(OfflineReviewDetails {
        binding: binding(state)?,
        conflict_id: conflict.to_string(),
        object_id: Id::from_proto(record.object_id.as_ref())?.to_string(),
        property: record.property.clone(),
        delete_vs_edit: record.delete_vs_edit,
        kept: historical(&record.kept_value)?,
        other: historical(&record.other_value)?,
        current: current_value(state, record)?,
        operation,
        transaction_id: review
            .transaction
            .as_ref()
            .map(|t| Id::from_proto(t.txn_id.as_ref()).map(|id| id.to_string()))
            .transpose()?,
        reviewed,
        progress: progress(review.status),
        next_lamport: state.history.next_lamport,
    })
}
fn complete_preview(value: &OfflineReviewDetails) -> WorkflowResult<()> {
    if [&value.kept, &value.other, &value.current]
        .into_iter()
        .any(|v| !v.available || v.truncated)
        || value
            .operation
            .as_ref()
            .is_none_or(|v| !v.available || v.truncated)
    {
        return Err(WorkflowError::Limit);
    }
    Ok(())
}

#[uniffi::export]
impl ProjectSession {
    pub async fn offline_review_page(
        &self,
        after_row: i64,
        limit: u32,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<OfflineReviewPage> {
        self.check_open()?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    workflow::check_live(&closed, &cancellation)?;
                    workflow::admit(state, memory_budget_bytes, false, 512 * 1024)?;
                    let entries = state
                        .store()?
                        .conflict_review_ids(after_row, limit)
                        .map_err(error)?
                        .into_iter()
                        .map(|(row, conflict)| -> WorkflowResult<OfflineReviewIndex> {
                            let (property, delete_vs_edit) = state
                                .store()?
                                .conflict_review_label(&conflict)
                                .map_err(error)?;
                            Ok(OfflineReviewIndex {
                                row,
                                conflict_id: conflict.to_string(),
                                property,
                                delete_vs_edit,
                            })
                        })
                        .collect::<WorkflowResult<Vec<_>>>()?;
                    Ok(OfflineReviewPage {
                        binding: binding(state)?,
                        entries,
                    })
                })())
            })
            .await?
    }
    pub async fn offline_review_details(
        &self,
        conflict_id: String,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<OfflineReviewDetails> {
        self.check_open()?;
        let conflict = id(&conflict_id)?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    workflow::check_live(&closed, &cancellation)?;
                    workflow::admit(state, memory_budget_bytes, false, REVIEW_WORK)?;
                    restore_counter(state)?;
                    details(state, &conflict)
                })())
            })
            .await?
    }
    pub async fn stage_offline_review(
        &self,
        conflict_id: String,
        expected: OfflineReviewBinding,
        metadata: WorkflowMetadata,
        choice: OfflineReviewChoice,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<OfflineReviewMutation> {
        self.check_open()?;
        let conflict = id(&conflict_id)?;
        metadata.validate()?;
        let choice = match choice {
            OfflineReviewChoice::SwapOther => vw_store::ConflictReviewChoice::SwapOther,
            OfflineReviewChoice::RestoreCopy {
                document_id,
                object_id,
            } => vw_store::ConflictReviewChoice::RestoreCopy {
                document: id(&document_id)?,
                object: id(&object_id)?,
            },
        };
        let closed = self.closed.clone();
        let active = self.active_gestures.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    workflow::check_live(&closed, &cancellation)?;
                    workflow::admit(state, memory_budget_bytes, true, REVIEW_WORK)?;
                    check_binding(state, &expected)?;
                    restore_counter(state)?;
                    workflow::check_metadata(state, &metadata)?;
                    if active.load(Ordering::Acquire) != 0 {
                        return Err(WorkflowError::Backpressure);
                    }
                    let revision = state.store()?.revision()?;
                    workflow::check_live(&closed, &cancellation)?;
                    state
                        .store
                        .as_mut()
                        .ok_or(WorkflowError::Closed)?
                        .stage_conflict_review(
                            &conflict,
                            &revision,
                            choice,
                            vw_store::ConflictReviewIdentity {
                                transaction: id(&metadata.transaction_id)?,
                                lamport: metadata.first_lamport,
                                wall_ms: metadata.created_at_ms,
                            },
                        )
                        .map_err(error)?;
                    // After this point the draft is durable; no cancellation can mean
                    // "unsaved". Restore counters before any later fallible projection.
                    restore_counter(state)?;
                    Ok(OfflineReviewMutation {
                        details: details(state, &conflict)?,
                        project: state.info()?,
                    })
                })())
            })
            .await?
    }
    pub async fn submit_offline_review(
        &self,
        conflict_id: String,
        expected: OfflineReviewBinding,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<OfflineReviewSubmission> {
        self.check_open()?;
        let conflict = id(&conflict_id)?;
        let closed = self.closed.clone();
        let active = self.active_gestures.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    workflow::check_live(&closed, &cancellation)?;
                    workflow::admit(state, memory_budget_bytes, true, REVIEW_WORK)?;
                    check_binding(state, &expected)?;
                    if active.load(Ordering::Acquire) != 0 {
                        return Err(WorkflowError::Backpressure);
                    }
                    let shown = details(state, &conflict)?;
                    // An already delivered retry needs no new value authorization.
                    if shown.progress == OfflineReviewProgress::Draft {
                        complete_preview(&shown)?;
                    }
                    let now = now_ms()?;
                    let transaction = state
                        .store()?
                        .conflict_review_submission(&conflict, now)
                        .map_err(error)?;
                    workflow::admit_transaction(
                        state,
                        &transaction,
                        memory_budget_bytes,
                        REVIEW_WORK,
                    )?;
                    let transaction_id = Id::from_proto(transaction.txn_id.as_ref())?;
                    let duplicate = state.previous(&transaction_id.to_proto())?.is_some();
                    let local = state.store()?.local_device()?;
                    workflow::check_live(&closed, &cancellation)?;
                    // Existing path prepares replica + history, persists, adopts them,
                    // publishes ChangeKind::Committed and wakes the existing OPS loop.
                    let revision = if duplicate {
                        state.info()?
                    } else {
                        state.commit(&transaction, &local, now)?
                    };
                    let progress = if state.replica.is_some()
                        && !state
                            .store()?
                            .accepted_transactions()
                            .any(|(t, _)| t.txn_id == transaction.txn_id)
                    {
                        OfflineReviewProgress::Queued
                    } else {
                        OfflineReviewProgress::Applied
                    };
                    let metadata_pending = state
                        .store
                        .as_mut()
                        .ok_or(WorkflowError::Closed)?
                        .submit_conflict_review(&conflict, now)
                        .is_err();
                    // Never manufacture cancellation after durable publication.
                    Ok(OfflineReviewSubmission {
                        receipt: WorkflowReceipt {
                            transaction_id: transaction_id.to_string(),
                            revision,
                            duplicate,
                        },
                        progress,
                        metadata_pending,
                    })
                })())
            })
            .await?
    }
    pub async fn keep_offline_review(
        &self,
        conflict_id: String,
        expected: OfflineReviewBinding,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<OfflineReviewMutation> {
        self.change_offline_review(
            conflict_id,
            expected,
            true,
            memory_budget_bytes,
            cancellation,
        )
        .await
    }
    pub async fn cancel_offline_review_draft(
        &self,
        conflict_id: String,
        expected: OfflineReviewBinding,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<OfflineReviewMutation> {
        self.change_offline_review(
            conflict_id,
            expected,
            false,
            memory_budget_bytes,
            cancellation,
        )
        .await
    }
}
impl ProjectSession {
    async fn change_offline_review(
        &self,
        conflict_id: String,
        expected: OfflineReviewBinding,
        keep: bool,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<OfflineReviewMutation> {
        self.check_open()?;
        let conflict = id(&conflict_id)?;
        let closed = self.closed.clone();
        let active = self.active_gestures.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    workflow::check_live(&closed, &cancellation)?;
                    workflow::admit(state, memory_budget_bytes, true, REVIEW_WORK)?;
                    check_binding(state, &expected)?;
                    if active.load(Ordering::Acquire) != 0 {
                        return Err(WorkflowError::Backpressure);
                    }
                    let revision = state.store()?.revision()?;
                    workflow::check_live(&closed, &cancellation)?;
                    let store = state.store.as_mut().ok_or(WorkflowError::Closed)?;
                    if keep {
                        store
                            .keep_conflict_current(&conflict, &revision, now_ms()?)
                            .map_err(error)?;
                    } else {
                        store.cancel_conflict_review(&conflict).map_err(error)?;
                    }
                    Ok(OfflineReviewMutation {
                        details: details(state, &conflict)?,
                        project: state.info()?,
                    })
                })())
            })
            .await?
    }
}

#[cfg(test)]
mod tests;
