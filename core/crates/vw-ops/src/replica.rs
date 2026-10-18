use crate::{
    HostSnapshot, OpsError,
    patch::{self, Change},
    plan,
};
use std::collections::{BTreeMap, BTreeSet};
use vw_model::{DeviceId, Id, Project};
use vw_proto::{Message, v1};

// SYNC-002 requires recovery of 5,000 offline transactions. Keep the queue
// finite independently of the existing 4,096-operation per-transaction bound.
const MAX_PENDING_TRANSACTIONS: usize = 5_000;
// Match the durable store's 64 MiB encoded-record ceiling, and also bound the
// combined retained outbox and raw batch input before payload allocations.
// This is a wire-payload bound, not a replacement for caller workspace budgets.
const MAX_PENDING_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;

fn admit_payload_bytes(total: usize, transaction: &v1::Transaction) -> Result<usize, OpsError> {
    total
        .checked_add(transaction.encoded_len())
        .filter(|bytes| *bytes <= MAX_PENDING_PAYLOAD_BYTES)
        .ok_or(OpsError::Invalid("pending payload bounds"))
}

/// Pending work remains available after rebase rejection; it is never silently
/// discarded because an object was removed or a layer became locked remotely.
#[derive(Debug, Clone)]
pub struct PendingTransaction {
    pub transaction: v1::Transaction,
    pub blocked_reason: Option<String>,
}

/// An optimistic local view over the latest verified host snapshot.
#[derive(Debug, Clone)]
pub struct Replica {
    device: DeviceId,
    authoritative: Project,
    revision: v1::Revision,
    visible: Project,
    pending: Vec<PendingTransaction>,
    inverse_previews: BTreeMap<Id, Vec<Change>>,
    authoritative_inverses: BTreeMap<Id, Vec<Change>>,
}

impl Replica {
    /// Borrow and account all authoritative/visible, pending and inverse
    /// state without cloning it or exposing its retained journal contents.
    /// This is a one-representation charge; callers budget clone concurrency.
    pub fn workspace_estimate_bytes(&self, limit: u64) -> Result<u64, OpsError> {
        let mut estimate = crate::host::WorkspaceEstimate::new(limit);
        estimate.value(&(
            &self.device,
            &self.authoritative,
            &self.revision,
            &self.visible,
            &self.inverse_previews,
            &self.authoritative_inverses,
        ))?;
        for pending in &self.pending {
            estimate.value(&(&pending.transaction, &pending.blocked_reason))?;
            if estimate.finish() > limit {
                break;
            }
        }
        Ok(estimate.finish())
    }

