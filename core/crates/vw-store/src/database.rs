use crate::{StoreError, blobs, migrations};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use vw_model::{AssetId, DeviceId, Id, Project};
use vw_ops::{Acceptance, HostSequencer, OpsError, Touched};
use vw_proto::{Message, v1};

pub(crate) const MAX_PAYLOAD: usize = 64 * 1024 * 1024;
/// No checkpoint is written for less replay than this, in the microseconds
/// of `ReplayCost`: a tenth of a second of opening.
const CHECKPOINT_MIN_MICROS: u64 = 100_000;
/// A checkpoint is urgent from this much replay on: five seconds of opening.
const CHECKPOINT_URGENT_MICROS: u64 = 5_000_000;
/// Unused pages are handed back to the filesystem once they make up this much
/// of the database and at least a quarter of it.
const RECLAIM_MIN_BYTES: i64 = 4 * 1024 * 1024;
/// Compacting copies the live pages once. Up to this size the copy is held in
/// memory, because not every platform gives SQLite a temporary directory.
const RECLAIM_IN_MEMORY_BYTES: i64 = 256 * 1024 * 1024;
/// How long a write waits for another connection.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// The same while an open tidies the file: a connected reader makes a step
/// fail, to be tried at a later open, instead of holding the open up.
const UPKEEP_BUSY_TIMEOUT: Duration = Duration::from_millis(250);

/// Diagnostic commit boundaries. Observers must return promptly and never panic.
/// The process-crash harness kills its own writer at randomized boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitStage {
    Prepared,
    LogWritten,
    BeforeCommit,
    Committed,
}

/// Diagnostic boundaries of the clean-up an open performs on a project that
/// still carries superseded checkpoints or unused pages. Observers have the
/// same no-panic/no-blocking contract as commit observers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenStage {
    /// The project is verified; nothing has been written.
    Restored,
    /// Superseded checkpoints are removed inside the uncommitted transaction.
    Pruned,
    Committed,
    /// A compaction is about to start; the attempt is noted in the database.
    Compacting,
    /// The compaction is copying rows. Nothing of it is committed.
    Copying,
    /// The compacted pages are committed to the write-ahead log.
    Vacuumed,
    /// They are in the database file; the log is about to be emptied.
    Transferred,
    /// Unused pages were handed back, or there were too few to bother.
    Compacted,
}

/// What an open did besides loading the project, for diagnostics: counts and
/// durations, and for a step that failed the error it failed with.
#[derive(Debug, Default)]
pub struct OpenReport {
    /// Stored checkpoints that did not load and were passed over.
    pub skipped_checkpoints: u64,
    /// The clean-up transaction: superseded checkpoints, cancellation
    /// records and the log seal.
    pub tidy: Upkeeping,
    /// A checkpoint written because this open had a costly log to replay.
    pub checkpoint: Upkeeping,
    /// Handing unused pages back to the filesystem.
    pub compaction: Upkeeping,
}

/// How one step of an open's upkeep went. None of them can fail the open.
#[derive(Debug, Default)]
pub enum Upkeeping {
    /// There was nothing to do.
    #[default]
    NotDue,
    Done(Duration),
    /// The project is open and the step changed nothing; a later open tries
    /// again.
    Failed(Duration, StoreError),
    /// Not attempted: an earlier step failed, or this one did not finish
    /// before and nothing has changed since.
    Skipped,
}
impl Upkeeping {
    fn of(started: Instant, result: Result<bool, StoreError>) -> Self {
        match result {
            Ok(true) => Self::Done(started.elapsed()),
            Ok(false) => Self::NotDue,
            Err(error) => Self::Failed(started.elapsed(), error),
        }
    }
}

/// How much the next open would gain from a checkpoint written now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CheckpointNeed {
    /// Replaying the log costs less than loading a checkpoint would.
    None,
    /// A checkpoint would shorten the next open: write it when the project
    /// closes.
    Due,
    /// Replaying would at least double the next open: write it when the
    /// project has been left alone for a while.
    Overdue,
    /// Replaying would take at least five seconds and twice as long as the
    /// rest of the open: write it at the next moment nothing else is waiting.
    Urgent,
}

/// A single-writer project database. Its advisory lock is released by the OS on
/// process death; WAL readers are allowed, but all writers must use this API.
///
/// The accepted log is the record of the project. A checkpoint holds the
/// sequencing state at one revision, so that opening can start there and
/// replay only the log after it. At most two are stored. The first is the one
/// the project began with: at revision zero it is small, it is never removed,
/// and gesture cancellations, the one thing the log cannot rebuild, are
/// recorded in it; each cancellation also has a row of its own, in case that
/// one row is ever damaged. A later checkpoint is written when replaying the
/// log has become more expensive than loading one. Saving never writes it:
/// this store does when a project opens, and the application asks for it
/// while the project is idle and when it closes (`checkpoint_if`). Before a
/// new one replaces the one before it, it is read back.
///
/// Opening decides again every transaction stored after the checkpoint it
/// starts from, with the engine of the build that opens. What that requires
/// of the engine is stated at `evaluate` in `vw-ops`, and what happens when
/// it does not hold at `restore` here.
pub struct ProjectStore {
    pub(crate) root: PathBuf,
    pub(crate) connection: Connection,
    host: HostSequencer,
    _lock: ProjectLock,
    /// Revision of the checkpoint that cancellations are recorded in.
    ledger_seq: u64,
    /// What the next open would replay as things stand.
    cost: ReplayCost,
    /// The size of the whole log.
    log: LogSpan,
    seal: LogSeal,
    replayed_transactions: u64,
    opened: OpenReport,
    /// The history has outgrown what a checkpoint may hold. Writing one was
    /// tried and refused; it is not tried again while the project is open.
    beyond_checkpoint: bool,
}

/// The advisory lock, given up explicitly. A lock belongs to the open file
/// and not to this handle: a child process forked while the project was open
/// keeps the file open until it starts its own program, and would keep the
/// lock alive past the close without the explicit release.
struct ProjectLock(File);
impl Drop for ProjectLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

