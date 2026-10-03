//! Durable client synchronization, including process exit inside SQLite writes.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use tempfile::TempDir;
use vw_model::{AssetId, DeviceId, Document, Id, Project};
use vw_ops::HostSequencer;
use vw_proto::v1 as pb;
use vw_store::{CommitStage, ProjectStore};

fn temporary() -> TempDir {
    let mut builder = tempfile::Builder::new();
    builder.prefix("vw-sync-recovery-");
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
    Id::from_parts(1_700_000_000_000 + n, [0x64; 10]).unwrap()
}
fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
fn project() -> Project {
    let mut project = Project::new(id(1), "Durable sync fixture".into(), device(1));
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
    project
}
fn create(path: &Path) -> ProjectStore {
    ProjectStore::create(path, project(), device(1), 0).unwrap()
}
fn host() -> HostSequencer {
    HostSequencer::new(project(), device(1)).unwrap()
}
fn transaction(revision: pb::Revision, n: u64, who: u8, kind: pb::op::Kind) -> pb::Transaction {
    pb::Transaction {
        txn_id: Some(id(n).to_proto()),
        project_id: Some(id(1).to_proto()),
        device_id: device(who).to_string(),
        base_revision: Some(revision),
        created_at_wall_ms: n as i64,
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: device(who).to_string(),
                lamport: n,
            }),
            kind: Some(kind),
        }],
        ..Default::default()
    }
}
fn title(revision: pb::Revision, n: u64, who: u8) -> pb::Transaction {
    transaction(
        revision,
        n,
        who,
        pb::op::Kind::UpdateDocument(pb::UpdateDocument {
            document_id: Some(id(2).to_proto()),
            title: format!("Title {n}"),
        }),
    )
}
fn asset(revision: pb::Revision, n: u64, size: u64) -> pb::Transaction {
    transaction(
        revision,
        n,
        2,
        pb::op::Kind::AddAsset(pb::AddAsset {
            asset_id: AssetId::hash(&n.to_le_bytes()).to_string(),
            format: "png".into(),
            width: 1,
            height: 1,
            orientation: 1,
            bit_depth: 8,
            byte_size: size,
            color_space: "sRGB".into(),
            source: "import".into(),
            metadata_json: "{}".into(),
            ..Default::default()
        }),
    )
}
fn query(path: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open(path.join("project.sqlite")).unwrap()
}