    /// Start from a hash-verified snapshot; pending transactions are device-owned.
    pub fn new(device: DeviceId, snapshot: HostSnapshot) -> Result<Self, OpsError> {
        validate_snapshot(&snapshot)?;
        let inverse_previews = owned_previews(&snapshot, &device);
        Ok(Self {
            device,
            visible: snapshot.project.clone(),
            authoritative: snapshot.project,
            revision: snapshot.revision,
            pending: Vec::new(),
            authoritative_inverses: inverse_previews.clone(),
            inverse_previews,
        })
    }
    pub const fn project(&self) -> &Project {
        &self.visible
    }
    pub const fn revision(&self) -> &v1::Revision {
        &self.revision
    }
    pub fn pending(&self) -> &[PendingTransaction] {
        &self.pending
    }
    /// Restore a transaction from the durable local outbox after the verified
    /// host base advanced. Preserve its original bytes/base for exact retry and
    /// conflict semantics; failed optimistic replays remain visibly blocked.
    /// This is recovery, not admission of a newly authored stale-base edit.
    pub fn queue_recovered(&mut self, transaction: v1::Transaction) -> Result<(), OpsError> {
        self.queue_recovered_batch(vec![transaction])
    }
    /// Restore one bounded durable outbox in its original order. Validate the
    /// whole input before cloning/replaying the queue, then publish once. Exact
    /// duplicate identities are no-ops; any invalid/colliding item leaves all
    /// visible, pending and inverse state unchanged. A failed optimistic edit
    /// stays in the queue with its normal blocked reason, rather than failing
    /// recovery or being silently discarded. Both the raw input batch and the
    /// combined unique retained queue must fit 64 MiB of encoded transactions.
    /// Refusal leaves the original durable source owned by the caller untouched.
    pub fn queue_recovered_batch(
        &mut self,
        transactions: Vec<v1::Transaction>,
    ) -> Result<(), OpsError> {
        if transactions.len() > MAX_PENDING_TRANSACTIONS {
            return Err(OpsError::Invalid("pending queue bounds"));
        }
        // encoded_len borrows payloads; reject oversized input before even
        // duplicate equality allocates encodings. Count duplicate input bytes as
        // work, but only unique new transactions against the retained queue.
        transactions.iter().try_fold(0, admit_payload_bytes)?;
        if transactions.is_empty() {
            return Ok(());
        }
        let mut retained_bytes = self.pending.iter().try_fold(0, |bytes, pending| {
            admit_payload_bytes(bytes, &pending.transaction)
        })?;
        // This bounded identity index borrows input; it does not clone payloads
        // or grow the retained queue before every item has been admitted.
        let mut identities = BTreeMap::new();
        for pending in &self.pending {
            identities.insert(
                Id::from_proto(pending.transaction.txn_id.as_ref())?,
                &pending.transaction,
            );
        }
        let mut admitted = Vec::new();
        for (index, transaction) in transactions.iter().enumerate() {
            if transaction.device_id != self.device.as_str() {
                return Err(OpsError::DeviceMismatch);
            }
            if Id::from_proto(transaction.project_id.as_ref())? != self.authoritative.id {
                return Err(OpsError::Invalid("project identity"));
            }
            let base = transaction
                .base_revision
                .as_ref()
                .ok_or(OpsError::RevisionMismatch)?;
            if base.host_seq > self.revision.host_seq
                || base.state_hash.len() != 32
                || (base.host_seq == self.revision.host_seq && base != &self.revision)
            {
                return Err(OpsError::RevisionMismatch);
            }
            let id = Id::from_proto(transaction.txn_id.as_ref())?;
            if let Some(previous) = identities.get(&id) {
                if previous.encode_to_vec() != transaction.encode_to_vec() {
                    return Err(OpsError::IdCollision);
                }
                continue;
            }
            if self.authoritative_inverses.contains_key(&id) {
                return Err(OpsError::IdCollision);
            }
            if identities.len() >= MAX_PENDING_TRANSACTIONS
                || transaction.ops.is_empty()
                || transaction.ops.len() > 4096
            {
                return Err(OpsError::Invalid("pending queue bounds"));
            }
            retained_bytes = admit_payload_bytes(retained_bytes, transaction)?;
            validate_transaction(transaction)?;
            identities.insert(id, transaction);
            admitted.push(index);
        }
        drop(identities);
        if admitted.is_empty() {
            return Ok(());
        }
        let mut pending = self.pending.clone();
        pending.reserve(admitted.len());
        let mut admitted = admitted.into_iter();
        let mut next = admitted.next();
        for (index, transaction) in transactions.into_iter().enumerate() {
            if next == Some(index) {
                pending.push(PendingTransaction {
                    transaction,
                    blocked_reason: None,
                });
                next = admitted.next();
            }
        }
        let (visible, inverses) = replay(
            &self.authoritative,
            &mut pending,
            self.authoritative_inverses.clone(),
        )?;
        self.pending = pending;
        self.visible = visible;
        self.inverse_previews = inverses;
        Ok(())
    }
    /// Explicitly abandon rejected/cancelled local work and its inverse chain.
    /// Other dependent edits remain queued with a blocked reason after replay.
    /// This never cancels work already accepted by the host.
    pub fn discard_pending(&mut self, id: &Id) -> Result<Vec<Id>, OpsError> {
        if !self
            .pending
            .iter()
            .any(|p| p.transaction.txn_id.as_ref() == Some(&id.to_proto()))
        {
            return Err(OpsError::NotFound);
        }
        let mut removed = BTreeSet::from([id.clone()]);
        loop {
            let count = removed.len();
            for pending in &self.pending {
                if let [
                    v1::Op {
                        kind: Some(v1::op::Kind::UndoTransaction(value)),
                        ..
                    },
                ] = pending.transaction.ops.as_slice()
                    && removed.contains(&Id::from_proto(value.target_txn_id.as_ref())?)
                {
                    removed.insert(Id::from_proto(pending.transaction.txn_id.as_ref())?);
                }
            }
            if count == removed.len() {
                break;
            }
        }
        let mut remaining = self
            .pending
            .iter()
            .filter(|p| {
                !removed
                    .iter()
                    .any(|id| p.transaction.txn_id.as_ref() == Some(&id.to_proto()))
            })
            .cloned()
            .collect::<Vec<_>>();
        let (visible, inverse_previews) = replay(
            &self.authoritative,
            &mut remaining,
            self.authoritative_inverses.clone(),
        )?;
        self.pending = remaining;
        self.visible = visible;
        self.inverse_previews = inverse_previews;
        Ok(removed.into_iter().collect())
    }
    /// Apply one local transaction optimistically. Its base is the last host
    /// revision, not a fabricated revision for earlier unacknowledged local edits.
    pub fn queue(&mut self, transaction: v1::Transaction) -> Result<(), OpsError> {
        if transaction.device_id != self.device.as_str() {
            return Err(OpsError::DeviceMismatch);
        }
        if transaction.base_revision.as_ref() != Some(&self.revision) {
            return Err(OpsError::RevisionMismatch);
        }
        if Id::from_proto(transaction.project_id.as_ref())? != self.authoritative.id {
            return Err(OpsError::Invalid("project identity"));
        }
        admit_payload_bytes(0, &transaction)?;
        let id = Id::from_proto(transaction.txn_id.as_ref())?;
        if let Some(existing) = self
            .pending
            .iter()
            .find(|p| p.transaction.txn_id == transaction.txn_id)
        {
            return if existing.transaction.encode_to_vec() == transaction.encode_to_vec() {
                Ok(())
            } else {
                Err(OpsError::IdCollision)
            };
        }
        if self.pending.len() >= MAX_PENDING_TRANSACTIONS
            || transaction.ops.is_empty()
            || transaction.ops.len() > 4096
        {
            return Err(OpsError::Invalid("pending queue bounds"));
        }
        self.pending
            .iter()
            .try_fold(transaction.encoded_len(), |bytes, pending| {
                admit_payload_bytes(bytes, &pending.transaction)
            })?;
        validate_transaction(&transaction)?;
        let project = optimistic(&self.visible, &transaction, &self.inverse_previews)?;
        let inverse = patch::between(&self.visible, &project)?
            .into_iter()
            .rev()
            .map(|change| change.inverse())
            .collect();
        self.inverse_previews.insert(id, inverse);
        self.visible = project;
        self.pending.push(PendingTransaction {
            transaction,
            blocked_reason: None,
        });
        Ok(())
    }
    /// Replace the authoritative base on an acknowledgement or hash mismatch,
    /// remove only byte-matched accepted work, then replay the remaining queue.
    /// Failures remain blocked in that queue with a notice for the caller.
    pub fn receive(&mut self, snapshot: HostSnapshot) -> Result<(), OpsError> {
        validate_snapshot(&snapshot)?;
        if snapshot.project.id != self.authoritative.id
            || snapshot.revision.host_seq < self.revision.host_seq
        {
            return Err(OpsError::RevisionMismatch);
        }
        let mut remaining = Vec::new();
        for pending in &self.pending {
            let id = Id::from_proto(pending.transaction.txn_id.as_ref())?;
            if let Some((accepted, ack)) = snapshot.accepted.get(&id) {
                if accepted.encode_to_vec() != pending.transaction.encode_to_vec()
                    || ack.txn_id != pending.transaction.txn_id
                {
                    return Err(OpsError::IdCollision);
                }
            } else {
                remaining.push(pending.clone());
            }
        }
        let authoritative_inverses = owned_previews(&snapshot, &self.device);
        let (visible, inverse_previews) = replay(
            &snapshot.project,
            &mut remaining,
            authoritative_inverses.clone(),
        )?;
        self.authoritative = snapshot.project;
        self.revision = snapshot.revision;
        self.visible = visible;
        self.pending = remaining;
        self.inverse_previews = inverse_previews;
        self.authoritative_inverses = authoritative_inverses;
        Ok(())
    }
}