impl ProjectStore {
    /// Canonical root selected when this locked project was opened. This is a
    /// read-only capability hint, not permission to bypass blob/cache APIs.
    pub fn root_path(&self) -> &Path {
        &self.root
    }

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
        let mut seal = LogSeal::empty();
        let mut log = LogSpan::default();
        for (txn, ack) in accepted {
            let stored;
            (seal, stored) = insert_accepted(&tx, &host, txn, ack, &seal)?;
            log.add(stored);
        }
        super::projections::persist(&tx, host.project(), seq, now_ms)?;
        for conflict in host.conflicts() {
            write_conflict(&tx, conflict)?;
        }
        write_snapshot(&tx, &host, now_ms)?;
        write_cancellations(&tx, &host)?;
        write_seal(&tx, &seal)?;
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
            ledger_seq: revision.host_seq,
            cost: ReplayCost::default(),
            log,
            seal,
            replayed_transactions: 0,
            opened: OpenReport::default(),
            beyond_checkpoint: false,
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
        let seal = LogSeal::empty();
        write_seal(&tx, &seal)?;
        tx.commit()?;
        blobs::ensure_directory(&root.join("blobs"))?;
        blobs::ensure_directory(&root.join("cache"))?;
        Ok(Self {
            root,
            connection,
            host,
            _lock: lock,
            ledger_seq: 0,
            cost: ReplayCost::default(),
            log: LogSpan::default(),
            seal,
            replayed_transactions: 0,
            opened: OpenReport::default(),
            beyond_checkpoint: false,
        })
    }

    /// Verify the newest checkpoint that loads and replay the accepted log
    /// after it. Unsupported schemas, a damaged log and damaged projections
    /// are rejected without repair, and so is a log this build's engine does
    /// not decide the way it was stored (`StoreError::EngineMismatch`); a
    /// refused project is left exactly as it was. A damaged checkpoint is
    /// skipped when an earlier one and the log give the same project. A
    /// project written by an older build carries a checkpoint for every 500
    /// saves: once the project is verified, all but the two this store keeps
    /// are removed and their space is handed back. `open_report` says what
    /// of this happened.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::open_observed(path, |_| {})
    }

    /// Diagnostic boundaries of the clean-up for the process-crash regression.
    /// One of them is reported from inside SQLite while it compacts, which is
    /// why the observer has to be owned and sendable.
    pub fn open_observed<F: FnMut(OpenStage) + Send + 'static>(
        path: &Path,
        observer: F,
    ) -> Result<Self, StoreError> {
        let observer = Arc::new(Mutex::new(observer));
        let mut observer = move |stage: OpenStage| {
            if let Ok(mut observer) = observer.lock() {
                observer(stage);
            }
        };
        blobs::check_directory(path)?;
        let root = path.canonicalize()?;
        let lock = lock_project(&root)?;
        let mut connection = connect(&root, false)?;
        migrations::upgrade(&mut connection)?;
        let Restored {
            host,
            replayed,
            cost,
            log,
            seal,
            upkeep,
            skipped,
        } = restore(&connection)?;
        observer(OpenStage::Restored);
        let mut store = Self {
            root,
            connection,
            host,
            _lock: lock,
            ledger_seq: upkeep.keep[0] as u64,
            cost,
            log,
            seal,
            replayed_transactions: replayed,
            opened: OpenReport {
                skipped_checkpoints: skipped,
                ..OpenReport::default()
            },
            beyond_checkpoint: false,
        };
        store.upkeep(&upkeep, &mut observer);
        observer(OpenStage::Compacted);
        Ok(store)
    }

    /// The project is verified and complete in memory. This only tidies the
    /// file: one transaction, a checkpoint if this open had to replay a lot,
    /// and one compaction, each all or nothing. If the disk refuses one (it
    /// is full, a reader holds the file), the project is open as it is and a
    /// later open tries again: an open has never needed room to write. What
    /// happened is kept in the report.
    fn upkeep<F: FnMut(OpenStage) + Clone + Send + 'static>(
        &mut self,
        upkeep: &Upkeep,
        observer: &mut F,
    ) {
        if upkeep.unsettled {
            self.opened.tidy = Upkeeping::Skipped;
            self.opened.checkpoint = Upkeeping::Skipped;
            self.opened.compaction = Upkeeping::Skipped;
            return;
        }
        let _ = self.connection.busy_timeout(UPKEEP_BUSY_TIMEOUT);
        let started = Instant::now();
        self.opened.tidy = Upkeeping::of(
            started,
            tidy(
                &mut self.connection,
                &self.host,
                &self.seal,
                upkeep,
                observer,
            ),
        );
        if matches!(self.opened.tidy, Upkeeping::Failed(..)) {
            self.opened.checkpoint = Upkeeping::Skipped;
            self.opened.compaction = Upkeeping::Skipped;
        } else {
            let started = Instant::now();
            let due = self.checkpoint_need() >= CheckpointNeed::Due;
            self.opened.checkpoint = Upkeeping::of(
                started,
                if due {
                    self.write_checkpoint(None, |_| {}).map(|()| true)
                } else {
                    Ok(false)
                },
            );
            let started = Instant::now();
            self.opened.compaction = match reclaim(&mut self.connection, observer) {
                Ok(Reclaimed::Skipped) => Upkeeping::Skipped,
                Ok(Reclaimed::NotDue) => Upkeeping::NotDue,
                Ok(Reclaimed::Done) => Upkeeping::Done(started.elapsed()),
                Err(error) => Upkeeping::Failed(started.elapsed(), error),
            };
        }
        let _ = self.connection.busy_timeout(BUSY_TIMEOUT);
    }

    /// What this open did besides loading the project.
    pub const fn open_report(&self) -> &OpenReport {
        &self.opened
    }

    pub fn project(&self) -> &Project {
        self.host.project()
    }
    pub fn revision(&self) -> Result<v1::Revision, StoreError> {
        Ok(self.host.revision()?)
    }
    /// The canonical state hash of the current revision, already known from
    /// the last commit; nothing is serialized or hashed to answer this.
    pub fn state_hash(&self) -> Result<&vw_model::StateHash, StoreError> {
        Ok(self.host.state_hash()?)
    }
    /// Validated complete sequencing state for authenticated transport resume.
    pub fn checkpoint_bytes(&self) -> Result<Vec<u8>, StoreError> {
        Ok(self.host.checkpoint_bytes()?)
    }

    /// Read-only canonical workspace admission, with no checkpoint allocation.
    /// Includes private retained sequencing history; returns limit+1 on overflow.
    pub fn workspace_estimate_bytes(&self, limit: u64) -> Result<u64, StoreError> {
        Ok(self.host.workspace_estimate_bytes(limit)?)
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

    /// One accepted transaction and its acknowledgement, by transaction ID.
    pub fn accepted_transaction(&self, id: &Id) -> Option<(&v1::Transaction, &v1::TxnAck)> {
        self.host.accepted_transaction(id)
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
    /// divergent histories cannot replace local durable state. What is stored
    /// is the checkpoint's new log rows; a stored checkpoint changes only to
    /// take in cancellations the host has recorded since.
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
        observer(CommitStage::Prepared);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        migrations::verify_schema(&tx, migrations::CURRENT_VERSION)?;
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
        let mut seal = self.seal.clone();
        let (mut cost, mut log) = (self.cost, self.log);
        for (txn, ack) in ordered
            .into_iter()
            .filter(|(_, ack)| ack.host_seq > previous.host_seq)
        {
            let stored;
            (seal, stored) = insert_accepted(&tx, &candidate, txn, ack, &seal)?;
            // Not replayed here: the next open counts what these rows cost.
            cost.span.add(stored);
            log.add(stored);
        }
        write_seal(&tx, &seal)?;
        observer(CommitStage::LogWritten);
        super::projections::persist(&tx, candidate.project(), seq, now_ms)?;
        for conflict in &candidate.conflicts()[self.host.conflicts().len()..] {
            write_conflict(&tx, conflict)?;
        }
        if !candidate.cancellations_are_subset_of(&self.host) {
            let mut ledger = load_checkpoint(&tx, sql_u64(self.ledger_seq)?)?;
            carry_cancellations(&mut ledger, &candidate)?;
            write_snapshot(&tx, &ledger, now_ms)?;
            write_cancellations(&tx, &candidate)?;
        }
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
        self.seal = seal;
        (self.cost, self.log) = (cost, log);
        observer(CommitStage::Committed);
        Ok(removed)
    }
    /// Number of accepted operations replayed from the log on this open.
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
        let (acceptance, canonical_len) =
            self.host
                .check(txn.clone(), authenticated, accepted_at_ms)?;
        if canonical_len > MAX_PAYLOAD {
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
    /// Persist one host acceptance, its exact ack, conflict records and
    /// projections in the same SQLite transaction. The work follows what the
    /// transaction changes, never the size of the project or its history.
    /// Rejections and SQL failures leave both the database and the in-memory
    /// host unchanged.
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
        let stored = bytes.len();
        let Self {
            host,
            connection,
            seal,
            cost,
            log,
            ..
        } = self;
        // The host accepts the transaction in place and takes it back out if
        // anything below fails, so neither side ever holds a half commit.
        let (acceptance, persisted) = host.submit_with(
            txn.clone(),
            authenticated,
            accepted_at_ms,
            |host, acceptance, touched| -> Result<(LogSeal, ReplayCost), StoreError> {
                if host.canonical_len() > MAX_PAYLOAD {
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
                let hash = host.state_hash()?.as_str();
                let ack = acceptance.ack.encode_to_vec();
                let sealed = seal.with_row(seq, &bytes, hash, accepted_at_ms, &ack);
                let mut replay = *cost;
                replay.add(host, touched, stored);
                observer(CommitStage::Prepared);
                let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                migrations::verify_schema(&tx, migrations::CURRENT_VERSION)?;
                tx.prepare("INSERT INTO op_log(host_seq,txn_id,device_id,base_host_seq,gesture_id,txn,state_hash,created_at_wall,accepted_at,ack) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)")?
                    .execute(params![seq,uuid(txn.txn_id.as_ref())?,authenticated.as_str(),base_seq,optional_uuid(txn.gesture_id.as_ref())?,bytes,hash,txn.created_at_wall_ms,accepted_at_ms,ack])?;
                write_seal(&tx, &sealed)?;
                observer(CommitStage::LogWritten);
                super::projections::persist_touched(
                    &tx,
                    host.project(),
                    touched,
                    seq,
                    accepted_at_ms,
                )?;
                for conflict in &acceptance.conflicts {
                    write_conflict(&tx, conflict)?;
                }
                observer(CommitStage::BeforeCommit);
                tx.commit()?;
                Ok((sealed, replay))
            },
        )?;
        // A retry of an accepted transaction persisted nothing.
        if let Some((sealed, replay)) = persisted {
            *seal = sealed;
            *cost = replay;
            log.add(stored);
            observer(CommitStage::Committed);
        }
        Ok(acceptance)
    }

    /// How much the next open would gain from a checkpoint written now. It
    /// is `None` for a log of new strokes however long: replaying them is
    /// cheaper than loading a checkpoint that holds them.
    pub fn checkpoint_need(&self) -> CheckpointNeed {
        if self.beyond_checkpoint {
            return CheckpointNeed::None;
        }
        checkpoint_need(&self.cost, &self.log)
    }

    /// Write a checkpoint at the current revision if one is needed at least
    /// as much as `need` says. It rewrites the whole sequencing state and
    /// reads it back, which takes as long as the history is large, so no
    /// save does it. This store writes a due one when a project opens;
    /// applications ask for a due one before they close a project, for an
    /// overdue one when the project is idle, and for an urgent one between
    /// two pieces of work. Returns whether one was written.
    pub fn checkpoint_if(&mut self, need: CheckpointNeed, now_ms: i64) -> Result<bool, StoreError> {
        if now_ms < 0 {
            return Err(StoreError::Invalid("checkpoint time"));
        }
        if need == CheckpointNeed::None || self.checkpoint_need() < need {
            return Ok(false);
        }
        self.write_checkpoint(Some(now_ms), |_| {})?;
        Ok(true)
    }

    /// `checkpoint_if` for a checkpoint that is due.
    pub fn checkpoint_if_due(&mut self, now_ms: i64) -> Result<bool, StoreError> {
        self.checkpoint_if(CheckpointNeed::Due, now_ms)
    }

    /// Write a checkpoint at the current revision whether or not one is due.
    /// Prefer `checkpoint_if_due`: for a log of strokes, opening from a
    /// checkpoint is slower than replaying them.
    pub fn checkpoint_now(&mut self, now_ms: i64) -> Result<(), StoreError> {
        self.checkpoint_now_observed(now_ms, |_| {})
    }

    /// Diagnostic transaction boundaries for the process-crash regression.
    /// `LogWritten` here means the checkpoint is written, uncommitted.
    pub fn checkpoint_now_observed<F: FnMut(CommitStage)>(
        &mut self,
        now_ms: i64,
        observer: F,
    ) -> Result<(), StoreError> {
        if now_ms < 0 {
            return Err(StoreError::Invalid("checkpoint time"));
        }
        self.write_checkpoint(Some(now_ms), observer)
    }

    fn write_checkpoint<F: FnMut(CommitStage)>(
        &mut self,
        now_ms: Option<i64>,
        observer: F,
    ) -> Result<(), StoreError> {
        let written = self.try_write_checkpoint(now_ms, observer);
        // Serializing a history that large takes seconds every time, and the
        // history only grows: once refused for its size, do not ask again.
        if matches!(
            written,
            Err(StoreError::Operation(OpsError::Invalid("checkpoint size")))
        ) {
            self.beyond_checkpoint = true;
        }
        written
    }

    fn try_write_checkpoint<F: FnMut(CommitStage)>(
        &mut self,
        now_ms: Option<i64>,
        mut observer: F,
    ) -> Result<(), StoreError> {
        observer(CommitStage::Prepared);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        migrations::verify_schema(&tx, migrations::CURRENT_VERSION)?;
        let created = match now_ms {
            Some(now_ms) => now_ms,
            None => tx.query_row("SELECT MAX(created_at) FROM snapshots", [], |row| {
                row.get(0)
            })?,
        };
        // The revision-zero checkpoint stays: the whole log replays from it
        // should the new one ever fail to load, and cancellations go on being
        // recorded in it. Any other is replaced, and removed first so that
        // the new one takes over its pages.
        tx.execute(
            "DELETE FROM snapshots WHERE host_seq<>0 OR ?1<>0",
            [sql_u64(self.ledger_seq)?],
        )?;
        write_snapshot(&tx, &self.host, created)?;
        // Where no revision-zero checkpoint is kept (every mirror), the new
        // row is now the only way into the project. Read it back the way an
        // open will before the old one is gone for good: if it does not load
        // and bind to the log, the transaction is abandoned and the stored
        // checkpoints stay as they were.
        let seq = sql_u64(self.host.revision()?.host_seq)?;
        verify_prefix(&tx, &load_checkpoint(&tx, seq)?, seq)?;
        observer(CommitStage::LogWritten);
        observer(CommitStage::BeforeCommit);
        tx.commit()?;
        if self.ledger_seq != 0 {
            self.ledger_seq = self.host.revision()?.host_seq;
        }
        self.cost = ReplayCost::default();
        observer(CommitStage::Committed);
        Ok(())
    }
    /// Query durable acceptance/cancellation without cloning a checkpoint.
    pub fn gesture_closed(&self, device: &DeviceId, gesture: &Id) -> bool {
        self.host.gesture_closed(device, gesture)
    }

    /// Persist cancellation before accepting any later commit with this gesture.
    /// The log cannot rebuild a cancellation, so it goes into the project's
    /// first checkpoint, which every open reads.
    pub fn cancel_gesture(
        &mut self,
        device: DeviceId,
        gesture: Id,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        self.cancel_gesture_observed(device, gesture, now_ms, |_| {})
    }

    /// Diagnostic transaction boundaries for the process-crash regression.
    /// `LogWritten` here means the checkpoint is rewritten, uncommitted.
    pub fn cancel_gesture_observed<F: FnMut(CommitStage)>(
        &mut self,
        device: DeviceId,
        gesture: Id,
        now_ms: i64,
        mut observer: F,
    ) -> Result<(), StoreError> {
        if now_ms < 0 {
            return Err(StoreError::Invalid("cancellation time"));
        }
        // A committed gesture is refused and a cancelled one is already
        // durable; neither touches the host or the database.
        if self.host.gesture_closed(&device, &gesture) {
            return Ok(self.host.cancel_gesture(device, gesture)?);
        }
        observer(CommitStage::Prepared);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        migrations::verify_schema(&tx, migrations::CURRENT_VERSION)?;
        let mut ledger = load_checkpoint(&tx, sql_u64(self.ledger_seq)?)?;
        ledger.cancel_gesture(device.clone(), gesture.clone())?;
        write_snapshot(&tx, &ledger, now_ms)?;
        write_cancellation(&tx, &device, &gesture)?;
        observer(CommitStage::LogWritten);
        observer(CommitStage::BeforeCommit);
        tx.commit()?;
        // Cannot fail: the gesture was open a moment ago and nothing ran since.
        self.host.cancel_gesture(device, gesture)?;
        observer(CommitStage::Committed);
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
        migrations::verify_schema(&tx, migrations::CURRENT_VERSION)?;
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

fn lock_project(root: &Path) -> Result<ProjectLock, StoreError> {
    let path = root.join(".store.lock");
    match fs::symlink_metadata(&path) {
        Ok(m) => blobs::check_regular_metadata(&m)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    blobs::no_follow(&mut options);
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
    Ok(ProjectLock(file))
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
    connection.busy_timeout(BUSY_TIMEOUT)?;
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
        params![sql_u64(host.revision()?.host_seq)?,state,host.state_hash()?.as_str(),now_ms,&checkpoint,AssetId::hash(&checkpoint).as_str()])?;
    Ok(())
}

/// Read one checkpoint row and check everything it says about itself: its
/// digest, the canonical sequencing state, the separately stored project and
/// hash, its revision and the project it belongs to. What it says about the
/// log is checked by `verify_prefix`.
fn load_checkpoint(connection: &Connection, seq: i64) -> Result<HostSequencer, StoreError> {
    let (state, hash, time, checkpoint, digest): (Vec<u8>, String, i64, Vec<u8>, String) =
        connection.query_row(
            "SELECT state,state_hash,created_at,checkpoint,checkpoint_hash FROM snapshots WHERE host_seq=?1",
            [seq],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
    if seq < 0 || time < 0 || AssetId::hash(&checkpoint).as_str() != digest {
        return Err(StoreError::Corrupt("checkpoint binding"));
    }
    let host = HostSequencer::from_checkpoint_bytes(&checkpoint)?;
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
    Ok(host)
}

/// Store one accepted transaction of a verified checkpoint as a log row, with
/// any original it introduced. Returns the seal that includes the row and the
/// size of the stored transaction.
fn insert_accepted(
    connection: &Connection,
    host: &HostSequencer,
    txn: &v1::Transaction,
    ack: &v1::TxnAck,
    seal: &LogSeal,
) -> Result<(LogSeal, usize), StoreError> {
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
        // Preserve originals introduced then removed before this checkpoint.
        if let Some(v1::op::Kind::AddAsset(asset)) = &op.kind {
            super::projections::persist_asset(connection, asset)?;
        }
    }
    let hash: [u8; 32] = ack
        .state_hash
        .as_slice()
        .try_into()
        .map_err(|_| StoreError::Invalid("checkpoint receipt hash"))?;
    let hash = blake3::Hash::from_bytes(hash).to_hex().to_string();
    let seq = sql_u64(ack.host_seq)?;
    let (txn_bytes, ack_bytes) = (bounded_encode(txn)?, bounded_encode(ack)?);
    let sealed = seal.with_row(seq, &txn_bytes, &hash, accepted_at, &ack_bytes);
    let stored = txn_bytes.len();
    connection.execute("INSERT INTO op_log(host_seq,txn_id,device_id,base_host_seq,gesture_id,txn,state_hash,created_at_wall,accepted_at,ack) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![seq,id.as_str(),txn.device_id,sql_u64(base.host_seq)?,optional_uuid(txn.gesture_id.as_ref())?,txn_bytes,hash,txn.created_at_wall_ms,accepted_at,ack_bytes])?;
    Ok((sealed, stored))
}

/// Record in `into` every cancellation `from` holds. One for a gesture that
/// `into` has as committed is left out: the commit already settled it.
fn carry_cancellations(into: &mut HostSequencer, from: &HostSequencer) -> Result<(), StoreError> {
    for (device, gesture) in from.cancelled_gestures() {
        if !into.gesture_closed(device, gesture) {
            into.cancel_gesture(device.clone(), gesture.clone())?;
        }
    }
    Ok(())
}

const CANCELLED_PREFIX: &str = "cancelled ";

/// A second copy of one cancellation, as a row of its own. The checkpoint
/// that cancellations are recorded in is one row; if that row is ever damaged
/// the project opens from another checkpoint, and these keep what only the
/// damaged one held.
fn write_cancellation(
    connection: &Connection,
    device: &DeviceId,
    gesture: &Id,
) -> Result<(), StoreError> {
    connection
        .prepare("INSERT INTO meta(key,value) VALUES(?1,'') ON CONFLICT(key) DO NOTHING")?
        .execute([format!("{CANCELLED_PREFIX}{device} {gesture}")])?;
    Ok(())
}
fn write_cancellations(connection: &Connection, host: &HostSequencer) -> Result<(), StoreError> {
    for (device, gesture) in host.cancelled_gestures() {
        write_cancellation(connection, device, gesture)?;
    }
    Ok(())
}
fn read_cancellations(connection: &Connection) -> Result<BTreeSet<(DeviceId, Id)>, StoreError> {
    let mut query = connection.prepare("SELECT key FROM meta WHERE key GLOB 'cancelled *'")?;
    let mut rows = query.query([])?;
    let mut cancelled = BTreeSet::new();
    while let Some(row) = rows.next()? {
        let key: String = row.get(0)?;
        let (device, gesture) = key
            .strip_prefix(CANCELLED_PREFIX)
            .and_then(|pair| pair.split_once(' '))
            .ok_or(StoreError::Corrupt("cancellation record"))?;
        cancelled.insert((
            DeviceId::try_from(device.to_owned())
                .map_err(|_| StoreError::Corrupt("cancellation record"))?,
            Id::try_from(gesture.to_owned())
                .map_err(|_| StoreError::Corrupt("cancellation record"))?,
        ));
    }
    Ok(cancelled)
}
fn cancelled(host: &HostSequencer) -> BTreeSet<(DeviceId, Id)> {
    host.cancelled_gestures()
        .map(|(device, gesture)| (device.clone(), gesture.clone()))
        .collect()
}

/// What replaying a stretch of the log costs. It is estimated from what each
/// transaction touched and never timed, so that the same log gives the same
/// answer on every machine and every run. The unit is a microsecond on the
/// machine the estimate was fitted on (a four-core Linux container); only its
/// ratio to `LogSpan::load_micros`, fitted on the same machine, matters.
///
/// Replaying a transaction costs what accepting it did. The engine keeps
/// that small for one that changes objects at the end of the object order,
/// which every new stroke is. For one that changes an earlier object it
/// hashes every object from there on again, and for one that changes
/// anything else (a layer, an instruction, a mask, a result, the document)
/// it validates and hashes the whole project. Measured at 8,000 objects:
/// 0.06 ms for a new stroke, 45 ms for a change to the first object, and for
/// a new document title 62 ms with small shapes and 170 ms with strokes.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ReplayCost {
    micros: u64,
    span: LogSpan,
}
impl ReplayCost {
    /// One transaction, given the state it left and what it touched.
    fn add(&mut self, host: &HostSequencer, touched: &Touched, stored: usize) {
        let project = host.project();
        let entities = (project.documents.len()
            + project.assets.len()
            + project.layers.len()
            + project.objects.len()
            + project.groups.len()
            + project.instructions.len()
            + project.semantic_snapshots.len()
            + project.results.len()
            + project.mask_versions.len()) as u64;
        let read_again = if touched.only_objects() {
            touched
                .objects
                .first()
                .map_or(0, |first| project.objects.range(first..).count()) as u64
        } else {
            entities
        };
        // Their share of the canonical bytes, which is what hashing reads.
        let bytes = (host.canonical_len() as u64).saturating_mul(read_again) / entities.max(1);
        self.micros = self
            .micros
            .saturating_add(50 + 6 * read_again + bytes / 160);
        self.span.add(stored);
    }
}

/// How much log a stretch is: its rows and the bytes of their transactions.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LogSpan {
    rows: u64,
    bytes: u64,
}
impl LogSpan {
    fn add(&mut self, stored: usize) {
        self.rows = self.rows.saturating_add(1);
        self.bytes = self.bytes.saturating_add(stored as u64);
    }
    /// What loading a checkpoint that covers this much log costs, in the
    /// microseconds of `ReplayCost`. Loading parses, re-serializes and
    /// compares every transaction the checkpoint covers, in proportion to
    /// its size: measured 2.3 s for 8,000 strokes, 12 MB of log.
    fn load_micros(&self) -> u64 {
        self.rows.saturating_mul(60).saturating_add(self.bytes / 5)
    }
}

/// How much a checkpoint at the end of a log of size `log` is needed when the
/// next open would otherwise replay `tail`. For it to be due, replaying has
/// to cost enough to notice, more than loading a checkpoint of the same rows
/// would, and at least a quarter of loading one of the whole history, which
/// is what gets rewritten. A log of new strokes never meets the second
/// condition. Overdue and urgent compare the replay with loading the whole
/// history, which is what the rest of an open costs.
fn checkpoint_need(tail: &ReplayCost, log: &LogSpan) -> CheckpointNeed {
    let whole = log.load_micros();
    if tail.micros < CHECKPOINT_MIN_MICROS
        || tail.micros < tail.span.load_micros()
        || tail.micros.saturating_mul(4) < whole
    {
        CheckpointNeed::None
    } else if tail.micros < whole {
        CheckpointNeed::Due
    } else if tail.micros < CHECKPOINT_URGENT_MICROS || tail.micros < whole.saturating_mul(2) {
        CheckpointNeed::Overdue
    } else {
        CheckpointNeed::Urgent
    }
}

const SEAL_KEY: &str = "log_seal";
const SEAL_DOMAIN: &[u8] = b"VisualWorkbench.LogSeal.v1\n";

/// A running digest of every accepted log row in host order: the stored
/// transaction, acknowledgement, state hash and receipt time. It is stored in
/// the transaction that adds a row and recomputed when the project opens, so
/// a stored row that changed afterwards is found even where replaying it
/// would give the same project. Rows added by a build that does not keep the
/// seal are taken in at the next open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogSeal {
    seq: i64,
    digest: blake3::Hash,
}
impl LogSeal {
    pub(crate) fn empty() -> Self {
        Self {
            seq: 0,
            digest: blake3::hash(SEAL_DOMAIN),
        }
    }
    fn with_row(
        &self,
        seq: i64,
        txn: &[u8],
        state_hash: &str,
        accepted_at: i64,
        ack: &[u8],
    ) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.digest.as_bytes());
        hasher.update(&seq.to_le_bytes());
        hasher.update(&accepted_at.to_le_bytes());
        for part in [txn, state_hash.as_bytes(), ack] {
            hasher.update(&(part.len() as u64).to_le_bytes());
            hasher.update(part);
        }
        Self {
            seq,
            digest: hasher.finalize(),
        }
    }
    fn parse(text: &str) -> Option<Self> {
        let mut fields = text.split(' ');
        let (version, seq, digest) = (fields.next()?, fields.next()?, fields.next()?);
        let seal = Self {
            seq: seq.parse().ok().filter(|seq| *seq >= 0)?,
            digest: blake3::Hash::from_hex(digest).ok()?,
        };
        // Only the exact text this build writes is a seal.
        (version == "v1" && fields.next().is_none() && seal.text() == text).then_some(seal)
    }
    fn text(&self) -> String {
        format!("v1 {} {}", self.seq, self.digest.to_hex())
    }
}

