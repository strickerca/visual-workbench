//! Shared admission and exact visible-revision fences for additive workflows.
//! This module requires the reviewed T2.01 borrowed workspace-accounting hooks.
use crate::{
    Cancellation, CoreError, ProjectInfo, ProjectSession, project::ProjectState, worker::Worker,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use vw_model::{DeviceId, Id, StateHash};
use vw_proto::v1 as pb;

pub(crate) const MAX_MEMORY: u64 = 256 * 1024 * 1024;
pub(crate) const INSTRUCTION_WORK: u64 = 48 * 1024 * 1024;
pub(crate) const SEMANTIC_WORK: u64 = 64 * 1024 * 1024;
static HANDLES: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, thiserror::Error, uniffi::Error)]
pub enum WorkflowError {
    #[error("invalid bounded workflow input")]
    Invalid,
    #[error("workflow resource limit exceeded")]
    Limit,
    #[error("workflow workspace exceeds the caller budget")]
    Memory { estimated: u64, budget: u64 },
    #[error("project, document or visible revision changed; refresh before retrying")]
    Stale,
    #[error("referenced content is unavailable")]
    Missing,
    #[error("an instruction target or its layer is locked")]
    Locked,
    #[error("marker links or numbering require explicit reconciliation")]
    Reconciliation,
    #[error("instruction targets were deleted; repair the references explicitly")]
    Detached,
    #[error("transaction identifier is already used; inspect its durable result")]
    ReusedTransaction,
    #[error("workflow or project is closed")]
    Closed,
    #[error("workflow cancelled before its commit point")]
    Cancelled,
    #[error("workflow capacity exhausted")]
    Backpressure,
    #[error("workflow storage failure; inspect the durable revision")]
    Storage,
}
pub type WorkflowResult<T> = std::result::Result<T, WorkflowError>;
impl From<CoreError> for WorkflowError {
    fn from(value: CoreError) -> Self {
        match value {
            CoreError::Closed => Self::Closed,
            CoreError::Cancelled => Self::Cancelled,
            CoreError::Backpressure | CoreError::Worker => Self::Backpressure,
            CoreError::Storage => Self::Storage,
            _ => Self::Invalid,
        }
    }
}
impl From<vw_model::ModelError> for WorkflowError {
    fn from(_: vw_model::ModelError) -> Self {
        Self::Invalid
    }
}
impl From<vw_store::StoreError> for WorkflowError {
    fn from(_: vw_store::StoreError) -> Self {
        Self::Storage
    }
}
impl From<vw_geom::GeometryError> for WorkflowError {
    fn from(_: vw_geom::GeometryError) -> Self {
        Self::Invalid
    }
}
impl From<vw_instructions::Error> for WorkflowError {
    fn from(value: vw_instructions::Error) -> Self {
        use vw_instructions::Error as E;
        match value {
            E::Limit(_) | E::Exhausted => Self::Limit,
            E::Missing => Self::Missing,
            E::Stale => Self::Stale,
            E::Locked => Self::Locked,
            E::Reconciliation => Self::Reconciliation,
            E::Detached => Self::Detached,
            E::Closed => Self::Closed,
            _ => Self::Invalid,
        }
    }
}
impl From<vw_semantics::Error> for WorkflowError {
    fn from(value: vw_semantics::Error) -> Self {
        use vw_semantics::Error as E;
        match value {
            E::Limit(_) => Self::Limit,
            E::Missing => Self::Missing,
            E::Stale => Self::Stale,
            E::Cancelled => Self::Cancelled,
            _ => Self::Invalid,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct WorkflowBinding {
    pub project_id: String,
    pub document_id: String,
    pub host_seq: u64,
    pub state_hash: String,
}
impl WorkflowBinding {
    pub(crate) fn validate(&self) -> WorkflowResult<()> {
        if self.project_id.len() != 36
            || self.document_id.len() != 36
            || self.state_hash.len() != 64
        {
            return Err(WorkflowError::Invalid);
        }
        Id::try_from(self.project_id.clone())?;
        Id::try_from(self.document_id.clone())?;
        StateHash::try_from(self.state_hash.clone())?;
        Ok(())
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct WorkflowMetadata {
    pub transaction_id: String,
    pub device_id: String,
    pub first_lamport: u64,
    pub created_at_ms: i64,
}
impl WorkflowMetadata {
    pub(crate) fn validate(&self) -> WorkflowResult<()> {
        if self.transaction_id.len() != 36
            || self.device_id.len() > 64
            || self.first_lamport == 0
            || !(0..(1i64 << 48)).contains(&self.created_at_ms)
        {
            return Err(WorkflowError::Invalid);
        }
        Id::try_from(self.transaction_id.clone())?;
        DeviceId::try_from(self.device_id.clone())?;
        Ok(())
    }
    pub(crate) fn instruction(&self) -> WorkflowResult<vw_instructions::EditMetadata> {
        self.validate()?;
        Ok(vw_instructions::EditMetadata {
            transaction_id: Id::try_from(self.transaction_id.clone())?,
            device: DeviceId::try_from(self.device_id.clone())?,
            first_lamport: self.first_lamport,
            created_at_ms: self.created_at_ms,
        })
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct WorkflowPlanInfo {
    pub transaction_id: String,
    pub expected: WorkflowBinding,
    pub operation_count: u32,
    pub next_lamport: u64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct WorkflowReceipt {
    pub transaction_id: String,
    pub revision: ProjectInfo,
    pub duplicate: bool,
}

pub(crate) struct HandlePermit;
impl HandlePermit {
    pub(crate) fn acquire() -> WorkflowResult<Self> {
        HANDLES
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 16).then_some(n + 1)
            })
            .map_err(|_| WorkflowError::Backpressure)?;
        Ok(Self)
    }
}
impl Drop for HandlePermit {
    fn drop(&mut self) {
        HANDLES.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Resident state is not charged twice, but every new clone/hash/serialization
/// representation is. This is the same policy as the reviewed mask helper.
pub(crate) fn canonical_workspace(
    state: &ProjectState,
    budget: u64,
    mutation: bool,
) -> WorkflowResult<u64> {
    if budget == 0 || budget > MAX_MEMORY {
        return Err(WorkflowError::Invalid);
    }
    let copies = if mutation { 16 } else { 4 };
    let reserve = 4 * 1024 * 1024;
    let single_limit = budget.saturating_sub(reserve) / copies;
    let mut single = state.store()?.workspace_estimate_bytes(single_limit)?;
    if single <= single_limit
        && let Some(replica) = &state.replica
    {
        single = single
            .checked_add(
                replica
                    .workspace_estimate_bytes(single_limit - single)
                    .map_err(|_| WorkflowError::Invalid)?,
            )
            .ok_or(WorkflowError::Limit)?;
    }
    let estimated = single
        .checked_mul(copies)
        .and_then(|n| n.checked_add(reserve))
        .ok_or(WorkflowError::Limit)?;
    if estimated > budget {
        return Err(WorkflowError::Memory { estimated, budget });
    }
    Ok(estimated)
}
pub(crate) fn admit(
    state: &ProjectState,
    budget: u64,
    mutation: bool,
    extra: u64,
) -> WorkflowResult<()> {
    if budget == 0 || budget > MAX_MEMORY {
        return Err(WorkflowError::Invalid);
    }
    if budget <= extra {
        return Err(WorkflowError::Memory {
            estimated: extra + 1,
            budget,
        });
    }
    let canonical = match canonical_workspace(state, budget - extra, mutation) {
        Err(WorkflowError::Memory { estimated, .. }) => {
            return Err(WorkflowError::Memory {
                estimated: estimated.checked_add(extra).ok_or(WorkflowError::Limit)?,
                budget,
            });
        }
        other => other?,
    };
    if canonical.checked_add(extra).ok_or(WorkflowError::Limit)? > budget {
        return Err(WorkflowError::Limit);
    }
    Ok(())
}
/// Newly planned transaction content is not resident in the admitted store.
/// Charge its borrowed canonical representation before cloning or committing it.
pub(crate) fn admit_transaction(
    state: &ProjectState,
    transaction: &pb::Transaction,
    budget: u64,
    extra: u64,
) -> WorkflowResult<()> {
    if budget == 0 || budget > MAX_MEMORY {
        return Err(WorkflowError::Invalid);
    }
    let copies = 16u64;
    let single_limit = budget.saturating_sub(extra) / copies;
    let single =
        vw_ops::HostSequencer::transaction_workspace_estimate_bytes(transaction, single_limit)
            .map_err(|_| WorkflowError::Invalid)?;
    let transaction_work = single.checked_mul(copies).ok_or(WorkflowError::Limit)?;
    let all_extra = extra
        .checked_add(transaction_work)
        .ok_or(WorkflowError::Limit)?;
    admit(state, budget, true, all_extra)
}
pub(crate) fn current(state: &ProjectState, document: &str) -> WorkflowResult<WorkflowBinding> {
    let id = Id::try_from(document.to_owned())?;
    if !state.project()?.documents.contains_key(&id) {
        return Err(WorkflowError::Missing);
    }
    let revision = state.view_revision()?;
    Ok(WorkflowBinding {
        project_id: state.project()?.id.to_string(),
        document_id: id.to_string(),
        host_seq: revision.host_seq,
        state_hash: state.project()?.state_hash()?.to_string(),
    })
}
pub(crate) fn check_binding(
    state: &ProjectState,
    expected: &WorkflowBinding,
) -> WorkflowResult<()> {
    expected.validate()?;
    if &current(state, &expected.document_id)? != expected {
        return Err(WorkflowError::Stale);
    }
    Ok(())
}
pub(crate) fn check_metadata(state: &ProjectState, value: &WorkflowMetadata) -> WorkflowResult<()> {
    value.validate()?;
    if DeviceId::try_from(value.device_id.clone())? != state.store()?.local_device()? {
        return Err(WorkflowError::Invalid);
    }
    if state
        .previous(&Id::try_from(value.transaction_id.clone())?.to_proto())?
        .is_some()
    {
        return Err(WorkflowError::ReusedTransaction);
    }
    if value.first_lamport < state.history.next_lamport {
        return Err(WorkflowError::Invalid);
    }
    Ok(())
}
pub(crate) fn check_live(closed: &AtomicBool, cancellation: &Cancellation) -> WorkflowResult<()> {
    cancellation.check()?;
    if closed.load(Ordering::Acquire) {
        return Err(WorkflowError::Closed);
    }
    Ok(())
}

struct Prepared {
    transaction: pb::Transaction,
    info: WorkflowPlanInfo,
    budget: u64,
    extra: u64,
    _permit: HandlePermit,
}
#[derive(uniffi::Object)]
pub struct WorkflowPlan {
    worker: Worker<ProjectState>,
    project_closed: Arc<AtomicBool>,
    disposed: Arc<AtomicBool>,
    prepared: Mutex<Option<Arc<Prepared>>>,
}
impl WorkflowPlan {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        project: &ProjectSession,
        state: &ProjectState,
        expected: WorkflowBinding,
        mut transaction: pb::Transaction,
        next_lamport: u64,
        budget: u64,
        extra: u64,
        permit: HandlePermit,
    ) -> WorkflowResult<Arc<Self>> {
        check_binding(state, &expected)?;
        if transaction.project_id != Some(Id::try_from(expected.project_id.clone())?.to_proto())
            || transaction.ops.is_empty()
            || transaction.ops.len() > 4096
        {
            return Err(WorkflowError::Invalid);
        }
        // Visible-state CAS above protects the plan. Wire transactions must use
        // the accepted store head; optimistic visible hashes are not host heads.
        transaction.base_revision = Some(state.store()?.revision()?);
        let info = WorkflowPlanInfo {
            transaction_id: Id::from_proto(transaction.txn_id.as_ref())?.to_string(),
            expected,
            operation_count: transaction.ops.len() as u32,
            next_lamport,
        };
        Ok(Arc::new(Self {
            worker: project.worker.clone(),
            project_closed: project.closed.clone(),
            disposed: Arc::new(AtomicBool::new(false)),
            prepared: Mutex::new(Some(Arc::new(Prepared {
                transaction,
                info,
                budget,
                extra,
                _permit: permit,
            }))),
        }))
    }
    fn prepared(&self) -> WorkflowResult<Arc<Prepared>> {
        if self.disposed.load(Ordering::Acquire) || self.project_closed.load(Ordering::Acquire) {
            return Err(WorkflowError::Closed);
        }
        self.prepared
            .lock()
            .map_err(|_| WorkflowError::Closed)?
            .as_ref()
            .cloned()
            .ok_or(WorkflowError::Closed)
    }
}
#[uniffi::export]
impl WorkflowPlan {
    pub fn describe(&self) -> WorkflowResult<WorkflowPlanInfo> {
        Ok(self.prepared()?.info.clone())
    }
    /// Exact retries of this immutable handle return the already durable result.
    /// Recreating a plan with a used ID refuses instead of guessing old intent.
    pub async fn commit(&self, cancellation: Arc<Cancellation>) -> WorkflowResult<WorkflowReceipt> {
        let prepared = self.prepared()?;
        let closed = self.project_closed.clone();
        let disposed = self.disposed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&closed, &cancellation)?;
                    if disposed.load(Ordering::Acquire) {
                        return Err(WorkflowError::Closed);
                    }
                    admit_transaction(
                        state,
                        &prepared.transaction,
                        prepared.budget,
                        prepared.extra,
                    )?;
                    let id = Id::from_proto(prepared.transaction.txn_id.as_ref())?.to_proto();
                    let duplicate = if let Some(previous) = state.previous(&id)? {
                        if previous != &prepared.transaction {
                            return Err(WorkflowError::ReusedTransaction);
                        }
                        true
                    } else {
                        check_binding(state, &prepared.info.expected)?;
                        false
                    };
                    check_live(&closed, &cancellation)?;
                    if disposed.load(Ordering::Acquire) {
                        return Err(WorkflowError::Closed);
                    }
                    let revision = if duplicate {
                        state.info()?
                    } else {
                        state.commit(
                            &prepared.transaction,
                            &DeviceId::try_from(prepared.transaction.device_id.clone())?,
                            prepared.transaction.created_at_wall_ms,
                        )?
                    };
                    // Never report cancellation after durable publication.
                    Ok(WorkflowReceipt {
                        transaction_id: prepared.info.transaction_id.clone(),
                        revision,
                        duplicate,
                    })
                })())
            })
            .await?
    }
    pub fn dispose(&self) {
        self.disposed.store(true, Ordering::Release);
        if let Ok(mut prepared) = self.prepared.lock() {
            *prepared = None;
        }
    }
}
impl Drop for WorkflowPlan {
    fn drop(&mut self) {
        self.dispose();
    }
}
impl vw_semantics::Cancellation for Cancellation {
    fn is_cancelled(&self) -> bool {
        Cancellation::is_cancelled(self)
    }
}
