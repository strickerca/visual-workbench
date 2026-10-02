//! Runtime synthetic draft databases; no historical owner data is used.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::{
    fs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;
use vw_model::{AssetId, DeviceId, Document, Id, Project};
use vw_ops::HostSequencer;
use vw_proto::{Message, v1 as pb};
use vw_store::{ProjectStore, StoreError};

const DRAFT: &str = include_str!("../migrations/001_draft.sql");
const UPGRADE: &str = include_str!("../migrations/002_checkpoints.sql");
const CURRENT: &str = include_str!("../../../../contracts/storage.sql");

fn temporary() -> TempDir {
    let mut builder = tempfile::Builder::new();
    builder.prefix("vw-migrations-test-");
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
fn id(number: u64) -> Id {
    Id::from_parts(1_700_000_000_000 + number, [9; 10]).unwrap()
}
fn device() -> DeviceId {
    DeviceId::from_bytes([2; 16])
}
fn project() -> Project {
    let mut project = Project::new(id(1), "Synthetic legacy fixture".into(), device());
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
fn transaction(host: &HostSequencer, number: u64) -> pb::Transaction {
    pb::Transaction {
        txn_id: Some(id(number).to_proto()),
        project_id: Some(id(1).to_proto()),
        device_id: device().to_string(),
        base_revision: Some(host.revision().unwrap()),
        created_at_wall_ms: number as i64,
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: device().to_string(),
                lamport: number,
            }),
            kind: Some(pb::op::Kind::UpdateDocument(pb::UpdateDocument {
                document_id: Some(id(2).to_proto()),
                title: format!("Legacy edit {number}"),
            })),
        }],
        ..Default::default()
    }
}
struct DraftFixture {
    path: PathBuf,
    host: HostSequencer,
    transactions: Vec<pb::Transaction>,
    pending: pb::Transaction,
}
fn draft(root: &Path, count: u64) -> DraftFixture {
    let path = root.join("draft.vwb");
    fs::create_dir(&path).unwrap();
    let database = rusqlite::Connection::open(path.join("project.sqlite")).unwrap();
    database.execute_batch(DRAFT).unwrap();
    database
        .execute(
            "INSERT INTO meta(key,value) VALUES('host_device',?1),('project_id',?2)",
            rusqlite::params![device().to_string(), id(1).to_string()],
        )
        .unwrap();
    database.execute("INSERT INTO devices(device_id,label,platform,lamport) VALUES(?1,'Synthetic legacy device','windows',0)", [device().to_string()]).unwrap();
    let mut host = HostSequencer::new(project(), device()).unwrap();
    database
        .execute(
            "INSERT INTO snapshots(host_seq,state,state_hash,created_at) VALUES(0,?1,?2,0)",
            rusqlite::params![
                host.project().canonical_bytes().unwrap(),
                host.project().state_hash().unwrap().to_string()
            ],
        )
        .unwrap();
    let mut transactions = Vec::new();
    for index in 0..count {
        let txn = transaction(&host, 10 + index);
        let acceptance = host
            .submit(txn.clone(), &device(), 100 + index as i64)
            .unwrap();
        database.execute("INSERT INTO op_log(host_seq,txn_id,device_id,base_host_seq,gesture_id,txn,state_hash,created_at_wall,accepted_at) VALUES(?1,?2,?3,?4,NULL,?5,?6,?7,?8)",
            rusqlite::params![acceptance.ack.host_seq as i64,Id::from_proto(txn.txn_id.as_ref()).unwrap().to_string(),txn.device_id,txn.base_revision.as_ref().unwrap().host_seq as i64,txn.encode_to_vec(),host.project().state_hash().unwrap().to_string(),txn.created_at_wall_ms,100 + index as i64]).unwrap();
        transactions.push(txn);
    }
    if count != 0 {
        database
            .execute(
                "INSERT INTO snapshots(host_seq,state,state_hash,created_at) VALUES(?1,?2,?3,100)",
                rusqlite::params![
                    count as i64,
                    host.project().canonical_bytes().unwrap(),
                    host.project().state_hash().unwrap().to_string()
                ],
            )
            .unwrap();
    }
    let pending = transaction(&host, 1000);
    database.execute("INSERT INTO pending_txns(txn_id,base_host_seq,txn,created_at_wall) VALUES(?1,?2,?3,?4)", rusqlite::params![id(1000).to_string(),pending.base_revision.as_ref().unwrap().host_seq as i64,pending.encode_to_vec(),pending.created_at_wall_ms]).unwrap();
    drop(database);
    DraftFixture {
        path,
        host,
        transactions,
        pending,
    }
}
fn schema(database: &rusqlite::Connection) -> Vec<(String, String, String, String)> {
    database.prepare("SELECT type,name,tbl_name,sql FROM sqlite_master WHERE name NOT GLOB 'sqlite_*' ORDER BY type,name").unwrap()
        .query_map([], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get::<_, String>(3)?.chars().filter(|ch| !ch.is_whitespace() && *ch != '"').collect()))).unwrap()
        .collect::<Result<_,_>>().unwrap()
}
fn assert_still_draft(path: &Path) {
    let database = rusqlite::Connection::open(path.join("project.sqlite")).unwrap();
    assert_eq!(
        database
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "1"
    );
    let columns: Vec<String> = database
        .prepare("PRAGMA table_info(snapshots)")
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!columns.iter().any(|column| column == "checkpoint"));
    assert_eq!(
        database
            .query_row("SELECT label FROM devices LIMIT 1", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "Synthetic legacy device"
    );
}

