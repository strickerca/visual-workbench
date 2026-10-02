//! Process-death recovery, using the real project commit path on both platforms.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::error::Error;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use vw_model::{DeviceId, Document, Id, Project};
use vw_proto::v1 as pb;
use vw_store::{CommitStage, ProjectStore, SNAPSHOT_INTERVAL_MS};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
const CLOCK: i64 = 1_700_000_000_000;
const MARKER: &str = "VW_STORE_CRASH ";
const STARTUP_LIMIT: Duration = Duration::from_secs(10);
const RUN_LIMIT: Duration = if cfg!(target_os = "android") {
    Duration::from_secs(600)
} else {
    Duration::from_secs(3_600)
};
const SEED: u64 = 0x32ae_1964_570d_c82f;

fn fixture() -> tempfile::TempDir {
    #[cfg(target_os = "android")]
    {
        tempfile::tempdir_in(std::env::current_dir().expect("runner directory"))
            .expect("crash fixture")
    }
    #[cfg(not(target_os = "android"))]
    {
        tempfile::tempdir().expect("crash fixture")
    }
}
fn id(number: u64) -> Id {
    Id::from_parts(CLOCK as u64 + number, [0x53; 10]).expect("fixture ID")
}
fn device() -> DeviceId {
    DeviceId::from_bytes([0x35; 16])
}
fn title(sequence: u64) -> String {
    format!("Synthetic crash title {sequence:08}")
}
fn expected_project(sequence: u64) -> Project {
    let mut project = Project::new(id(1), "Crash recovery fixture".into(), device());
    project.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(2).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                title: title(sequence),
                schema_version: 1,
                ..Default::default()
            },
            pages: Vec::new(),
            created_at_ms: CLOCK,
        },
    );
    project
}
fn transaction(store: &ProjectStore, sequence: u64, time: i64) -> TestResult<pb::Transaction> {
    Ok(pb::Transaction {
        txn_id: Some(id(100 + sequence).to_proto()),
        project_id: Some(id(1).to_proto()),
        device_id: device().to_string(),
        base_revision: Some(store.revision()?),
        created_at_wall_ms: time,
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: device().to_string(),
                lamport: sequence,
            }),
            kind: Some(pb::op::Kind::UpdateDocument(pb::UpdateDocument {
                document_id: Some(id(2).to_proto()),
                title: title(sequence),
            })),
        }],
        ..Default::default()
    })
}
fn stage_number(stage: CommitStage) -> u8 {
    match stage {
        CommitStage::Prepared => 0,
        CommitStage::LogWritten => 1,
        CommitStage::BeforeCommit => 2,
        CommitStage::Committed => 3,
    }
}
fn next_random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}
fn failure(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(std::io::Error::other(message.into()))
}

#[derive(Debug)]
enum WorkerEvent {
    Ready,
    Stage { sequence: u64, stage: u8 },
}

