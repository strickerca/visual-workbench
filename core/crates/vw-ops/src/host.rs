use crate::{
    OpsError,
    draft::{self, Draft, Originals, Touched},
    patch::{self, Address, Change, Values},
    plan,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Bound,
};
use vw_model::{AssetId, DeviceId, Id, Object, Project, StateHash, StateHasher};
use vw_proto::{
    Message,
    v1::{self, op::Kind},
};

const CHECKPOINT_PREFIX: &[u8] = b"VisualWorkbench.HostCheckpoint.v1\n";
const MAX_CHECKPOINT_BYTES: usize = 256 * 1024 * 1024;

/// Internal admission meter, NOT a journal export API. serde streams borrowed
/// fields through a fixed-size lexical state; only bounded ID serialization
/// temporaries are created. Each JSON byte costs four bytes, and each string,
/// key, array/object or scalar costs 256 bytes for heap/value/collection slack.
/// Callers separately multiply this one-representation charge by the maximum
/// simultaneously materialized stages in their operation. In particular a
/// million tiny numbers cannot hide behind a small JSON-byte count.
pub(crate) struct WorkspaceEstimate {
    amount: u64,
    limit: u64,
    exceeded: bool,
    quoted: bool,
    escaped: bool,
    scalar: bool,
}
impl WorkspaceEstimate {
    pub(crate) fn new(limit: u64) -> Self {
        Self {
            amount: 4096,
            limit,
            exceeded: limit < 4096,
            quoted: false,
            escaped: false,
            scalar: false,
        }
    }
    pub(crate) fn value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), OpsError> {
        if self.exceeded {
            return Ok(());
        }
        self.scalar = false;
        match serde_json::to_writer(&mut *self, value) {
            Ok(()) => Ok(()),
            Err(_) if self.exceeded => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
    pub(crate) fn finish(&self) -> u64 {
        if self.exceeded {
            self.limit.saturating_add(1)
        } else {
            self.amount
        }
    }
    fn charge(&mut self, amount: u64) -> std::io::Result<()> {
        self.amount = self.amount.saturating_add(amount);
        if self.amount > self.limit {
            self.exceeded = true;
            return Err(std::io::Error::other("workspace admission bound"));
        }
        Ok(())
    }
}
impl std::io::Write for WorkspaceEstimate {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.charge((bytes.len() as u64).saturating_mul(4))?;
        for &byte in bytes {
            if self.quoted {
                if self.escaped {
                    self.escaped = false;
                } else if byte == b'\\' {
                    self.escaped = true;
                } else if byte == b'"' {
                    self.quoted = false;
                }
            } else {
                match byte {
                    b'"' => {
                        self.charge(256)?;
                        self.quoted = true;
                        self.scalar = false;
                    }
                    b'[' | b'{' => {
                        self.charge(256)?;
                        self.scalar = false;
                    }
                    b']' | b'}' | b',' | b':' | b' ' | b'\n' | b'\r' | b'\t' => {
                        self.scalar = false;
                    }
                    _ if !self.scalar => {
                        self.charge(256)?;
                        self.scalar = true;
                    }
                    _ => {}
                }
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteStamp {
    address: Address,
    device: DeviceId,
    sequence: u64,
    wall_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    transaction: v1::Transaction,
    ack: v1::TxnAck,
    changes: Vec<Change>,
    accepted_at_ms: i64,
    conflicts: Vec<v1::ConflictRecord>,
}

/// Result of an accepted transaction. A retry returns the original ack and no
/// duplicate conflict events. Persist the host checkpoint atomically with its log.
#[derive(Debug, Clone)]
pub struct Acceptance {
    pub ack: v1::TxnAck,
    pub conflicts: Vec<v1::ConflictRecord>,
    pub duplicate: bool,
}

/// Verified full-state rebase input. Transport delta/snapshot framing is T1.06a.
#[derive(Debug, Clone)]
pub struct HostSnapshot {
    pub project: Project,
    pub revision: v1::Revision,
    /// Transaction bytes bind acknowledgements to the client's exact pending work.
    pub accepted: BTreeMap<Id, (v1::Transaction, v1::TxnAck)>,
    /// Internal previews derived from trusted accepted journals, never wire patches.
    pub(crate) inverse_previews: BTreeMap<Id, Vec<Change>>,
}

/// Host-ordered in-memory engine. A submission edits the project in place and
/// is recorded only after validation; a rejection puts back every entity it
/// touched, so it cannot partly mutate state. The work per submission follows
/// what the transaction changes, not the size of the project or its history.
#[derive(Debug, Clone, Serialize)]
pub struct HostSequencer {
    host_device: DeviceId,
    project: Project,
    sequence: u64,
    revisions: BTreeMap<u64, StateHash>,
    records: BTreeMap<Id, Record>,
    writes: BTreeMap<String, WriteStamp>,
    tombstones: BTreeMap<Id, Object>,
    seen_ops: BTreeSet<(DeviceId, u64)>,
    gestures: BTreeSet<(DeviceId, Id)>,
    cancelled: BTreeSet<(DeviceId, Id)>,
    conflicts: Vec<v1::ConflictRecord>,
    /// Derived from `project`; never part of a checkpoint.
    #[serde(skip)]
    hasher: StateHasher,
    #[serde(skip)]
    canonical_len: usize,
}

// A private remote deserializer keeps callers from bypassing the validated
// checkpoint entry point via serde_json::from_slice::<HostSequencer>.
#[derive(Deserialize)]
#[serde(remote = "HostSequencer", deny_unknown_fields)]
struct CheckpointState {
    host_device: DeviceId,
    project: Project,
    sequence: u64,
    revisions: BTreeMap<u64, StateHash>,
    records: BTreeMap<Id, Record>,
    writes: BTreeMap<String, WriteStamp>,
    tombstones: BTreeMap<Id, Object>,
    seen_ops: BTreeSet<(DeviceId, u64)>,
    gestures: BTreeSet<(DeviceId, Id)>,
    cancelled: BTreeSet<(DeviceId, Id)>,
    conflicts: Vec<v1::ConflictRecord>,
    #[serde(skip)]
    hasher: StateHasher,
    #[serde(skip)]
    canonical_len: usize,
}

/// What deciding a transaction reads but never changes.
struct View<'a> {
    host_device: &'a DeviceId,
    sequence: u64,
    revisions: &'a BTreeMap<u64, StateHash>,
    records: &'a BTreeMap<Id, Record>,
    writes: &'a BTreeMap<String, WriteStamp>,
    tombstones: &'a BTreeMap<Id, Object>,
    seen_ops: &'a BTreeSet<(DeviceId, u64)>,
    gestures: &'a BTreeSet<(DeviceId, Id)>,
    cancelled: &'a BTreeSet<(DeviceId, Id)>,
}

/// A transaction the host accepts: the project already holds its result, and
/// this is everything still to be recorded.
struct Decision {
    id: Id,
    device: DeviceId,
    sequence: u64,
    hash: StateHash,
    canonical_len: usize,
    record: Record,
    originals: Originals,
    operations: Vec<(DeviceId, u64)>,
    gesture: Option<(DeviceId, Id)>,
    /// Object tombstones to set (`Some`) or clear (`None`), in journal order.
    tombstones: Vec<(Id, Option<Object>)>,
}

/// How to take one recorded transaction back out again.
struct Undo {
    id: Id,
    sequence: u64,
    canonical_len: usize,
    conflicts: usize,
    originals: Originals,
    operations: Vec<(DeviceId, u64)>,
    gesture: Option<(DeviceId, Id)>,
    tombstones: Vec<(Id, Option<Object>)>,
    writes: Vec<(String, Option<WriteStamp>)>,
}

/// Puts a recorded transaction back unless its persistence succeeded, also
/// when the persisting code unwinds.
struct Recorded<'a> {
    host: &'a mut HostSequencer,
    undo: Option<Undo>,
}
impl Drop for Recorded<'_> {
    fn drop(&mut self) {
        if let Some(undo) = self.undo.take() {
            self.host.revert(undo);
        }
    }
}

enum Admission {
    Duplicate(Acceptance),
    New(DeviceId, Id),
}

impl HostSequencer {
    /// Conservative single-representation workspace charge for admission by
    /// callers which clone/hash this state. Includes private journal values,
    /// tombstones, conflicts and collection overhead, not only visible pixels.
    /// Stops at limit and returns limit+1 without exporting or retaining JSON.
    pub fn workspace_estimate_bytes(&self, limit: u64) -> Result<u64, OpsError> {
        let mut estimate = WorkspaceEstimate::new(limit);
        estimate.value(self)?;
        Ok(estimate.finish())
    }

    /// The authoritative host identity persisted with this operation history.
    pub const fn host_device(&self) -> &DeviceId {
        &self.host_device
    }

    /// Borrowed admission for newly authored transactions, before speculative
    /// host/replica/history cloning. No JSON or checkpoint buffer is retained.
    /// Uses the same node/byte accounting as canonical workspace admission.
    pub fn transaction_workspace_estimate_bytes(
        transaction: &v1::Transaction,
        limit: u64,
    ) -> Result<u64, OpsError> {
        let mut estimate = WorkspaceEstimate::new(limit);
        estimate.value(transaction)?;
        Ok(estimate.finish())
    }

    /// An authenticated replacement checkpoint must retain every durable
    /// cancellation guard, including guards that do not change the revision.
    pub fn cancellations_are_subset_of(&self, newer: &Self) -> bool {
        self.cancelled.is_subset(&newer.cancelled)
    }

    /// Every durable gesture cancellation. They are the one part of this state
    /// the accepted journal cannot rebuild, so storage keeps them with the
    /// checkpoint its journal replays from.
    pub fn cancelled_gestures(&self) -> impl Iterator<Item = (&DeviceId, &Id)> {
        self.cancelled
            .iter()
            .map(|(device, gesture)| (device, gesture))
    }

    /// Whether `other` holds the same accepted history: the project, every
    /// revision, every accepted transaction with its acknowledgement, journal
    /// and conflict records, and the indexes kept over them. Cancellations
    /// are not history and are not compared. Storage uses this to tell that
    /// a state it decided again from the log is the one a checkpoint holds.
    ///
    /// The journal and the indexes are compared as a checkpoint would write
    /// them: a state read from a checkpoint and the same state as the engine
    /// left it differ in memory where the written form does not, for one in
    /// an absent value that the engine holds as a null.
    pub fn same_history(&self, other: &Self) -> bool {
        fn written<T: Serialize>(left: &T, right: &T) -> bool {
            matches!(
                (serde_json::to_vec(left), serde_json::to_vec(right)),
                (Ok(left), Ok(right)) if left == right
            )
        }
        self.host_device == other.host_device
            && self.sequence == other.sequence
            && self.revisions == other.revisions
            && self.seen_ops == other.seen_ops
            && self.gestures == other.gestures
            && self.project == other.project
            && self.records.len() == other.records.len()
            && self
                .records
                .iter()
                .zip(&other.records)
                .all(|(left, right)| left.0 == right.0 && written(left.1, right.1))
            && written(&self.writes, &other.writes)
            && written(&self.tombstones, &other.tombstones)
            && written(&self.conflicts, &other.conflicts)
    }

    /// Exact accepted transaction/acknowledgement binding for storage audits,
    /// without computing optimistic inverse previews for the entire history.
    pub fn accepted_transaction(&self, id: &Id) -> Option<(&v1::Transaction, &v1::TxnAck)> {
        self.records
            .get(id)
            .map(|record| (&record.transaction, &record.ack))
    }

    /// Original host receipt time, retained so persisted log metadata can be
    /// checked against the checkpoint without reapplying its transactions.
    pub fn accepted_at(&self, id: &Id) -> Option<i64> {
        self.records.get(id).map(|record| record.accepted_at_ms)
    }

    /// Borrow accepted journal bindings without deriving inverse previews.
    /// Entries are keyed by transaction ID; consumers needing host order sort
    /// by the acknowledgement's sequence.
    pub fn accepted_transactions(&self) -> impl Iterator<Item = (&v1::Transaction, &v1::TxnAck)> {
        self.records
            .values()
            .map(|record| (&record.transaction, &record.ack))
    }

    /// Persist a versioned, checksummed checkpoint including retry, conflict,
    /// undo and gesture guards. This contains project data; never log its bytes.
    /// The storage layer must commit it atomically with its matching revision.
    pub fn checkpoint_bytes(&self) -> Result<Vec<u8>, OpsError> {
        let payload = serde_json::to_vec(&serde_json::to_value(self)?)?;
        let length = CHECKPOINT_PREFIX.len() + 32 + payload.len();
        if length > MAX_CHECKPOINT_BYTES {
            return Err(OpsError::Invalid("checkpoint size"));
        }
        let mut bytes = Vec::with_capacity(length);
        bytes.extend_from_slice(CHECKPOINT_PREFIX);
        bytes.extend_from_slice(&AssetId::hash(&payload).bytes());
        bytes.extend_from_slice(&payload);
        Ok(bytes)
    }

    /// Restore private sequencing state without re-executing historical
    /// transactions. Retained journal metadata is checked against its indexes;
    /// the current project is fully validated and bound to the last revision.
    /// Only this version's canonical bytes are accepted, rejecting duplicate or
    /// unknown nested fields as well as accidental corruption. The checksum is
    /// an integrity check, not authentication of an untrusted project archive.
    pub fn from_checkpoint_bytes(bytes: &[u8]) -> Result<Self, OpsError> {
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            return Err(OpsError::Invalid("checkpoint size"));
        }
        let body = bytes
            .strip_prefix(CHECKPOINT_PREFIX)
            .ok_or(OpsError::Invalid("checkpoint schema version"))?;
        if body.len() < 32 {
            return Err(OpsError::Invalid("checkpoint header"));
        }
        let (digest, payload) = body.split_at(32);
        if AssetId::hash(payload).bytes().as_slice() != digest {
            return Err(OpsError::Invalid("checkpoint checksum"));
        }
        let mut decoder = serde_json::Deserializer::from_slice(payload);
        let mut host = CheckpointState::deserialize(&mut decoder)?;
        decoder.end()?;
        // This also rejects unknown fields in journal Address/Change values and
        // duplicate JSON map keys, whose serde representation would be lossy.
        if serde_json::to_vec(&serde_json::to_value(&host)?)?.as_slice() != payload {
            return Err(OpsError::Invalid("noncanonical checkpoint"));
        }
        host.validate_checkpoint()?;
        Ok(host)
    }

    fn validate_checkpoint(&mut self) -> Result<(), OpsError> {
        let count =
            usize::try_from(self.sequence).map_err(|_| OpsError::Invalid("checkpoint sequence"))?;
        if self.records.len() != count
            || self.revisions.len() != count.checked_add(1).ok_or(OpsError::CounterExhausted)?
        {
            return Err(OpsError::Invalid("checkpoint revision binding"));
        }
        self.project.validate()?;
        let (hash, canonical_len) = self.hasher.hash(&self.project)?;
        self.canonical_len = canonical_len;
        if self.revisions.get(&self.sequence) != Some(&hash) {
            return Err(OpsError::Invalid("checkpoint revision binding"));
        }
        for (expected, sequence) in self.revisions.keys().enumerate() {
            if *sequence != expected as u64 {
                return Err(OpsError::Invalid("checkpoint revision continuity"));
            }
        }
        let mut ordered = BTreeMap::new();
        let mut devices = BTreeSet::from([self.host_device.clone()]);
        for (id, record) in &self.records {
            if Id::from_proto(record.transaction.txn_id.as_ref())? != *id
                || record.ack.txn_id != record.transaction.txn_id
                || record.ack.host_seq == 0
                || record.ack.host_seq > self.sequence
                || ordered.insert(record.ack.host_seq, (id, record)).is_some()
            {
                return Err(OpsError::Invalid("checkpoint transaction binding"));
            }
            devices.insert(DeviceId::try_from(record.transaction.device_id.clone())?);
        }
        let mut seen_ops = BTreeSet::new();
        let mut gestures = BTreeSet::new();
        let mut writes = BTreeMap::new();
        let mut deleted = BTreeSet::new();
        let mut conflicts = Vec::new();
        let mut conflict_ids = BTreeSet::new();
        for (index, (sequence, (id, record))) in ordered.into_iter().enumerate() {
            let transaction = &record.transaction;
            let device = DeviceId::try_from(transaction.device_id.clone())?;
            let base = transaction
                .base_revision
                .as_ref()
                .ok_or(OpsError::Invalid("checkpoint base revision"))?;
            if sequence != index as u64 + 1
                || Id::from_proto(transaction.project_id.as_ref())? != self.project.id
                || transaction.created_at_wall_ms < 0
                || record.accepted_at_ms < 0
                || record.accepted_at_ms as u64 >= 1u64 << 48
                || transaction.ops.is_empty()
                || transaction.ops.len() > 4096
                || base.host_seq >= sequence
                || self
                    .revisions
                    .get(&base.host_seq)
                    .map(|hash| hash.bytes().to_vec())
                    != Some(base.state_hash.clone())
                || self
                    .revisions
                    .get(&sequence)
                    .map(|hash| hash.bytes().to_vec())
                    != Some(record.ack.state_hash.clone())
                || !valid_notices(&record.ack.notices)
            {
                return Err(OpsError::Invalid("checkpoint record metadata"));
            }
            for operation in &transaction.ops {
                let op_id = operation
                    .op_id
                    .as_ref()
                    .ok_or(OpsError::Invalid("checkpoint operation identity"))?;
                if op_id.device_id != device.as_str()
                    || op_id.lamport == 0
                    || !seen_ops.insert((device.clone(), op_id.lamport))
                {
                    return Err(OpsError::Invalid("checkpoint operation identity"));
                }
                validate_checkpoint_operation(operation, &device)?;
                if let Some(Kind::UndoTransaction(inverse)) = &operation.kind {
                    let target = self
                        .records
                        .get(&Id::from_proto(inverse.target_txn_id.as_ref())?)
                        .ok_or(OpsError::Invalid("checkpoint inverse target"))?;
                    if transaction.ops.len() != 1
                        || target.transaction.device_id != transaction.device_id
                        || target.ack.host_seq >= sequence
                    {
                        return Err(OpsError::Invalid("checkpoint inverse ownership"));
                    }
                }
            }
            if let Some(gesture) = &transaction.gesture_id
                && !gestures.insert((device.clone(), Id::from_proto(Some(gesture))?))
            {
                return Err(OpsError::Invalid("checkpoint duplicate gesture"));
            }
            let mut addresses = BTreeSet::new();
            for change in &record.changes {
                validate_checkpoint_change(change)?;
                if !addresses.insert(change.address.key()) {
                    return Err(OpsError::Invalid("checkpoint duplicate journal address"));
                }
                if change.address.collection == "objects" && change.address.path.is_empty() {
                    let object = Id::try_from(change.address.entity.clone())?;
                    if change.after.is_none() {
                        deleted.insert(object);
                    } else {
                        deleted.remove(&object);
                    }
                }
                writes.insert(
                    change.address.key(),
                    WriteStamp {
                        address: change.address.clone(),
                        device: device.clone(),
                        sequence,
                        wall_ms: transaction.created_at_wall_ms,
                    },
                );
            }
            for conflict in &record.conflicts {
                validate_checkpoint_conflict(
                    conflict,
                    id,
                    record.accepted_at_ms,
                    &device,
                    &devices,
                )?;
                if !conflict_ids.insert(Id::from_proto(conflict.conflict_id.as_ref())?) {
                    return Err(OpsError::Invalid("checkpoint duplicate conflict"));
                }
                conflicts.push(conflict.clone());
            }
        }
        if seen_ops != self.seen_ops
            || gestures != self.gestures
            || writes != self.writes
            || conflicts != self.conflicts
            || !self.gestures.is_disjoint(&self.cancelled)
            || deleted != self.tombstones.keys().cloned().collect()
        {
            return Err(OpsError::Invalid("checkpoint journal indexes"));
        }
        for (id, object) in &self.tombstones {
            if self.project.objects.contains_key(id)
                || Id::from_proto(object.state.object_id.as_ref())? != *id
            {
                return Err(OpsError::Invalid("checkpoint tombstone identity"));
            }
            vw_model::validate_object_state(&object.state)?;
        }
        Ok(())
    }

    /// Establish revision zero for a validated project and an explicit host ID.
    pub fn new(project: Project, host_device: DeviceId) -> Result<Self, OpsError> {
        project.validate()?;
        let mut hasher = StateHasher::default();
        let (hash, canonical_len) = hasher.hash(&project)?;
        Ok(Self {
            host_device,
            project,
            sequence: 0,
            revisions: BTreeMap::from([(0, hash)]),
            records: BTreeMap::new(),
            writes: BTreeMap::new(),
            tombstones: BTreeMap::new(),
            seen_ops: BTreeSet::new(),
            gestures: BTreeSet::new(),
            cancelled: BTreeSet::new(),
            conflicts: Vec::new(),
            hasher,
            canonical_len,
        })
    }
    /// The canonical state hash of the current revision.
    pub fn state_hash(&self) -> Result<&StateHash, OpsError> {
        self.revisions
            .get(&self.sequence)
            .ok_or(OpsError::RevisionMismatch)
    }
    /// Length of the canonical project bytes the current revision hashes.
    pub const fn canonical_len(&self) -> usize {
        self.canonical_len
    }
    fn view(&self) -> View<'_> {
        View {
            host_device: &self.host_device,
            sequence: self.sequence,
            revisions: &self.revisions,
            records: &self.records,
            writes: &self.writes,
            tombstones: &self.tombstones,
            seen_ops: &self.seen_ops,
            gestures: &self.gestures,
            cancelled: &self.cancelled,
        }
    }
    /// Current visible state; callers cannot mutate it outside transactions.
    pub const fn project(&self) -> &Project {
        &self.project
    }
    /// Durable conflict candidates accumulated once per accepted transaction.
    pub fn conflicts(&self) -> &[v1::ConflictRecord] {
        &self.conflicts
    }
    /// Current host revision, with the exact canonical state hash.
    pub fn revision(&self) -> Result<v1::Revision, OpsError> {
        let hash = self
            .revisions
            .get(&self.sequence)
            .ok_or(OpsError::RevisionMismatch)?;
        Ok(v1::Revision {
            host_seq: self.sequence,
            state_hash: hash.bytes().to_vec(),
        })
    }
    /// Snapshot plus exact retry bindings for local simulation and replica rebase.
    pub fn snapshot(&self) -> Result<HostSnapshot, OpsError> {
        // Inverses are tried on one working copy, which each trial restores.
        let mut project = self.project.clone();
        let view = self.view();
        let inverse_previews = self
            .records
            .iter()
            .map(|(id, record)| {
                let device = DeviceId::try_from(record.transaction.device_id.clone())?;
                let undo = v1::UndoTransaction {
                    target_txn_id: Some(id.to_proto()),
                };
                Ok((
                    id.clone(),
                    view.inverse(&mut project, &undo, &device, &mut Vec::new())?,
                ))
            })
            .collect::<Result<_, OpsError>>()?;
        Ok(HostSnapshot {
            project,
            revision: self.revision()?,
            inverse_previews,
            accepted: self
                .records
                .iter()
                .map(|(id, r)| (id.clone(), (r.transaction.clone(), r.ack.clone())))
                .collect(),
        })
    }
    /// Explicit cancellation rejects a later commit with the same device/gesture.
    /// Expiring a provisional preview alone does not cancel a reliable commit.
    pub fn cancel_gesture(&mut self, device: DeviceId, gesture: Id) -> Result<(), OpsError> {
        if self.gestures.contains(&(device.clone(), gesture.clone())) {
            return Err(OpsError::Invalid("gesture already committed"));
        }
        self.cancelled.insert((device, gesture));
        Ok(())
    }
    /// Borrow the accepted/cancelled authority without serializing a checkpoint.
    /// Preview expiry or retirement never modifies this durable authority.
    pub fn gesture_closed(&self, device: &DeviceId, gesture: &Id) -> bool {
        let key = (device.clone(), gesture.clone());
        self.gestures.contains(&key) || self.cancelled.contains(&key)
    }
    /// Accept using the authenticated transport identity and the host's receipt
    /// time. Client wall time is used only for stale property conflict resolution.
    pub fn submit(
        &mut self,
        transaction: v1::Transaction,
        authenticated: &DeviceId,
        accepted_at_ms: i64,
    ) -> Result<Acceptance, OpsError> {
        let (acceptance, _) =
            self.submit_with(transaction, authenticated, accepted_at_ms, |_, _, _| {
                Ok::<(), OpsError>(())
            })?;
        Ok(acceptance)
    }

    /// Accept a transaction only if `persist` then succeeds. `persist` sees the
    /// host as it will be, the acknowledgement and the entities the transaction
    /// touched; when it fails or unwinds, the host is exactly as before the
    /// call. A retry of an accepted transaction returns the original
    /// acknowledgement and `None` without calling `persist`.
    pub fn submit_with<T, E: From<OpsError>>(
        &mut self,
        transaction: v1::Transaction,
        authenticated: &DeviceId,
        accepted_at_ms: i64,
        persist: impl FnOnce(&Self, &Acceptance, &Touched) -> Result<T, E>,
    ) -> Result<(Acceptance, Option<T>), E> {
        let (device, id) = match self.admit(&transaction, authenticated)? {
            Admission::Duplicate(acceptance) => return Ok((acceptance, None)),
            Admission::New(device, id) => (device, id),
        };
        let (acceptance, undo) = self.commit(transaction, device, id, accepted_at_ms)?;
        let touched = undo.originals.touched();
        let mut recorded = Recorded {
            host: self,
            undo: Some(undo),
        };
        let value = persist(&*recorded.host, &acceptance, &touched)?;
        recorded.undo = None;
        Ok((acceptance, Some(value)))
    }

    /// Decide again a transaction that storage holds as accepted, and keep the
    /// acknowledgement it was accepted with. Returns what it touched, or
    /// nothing when this engine does not reproduce the stored outcome: it
    /// refuses the transaction, or gives it another revision or state hash.
    /// The host is then exactly as before the call.
    ///
    /// The outcome is the revision and the state hash. Notices are text for a
    /// person and are not compared: the stored ones are kept as they are, so a
    /// retry is answered, and a checkpoint is written, with the bytes the
    /// device first received whatever this build would word them as. See the
    /// replay rule at `evaluate`.
    pub fn replay(
        &mut self,
        transaction: v1::Transaction,
        accepted_at_ms: i64,
        stored: &v1::TxnAck,
    ) -> Result<Option<Touched>, OpsError> {
        let author = DeviceId::try_from(transaction.device_id.clone())?;
        let (device, id) = match self.admit(&transaction, &author)? {
            // A stored transaction is never a retry of an earlier one.
            Admission::Duplicate(_) => return Err(OpsError::AlreadyExists),
            Admission::New(device, id) => (device, id),
        };
        if !valid_notices(&stored.notices) {
            return Err(OpsError::Invalid("stored acknowledgement"));
        }
        let Ok((acceptance, undo)) = self.commit(transaction, device, id.clone(), accepted_at_ms)
        else {
            return Ok(None);
        };
        let ack = &acceptance.ack;
        if ack.txn_id != stored.txn_id
            || ack.host_seq != stored.host_seq
            || ack.state_hash != stored.state_hash
        {
            self.revert(undo);
            return Ok(None);
        }
        let touched = undo.originals.touched();
        if let Some(record) = self.records.get_mut(&id) {
            record.ack.notices.clone_from(&stored.notices);
        }
        Ok(Some(touched))
    }

    /// What `submit` would answer, without accepting anything, and the length
    /// of the canonical project bytes the transaction would leave. Only the
    /// project is copied to try it on, not the accepted history.
    pub fn check(
        &self,
        transaction: v1::Transaction,
        authenticated: &DeviceId,
        accepted_at_ms: i64,
    ) -> Result<(Acceptance, usize), OpsError> {
        let (device, id) = match self.admit(&transaction, authenticated)? {
            Admission::Duplicate(acceptance) => return Ok((acceptance, self.canonical_len)),
            Admission::New(device, id) => (device, id),
        };
        let mut project = self.project.clone();
        let mut hasher = self.hasher.clone();
        let decision = evaluate(
            &self.view(),
            &mut project,
            &mut hasher,
            transaction,
            device,
            id,
            accepted_at_ms,
        )?;
        Ok((
            Acceptance {
                ack: decision.record.ack,
                conflicts: decision.record.conflicts,
                duplicate: false,
            },
            decision.canonical_len,
        ))
    }

    fn admit(
        &self,
        transaction: &v1::Transaction,
        authenticated: &DeviceId,
    ) -> Result<Admission, OpsError> {
        let device = DeviceId::try_from(transaction.device_id.clone())?;
        if &device != authenticated {
            return Err(OpsError::DeviceMismatch);
        }
        let id = Id::from_proto(transaction.txn_id.as_ref())?;
        if let Some(record) = self.records.get(&id) {
            if record.transaction.encode_to_vec() != transaction.encode_to_vec() {
                return Err(OpsError::IdCollision);
            }
            return Ok(Admission::Duplicate(Acceptance {
                ack: record.ack.clone(),
                conflicts: Vec::new(),
                duplicate: true,
            }));
        }
        Ok(Admission::New(device, id))
    }

    /// Decide the transaction against the project in place, then record it.
    /// Nothing changes when the decision is a rejection.
    fn commit(
        &mut self,
        transaction: v1::Transaction,
        device: DeviceId,
        id: Id,
        accepted_at_ms: i64,
    ) -> Result<(Acceptance, Undo), OpsError> {
        let Self {
            host_device,
            project,
            sequence,
            revisions,
            records,
            writes,
            tombstones,
            seen_ops,
            gestures,
            cancelled,
            conflicts: _,
            hasher,
            canonical_len: _,
        } = self;
        let view = View {
            host_device,
            sequence: *sequence,
            revisions,
            records,
            writes,
            tombstones,
            seen_ops,
            gestures,
            cancelled,
        };
        let decision = evaluate(
            &view,
            project,
            hasher,
            transaction,
            device,
            id,
            accepted_at_ms,
        )?;
        let Decision {
            id,
            device,
            sequence,
            hash,
            canonical_len,
            record,
            originals,
            operations,
            gesture,
            tombstones,
        } = decision;
        let mut undo = Undo {
            id: id.clone(),
            sequence: self.sequence,
            canonical_len: self.canonical_len,
            conflicts: self.conflicts.len(),
            originals,
            operations: operations.clone(),
            gesture: gesture.clone(),
            tombstones: Vec::with_capacity(tombstones.len()),
            writes: Vec::with_capacity(record.changes.len()),
        };
        self.seen_ops.extend(operations);
        self.gestures.extend(gesture);
        for (object, deleted) in tombstones {
            let previous = match deleted {
                Some(deleted) => self.tombstones.insert(object.clone(), deleted),
                None => self.tombstones.remove(&object),
            };
            undo.tombstones.push((object, previous));
        }
        for change in &record.changes {
            let key = change.address.key();
            let previous = self.writes.insert(
                key.clone(),
                WriteStamp {
                    address: change.address.clone(),
                    device: device.clone(),
                    sequence,
                    wall_ms: record.transaction.created_at_wall_ms,
                },
            );
            undo.writes.push((key, previous));
        }
        let acceptance = Acceptance {
            ack: record.ack.clone(),
            conflicts: record.conflicts.clone(),
            duplicate: false,
        };
        self.conflicts.extend(record.conflicts.iter().cloned());
        self.records.insert(id, record);
        self.sequence = sequence;
        self.revisions.insert(sequence, hash);
        self.canonical_len = canonical_len;
        Ok((acceptance, undo))
    }

    /// Take the newest recorded transaction back out.
    fn revert(&mut self, undo: Undo) {
        for (key, previous) in undo.writes.into_iter().rev() {
            match previous {
                Some(stamp) => self.writes.insert(key, stamp),
                None => self.writes.remove(&key),
            };
        }
        for (object, previous) in undo.tombstones.into_iter().rev() {
            match previous {
                Some(deleted) => self.tombstones.insert(object, deleted),
                None => self.tombstones.remove(&object),
            };
        }
        for operation in &undo.operations {
            self.seen_ops.remove(operation);
        }
        if let Some(gesture) = &undo.gesture {
            self.gestures.remove(gesture);
        }
        self.records.remove(&undo.id);
        self.revisions.remove(&self.sequence);
        self.sequence = undo.sequence;
        self.conflicts.truncate(undo.conflicts);
        self.canonical_len = undo.canonical_len;
        undo.originals.forget_in(&mut self.hasher);
        undo.originals.restore(&mut self.project);
    }
}