#[test]
fn fresh_schema_and_numbered_migrations_produce_identical_sqlite_objects() {
    let fresh = rusqlite::Connection::open_in_memory().unwrap();
    fresh.execute_batch(CURRENT).unwrap();
    let upgraded = rusqlite::Connection::open_in_memory().unwrap();
    upgraded.execute_batch(DRAFT).unwrap();
    upgraded.execute_batch(UPGRADE).unwrap();
    assert_eq!(schema(&fresh), schema(&upgraded));
    assert_eq!(
        fresh
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "2"
    );
}

#[test]
fn draft_migration_replays_history_preserves_hash_retries_labels_and_pending_queue() {
    for count in [0, 3] {
        let temporary = temporary();
        let fixture = draft(temporary.path(), count);
        let expected = fixture.host.revision().unwrap();
        let mut store = ProjectStore::open(&fixture.path).unwrap();
        assert_eq!(store.revision().unwrap(), expected);
        assert_eq!(store.project(), fixture.host.project());
        assert_eq!(store.replayed_transactions(), 0);
        assert_eq!(store.pending().unwrap(), vec![fixture.pending]);
        for txn in &fixture.transactions {
            let retry = store.commit(txn, &device(), 200).unwrap();
            assert!(retry.duplicate);
            assert_eq!(
                &retry.ack,
                fixture
                    .host
                    .accepted_transaction(&Id::from_proto(txn.txn_id.as_ref()).unwrap())
                    .unwrap()
                    .1
            );
        }
        let database = rusqlite::Connection::open(fixture.path.join("project.sqlite")).unwrap();
        assert_eq!(
            database
                .query_row("SELECT label FROM devices LIMIT 1", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "Synthetic legacy device"
        );
        let fresh = rusqlite::Connection::open_in_memory().unwrap();
        fresh.execute_batch(CURRENT).unwrap();
        assert_eq!(schema(&database), schema(&fresh));
        drop(database);
        drop(store);
        let reopened = ProjectStore::open(&fixture.path).unwrap();
        assert_eq!(reopened.revision().unwrap(), expected);
        drop(reopened);
        temporary.close().unwrap();
    }
}

#[test]
fn incomplete_or_corrupted_drafts_fail_without_partial_schema_upgrade() {
    for sql in [
        "DELETE FROM snapshots WHERE host_seq=0",
        "DELETE FROM meta WHERE key='host_device'",
        "DELETE FROM meta WHERE key='project_id'",
        "UPDATE meta SET value='018bcfe5-6863-7909-8909-090909090909' WHERE key='project_id'",
        "UPDATE op_log SET txn=x'ff' WHERE host_seq=1",
        "UPDATE snapshots SET state_hash='0000000000000000000000000000000000000000000000000000000000000000' WHERE host_seq=1",
    ] {
        let temporary = temporary();
        let fixture = draft(temporary.path(), 1);
        let database = rusqlite::Connection::open(fixture.path.join("project.sqlite")).unwrap();
        database.execute_batch(sql).unwrap();
        drop(database);
        assert!(ProjectStore::open(&fixture.path).is_err());
        assert_still_draft(&fixture.path);
        temporary.close().unwrap();
    }
}

#[test]
fn orphan_historical_asset_failure_rolls_back_migration_before_committing_version_two() {
    let temporary = temporary();
    let fixture = draft(temporary.path(), 1);
    let database = rusqlite::Connection::open(fixture.path.join("project.sqlite")).unwrap();
    database.execute("INSERT INTO assets(asset_id,format,width,height,orientation,bit_depth,byte_size,source,metadata_json) VALUES(?1,'png',1,1,1,8,17,'import','{}')", [AssetId::hash(b"orphan synthetic asset").to_string()]).unwrap();
    drop(database);
    assert!(ProjectStore::open(&fixture.path).is_err());
    assert_still_draft(&fixture.path);
    temporary.close().unwrap();
}

#[test]
fn future_schema_is_rejected_without_rewriting_version_or_project() {
    let temporary = temporary();
    let path = temporary.path().join("future.vwb");
    let store = ProjectStore::create(&path, project(), device(), 0).unwrap();
    let before = store.project().state_hash().unwrap();
    drop(store);
    let database = rusqlite::Connection::open(path.join("project.sqlite")).unwrap();
    database
        .execute("UPDATE meta SET value='99' WHERE key='schema_version'", [])
        .unwrap();
    drop(database);
    assert!(matches!(
        ProjectStore::open(&path),
        Err(StoreError::UnsupportedSchema(99))
    ));
    let database = rusqlite::Connection::open(path.join("project.sqlite")).unwrap();
    assert_eq!(
        database
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "99"
    );
    assert_eq!(
        database
            .query_row(
                "SELECT state_hash FROM snapshots WHERE host_seq=0",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        before.to_string()
    );
    drop(database);
    temporary.close().unwrap();
}

#[test]
fn altered_schema_and_history_deleting_triggers_are_rejected_on_open() {
    for sql in [
        "CREATE TRIGGER sqlitex AFTER INSERT ON op_log BEGIN DELETE FROM op_log; END;",
        "ALTER TABLE devices ADD COLUMN unexpected_private_field TEXT;",
        "CREATE TABLE unexpected_fixture(payload BLOB);",
    ] {
        let temporary = temporary();
        let path = temporary.path().join("altered.vwb");
        let store = ProjectStore::create(&path, project(), device(), 0).unwrap();
        drop(store);
        let database = rusqlite::Connection::open(path.join("project.sqlite")).unwrap();
        database.execute_batch(sql).unwrap();
        drop(database);
        assert!(ProjectStore::open(&path).is_err());
        let database = rusqlite::Connection::open(path.join("project.sqlite")).unwrap();
        assert_eq!(
            database
                .query_row("SELECT COUNT(*) FROM op_log", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(database);
        temporary.close().unwrap();
    }
}
