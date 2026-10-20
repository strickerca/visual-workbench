//! Runtime synthetic historical databases; no historical owner data is used.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::{
    fs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;
use vw_model::{AssetId, DeviceId, Document, Id, Layer, Object, Project};
use vw_ops::HostSequencer;
use vw_proto::{Message, v1 as pb};
use vw_store::{ProjectStore, StoreError};

const DRAFT: &str = include_str!("../migrations/001_draft.sql");
const CHECKPOINTS: &str = include_str!("../migrations/002_checkpoints.sql");
const IMAGE_OUTPUT: &str = include_str!("../migrations/003_image_output.sql");
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

struct VersionTwoFixture {
    path: PathBuf,
    host: HostSequencer,
    transactions: Vec<pb::Transaction>,
    pending: pb::Transaction,
}

fn write_checkpoint(database: &rusqlite::Connection, host: &HostSequencer, time: i64) {
    let checkpoint = host.checkpoint_bytes().unwrap();
    database.execute(
        "INSERT INTO snapshots(host_seq,state,state_hash,created_at,checkpoint,checkpoint_hash) VALUES(?1,?2,?3,?4,?5,?6)",
        rusqlite::params![host.revision().unwrap().host_seq as i64,host.project().canonical_bytes().unwrap(),host.project().state_hash().unwrap().to_string(),time,&checkpoint,AssetId::hash(&checkpoint).to_string()],
    ).unwrap();
}

// This fixture must remain independent of ProjectStore::create/open and the
// current schema: it represents a database written by the schema 2 store.
fn version_two(root: &Path) -> VersionTwoFixture {
    let path = root.join("version-two.vwb");
    fs::create_dir(&path).unwrap();
    let database = rusqlite::Connection::open(path.join("project.sqlite")).unwrap();
    database.pragma_update(None, "foreign_keys", "ON").unwrap();
    database.execute_batch(DRAFT).unwrap();
    database.execute_batch(CHECKPOINTS).unwrap();
    database.execute(
        "INSERT INTO meta(key,value) VALUES('host_device',?1),('project_id',?2),('synthetic-note','retain exactly')",
        rusqlite::params![device().to_string(),id(1).to_string()],
    ).unwrap();
    database.execute(
        "INSERT INTO devices(device_id,label,platform,lamport) VALUES(?1,'Synthetic schema two device','windows',11)",
        [device().to_string()],
    ).unwrap();
    database.execute(
        "INSERT INTO settings(key,value) VALUES('synthetic.migration.fixture','{\"preserve\":true}')",
        [],
    ).unwrap();
    let mut initial = project();
    for (index, (kind, order_key)) in [
        ("annotation", "A"),
        ("mask", "M"),
        ("result", "V"),
        ("adjustment", "z"),
    ]
    .into_iter()
    .enumerate()
    {
        let layer_id = id(100 + index as u64);
        let object_id = id(200 + index as u64);
        initial.layers.insert(
            layer_id.clone(),
            Layer {
                definition: pb::CreateLayer {
                    layer_id: Some(layer_id.to_proto()),
                    document_id: Some(id(2).to_proto()),
                    page_index: -1,
                    name: format!("Synthetic {kind}"),
                    kind: kind.into(),
                    order_key: order_key.into(),
                },
                visible: index % 2 == 0,
                locked: index == 1,
                opacity: (index + 1) as f64 / 4.0,
                blend: if index == 2 { "multiply" } else { "normal" }.into(),
            },
        );
        initial.objects.insert(
            object_id.clone(),
            Object {
                document_id: id(2),
                state: pb::ObjectState {
                    object_id: Some(object_id.to_proto()),
                    layer_id: Some(layer_id.to_proto()),
                    order_key: "V".into(),
                    transform: Some(pb::Affine {
                        a: 1.0,
                        d: 1.0,
                        ..Default::default()
                    }),
                    style: Some(pb::Style {
                        stroke: Some(pb::Color { rgba: 0x123456ff }),
                        width: 2.0,
                        ..Default::default()
                    }),
                    role: pb::Role::None as i32,
                    created_by: device().to_string(),
                    created_at_ms: 1,
                    shape: Some(pb::object_state::Shape::Rect(pb::RectD {
                        x: index as f64,
                        y: 2.0,
                        w: 10.0,
                        h: 12.0,
                    })),
                    ..Default::default()
                },
            },
        );
    }
    let mut host = HostSequencer::new(initial, device()).unwrap();
    write_checkpoint(&database, &host, 0);
    let mut transactions = Vec::new();
    for number in [10, 11] {
        let txn = transaction(&host, number);
        let accepted_at = number as i64 + 100;
        let accepted = host.submit(txn.clone(), &device(), accepted_at).unwrap();
        database.execute(
            "INSERT INTO op_log(host_seq,txn_id,device_id,base_host_seq,gesture_id,txn,state_hash,created_at_wall,accepted_at,ack) VALUES(?1,?2,?3,?4,NULL,?5,?6,?7,?8,?9)",
            rusqlite::params![accepted.ack.host_seq as i64,Id::from_proto(txn.txn_id.as_ref()).unwrap().to_string(),txn.device_id,txn.base_revision.as_ref().unwrap().host_seq as i64,txn.encode_to_vec(),host.project().state_hash().unwrap().to_string(),txn.created_at_wall_ms,accepted_at,accepted.ack.encode_to_vec()],
        ).unwrap();
        transactions.push(txn);
    }
    write_checkpoint(&database, &host, 200);
    let document = &host.project().documents[&id(2)];
    database.execute(
        "INSERT INTO documents(document_id,kind,schema_version,title,primary_asset_id,created_at,deleted_at,state) VALUES(?1,'image',1,?2,NULL,?3,NULL,?4)",
        rusqlite::params![id(2).to_string(),document.definition.title,document.created_at_ms,serde_json::to_vec(document).unwrap()],
    ).unwrap();
    for (id, layer) in &host.project().layers {
        let definition = &layer.definition;
        database.execute(
            "INSERT INTO layers(layer_id,document_id,page_index,name,kind,order_key,visible,locked,opacity,blend) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            rusqlite::params![id.to_string(),Id::from_proto(definition.document_id.as_ref()).unwrap().to_string(),definition.page_index,definition.name,definition.kind,definition.order_key,layer.visible,layer.locked,layer.opacity,layer.blend],
        ).unwrap();
    }
    for (id, object) in &host.project().objects {
        database.execute(
            "INSERT INTO objects(object_id,document_id,layer_id,kind,order_key,role,state,updated_seq) VALUES(?1,?2,?3,'rect',?4,'none',?5,1)",
            rusqlite::params![id.to_string(),object.document_id.to_string(),Id::from_proto(object.state.layer_id.as_ref()).unwrap().to_string(),object.state.order_key,object.state.encode_to_vec()],
        ).unwrap();
    }
    let pending = transaction(&host, 1000);
    database.execute(
        "INSERT INTO pending_txns(txn_id,base_host_seq,txn,created_at_wall) VALUES(?1,?2,?3,?4)",
        rusqlite::params![id(1000).to_string(),pending.base_revision.as_ref().unwrap().host_seq as i64,pending.encode_to_vec(),pending.created_at_wall_ms],
    ).unwrap();
    assert!(query_rows(&database, "PRAGMA foreign_key_check").is_empty());
    drop(database);
    VersionTwoFixture {
        path,
        host,
        transactions,
        pending,
    }
}

