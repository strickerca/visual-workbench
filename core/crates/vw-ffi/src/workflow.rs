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
mod cost;

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

/// Kept out of every workflow budget for fixed state and small allocations.
pub(crate) const RESERVE: u64 = 4 * 1024 * 1024;

// Resident state is not charged again. What a workflow adds is charged by
// what it does with the canonical state, each part from the data it touches:
//
// * `canonical_workspace`: the workflow may hash or serialize the whole
//   visible project once (`DocumentView`, semantic bindings and published
//   giant images call `Project::state_hash`): one `Value` tree of the visible
//   project and its bytes. Never the history, which no such workflow reads.
// * `borrowed_workspace`: the workflow only reads the project in place.
// * `commit_workspace`: deciding a transaction in place, as validation and
//   commit both do: its own representations, and the entities it changes or
//   removes, which are copied, journaled as `Value`s and hashed again; on a
//   mirror also the copy of the authoritative project that taking a queued
//   edit back replays from. Every commit is admitted by it (directly or by
//   `admit_transaction`) after any hashing of the same workflow has ended,
//   so a mutation's earlier admission charges nothing more than a read.
//
// This used to charge four (for a mutation sixteen) conservative copies of
// the store, its entire history and the replica, found by serializing all of
// them on every call: a page of a hundred pen strokes passed every budget,
// and the estimate took as long as the history was large.

/// Costs of projects by their state hash. A project is counted once per
/// state, and a state hash identifies the content exactly, so an entry never
/// goes stale. A project is counted to the end: even one too large to hash
/// within any budget may have a copy that fits one.
type Counted = (StateHash, Option<cost::JsonCost>);
static PROJECT_COSTS: Mutex<Vec<Counted>> = Mutex::new(Vec::new());
const PROJECT_COST_ENTRIES: usize = 8;