pub(crate) fn write_seal(connection: &Connection, seal: &LogSeal) -> Result<(), StoreError> {
    connection
        .prepare("INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value")?
        .execute(params![SEAL_KEY, seal.text()])?;
    Ok(())
}

/// What the stored seal says about the log.
struct Sealed {
    /// The seal of the complete log.
    seal: LogSeal,
    /// Rows up to this revision are as the build that sealed them wrote them.
    through: i64,
    /// The stored seal is missing or ends before the last row.
    stale: bool,
    log: LogSpan,
}

/// Recompute the seal over the whole log and compare it with the stored one
/// at the row the stored one ends on.
fn verify_seal(connection: &Connection) -> Result<Sealed, StoreError> {
    let stored: Option<String> = connection
        .query_row("SELECT value FROM meta WHERE key=?1", [SEAL_KEY], |row| {
            row.get(0)
        })
        .optional()?;
    let stored = stored
        .map(|text| LogSeal::parse(&text).ok_or(StoreError::Corrupt("operation log seal")))
        .transpose()?;
    let mut seal = LogSeal::empty();
    let mut log = LogSpan::default();
    let mut matched = stored.as_ref() == Some(&seal);
    let mut query = connection
        .prepare("SELECT host_seq,txn,state_hash,accepted_at,ack FROM op_log ORDER BY host_seq")?;
    let mut rows = query.query([])?;
    while let Some(row) = rows.next()? {
        let txn: Vec<u8> = row.get(1)?;
        log.add(txn.len());
        seal = seal.with_row(
            row.get(0)?,
            &txn,
            &row.get::<_, String>(2)?,
            row.get(3)?,
            &row.get::<_, Vec<u8>>(4)?,
        );
        if let Some(stored) = &stored
            && stored.seq == seal.seq
        {
            if stored != &seal {
                return Err(StoreError::Corrupt("operation log seal"));
            }
            matched = true;
        }
    }
    // A seal that ends on a row the log no longer has means rows are gone.
    if stored.is_some() && !matched {
        return Err(StoreError::Corrupt("operation log seal"));
    }
    let stale = stored.as_ref() != Some(&seal);
    Ok(Sealed {
        seal,
        through: stored.map_or(0, |stored| stored.seq),
        stale,
        log,
    })
}