/// Owns the child immediately after spawn; test errors and assertion unwinds
/// always terminate and reap it before removing its fixture directory.
struct WriterChild {
    child: Child,
    events: Receiver<WorkerEvent>,
    reader: Option<JoinHandle<()>>,
    reaped: bool,
}
impl WriterChild {
    fn spawn(path: &Path, stage: u8, ordinal: u64, snapshot: bool) -> TestResult<Self> {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args([
                "--ignored",
                "--exact",
                "crash_worker",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("VW_STORE_CRASH_PATH", path)
            .env("VW_STORE_CRASH_STAGE", stage.to_string())
            .env("VW_STORE_CRASH_ORDINAL", ordinal.to_string())
            .env("VW_STORE_CRASH_SNAPSHOT", if snapshot { "1" } else { "0" })
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let (sender, events) = mpsc::channel();
        let mut owner = Self {
            child: command.spawn()?,
            events,
            reader: None,
            reaped: false,
        };
        let stdout = owner
            .child
            .stdout
            .take()
            .ok_or_else(|| failure("crash worker stdout missing"))?;
        owner.reader = Some(
            thread::Builder::new()
                .name("crash-worker-events".into())
                .spawn(move || {
                    for line in BufReader::new(stdout).lines() {
                        let Ok(line) = line else {
                            break;
                        };
                        // libtest may precede the first marker on the same line. Its own
                        // summary lines are discarded and never enter the parent output.
                        let Some(start) = line.find(MARKER) else {
                            continue;
                        };
                        let mut fields = line[start + MARKER.len()..].split_whitespace();
                        let event = match fields.next() {
                            Some("READY") => Some(WorkerEvent::Ready),
                            Some("STAGE") => fields
                                .next()
                                .and_then(|value| value.parse::<u64>().ok())
                                .zip(fields.next().and_then(|value| value.parse::<u8>().ok()))
                                .map(|(sequence, stage)| WorkerEvent::Stage { sequence, stage }),
                            _ => None,
                        };
                        if let Some(event) = event
                            && sender.send(event).is_err()
                        {
                            break;
                        }
                    }
                })?,
        );
        Ok(owner)
    }
    fn await_event(&mut self, predicate: impl Fn(&WorkerEvent) -> bool, label: &str) -> TestResult {
        let deadline = Instant::now() + STARTUP_LIMIT;
        loop {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Err(failure(format!("crash worker timed out during {label}")));
            };
            match self
                .events
                .recv_timeout(remaining.min(Duration::from_millis(100)))
            {
                Ok(event) if predicate(&event) => return Ok(()),
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(status) = self.child.try_wait()? {
                        return Err(failure(format!(
                            "crash worker exited during {label}: {status}"
                        )));
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(failure(format!("crash worker output ended during {label}")));
                }
            }
        }
    }
    fn kill_and_reap(&mut self) -> TestResult {
        if let Some(status) = self.child.try_wait()? {
            return Err(failure(format!(
                "crash worker exited before forced termination: {status}"
            )));
        }
        self.child.kill()?;
        let status = self.child.wait()?;
        self.reaped = true;
        if status.success() {
            return Err(failure(
                "crash worker exited successfully instead of being killed",
            ));
        }
        if let Some(reader) = self.reader.take() {
            reader
                .join()
                .map_err(|_| failure("crash worker reader failed"))?;
        }
        Ok(())
    }
}
impl Drop for WriterChild {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.reaped = true;
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// Only the owning test launches this worker. It continually commits genuine
/// UpdateDocument operations until the parent terminates the OS process.
#[test]
#[ignore = "private subprocess entry point for crash recovery"]
fn crash_worker() -> TestResult {
    let path =
        std::env::var_os("VW_STORE_CRASH_PATH").ok_or_else(|| failure("missing worker fixture"))?;
    let selected_stage: u8 = std::env::var("VW_STORE_CRASH_STAGE")?.parse()?;
    let ordinal: u64 = std::env::var("VW_STORE_CRASH_ORDINAL")?.parse()?;
    let snapshot = std::env::var("VW_STORE_CRASH_SNAPSHOT")? == "1";
    let mut store = ProjectStore::open(Path::new(&path))?;
    println!("{MARKER}READY");
    std::io::stdout().flush()?;
    let started = Instant::now();
    for sequence in 1..=100_000 {
        if started.elapsed() > Duration::from_secs(30) {
            return Err(failure("worker lifetime exceeded"));
        }
        let time = CLOCK
            + sequence as i64
                * if snapshot {
                    SNAPSHOT_INTERVAL_MS + 1
                } else {
                    1
                };
        let txn = transaction(&store, sequence, time)?;
        store.commit_observed(&txn, &device(), time, |stage| {
            let number = stage_number(stage);
            if sequence == ordinal
                && (number == selected_stage || (selected_stage == 4 && number == 0))
            {
                println!("{MARKER}STAGE {sequence} {selected_stage}");
                let _ = std::io::stdout().flush();
                // Held boundaries prove actual pre-/in-/post-transaction kills.
                // Natural-window cases return immediately and keep committing.
                if selected_stage < 4 {
                    thread::sleep(Duration::from_secs(2));
                }
            }
        })?;
    }
    Err(failure("worker unexpectedly exhausted commit loop"))
}

fn independently_verify(path: &Path) -> TestResult<u64> {
    let database = Connection::open_with_flags(
        path.join("project.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    let checks = database
        .prepare("PRAGMA integrity_check")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(checks, ["ok"], "independent SQLite integrity check");
    let sequence: i64 =
        database.query_row("SELECT COALESCE(MAX(host_seq),0) FROM op_log", [], |row| {
            row.get(0)
        })?;
    let count: i64 = database.query_row("SELECT COUNT(*) FROM op_log", [], |row| row.get(0))?;
    let sequence = u64::try_from(sequence)?;
    let count = u64::try_from(count)?;
    assert_eq!(sequence, count, "committed log remains contiguous");
    let expected = expected_project(sequence);
    let hash = expected.state_hash()?;
    let stored: Option<String> = database
        .query_row(
            "SELECT state_hash FROM op_log ORDER BY host_seq DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if sequence == 0 {
        assert!(stored.is_none());
    } else {
        assert_eq!(
            stored.as_deref(),
            Some(hash.as_str()),
            "last committed database hash"
        );
    }
    drop(database);
    let opened = ProjectStore::open(path)?;
    opened.integrity_check()?;
    assert_eq!(opened.revision()?.host_seq, sequence, "recovery sequence");
    assert_eq!(
        opened.revision()?.state_hash,
        hash.bytes(),
        "recovery revision hash"
    );
    assert_eq!(
        opened.project().state_hash()?,
        hash,
        "independently derived state hash"
    );
    assert_eq!(
        opened.project(),
        &expected,
        "independently derived complete project"
    );
    Ok(sequence)
}

#[test]
fn random_process_kills_preserve_last_committed_state() -> TestResult {
    let iterations: u64 = if cfg!(windows) { 1_000 } else { 100 };
    let temp = fixture();
    let baseline = temp.path().join("baseline");
    drop(ProjectStore::create(
        &baseline,
        expected_project(0),
        device(),
        CLOCK,
    )?);
    let initial = independently_verify(&baseline)?;
    assert_eq!(initial, 0);
    assert!(
        !baseline.join("project.sqlite-wal").exists(),
        "closed baseline must have no live WAL"
    );
    let started = Instant::now();
    let mut random = SEED;
    let mut attempted_stages = [0u64; 5];
    let mut interrupted = 0u64;
    let mut committed = 0u64;
    let mut interrupted_in_transaction = 0u64;
    let mut snapshots_exercised = 0u64;
    let mut last_progress = Instant::now();
    println!("crash_recovery start iterations={iterations} seed={SEED:016x}");
    for iteration in 0..iterations {
        if started.elapsed() > RUN_LIMIT {
            return Err(failure(format!(
                "crash suite exceeded bound after {iteration} kills"
            )));
        }
        let path = temp.path().join(format!("run-{iteration:04}"));
        fs::create_dir(&path)?;
        fs::copy(baseline.join("project.sqlite"), path.join("project.sqlite"))?;
        fs::create_dir(path.join("blobs"))?;
        fs::create_dir(path.join("cache"))?;
        let stage = (next_random(&mut random) % 5) as u8;
        let ordinal = 2 + next_random(&mut random) % 5;
        let jitter_ms = next_random(&mut random) % if stage == 4 { 11 } else { 3 };
        let snapshot = next_random(&mut random) & 1 == 0;
        let mut writer = WriterChild::spawn(&path, stage, ordinal, snapshot)?;
        writer.await_event(
            |event| matches!(event, WorkerEvent::Ready),
            "open readiness",
        )?;
        writer.await_event(
            |event| {
                matches!(event, WorkerEvent::Stage { sequence, stage: observed }
            if *sequence == ordinal && *observed == stage)
            },
            "selected commit boundary",
        )?;
        if jitter_ms > 0 {
            thread::sleep(Duration::from_millis(jitter_ms));
        }
        writer.kill_and_reap()?;
        drop(writer);
        attempted_stages[stage as usize] += 1;
        let sequence = independently_verify(&path)?;
        if sequence < ordinal {
            assert_eq!(sequence, ordinal - 1, "all earlier transactions committed");
            interrupted += 1;
            if stage == 1 || stage == 2 {
                interrupted_in_transaction += 1;
            }
        } else {
            committed += 1;
        }
        if snapshot {
            snapshots_exercised += 1;
        }
        // This is an owned direct child of the fresh fixture root; no paths from
        // databases, environment content or archive entries determine deletion.
        let resolved_root = temp.path().canonicalize()?;
        let resolved_run = path.canonicalize()?;
        assert_eq!(resolved_run.parent(), Some(resolved_root.as_path()));
        fs::remove_dir_all(&resolved_run)?;
        if (iteration + 1) % 25 == 0
            || iteration + 1 == iterations
            || last_progress.elapsed() >= Duration::from_secs(15)
        {
            println!(
                "crash_recovery progress kills={} interrupted={interrupted} committed={committed} in_transaction={interrupted_in_transaction} elapsed_ms={}",
                iteration + 1,
                started.elapsed().as_millis()
            );
            last_progress = Instant::now();
        }
    }
    assert_eq!(interrupted + committed, iterations);
    assert!(
        interrupted_in_transaction > 0 && committed > 0,
        "both atomic outcomes exercised"
    );
    assert!(
        attempted_stages.iter().all(|count| *count > 0),
        "all commit boundaries and natural windows exercised"
    );
    assert!(snapshots_exercised > 0 && snapshots_exercised < iterations);
    println!(
        "crash_recovery complete iterations={iterations} stages={attempted_stages:?} interrupted={interrupted} committed={committed} in_transaction={interrupted_in_transaction} snapshot_runs={snapshots_exercised} elapsed_ms={} children_reaped={iterations}",
        started.elapsed().as_millis()
    );
    Ok(())
}