impl<'a> View<'a> {
    /// The newest write stamp at, above or below an address. Stamps are keyed
    /// `collection/entity/path…`, so one entity's stamps are adjacent.
    fn latest(&self, address: &Address) -> Option<&'a WriteStamp> {
        let entity = format!("{}/{}", address.collection, address.entity);
        self.writes
            .range::<str, _>((Bound::Included(entity.as_str()), Bound::Unbounded))
            .take_while(|(key, _)| key.starts_with(&entity))
            .map(|(_, stamp)| stamp)
            .filter(|stamp| stamp.address.overlaps(address))
            .max_by_key(|stamp| stamp.sequence)
    }
    // A parent replacement/deletion covers multiple independently owned
    // properties. Determine each current leaf's effective owner before choosing
    // an opposing write; a newer own sibling cannot hide another peer's edit.
    // Looking up each leaf first also ignores fully superseded ancestor stamps.
    fn effective_opposing(
        &self,
        change: &Change,
        base_sequence: u64,
        device: &DeviceId,
    ) -> Option<&'a WriteStamp> {
        let Some(before) = change.before.as_ref() else {
            return self
                .latest(&change.address)
                .filter(|stamp| stamp.sequence > base_sequence && &stamp.device != device);
        };
        let mut pending = vec![(change.address.clone(), before)];
        let mut opposing: Option<&WriteStamp> = None;
        while let Some((address, value)) = pending.pop() {
            if let Some(fields) = value.as_object().filter(|fields| !fields.is_empty()) {
                for (key, value) in fields {
                    let mut child = address.clone();
                    child.path.push(key.clone());
                    pending.push((child, value));
                }
            } else if let Some(stamp) = self
                .latest(&address)
                .filter(|stamp| stamp.sequence > base_sequence && &stamp.device != device)
                && opposing.is_none_or(|current| stamp.sequence > current.sequence)
            {
                opposing = Some(stamp);
            }
        }
        opposing
    }

    /// The changes that undo a recorded transaction now. They are tried on
    /// `project`, which must be the current project and is left unchanged.
    fn inverse(
        &self,
        project: &mut Project,
        undo: &v1::UndoTransaction,
        device: &DeviceId,
        notices: &mut Vec<String>,
    ) -> Result<Vec<Change>, OpsError> {
        let record = self
            .records
            .get(&Id::from_proto(undo.target_txn_id.as_ref())?)
            .ok_or(OpsError::NotFound)?;
        if record.transaction.device_id != device.as_str() {
            return Err(OpsError::UndoOwnership);
        }
        let mut inverse = Vec::new();
        for change in record.changes.iter().rev() {
            // The last-write index is insufficient here: an intervening peer
            // edit may have since been overwritten by this device. Undo still
            // must not erase that peer's contribution without a notice.
            let peer_changed = self.records.values().any(|later| {
                later.ack.host_seq > record.ack.host_seq
                    && later.transaction.device_id != device.as_str()
                    && later
                        .changes
                        .iter()
                        .any(|other| other.address.overlaps(&change.address))
            });
            if peer_changed {
                // Only property names are returned; no IDs, source names or values.
                notices.push(format!(
                    "Skipped {}: changed on another device",
                    change.address.property()
                ));
            } else {
                inverse.push(change.inverse());
            }
        }
        if let Ok(replaced) = draft::edit(project, |draft| patch::apply(draft, &inverse)) {
            replaced.restore(project);
            return Ok(inverse);
        }
        // A skipped child edit can leave a parent deletion invalid. Preserve
        // dependent state and still undo every independent valid change. Retry
        // pending changes after each pass, since parents may be restored later.
        let mut pending = inverse;
        let mut applied = Vec::new();
        let mut replaced = Vec::new();
        loop {
            let mut remaining = Vec::new();
            let mut progress = false;
            for change in pending {
                match draft::edit(project, |draft| {
                    patch::apply(draft, std::slice::from_ref(&change))
                }) {
                    Ok(originals) => {
                        replaced.push(originals);
                        applied.push(change);
                        progress = true;
                    }
                    Err(_) => remaining.push(change),
                }
            }
            if remaining.is_empty() {
                break;
            }
            if !progress {
                for change in remaining {
                    notices.push(format!(
                        "Skipped {}: dependent state changed",
                        change.address.property()
                    ));
                }
                break;
            }
            pending = remaining;
        }
        for originals in replaced.into_iter().rev() {
            originals.restore(project);
        }
        Ok(applied)
    }
}

