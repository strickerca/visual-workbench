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
//   mirror also the replay of its queued work that an outbox refusal makes.
//   Every commit is admitted by it (directly or by
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
/// What a mirror allocates to take its queued work back, as the replay does
/// when the outbox refuses a new edit (`discard_pending`) or takes recovered
/// work (`queue_recovered_batch`), beside the replica's resident state:
///
/// * a copy of the authoritative project (the mirror store's project), which
///   grows by everything the queue creates: at most the queue's content;
/// * a copy of the queue itself;
/// * a new state hasher over the resulting visible project;
/// * the copy of this device's accepted undo previews the replay starts
///   from, and a new preview for every queued edit, of the same entities as
///   the preview held for it now; the held set stays alive until the new
///   one is installed.
///
/// The queue and the previews are counted on every call: they are this
/// device's own unsent and undoable work, not the project's history.
fn replay_bytes(state: &ProjectState) -> WorkflowResult<Option<u64>> {
    let Some(replica) = &state.replica else {
        return Ok(Some(0));
    };
    let store = state.store()?;
    let authoritative = project_cost(store.project(), store.state_hash()?)?;
    let visible = project_cost(replica.project(), replica.state_hash())?;
    let queue = measure(&Queue(replica.pending()))?;
    let previews = measure(&replica.undo_previews())?;
    let (Some(authoritative), Some(visible), Some(queue), Some(previews)) =
        (authoritative, visible, queue, previews)
    else {
        return Ok(None);
    };
    Ok(Some(
        authoritative
            .copy
            .saturating_add(queue.copy.saturating_mul(2))
            .saturating_add(visible.hasher())
            .saturating_add(previews.tree),
    ))
}
/// A mirror's queued transactions, serialized as a sequence.
struct Queue<'a>(&'a [vw_ops::PendingTransaction]);
impl serde::Serialize for Queue<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().map(|pending| &pending.transaction))
    }
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
/// what deciding it holds of the entities it touches, on a mirror the replay
/// of its queue, and in a build of vw-ops that cross-checks its hashes the
/// whole-project hash every decided edit then makes.
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
) -> WorkflowResult<[Option<u64>; 4]> {
    let single =
        vw_ops::HostSequencer::transaction_workspace_estimate_bytes(transaction, MAX_MEMORY / 16)
            .map_err(|_| WorkflowError::Invalid)?;
    let representations = (single <= MAX_MEMORY / 16).then(|| single * 16);
    Ok([
        representations,
        touched_bytes(state.project()?, visible_cost(state)?, transaction)?,
        replay_bytes(state)?,
        verification_bytes(state)?,
    ])
}
/// A build of vw-ops with `cross-check` (its own tests and the workspace's
/// test build; never the apps) checks what it does incrementally against
/// doing it whole. Every edit it decides, and the end of a mirror's replay,
/// also hashes the whole visible project (one hashed cost) while what
/// deciding holds is alive, and validates the whole project, which takes
/// less: references per entity where the hash holds a value per scalar.
/// Recording the edit for undo copies the undo history, which holds every
/// transaction of this device as its bytes, and orders it again. Nothing in
/// other builds, so this walk over the device's own history is made only
/// where those checks walk it too.
fn verification_bytes(state: &ProjectState) -> WorkflowResult<Option<u64>> {
    use vw_proto::Message;
    if !vw_ops::CROSS_CHECKED {
        return Ok(Some(0));
    }
    let store = state.store()?;
    let device = store.local_device()?;
    let own = store
        .accepted_transactions()
        .map(|(transaction, _)| transaction)
        .chain(
            state
                .replica
                .iter()
                .flat_map(|replica| replica.pending().iter().map(|pending| &pending.transaction)),
        )
        .filter(|transaction| transaction.device_id == device.as_str());
    // An event keeps the bytes, its ID, kind and receipt, in a map node.
    let undo = own.fold(0u64, |total, transaction| {
        total.saturating_add((transaction.encoded_len() as u64).saturating_add(1024))
    });
    Ok(visible_hash_bytes(state)?.map(|hash| hash.saturating_add(undo)))
}
/// The entities `transaction` changes or removes, as deciding it holds them
/// at once: the copy kept to restore each, the `Value`s before and after
/// that the journal and the conflict check keep, the removed entity's
/// journal value, and the entity as the state hasher writes it again (a
/// `Value` and its bytes). Four hashed costs per entity cover those; two
/// more of the largest cover the pair of values the journal diff converts
/// one entity at a time. Created entities are content of the transaction,
/// charged with it.
///
/// A change to any member other than `objects` also makes the state hasher
/// join that member's bytes and those in front of or behind `objects` again,
/// untouched entities included: five times the text of those members covers
/// the old and new bytes of both joins.
///
/// An undo touches what its target touched, entities it deleted included,
/// which only the history holds: it is refused here rather than charged
/// from the current project. No workflow plans one.
fn touched_bytes(
    project: &vw_model::Project,
    counted: Option<cost::JsonCost>,
    transaction: &pb::Transaction,
) -> WorkflowResult<Option<u64>> {
    use pb::op::Kind;
    use std::collections::BTreeSet;
    fn ids<'a>(values: impl IntoIterator<Item = &'a pb::Uuid>) -> impl Iterator<Item = Id> {
        values
            .into_iter()
            .filter_map(|value| Id::from_proto(Some(value)).ok())
    }
    // Entities the ops name, each once; entities a layer or group holds are
    // counted where they are found, without a list of them.
    let mut documents = BTreeSet::new();
    let mut layers = BTreeSet::new();
    let mut objects = BTreeSet::new();
    let mut groups = BTreeSet::new();
    let mut instructions = BTreeSet::new();
    let mut results = BTreeSet::new();
    let mut masks = BTreeSet::new();
    let mut members = false;
    let mut total = 0u64;
    let mut largest = 0u64;
    let mut add = |entity: Option<cost::JsonCost>| -> Option<()> {
        let hashed = entity?.hashed();
        total = total.saturating_add(hashed);
        largest = largest.max(hashed);
        Some(())
    };
    for op in &transaction.ops {
        match &op.kind {
            Some(Kind::UpdateDocument(value)) => documents.extend(ids(&value.document_id)),
            Some(Kind::UpdateLayer(value)) => layers.extend(ids(&value.layer_id)),
            Some(Kind::DeleteLayer(value)) => {
                layers.extend(ids(&value.layer_id));
                for object in project.objects.values() {
                    if object.state.layer_id == value.layer_id && add(measure(object)?).is_none() {
                        return Ok(None);
                    }
                }
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
                for object in project.objects.values() {
                    if object.state.group_id == value.group_id && add(measure(object)?).is_none() {
                        return Ok(None);
                    }
                }
                if let Some(group) = group {
                    for child in project.groups.values() {
                        if child.parent.as_ref() == Some(&group) && add(measure(child)?).is_none() {
                            return Ok(None);
                        }
                    }
                    groups.insert(group);
                }
            }
            Some(Kind::SetInstruction(value)) => instructions.extend(ids(&value.instruction_id)),
            Some(Kind::DeleteInstruction(value)) => {
                instructions.extend(ids(&value.instruction_id));
            }
            Some(Kind::UpdateResult(value)) => results.extend(ids(&value.result_id)),
            Some(Kind::MaskOp(value)) => masks.extend(ids(&value.output_object_id)),
            Some(Kind::UndoTransaction(_)) => return Ok(None),
            // New entities, or ops a host refuses before it touches anything.
            _ => {}
        }
        members |= !matches!(
            &op.kind,
            Some(
                Kind::CreateObject(_)
                    | Kind::SetProperty(_)
                    | Kind::DeleteObject(_)
                    | Kind::Reorder(_)
            )
        );
    }
    macro_rules! named {
        ($ids:ident, $collection:ident) => {
            for id in &$ids {
                if let Some(value) = project.$collection.get(id)
                    && add(measure(value)?).is_none()
                {
                    return Ok(None);
                }
            }
        };
    }
    named!(documents, documents);
    named!(layers, layers);
    named!(objects, objects);
    named!(groups, groups);
    named!(instructions, instructions);
    named!(results, results);
    named!(masks, mask_versions);
    let rejoined = if members {
        match counted {
            Some(counted) => counted.shell_text().saturating_mul(5),
            None => return Ok(None),
        }
    } else {
        0
    };
    let bytes = total
        .saturating_mul(4)
        .saturating_add(largest.saturating_mul(2))
        .saturating_add(rejoined);
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
/// which hashes the copy with a new state hasher and counts its checkpoint
/// length from a `Value` tree of it. The private host never replays a
/// mirror's queue.
pub(crate) fn admit_isolated_host(
    state: &ProjectState,
    transaction: &pb::Transaction,
    budget: u64,
    extra: u64,
) -> WorkflowResult<()> {
    admit_with(budget, extra, |left| {
        let [representations, touched, _, verification] = commit_parts(state, transaction)?;
        let host = visible_cost(state)?.map(|counted| {
            counted
                .copy
                .saturating_add(counted.hasher())
                .saturating_add(counted.tree)
        });
        charge(left, &[representations, touched, host, verification])
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
impl ProjectSession {
    /// What every commit is charged now for the whole-project checks of a
    /// build of vw-ops with `cross-check`; zero in other builds. For tests
    /// that tell a refusal those checks cause from any other. Not part of
    /// the bindings.
    #[doc(hidden)]
    pub async fn cross_check_charge(&self) -> WorkflowResult<u64> {
        self.worker
            .call(|state| Ok(verification_bytes(state).map(|bytes| bytes.unwrap_or(u64::MAX))))
            .await?
    }
}
impl WorkflowPlan {
    /// What `commit` would admit now beside the workflow's own work, for tests
    /// that measure a commit against it. Not part of the bindings.
    #[doc(hidden)]
    pub async fn commit_charge(&self) -> WorkflowResult<u64> {
        let prepared = self.prepared()?;
        self.worker
            .call(move |state| {
                Ok(
                    match commit_workspace(state, &prepared.transaction, MAX_MEMORY) {
                        Err(WorkflowError::Memory { estimated, .. }) => Ok(estimated),
                        other => other,
                    },
                )
            })
            .await?
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