/// Make durable what `restore` found left to do: record cancellations a
/// later checkpoint held in the first one, give every cancellation its
/// second copy, remove every checkpoint that is not kept, and store the seal
/// of the complete log. Returns whether there was anything to do.
fn tidy<F: FnMut(OpenStage)>(
    connection: &mut Connection,
    host: &HostSequencer,
    seal: &LogSeal,
    upkeep: &Upkeep,
    observer: &mut F,
) -> Result<bool, StoreError> {
    if !(upkeep.superseded || upkeep.cancellations || upkeep.copies || upkeep.seal) {
        return Ok(false);
    }
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    migrations::verify_schema(&tx, migrations::CURRENT_VERSION)?;
    if upkeep.cancellations {
        let created: i64 = tx.query_row(
            "SELECT created_at FROM snapshots WHERE host_seq=?1",
            [upkeep.keep[0]],
            |row| row.get(0),
        )?;
        let mut ledger = load_checkpoint(&tx, upkeep.keep[0])?;
        carry_cancellations(&mut ledger, host)?;
        write_snapshot(&tx, &ledger, created)?;
    }
    if upkeep.copies {
        write_cancellations(&tx, host)?;
    }
    if upkeep.superseded {
        tx.execute(
            "DELETE FROM snapshots WHERE host_seq NOT IN (?1,?2)",
            upkeep.keep,
        )?;
    }
    if upkeep.seal {
        write_seal(&tx, seal)?;
    }
    observer(OpenStage::Pruned);
    tx.commit()?;
    observer(OpenStage::Committed);
    Ok(true)
}