/// What a stored acknowledgement's notices may be.
fn valid_notices(notices: &[String]) -> bool {
    notices
        .iter()
        .all(|notice| notice.len() <= 1024 && !notice.contains('\0'))
}

/// Decide one new transaction. On success `project` holds the accepted result
/// and `hasher` has followed it; on any error both describe the project as it
/// was. Nothing in `view` is changed either way.
///
/// REPLAY RULE. Storage keeps the accepted transactions and not their
/// results. When a project opens, every transaction stored after its last
/// checkpoint is decided again here, and for most projects that is every
/// transaction since the project was created. So for an input that a released
/// build accepted, this function, and everything it calls in this crate and
/// in `vw-model`, must go on accepting it with the same resulting project
/// (the state hash) and the same conflict records. A build that decides such
/// an input differently cannot open the projects that hold it: storage does
/// not guess, it refuses them as saved by a different version of the app
/// (`HostSequencer::replay`, `vw_store::StoreError::EngineMismatch`).
///
/// Free to change: the wording and number of notices (replay keeps the stored
/// ones), what is refused that was refused before, new kinds of operation,
/// speed. Not free: who wins a conflict, what an undo restores, defaults
/// filled in, validation that turns an accepted input into a refused one,
/// the canonical bytes, any field of a conflict record. To change one of
/// those, keep the old rule for transactions stored before the change. A log
/// row carries no engine version today; the build that first needs one adds
/// it, and rows without it were decided by this rule.
///
/// The pinned logs in `vw-store/tests/replay_rule.rs` fail when this is
/// broken. Do not update their expected values to make them pass.
fn evaluate(
    view: &View<'_>,
    project: &mut Project,
    hasher: &mut StateHasher,
    transaction: v1::Transaction,
    device: DeviceId,
    id: Id,
    accepted_at_ms: i64,
) -> Result<Decision, OpsError> {
    if Id::from_proto(transaction.project_id.as_ref())? != project.id {
        return Err(OpsError::Invalid("project identity"));
    }
    if transaction.ops.is_empty()
        || transaction.ops.len() > 4096
        || transaction.created_at_wall_ms < 0
        || accepted_at_ms < 0
        || accepted_at_ms as u64 >= 1u64 << 48
    {
        return Err(OpsError::Invalid("transaction bounds"));
    }
    let base = transaction
        .base_revision
        .as_ref()
        .ok_or(OpsError::Invalid("base revision"))?;
    if view
        .revisions
        .get(&base.host_seq)
        .map(|v| v.bytes().to_vec())
        != Some(base.state_hash.clone())
    {
        return Err(OpsError::RevisionMismatch);
    }
    let sequence = view
        .sequence
        .checked_add(1)
        .ok_or(OpsError::CounterExhausted)?;
    let mut operations = Vec::with_capacity(transaction.ops.len());
    let mut counters = BTreeSet::new();
    for op in &transaction.ops {
        let op_id = op
            .op_id
            .as_ref()
            .ok_or(OpsError::Invalid("operation identity"))?;
        let operation = (device.clone(), op_id.lamport);
        if op_id.device_id != device.as_str()
            || op_id.lamport == 0
            || view.seen_ops.contains(&operation)
            || !counters.insert(op_id.lamport)
        {
            return Err(OpsError::Invalid("operation identity/counter"));
        }
        operations.push(operation);
    }
    let gesture = match &transaction.gesture_id {
        Some(gesture) => {
            let key = (device.clone(), Id::from_proto(Some(gesture))?);
            if view.cancelled.contains(&key) || view.gestures.contains(&key) {
                return Err(OpsError::Invalid("cancelled or duplicate gesture"));
            }
            Some(key)
        }
        None => None,
    };
    let mut notices = Vec::new();
    let mut conflicts = Vec::new();
    let changes = if let [
        v1::Op {
            kind: Some(Kind::UndoTransaction(undo)),
            ..
        },
    ] = transaction.ops.as_slice()
    {
        view.inverse(project, undo, &device, &mut notices)?
    } else {
        let mut filtered = transaction.clone();
        filtered.ops.clear();
        let mut ghosts = BTreeMap::<Id, v1::ObjectState>::new();
        let mut ghost_paths = BTreeSet::<(Id, String)>::new();
        for op in &transaction.ops {
            match op.kind.as_ref() {
                Some(Kind::SetProperty(property)) => {
                    let object_id = Id::from_proto(property.object_id.as_ref())?;
                    if !project.objects.contains_key(&object_id)
                        && let Some(deleted) = view.tombstones.get(&object_id)
                    {
                        let state = ghosts
                            .entry(object_id.clone())
                            .or_insert_with(|| deleted.state.clone());
                        plan::set_property(state, property)?;
                        ghost_paths.insert((object_id, property.property.clone()));
                        continue;
                    }
                }
                Some(Kind::Reorder(value)) => {
                    let object_id = Id::from_proto(value.object_id.as_ref())?;
                    if !project.objects.contains_key(&object_id)
                        && let Some(deleted) = view.tombstones.get(&object_id)
                    {
                        let destination = project
                            .layers
                            .get(&Id::from_proto(value.layer_id.as_ref())?)
                            .ok_or(OpsError::NotFound)?;
                        if Id::from_proto(destination.definition.document_id.as_ref())?
                            != deleted.document_id
                        {
                            return Err(OpsError::Invalid("cross-document object layer"));
                        }
                        if destination.locked || deleted.state.locked {
                            return Err(OpsError::Locked);
                        }
                        vw_model::OrderKey::try_from(value.order_key.clone())?;
                        let state = ghosts
                            .entry(object_id.clone())
                            .or_insert_with(|| deleted.state.clone());
                        state.layer_id = value.layer_id.clone();
                        state.order_key = value.order_key.clone();
                        ghost_paths.insert((object_id, "reorder".into()));
                        continue;
                    }
                }
                Some(Kind::DeleteObject(value)) => {
                    let object_id = Id::from_proto(value.object_id.as_ref())?;
                    if !project.objects.contains_key(&object_id)
                        && view.tombstones.contains_key(&object_id)
                    {
                        continue;
                    }
                }
                Some(Kind::CreateObject(value)) => {
                    let state = value
                        .state
                        .as_ref()
                        .ok_or(OpsError::Invalid("object state"))?;
                    if view
                        .tombstones
                        .contains_key(&Id::from_proto(state.object_id.as_ref())?)
                    {
                        return Err(OpsError::AlreadyExists);
                    }
                }
                _ => {}
            }
            filtered.ops.push(op.clone());
        }
        for (object_id, property) in ghost_paths {
            let edited = ghosts.get(&object_id).ok_or(OpsError::NotFound)?;
            let address = Address {
                collection: "objects".into(),
                entity: object_id.to_string(),
                path: Vec::new(),
            };
            let stamp = view
                .latest(&address)
                .ok_or(OpsError::Invalid("deletion stamp"))?;
            conflicts.push(conflict(
                &id,
                &address,
                &property,
                None,
                Some(&serde_json::to_value(edited)?),
                &stamp.device,
                &device,
                accepted_at_ms,
                true,
                Some(edited.clone()),
            )?);
        }
        // Carry the operations out in place to learn what they change, then
        // put the project back: stale changes may still lose below.
        let mut draft = Draft::new(project);
        let planned = plan::apply(&mut draft, &filtered)
            .and_then(|()| Ok(draft.validate()?))
            .and_then(|()| {
                let mut planned = patch::between(draft.originals(), draft.project())?;
                append_explicit_writes(
                    draft.originals(),
                    draft.project(),
                    &filtered,
                    &mut planned,
                )?;
                Ok(planned)
            });
        draft.discard();
        let mut accepted = Vec::new();
        for change in planned? {
            let opposing = view.effective_opposing(&change, base.host_seq, &device);
            if let Some(stamp) = opposing {
                let deleting = change.address.path.is_empty() && change.after.is_none();
                let incoming_wins = deleting
                    || transaction.created_at_wall_ms > stamp.wall_ms
                    || (transaction.created_at_wall_ms == stamp.wall_ms
                        && &device == view.host_device);
                let (kept, other, kept_device, other_device) = if incoming_wins {
                    (
                        change.after.as_ref(),
                        change.before.as_ref(),
                        &device,
                        &stamp.device,
                    )
                } else {
                    (
                        change.before.as_ref(),
                        change.after.as_ref(),
                        &stamp.device,
                        &device,
                    )
                };
                let edited = if deleting && change.address.collection == "objects" {
                    project
                        .objects
                        .get(&Id::try_from(change.address.entity.clone())?)
                        .map(|o| o.state.clone())
                } else {
                    None
                };
                // Equal-value writes still establish ownership, but have
                // no distinct losing value for the user to review.
                if change.before != change.after {
                    conflicts.push(conflict(
                        &id,
                        &change.address,
                        &change.address.property(),
                        kept,
                        other,
                        kept_device,
                        other_device,
                        accepted_at_ms,
                        edited.is_some(),
                        edited,
                    )?);
                }
                if incoming_wins {
                    accepted.push(change);
                }
            } else {
                accepted.push(change);
            }
        }
        accepted
    };
    let originals = draft::edit(project, |draft| patch::apply(draft, &changes))?;
    originals.forget_in(hasher);
    // The project now holds the result. Anything that still fails has to put
    // it back before reporting the error.
    let concluded = (|| {
        let mut tombstones = Vec::new();
        for change in &changes {
            if change.address.collection == "objects" && change.address.path.is_empty() {
                let object_id = Id::try_from(change.address.entity.clone())?;
                if change.after.is_some() {
                    tombstones.push((object_id, None));
                } else if let Some(Some(old)) = originals.objects.get(&object_id) {
                    tombstones.push((object_id, Some(old.clone())));
                }
            }
        }
        let (hash, canonical_len) = hasher.hash(project)?;
        #[cfg(feature = "cross-check")]
        assert_eq!(
            Some(&hash),
            project.state_hash().ok().as_ref(),
            "incremental state hash differs from the whole-project hash"
        );
        Ok::<_, OpsError>((tombstones, hash, canonical_len))
    })();
    let (tombstones, hash, canonical_len) = match concluded {
        Ok(concluded) => concluded,
        Err(error) => {
            originals.forget_in(hasher);
            originals.restore(project);
            return Err(error);
        }
    };
    let ack = v1::TxnAck {
        txn_id: Some(id.to_proto()),
        host_seq: sequence,
        state_hash: hash.bytes().to_vec(),
        notices,
    };
    Ok(Decision {
        id,
        device,
        sequence,
        hash,
        canonical_len,
        record: Record {
            transaction,
            ack,
            changes,
            accepted_at_ms,
            conflicts,
        },
        originals,
        operations,
        gesture,
        tombstones,
    })
}