fn replay(
    base: &Project,
    pending: &mut [PendingTransaction],
    mut inverses: BTreeMap<Id, Vec<Change>>,
) -> Result<(Project, BTreeMap<Id, Vec<Change>>), OpsError> {
    let mut visible = base.clone();
    for pending in pending {
        match optimistic(&visible, &pending.transaction, &inverses) {
            Ok(next) => {
                let inverse = patch::between(&visible, &next)?
                    .into_iter()
                    .rev()
                    .map(|change| change.inverse())
                    .collect();
                inverses.insert(
                    Id::from_proto(pending.transaction.txn_id.as_ref())?,
                    inverse,
                );
                visible = next;
                pending.blocked_reason = None;
            }
            Err(error) => pending.blocked_reason = Some(error.to_string()),
        }
    }
    Ok((visible, inverses))
}

fn owned_previews(snapshot: &HostSnapshot, device: &DeviceId) -> BTreeMap<Id, Vec<Change>> {
    snapshot
        .inverse_previews
        .iter()
        .filter(|(id, _)| {
            snapshot
                .accepted
                .get(*id)
                .is_some_and(|(txn, _)| txn.device_id == device.as_str())
        })
        .map(|(id, changes)| (id.clone(), changes.clone()))
        .collect()
}

fn optimistic(
    project: &Project,
    transaction: &v1::Transaction,
    inverses: &BTreeMap<Id, Vec<Change>>,
) -> Result<Project, OpsError> {
    if let [
        v1::Op {
            kind: Some(v1::op::Kind::UndoTransaction(undo)),
            ..
        },
    ] = transaction.ops.as_slice()
    {
        let id = Id::from_proto(undo.target_txn_id.as_ref())?;
        let changes = inverses.get(&id).ok_or(OpsError::UndoOwnership)?;
        patch::apply(project, changes)
    } else {
        plan::plan(project, transaction)
    }
}

