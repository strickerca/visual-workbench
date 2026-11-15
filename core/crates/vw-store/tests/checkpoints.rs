//! Checkpoints and the log: when a checkpoint is written and which are kept,
//! projects written by builds that kept a pile of them, damaged checkpoints,
//! process exit inside the clean-up, a cancellation and a checkpoint, and what
//! the other device receives.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};
use tempfile::TempDir;
use vw_model::{AssetId, DeviceId, Document, Id, Layer, Project};
use vw_ops::HostSequencer;
use vw_proto::{Message, v1 as pb};
use vw_store::{CheckpointNeed, CommitStage, OpenStage, ProjectStore, Upkeeping};

fn temporary() -> TempDir {
    let mut builder = tempfile::Builder::new();
    builder.prefix("vw-checkpoints-");
    #[cfg(target_os = "android")]
    {
        builder
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap()
    }
    #[cfg(not(target_os = "android"))]
    {
        builder.tempdir().unwrap()
    }
}
fn id(n: u64) -> Id {
    Id::from_parts(1_700_000_000_000 + n, [0x43; 10]).unwrap()
}
fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
fn project() -> Project {
    let mut project = Project::new(id(1), "Checkpoint fixture".into(), device(1));
    project.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(2).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Before".into(),
                ..Default::default()
            },
            pages: vec![],
            created_at_ms: 0,
        },
    );
    project.layers.insert(
        id(3),
        Layer {
            definition: pb::CreateLayer {
                layer_id: Some(id(3).to_proto()),
                document_id: Some(id(2).to_proto()),
                page_index: -1,
                name: "Annotations".into(),
                kind: "annotation".into(),
                order_key: "V".into(),
            },
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: "normal".into(),
        },
    );
    project
}
/// `count` new rectangles in one transaction, numbered from `first`. A
/// transaction of new objects at the end of the object order is the cheap
/// kind: it costs what it adds.
fn rectangles(revision: pb::Revision, n: u64, first: u64, count: u64) -> pb::Transaction {
    pb::Transaction {
        txn_id: Some(id(n).to_proto()),
        project_id: Some(id(1).to_proto()),
        device_id: device(1).to_string(),
        base_revision: Some(revision),
        created_at_wall_ms: n as i64,
        ops: (0..count)
            .map(|index| pb::Op {
                op_id: Some(pb::OpId {
                    device_id: device(1).to_string(),
                    lamport: 1_000_000 + first + index,
                }),
                kind: Some(pb::op::Kind::CreateObject(pb::CreateObject {
                    document_id: Some(id(2).to_proto()),
                    state: Some(pb::ObjectState {
                        object_id: Some(id(1_000_000 + first + index).to_proto()),
                        layer_id: Some(id(3).to_proto()),
                        order_key: "V".into(),
                        transform: Some(pb::Affine {
                            a: 1.0,
                            d: 1.0,
                            ..Default::default()
                        }),
                        style: Some(pb::Style {
                            stroke: Some(pb::Color { rgba: 0x112233ff }),
                            width: 1.0,
                            ..Default::default()
                        }),
                        role: pb::Role::None as i32,
                        created_by: device(1).to_string(),
                        created_at_ms: 1,
                        shape: Some(pb::object_state::Shape::Rect(pb::RectD {
                            x: index as f64,
                            y: 1.0,
                            w: 4.0,
                            h: 3.0,
                        })),
                        ..Default::default()
                    }),
                })),
            })
            .collect(),
        ..Default::default()
    }
}
/// A title change by the host device; `text` pads it to make history heavy.
fn title(revision: pb::Revision, n: u64, text: usize) -> pb::Transaction {
    pb::Transaction {
        txn_id: Some(id(n).to_proto()),
        project_id: Some(id(1).to_proto()),
        device_id: device(1).to_string(),
        base_revision: Some(revision),
        created_at_wall_ms: n as i64,
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: device(1).to_string(),
                lamport: n,
            }),
            kind: Some(pb::op::Kind::UpdateDocument(pb::UpdateDocument {
                document_id: Some(id(2).to_proto()),
                title: format!("{n:06} {}", "x".repeat(text)),
            })),
        }],
        ..Default::default()
    }
}
fn query(path: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open(path.join("project.sqlite")).unwrap()
}
fn checkpoints(path: &Path) -> Vec<i64> {
    query(path)
        .prepare("SELECT host_seq FROM snapshots ORDER BY host_seq")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}
fn database_bytes(path: &Path) -> u64 {
    assert!(!path.join("project.sqlite-wal").exists());
    fs::metadata(path.join("project.sqlite")).unwrap().len()
}
fn seal(path: &Path) -> Option<String> {
    use rusqlite::OptionalExtension;
    query(path)
        .query_row("SELECT value FROM meta WHERE key='log_seal'", [], |row| {
            row.get(0)
        })
        .optional()
        .unwrap()
}
const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn write_checkpoint(database: &rusqlite::Connection, host: &HostSequencer) {
    let checkpoint = host.checkpoint_bytes().unwrap();
    database
        .execute(
            "INSERT OR REPLACE INTO snapshots(host_seq,state,state_hash,created_at,checkpoint,checkpoint_hash) VALUES(?1,?2,?3,?4,?5,?6)",
            rusqlite::params![
                host.revision().unwrap().host_seq as i64,
                host.project().canonical_bytes().unwrap(),
                host.project().state_hash().unwrap().to_string(),
                7,
                &checkpoint,
                AssetId::hash(&checkpoint).to_string()
            ],
        )
        .unwrap();
}

struct OldProject {
    path: PathBuf,
    /// The sequencing state the project must open to, cancellation included.
    host: HostSequencer,
}
const CANCELLED: u64 = 77;

/// A project as a build from before this change left it: a checkpoint of the
/// whole history every `stride` saves (it was every 500) and at each
/// cancellation, none ever removed, and no log seal. `cancel_after` cancels
/// gesture `CANCELLED` of device 2 once that many saves are stored.
fn old_project(
    root: &Path,
    saves: u64,
    stride: u64,
    text: usize,
    cancel_after: Option<u64>,
) -> OldProject {
    let path = root.join("old.vwb");
    let mut store = ProjectStore::create(&path, project(), device(1), 0).unwrap();
    let mut host = HostSequencer::new(project(), device(1)).unwrap();
    let mut pile = Vec::new();
    for n in 1..=saves {
        let txn = title(store.revision().unwrap(), 100 + n, text);
        store.commit(&txn, &device(1), 1000 + n as i64).unwrap();
        host.submit(txn, &device(1), 1000 + n as i64).unwrap();
        if cancel_after == Some(n) {
            host.cancel_gesture(device(2), id(CANCELLED)).unwrap();
        }
        if n % stride == 0 || cancel_after == Some(n) {
            pile.push(host.clone());
        }
    }
    drop(store);
    let database = query(&path);
    for checkpoint in &pile {
        write_checkpoint(&database, checkpoint);
    }
    database
        .execute("DELETE FROM meta WHERE key='log_seal'", [])
        .unwrap();
    drop(database);
    OldProject { path, host }
}