#[test]
fn checkpoint_install_preserves_exact_history_conflicts_and_unrelated_local_data() {
    let temp = temporary();
    let path = temp.path().join("client");
    let mut client = create(&path);
    let mut remote = host();
    let baseline = remote.revision().unwrap();
    client
        .register_device(&device(2), Some("Local synthetic label"), "android", 40)
        .unwrap();
    query(&path)
        .execute(
            "INSERT INTO settings(key,value) VALUES ('synthetic','keep')",
            [],
        )
        .unwrap();
    let first = title(baseline.clone(), 10, 1);
    let pending = title(baseline.clone(), 11, 2);
    let unrelated = asset(baseline, 12, 8);
    client.enqueue_pending(&pending).unwrap();
    client.enqueue_pending(&unrelated).unwrap();
    let accepted_first = remote.submit(first.clone(), &device(1), 100).unwrap();
    let accepted = remote.submit(pending.clone(), &device(2), 101).unwrap();
    assert_eq!(accepted.conflicts.len(), 1);
    // A receipt is not a durable copy of the accepted state.
    assert!(client.acknowledge_pending(&pending, &accepted.ack).is_err());
    assert_eq!(
        client.pending().unwrap(),
        vec![pending.clone(), unrelated.clone()]
    );
    let checkpoint = remote.checkpoint_bytes().unwrap();
    assert_eq!(
        client
            .install_authenticated_checkpoint(&checkpoint, &device(1), 102)
            .unwrap(),
        1
    );
    assert_eq!(
        client
            .install_authenticated_checkpoint(&checkpoint, &device(1), 103)
            .unwrap(),
        0
    );
    assert_eq!(client.project(), remote.project());
    assert_eq!(client.pending().unwrap(), vec![unrelated.clone()]);
    query(&path)
        .execute("UPDATE conflicts SET resolved_at=103,resolution='kept'", [])
        .unwrap();
    let later = title(remote.revision().unwrap(), 13, 1);
    remote.submit(later, &device(1), 104).unwrap();
    client
        .install_authenticated_checkpoint(&remote.checkpoint_bytes().unwrap(), &device(1), 105)
        .unwrap();
    drop(client);
    let mut client = ProjectStore::open(&path).unwrap();
    assert_eq!(client.replayed_transactions(), 0);
    assert_eq!(client.revision().unwrap(), remote.revision().unwrap());
    assert_eq!(client.conflicts().unwrap(), accepted.conflicts);
    assert_eq!(client.pending().unwrap(), vec![unrelated]);
    assert_eq!(
        client.commit(&first, &device(1), 200).unwrap().ack,
        accepted_first.ack
    );
    assert_eq!(
        client.commit(&pending, &device(2), 201).unwrap().ack,
        accepted.ack
    );
    let db = query(&path);
    let label: String = db
        .query_row("SELECT label FROM devices", [], |row| row.get(0))
        .unwrap();
    assert_eq!(label, "Local synthetic label");
    let setting: String = db
        .query_row(
            "SELECT value FROM settings WHERE key='synthetic'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(setting, "keep");
    let asset_count: i64 = db
        .query_row("SELECT count(*) FROM assets", [], |row| row.get(0))
        .unwrap();
    assert_eq!(asset_count, 1); // Pending original survives the state replacement.
    let resolved: Option<i64> = db
        .query_row("SELECT resolved_at FROM conflicts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(resolved, Some(103));
    drop(db);
    drop(client);
    temp.close().unwrap();
}

#[test]
fn only_the_exact_durable_receipt_can_remove_pending_bytes() {
    let temp = temporary();
    let path = temp.path().join("client");
    let mut client = create(&path);
    let txn = title(client.revision().unwrap(), 10, 2);
    client.enqueue_pending(&txn).unwrap();
    let receipt = client.commit(&txn, &device(2), 100).unwrap().ack;
    let mut wrong = receipt.clone();
    wrong.state_hash[0] ^= 1;
    assert!(client.acknowledge_pending(&txn, &wrong).is_err());
    let mut wrong = receipt.clone();
    wrong.host_seq += 1;
    assert!(client.acknowledge_pending(&txn, &wrong).is_err());
    assert_eq!(client.pending().unwrap(), vec![txn.clone()]);
    assert!(client.acknowledge_pending(&txn, &receipt).unwrap());
    assert!(!client.acknowledge_pending(&txn, &receipt).unwrap());
    drop(client);
    let client = ProjectStore::open(&path).unwrap();
    assert!(client.pending().unwrap().is_empty());
    assert_eq!(client.revision().unwrap().host_seq, 1);
    drop(client);
    temp.close().unwrap();
}

#[test]
fn checkpoint_keeps_original_inventory_for_assets_added_then_undone() {
    let temp = temporary();
    let path = temp.path().join("client");
    let mut client = create(&path);
    let mut remote = host();
    let original = asset(remote.revision().unwrap(), 10, 8);
    remote.submit(original.clone(), &device(2), 100).unwrap();
    let undo = transaction(
        remote.revision().unwrap(),
        11,
        2,
        pb::op::Kind::UndoTransaction(pb::UndoTransaction {
            target_txn_id: original.txn_id.clone(),
        }),
    );
    remote.submit(undo, &device(2), 101).unwrap();
    assert!(remote.project().assets.is_empty());
    client
        .install_authenticated_checkpoint(&remote.checkpoint_bytes().unwrap(), &device(1), 102)
        .unwrap();
    drop(client);
    let client = ProjectStore::open(&path).unwrap();
    assert_eq!(client.project(), remote.project());
    assert_eq!(
        query(&path)
            .query_row("SELECT count(*) FROM assets", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(client);
    temp.close().unwrap();
}

#[test]
fn stale_foreign_corrupt_and_receipt_divergent_checkpoints_are_atomic() {
    let temp = temporary();
    let path = temp.path().join("client");
    let mut client = create(&path);
    let mut remote = host();
    let stale = remote.checkpoint_bytes().unwrap();
    let first = title(remote.revision().unwrap(), 10, 2);
    remote.submit(first.clone(), &device(2), 100).unwrap();
    client
        .install_authenticated_checkpoint(&remote.checkpoint_bytes().unwrap(), &device(1), 101)
        .unwrap();
    let pending = title(client.revision().unwrap(), 11, 2);
    client.enqueue_pending(&pending).unwrap();
    let before = client.checkpoint_bytes().unwrap();
    assert!(
        client
            .install_authenticated_checkpoint(&stale, &device(1), 102)
            .is_err()
    );
    assert!(
        client
            .install_authenticated_checkpoint(&before, &device(3), 102)
            .is_err()
    );
    let mut corrupt = before.clone();
    corrupt[0] ^= 1;
    assert!(
        client
            .install_authenticated_checkpoint(&corrupt, &device(1), 102)
            .is_err()
    );
    let mut foreign = project();
    foreign.id = id(99);
    let foreign = HostSequencer::new(foreign, device(1))
        .unwrap()
        .checkpoint_bytes()
        .unwrap();
    assert!(
        client
            .install_authenticated_checkpoint(&foreign, &device(1), 102)
            .is_err()
    );
    let foreign_host = HostSequencer::new(project(), device(3))
        .unwrap()
        .checkpoint_bytes()
        .unwrap();
    assert!(
        client
            .install_authenticated_checkpoint(&foreign_host, &device(1), 102)
            .is_err()
    );
    // Same transaction, hash, and visible project, but a different exact receipt time.
    let mut divergent = host();
    divergent.submit(first, &device(2), 999).unwrap();
    assert_eq!(divergent.revision().unwrap(), client.revision().unwrap());
    assert!(
        client
            .install_authenticated_checkpoint(
                &divergent.checkpoint_bytes().unwrap(),
                &device(1),
                102
            )
            .is_err()
    );
    assert_eq!(client.checkpoint_bytes().unwrap(), before);
    drop(client);
    let client = ProjectStore::open(&path).unwrap();
    assert_eq!(client.checkpoint_bytes().unwrap(), before);
    assert_eq!(client.pending().unwrap(), vec![pending]);
    drop(client);
    temp.close().unwrap();
}

#[test]
fn same_project_id_cannot_substitute_a_different_initial_state() {
    let temp = temporary();
    let path = temp.path().join("client");
    let mut client = create(&path);
    let before = client.checkpoint_bytes().unwrap();
    let mut other = project();
    other.title = "Different baseline".into();
    let mut remote = HostSequencer::new(other, device(1)).unwrap();
    let txn = title(remote.revision().unwrap(), 10, 2);
    remote.submit(txn, &device(2), 100).unwrap();
    assert!(
        client
            .install_authenticated_checkpoint(&remote.checkpoint_bytes().unwrap(), &device(1), 101)
            .is_err()
    );
    assert_eq!(client.checkpoint_bytes().unwrap(), before);
    drop(client);
    temp.close().unwrap();
}

#[test]
fn pending_id_collision_rolls_back_the_entire_install() {
    let temp = temporary();
    let path = temp.path().join("client");
    let mut client = create(&path);
    let mut remote = host();
    let pending = title(remote.revision().unwrap(), 10, 2);
    client.enqueue_pending(&pending).unwrap();
    let mut collision = pending.clone();
    if let Some(pb::op::Kind::UpdateDocument(value)) = collision.ops[0].kind.as_mut() {
        value.title = "Different exact bytes".into();
    }
    remote.submit(collision, &device(2), 100).unwrap();
    let before = client.checkpoint_bytes().unwrap();
    assert!(
        client
            .install_authenticated_checkpoint(&remote.checkpoint_bytes().unwrap(), &device(1), 101)
            .is_err()
    );
    drop(client);
    let client = ProjectStore::open(&path).unwrap();
    assert_eq!(client.checkpoint_bytes().unwrap(), before);
    assert_eq!(client.pending().unwrap(), vec![pending]);
    drop(client);
    temp.close().unwrap();
}

#[test]
fn sqlite_integer_failure_after_an_earlier_log_write_rolls_back_everything() {
    let temp = temporary();
    let path = temp.path().join("client");
    let mut client = create(&path);
    let mut remote = host();
    let pending = title(remote.revision().unwrap(), 10, 2);
    client.enqueue_pending(&pending).unwrap();
    remote.submit(pending.clone(), &device(2), 100).unwrap();
    let oversized = asset(remote.revision().unwrap(), 11, u64::MAX);
    remote.submit(oversized, &device(2), 101).unwrap();
    let before = client.checkpoint_bytes().unwrap();
    assert!(
        client
            .install_authenticated_checkpoint(&remote.checkpoint_bytes().unwrap(), &device(1), 102)
            .is_err()
    );
    assert_eq!(
        query(&path)
            .query_row("SELECT count(*) FROM op_log", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(client);
    let client = ProjectStore::open(&path).unwrap();
    assert_eq!(client.checkpoint_bytes().unwrap(), before);
    assert_eq!(client.pending().unwrap(), vec![pending]);
    drop(client);
    temp.close().unwrap();
}

#[test]
fn altered_schema_is_rejected_before_checkpoint_writes() {
    let temp = temporary();
    let path = temp.path().join("client");
    let mut client = create(&path);
    let mut remote = host();
    let pending = title(remote.revision().unwrap(), 10, 2);
    client.enqueue_pending(&pending).unwrap();
    remote.submit(pending.clone(), &device(2), 100).unwrap();
    let before = client.checkpoint_bytes().unwrap();
    let db = query(&path);
    db.execute_batch(
        "CREATE TRIGGER sqlitex AFTER INSERT ON op_log BEGIN DELETE FROM op_log; END;",
    )
    .unwrap();
    assert!(
        client
            .install_authenticated_checkpoint(&remote.checkpoint_bytes().unwrap(), &device(1), 101)
            .is_err()
    );
    db.execute_batch("DROP TRIGGER sqlitex;").unwrap();
    drop(db);
    drop(client);
    let client = ProjectStore::open(&path).unwrap();
    assert_eq!(client.checkpoint_bytes().unwrap(), before);
    assert_eq!(client.pending().unwrap(), vec![pending]);
    drop(client);
    temp.close().unwrap();
}

#[test]
fn checkpoints_cannot_forget_durable_gesture_cancellations() {
    let temp = temporary();
    let path = temp.path().join("client");
    let mut client = create(&path);
    let mut remote = host();
    client.cancel_gesture(device(2), id(77), 1).unwrap();
    let before = client.checkpoint_bytes().unwrap();
    assert!(
        client
            .install_authenticated_checkpoint(&remote.checkpoint_bytes().unwrap(), &device(1), 2)
            .is_err()
    );
    assert_eq!(client.checkpoint_bytes().unwrap(), before);
    remote.cancel_gesture(device(2), id(77)).unwrap();
    client
        .install_authenticated_checkpoint(&remote.checkpoint_bytes().unwrap(), &device(1), 3)
        .unwrap();
    drop(client);
    let mut client = ProjectStore::open(&path).unwrap();
    let mut late = title(client.revision().unwrap(), 10, 2);
    late.gesture_id = Some(id(77).to_proto());
    assert!(client.commit(&late, &device(2), 100).is_err());
    drop(client);
    temp.close().unwrap();
}

#[test]
fn process_exit_at_each_sync_boundary_recovers_both_state_and_outbox_atomically() {
    for stage in [
        CommitStage::Prepared,
        CommitStage::LogWritten,
        CommitStage::BeforeCommit,
        CommitStage::Committed,
    ] {
        let temp = temporary();
        let path = temp.path().join("client");
        let mut client = create(&path);
        let mut remote = host();
        let old = client.revision().unwrap();
        let pending = title(old.clone(), 10, 2);
        client.enqueue_pending(&pending).unwrap();
        let ack = remote.submit(pending.clone(), &device(2), 100).unwrap().ack;
        let checkpoint_path = temp.path().join("host.checkpoint");
        fs::write(&checkpoint_path, remote.checkpoint_bytes().unwrap()).unwrap();
        drop(client);
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "sync_checkpoint_crash_worker"])
            .env("VW_SYNC_CRASH_PROJECT", &path)
            .env("VW_SYNC_CRASH_CHECKPOINT", &checkpoint_path)
            .env("VW_SYNC_CRASH_STAGE", format!("{stage:?}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut next_progress = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("owned sync worker exceeded its bounded run at {stage:?}");
            }
            if Instant::now() >= next_progress {
                eprintln!("waiting for owned sync crash worker at {stage:?}");
                next_progress = Instant::now() + Duration::from_secs(10);
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            status.code(),
            Some(83),
            "worker must exit without running Rust destructors at {stage:?}"
        );
        let mut client = ProjectStore::open(&path).unwrap();
        if stage == CommitStage::Committed {
            assert_eq!(client.revision().unwrap(), remote.revision().unwrap());
            assert!(client.pending().unwrap().is_empty());
            assert_eq!(client.commit(&pending, &device(2), 200).unwrap().ack, ack);
        } else {
            assert_eq!(client.revision().unwrap(), old);
            assert_eq!(client.pending().unwrap(), vec![pending]);
        }
        client.integrity_check().unwrap();
        drop(client);
        temp.close().unwrap();
    }
}

#[test]
#[ignore = "private owned subprocess for process-death checkpoint recovery"]
fn sync_checkpoint_crash_worker() {
    let path = std::env::var_os("VW_SYNC_CRASH_PROJECT").expect("owned project");
    let checkpoint =
        fs::read(std::env::var_os("VW_SYNC_CRASH_CHECKPOINT").expect("owned checkpoint")).unwrap();
    let stage = std::env::var("VW_SYNC_CRASH_STAGE").expect("owned boundary");
    let mut client = ProjectStore::open(Path::new(&path)).unwrap();
    client
        .install_authenticated_checkpoint_observed(&checkpoint, &device(1), 101, |at| {
            if format!("{at:?}") == stage {
                std::process::exit(83);
            }
        })
        .unwrap();
    panic!("requested crash boundary was not reached");
}