fn validate_checkpoint_operation(operation: &v1::Op, device: &DeviceId) -> Result<(), OpsError> {
    let kind = operation
        .kind
        .as_ref()
        .ok_or(OpsError::Invalid("checkpoint operation kind"))?;
    let ids = match kind {
        Kind::CreateDocument(value) => {
            if value.schema_version != 1 || !(1..=4).contains(&value.kind) {
                return Err(OpsError::Invalid("checkpoint document schema"));
            }
            if let Some(capture) = &value.capture {
                vw_model::validate_capture(capture)?;
            }
            vec![value.document_id.as_ref()]
        }
        Kind::UpdateDocument(value) => vec![value.document_id.as_ref()],
        Kind::AddAsset(value) => {
            vw_model::validate_asset(value)?;
            Vec::new()
        }
        Kind::CreateLayer(value) => vec![value.layer_id.as_ref(), value.document_id.as_ref()],
        Kind::UpdateLayer(value) => vec![value.layer_id.as_ref()],
        Kind::DeleteLayer(value) => vec![value.layer_id.as_ref()],
        Kind::CreateObject(value) => {
            let state = value
                .state
                .as_ref()
                .ok_or(OpsError::Invalid("checkpoint object"))?;
            vw_model::validate_object_state(state)?;
            if state.created_by != device.as_str() {
                return Err(OpsError::DeviceMismatch);
            }
            vec![value.document_id.as_ref()]
        }
        Kind::SetProperty(value) => {
            if value
                .value
                .as_ref()
                .and_then(|value| value.value.as_ref())
                .is_none()
            {
                return Err(OpsError::Invalid("checkpoint property"));
            }
            vec![value.object_id.as_ref()]
        }
        Kind::DeleteObject(value) => vec![value.object_id.as_ref()],
        Kind::Reorder(value) => {
            vw_model::OrderKey::try_from(value.order_key.clone())?;
            vec![value.object_id.as_ref(), value.layer_id.as_ref()]
        }
        Kind::Group(value) => {
            let mut ids = vec![value.group_id.as_ref()];
            ids.extend(value.object_ids.iter().map(Some));
            ids
        }
        Kind::Ungroup(value) => vec![value.group_id.as_ref()],
        Kind::SetInstruction(value) => {
            let mut ids = vec![value.instruction_id.as_ref(), value.document_id.as_ref()];
            ids.extend(value.target_object_ids.iter().map(Some));
            ids
        }
        Kind::DeleteInstruction(value) => vec![value.instruction_id.as_ref()],
        Kind::AddSemanticSnapshot(value) => {
            vec![value.snapshot_id.as_ref(), value.document_id.as_ref()]
        }
        Kind::AddResult(value) => {
            let mut ids = vec![value.result_id.as_ref(), value.document_id.as_ref()];
            ids.extend(value.package_id.as_ref().map(Some));
            ids
        }
        Kind::UpdateResult(value) => vec![value.result_id.as_ref()],
        Kind::MaskOp(value) => {
            let mut ids = vec![value.output_object_id.as_ref()];
            ids.extend(value.input_object_ids.iter().map(Some));
            ids
        }
        Kind::UndoTransaction(value) => vec![value.target_txn_id.as_ref()],
    };
    for id in ids {
        Id::from_proto(id)?;
    }
    Ok(())
}

