//! A project's history has a limit: what one checkpoint may hold, 256 MB.
//! A checkpoint is what a joining device is given and what an older build
//! writes when it saves, so a history beyond it could no longer be joined or
//! saved there. Saving used to write a checkpoint and was refused at the
//! limit by that; it no longer writes one, so the size is kept up to date by
//! every change and the save that would cross the limit is refused from it.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::Path;
use vw_model::{AssetId, DeviceId, Document, Id, Layer, Project};
use vw_ops::{HostSequencer, OpsError};
use vw_proto::v1::{self as pb, object_state::Shape, op::Kind, property_value::Value};
use vw_store::{ProjectStore, StoreError};

const CLOCK: u64 = 1_700_000_000_000;
fn id(n: u64) -> Id {
    Id::from_parts(CLOCK + n, [0x71; 10]).unwrap()
}
fn uid(n: u64) -> Option<pb::Uuid> {
    Some(id(n).to_proto())
}
fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
fn project() -> Project {
    let mut value = Project::new(id(1), "History limit fixture".into(), device(1));
    value.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: uid(2),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Before".into(),
                ..Default::default()
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
/// A stroke with 32-bit pressures and tilts and zeros among its numbers: what
/// a checkpoint writes longer than the canonical bytes do. `zero` is one more
/// of them, of either sign.
fn stroke(n: u64, author: &DeviceId) -> Kind {
    stroke_with(n, author, 0.0)
}
fn stroke_with(n: u64, author: &DeviceId, zero: f64) -> Kind {
    Kind::CreateObject(pb::CreateObject {
        document_id: uid(2),
        state: Some(pb::ObjectState {
            object_id: uid(1000 + n),
            layer_id: uid(3),
            order_key: "V".into(),
            transform: Some(pb::Affine {
                a: 1.0,
                d: 1.0,
                e: n as f64,
                f: zero,
                ..Default::default()
            }),
            style: Some(pb::Style {
                stroke: Some(pb::Color { rgba: 0x112233ff }),
                width: 2.0,
                ..Default::default()
            }),
            role: pb::Role::None as i32,
            created_by: author.to_string(),
            created_at_ms: CLOCK as i64,
            shape: Some(Shape::Stroke(pb::Stroke {
                x: (0..24).map(|i| n as f64 + f64::from(i) * 1.25).collect(),
                y: (0..24).map(|i| f64::from(i) * 0.5).collect(),
                t_ms: (0..24).map(|i| i * 4).collect(),
                pressure: (0..24).map(|i| 0.1 + (i % 7) as f32 * 0.125).collect(),
                tilt: (0..24).map(|i| (i % 3) as f32 * 0.3).collect(),
                orientation: vec![],
                brush: Some(pb::Brush {
                    family: "pen".into(),
                    algorithm_version: 1,
                    base_width: 4.0,
                    pressure_curve: vec![0.0, 0.0, 0.25, 0.25, 0.75, 0.75, 1.0, 1.0],
                    stabilization: 0.0,
                }),
            })),
            ..Default::default()
        }),
    })
}
fn moved(n: u64, by: f64) -> Kind {
    Kind::SetProperty(pb::SetProperty {
        object_id: uid(1000 + n),
        property: "transform".into(),
        value: Some(pb::PropertyValue {
            value: Some(Value::Transform(pb::Affine {
                a: 1.0,
                d: 1.0,
                e: by,
                ..Default::default()
            })),
        }),
    })
}
fn transaction(
    store: &ProjectStore,
    author: &DeviceId,
    n: u64,
    kinds: Vec<Kind>,
) -> pb::Transaction {
    pb::Transaction {
        txn_id: uid(100_000 + n),
        project_id: uid(1),
        device_id: author.to_string(),
        base_revision: Some(store.revision().unwrap()),
        created_at_wall_ms: 1000 + n as i64,
        ops: kinds
            .into_iter()
            .enumerate()
            .map(|(index, kind)| pb::Op {
                op_id: Some(pb::OpId {
                    device_id: author.to_string(),
                    lamport: n * 100 + index as u64 + 1,
                }),
                kind: Some(kind),
            })
            .collect(),
        gesture_id: n.is_multiple_of(2).then(|| id(200_000 + n).to_proto()),
    }
}
fn assert_known(store: &ProjectStore, what: &str) {
    assert_eq!(
        store.checkpoint_len(),
        store.checkpoint_bytes().unwrap().len(),
        "after {what}"
    );
}
fn rows(path: &Path) -> i64 {
    rusqlite::Connection::open(path.join("project.sqlite"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM op_log", [], |row| row.get(0))
        .unwrap()
}

/// The length of a checkpoint is known exactly after every kind of change to
/// what a checkpoint holds, without writing one.
#[test]
fn the_size_of_a_checkpoint_is_known_after_every_kind_of_change() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("project.vwb");
    let mut store = ProjectStore::create(&path, project(), device(1), 0).unwrap();
    assert_known(&store, "creation");
    let n = std::cell::Cell::new(0);
    let commit = |store: &mut ProjectStore, author: u8, kinds: Vec<Kind>, what: &str| {
        n.set(n.get() + 1);
        let txn = transaction(store, &device(author), n.get(), kinds);
        let accepted = store.commit(&txn, &device(author), 50).unwrap();
        assert_known(store, what);
        (txn, accepted)
    };
    for k in 0..12 {
        // A negative zero now and then: the host holds it as it came, and a
        // checkpoint writes it one byte longer than a positive one.
        let zero = if k % 3 == 0 { -0.0 } else { 0.0 };
        let kinds = vec![stroke_with(k, &device(1), zero)];
        commit(&mut store, 1, kinds, "a stroke");
    }
    let (first_move, _) = commit(&mut store, 1, vec![moved(0, 5.0)], "a move");
    commit(
        &mut store,
        1,
        vec![Kind::UndoTransaction(pb::UndoTransaction {
            target_txn_id: first_move.txn_id.clone(),
        })],
        "its undo",
    );
    // A deletion leaves a tombstone, its undo takes the tombstone away, and
    // deleting again puts one back where others are.
    let (deleted, _) = commit(
        &mut store,
        1,
        vec![Kind::DeleteObject(pb::DeleteObject {
            object_id: uid(1003),
        })],
        "a deletion",
    );
    commit(
        &mut store,
        1,
        vec![Kind::DeleteObject(pb::DeleteObject {
            object_id: uid(1004),
        })],
        "a second deletion",
    );
    commit(
        &mut store,
        1,
        vec![Kind::UndoTransaction(pb::UndoTransaction {
            target_txn_id: deleted.txn_id.clone(),
        })],
        "a deletion undone",
    );
    commit(
        &mut store,
        1,
        vec![
            Kind::UpdateLayer(pb::UpdateLayer {
                layer_id: uid(3),
                name: Some("Ink \"quoted\" \u{03a9}".into()),
                opacity: Some(0.6),
                ..Default::default()
            }),
            Kind::UpdateDocument(pb::UpdateDocument {
                document_id: uid(2),
                title: "After".into(),
            }),
            Kind::AddAsset(pb::AddAsset {
                asset_id: AssetId::hash(b"an original").to_string(),
                format: "png".into(),
                width: 8,
                height: 8,
                orientation: 1,
                bit_depth: 8,
                color_space: "sRGB".into(),
                byte_size: 11,
                source: "import".into(),
                metadata_json: "{}".into(),
                ..Default::default()
            }),
        ],
        "a layer, a title and an asset",
    );
    // The other device: a stroke of its own, then a move of a stroke the
    // host moved meanwhile and an edit of one the host deleted, which leave
    // conflict records, one with the edited object in it.
    commit(
        &mut store,
        2,
        vec![stroke(50, &device(2))],
        "a peer's stroke",
    );
    let seen = store.revision().unwrap();
    commit(&mut store, 1, vec![moved(1, 2.0)], "the host's move");
    commit(
        &mut store,
        1,
        vec![Kind::DeleteObject(pb::DeleteObject {
            object_id: uid(1002),
        })],
        "the host's deletion",
    );
    for (what, kinds) in [
        ("a conflicting move", vec![moved(1, 9.0)]),
        ("an edit of a deleted object", vec![moved(2, 9.0)]),
    ] {
        n.set(n.get() + 1);
        let mut txn = transaction(&store, &device(2), n.get(), kinds);
        txn.base_revision = Some(seen.clone());
        let accepted = store.commit(&txn, &device(2), 60).unwrap();
        assert_eq!(accepted.conflicts.len(), 1, "{what}");
        assert_known(&store, what);
    }
    // A cancelled gesture is in the checkpoint too.
    store.cancel_gesture(device(2), id(900), 70).unwrap();
    assert_known(&store, "a cancellation");
    store.cancel_gesture(device(2), id(901), 71).unwrap();
    store.cancel_gesture(device(2), id(901), 72).unwrap();
    assert_known(&store, "a second cancellation, twice");
    // A transaction that is refused, tried or retried adds nothing.
    let size = store.checkpoint_len();
    let missing = transaction(&store, &device(1), 900, vec![moved(700, 1.0)]);
    assert!(store.commit(&missing, &device(1), 80).is_err());
    assert!(
        store
            .validate_transaction(&missing, &device(1), 80)
            .is_err()
    );
    let fits = transaction(&store, &device(1), 901, vec![moved(5, 1.0)]);
    store.validate_transaction(&fits, &device(1), 80).unwrap();
    assert!(store.commit(&first_move, &device(1), 80).unwrap().duplicate);
    assert_eq!(store.checkpoint_len(), size);
    assert_known(&store, "a refusal, a trial and a retry");

    // Opened again by deciding the log again, and from a checkpoint. (The
    // log holds a negative zero as a zero, so the state read back is a few
    // bytes shorter than the one that was held; each knows its own length.)
    drop(store);
    let mut store = ProjectStore::open(&path).unwrap();
    assert!(store.replayed_transactions() > 0);
    assert_known(&store, "opening by deciding the log again");
    store.checkpoint_now(90).unwrap();
    commit(
        &mut store,
        1,
        vec![stroke(60, &device(1))],
        "a stroke after",
    );
    let state = store.checkpoint_bytes().unwrap();
    drop(store);
    let store = ProjectStore::open(&path).unwrap();
    assert_eq!(store.replayed_transactions(), 1);
    assert_eq!(store.checkpoint_len(), state.len());
    // The other device, which starts from the checkpoint it is given.
    let mirror = ProjectStore::create_authenticated_checkpoint(
        &temp.path().join("mirror.vwb"),
        &state,
        &device(1),
        &device(2),
        95,
    )
    .unwrap();
    assert_eq!(mirror.checkpoint_len(), state.len());
    assert_known(&mirror, "a start from a checkpoint");
}

/// The save that would leave a checkpoint larger than the limit is refused
/// with the error a checkpoint of that size has, and changes nothing.
#[test]
fn the_save_that_would_cross_the_limit_is_refused_and_nothing_changes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("project.vwb");
    let mut store = ProjectStore::create(&path, project(), device(1), 0).unwrap();
    // Room for a handful of strokes, not for twenty.
    let limit = store.checkpoint_len() + 60_000;
    store.limit_history(limit);
    let too_large = |result: Result<vw_ops::Acceptance, StoreError>| {
        matches!(
            result,
            Err(StoreError::Operation(OpsError::Invalid("checkpoint size")))
        )
    };
    let mut accepted = Vec::new();
    let mut refused = None;
    for n in 1..=20 {
        let txn = transaction(&store, &device(1), n, vec![stroke(n, &device(1))]);
        let before = store.checkpoint_bytes().unwrap();
        // What the save would leave, decided apart from the store.
        let mut twin = HostSequencer::from_checkpoint_bytes(&before).unwrap();
        twin.submit(txn.clone(), &device(1), 50).unwrap();
        let would_be = twin.checkpoint_bytes().unwrap().len();
        let tried = store.validate_transaction(&txn, &device(1), 50);
        let saved = store.commit(&txn, &device(1), 50);
        if would_be <= limit {
            assert_eq!(tried.unwrap().ack, saved.unwrap().ack, "stroke {n}");
            assert_eq!(store.checkpoint_len(), would_be, "stroke {n}");
            accepted.push(txn);
        } else {
            // The first save that would cross the limit, and exactly that one.
            assert!(too_large(tried), "stroke {n}: trial");
            assert!(too_large(saved), "stroke {n}: save");
            assert!(store.checkpoint_bytes().unwrap() == before, "stroke {n}");
            assert_eq!(store.checkpoint_len(), before.len(), "stroke {n}");
            refused = Some(txn);
            break;
        }
    }
    let refused = refused.expect("twenty strokes fit under the limit");
    assert!(accepted.len() >= 3, "{}", accepted.len());
    assert!(store.checkpoint_len() <= limit);
    assert_eq!(rows(&path), accepted.len() as i64);
    // Refused again however often it is tried, and by its trial as well.
    assert!(too_large(store.commit(&refused, &device(1), 51)));
    assert!(too_large(store.validate_transaction(
        &refused,
        &device(1),
        51
    )));
    // A retry of a save that was accepted is still answered: it adds nothing.
    let retried = store.commit(&accepted[0], &device(1), 52).unwrap();
    assert!(retried.duplicate);
    assert!(
        store
            .validate_transaction(&accepted[0], &device(1), 52)
            .unwrap()
            .duplicate
    );
    // An edit small enough still fits.
    let small = transaction(&store, &device(1), 500, vec![moved(1, 3.0)]);
    let mut twin =
        HostSequencer::from_checkpoint_bytes(&store.checkpoint_bytes().unwrap()).unwrap();
    twin.submit(small.clone(), &device(1), 53).unwrap();
    assert!(twin.checkpoint_bytes().unwrap().len() <= limit);
    store.commit(&small, &device(1), 53).unwrap();
    assert_known(&store, "a small edit under the limit");

    // The stored project is what was accepted, and opens. A history that is
    // already there is never refused: with a limit below it the project
    // still opens and is read, and only further saves are refused.
    let state = store.checkpoint_bytes().unwrap();
    drop(store);
    let mut store = ProjectStore::open(&path).unwrap();
    assert!(store.checkpoint_bytes().unwrap() == state);
    assert_eq!(rows(&path), accepted.len() as i64 + 1);
    store.limit_history(state.len() / 2);
    let next = transaction(&store, &device(1), 600, vec![moved(1, 4.0)]);
    assert!(too_large(store.commit(&next, &device(1), 54)));
    assert!(store.checkpoint_bytes().unwrap() == state);
    drop(store);
    let mut store = ProjectStore::open(&path).unwrap();
    assert!(store.checkpoint_bytes().unwrap() == state);
    // Without a lower limit the one a checkpoint has applies: this fits.
    store.commit(&next, &device(1), 55).unwrap();
    assert_known(&store, "a save under the limit of a checkpoint");
}