const RECLAIM_KEY: &str = "compaction_attempt";
/// How often a compaction is started for about the same unused space.
const RECLAIM_ATTEMPTS: u32 = 2;

enum Reclaimed {
    NotDue,
    Skipped,
    Done,
}

/// Hand unused pages back to the filesystem when they are a large part of the
/// database, which happens once after an old checkpoint pile is removed.
///
/// A compaction that does not finish is not started again at every open. It
/// is refused where the copy needs a temporary file and there is no directory
/// for one, and it ends with the process when the system takes the process
/// away for the memory the copy needs. So each start is noted in the
/// database first and the note is removed when the compaction is through;
/// with two starts noted, nothing is tried until there is a quarter more to
/// hand back.
fn reclaim<F: FnMut(OpenStage) + Clone + Send + 'static>(
    connection: &mut Connection,
    observer: &mut F,
) -> Result<Reclaimed, StoreError> {
    let pragma = |name: &str| -> Result<i64, StoreError> {
        Ok(connection.query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))?)
    };
    let (pages, free, size) = (
        pragma("page_count")?,
        pragma("freelist_count")?,
        pragma("page_size")?,
    );
    if free.saturating_mul(size) < RECLAIM_MIN_BYTES || free.saturating_mul(4) < pages {
        return Ok(Reclaimed::NotDue);
    }
    let in_memory = (pages - free).saturating_mul(size) <= RECLAIM_IN_MEMORY_BYTES;
    // Android gives SQLite no directory for the temporary file a larger copy
    // needs, so there the attempt could only fail.
    if !in_memory && cfg!(target_os = "android") {
        return Ok(Reclaimed::Skipped);
    }
    let noted: Option<String> = connection
        .query_row(
            "SELECT value FROM meta WHERE key=?1",
            [RECLAIM_KEY],
            |row| row.get(0),
        )
        .optional()?;
    let (attempts, noted_free) = noted
        .as_deref()
        .and_then(|note| note.split_once(' '))
        .and_then(|(attempts, free)| {
            Some((attempts.parse::<u32>().ok()?, free.parse::<i64>().ok()?))
        })
        .unwrap_or((0, 0));
    let more = free > noted_free.saturating_add(noted_free / 4);
    if attempts >= RECLAIM_ATTEMPTS && !more {
        return Ok(Reclaimed::Skipped);
    }
    let attempts = if more { 1 } else { attempts + 1 };
    connection
        .prepare("INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value")?
        .execute(params![RECLAIM_KEY, format!("{attempts} {free}")])?;
    observer(OpenStage::Compacting);
    if in_memory {
        connection.execute_batch("PRAGMA temp_store=MEMORY")?;
    }
    // Tell the observer once from inside the copy.
    let mut inside = Some(observer.clone());
    connection.progress_handler(
        1000,
        Some(move || {
            if let Some(mut observer) = inside.take() {
                observer(OpenStage::Copying);
            }
            false
        }),
    )?;
    let compacted = connection.execute_batch("VACUUM");
    connection.progress_handler(0, None::<fn() -> bool>)?;
    if in_memory {
        connection.execute_batch("PRAGMA temp_store=DEFAULT")?;
    }
    compacted?;
    connection.execute("DELETE FROM meta WHERE key=?1", [RECLAIM_KEY])?;
    // The compacted pages are in the write-ahead log. Move them into the
    // database file, then empty the log. With a reader connected either step
    // gives up after the busy timeout and the next checkpoint finishes it.
    observer(OpenStage::Vacuumed);
    connection.execute_batch("PRAGMA wal_checkpoint(FULL)")?;
    observer(OpenStage::Transferred);
    connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    Ok(Reclaimed::Done)
}