fn validate_checkpoint_change(change: &Change) -> Result<(), OpsError> {
    let address = &change.address;
    if !matches!(
        address.collection.as_str(),
        "documents"
            | "assets"
            | "layers"
            | "objects"
            | "groups"
            | "instructions"
            | "semantic_snapshots"
            | "results"
            | "mask_versions"
    ) || address.path.len() > 16
        || address.path.iter().any(|part| {
            part.is_empty()
                || part.len() > 64
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
        // An entity always has a value on one side. A property need not: a
        // write that leaves an absent optional value absent is journaled as
        // null before and after, and null reads back as no value at all.
        || (address.path.is_empty() && change.before.is_none() && change.after.is_none())
    {
        return Err(OpsError::Invalid("checkpoint journal address"));
    }
    if address.collection == "assets" {
        AssetId::try_from(address.entity.clone())?;
    } else {
        Id::try_from(address.entity.clone())?;
    }
    if !address.path.is_empty() && !valid_checkpoint_path(address) {
        return Err(OpsError::Invalid("checkpoint journal property"));
    }
    if address.path.is_empty() {
        for value in [change.before.as_ref(), change.after.as_ref()]
            .into_iter()
            .flatten()
        {
            validate_checkpoint_entity(address, value)?;
        }
    }
    Ok(())
}

fn valid_checkpoint_path(address: &Address) -> bool {
    let path: Vec<_> = address.path.iter().map(String::as_str).collect();
    matches!(
        (address.collection.as_str(), path.as_slice()),
        ("documents", ["definition", "title"])
            | ("layers", ["definition", "name" | "order_key"])
            | ("layers", ["visible" | "locked" | "opacity" | "blend"])
            | ("groups", ["parent"])
            | ("instructions", ["updated_at_ms"])
            | (
                "instructions",
                [
                    "definition",
                    "document_id"
                        | "target_object_ids"
                        | "role"
                        | "text"
                        | "entry_method"
                        | "language",
                ],
            )
            | ("instructions", ["definition", "instruction_id", "value"])
            | ("results", ["status" | "acceptance_mask_asset_id"])
            | (
                "objects",
                [
                    "state",
                    "order_key"
                        | "layer_id"
                        | "group_id"
                        | "transform"
                        | "role"
                        | "locked"
                        | "hidden"
                        | "shape",
                ],
            )
            | (
                "objects",
                [
                    "state",
                    "style",
                    "width" | "screen_constant_width" | "has_fill" | "fill" | "stroke",
                ],
            )
            | ("objects", ["state", "style", "stroke" | "fill", "rgba"])
            | (
                "objects",
                [
                    "state",
                    "shape",
                    "Stroke",
                    "x" | "y" | "t_ms" | "pressure" | "tilt" | "orientation",
                ],
            )
            | (
                "objects",
                [
                    "state",
                    "shape",
                    "Stroke",
                    "brush",
                    "family"
                        | "algorithm_version"
                        | "base_width"
                        | "pressure_curve"
                        | "stabilization",
                ],
            )
            | (
                "objects",
                [
                    "state",
                    "shape",
                    "Line" | "Arrow" | "Polygon" | "SelectionVector",
                    "points" | "closed",
                ],
            )
            | (
                "objects",
                [
                    "state",
                    "shape",
                    "Rect" | "Ellipse" | "Crop",
                    "x" | "y" | "w" | "h",
                ],
            )
            | (
                "objects",
                [
                    "state",
                    "shape",
                    "Text",
                    "text" | "font_family" | "font_size",
                ],
            )
            | ("objects", ["state", "shape", "Text", "anchor", "x" | "y"])
            | (
                "objects",
                [
                    "state",
                    "shape",
                    "Marker",
                    "number" | "element_eids" | "box",
                ],
            )
            | ("objects", ["state", "shape", "Marker", "point", "x" | "y"])
            | (
                "objects",
                ["state", "shape", "Marker", "box", "x" | "y" | "w" | "h"]
            )
            | (
                "objects",
                [
                    "state",
                    "shape",
                    "SelectionRaster",
                    "mask_asset_id" | "feather",
                ],
            )
            | (
                "objects",
                [
                    "state",
                    "shape",
                    "SelectionRaster",
                    "bounds",
                    "x" | "y" | "w" | "h",
                ],
            )
            | ("objects", ["state", "shape", "ResultId", "value"])
            | (
                "objects",
                [
                    "state",
                    "shape",
                    "Adjustment",
                    "brightness" | "contrast" | "levels",
                ],
            )
    )
}

fn validate_checkpoint_entity(
    address: &Address,
    value: &serde_json::Value,
) -> Result<(), OpsError> {
    let wire_id = match address.collection.as_str() {
        "objects" => {
            let object: Object = serde_json::from_value(value.clone())?;
            vw_model::validate_object_state(&object.state)?;
            object.state.object_id
        }
        "documents" => {
            let document: vw_model::Document = serde_json::from_value(value.clone())?;
            document.definition.document_id
        }
        "layers" => {
            let layer: vw_model::Layer = serde_json::from_value(value.clone())?;
            Id::from_proto(layer.definition.document_id.as_ref())?;
            vw_model::OrderKey::try_from(layer.definition.order_key)?;
            layer.definition.layer_id
        }
        "groups" => {
            let group: vw_model::Group = serde_json::from_value(value.clone())?;
            Some(group.id.to_proto())
        }
        "instructions" => {
            let instruction: vw_model::Instruction = serde_json::from_value(value.clone())?;
            Id::from_proto(instruction.definition.document_id.as_ref())?;
            instruction.definition.instruction_id
        }
        "semantic_snapshots" => {
            let snapshot: vw_model::SemanticSnapshot = serde_json::from_value(value.clone())?;
            Id::from_proto(snapshot.definition.document_id.as_ref())?;
            snapshot.definition.snapshot_id
        }
        "results" => {
            let result: vw_model::ResultCandidate = serde_json::from_value(value.clone())?;
            Id::from_proto(result.definition.document_id.as_ref())?;
            result.definition.result_id
        }
        "assets" => {
            let asset: v1::AddAsset = serde_json::from_value(value.clone())?;
            vw_model::validate_asset(&asset)?;
            if asset.asset_id != address.entity {
                return Err(OpsError::Invalid("checkpoint asset identity"));
            }
            return Ok(());
        }
        "mask_versions" => {
            let versions: Vec<v1::MaskOp> = serde_json::from_value(value.clone())?;
            for version in versions {
                if Id::from_proto(version.output_object_id.as_ref())?.as_str() != address.entity {
                    return Err(OpsError::Invalid("checkpoint mask identity"));
                }
                for input in &version.input_object_ids {
                    Id::from_proto(Some(input))?;
                }
            }
            return Ok(());
        }
        _ => return Err(OpsError::Invalid("checkpoint collection")),
    };
    if Id::from_proto(wire_id.as_ref())?.as_str() != address.entity {
        return Err(OpsError::Invalid("checkpoint journal entity identity"));
    }
    Ok(())
}

fn validate_checkpoint_conflict(
    value: &v1::ConflictRecord,
    transaction: &Id,
    accepted_at_ms: i64,
    author: &DeviceId,
    devices: &BTreeSet<DeviceId>,
) -> Result<(), OpsError> {
    let id = Id::from_proto(value.conflict_id.as_ref())?;
    let object = Id::from_proto(value.object_id.as_ref())?;
    let kept_device = DeviceId::try_from(value.kept_device.clone())?;
    let other_device = DeviceId::try_from(value.other_device.clone())?;
    if value.created_at_ms != accepted_at_ms
        || !devices.contains(&kept_device)
        || !devices.contains(&other_device)
        || (&kept_device != author && &other_device != author)
        || value.property.len() > 1024
        || value.property.contains('\0')
        || value.delete_vs_edit != value.edited_object.is_some()
    {
        return Err(OpsError::Invalid("checkpoint conflict metadata"));
    }
    let (collection, property) = value
        .property
        .split_once('.')
        .ok_or(OpsError::Invalid("checkpoint conflict property"))?;
    let mut address = Address {
        collection: collection.into(),
        entity: object.to_string(),
        path: Vec::new(),
    };
    let matches = |address: &Address| -> Result<bool, OpsError> {
        let digest =
            AssetId::hash(format!("{transaction}:{}:{property}", address.key()).as_bytes()).bytes();
        let mut entropy = [0; 10];
        entropy.copy_from_slice(&digest[..10]);
        Ok(Id::from_parts(accepted_at_ms as u64, entropy)? == id)
    };
    let ghost = matches(&address)?;
    if property != "$entity" {
        address.path = property.split('.').map(str::to_owned).collect();
    }
    if !ghost && !matches(&address)? {
        return Err(OpsError::Invalid("checkpoint conflict identity"));
    }
    for property_value in [&value.kept_value, &value.other_value] {
        let Some(v1::property_value::Value::Raw(bytes)) = property_value
            .as_ref()
            .and_then(|value| value.value.as_ref())
        else {
            return Err(OpsError::Invalid("checkpoint conflict value"));
        };
        let _: serde_json::Value = serde_json::from_slice(bytes)?;
    }
    if let Some(edited) = &value.edited_object {
        vw_model::validate_object_state(edited)?;
        if Id::from_proto(edited.object_id.as_ref())? != object {
            return Err(OpsError::Invalid("checkpoint edited object identity"));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn conflict(
    txn: &Id,
    address: &Address,
    property: &str,
    kept: Option<&serde_json::Value>,
    other: Option<&serde_json::Value>,
    kept_device: &DeviceId,
    other_device: &DeviceId,
    time: i64,
    delete_vs_edit: bool,
    edited_object: Option<v1::ObjectState>,
) -> Result<v1::ConflictRecord, OpsError> {
    let digest = AssetId::hash(format!("{}:{}:{property}", txn, address.key()).as_bytes()).bytes();
    let mut entropy = [0u8; 10];
    entropy.copy_from_slice(&digest[..10]);
    let conflict_id = Id::from_parts(time as u64, entropy)?;
    let value = |value: Option<&serde_json::Value>| -> Result<v1::PropertyValue, OpsError> {
        Ok(v1::PropertyValue {
            value: Some(v1::property_value::Value::Raw(serde_json::to_vec(&value)?)),
        })
    };
    Ok(v1::ConflictRecord {
        conflict_id: Some(conflict_id.to_proto()),
        object_id: Some(Id::try_from(address.entity.clone())?.to_proto()),
        property: format!("{}.{property}", address.collection),
        kept_value: Some(value(kept)?),
        other_value: Some(value(other)?),
        kept_device: kept_device.to_string(),
        other_device: other_device.to_string(),
        created_at_ms: time,
        delete_vs_edit,
        edited_object,
    })
}

// A value-equal explicit write is still a write: it establishes ordering and
// ownership for later stale conflicts and undo. A pure state diff loses it.
fn append_explicit_writes(
    before: &Originals,
    after: &Project,
    transaction: &v1::Transaction,
    changes: &mut Vec<Change>,
) -> Result<(), OpsError> {
    let mut values = Values::new(before, after);
    for op in &transaction.ops {
        let mut addresses = Vec::new();
        match op.kind.as_ref() {
            Some(Kind::SetProperty(property)) => {
                let property_path = match property.property.as_str() {
                    "text.text" => "shape.Text.text",
                    "text.font_family" => "shape.Text.font_family",
                    "text.font_size" => "shape.Text.font_size",
                    "marker.number" => "shape.Marker.number",
                    other => other,
                };
                let mut path = vec!["state".to_owned()];
                path.extend(property_path.split('.').map(str::to_owned));
                addresses.push(Address {
                    collection: "objects".into(),
                    entity: Id::from_proto(property.object_id.as_ref())?.to_string(),
                    path,
                });
                if property.property == "style.fill" {
                    addresses.push(Address {
                        collection: "objects".into(),
                        entity: Id::from_proto(property.object_id.as_ref())?.to_string(),
                        path: vec!["state".into(), "style".into(), "has_fill".into()],
                    });
                }
            }
            Some(Kind::SetInstruction(value)) => addresses.push(Address {
                collection: "instructions".into(),
                entity: Id::from_proto(value.instruction_id.as_ref())?.to_string(),
                path: vec!["definition".into()],
            }),
            Some(Kind::UpdateResult(value)) => {
                for property in ["status", "acceptance_mask_asset_id"] {
                    addresses.push(Address {
                        collection: "results".into(),
                        entity: Id::from_proto(value.result_id.as_ref())?.to_string(),
                        path: vec![property.into()],
                    });
                }
            }
            Some(Kind::Group(value)) => {
                for member in &value.object_ids {
                    let member = Id::from_proto(Some(member))?;
                    let (collection, path) = if after.objects.contains_key(&member) {
                        ("objects", vec!["state".into(), "group_id".into()])
                    } else {
                        ("groups", vec!["parent".into()])
                    };
                    addresses.push(Address {
                        collection: collection.into(),
                        entity: member.to_string(),
                        path,
                    });
                }
            }
            Some(Kind::UpdateDocument(value)) => addresses.push(Address {
                collection: "documents".into(),
                entity: Id::from_proto(value.document_id.as_ref())?.to_string(),
                path: vec!["definition".into(), "title".into()],
            }),
            Some(Kind::UpdateLayer(value)) => {
                for (present, path) in [
                    (value.name.is_some(), "definition.name"),
                    (value.order_key.is_some(), "definition.order_key"),
                    (value.visible.is_some(), "visible"),
                    (value.locked.is_some(), "locked"),
                    (value.opacity.is_some(), "opacity"),
                    (value.blend.is_some(), "blend"),
                ] {
                    if present {
                        addresses.push(Address {
                            collection: "layers".into(),
                            entity: Id::from_proto(value.layer_id.as_ref())?.to_string(),
                            path: path.split('.').map(str::to_owned).collect(),
                        });
                    }
                }
            }
            Some(Kind::Reorder(value)) => {
                for property in ["order_key", "layer_id"] {
                    addresses.push(Address {
                        collection: "objects".into(),
                        entity: Id::from_proto(value.object_id.as_ref())?.to_string(),
                        path: vec!["state".into(), property.into()],
                    });
                }
            }
            _ => {}
        }
        for address in addresses {
            append_same_leaves(&mut values, address, changes)?;
        }
    }
    Ok(())
}
fn append_same_leaves(
    values: &mut Values<'_>,
    address: Address,
    changes: &mut Vec<Change>,
) -> Result<(), OpsError> {
    let (Some(before), Some(after)) = values.at(&address)? else {
        return Ok(());
    };
    // Transform and identifiers are indivisible validated properties.
    let shape_variant_changed = address.path.last().is_some_and(|part| part == "shape")
        && before
            .as_object()
            .map(|value| value.keys().collect::<Vec<_>>())
            != after
                .as_object()
                .map(|value| value.keys().collect::<Vec<_>>());
    let atomic = shape_variant_changed
        || address.path.last().is_some_and(|part| {
            matches!(
                part.as_str(),
                "transform" | "group_id" | "layer_id" | "document_id"
            )
        });
    if !atomic && let (Some(before), Some(after)) = (before.as_object(), after.as_object()) {
        let shared = before
            .keys()
            .filter(|key| after.contains_key(*key))
            .cloned()
            .collect::<Vec<_>>();
        for key in shared {
            let mut child = address.clone();
            child.path.push(key);
            append_same_leaves(values, child, changes)?;
        }
        return Ok(());
    }
    if before == after
        && !changes
            .iter()
            .any(|change| change.address.overlaps(&address))
    {
        let (before, after) = (Some(before.clone()), Some(after.clone()));
        changes.push(Change {
            address,
            before,
            after,
        });
    }
    Ok(())
}