/// How a build from before this change opens a project: the newest checkpoint
/// must be whole and agree with the log it covers, and the log after it is
/// replayed. Returns the sequencing state that build would hold.
fn open_as_an_older_build(path: &Path) -> HostSequencer {
    let database = query(path);
    let (seq, state, hash, checkpoint, digest): (i64, Vec<u8>, String, Vec<u8>, String) = database
        .query_row(
            "SELECT host_seq,state,state_hash,checkpoint,checkpoint_hash FROM snapshots ORDER BY host_seq DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(AssetId::hash(&checkpoint).to_string(), digest);
    let mut host = HostSequencer::from_checkpoint_bytes(&checkpoint).unwrap();
    assert_eq!(
        host.project(),
        &Project::from_canonical_bytes(&state).unwrap()
    );
    assert_eq!(host.project().state_hash().unwrap().to_string(), hash);
    assert_eq!(host.revision().unwrap().host_seq, seq as u64);
    let mut log = database
        .prepare("SELECT host_seq,txn,state_hash,accepted_at,ack FROM op_log ORDER BY host_seq")
        .unwrap();
    let mut rows = log.query([]).unwrap();
    let mut next = 1;
    while let Some(row) = rows.next().unwrap() {
        let (row_seq, bytes, state_hash, accepted_at, ack): (i64, Vec<u8>, String, i64, Vec<u8>) = (
            row.get(0).unwrap(),
            row.get(1).unwrap(),
            row.get(2).unwrap(),
            row.get(3).unwrap(),
            row.get(4).unwrap(),
        );
        assert_eq!(row_seq, next);
        next += 1;
        let txn = pb::Transaction::decode(bytes.as_slice()).unwrap();
        if row_seq <= seq {
            let id = Id::from_proto(txn.txn_id.as_ref()).unwrap();
            let (stored, receipt) = host.accepted_transaction(&id).unwrap();
            assert_eq!(stored.encode_to_vec(), bytes);
            assert_eq!(receipt.encode_to_vec(), ack);
            assert_eq!(host.accepted_at(&id), Some(accepted_at));
        } else {
            let author = DeviceId::try_from(txn.device_id.clone()).unwrap();
            let accepted = host.submit(txn, &author, accepted_at).unwrap();
            assert!(!accepted.duplicate);
            assert_eq!(accepted.ack.encode_to_vec(), ack);
            assert_eq!(host.state_hash().unwrap().to_string(), state_hash);
        }
    }
    assert!(next > seq);
    host
}

fn assert_opens_as(path: &Path, host: &HostSequencer) -> ProjectStore {
    let store = ProjectStore::open(path).unwrap();
    assert_eq!(store.revision().unwrap(), host.revision().unwrap());
    assert_eq!(store.state_hash().unwrap(), host.state_hash().unwrap());
    assert_eq!(store.project(), host.project());
    assert_eq!(
        store.checkpoint_bytes().unwrap(),
        host.checkpoint_bytes().unwrap()
    );
    store.integrity_check().unwrap();
    store
}

#[test]
fn a_project_from_an_older_build_opens_to_the_same_state_and_gives_its_pile_back() {
    let temp = temporary();
    let old = old_project(temp.path(), 160, 20, 4000, Some(90));
    assert_eq!(
        checkpoints(&old.path),
        [0, 20, 40, 60, 80, 90, 100, 120, 140, 160]
    );
    let before = database_bytes(&old.path);
    assert!(before > 8 * 1024 * 1024);
    // The older build itself opens it from its newest checkpoint.
    assert_eq!(
        open_as_an_older_build(&old.path)
            .checkpoint_bytes()
            .unwrap(),
        old.host.checkpoint_bytes().unwrap()
    );

    let mut store = assert_opens_as(&old.path, &old.host);
    assert_eq!(store.replayed_transactions(), 160);
    // The cancellation an old checkpoint held still guards its gesture.
    assert!(store.gesture_closed(&device(2), &id(CANCELLED)));
    let mut late = title(store.revision().unwrap(), 900, 0);
    late.device_id = device(2).to_string();
    late.ops[0].op_id.as_mut().unwrap().device_id = device(2).to_string();
    late.gesture_id = Some(id(CANCELLED).to_proto());
    assert!(store.commit(&late, &device(2), 5000).is_err());
    drop(store);

    // One checkpoint is left, the file is a fraction of what it was and holds
    // no unused pages, and the log is sealed.
    assert_eq!(checkpoints(&old.path), [0]);
    let after = database_bytes(&old.path);
    assert!(after * 4 < before, "{before} bytes became {after}");
    assert_eq!(
        query(&old.path)
            .query_row("PRAGMA freelist_count", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(seal(&old.path).unwrap().starts_with("v1 160 "));

    // It opens the same again, here and in the older build, which finds the
    // cancellation in the one checkpoint that is left.
    let store = assert_opens_as(&old.path, &old.host);
    assert_eq!(store.replayed_transactions(), 160);
    drop(store);
    let older = open_as_an_older_build(&old.path);
    assert_eq!(
        older.checkpoint_bytes().unwrap(),
        old.host.checkpoint_bytes().unwrap()
    );
    assert!(older.gesture_closed(&device(2), &id(CANCELLED)));
    assert_eq!(database_bytes(&old.path), after);
    temp.close().unwrap();
}

#[test]
fn a_damaged_checkpoint_is_skipped_and_the_project_opens_from_another_and_the_log() {
    let zero_state_hash =
        format!("UPDATE snapshots SET state_hash='{ZERO_HASH}' WHERE host_seq=12");
    let zero_digest =
        format!("UPDATE snapshots SET checkpoint_hash='{ZERO_HASH}' WHERE host_seq=12");
    for (sql, kept) in [
        // The newest, where an older build starts and would have refused.
        ("UPDATE snapshots SET checkpoint=x'00' WHERE host_seq=12", 0),
        (zero_state_hash.as_str(), 0),
        (zero_digest.as_str(), 0),
        ("UPDATE snapshots SET state=x'7b7d' WHERE host_seq=12", 0),
        ("UPDATE snapshots SET checkpoint=NULL WHERE host_seq=12", 0),
        // The oldest, one in the middle, and all but one. Without the
        // revision-zero checkpoint the newest that loads is the one kept.
        ("UPDATE snapshots SET checkpoint=x'00' WHERE host_seq=0", 12),
        ("DELETE FROM snapshots WHERE host_seq=0", 12),
        ("UPDATE snapshots SET checkpoint=x'00' WHERE host_seq=8", 0),
        (
            "UPDATE snapshots SET checkpoint=x'00' WHERE host_seq IN (0,12)",
            8,
        ),
        (
            "UPDATE snapshots SET checkpoint=x'00' WHERE host_seq<12",
            12,
        ),
        (
            "UPDATE snapshots SET checkpoint=x'00' WHERE host_seq NOT IN (8)",
            8,
        ),
    ] {
        let temp = temporary();
        let old = old_project(temp.path(), 12, 4, 16, Some(2));
        assert_eq!(checkpoints(&old.path), [0, 2, 4, 8, 12]);
        assert_ne!(query(&old.path).execute(sql, []).unwrap(), 0, "{sql}");
        let store = assert_opens_as(&old.path, &old.host);
        assert_eq!(store.replayed_transactions(), 12 - kept as u64, "{sql}");
        drop(store);
        assert_eq!(checkpoints(&old.path), [kept], "{sql}");
        let store = assert_opens_as(&old.path, &old.host);
        assert_eq!(store.replayed_transactions(), 12 - kept as u64, "{sql}");
        drop(store);
        assert_eq!(
            open_as_an_older_build(&old.path)
                .checkpoint_bytes()
                .unwrap(),
            old.host.checkpoint_bytes().unwrap(),
            "{sql}"
        );
        temp.close().unwrap();
    }
}

#[test]
fn damage_that_leaves_nothing_to_open_from_is_refused_and_nothing_is_removed() {
    for sql in [
        // No checkpoint is whole.
        "UPDATE snapshots SET checkpoint=x'00'",
        "DELETE FROM snapshots",
        // The log no longer says what a whole checkpoint recorded of it.
        "UPDATE op_log SET accepted_at=accepted_at+1 WHERE host_seq=2",
        "UPDATE op_log SET accepted_at=accepted_at+1 WHERE host_seq=12",
        "DELETE FROM op_log WHERE host_seq=12",
        "UPDATE op_log SET ack=x'ff' WHERE host_seq=5",
        // The only whole checkpoint is not where the log continues.
        "UPDATE snapshots SET checkpoint=x'00' WHERE host_seq<>4; DELETE FROM op_log WHERE host_seq=5",
    ] {
        let temp = temporary();
        let old = old_project(temp.path(), 12, 4, 16, None);
        let database = query(&old.path);
        database.execute_batch(sql).unwrap();
        let rows = |database: &rusqlite::Connection| -> Vec<(i64, Vec<u8>, Option<Vec<u8>>)> {
            database
                .prepare("SELECT host_seq,state,checkpoint FROM snapshots ORDER BY host_seq")
                .unwrap()
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        let before = rows(&database);
        assert!(ProjectStore::open(&old.path).is_err(), "accepted {sql}");
        assert_eq!(rows(&database), before, "{sql}");
        assert_eq!(seal(&old.path), None, "{sql}");
        drop(database);
        temp.close().unwrap();
    }
}

#[test]
fn the_newest_checkpoint_is_kept_when_the_log_does_not_replay_from_the_oldest() {
    let temp = temporary();
    let old = old_project(temp.path(), 12, 4, 16, None);
    // The oldest checkpoint is whole and belongs to this project, but the log
    // was not written on top of it. It holds a cancellation of its own.
    let mut other = project();
    other.title = "Another beginning".into();
    let mut other = HostSequencer::new(other, device(1)).unwrap();
    other.cancel_gesture(device(2), id(88)).unwrap();
    write_checkpoint(&query(&old.path), &other);
    assert_eq!(checkpoints(&old.path), [0, 4, 8, 12]);
    // The project opens from the newest checkpoint. A revision-zero
    // checkpoint that loads is never removed, whatever the log makes of it:
    // its cancellation counts, and it stays the place they are recorded.
    let mut host = old.host.clone();
    host.cancel_gesture(device(2), id(88)).unwrap();
    let store = assert_opens_as(&old.path, &host);
    assert_eq!(store.replayed_transactions(), 0);
    drop(store);
    assert_eq!(checkpoints(&old.path), [0, 12]);
    let mut store = assert_opens_as(&old.path, &host);
    // Saving and cancelling go on from the checkpoint that is in use.
    let txn = title(store.revision().unwrap(), 500, 0);
    store.commit(&txn, &device(1), 9000).unwrap();
    host.submit(txn, &device(1), 9000).unwrap();
    store
        .cancel_gesture(device(2), id(CANCELLED), 9001)
        .unwrap();
    host.cancel_gesture(device(2), id(CANCELLED)).unwrap();
    drop(store);
    assert_eq!(checkpoints(&old.path), [0, 12]);
    let store = assert_opens_as(&old.path, &host);
    assert_eq!(store.replayed_transactions(), 1);
    drop(store);
    temp.close().unwrap();
}

#[test]
fn rows_saved_by_a_build_without_the_seal_are_taken_in_and_sealed_rows_cannot_change() {
    let temp = temporary();
    let path = temp.path().join("project.vwb");
    let mut store = ProjectStore::create(&path, project(), device(1), 0).unwrap();
    assert!(seal(&path).unwrap().starts_with("v1 0 "));
    for n in 1..=2 {
        let txn = title(store.revision().unwrap(), 100 + n, 0);
        store.commit(&txn, &device(1), 1000 + n as i64).unwrap();
    }
    let two = seal(&path).unwrap();
    assert!(two.starts_with("v1 2 "));
    for n in 3..=4 {
        let txn = title(store.revision().unwrap(), 100 + n, 0);
        store.commit(&txn, &device(1), 1000 + n as i64).unwrap();
    }
    let four = seal(&path).unwrap();
    let expected = store.revision().unwrap();
    drop(store);

    // An older build saved rows 3 and 4: it adds them and leaves the seal.
    let set_seal = |text: &str| {
        query(&path)
            .execute("UPDATE meta SET value=?1 WHERE key='log_seal'", [text])
            .unwrap();
    };
    set_seal(&two);
    let store = ProjectStore::open(&path).unwrap();
    assert_eq!(store.revision().unwrap(), expected);
    drop(store);
    assert_eq!(seal(&path).unwrap(), four);

    // A row under the seal that changes is refused, wherever it is, although
    // replaying it gives the same project: a title change does not use the
    // receipt time.
    for row in 1..=4 {
        let change = format!("UPDATE op_log SET accepted_at=accepted_at+1 WHERE host_seq={row}");
        let restore = format!("UPDATE op_log SET accepted_at=accepted_at-1 WHERE host_seq={row}");
        query(&path).execute(&change, []).unwrap();
        assert!(ProjectStore::open(&path).is_err(), "row {row}");
        if row <= 2 {
            set_seal(&two);
            assert!(ProjectStore::open(&path).is_err(), "row {row}");
            set_seal(&four);
        }
        query(&path).execute(&restore, []).unwrap();
    }
    // A seal that ends beyond the log means sealed rows were removed.
    query(&path)
        .execute("DELETE FROM op_log WHERE host_seq=4", [])
        .unwrap();
    assert!(ProjectStore::open(&path).is_err());
    assert_eq!(seal(&path).unwrap(), four);
    temp.close().unwrap();
}

#[test]
fn the_other_device_can_start_from_checkpoint_bytes_at_any_moment() {
    let temp = temporary();
    let path = temp.path().join("host.vwb");
    let mut host = ProjectStore::create(&path, project(), device(1), 0).unwrap();
    let follower_path = temp.path().join("follower.vwb");
    let mut follower = ProjectStore::create_authenticated_checkpoint(
        &follower_path,
        &host.checkpoint_bytes().unwrap(),
        &device(1),
        &device(2),
        1,
    )
    .unwrap();
    let mut joined = 0;
    // Older builds wrote a checkpoint at the 500th save and after five
    // minutes. The other device never read those rows: it is given the
    // sequencing state in memory, which is complete at every moment, also
    // right after a cancellation and at the saves where a checkpoint was due.
    for n in 1..=503_u64 {
        let txn = title(host.revision().unwrap(), 100 + n, 0);
        host.commit(&txn, &device(1), n as i64 * 301_000).unwrap();
        if n == 250 || n == 500 {
            host.cancel_gesture(device(2), id(CANCELLED + n), n as i64 * 301_000)
                .unwrap();
        }
        if ![1, 250, 499, 500, 501, 503].contains(&n) {
            continue;
        }
        let bytes = host.checkpoint_bytes().unwrap();
        // A device that joins now.
        let joining = temp.path().join(format!("joining-{n}.vwb"));
        let mirror = ProjectStore::create_authenticated_checkpoint(
            &joining,
            &bytes,
            &device(1),
            &device(2),
            2,
        )
        .unwrap();
        assert_eq!(mirror.checkpoint_bytes().unwrap(), bytes);
        drop(mirror);
        let mirror = ProjectStore::open(&joining).unwrap();
        assert_eq!(mirror.checkpoint_bytes().unwrap(), bytes);
        assert_eq!(mirror.replayed_transactions(), 0);
        drop(mirror);
        assert_eq!(checkpoints(&joining), [n as i64]);
        joined += 1;
        // A device that joined at the beginning and catches up now.
        follower
            .install_authenticated_checkpoint(&bytes, &device(1), 3)
            .unwrap();
        assert_eq!(follower.checkpoint_bytes().unwrap(), bytes);
        assert_eq!(checkpoints(&follower_path), [0]);
        if n == 250 {
            drop(follower);
            follower = ProjectStore::open(&follower_path).unwrap();
            assert_eq!(follower.checkpoint_bytes().unwrap(), bytes);
            assert_eq!(follower.replayed_transactions(), 250);
        }
    }
    assert_eq!(joined, 6);
    let bytes = host.checkpoint_bytes().unwrap();
    drop(follower);
    let follower = ProjectStore::open(&follower_path).unwrap();
    assert_eq!(follower.checkpoint_bytes().unwrap(), bytes);
    assert_eq!(follower.replayed_transactions(), 503);
    for n in [250, 500] {
        assert!(follower.gesture_closed(&device(2), &id(CANCELLED + n)));
    }
    drop(follower);
    assert_eq!(checkpoints(&follower_path), [0]);
    assert_eq!(checkpoints(&path), [0]);
    // An older build opens both and holds the same state.
    for project in [&path, &follower_path] {
        assert_eq!(
            open_as_an_older_build(project).checkpoint_bytes().unwrap(),
            bytes
        );
    }
    drop(host);
    temp.close().unwrap();
}

fn run_worker(name: &str, project: &Path, stage: &str) -> ExitStatus {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", name])
        .env("VW_CHECKPOINT_PROJECT", project)
        .env("VW_CHECKPOINT_STAGE", stage)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("owned worker {name} exceeded its bounded run at {stage}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn process_exit_at_each_boundary_of_the_clean_up_leaves_a_project_that_opens_the_same() {
    for stage in [
        OpenStage::Restored,
        OpenStage::Pruned,
        OpenStage::Committed,
        // The compaction: noted but not started, inside SQLite's copy of the
        // rows, committed to the write-ahead log, moved into the database
        // file with the log not yet emptied, and done.
        OpenStage::Compacting,
        OpenStage::Copying,
        OpenStage::Vacuumed,
        OpenStage::Transferred,
        OpenStage::Compacted,
    ] {
        let temp = temporary();
        let old = old_project(temp.path(), 160, 20, 4000, Some(90));
        let pile = checkpoints(&old.path);
        let before = database_bytes(&old.path);
        let status = run_worker("clean_up_exit_worker", &old.path, &format!("{stage:?}"));
        assert_eq!(
            status.code(),
            Some(83),
            "worker must exit without running Rust destructors at {stage:?}"
        );
        // The pile goes in one transaction: all of it is there, or only the
        // checkpoint the log replays from.
        let committed = !matches!(stage, OpenStage::Restored | OpenStage::Pruned);
        assert_eq!(
            checkpoints(&old.path),
            if committed { vec![0] } else { pile },
            "{stage:?}"
        );
        let store = assert_opens_as(&old.path, &old.host);
        assert!(store.gesture_closed(&device(2), &id(CANCELLED)));
        drop(store);
        assert_eq!(checkpoints(&old.path), [0], "{stage:?}");
        assert!(database_bytes(&old.path) * 4 < before, "{stage:?}");
        drop(assert_opens_as(&old.path, &old.host));
        temp.close().unwrap();
    }
}

#[test]
#[ignore = "private owned subprocess for process exit inside the open clean-up"]
fn clean_up_exit_worker() {
    let path = std::env::var_os("VW_CHECKPOINT_PROJECT").expect("owned project");
    let stage = std::env::var("VW_CHECKPOINT_STAGE").expect("owned boundary");
    let _store = ProjectStore::open_observed(Path::new(&path), move |at| {
        if format!("{at:?}") == stage {
            std::process::exit(83);
        }
    })
    .unwrap();
    panic!("requested clean-up boundary was not reached");
}

#[test]
fn a_kill_while_the_pile_is_handed_back_leaves_a_project_that_opens_the_same() {
    // Besides the exits at the compaction's own boundaries above, the writer
    // is killed at a spread of moments after it starts.
    for delay_ms in [0, 1, 3, 8, 21] {
        let temp = temporary();
        let old = old_project(temp.path(), 160, 20, 4000, Some(90));
        let before = database_bytes(&old.path);
        let marker = temp.path().join("pile-removed");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "clean_up_kill_worker"])
            .env("VW_CHECKPOINT_PROJECT", &old.path)
            .env("VW_CHECKPOINT_STAGE", &marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        while !marker.exists() {
            if child.try_wait().unwrap().is_some() || Instant::now() >= deadline {
                let _ = child.kill();
                child.wait().unwrap();
                panic!("owned worker did not reach the compaction");
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_millis(delay_ms));
        child.kill().unwrap();
        assert!(!child.wait().unwrap().success());
        assert_eq!(checkpoints(&old.path), [0]);
        drop(assert_opens_as(&old.path, &old.host));
        // Whatever the kill interrupted, this open finished the job.
        assert!(database_bytes(&old.path) * 4 < before, "{delay_ms} ms");
        drop(assert_opens_as(&old.path, &old.host));
        temp.close().unwrap();
    }
}

#[test]
#[ignore = "private owned subprocess killed while it compacts"]
fn clean_up_kill_worker() {
    let path = std::env::var_os("VW_CHECKPOINT_PROJECT").expect("owned project");
    let marker = std::env::var_os("VW_CHECKPOINT_STAGE").expect("owned marker");
    let _store = ProjectStore::open_observed(Path::new(&path), move |at| {
        if at == OpenStage::Committed {
            fs::write(&marker, b"").unwrap();
        }
    })
    .unwrap();
    std::thread::sleep(Duration::from_secs(60));
    panic!("the owning test did not end this worker");
}

#[test]
fn a_compaction_that_ended_with_the_process_twice_is_not_started_at_every_open() {
    let temp = temporary();
    let old = old_project(temp.path(), 160, 20, 4000, Some(90));
    let before = database_bytes(&old.path);
    // The process ends inside the copy, as it does when the system takes it
    // away for the memory the copy needs. Twice.
    for _ in 0..2 {
        let status = run_worker("clean_up_exit_worker", &old.path, "Copying");
        assert_eq!(status.code(), Some(83));
    }
    // The third open leaves it alone, says so, and is a normal open.
    let store = assert_opens_as(&old.path, &old.host);
    assert!(matches!(store.open_report().tidy, Upkeeping::NotDue));
    assert!(matches!(store.open_report().compaction, Upkeeping::Skipped));
    drop(store);
    assert_eq!(checkpoints(&old.path), [0]);
    assert!(database_bytes(&old.path) * 2 > before);
    // Without the note the next open compacts.
    assert_eq!(
        query(&old.path)
            .execute("DELETE FROM meta WHERE key='compaction_attempt'", [])
            .unwrap(),
        1
    );
    let store = assert_opens_as(&old.path, &old.host);
    assert!(matches!(store.open_report().compaction, Upkeeping::Done(_)));
    drop(store);
    assert!(database_bytes(&old.path) * 4 < before);
    let store = assert_opens_as(&old.path, &old.host);
    assert!(matches!(store.open_report().compaction, Upkeeping::NotDue));
    drop(store);
    temp.close().unwrap();
}

#[test]
fn a_clean_up_the_file_refuses_leaves_the_project_open_and_the_next_open_does_it() {
    let temp = temporary();
    let old = old_project(temp.path(), 160, 20, 4000, Some(90));
    let pile = checkpoints(&old.path);
    let before = database_bytes(&old.path);
    // Something else holds the database for writing, which this store's own
    // lock does not rule out for a program that ignores it.
    let other = query(&old.path);
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = Instant::now();
    let store = assert_opens_as(&old.path, &old.host);
    // The open did not sit out the five seconds a save waits for the file.
    assert!(started.elapsed() < Duration::from_secs(4));
    assert!(store.gesture_closed(&device(2), &id(CANCELLED)));
    let report = store.open_report();
    assert!(matches!(report.tidy, Upkeeping::Failed(..)), "{report:?}");
    assert!(matches!(report.checkpoint, Upkeeping::Skipped));
    assert!(matches!(report.compaction, Upkeeping::Skipped));
    drop(store);
    other.execute_batch("ROLLBACK").unwrap();
    drop(other);
    // Nothing was removed and nothing sealed.
    assert_eq!(checkpoints(&old.path), pile);
    assert_eq!(seal(&old.path), None);
    assert_eq!(database_bytes(&old.path), before);

    let store = assert_opens_as(&old.path, &old.host);
    let report = store.open_report();
    assert!(matches!(report.tidy, Upkeeping::Done(_)), "{report:?}");
    assert!(matches!(report.compaction, Upkeeping::Done(_)));
    assert_eq!(report.skipped_checkpoints, 0);
    drop(store);
    assert_eq!(checkpoints(&old.path), [0]);
    assert!(database_bytes(&old.path) * 4 < before);
    temp.close().unwrap();
}

#[test]
fn process_exit_at_each_boundary_of_a_cancellation_keeps_all_of_it_or_none() {
    for stage in [
        CommitStage::Prepared,
        CommitStage::LogWritten,
        CommitStage::BeforeCommit,
        CommitStage::Committed,
    ] {
        let temp = temporary();
        let path = temp.path().join("project.vwb");
        let mut store = ProjectStore::create(&path, project(), device(1), 0).unwrap();
        let mut host = HostSequencer::new(project(), device(1)).unwrap();
        for n in 1..=3 {
            let txn = title(store.revision().unwrap(), 100 + n, 0);
            store.commit(&txn, &device(1), 1000 + n as i64).unwrap();
            host.submit(txn, &device(1), 1000 + n as i64).unwrap();
        }
        drop(store);
        let status = run_worker("cancellation_exit_worker", &path, &format!("{stage:?}"));
        assert_eq!(
            status.code(),
            Some(83),
            "worker must exit without running Rust destructors at {stage:?}"
        );
        if stage == CommitStage::Committed {
            host.cancel_gesture(device(2), id(CANCELLED)).unwrap();
        }
        let store = assert_opens_as(&path, &host);
        assert_eq!(
            store.gesture_closed(&device(2), &id(CANCELLED)),
            stage == CommitStage::Committed,
            "{stage:?}"
        );
        drop(store);
        assert_eq!(checkpoints(&path), [0]);
        assert_eq!(
            open_as_an_older_build(&path).checkpoint_bytes().unwrap(),
            host.checkpoint_bytes().unwrap()
        );
        temp.close().unwrap();
    }
}

#[test]
#[ignore = "private owned subprocess for process exit inside a cancellation"]
fn cancellation_exit_worker() {
    let path = std::env::var_os("VW_CHECKPOINT_PROJECT").expect("owned project");
    let stage = std::env::var("VW_CHECKPOINT_STAGE").expect("owned boundary");
    let mut store = ProjectStore::open(Path::new(&path)).unwrap();
    store
        .cancel_gesture_observed(device(2), id(CANCELLED), 5, |at| {
            if format!("{at:?}") == stage {
                std::process::exit(83);
            }
        })
        .unwrap();
    panic!("requested cancellation boundary was not reached");
}

/// A project whose log is expensive to replay: many objects, then changes of
/// the document title. A title is not an object, so the engine checks the
/// whole project to accept the change, and again to replay it.
const HEAVY_OBJECTS: u64 = 1000;
fn heavy_project(path: &Path) -> ProjectStore {
    let mut store = ProjectStore::create(path, project(), device(1), 0).unwrap();
    let txn = rectangles(store.revision().unwrap(), 50, 0, HEAVY_OBJECTS);
    store.commit(&txn, &device(1), 50).unwrap();
    store
}
/// Title changes numbered from `first`.
fn retitle(store: &mut ProjectStore, first: u64, count: u64) {
    for n in first..first + count {
        let txn = title(store.revision().unwrap(), 2000 + n, 0);
        store.commit(&txn, &device(1), 2000 + n as i64).unwrap();
    }
}
/// One more rectangle each, numbered from `first`: the cheap kind of save.
fn draw(store: &mut ProjectStore, first: u64, count: u64) {
    for n in first..first + count {
        let txn = rectangles(store.revision().unwrap(), 100 + n, HEAVY_OBJECTS + n, 1);
        store.commit(&txn, &device(1), 100 + n as i64).unwrap();
    }
}
/// Open and compare with the sequencing state another store held.
fn assert_opens_to(path: &Path, checkpoint: &[u8]) -> ProjectStore {
    let store = ProjectStore::open(path).unwrap();
    assert!(store.checkpoint_bytes().unwrap() == checkpoint);
    assert_eq!(
        store.state_hash().unwrap(),
        &store.project().state_hash().unwrap()
    );
    store.integrity_check().unwrap();
    store
}

#[test]
fn a_checkpoint_is_written_once_replaying_costs_more_than_loading_one_and_never_by_a_save() {
    let temp = temporary();
    let path = temp.path().join("project.vwb");
    let mut store = heavy_project(&path);
    // Cheap saves, however many, never make one due.
    draw(&mut store, 0, 40);
    assert!(!store.checkpoint_if_due(900).unwrap());
    // Expensive ones do, and only this call writes it.
    let mut expensive = 0;
    while !store.checkpoint_if_due(901).unwrap() {
        assert_eq!(checkpoints(&path), [0]);
        retitle(&mut store, expensive, 1);
        expensive += 1;
        assert!(expensive < 40, "a checkpoint never became due");
    }
    assert!((8..=20).contains(&expensive), "{expensive}");
    let revision = store.revision().unwrap().host_seq;
    assert_eq!(checkpoints(&path), [0, revision as i64]);
    assert!(!store.checkpoint_if_due(902).unwrap());
    let state = store.checkpoint_bytes().unwrap();
    drop(store);

    // Opening starts from it, here and in an older build.
    let mut store = assert_opens_to(&path, &state);
    assert_eq!(store.replayed_transactions(), 0);
    assert!(open_as_an_older_build(&path).checkpoint_bytes().unwrap() == state);
    // A cancellation is recorded in the revision-zero checkpoint; the later
    // one, which holds the whole history, is not rewritten for it.
    let later = |path: &Path| -> String {
        query(path)
            .query_row(
                "SELECT checkpoint_hash FROM snapshots WHERE host_seq>0",
                [],
                |row| row.get(0),
            )
            .unwrap()
    };
    let before = later(&path);
    store.cancel_gesture(device(2), id(CANCELLED), 903).unwrap();
    assert_eq!(later(&path), before);
    draw(&mut store, 40, 5);
    assert!(!store.checkpoint_if_due(904).unwrap());
    let state = store.checkpoint_bytes().unwrap();
    drop(store);
    assert_eq!(checkpoints(&path), [0, revision as i64]);
    let store = assert_opens_to(&path, &state);
    assert_eq!(store.replayed_transactions(), 5);
    assert!(store.gesture_closed(&device(2), &id(CANCELLED)));
    drop(store);

    // Damage it. The project opens from revision zero and the whole log,
    // cancellation included, and that open, having replayed so much, writes
    // a new one.
    assert_eq!(
        query(&path)
            .execute("UPDATE snapshots SET checkpoint=x'00' WHERE host_seq>0", [])
            .unwrap(),
        1
    );
    let store = assert_opens_to(&path, &state);
    assert_eq!(store.replayed_transactions(), revision + 5);
    drop(store);
    assert_eq!(checkpoints(&path), [0, revision as i64 + 5]);
    let store = assert_opens_to(&path, &state);
    assert_eq!(store.replayed_transactions(), 0);
    drop(store);
    let older = open_as_an_older_build(&path);
    assert!(older.checkpoint_bytes().unwrap() == state);
    assert!(older.gesture_closed(&device(2), &id(CANCELLED)));
    temp.close().unwrap();
}

#[test]
fn an_open_that_had_to_replay_a_lot_writes_the_checkpoint_itself() {
    let temp = temporary();
    let path = temp.path().join("project.vwb");
    let mut store = heavy_project(&path);
    // A device that joins now starts from this revision.
    let mirror_path = temp.path().join("mirror.vwb");
    let mut mirror = ProjectStore::create_authenticated_checkpoint(
        &mirror_path,
        &store.checkpoint_bytes().unwrap(),
        &device(1),
        &device(2),
        1,
    )
    .unwrap();
    retitle(&mut store, 0, 14);
    let state = store.checkpoint_bytes().unwrap();
    // The application was killed: nothing asked for a checkpoint.
    drop(store);
    assert_eq!(checkpoints(&path), [0]);
    let store = assert_opens_to(&path, &state);
    assert_eq!(store.replayed_transactions(), 15);
    drop(store);
    assert_eq!(checkpoints(&path), [0, 15]);
    let store = assert_opens_to(&path, &state);
    assert_eq!(store.replayed_transactions(), 0);
    drop(store);

    // The mirror has no revision-zero checkpoint. Its new one replaces the
    // one it started from.
    mirror
        .install_authenticated_checkpoint(&state, &device(1), 2)
        .unwrap();
    assert!(!mirror.checkpoint_if_due(3).unwrap());
    drop(mirror);
    assert_eq!(checkpoints(&mirror_path), [1]);
    let mirror = assert_opens_to(&mirror_path, &state);
    assert_eq!(mirror.replayed_transactions(), 14);
    drop(mirror);
    assert_eq!(checkpoints(&mirror_path), [15]);
    let mirror = assert_opens_to(&mirror_path, &state);
    assert_eq!(mirror.replayed_transactions(), 0);
    drop(mirror);
    assert!(
        open_as_an_older_build(&mirror_path)
            .checkpoint_bytes()
            .unwrap()
            == state
    );
    temp.close().unwrap();
}

#[test]
fn a_project_from_an_older_build_keeps_its_newest_checkpoint_when_its_log_is_expensive() {
    let temp = temporary();
    let path = temp.path().join("old.vwb");
    let mut store = heavy_project(&path);
    let mut pile = Vec::new();
    for round in 0..3 {
        retitle(&mut store, round * 5, 5);
        pile.push(
            HostSequencer::from_checkpoint_bytes(&store.checkpoint_bytes().unwrap()).unwrap(),
        );
    }
    let state = store.checkpoint_bytes().unwrap();
    drop(store);
    let database = query(&path);
    for checkpoint in &pile {
        write_checkpoint(&database, checkpoint);
    }
    database
        .execute("DELETE FROM meta WHERE key='log_seal'", [])
        .unwrap();
    drop(database);
    assert_eq!(checkpoints(&path), [0, 6, 11, 16]);
    let store = assert_opens_to(&path, &state);
    assert_eq!(store.replayed_transactions(), 0);
    drop(store);
    assert_eq!(checkpoints(&path), [0, 16]);
    let store = assert_opens_to(&path, &state);
    assert_eq!(store.replayed_transactions(), 0);
    drop(store);
    assert_eq!(checkpoints(&path), [0, 16]);
    temp.close().unwrap();
}

#[test]
fn process_exit_at_each_boundary_of_a_checkpoint_keeps_the_old_one_or_the_new_one() {
    for stage in [
        CommitStage::Prepared,
        CommitStage::LogWritten,
        CommitStage::BeforeCommit,
        CommitStage::Committed,
    ] {
        let temp = temporary();
        let path = temp.path().join("project.vwb");
        let mut store = ProjectStore::create(&path, project(), device(1), 0).unwrap();
        let mut host = HostSequencer::new(project(), device(1)).unwrap();
        for n in 1..=5 {
            let txn = title(store.revision().unwrap(), 100 + n, 0);
            store.commit(&txn, &device(1), 1000 + n as i64).unwrap();
            host.submit(txn, &device(1), 1000 + n as i64).unwrap();
            if n == 3 {
                store.checkpoint_now(4).unwrap();
            }
        }
        store.cancel_gesture(device(2), id(CANCELLED), 5).unwrap();
        host.cancel_gesture(device(2), id(CANCELLED)).unwrap();
        drop(store);
        assert_eq!(checkpoints(&path), [0, 3]);
        let status = run_worker("checkpoint_exit_worker", &path, &format!("{stage:?}"));
        assert_eq!(
            status.code(),
            Some(83),
            "worker must exit without running Rust destructors at {stage:?}"
        );
        let committed = stage == CommitStage::Committed;
        assert_eq!(
            checkpoints(&path),
            if committed { [0, 5] } else { [0, 3] },
            "{stage:?}"
        );
        let store = assert_opens_as(&path, &host);
        assert_eq!(
            store.replayed_transactions(),
            if committed { 0 } else { 2 },
            "{stage:?}"
        );
        drop(store);
        // The older build starts from the newest and finds the cancellation
        // only when that one was written after it.
        let older = open_as_an_older_build(&path);
        assert_eq!(older.revision().unwrap(), host.revision().unwrap());
        assert_eq!(
            older.gesture_closed(&device(2), &id(CANCELLED)),
            committed,
            "{stage:?}"
        );
        temp.close().unwrap();
    }
}

#[test]
#[ignore = "private owned subprocess for process exit inside a checkpoint"]
fn checkpoint_exit_worker() {
    let path = std::env::var_os("VW_CHECKPOINT_PROJECT").expect("owned project");
    let stage = std::env::var("VW_CHECKPOINT_STAGE").expect("owned boundary");
    let mut store = ProjectStore::open(Path::new(&path)).unwrap();
    store
        .checkpoint_now_observed(6, |at| {
            if format!("{at:?}") == stage {
                std::process::exit(83);
            }
        })
        .unwrap();
    panic!("requested checkpoint boundary was not reached");
}

/// The `host_seq` and digest of every stored checkpoint.
fn stored_checkpoints(path: &Path) -> Vec<(i64, String)> {
    query(path)
        .prepare("SELECT host_seq,checkpoint_hash FROM snapshots ORDER BY host_seq")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// A new checkpoint replaces the one before it in the same transaction. On
/// the other device no revision-zero checkpoint stays behind, so the new row
/// is then the only way into the project and into the edits queued in it.
/// Here the new row is written and cannot be read back: the project the
/// database says it belongs to is not the one the checkpoint holds.
#[test]
fn a_checkpoint_that_does_not_read_back_is_not_stored_and_the_one_before_it_stays() {
    for other_device in [false, true] {
        let temp = temporary();
        let host_path = temp.path().join("host.vwb");
        let mut host = heavy_project(&host_path);
        let path = if other_device {
            temp.path().join("mirror.vwb")
        } else {
            host_path.clone()
        };
        let mut mirror = other_device.then(|| {
            ProjectStore::create_authenticated_checkpoint(
                &path,
                &host.checkpoint_bytes().unwrap(),
                &device(1),
                &device(2),
                1,
            )
            .unwrap()
        });
        retitle(&mut host, 0, 3);
        let state = host.checkpoint_bytes().unwrap();
        if let Some(mirror) = &mut mirror {
            mirror
                .install_authenticated_checkpoint(&state, &device(1), 2)
                .unwrap();
            // An edit made offline, waiting for the host.
            let mut queued = title(mirror.revision().unwrap(), 700, 0);
            queued.device_id = device(2).to_string();
            queued.ops[0].op_id.as_mut().unwrap().device_id = device(2).to_string();
            mirror.enqueue_pending(&queued).unwrap();
        }
        let store = mirror.as_mut().unwrap_or(&mut host);
        let before = stored_checkpoints(&path);
        assert_eq!(
            before.iter().map(|(seq, _)| *seq).collect::<Vec<_>>(),
            if other_device { vec![1] } else { vec![0] }
        );

        let identity = |value: &str| {
            assert_eq!(
                query(&path)
                    .execute("UPDATE meta SET value=?1 WHERE key='project_id'", [value])
                    .unwrap(),
                1
            );
        };
        identity(id(999).as_str());
        assert!(store.checkpoint_now(9).is_err(), "{other_device}");
        identity(id(1).as_str());
        // What was stored is untouched, and so is the open project.
        assert_eq!(stored_checkpoints(&path), before);
        assert!(store.checkpoint_bytes().unwrap() == state);
        assert_eq!(store.pending().unwrap().len(), usize::from(other_device));
        drop(mirror);
        drop(host);
        let mut store = assert_opens_to(&path, &state);
        assert_eq!(
            store.replayed_transactions(),
            if other_device { 3 } else { 4 }
        );
        assert_eq!(store.pending().unwrap().len(), usize::from(other_device));

        // One that reads back replaces it.
        store.checkpoint_now(10).unwrap();
        drop(store);
        assert_eq!(
            checkpoints(&path),
            if other_device { vec![4] } else { vec![0, 4] }
        );
        let store = assert_opens_to(&path, &state);
        assert_eq!(store.replayed_transactions(), 0);
        assert_eq!(store.pending().unwrap().len(), usize::from(other_device));
        drop(store);
        temp.close().unwrap();
    }
}

#[test]
fn process_exit_at_each_boundary_of_a_checkpoint_on_the_other_device_keeps_a_way_in() {
    for stage in [
        CommitStage::Prepared,
        CommitStage::LogWritten,
        CommitStage::BeforeCommit,
        CommitStage::Committed,
    ] {
        let temp = temporary();
        let mut host =
            ProjectStore::create(&temp.path().join("host.vwb"), project(), device(1), 0).unwrap();
        retitle(&mut host, 0, 2);
        // The other device joins at revision two: that checkpoint is its
        // only one, and there is no revision-zero checkpoint beside it.
        let path = temp.path().join("mirror.vwb");
        let mut mirror = ProjectStore::create_authenticated_checkpoint(
            &path,
            &host.checkpoint_bytes().unwrap(),
            &device(1),
            &device(2),
            1,
        )
        .unwrap();
        retitle(&mut host, 2, 3);
        host.cancel_gesture(device(2), id(CANCELLED), 5).unwrap();
        let state = host.checkpoint_bytes().unwrap();
        mirror
            .install_authenticated_checkpoint(&state, &device(1), 2)
            .unwrap();
        let mut queued = title(mirror.revision().unwrap(), 700, 0);
        queued.device_id = device(2).to_string();
        queued.ops[0].op_id.as_mut().unwrap().device_id = device(2).to_string();
        mirror.enqueue_pending(&queued).unwrap();
        drop(mirror);
        assert_eq!(checkpoints(&path), [2]);

        let status = run_worker("checkpoint_exit_worker", &path, &format!("{stage:?}"));
        assert_eq!(
            status.code(),
            Some(83),
            "worker must exit without running Rust destructors at {stage:?}"
        );
        let committed = stage == CommitStage::Committed;
        assert_eq!(
            checkpoints(&path),
            if committed { [5] } else { [2] },
            "{stage:?}"
        );
        let store = assert_opens_to(&path, &state);
        assert_eq!(
            store.replayed_transactions(),
            if committed { 0 } else { 3 },
            "{stage:?}"
        );
        assert!(store.gesture_closed(&device(2), &id(CANCELLED)));
        assert_eq!(store.pending().unwrap(), [queued], "{stage:?}");
        drop(store);
        drop(host);
        temp.close().unwrap();
    }
}

/// Cancellations are recorded in the revision-zero checkpoint, which is one
/// row. Each also has a row of its own, so a project that has to open from
/// its later checkpoint still holds those recorded after that one was
/// written.
#[test]
fn cancellations_recorded_in_a_checkpoint_that_is_then_damaged_are_kept() {
    let temp = temporary();
    let path = temp.path().join("project.vwb");
    let mut store = heavy_project(&path);
    store.cancel_gesture(device(2), id(CANCELLED), 800).unwrap();
    retitle(&mut store, 0, 25);
    assert!(store.checkpoint_if_due(900).unwrap());
    let later = store.revision().unwrap().host_seq as i64;
    store
        .cancel_gesture(device(2), id(CANCELLED + 1), 901)
        .unwrap();
    let state = store.checkpoint_bytes().unwrap();
    drop(store);
    assert_eq!(checkpoints(&path), [0, later]);
    // The later checkpoint does not hold the second cancellation.
    assert!(!open_as_an_older_build(&path).gesture_closed(&device(2), &id(CANCELLED + 1)));

    assert_eq!(
        query(&path)
            .execute("UPDATE snapshots SET checkpoint=x'00' WHERE host_seq=0", [])
            .unwrap(),
        1
    );
    let mut store = assert_opens_to(&path, &state);
    assert_eq!(store.open_report().skipped_checkpoints, 1);
    for n in [CANCELLED, CANCELLED + 1] {
        assert!(store.gesture_closed(&device(2), &id(n)));
        let mut late = title(store.revision().unwrap(), 900 + n, 0);
        late.device_id = device(2).to_string();
        late.ops[0].op_id.as_mut().unwrap().device_id = device(2).to_string();
        late.gesture_id = Some(id(n).to_proto());
        assert!(store.commit(&late, &device(2), 5000).is_err());
    }
    drop(store);
    // The damaged row is gone, the cancellations are recorded in the one
    // that is left, and an older build finds both there.
    assert_eq!(checkpoints(&path), [later]);
    let store = assert_opens_to(&path, &state);
    assert_eq!(store.open_report().skipped_checkpoints, 0);
    drop(store);
    assert!(open_as_an_older_build(&path).checkpoint_bytes().unwrap() == state);
    // A copy that is not a cancellation is damage, not something to skip.
    query(&path)
        .execute(
            "INSERT INTO meta(key,value) VALUES('cancelled nonsense','')",
            [],
        )
        .unwrap();
    assert!(ProjectStore::open(&path).is_err());
    temp.close().unwrap();
}

/// How much a checkpoint is needed grows with what the next open would
/// replay, and each caller asks for its own degree: closing for one that is
/// due, an idle project for one that is overdue, a busy one only for one that
/// is urgent.
#[test]
fn the_need_for_a_checkpoint_grows_with_the_replay_it_would_save() {
    let temp = temporary();
    let path = temp.path().join("project.vwb");
    let mut store = heavy_project(&path);
    draw(&mut store, 0, 40);
    // Enough history that loading it takes longer than the least replay a
    // checkpoint is written for; otherwise due and overdue are one step.
    for bulk in 0..8 {
        let txn = rectangles(
            store.revision().unwrap(),
            60 + bulk,
            10_000 + bulk * 900,
            900,
        );
        store.commit(&txn, &device(1), 60 + bulk as i64).unwrap();
    }
    assert_eq!(store.checkpoint_need(), CheckpointNeed::None);
    for need in [
        CheckpointNeed::None,
        CheckpointNeed::Due,
        CheckpointNeed::Overdue,
        CheckpointNeed::Urgent,
    ] {
        assert!(!store.checkpoint_if(need, 900).unwrap());
    }
    // From a checkpoint on, as after an open: while the whole log is still
    // to replay, a checkpoint that is due is overdue as well.
    store.checkpoint_now(900).unwrap();
    let start = store.revision().unwrap().host_seq as i64;
    let mut retitled = 0;
    let mut reached = Vec::new();
    for need in [
        CheckpointNeed::Due,
        CheckpointNeed::Overdue,
        CheckpointNeed::Urgent,
    ] {
        while store.checkpoint_need() < need {
            retitle(&mut store, retitled, 1);
            retitled += 1;
            assert!(retitled < 4000, "{need:?} was never reached");
        }
        reached.push(retitled);
        if need < CheckpointNeed::Urgent {
            assert_eq!(store.checkpoint_need(), need);
            // Not written for a caller that asks for more than this.
            assert!(!store.checkpoint_if(CheckpointNeed::Urgent, 901).unwrap());
            assert_eq!(checkpoints(&path), [0, start]);
        }
    }
    // Overdue takes more replay than due, and urgent is seconds of it.
    assert!(reached[0] < reached[1], "{reached:?}");
    assert!(reached[1] * 4 < reached[2], "{reached:?}");
    // Asking for nothing writes nothing; asking for what is there writes it.
    assert!(!store.checkpoint_if(CheckpointNeed::None, 902).unwrap());
    assert!(store.checkpoint_if(CheckpointNeed::Urgent, 903).unwrap());
    let revision = store.revision().unwrap().host_seq as i64;
    assert_eq!(checkpoints(&path), [0, revision]);
    assert_eq!(store.checkpoint_need(), CheckpointNeed::None);
    let state = store.checkpoint_bytes().unwrap();
    drop(store);
    let store = assert_opens_to(&path, &state);
    assert_eq!(store.replayed_transactions(), 0);
    assert!(matches!(store.open_report().checkpoint, Upkeeping::NotDue));
    drop(store);
    temp.close().unwrap();
}
