//! A commit writes only the rows of the entities it touched. Opening a project
//! compares every stored row with a full rebuild from the restored state, so
//! reopening after each kind of change proves the two stay equal.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::Path;
use vw_model::{AssetId, DeviceId, Document, Id, Layer, Project};
use vw_proto::{
    Message,
    v1::{self as pb, object_state::Shape, op::Kind, property_value::Value},
};
use vw_store::ProjectStore;

const CLOCK: u64 = 1_700_000_000_000;
fn id(n: u64) -> Id {
    Id::from_parts(CLOCK + n, [0x41; 10]).unwrap()
}
fn uid(n: u64) -> Option<pb::Uuid> {
    Some(id(n).to_proto())
}
fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
fn asset(seed: &[u8]) -> pb::AddAsset {
    pb::AddAsset {
        asset_id: AssetId::hash(seed).to_string(),
        format: "png".into(),
        width: 128,
        height: 64,
        orientation: 1,
        bit_depth: 8,
        color_space: "sRGB".into(),
        byte_size: 26,
        source: "import".into(),
        metadata_json: "{}".into(),
        ..Default::default()
    }
}
fn project() -> Project {
    let mut value = Project::new(id(1), "Touched rows fixture".into(), device(1));
    let original = asset(b"original");
    value.assets.insert(
        AssetId::try_from(original.asset_id.clone()).unwrap(),
        original.clone(),
    );
    value.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: uid(2),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Before".into(),
                primary_asset_id: original.asset_id,
                capture: None,
            },
            pages: vec![],
            created_at_ms: 0,
        },
    );
    value.layers.insert(
        id(3),
        Layer {
            definition: pb::CreateLayer {
                layer_id: uid(3),
                document_id: uid(2),
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
    value
}
fn layer(n: u64) -> Kind {
    Kind::CreateLayer(pb::CreateLayer {
        layer_id: uid(n),
        document_id: uid(2),
        page_index: -1,
        name: "Second layer".into(),
        kind: "annotation".into(),
        order_key: "k".into(),
    })
}
fn object(n: u64, layer: u64, shape: Shape) -> Kind {
    Kind::CreateObject(pb::CreateObject {
        document_id: uid(2),
        state: Some(pb::ObjectState {
            object_id: uid(n),
            layer_id: uid(layer),
            order_key: "V".into(),
            transform: Some(pb::Affine {
                a: 1.0,
                d: 1.0,
                e: n as f64,
                ..Default::default()
            }),
            style: Some(pb::Style {
                stroke: Some(pb::Color { rgba: 0x112233ff }),
                width: 1.0,
                ..Default::default()
            }),
            role: pb::Role::None as i32,
            created_by: device(1).to_string(),
            created_at_ms: CLOCK as i64,
            shape: Some(shape),
            ..Default::default()
        }),
    })
}
fn rect() -> Shape {
    Shape::Rect(pb::RectD {
        x: 0.25,
        y: 1.5,
        w: 12.0,
        h: 9.0,
    })
}
fn mask() -> Shape {
    Shape::SelectionVector(pb::Polyline {
        points: (0..3)
            .map(|n| pb::PointD {
                x: f64::from(n),
                y: f64::from(n * n),
            })
            .collect(),
        closed: true,
    })
}
fn property(n: u64, name: &str, value: Value) -> Kind {
    Kind::SetProperty(pb::SetProperty {
        object_id: uid(n),
        property: name.into(),
        value: Some(pb::PropertyValue { value: Some(value) }),
    })
}
fn undo(target: u64) -> Kind {
    Kind::UndoTransaction(pb::UndoTransaction {
        target_txn_id: uid(10_000 + target),
    })
}
fn transaction(store: &ProjectStore, n: u64, kinds: Vec<Kind>) -> pb::Transaction {
    pb::Transaction {
        txn_id: uid(10_000 + n),
        project_id: uid(1),
        device_id: device(1).to_string(),
        base_revision: Some(store.revision().unwrap()),
        created_at_wall_ms: n as i64,
        ops: kinds
            .into_iter()
            .enumerate()
            .map(|(i, kind)| pb::Op {
                op_id: Some(pb::OpId {
                    device_id: device(1).to_string(),
                    lamport: n * 100 + i as u64 + 1,
                }),
                kind: Some(kind),
            })
            .collect(),
        gesture_id: None,
    }
}

/// Every operation kind that writes or removes a projection row, with undo
/// and redo of the structural ones.
fn script() -> Vec<Vec<Kind>> {
    let result = asset(b"result");
    vec![
        // 2..=4: objects and a second layer
        vec![object(10, 3, rect())],
        vec![object(11, 3, rect()), object(12, 3, mask())],
        vec![layer(4), object(13, 4, rect()), object(14, 4, mask())],
        // 5..=8: properties, order and layer changes
        vec![property(10, "style.width", Value::Number(5.0))],
        vec![Kind::Reorder(pb::Reorder {
            object_id: uid(11),
            layer_id: uid(4),
            order_key: "k".into(),
        })],
        vec![Kind::UpdateLayer(pb::UpdateLayer {
            layer_id: uid(4),
            name: Some("Renamed".into()),
            opacity: Some(0.25),
            ..Default::default()
        })],
        vec![Kind::UpdateDocument(pb::UpdateDocument {
            document_id: uid(2),
            title: "After".into(),
        })],
        // 9..=12: groups, instructions and mask history
        vec![Kind::Group(pb::Group {
            group_id: uid(20),
            object_ids: vec![id(10).to_proto(), id(11).to_proto()],
        })],
        vec![Kind::SetInstruction(pb::SetInstruction {
            instruction_id: uid(21),
            document_id: uid(2),
            target_object_ids: vec![id(10).to_proto()],
            role: pb::Role::Change as i32,
            text: "Synthetic instruction".into(),
            entry_method: "pc_keyboard".into(),
            language: "en".into(),
        })],
        vec![Kind::MaskOp(pb::MaskOp {
            op: "add".into(),
            input_object_ids: vec![id(14).to_proto()],
            output_object_id: uid(12),
            amount: 0.0,
        })],
        vec![Kind::MaskOp(pb::MaskOp {
            op: "invert".into(),
            input_object_ids: vec![],
            output_object_id: uid(12),
            amount: 0.0,
        })],
        // 13..=15: an asset, a result and its status
        vec![
            Kind::AddAsset(result.clone()),
            Kind::AddResult(pb::AddResult {
                result_id: uid(22),
                document_id: uid(2),
                provider: "synthetic-provider".into(),
                model: "synthetic-model".into(),
                request_json: "{}".into(),
                ..Default::default()
            }),
        ],
        vec![Kind::UpdateResult(pb::UpdateResult {
            result_id: uid(22),
            status: "rejected".into(),
            acceptance_mask_asset_id: String::new(),
        })],
        vec![object(15, 3, Shape::ResultId(id(22).to_proto()))],
        // 16..=19: removals
        vec![Kind::DeleteObject(pb::DeleteObject { object_id: uid(15) })],
        vec![Kind::DeleteInstruction(pb::DeleteInstruction {
            instruction_id: uid(21),
        })],
        vec![Kind::Ungroup(pb::Ungroup { group_id: uid(20) })],
        vec![Kind::DeleteLayer(pb::DeleteLayer { layer_id: uid(4) })],
        // 20..=22: undo the layer removal, redo it, undo it again
        vec![undo(19)],
        vec![undo(20)],
        vec![undo(21)],
        // 23..=25: undo the group removal, the instruction removal, a creation
        vec![undo(18)],
        vec![undo(17)],
        vec![undo(2)],
    ]
}

fn rows(path: &Path, sql: &str) -> Vec<(String, i64)> {
    let database = rusqlite::Connection::open(path.join("project.sqlite")).unwrap();
    let mut query = database.prepare(sql).unwrap();
    query
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn every_kind_of_change_leaves_rows_equal_to_a_full_rebuild() {
    let root = tempfile::Builder::new()
        .prefix("vw-touched-rows-")
        .tempdir()
        .unwrap();
    let path = root.path().join("project.vwb");
    let mut store = ProjectStore::create(&path, project(), device(1), 0).unwrap();
    for (step, kinds) in script().into_iter().enumerate() {
        let n = step as u64 + 2;
        let txn = transaction(&store, n, kinds);
        let accepted = store.commit(&txn, &device(1), n as i64).unwrap();
        assert!(!accepted.duplicate, "step {n}");
        assert_eq!(
            store.state_hash().unwrap(),
            &store.project().state_hash().unwrap(),
            "carried hash at step {n}"
        );
        assert_eq!(
            accepted.ack.state_hash,
            store.state_hash().unwrap().bytes().to_vec()
        );
        store.integrity_check().unwrap();
        // Every fifth step also stores a checkpoint, so that opening loads
        // one that holds each kind of change, journal and all, and replays
        // the rest on top of it.
        if n.is_multiple_of(5) {
            store.checkpoint_now(n as i64).unwrap();
        }
        let revision = store.revision().unwrap();
        let checkpoint = store.checkpoint_bytes().unwrap();
        drop(store);
        // Open verifies the checkpoint, replays the log and compares every
        // projection row with a rebuild from the restored project.
        let reopened = ProjectStore::open(&path);
        assert!(
            reopened.is_ok(),
            "step {n} left rows a full rebuild does not produce: {:?}",
            reopened.as_ref().err()
        );
        store = reopened.unwrap();
        assert_eq!(store.revision().unwrap(), revision, "step {n}");
        assert_eq!(store.checkpoint_bytes().unwrap(), checkpoint, "step {n}");
        // Steps since the last stored checkpoint were replayed, no more.
        assert_eq!(
            store.replayed_transactions(),
            if n < 5 { n - 1 } else { n % 5 },
            "step {n}"
        );
    }
    // The script ended with the layer and group restored and object 10 gone.
    let project = store.project();
    assert!(project.layers.contains_key(&id(4)));
    assert!(project.groups.contains_key(&id(20)));
    assert!(!project.objects.contains_key(&id(10)));
    let sequence = store.revision().unwrap().host_seq as i64;
    drop(store);

    // An object row keeps the sequence of the last transaction that touched
    // it, and removed entities have no row.
    let objects = rows(
        &path,
        "SELECT object_id,updated_seq FROM objects ORDER BY object_id",
    );
    assert!(objects.iter().all(|(_, seq)| (1..=sequence).contains(seq)));
    assert!(objects.iter().any(|(_, seq)| *seq < sequence));
    assert!(!objects.iter().any(|(key, _)| key == id(10).as_str()));
    assert_eq!(
        rows(&path, "SELECT layer_id,1 FROM layers ORDER BY layer_id").len(),
        2
    );
    assert_eq!(rows(&path, "SELECT group_id,1 FROM groups").len(), 1);
    assert_eq!(
        rows(&path, "SELECT instruction_id,1 FROM instructions").len(),
        1
    );
    assert_eq!(rows(&path, "SELECT result_id,1 FROM results").len(), 1);
    assert_eq!(
        rows(&path, "SELECT object_id,1 FROM mask_versions").len(),
        1
    );
}

/// A project opens by replaying its log, so every later build has to accept
/// each stored transaction with the acknowledgement and state hash the build
/// that stored it gave. The seal is a digest of exactly those stored bytes.
/// A change to this value means projects already on disk stop opening: it
/// needs a migration, not a new constant.
#[test]
fn the_stored_log_of_every_kind_of_change_is_pinned() {
    let root = tempfile::Builder::new()
        .prefix("vw-touched-rows-")
        .tempdir()
        .unwrap();
    let path = root.path().join("project.vwb");
    let mut store = ProjectStore::create(&path, project(), device(1), 0).unwrap();
    for (step, kinds) in script().into_iter().enumerate() {
        let n = step as u64 + 2;
        let txn = transaction(&store, n, kinds);
        store.commit(&txn, &device(1), n as i64).unwrap();
    }
    drop(store);
    let seal: String = rusqlite::Connection::open(path.join("project.sqlite"))
        .unwrap()
        .query_row("SELECT value FROM meta WHERE key='log_seal'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        seal,
        "v1 24 416d6e10a7e1a0b16ae2b2794f007663c6609d2362d7c6905b0068c2393024b5"
    );
}

#[test]
fn a_commit_sqlite_refuses_leaves_the_store_and_its_rows_unchanged() {
    let root = tempfile::Builder::new()
        .prefix("vw-touched-rows-")
        .tempdir()
        .unwrap();
    let path = root.path().join("project.vwb");
    let mut store = ProjectStore::create(&path, project(), device(1), 0).unwrap();
    for (step, kinds) in script().into_iter().take(4).enumerate() {
        let n = step as u64 + 2;
        let txn = transaction(&store, n, kinds);
        store.commit(&txn, &device(1), n as i64).unwrap();
    }
    let before = store.checkpoint_bytes().unwrap();
    let rows_before = rows(
        &path,
        "SELECT object_id,updated_seq FROM objects ORDER BY object_id",
    );
    // Removing a layer touches a layer row, object rows, tombstones and write
    // stamps; a second writer makes SQLite refuse the commit.
    let txn = transaction(
        &store,
        50,
        vec![Kind::DeleteLayer(pb::DeleteLayer { layer_id: uid(4) })],
    );
    let blocker = rusqlite::Connection::open(path.join("project.sqlite")).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(store.commit(&txn, &device(1), 50).is_err());
    blocker.execute_batch("ROLLBACK").unwrap();
    drop(blocker);
    assert_eq!(store.checkpoint_bytes().unwrap(), before);
    assert_eq!(
        store.state_hash().unwrap(),
        &store.project().state_hash().unwrap()
    );
    assert_eq!(
        rows(
            &path,
            "SELECT object_id,updated_seq FROM objects ORDER BY object_id"
        ),
        rows_before
    );
    // The same transaction then commits, and the result reopens.
    let accepted = store.commit(&txn, &device(1), 50).unwrap();
    assert_eq!(
        accepted.ack.encode_to_vec(),
        store
            .commit(&txn, &device(1), 99)
            .unwrap()
            .ack
            .encode_to_vec()
    );
    assert!(!store.project().layers.contains_key(&id(4)));
    drop(store);
    let store = ProjectStore::open(&path).unwrap();
    assert!(!store.project().objects.contains_key(&id(13)));
}