fn validate_snapshot(snapshot: &HostSnapshot) -> Result<(), OpsError> {
    if snapshot.project.state_hash()?.bytes().as_slice() != snapshot.revision.state_hash {
        return Err(OpsError::RevisionMismatch);
    }
    let mut sequences = BTreeSet::new();
    for (id, (txn, ack)) in &snapshot.accepted {
        if Id::from_proto(txn.txn_id.as_ref())? != *id
            || ack.txn_id != txn.txn_id
            || ack.host_seq > snapshot.revision.host_seq
            || ack.state_hash.len() != 32
            || ack.host_seq == 0
            || !sequences.insert(ack.host_seq)
            || Id::from_proto(txn.project_id.as_ref())? != snapshot.project.id
        {
            return Err(OpsError::RevisionMismatch);
        }
    }
    Ok(())
}

fn is_inverse(transaction: &v1::Transaction) -> bool {
    matches!(
        transaction.ops.as_slice(),
        [v1::Op {
            kind: Some(v1::op::Kind::UndoTransaction(_)),
            ..
        }]
    )
}

fn validate_transaction(transaction: &v1::Transaction) -> Result<(), OpsError> {
    if transaction.created_at_wall_ms < 0 {
        return Err(OpsError::Invalid("transaction time"));
    }
    if let Some(gesture) = &transaction.gesture_id {
        Id::from_proto(Some(gesture))?;
    }
    let mut counters = BTreeSet::new();
    for op in &transaction.ops {
        let id = op
            .op_id
            .as_ref()
            .ok_or(OpsError::Invalid("operation identity"))?;
        if id.device_id != transaction.device_id || id.lamport == 0 || !counters.insert(id.lamport)
        {
            return Err(OpsError::Invalid("operation identity/counter"));
        }
        if let Some(v1::op::Kind::UndoTransaction(undo)) = &op.kind {
            if !is_inverse(transaction) {
                return Err(OpsError::Invalid("standalone inverse required"));
            }
            Id::from_proto(undo.target_txn_id.as_ref())?;
        }
    }
    Ok(())
}