/// How replaying a stretch of the log ended.
pub(crate) enum Replay {
    /// Every row was decided as stored: how many rows, and what that cost.
    Done(u64, ReplayCost),
    /// The cost reached the limit first.
    GaveUp,
    /// This engine does not decide the row at this revision as stored.
    Differs(i64),
}

/// What replay does with the acknowledgement of each row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Acks {
    /// Keep the stored one. The engine has to reproduce the revision and the
    /// state hash it names; its notices are text and are kept as stored.
    Stored,
    /// Store the one the engine gives: the first migration, whose rows have
    /// none yet.
    Write,
}

/// Replay the log after revision `after`. An error is a row that is not what
/// a log row has to be, or a failed read; a row this engine decides
/// differently is reported as that and leaves the host at the row before.
pub(crate) fn replay_log(
    connection: &Connection,
    host: &mut HostSequencer,
    after: i64,
    acks: Acks,
    give_up_at: Option<u64>,
) -> Result<Replay, StoreError> {
    let mut query=connection.prepare("SELECT host_seq,txn_id,device_id,base_host_seq,gesture_id,txn,state_hash,created_at_wall,accepted_at,ack FROM op_log WHERE host_seq>?1 ORDER BY host_seq")?;
    let mut rows = query.query([after])?;
    let mut count = 0;
    let mut cost = ReplayCost::default();
    while let Some(row) = rows.next()? {
        let seq: i64 = row.get(0)?;
        let bytes: Vec<u8> = row.get(5)?;
        let txn: v1::Transaction = decode(&bytes)?;
        verify_log_columns(row, &txn)?;
        let accepted_at: i64 = row.get(8)?;
        match acks {
            Acks::Stored => {
                let stored: v1::TxnAck = decode(&row.get::<_, Vec<u8>>(9)?)?;
                let hash = AssetId::try_from(row.get::<_, String>(6)?)?;
                // The row has to agree with itself before the engine is asked.
                if stored.txn_id != txn.txn_id
                    || sql_u64(stored.host_seq)? != seq
                    || stored.state_hash != hash.bytes()
                {
                    return Err(StoreError::Corrupt("operation ack"));
                }
                match host.replay(txn, accepted_at, &stored) {
                    Ok(Some(touched)) => cost.add(host, &touched, bytes.len()),
                    Ok(None) | Err(_) => return Ok(Replay::Differs(seq)),
                }
            }
            Acks::Write => {
                let device = DeviceId::try_from(txn.device_id.clone())?;
                let (accepted, replayed) =
                    host.submit_with(txn, &device, accepted_at, |host, _, touched| {
                        cost.add(host, touched, bytes.len());
                        Ok::<(), StoreError>(())
                    })?;
                // A stored row is never a retry of an earlier one.
                if replayed.is_none()
                    || sql_u64(accepted.ack.host_seq)? != seq
                    || host.state_hash()?.as_str() != row.get::<_, String>(6)?
                {
                    return Err(StoreError::Corrupt("operation replay"));
                }
                connection.execute(
                    "UPDATE op_log SET ack=?1 WHERE host_seq=?2",
                    params![accepted.ack.encode_to_vec(), seq],
                )?;
            }
        }
        count += 1;
        if give_up_at.is_some_and(|limit| cost.micros >= limit) {
            return Ok(Replay::GaveUp);
        }
    }
    Ok(Replay::Done(count, cost))
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

/// Exact ordinary-open validation, also used inside the v2-to-v3 migration
/// transaction before any schema mutation. Canonical history stays authoritative.
pub(crate) struct Restored {
    pub(crate) host: HostSequencer,
    pub(crate) replayed: u64,
    /// What replaying the log after the checkpoint that was used cost.
    pub(crate) cost: ReplayCost,
    pub(crate) log: LogSpan,
    pub(crate) seal: LogSeal,
    pub(crate) upkeep: Upkeep,
    /// Stored checkpoints that did not load.
    pub(crate) skipped: u64,
}

/// What an open still has to write once the project is verified.
#[derive(Debug)]
pub(crate) struct Upkeep {
    /// Revisions of the checkpoints to keep: the one cancellations are
    /// recorded in, then the one the log was replayed from. Often the same.
    pub(crate) keep: [i64; 2],
    /// Other checkpoints are stored.
    pub(crate) superseded: bool,
    /// The first of `keep` lacks cancellations the project holds.
    pub(crate) cancellations: bool,
    /// Some cancellation has no second copy.
    pub(crate) copies: bool,
    /// The stored log seal is missing or ends before the last row.
    pub(crate) seal: bool,
    /// A read failed while working out what to keep. The project is open
    /// from a verified checkpoint; nothing is written this time.
    pub(crate) unsettled: bool,
}

enum Replayed {
    Done(Box<HostSequencer>, u64, ReplayCost),
    GaveUp,
    Differs(i64),
}

/// The checkpoint at `seq` with the log after it replayed, how many rows that
/// was and what it cost.
fn replay_from(
    connection: &Connection,
    mut host: HostSequencer,
    seq: i64,
    give_up_at: Option<u64>,
) -> Result<Replayed, StoreError> {
    verify_prefix(connection, &host, seq)?;
    match replay_log(connection, &mut host, seq, Acks::Stored, give_up_at)? {
        Replay::Done(replayed, cost) => {
            // Each replayed step was matched with the hash the host carries
            // forward. Recompute the final state from nothing once, so the
            // stored history and the carried hash cannot agree on a state
            // the project is not in.
            if replayed > 0 && &host.project().state_hash()? != host.state_hash()? {
                return Err(StoreError::Corrupt("operation replay"));
            }
            Ok(Replayed::Done(Box::new(host), replayed, cost))
        }
        Replay::GaveUp => Ok(Replayed::GaveUp),
        Replay::Differs(seq) => Ok(Replayed::Differs(seq)),
    }
}

/// Load the project and check everything stored against it. Nothing is
/// written.
///
/// The log is replayed from the newest checkpoint that loads. When this
/// engine does not decide a row of it as stored, the project is refused, and
/// how depends on what is known about the row. Rows the seal covers are as
/// the build that accepted them wrote them, so the difference is the engine:
/// the project was saved by another version, `EngineMismatch`. A row the seal
/// does not cover may have changed since, and is reported as damage, as it
/// always was. Either way nothing has been changed, and the build that saved
/// the project still opens it. An earlier checkpoint is no way round this:
/// the same rows would have to be replayed from there as well.
pub(crate) fn restore(connection: &Connection) -> Result<Restored, StoreError> {
    integrity(connection)?;
    check_all_column_bounds(connection)?;
    check_payload_bounds(connection)?;
    let Sealed {
        seal,
        through,
        stale,
        log,
    } = verify_seal(connection)?;
    let rows = connection
        .prepare("SELECT host_seq FROM snapshots ORDER BY host_seq")?
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    // Start from the newest checkpoint that loads, as every build has. One
    // that does not load is skipped and counted: an earlier one and more of
    // the log give the same project.
    let mut failure = None;
    let mut newest = None;
    let mut skipped = 0;
    for (index, seq) in rows.iter().enumerate().rev() {
        match load_checkpoint(connection, *seq) {
            Ok(checkpoint) => {
                newest = Some((index, checkpoint));
                break;
            }
            Err(error) => {
                skipped += 1;
                failure.get_or_insert(error);
            }
        }
    }
    let Some((index, checkpoint)) = newest else {
        return Err(failure.unwrap_or(StoreError::Corrupt("checkpoint binding")));
    };
    let start = rows[index];
    // The cancellations held by the checkpoint they are recorded in.
    let mut recorded = cancelled(&checkpoint);
    // Conflict records up to here are the checkpoint's; later ones are
    // decided again by the replay.
    let mut loaded_conflicts = checkpoint.conflicts().len();
    let (mut host, mut replayed, mut cost) = match replay_from(connection, checkpoint, start, None)?
    {
        Replayed::Done(host, replayed, cost) => (*host, replayed, cost),
        Replayed::Differs(seq) if seq <= through => return Err(StoreError::EngineMismatch),
        Replayed::Differs(_) | Replayed::GaveUp => {
            return Err(StoreError::Corrupt("operation replay"));
        }
    };
    let mut keep = [start, start];
    let mut unsettled = false;
    // The revision-zero checkpoint, when the project has one besides the one
    // just used, is where cancellations are recorded. It is never removed.
    if index > 0 && rows[0] == 0 {
        match load_checkpoint(connection, 0) {
            Ok(first) => {
                if rows.len() > 2 || stale {
                    // An older build was here: it wrote a checkpoint for
                    // every 500 saves whether or not replaying them was
                    // cheaper, and for a log of strokes it is. Find out by
                    // replaying the whole log from revision zero, up to the
                    // cost at which a checkpoint pays.
                    let limit = CHECKPOINT_MIN_MICROS.max(log.load_micros());
                    match replay_from(connection, first.clone(), 0, Some(limit)) {
                        // Use it only if it is the history the checkpoint
                        // holds, journal and all.
                        Ok(Replayed::Done(mut whole, count, whole_cost))
                            if whole.same_history(&host) =>
                        {
                            carry_cancellations(&mut whole, &host)?;
                            (host, replayed, cost) = (*whole, count, whole_cost);
                            keep = [0, 0];
                            loaded_conflicts = 0;
                        }
                        // Replaying everything costs more than loading the
                        // later checkpoint, or this engine does not decide
                        // the early history as it was decided, or the log
                        // was not written on top of this checkpoint: the
                        // later checkpoint stays in use.
                        Ok(_) => {}
                        // The database could not be read. That says nothing
                        // about the checkpoints; decide another time.
                        Err(_) => unsettled = true,
                    }
                }
                carry_cancellations(&mut host, &first)?;
                recorded = cancelled(&first);
                keep[0] = 0;
            }
            Err(_) => skipped += 1,
        }
    }
    let copies = read_cancellations(connection)?;
    for (device, gesture) in &copies {
        if !host.gesture_closed(device, gesture) {
            host.cancel_gesture(device.clone(), gesture.clone())?;
        }
    }
    // The host now holds every cancellation stored anywhere.
    let held = cancelled(&host);
    let conflicts = read_conflicts(connection)?;
    if conflicts != host.conflicts() {
        // What a checkpoint held has to be in the table as it is. What the
        // replay of sealed rows decided again differs only if the engine does.
        let loaded = host.conflicts().get(..loaded_conflicts);
        return Err(
            if replayed > 0
                && !stale
                && loaded.is_some()
                && conflicts.get(..loaded_conflicts) == loaded
            {
                StoreError::EngineMismatch
            } else {
                StoreError::Corrupt("conflict projection")
            },
        );
    }
    super::projections::verify_assets(connection, host.project())?;
    super::projections::verify(
        connection,
        host.project(),
        sql_u64(host.revision()?.host_seq)?,
    )?;
    validate_pending(connection, host.project())?;
    Ok(Restored {
        host,
        replayed,
        cost,
        log,
        seal,
        upkeep: Upkeep {
            keep,
            superseded: !unsettled && rows.iter().any(|seq| !keep.contains(seq)),
            cancellations: !held.is_subset(&recorded),
            copies: !held.is_subset(&copies),
            seal: stale,
            unsettled,
        },
        skipped,
    })
}