fn query_rows(database: &rusqlite::Connection, sql: &str) -> Vec<Vec<rusqlite::types::Value>> {
    let mut query = database.prepare(sql).unwrap();
    let count = query.column_count();
    query
        .query_map([], |row| {
            (0..count)
                .map(|column| row.get(column))
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn stored_rows(database: &rusqlite::Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    // Include historical inventory, local rows and exact protobuf BLOBs. Only
    // the schema-version value is expected to change during this migration.
    [
        "SELECT * FROM meta WHERE key!='schema_version' ORDER BY key",
        "SELECT * FROM devices ORDER BY device_id",
        "SELECT * FROM assets ORDER BY asset_id",
        "SELECT * FROM documents ORDER BY document_id",
        "SELECT * FROM captures ORDER BY document_id",
        "SELECT * FROM layers ORDER BY layer_id",
        "SELECT * FROM objects ORDER BY object_id",
        "SELECT * FROM op_log ORDER BY host_seq",
        "SELECT * FROM pending_txns ORDER BY local_seq",
        "SELECT * FROM snapshots ORDER BY host_seq",
        "SELECT * FROM instructions ORDER BY instruction_id",
        "SELECT * FROM semantic_snapshots ORDER BY snapshot_id",
        "SELECT * FROM packages ORDER BY package_id",
        "SELECT * FROM results ORDER BY result_id",
        "SELECT * FROM conflicts ORDER BY conflict_id",
        "SELECT * FROM settings ORDER BY key",
        "SELECT * FROM groups ORDER BY group_id",
        "SELECT * FROM pdf_pages ORDER BY document_id,page_index",
        "SELECT * FROM mask_versions ORDER BY object_id",
    ]
    .into_iter()
    .map(|sql| query_rows(database, sql))
    .collect()
}

fn schema_version(database: &rusqlite::Connection) -> String {
    database
        .query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap()
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
    upgraded.pragma_update(None, "foreign_keys", "ON").unwrap();
    upgraded.execute_batch(DRAFT).unwrap();
    upgraded.execute_batch(CHECKPOINTS).unwrap();
    upgraded.execute_batch(IMAGE_OUTPUT).unwrap();
    assert_eq!(schema(&fresh), schema(&upgraded));
    assert_eq!(
        fresh
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "3"
    );
}

#[test]
fn version_two_upgrade_preserves_exact_history_local_data_and_linked_projection_rows() {
    let temporary = temporary();
    let fixture = version_two(temporary.path());
    let expected_revision = fixture.host.revision().unwrap();
    let expected_checkpoint = fixture.host.checkpoint_bytes().unwrap();
    let database = rusqlite::Connection::open(fixture.path.join("project.sqlite")).unwrap();
    let historical = rusqlite::Connection::open_in_memory().unwrap();
    historical.execute_batch(DRAFT).unwrap();
    historical.execute_batch(CHECKPOINTS).unwrap();
    assert_eq!(schema_version(&database), "2");
    assert_eq!(schema(&database), schema(&historical));
    let before = stored_rows(&database);
    drop(database);

    let mut store = ProjectStore::open(&fixture.path).unwrap();
    assert_eq!(store.revision().unwrap(), expected_revision);
    assert_eq!(store.project(), fixture.host.project());
    assert_eq!(store.checkpoint_bytes().unwrap(), expected_checkpoint);
    assert_eq!(store.pending().unwrap(), vec![fixture.pending.clone()]);
    assert_eq!(store.replayed_transactions(), 0);
    let database = rusqlite::Connection::open(fixture.path.join("project.sqlite")).unwrap();
    assert_eq!(schema_version(&database), "3");
    assert_eq!(stored_rows(&database), before);
    let fresh = rusqlite::Connection::open_in_memory().unwrap();
    fresh.execute_batch(CURRENT).unwrap();
    assert_eq!(schema(&database), schema(&fresh));
    assert!(query_rows(&database, "PRAGMA foreign_key_check").is_empty());
    assert_eq!(
        query_rows(
            &database,
            "SELECT \"table\",\"from\",\"to\" FROM pragma_foreign_key_list('objects') ORDER BY \"from\""
        ),
        vec![
            vec![
                rusqlite::types::Value::Text("documents".into()),
                rusqlite::types::Value::Text("document_id".into()),
                rusqlite::types::Value::Text("document_id".into())
            ],
            vec![
                rusqlite::types::Value::Text("layers".into()),
                rusqlite::types::Value::Text("layer_id".into()),
                rusqlite::types::Value::Text("layer_id".into())
            ],
        ]
    );
    database.pragma_update(None, "foreign_keys", "ON").unwrap();
    assert!(
        database
            .execute(
                "DELETE FROM layers WHERE layer_id=?1",
                [id(100).to_string()]
            )
            .is_err()
    );
    assert!(
        database
            .execute(
                "UPDATE objects SET layer_id=?1 WHERE object_id=?2",
                rusqlite::params![id(9999).to_string(), id(200).to_string()]
            )
            .is_err()
    );
    assert_eq!(stored_rows(&database), before);
    drop(database);
    for txn in &fixture.transactions {
        let retry = store.commit(txn, &device(), 300).unwrap();
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
    store.integrity_check().unwrap();
    drop(store);
    let reopened = ProjectStore::open(&fixture.path).unwrap();
    assert_eq!(reopened.revision().unwrap(), expected_revision);
    assert_eq!(reopened.checkpoint_bytes().unwrap(), expected_checkpoint);
    assert_eq!(reopened.pending().unwrap(), vec![fixture.pending]);
    drop(reopened);
    temporary.close().unwrap();
}

#[test]
fn malformed_version_two_is_refused_without_partial_version_or_table_changes() {
    for sql in [
        "ALTER TABLE layers ADD COLUMN unexpected_fixture TEXT",
        "CREATE TRIGGER sqlitex AFTER INSERT ON objects BEGIN DELETE FROM objects; END",
        "UPDATE snapshots SET checkpoint=x'00' WHERE host_seq=2",
        "UPDATE snapshots SET state_hash='0000000000000000000000000000000000000000000000000000000000000000' WHERE host_seq=2",
        "UPDATE op_log SET ack=x'ff' WHERE host_seq=1",
        "UPDATE op_log SET accepted_at=accepted_at+1 WHERE host_seq=1",
        "UPDATE pending_txns SET txn=x'ff'",
        "UPDATE documents SET title='Corrupted projection'",
        "UPDATE objects SET state=x'ff'",
        "PRAGMA foreign_keys=OFF; UPDATE objects SET layer_id='missing-synthetic-layer'",
    ] {
        let temporary = temporary();
        let fixture = version_two(temporary.path());
        let database = rusqlite::Connection::open(fixture.path.join("project.sqlite")).unwrap();
        database.execute_batch(sql).unwrap();
        let before_schema = schema(&database);
        let before_rows = stored_rows(&database);
        drop(database);
        assert!(ProjectStore::open(&fixture.path).is_err(), "accepted {sql}");
        let database = rusqlite::Connection::open(fixture.path.join("project.sqlite")).unwrap();
        assert_eq!(schema_version(&database), "2", "changed version for {sql}");
        assert_eq!(schema(&database), before_schema, "changed schema for {sql}");
        assert_eq!(
            stored_rows(&database),
            before_rows,
            "changed rows for {sql}"
        );
        drop(database);
        temporary.close().unwrap();
    }
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
fn orphan_historical_asset_failure_rolls_back_migration_before_committing_current_version() {
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