fn measure<T: serde::Serialize + ?Sized>(value: &T) -> WorkflowResult<Option<cost::JsonCost>> {
    cost::measure(value, MAX_MEMORY).map_err(|_| WorkflowError::Invalid)
}
/// The cost of `project`, whose state hash is `hash`: one walk over the
/// project (not its history) the first time a state is asked for.
fn project_cost(
    project: &vw_model::Project,
    hash: &StateHash,
) -> WorkflowResult<Option<cost::JsonCost>> {
    if let Ok(costs) = PROJECT_COSTS.lock()
        && let Some((_, known)) = costs.iter().find(|(key, _)| key == hash)
    {
        return Ok(*known);
    }
    let counted = cost::measure(project, u64::MAX).map_err(|_| WorkflowError::Invalid)?;
    if let Ok(mut costs) = PROJECT_COSTS.lock() {
        if costs.len() >= PROJECT_COST_ENTRIES {
            costs.remove(0);
        }
        costs.push((hash.clone(), counted));
    }
    Ok(counted)
}
/// The cost of the visible project: with a mirror's pending edits.
fn visible_cost(state: &ProjectState) -> WorkflowResult<Option<cost::JsonCost>> {
    match &state.replica {
        Some(replica) => project_cost(replica.project(), replica.state_hash()),
        None => {
            let store = state.store()?;
            project_cost(store.project(), store.state_hash()?)
        }
    }
}
/// Hashing the visible project once: `Project::state_hash`.
fn visible_hash_bytes(state: &ProjectState) -> WorkflowResult<Option<u64>> {
    Ok(visible_cost(state)?.map(cost::JsonCost::hashed))
}
/// What a mirror allocates to take a queued edit back when its outbox refuses
/// it: a copy of the authoritative project (the mirror store's project), and
/// a fresh state hasher over the copy, whose pages of canonical bytes grow by
/// doubling.
///
/// Not counted: the copy of the undo previews of this device's own accepted
/// edits that the same replay makes. Their size is private to
/// `vw_ops::Replica`; it is about the `Value` tree of what this device drew,
/// more after many of its own deletions or undos.
fn replay_bytes(state: &ProjectState) -> WorkflowResult<Option<u64>> {
    if state.replica.is_none() {
        return Ok(Some(0));
    }
    let store = state.store()?;
    Ok(project_cost(store.project(), store.state_hash()?)?
        .map(|counted| counted.copy.saturating_add(counted.text.saturating_mul(2))))
}
/// The reserve and `parts`, refused once they pass `budget`. A part that is
/// `None` was past `MAX_MEMORY` and passes any budget.
fn charge(budget: u64, parts: &[Option<u64>]) -> WorkflowResult<u64> {
    if budget == 0 || budget > MAX_MEMORY {
        return Err(WorkflowError::Invalid);
    }
    let estimated = parts
        .iter()
        .try_fold(RESERVE, |total, part| total.checked_add((*part)?));
    match estimated {
        Some(estimated) if estimated <= budget => Ok(estimated),
        Some(estimated) => Err(WorkflowError::Memory { estimated, budget }),
        None => Err(WorkflowError::Memory {
            estimated: MAX_MEMORY + 1,
            budget,
        }),
    }
}
/// A workflow that may hash or serialize the whole visible project once.
/// `_mutation` charges nothing more: a commit is admitted by
/// `commit_workspace` once the transaction exists.
pub(crate) fn canonical_workspace(
    state: &ProjectState,
    budget: u64,
    _mutation: bool,
) -> WorkflowResult<u64> {
    charge(budget, &[visible_hash_bytes(state)?])
}
/// A workflow that only reads the canonical state in place: the reserve.
pub(crate) fn borrowed_workspace(budget: u64) -> WorkflowResult<u64> {
    charge(budget, &[])
}
/// Copying the whole host, history included, through its checkpoint, as
/// compiling a verification package does: the store writes a `Value` tree of
/// the host and its bytes, the host is read back from them and written once
/// more to check they are canonical, and the copy stays alive while its
/// project is hashed. The store's conservative serialization charge bounds
/// one in-memory copy, or one tree with its bytes; twice it covers each step.
/// This walks the history, as the work it admits does.
pub(crate) fn checkpoint_workspace(state: &ProjectState, budget: u64) -> WorkflowResult<u64> {
    if budget == 0 || budget > MAX_MEMORY {
        return Err(WorkflowError::Invalid);
    }
    let limit = budget.saturating_sub(RESERVE) / 2;
    let host = state.store()?.workspace_estimate_bytes(limit)?;
    charge(budget, &[(host <= limit).then(|| host * 2)])
}
/// Deciding `transaction` in place, by validation or commit: sixteen
/// borrowed representations of the transaction (its new content included),
/// what deciding it holds of the entities it touches, and on a mirror the
/// replay copy.
pub(crate) fn commit_workspace(
    state: &ProjectState,
    transaction: &pb::Transaction,
    budget: u64,
) -> WorkflowResult<u64> {
    charge(budget, &commit_parts(state, transaction)?)
}
fn commit_parts(
    state: &ProjectState,
    transaction: &pb::Transaction,
) -> WorkflowResult<[Option<u64>; 3]> {
    let single =
        vw_ops::HostSequencer::transaction_workspace_estimate_bytes(transaction, MAX_MEMORY / 16)
            .map_err(|_| WorkflowError::Invalid)?;
    let representations = (single <= MAX_MEMORY / 16).then(|| single * 16);
    Ok([
        representations,
        touched_bytes(state.project()?, transaction)?,
        replay_bytes(state)?,
    ])
}
/// The entities `transaction` changes or removes, as deciding it holds them
/// at once: the copy kept to restore each, the `Value`s before and after
/// that the journal and the conflict check keep, the removed entity's
/// journal value, and its bytes written again into the state hasher. Four
/// hashed costs per entity cover those; two more of the largest cover the
/// pair of values the journal diff converts one entity at a time. Created
/// entities are content of the transaction, charged with it. An undo touches
/// what its target touched, which only the history knows: the whole project
/// stands in for it.
fn touched_bytes(
    project: &vw_model::Project,
    transaction: &pb::Transaction,
) -> WorkflowResult<Option<u64>> {
    use pb::op::Kind;
    use std::collections::BTreeSet;
    fn ids<'a>(values: impl IntoIterator<Item = &'a pb::Uuid>) -> impl Iterator<Item = Id> {
        values
            .into_iter()
            .filter_map(|value| Id::from_proto(Some(value)).ok())
    }
    let mut documents = BTreeSet::new();
    let mut layers = BTreeSet::new();
    let mut objects = BTreeSet::new();
    let mut groups = BTreeSet::new();
    let mut instructions = BTreeSet::new();
    let mut results = BTreeSet::new();
    let mut masks = BTreeSet::new();
    let mut undo = false;
    for op in &transaction.ops {
        match &op.kind {
            Some(Kind::UpdateDocument(value)) => documents.extend(ids(&value.document_id)),
            Some(Kind::UpdateLayer(value)) => layers.extend(ids(&value.layer_id)),
            Some(Kind::DeleteLayer(value)) => {
                layers.extend(ids(&value.layer_id));
                objects.extend(
                    project
                        .objects
                        .iter()
                        .filter(|(_, object)| object.state.layer_id == value.layer_id)
                        .map(|(id, _)| id.clone()),
                );
            }
            Some(Kind::SetProperty(value)) => objects.extend(ids(&value.object_id)),
            Some(Kind::DeleteObject(value)) => objects.extend(ids(&value.object_id)),
            Some(Kind::Reorder(value)) => objects.extend(ids(&value.object_id)),
            Some(Kind::Group(value)) => {
                groups.extend(ids(&value.group_id));
                for member in ids(&value.object_ids) {
                    objects.insert(member.clone());
                    groups.insert(member);
                }
            }
            Some(Kind::Ungroup(value)) => {
                let group = ids(&value.group_id).next();
                objects.extend(
                    project
                        .objects
                        .iter()
                        .filter(|(_, object)| object.state.group_id == value.group_id)
                        .map(|(id, _)| id.clone()),
                );
                if let Some(group) = group {
                    groups.extend(
                        project
                            .groups
                            .iter()
                            .filter(|(_, child)| child.parent.as_ref() == Some(&group))
                            .map(|(id, _)| id.clone()),
                    );
                    groups.insert(group);
                }
            }
            Some(Kind::SetInstruction(value)) => instructions.extend(ids(&value.instruction_id)),
            Some(Kind::DeleteInstruction(value)) => {
                instructions.extend(ids(&value.instruction_id));
            }
            Some(Kind::UpdateResult(value)) => results.extend(ids(&value.result_id)),
            Some(Kind::MaskOp(value)) => masks.extend(ids(&value.output_object_id)),
            Some(Kind::UndoTransaction(_)) => undo = true,
            // New entities, or ops a host refuses before it touches anything.
            _ => {}
        }
    }
    let mut counted = Vec::new();
    if undo {
        counted.push(measure(project)?);
    }
    for id in &documents {
        counted.extend(project.documents.get(id).map(measure).transpose()?);
    }
    for id in &layers {
        counted.extend(project.layers.get(id).map(measure).transpose()?);
    }
    for id in &objects {
        counted.extend(project.objects.get(id).map(measure).transpose()?);
    }
    for id in &groups {
        counted.extend(project.groups.get(id).map(measure).transpose()?);
    }
    for id in &instructions {
        counted.extend(project.instructions.get(id).map(measure).transpose()?);
    }
    for id in &results {
        counted.extend(project.results.get(id).map(measure).transpose()?);
    }
    for id in &masks {
        counted.extend(project.mask_versions.get(id).map(measure).transpose()?);
    }
    let mut total = 0u64;
    let mut largest = 0u64;
    for entity in counted {
        let Some(entity) = entity else {
            return Ok(None);
        };
        total = total.saturating_add(entity.hashed());
        largest = largest.max(entity.hashed());
    }
    let bytes = total
        .saturating_mul(4)
        .saturating_add(largest.saturating_mul(2));
    Ok((bytes <= MAX_MEMORY).then_some(bytes))
}
/// Admit `workspace` (counted within what `extra` leaves of `budget`) and
/// the workflow's own `extra` together.
fn admit_with(
    budget: u64,
    extra: u64,
    workspace: impl FnOnce(u64) -> WorkflowResult<u64>,
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
    let canonical = match workspace(budget - extra) {
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
/// `canonical_workspace` and the workflow's own `extra`.
pub(crate) fn admit(
    state: &ProjectState,
    budget: u64,
    mutation: bool,
    extra: u64,
) -> WorkflowResult<()> {
    admit_with(budget, extra, |left| {
        canonical_workspace(state, left, mutation)
    })
}
/// `borrowed_workspace` and the workflow's own `extra`.
pub(crate) fn admit_borrowed(budget: u64, extra: u64) -> WorkflowResult<()> {
    admit_with(budget, extra, borrowed_workspace)
}
/// Newly planned transaction content is not resident in the admitted store.
/// Charge it and what deciding it touches before validating or committing it.
pub(crate) fn admit_transaction(
    state: &ProjectState,
    transaction: &pb::Transaction,
    budget: u64,
    extra: u64,
) -> WorkflowResult<()> {
    admit_with(budget, extra, |left| {
        commit_workspace(state, transaction, left)
    })
}
/// A preview decided on a private host built from a copy of the visible
/// project: the transaction and what it touches, the copy, and the new host,
/// which writes the project into its state hasher and counts its checkpoint
/// length from a `Value` tree of it (together less than one hashed cost).
/// The private host never replays a mirror's queue.
pub(crate) fn admit_isolated_host(
    state: &ProjectState,
    transaction: &pb::Transaction,
    budget: u64,
    extra: u64,
) -> WorkflowResult<()> {
    admit_with(budget, extra, |left| {
        let [representations, touched, _] = commit_parts(state, transaction)?;
        let host =
            visible_cost(state)?.and_then(|counted| counted.copy.checked_add(counted.hashed()));
        charge(left, &[representations, touched, host])
    })
}
pub(crate) fn current(state: &ProjectState, document: &str) -> WorkflowResult<WorkflowBinding> {
    let id = Id::try_from(document.to_owned())?;
    if !state.project()?.documents.contains_key(&id) {
        return Err(WorkflowError::Missing);
    }
    let revision = state.view_revision()?;
    // The hash of what is shown is carried forward by every edit; hashing the
    // whole project here would take as long as the project is large, three
    // times for every edit a workflow makes.
    let state_hash = match &state.replica {
        Some(replica) => replica.state_hash().to_string(),
        None => state.store()?.state_hash()?.to_string(),
    };
    Ok(WorkflowBinding {
        project_id: state.project()?.id.to_string(),
        document_id: id.to_string(),
        host_seq: revision.host_seq,
        state_hash,
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
