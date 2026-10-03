use crate::{StoreError, blobs, migrations};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    time::Duration,
};
use vw_model::{AssetId, DeviceId, Id, Project};
use vw_ops::{Acceptance, HostSequencer};
use vw_proto::{Message, v1};

/// Accepted transaction count between recovery checkpoints.
pub const SNAPSHOT_TRANSACTIONS: u64 = 500;
/// Elapsed wall time between recovery checkpoints, evaluated at each commit.
pub const SNAPSHOT_INTERVAL_MS: i64 = 300_000;
pub(crate) const MAX_PAYLOAD: usize = 64 * 1024 * 1024;

/// Diagnostic commit boundaries. Observers must return promptly and never panic.
/// The process-crash harness kills its own writer at randomized boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitStage {
    Prepared,
    LogWritten,
    BeforeCommit,
    Committed,
}

/// A single-writer project database. Its advisory lock is released by the OS on
/// process death; WAL readers are allowed, but all writers must use this API.
pub struct ProjectStore {
    pub(crate) root: PathBuf,
    pub(crate) connection: Connection,
    host: HostSequencer,
    _lock: File,
    last_snapshot_seq: u64,
    last_snapshot_ms: i64,
    replayed_transactions: u64,
}

impl ProjectStore {
    /// Bootstrap a new mirror in an absent, caller-owned staging directory.
    /// Authentication comes from the pinned carrier, never a project field.
    /// The caller must verify/install all required originals before publishing
    /// this directory to its project list. Existing paths are never replaced.
    pub fn create_authenticated_checkpoint(
        path: &Path,
        bytes: &[u8],
        authenticated_host: &DeviceId,
        local_device: &DeviceId,
        now_ms: i64,
    ) -> Result<Self, StoreError> {
        if now_ms < 0 {
            return Err(StoreError::Invalid("creation time"));
        }
        let host = HostSequencer::from_checkpoint_bytes(bytes)?;
        if host.host_device() != authenticated_host
            || host.project().canonical_bytes()?.len() > MAX_PAYLOAD
        {
            return Err(StoreError::Invalid("authenticated bootstrap identity/size"));
        }
        let revision = host.revision()?;
        let seq = sql_u64(revision.host_seq)?;
        fs::create_dir(path)?;
        blobs::check_directory(path)?;
        let root = path.canonicalize()?;
        let lock = lock_project(&root)?;
        let mut connection = connect(&root, true)?;
        connection.execute_batch(migrations::SCHEMA)?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO meta(key,value) VALUES ('host_device',?1),('project_id',?2),('local_device',?3)",params![host.host_device().as_str(),host.project().id.as_str(),local_device.as_str()])?;
        let mut accepted = host.accepted_transactions().collect::<Vec<_>>();
        accepted.sort_by_key(|(_, ack)| ack.host_seq);
        for (txn, ack) in accepted {
            let id = Id::from_proto(txn.txn_id.as_ref())?;
            let accepted_at = host
                .accepted_at(&id)
                .ok_or(StoreError::Invalid("checkpoint receipt time"))?;
            let base = txn
                .base_revision
                .as_ref()
                .ok_or(StoreError::Invalid("checkpoint base"))?;
            for op in &txn.ops {
                sql_u64(
                    op.op_id
                        .as_ref()
                        .ok_or(StoreError::Invalid("op ID"))?
                        .lamport,
                )?;
                if let Some(v1::op::Kind::AddAsset(asset)) = &op.kind {
                    super::projections::persist_asset(&tx, asset)?;
                }
            }
            let hash: [u8; 32] = ack
                .state_hash
                .as_slice()
                .try_into()
                .map_err(|_| StoreError::Invalid("checkpoint receipt hash"))?;
            let hash = blake3::Hash::from_bytes(hash).to_hex().to_string();
            tx.execute("INSERT INTO op_log(host_seq,txn_id,device_id,base_host_seq,gesture_id,txn,state_hash,created_at_wall,accepted_at,ack) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![sql_u64(ack.host_seq)?,id.as_str(),txn.device_id,sql_u64(base.host_seq)?,optional_uuid(txn.gesture_id.as_ref())?,bounded_encode(txn)?,hash,txn.created_at_wall_ms,accepted_at,bounded_encode(ack)?])?;
        }
        super::projections::persist(&tx, host.project(), seq, now_ms)?;
        for conflict in host.conflicts() {
            write_conflict(&tx, conflict)?;
        }
        write_snapshot(&tx, &host, now_ms)?;
        verify_prefix(&tx, &host, seq)?;
        super::projections::verify_assets(&tx, host.project())?;
        super::projections::verify(&tx, host.project(), seq)?;
        integrity(&tx)?;
        tx.commit()?;
        blobs::ensure_directory(&root.join("blobs"))?;
        blobs::ensure_directory(&root.join("cache"))?;
        Ok(Self {
            root,
            connection,
            host,
            _lock: lock,
            last_snapshot_seq: revision.host_seq,
            last_snapshot_ms: now_ms,
            replayed_transactions: 0,
        })
    }

    /// The local editing identity is separate from the authoritative sequencer.
    /// Older locally created projects use their host identity unchanged.
    pub fn local_device(&self) -> Result<DeviceId, StoreError> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key='local_device'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(match value {
            Some(value) => DeviceId::try_from(value)?,
            None => self.host.host_device().clone(),
        })
    }
    /// Create at an absent caller-selected path. Asset bytes may arrive later;
    /// their immutable metadata already belongs to the project state.
    pub fn create(
        path: &Path,
        project: Project,
        host_device: DeviceId,
        now_ms: i64,
    ) -> Result<Self, StoreError> {
        if now_ms < 0 {
            return Err(StoreError::Invalid("creation time"));
        }
        let host = HostSequencer::new(project, host_device)?;
        if host.project().canonical_bytes()?.len() > MAX_PAYLOAD {
            return Err(StoreError::Invalid("snapshot size"));
        }
        fs::create_dir(path)?;
        blobs::check_directory(path)?;
        let root = path.canonicalize()?;
        let lock = lock_project(&root)?;
        let mut connection = connect(&root, true)?;
        connection.execute_batch(migrations::SCHEMA)?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO meta(key,value) VALUES ('host_device',?1),('project_id',?2)",
            params![host.host_device().as_str(), host.project().id.as_str()],
        )?;
        super::projections::persist(&tx, host.project(), 0, now_ms)?;
        write_snapshot(&tx, &host, now_ms)?;
        tx.commit()?;
        blobs::ensure_directory(&root.join("blobs"))?;
        blobs::ensure_directory(&root.join("cache"))?;
        Ok(Self {
            root,
            connection,
            host,
            _lock: lock,
            last_snapshot_seq: 0,
            last_snapshot_ms: now_ms,
            replayed_transactions: 0,
        })
    }

    /// Restore the newest validated checkpoint and replay only its committed
    /// tail. Unsupported schemas and damaged state are rejected without repair.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        blobs::check_directory(path)?;
        let root = path.canonicalize()?;
        let lock = lock_project(&root)?;
        let mut connection = connect(&root, false)?;
        migrations::upgrade(&mut connection)?;
        integrity(&connection)?;
        check_all_column_bounds(&connection)?;
        check_payload_bounds(&connection)?;
        let (seq,state,hash,time,checkpoint,digest):(i64,Vec<u8>,String,i64,Vec<u8>,String) = connection.query_row(
            "SELECT host_seq,state,state_hash,created_at,checkpoint,checkpoint_hash FROM snapshots ORDER BY host_seq DESC LIMIT 1", [],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
        if seq < 0 || time < 0 || AssetId::hash(&checkpoint).as_str() != digest {
            return Err(StoreError::Corrupt("checkpoint binding"));
        }
        let mut host = HostSequencer::from_checkpoint_bytes(&checkpoint)?;
        let state = Project::from_canonical_bytes(&state)?;
        if host.project() != &state
            || host.project().state_hash()?.as_str() != hash
            || host.revision()?.host_seq != seq as u64
        {
            return Err(StoreError::Corrupt("snapshot state"));
        }
        let device: String =
            connection.query_row("SELECT value FROM meta WHERE key='host_device'", [], |r| {
                r.get(0)
            })?;
        let project_id: String =
            connection.query_row("SELECT value FROM meta WHERE key='project_id'", [], |r| {
                r.get(0)
            })?;
        if device != host.host_device().as_str() || project_id != host.project().id.as_str() {
            return Err(StoreError::Corrupt("project identity"));
        }
        verify_prefix(&connection, &host, seq)?;
        let replayed = replay_log(&connection, &mut host, seq, true)?;
        let conflicts = read_conflicts(&connection)?;
        if conflicts != host.conflicts() {
            return Err(StoreError::Corrupt("conflict projection"));
        }
        super::projections::verify_assets(&connection, host.project())?;
        super::projections::verify(
            &connection,
            host.project(),
            sql_u64(host.revision()?.host_seq)?,
        )?;
        validate_pending(&connection, host.project())?;
        Ok(Self {
            root,
            connection,
            host,
            _lock: lock,
            last_snapshot_seq: seq as u64,
            last_snapshot_ms: time,
            replayed_transactions: replayed,
        })
    }

    pub fn project(&self) -> &Project {
        self.host.project()
    }
    pub fn revision(&self) -> Result<v1::Revision, StoreError> {
        Ok(self.host.revision()?)
    }
    /// Validated complete sequencing state for authenticated transport resume.
    pub fn checkpoint_bytes(&self) -> Result<Vec<u8>, StoreError> {
        Ok(self.host.checkpoint_bytes()?)
    }

    /// All immutable originals retained by the accepted journal, including
    /// assets currently hidden by undo. A replica must transfer this inventory
    /// before reporting that the complete project is available offline.
    pub fn retained_assets(&self) -> Result<std::collections::BTreeMap<AssetId, u64>, StoreError> {
        super::projections::verify_assets(&self.connection, self.host.project())?;
        let mut query = self
            .connection
            .prepare("SELECT asset_id,byte_size FROM assets ORDER BY asset_id")?;
        let mut rows = query.query([])?;
        let mut assets = std::collections::BTreeMap::new();
        while let Some(row) = rows.next()? {
            let id = AssetId::try_from(row.get::<_, String>(0)?)?;
            let size = u64::try_from(row.get::<_, i64>(1)?)
                .map_err(|_| StoreError::Corrupt("asset byte size"))?;
            if size == 0 || assets.insert(id, size).is_some() {
                return Err(StoreError::Corrupt("asset inventory"));
            }
        }
        Ok(assets)
    }

    /// An owned temporary file in checked project-private storage. Android
    /// applications cannot assume the process-default temporary path is usable.
    /// The returned handle removes only its own file when dropped.
    pub fn transfer_file(&self) -> Result<tempfile::NamedTempFile, StoreError> {
        blobs::check_directory(&self.root)?;
        let cache = self.root.join("cache");
        blobs::check_directory(&cache)?;
        let directory = cache.join("session-transfers-v1");
        blobs::ensure_directory(&directory)?;
        Ok(tempfile::Builder::new()
            .prefix("transfer-")
            .tempfile_in(directory)?)
    }

    /// Local authoritative identity, persisted in the verified checkpoint.
    pub fn host_device(&self) -> &DeviceId {
        self.host.host_device()
    }

    /// Accepted bindings for restoring application undo cursors without replay
    /// or quadratic generation of every transaction's inverse preview.
    pub fn accepted_transactions(&self) -> impl Iterator<Item = (&v1::Transaction, &v1::TxnAck)> {
        self.host.accepted_transactions()
    }

    /// Install a host-authenticated, validated checkpoint and acknowledge only
    /// exact matching pending transactions in the same SQLite commit. The caller
    /// must obtain `authenticated_host` from its pinned transport, not a payload.
    /// Existing accepted history must be an exact prefix; stale, foreign and
    /// divergent histories cannot replace local durable state.
    pub fn install_authenticated_checkpoint(
        &mut self,
        bytes: &[u8],
        authenticated_host: &DeviceId,
        now_ms: i64,
    ) -> Result<usize, StoreError> {
        self.install_authenticated_checkpoint_observed(bytes, authenticated_host, now_ms, |_| {})
    }

    /// Diagnostic transaction boundaries for the process-crash regression.
    /// Observers have the same no-panic/no-blocking contract as commit observers.
    pub fn install_authenticated_checkpoint_observed<F: FnMut(CommitStage)>(
        &mut self,
        bytes: &[u8],
        authenticated_host: &DeviceId,
        now_ms: i64,
        mut observer: F,
    ) -> Result<usize, StoreError> {
        if now_ms < 0 || authenticated_host != self.host.host_device() {
            return Err(StoreError::Invalid(
                "authenticated checkpoint identity/time",
            ));
        }
        let candidate = HostSequencer::from_checkpoint_bytes(bytes)?;
        if candidate.host_device() != authenticated_host
            || candidate.project().id != self.project().id
        {
            return Err(StoreError::Invalid("authenticated checkpoint project/host"));
        }
        let previous = self.host.revision()?;
        let revision = candidate.revision()?;
        if revision.host_seq < previous.host_seq
            || (revision.host_seq == previous.host_seq && revision != previous)
            || !candidate.conflicts().starts_with(self.host.conflicts())
            || !self.host.cancellations_are_subset_of(&candidate)
            || candidate.project().canonical_bytes()?.len() > MAX_PAYLOAD
        {
            return Err(StoreError::Invalid("stale or divergent checkpoint"));
        }
        let snapshot = candidate.snapshot()?;
        let mut ordered = snapshot.accepted.values().collect::<Vec<_>>();
        ordered.sort_by_key(|(_, ack)| ack.host_seq);
        if previous.host_seq == 0
            && revision.host_seq > 0
            && ordered
                .first()
                .and_then(|(txn, _)| txn.base_revision.as_ref())
                != Some(&previous)
        {
            return Err(StoreError::Invalid("checkpoint initial state"));
        }
        let seq = sql_u64(revision.host_seq)?;
        let old_seq = sql_u64(previous.host_seq)?;
        // Monotonic checkpoint bookkeeping even if the caller's wall clock moves back.
        let snapshot_time = now_ms.max(self.last_snapshot_ms);
        observer(CommitStage::Prepared);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        migrations::verify_schema(&tx, 2)?;
        check_all_column_bounds(&tx)?;
        check_payload_bounds(&tx)?;
        let durable_seq: i64 =
            tx.query_row("SELECT COALESCE(MAX(host_seq),0) FROM op_log", [], |row| {
                row.get(0)
            })?;
        if durable_seq != old_seq {
            return Err(StoreError::Corrupt("untracked accepted log tail"));
        }
        verify_prefix(&tx, &self.host, old_seq)?;
        // Also compare every original receipt/time to the incoming prefix, not
        // just the visible-state hash (different histories can have the same state).
        verify_prefix(&tx, &candidate, old_seq)?;
        if read_conflicts(&tx)? != self.host.conflicts() {
            return Err(StoreError::Corrupt("conflict projection"));
        }
        super::projections::verify_assets(&tx, self.host.project())?;
        super::projections::verify(&tx, self.host.project(), old_seq)?;
        let pending = validate_pending(&tx, self.host.project())?;
        let mut acknowledged = Vec::new();
        for txn in &pending {
            let id = Id::from_proto(txn.txn_id.as_ref())?;
            if let Some((accepted, _)) = candidate.accepted_transaction(&id) {
                if bounded_encode(txn)? != bounded_encode(accepted)? {
                    return Err(StoreError::Invalid(
                        "pending checkpoint transaction collision",
                    ));
                }
                acknowledged.push(txn);
            }
        }
        for (txn, ack) in ordered
            .into_iter()
            .filter(|(_, ack)| ack.host_seq > previous.host_seq)
        {
            let id = Id::from_proto(txn.txn_id.as_ref())?;
            let accepted_at = candidate
                .accepted_at(&id)
                .ok_or(StoreError::Invalid("checkpoint receipt time"))?;
            let base = txn
                .base_revision
                .as_ref()
                .ok_or(StoreError::Invalid("checkpoint base"))?;
            for op in &txn.ops {
                sql_u64(
                    op.op_id
                        .as_ref()
                        .ok_or(StoreError::Invalid("op ID"))?
                        .lamport,
                )?;
                // Preserve originals introduced then removed before this checkpoint.
                if let Some(v1::op::Kind::AddAsset(asset)) = &op.kind {
                    super::projections::persist_asset(&tx, asset)?;
                }
            }
            let hash: [u8; 32] = ack
                .state_hash
                .as_slice()
                .try_into()
                .map_err(|_| StoreError::Invalid("checkpoint receipt hash"))?;
            let hash = blake3::Hash::from_bytes(hash).to_hex().to_string();
            tx.execute("INSERT INTO op_log(host_seq,txn_id,device_id,base_host_seq,gesture_id,txn,state_hash,created_at_wall,accepted_at,ack) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![sql_u64(ack.host_seq)?,id.as_str(),txn.device_id,sql_u64(base.host_seq)?,optional_uuid(txn.gesture_id.as_ref())?,bounded_encode(txn)?,hash,txn.created_at_wall_ms,accepted_at,bounded_encode(ack)?])?;
        }
        observer(CommitStage::LogWritten);
        super::projections::persist(&tx, candidate.project(), seq, snapshot_time)?;
        for conflict in &candidate.conflicts()[self.host.conflicts().len()..] {
            write_conflict(&tx, conflict)?;
        }
        write_snapshot(&tx, &candidate, snapshot_time)?;
        let mut removed = 0;
        for txn in acknowledged {
            removed += tx.execute(
                "DELETE FROM pending_txns WHERE txn_id=?1 AND txn=?2",
                params![uuid(txn.txn_id.as_ref())?, bounded_encode(txn)?],
            )?;
        }
        verify_prefix(&tx, &candidate, seq)?;
        super::projections::verify_assets(&tx, candidate.project())?;
        super::projections::verify(&tx, candidate.project(), seq)?;
        validate_pending(&tx, candidate.project())?;
        integrity(&tx)?;
        observer(CommitStage::BeforeCommit);
        tx.commit()?;
        self.host = candidate;
        self.last_snapshot_seq = revision.host_seq;
        self.last_snapshot_ms = snapshot_time;
        self.replayed_transactions = 0;
        observer(CommitStage::Committed);
        Ok(removed)
    }
    /// Number of operations replayed after the newest checkpoint on this open.
    pub const fn replayed_transactions(&self) -> u64 {
        self.replayed_transactions
    }
    pub fn integrity_check(&self) -> Result<(), StoreError> {
        integrity(&self.connection)
    }

    /// Validate against the current authority without advancing it. Transport
    /// uses this before acquiring new originals; commit revalidates after I/O.
    pub fn validate_transaction(
        &self,
        txn: &v1::Transaction,
        authenticated: &DeviceId,
        accepted_at_ms: i64,
    ) -> Result<Acceptance, StoreError> {
        bounded_encode(txn)?;
        let mut candidate = self.host.clone();
        let acceptance = candidate.submit(txn.clone(), authenticated, accepted_at_ms)?;
        if candidate.project().canonical_bytes()?.len() > MAX_PAYLOAD {
            return Err(StoreError::Invalid("snapshot size"));
        }
        sql_u64(acceptance.ack.host_seq)?;
        sql_u64(
            txn.base_revision
                .as_ref()
                .ok_or(StoreError::Invalid("base revision"))?
                .host_seq,
        )?;
        for op in &txn.ops {
            sql_u64(
                op.op_id
                    .as_ref()
                    .ok_or(StoreError::Invalid("op ID"))?
                    .lamport,
            )?;
        }
        Ok(acceptance)
    }
    /// Persist one host acceptance, its exact ack, conflict records, projections
    /// and any due checkpoint in the same SQLite transaction. Rejections and
    /// SQL failures leave both the database and the in-memory host unchanged.
    pub fn commit(
        &mut self,
        txn: &v1::Transaction,
        authenticated: &DeviceId,
        accepted_at_ms: i64,
    ) -> Result<Acceptance, StoreError> {
        self.commit_observed(txn, authenticated, accepted_at_ms, |_| {})
    }
    pub fn commit_observed<F: FnMut(CommitStage)>(
        &mut self,
        txn: &v1::Transaction,
        authenticated: &DeviceId,
        accepted_at_ms: i64,
        mut observer: F,
    ) -> Result<Acceptance, StoreError> {
        let bytes = bounded_encode(txn)?;
        let mut candidate = self.host.clone();
        let acceptance = candidate.submit(txn.clone(), authenticated, accepted_at_ms)?;
        if acceptance.duplicate {
            return Ok(acceptance);
        }
        if candidate.project().canonical_bytes()?.len() > MAX_PAYLOAD {
            return Err(StoreError::Invalid("snapshot size"));
        }
        let seq = sql_u64(acceptance.ack.host_seq)?;
        let base = txn
            .base_revision
            .as_ref()
            .ok_or(StoreError::Invalid("base revision"))?;
        let base_seq = sql_u64(base.host_seq)?;
        for op in &txn.ops {
            sql_u64(
                op.op_id
                    .as_ref()
                    .ok_or(StoreError::Invalid("op ID"))?
                    .lamport,
            )?;
        }
        let snapshot_due = acceptance.ack.host_seq - self.last_snapshot_seq
            >= SNAPSHOT_TRANSACTIONS
            || accepted_at_ms.saturating_sub(self.last_snapshot_ms) >= SNAPSHOT_INTERVAL_MS;
        observer(CommitStage::Prepared);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        migrations::verify_schema(&tx, 2)?;
        tx.execute("INSERT INTO op_log(host_seq,txn_id,device_id,base_host_seq,gesture_id,txn,state_hash,created_at_wall,accepted_at,ack) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![seq,uuid(txn.txn_id.as_ref())?,authenticated.as_str(),base_seq,optional_uuid(txn.gesture_id.as_ref())?,bytes,candidate.project().state_hash()?.as_str(),txn.created_at_wall_ms,accepted_at_ms,acceptance.ack.encode_to_vec()])?;
        observer(CommitStage::LogWritten);
        super::projections::persist(&tx, candidate.project(), seq, accepted_at_ms)?;
        for conflict in &acceptance.conflicts {
            write_conflict(&tx, conflict)?;
        }
        if snapshot_due {
            write_snapshot(&tx, &candidate, accepted_at_ms)?;
        }
        observer(CommitStage::BeforeCommit);
        tx.commit()?;
        self.host = candidate;
        if snapshot_due {
            self.last_snapshot_seq = acceptance.ack.host_seq;
            self.last_snapshot_ms = accepted_at_ms;
        }
        observer(CommitStage::Committed);
        Ok(acceptance)
    }

    /// Explicit idle/save checkpoint; applications may call after five idle
    /// minutes. An accepted transaction always checks the interval itself.
    pub fn snapshot_now(&mut self, now_ms: i64) -> Result<(), StoreError> {
        if now_ms < 0 {
            return Err(StoreError::Invalid("snapshot time"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        write_snapshot(&tx, &self.host, now_ms)?;
        tx.commit()?;
        self.last_snapshot_seq = self.host.revision()?.host_seq;
        self.last_snapshot_ms = now_ms;
        Ok(())
    }
    /// Query durable acceptance/cancellation without cloning a checkpoint.
    pub fn gesture_closed(&self, device: &DeviceId, gesture: &Id) -> bool {
        self.host.gesture_closed(device, gesture)
    }

    /// Persist cancellation before accepting any later commit with this gesture.
    pub fn cancel_gesture(
        &mut self,
        device: DeviceId,
        gesture: Id,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        if now_ms < 0 {
            return Err(StoreError::Invalid("snapshot time"));
        }
        let mut host = self.host.clone();
        host.cancel_gesture(device, gesture)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        write_snapshot(&tx, &host, now_ms)?;
        tx.commit()?;
        self.host = host;
        self.last_snapshot_seq = self.host.revision()?.host_seq;
        self.last_snapshot_ms = now_ms;
        Ok(())
    }

    /// Queue exact offline bytes in stable local order. An identical retry is a
    /// no-op; reusing a transaction ID for other bytes fails instead of replacing.
    pub fn enqueue_pending(&mut self, txn: &v1::Transaction) -> Result<i64, StoreError> {
        validate_pending_txn(txn, self.project())?;
        let id = uuid(txn.txn_id.as_ref())?;
        if Id::from_proto(txn.project_id.as_ref())? != self.project().id
            || txn.ops.is_empty()
            || txn.created_at_wall_ms < 0
        {
            return Err(StoreError::Invalid("pending transaction"));
        }
        let device = DeviceId::try_from(txn.device_id.clone())?;
        let base = txn
            .base_revision
            .as_ref()
            .ok_or(StoreError::Invalid("pending base"))?;
        if base.state_hash.len() != 32 {
            return Err(StoreError::Invalid("pending hash"));
        }
        for op in &txn.ops {
            let op_id = op
                .op_id
                .as_ref()
                .ok_or(StoreError::Invalid("pending op ID"))?;
            if op_id.device_id != device.as_str() || op_id.lamport == 0 || op.kind.is_none() {
                return Err(StoreError::Invalid("pending op"));
            }
            sql_u64(op_id.lamport)?;
        }
        let bytes = bounded_encode(txn)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(i64, Vec<u8>)> = tx
            .query_row(
                "SELECT local_seq,txn FROM pending_txns WHERE txn_id=?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((seq, previous)) = existing {
            if previous != bytes {
                return Err(StoreError::Invalid("pending ID reuse"));
            }
            return Ok(seq);
        }
        for op in &txn.ops {
            if let Some(v1::op::Kind::AddAsset(asset)) = &op.kind {
                crate::projections::persist_asset(&tx, asset)?;
            }
        }
        tx.execute("INSERT INTO pending_txns(txn_id,base_host_seq,txn,created_at_wall) VALUES(?1,?2,?3,?4)",params![id,sql_u64(base.host_seq)?,bytes,txn.created_at_wall_ms])?;
        let seq = tx.last_insert_rowid();
        tx.commit()?;
        Ok(seq)
    }
    pub fn pending(&self) -> Result<Vec<v1::Transaction>, StoreError> {
        validate_pending(&self.connection, self.project())
    }
    /// Remove only the exact queued transaction already present with this exact
    /// receipt in the durable accepted log. A network receipt by itself cannot
    /// remove the only durable copy of an offline edit.
    pub fn acknowledge_pending(
        &mut self,
        txn: &v1::Transaction,
        ack: &v1::TxnAck,
    ) -> Result<bool, StoreError> {
        if ack.txn_id != txn.txn_id || ack.host_seq == 0 || ack.state_hash.len() != 32 {
            return Err(StoreError::Invalid("pending ack"));
        }
        let id = uuid(txn.txn_id.as_ref())?;
        let bytes = bounded_encode(txn)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        migrations::verify_schema(&tx, 2)?;
        let durable: Option<(Vec<u8>, Vec<u8>)> = tx
            .query_row("SELECT txn,ack FROM op_log WHERE txn_id=?1", [&id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?;
        let Some((accepted, receipt)) = durable else {
            return Err(StoreError::Invalid("pending ack has no durable acceptance"));
        };
        if accepted != bytes {
            return Ok(false);
        }
        if receipt != bounded_encode(ack)? {
            return Err(StoreError::Invalid(
                "pending ack differs from durable receipt",
            ));
        }
        let removed = tx.execute(
            "DELETE FROM pending_txns WHERE txn_id=?1 AND txn=?2",
            params![id, bytes],
        )? == 1;
        tx.commit()?;
        Ok(removed)
    }
    pub fn conflicts(&self) -> Result<Vec<v1::ConflictRecord>, StoreError> {
        read_conflicts(&self.connection)
    }
    /// Device labels are local project conveniences, never pairing credentials.
    pub fn register_device(
        &mut self,
        id: &DeviceId,
        label: Option<&str>,
        platform: &str,
        lamport: u64,
    ) -> Result<(), StoreError> {
        if !matches!(platform, "windows" | "android" | "ios")
            || label.is_some_and(|v| v.len() > 1024)
        {
            return Err(StoreError::Invalid("device metadata"));
        }
        self.connection.execute("INSERT INTO devices(device_id,label,platform,lamport) VALUES(?1,?2,?3,?4) ON CONFLICT(device_id) DO UPDATE SET label=excluded.label,platform=excluded.platform,lamport=MAX(devices.lamport,excluded.lamport)",params![id.as_str(),label,platform,sql_u64(lamport)?])?;
        Ok(())
    }
}

fn lock_project(root: &Path) -> Result<File, StoreError> {
    let path = root.join(".store.lock");
    match fs::symlink_metadata(&path) {
        Ok(m) => blobs::check_regular_metadata(&m)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000);
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x0002_0000);
    }
    let file = options.open(path)?;
    blobs::check_regular_metadata(&file.metadata()?)?;
    fs2::FileExt::try_lock_exclusive(&file).map_err(|e| {
        if e.kind() == std::io::ErrorKind::WouldBlock
            || e.raw_os_error() == fs2::lock_contended_error().raw_os_error()
        {
            StoreError::Busy
        } else {
            StoreError::Io(e)
        }
    })?;
    Ok(file)
}
fn connect(root: &Path, create: bool) -> Result<Connection, StoreError> {
    for name in [
        "project.sqlite",
        "project.sqlite-wal",
        "project.sqlite-shm",
        "project.sqlite-journal",
    ] {
        match fs::symlink_metadata(root.join(name)) {
            Ok(m) => blobs::check_regular_metadata(&m)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    if create {
        flags |= OpenFlags::SQLITE_OPEN_CREATE;
    }
    let connection = Connection::open_with_flags(root.join("project.sqlite"), flags)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
    Ok(connection)
}
pub(crate) fn sql_u64(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::Invalid("SQLite integer range"))
}
pub(crate) fn uuid(value: Option<&v1::Uuid>) -> Result<String, StoreError> {
    Ok(Id::from_proto(value)?.to_string())
}
pub(crate) fn optional_uuid(value: Option<&v1::Uuid>) -> Result<Option<String>, StoreError> {
    value.map(|v| uuid(Some(v))).transpose()
}
pub(crate) fn decode<T: Message + Default>(bytes: &[u8]) -> Result<T, StoreError> {
    if bytes.len() > MAX_PAYLOAD {
        return Err(StoreError::Corrupt("payload size"));
    }
    T::decode(bytes).map_err(|_| StoreError::Corrupt("protobuf payload"))
}
fn bounded_encode<T: Message>(message: &T) -> Result<Vec<u8>, StoreError> {
    if message.encoded_len() > MAX_PAYLOAD {
        return Err(StoreError::Invalid("payload size"));
    }
    Ok(message.encode_to_vec())
}
pub(crate) fn write_snapshot(
    connection: &Connection,
    host: &HostSequencer,
    now_ms: i64,
) -> Result<(), StoreError> {
    let checkpoint = host.checkpoint_bytes()?;
    let state = host.project().canonical_bytes()?;
    if state.len() > MAX_PAYLOAD {
        return Err(StoreError::Invalid("snapshot size"));
    }
    connection.execute("INSERT INTO snapshots(host_seq,state,state_hash,created_at,checkpoint,checkpoint_hash) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(host_seq) DO UPDATE SET state=excluded.state,state_hash=excluded.state_hash,created_at=excluded.created_at,checkpoint=excluded.checkpoint,checkpoint_hash=excluded.checkpoint_hash",
        params![sql_u64(host.revision()?.host_seq)?,state,host.project().state_hash()?.as_str(),now_ms,&checkpoint,AssetId::hash(&checkpoint).as_str()])?;
    Ok(())
}
pub(crate) fn replay_log(
    connection: &Connection,
    host: &mut HostSequencer,
    after: i64,
    verify_ack: bool,
) -> Result<u64, StoreError> {
    let mut query=connection.prepare("SELECT host_seq,txn_id,device_id,base_host_seq,gesture_id,txn,state_hash,created_at_wall,accepted_at,ack FROM op_log WHERE host_seq>?1 ORDER BY host_seq")?;
    let mut rows = query.query([after])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        let seq: i64 = row.get(0)?;
        let bytes: Vec<u8> = row.get(5)?;
        let txn: v1::Transaction = decode(&bytes)?;
        verify_log_columns(row, &txn)?;
        let device = DeviceId::try_from(txn.device_id.clone())?;
        let accepted = host.submit(txn, &device, row.get(8)?)?;
        if accepted.duplicate
            || sql_u64(accepted.ack.host_seq)? != seq
            || host.project().state_hash()?.as_str() != row.get::<_, String>(6)?
        {
            return Err(StoreError::Corrupt("operation replay"));
        }
        if verify_ack {
            let ack: Vec<u8> = row.get(9)?;
            if ack != accepted.ack.encode_to_vec() {
                return Err(StoreError::Corrupt("operation ack"));
            }
        } else {
            connection.execute(
                "UPDATE op_log SET ack=?1 WHERE host_seq=?2",
                params![accepted.ack.encode_to_vec(), seq],
            )?;
        }
        count += 1;
    }
    Ok(count)
}
fn verify_log_columns(row: &rusqlite::Row<'_>, txn: &v1::Transaction) -> Result<(), StoreError> {
    if uuid(txn.txn_id.as_ref())? != row.get::<_, String>(1)?
        || txn.device_id != row.get::<_, String>(2)?
        || sql_u64(
            txn.base_revision
                .as_ref()
                .ok_or(StoreError::Corrupt("log base"))?
                .host_seq,
        )? != row.get::<_, i64>(3)?
        || optional_uuid(txn.gesture_id.as_ref())? != row.get::<_, Option<String>>(4)?
        || txn.created_at_wall_ms != row.get::<_, i64>(7)?
    {
        return Err(StoreError::Corrupt("log metadata"));
    }
    Ok(())
}
fn verify_prefix(
    connection: &Connection,
    host: &HostSequencer,
    through: i64,
) -> Result<(), StoreError> {
    let mut query=connection.prepare("SELECT host_seq,txn_id,device_id,base_host_seq,gesture_id,txn,state_hash,created_at_wall,accepted_at,ack FROM op_log WHERE host_seq<=?1 ORDER BY host_seq")?;
    let mut rows = query.query([through])?;
    let mut next = 1;
    while let Some(row) = rows.next()? {
        let seq: i64 = row.get(0)?;
        let bytes: Vec<u8> = row.get(5)?;
        let ack: Vec<u8> = row.get(9)?;
        let txn: v1::Transaction = decode(&bytes)?;
        verify_log_columns(row, &txn)?;
        let id = Id::from_proto(txn.txn_id.as_ref())?;
        let (expected, expected_ack) = host
            .accepted_transaction(&id)
            .ok_or(StoreError::Corrupt("checkpoint log binding"))?;
        let hash = AssetId::try_from(row.get::<_, String>(6)?)?;
        if seq != next
            || expected.encode_to_vec() != bytes
            || expected_ack.encode_to_vec() != ack
            || host.accepted_at(&id) != Some(row.get::<_, i64>(8)?)
            || expected_ack.host_seq != seq as u64
            || expected_ack.state_hash != hash.bytes()
        {
            return Err(StoreError::Corrupt("checkpoint log binding"));
        }
        next += 1;
    }
    if next - 1 != through {
        return Err(StoreError::Corrupt("log gap"));
    }
    Ok(())
}
fn check_payload_bounds(connection: &Connection) -> Result<(), StoreError> {
    for sql in [
        "SELECT COUNT(*) FROM snapshots WHERE length(state)>67108864 OR length(checkpoint)>268435456",
        "SELECT COUNT(*) FROM op_log WHERE length(txn)>67108864 OR length(ack)>67108864",
        "SELECT COUNT(*) FROM pending_txns WHERE length(txn)>67108864",
        "SELECT COUNT(*) FROM conflicts WHERE length(record)>67108864",
    ] {
        if connection.query_row(sql, [], |r| r.get::<_, i64>(0))? != 0 {
            return Err(StoreError::Corrupt("payload size"));
        }
    }
    Ok(())
}

fn validate_pending_txn(txn: &v1::Transaction, project: &Project) -> Result<(), StoreError> {
    uuid(txn.txn_id.as_ref())?;
    optional_uuid(txn.gesture_id.as_ref())?;
    let device = DeviceId::try_from(txn.device_id.clone())?;
    if Id::from_proto(txn.project_id.as_ref())? != project.id
        || txn.ops.is_empty()
        || txn.ops.len() > 4096
        || txn.created_at_wall_ms < 0
    {
        return Err(StoreError::Invalid("pending transaction"));
    }
    let base = txn
        .base_revision
        .as_ref()
        .ok_or(StoreError::Invalid("pending base"))?;
    sql_u64(base.host_seq)?;
    if base.state_hash.len() != 32 {
        return Err(StoreError::Invalid("pending hash"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for op in &txn.ops {
        let id = op
            .op_id
            .as_ref()
            .ok_or(StoreError::Invalid("pending op ID"))?;
        if id.device_id != device.as_str()
            || id.lamport == 0
            || !seen.insert(id.lamport)
            || op.kind.is_none()
        {
            return Err(StoreError::Invalid("pending op"));
        }
        sql_u64(id.lamport)?;
    }
    Ok(())
}
pub(crate) fn validate_pending(
    connection: &Connection,
    project: &Project,
) -> Result<Vec<v1::Transaction>, StoreError> {
    if connection.query_row(
        "SELECT COUNT(*) FROM pending_txns WHERE length(txn)>67108864",
        [],
        |r| r.get::<_, i64>(0),
    )? != 0
    {
        return Err(StoreError::Corrupt("payload size"));
    }
    let mut query=connection.prepare("SELECT local_seq,txn_id,base_host_seq,created_at_wall,txn FROM pending_txns ORDER BY local_seq")?;
    let mut rows = query.query([])?;
    let mut pending = Vec::new();
    while let Some(row) = rows.next()? {
        let txn: v1::Transaction = decode(&row.get::<_, Vec<u8>>(4)?)?;
        validate_pending_txn(&txn, project)?;
        if row.get::<_, i64>(0)? <= 0
            || uuid(txn.txn_id.as_ref())? != row.get::<_, String>(1)?
            || sql_u64(
                txn.base_revision
                    .as_ref()
                    .ok_or(StoreError::Invalid("pending base"))?
                    .host_seq,
            )? != row.get::<_, i64>(2)?
            || txn.created_at_wall_ms != row.get::<_, i64>(3)?
        {
            return Err(StoreError::Corrupt("pending metadata"));
        }
        pending.push(txn);
    }
    Ok(pending)
}
pub(crate) fn integrity(connection: &Connection) -> Result<(), StoreError> {
    let mut query = connection.prepare("PRAGMA integrity_check")?;
    let values = query
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if values != ["ok"] {
        return Err(StoreError::Corrupt("SQLite integrity"));
    }
    if connection
        .prepare("PRAGMA foreign_key_check")?
        .query([])?
        .next()?
        .is_some()
    {
        return Err(StoreError::Corrupt("SQLite foreign key"));
    }
    Ok(())
}
pub(crate) fn write_conflict(
    connection: &Connection,
    record: &v1::ConflictRecord,
) -> Result<(), StoreError> {
    let encoded = bounded_encode(record)?;
    connection.execute("INSERT INTO conflicts(conflict_id,object_id,property,kept_value,other_value,kept_device,other_device,delete_vs_edit,edited_object,created_at,record) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![uuid(record.conflict_id.as_ref())?,uuid(record.object_id.as_ref())?,record.property,record.kept_value.as_ref().map(Message::encode_to_vec),record.other_value.as_ref().map(Message::encode_to_vec),record.kept_device,record.other_device,record.delete_vs_edit,record.edited_object.as_ref().map(Message::encode_to_vec),record.created_at_ms,encoded])?;
    Ok(())
}
fn read_conflicts(connection: &Connection) -> Result<Vec<v1::ConflictRecord>, StoreError> {
    // Insertion rowid preserves host acceptance order, even for tied timestamps.
    let mut query=connection.prepare("SELECT record,conflict_id,object_id,property,kept_value,other_value,kept_device,other_device,delete_vs_edit,edited_object,created_at FROM conflicts ORDER BY rowid")?;
    let mut rows = query.query([])?;
    let mut result = Vec::new();
    while let Some(row) = rows.next()? {
        let record: v1::ConflictRecord = decode(&row.get::<_, Vec<u8>>(0)?)?;
        if uuid(record.conflict_id.as_ref())? != row.get::<_, String>(1)?
            || uuid(record.object_id.as_ref())? != row.get::<_, String>(2)?
            || record.property != row.get::<_, String>(3)?
            || record.kept_value.as_ref().map(Message::encode_to_vec)
                != row.get::<_, Option<Vec<u8>>>(4)?
            || record.other_value.as_ref().map(Message::encode_to_vec)
                != row.get::<_, Option<Vec<u8>>>(5)?
            || record.kept_device != row.get::<_, String>(6)?
            || record.other_device != row.get::<_, String>(7)?
            || record.delete_vs_edit != row.get::<_, bool>(8)?
            || record.edited_object.as_ref().map(Message::encode_to_vec)
                != row.get::<_, Option<Vec<u8>>>(9)?
            || record.created_at_ms != row.get::<_, i64>(10)?
        {
            return Err(StoreError::Corrupt("conflict metadata"));
        }
        result.push(record);
    }
    Ok(result)
}

/// Called only after exact schema validation. SQL length checks precede loading
/// any attacker-controlled projection/payload column into a Rust allocation.
pub(crate) fn check_all_column_bounds(db: &Connection) -> Result<(), StoreError> {
    let tables = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT GLOB 'sqlite_*'")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for table in tables {
        let fields = db
            .prepare(&format!("PRAGMA table_info({table})"))?
            .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        let checks = fields
            .into_iter()
            .map(|(field, _)| {
                let bound = if table == "snapshots" && field == "checkpoint" {
                    268_435_456
                } else {
                    67_108_864
                };
                format!("length(CAST({field} AS BLOB))>{bound}")
            })
            .collect::<Vec<_>>();
        if !checks.is_empty()
            && db.query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {}", checks.join(" OR ")),
                [],
                |r| r.get::<_, i64>(0),
            )? != 0
        {
            return Err(StoreError::Corrupt("payload size"));
        }
    }
    Ok(())
}
