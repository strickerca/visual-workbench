//! Every kind of edit on a document large enough for the engine's shortcuts
//! to matter gives exactly what deciding it from nothing gives.
//!
//! The store decides an edit on the project it holds: it checks only what
//! the edit touched, keeps the canonical bytes of everything else from the
//! last hash and writes only the touched rows. For each edit here the same
//! transaction is also decided by a host read back from the checkpoint taken
//! before it, which has nothing kept from earlier edits, and the two have to
//! agree byte for byte: the acknowledgement, the state hash, and the whole
//! sequencing state with its journal. The state hash is also compared with
//! the hash of the whole project, and the project is opened again, which
//! compares every stored row with a full rebuild. This build of the tests
//! additionally stops inside the engine if a partial check or hash differs
//! from the whole-project one (`cross-check` in vw-ops).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::Path;
use vw_model::{AssetId, DeviceId, Document, Id, Layer, Project};
use vw_ops::HostSequencer;
use vw_proto::{
    Message,
    v1::{self as pb, object_state::Shape, op::Kind, property_value::Value},
};
use vw_store::ProjectStore;

const CLOCK: u64 = 1_700_000_000_000;
/// Strokes in the document: 0.8 MB of canonical bytes, several pages of the
/// bytes the hasher keeps.
const STROKES: u64 = 400;
const SAMPLES: usize = 48;

fn id(n: u64) -> Id {
    Id::from_parts(CLOCK + n, [0x51; 10]).unwrap()
}
fn uid(n: u64) -> Option<pb::Uuid> {
    Some(id(n).to_proto())
}
fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
/// Objects are numbered from here, so stroke `n` is object `OBJECT + n`.
const OBJECT: u64 = 10_000;
const MASK: u64 = 50_000;

fn asset(seed: &str) -> pb::AddAsset {
    pb::AddAsset {
        asset_id: AssetId::hash(seed.as_bytes()).to_string(),
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
    let mut value = Project::new(id(1), "Large edits fixture".into(), device(1));
    let original = asset("original");
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
    for (n, kind, order) in [
        (3, "annotation", "V"),
        (4, "mask", "k"),
        (5, "annotation", "q"),
    ] {
        value.layers.insert(
            id(n),
            Layer {
                definition: pb::CreateLayer {
                    layer_id: uid(n),
                    document_id: uid(2),
                    page_index: -1,
                    name: format!("Layer {n}"),
                    kind: kind.into(),
                    order_key: order.into(),
                },
                visible: true,
                locked: false,
                opacity: 1.0,
                blend: "normal".into(),
            },
        );
    }
    value
}
fn state(n: u64, layer: u64, author: &DeviceId, shape: Shape) -> pb::ObjectState {
    pb::ObjectState {
        object_id: uid(n),
        layer_id: uid(layer),
        order_key: "V".into(),
        transform: Some(pb::Affine {
            a: 1.0,
            d: 1.0,
            e: (n % 97) as f64,
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
        shape: Some(shape),
        ..Default::default()
    }
}
fn stroke(n: u64) -> Shape {
    let at = |i: usize| i as f64;
    Shape::Stroke(pb::Stroke {
        x: (0..SAMPLES)
            .map(|i| (n % 900) as f64 + at(i) * 1.25)
            .collect(),
        y: (0..SAMPLES)
            .map(|i| (n % 700) as f64 + at(i) * 0.5)
            .collect(),
        t_ms: (0..SAMPLES).map(|i| (i * 4) as u32).collect(),
        pressure: (0..SAMPLES)
            .map(|i| 0.25 + (i % 5) as f32 * 0.125)
            .collect(),
        tilt: vec![],
        orientation: vec![],
        brush: Some(pb::Brush {
            family: "pen".into(),
            algorithm_version: 1,
            base_width: 4.0,
            pressure_curve: vec![0.0, 0.0, 0.25, 0.25, 0.75, 0.75, 1.0, 1.0],
            stabilization: 0.0,
        }),
    })
}
fn create(n: u64, layer: u64, author: &DeviceId, shape: Shape) -> Kind {
    Kind::CreateObject(pb::CreateObject {
        document_id: uid(2),
        state: Some(state(n, layer, author, shape)),
    })
}
fn property(n: u64, name: &str, value: Value) -> Kind {
    Kind::SetProperty(pb::SetProperty {
        object_id: uid(n),
        property: name.into(),
        value: Some(pb::PropertyValue { value: Some(value) }),
    })
}
fn moved(n: u64, by: f64) -> Kind {
    property(
        n,
        "transform",
        Value::Transform(pb::Affine {
            a: 1.0,
            d: 1.0,
            e: by,
            // Never a negative zero: protobuf stores it as zero, so a
            // transaction read back from the log would differ in its sign.
            f: 0.25 - by * 0.5,
            ..Default::default()
        }),
    )
}
fn layer_update(layer: u64, change: impl FnOnce(&mut pb::UpdateLayer)) -> Kind {
    let mut update = pb::UpdateLayer {
        layer_id: uid(layer),
        ..Default::default()
    };
    change(&mut update);
    Kind::UpdateLayer(update)
}
fn instruction(n: u64, text: &str) -> Kind {
    Kind::SetInstruction(pb::SetInstruction {
        instruction_id: uid(n),
        document_id: uid(2),
        target_object_ids: vec![id(OBJECT + 1).to_proto()],
        role: pb::Role::Change as i32,
        text: text.into(),
        entry_method: "pc_keyboard".into(),
        language: "en".into(),
    })
}
fn mask_shape(seed: &str) -> Shape {
    Shape::SelectionRaster(pb::RasterMaskRef {
        mask_asset_id: asset(seed).asset_id,
        bounds: Some(pb::RectD {
            x: 0.0,
            y: 0.0,
            w: 128.0,
            h: 64.0,
        }),
        feather: 0.0,
    })
}
fn result(n: u64) -> Kind {
    Kind::AddResult(pb::AddResult {
        result_id: uid(n),
        document_id: uid(2),
        provider: "synthetic".into(),
        model: "synthetic".into(),
        request_json: "{}".into(),
        ..Default::default()
    })
}

/// A transaction by `author` on the store's current revision.
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
                    lamport: n * 4096 + index as u64 + 1,
                }),
                kind: Some(kind),
            })
            .collect(),
        gesture_id: None,
    }
}

/// The document every edit below is made on.
fn fixture(path: &Path) -> ProjectStore {
    let mut store = ProjectStore::create(path, project(), device(1), 0).unwrap();
    for batch in 0..STROKES / 100 {
        let kinds = (batch * 100..(batch + 1) * 100)
            .map(|n| create(OBJECT + n, 3, &device(1), stroke(n)))
            .collect();
        let txn = transaction(&store, &device(1), batch, kinds);
        store.commit(&txn, &device(1), 10).unwrap();
    }
    let txn = transaction(
        &store,
        &device(1),
        20,
        vec![
            Kind::AddAsset(asset("mask 0")),
            create(MASK, 4, &device(1), mask_shape("mask 0")),
            instruction(60_000, "Make this part darker"),
            result(70_000),
        ],
    );
    store.commit(&txn, &device(1), 10).unwrap();
    store
}

/// The edits, in the order they are made. Each is one transaction; an `Undo`
/// takes back the one before it.
enum Step {
    Edit(&'static str, Vec<Kind>),
    Undo,
    /// By the other device, on the revision it last saw: `back` revisions ago.
    Stale(&'static str, u64, Vec<Kind>),
}
fn steps() -> Vec<Step> {
    use Step::{Edit, Stale, Undo};
    let polygon = Shape::Polygon(pb::Polyline {
        points: (0..64)
            .map(|i| pb::PointD {
                x: 20.0 + f64::from(i) * 1.25,
                y: 30.0 + f64::from(i % 7) * 2.5,
            })
            .collect(),
        closed: true,
    });
    vec![
        Edit(
            "a new stroke",
            vec![create(OBJECT + STROKES, 3, &device(1), stroke(9))],
        ),
        Undo,
        Edit("the first stroke moved", vec![moved(OBJECT, 12.5)]),
        Undo,
        Edit(
            "the first stroke restyled",
            vec![property(
                OBJECT,
                "style",
                Value::Style(pb::Style {
                    stroke: Some(pb::Color { rgba: 0x445566ff }),
                    width: 6.5,
                    ..Default::default()
                }),
            )],
        ),
        Undo,
        Edit(
            "an early stroke deleted",
            vec![Kind::DeleteObject(pb::DeleteObject {
                object_id: uid(OBJECT + 3),
            })],
        ),
        Undo,
        Edit(
            "part of an early stroke erased",
            vec![
                Kind::DeleteObject(pb::DeleteObject {
                    object_id: uid(OBJECT + 5),
                }),
                create(OBJECT + STROKES + 1, 3, &device(1), polygon),
            ],
        ),
        Undo,
        Edit(
            "a stroke in the middle moved",
            vec![moved(OBJECT + STROKES / 2, -3.0)],
        ),
        Edit(
            "the last stroke moved",
            vec![moved(OBJECT + STROKES - 1, 7.0)],
        ),
        Undo,
        Edit(
            "the layer renamed",
            vec![layer_update(3, |u| u.name = Some("Ink".into()))],
        ),
        Undo,
        Edit(
            "the layer reordered",
            vec![layer_update(3, |u| u.order_key = Some("X".into()))],
        ),
        Undo,
        Edit(
            "the layer hidden",
            vec![layer_update(3, |u| u.visible = Some(false))],
        ),
        Undo,
        Edit(
            "the layer's opacity",
            vec![layer_update(3, |u| u.opacity = Some(0.5))],
        ),
        Undo,
        Edit(
            "the document title",
            vec![Kind::UpdateDocument(pb::UpdateDocument {
                document_id: uid(2),
                title: "After".into(),
            })],
        ),
        Undo,
        Edit(
            "an instruction added",
            vec![instruction(60_001, "Remove this")],
        ),
        Undo,
        Edit(
            "an instruction edited",
            vec![instruction(60_000, "Make this part much darker")],
        ),
        Undo,
        Edit(
            "a selection mask edited",
            vec![
                Kind::AddAsset(asset("mask 1")),
                property(
                    MASK,
                    "shape",
                    Value::Raw(serde_json::to_vec(&mask_shape("mask 1")).unwrap()),
                ),
                Kind::MaskOp(pb::MaskOp {
                    output_object_id: uid(MASK),
                    op: "add".into(),
                    input_object_ids: vec![],
                    amount: 0.0,
                }),
            ],
        ),
        Undo,
        Edit("a result added", vec![result(70_001)]),
        Undo,
        Edit(
            "a result's status",
            vec![Kind::UpdateResult(pb::UpdateResult {
                result_id: uid(70_000),
                status: "rejected".into(),
                acceptance_mask_asset_id: String::new(),
            })],
        ),
        Undo,
        Edit(
            "fifty early strokes moved at once",
            (0..50)
                .map(|k| moved(OBJECT + 20 + k * 3, k as f64))
                .collect(),
        ),
        Undo,
        Edit(
            "strokes grouped",
            vec![Kind::Group(pb::Group {
                group_id: uid(80_000),
                object_ids: (0..20).map(|k| id(OBJECT + 200 + k).to_proto()).collect(),
            })],
        ),
        Edit(
            "the group taken apart",
            vec![Kind::Ungroup(pb::Ungroup {
                group_id: uid(80_000),
            })],
        ),
        Undo,
        Undo,
        Edit(
            "a stroke sent to another layer",
            vec![Kind::Reorder(pb::Reorder {
                object_id: uid(OBJECT + 40),
                order_key: "k".into(),
                layer_id: uid(5),
            })],
        ),
        Edit(
            "that layer deleted with what is on it",
            vec![Kind::DeleteLayer(pb::DeleteLayer { layer_id: uid(5) })],
        ),
        Undo,
        Edit(
            "a layer created",
            vec![Kind::CreateLayer(pb::CreateLayer {
                layer_id: uid(6),
                document_id: uid(2),
                page_index: -1,
                name: "Later".into(),
                kind: "annotation".into(),
                order_key: "z".into(),
            })],
        ),
        Undo,
        // The other device: an edit of its own, then two made without having
        // seen what the host did in between, which the host has to settle.
        Stale(
            "a stroke by the other device",
            0,
            vec![create(OBJECT + STROKES + 2, 3, &device(2), stroke(11))],
        ),
        Edit("the host moves a stroke", vec![moved(OBJECT + 60, 1.0)]),
        Edit(
            "the host hides the layer",
            vec![layer_update(3, |u| u.visible = Some(false))],
        ),
        Stale(
            "the other device moves the same stroke",
            2,
            vec![moved(OBJECT + 60, 2.0)],
        ),
        Stale(
            "the other device shows the layer",
            3,
            vec![layer_update(3, |u| u.visible = Some(true))],
        ),
        Edit(
            "the host deletes a stroke the other device then edits",
            vec![Kind::DeleteObject(pb::DeleteObject {
                object_id: uid(OBJECT + 61),
            })],
        ),
        Stale(
            "the other device edits the deleted stroke",
            1,
            vec![moved(OBJECT + 61, 4.0)],
        ),
    ]
}

/// Where two checkpoints first differ, for the failure message.
fn difference(left: &[u8], right: &[u8]) -> String {
    fn walk(path: &str, left: &serde_json::Value, right: &serde_json::Value) -> Option<String> {
        use serde_json::Value::{Array, Object};
        match (left, right) {
            (Object(a), Object(b)) => {
                for key in a.keys().chain(b.keys()) {
                    match (a.get(key), b.get(key)) {
                        (Some(x), Some(y)) => {
                            if let Some(found) = walk(&format!("{path}/{key}"), x, y) {
                                return Some(found);
                            }
                        }
                        (x, y) => return Some(format!("{path}/{key}: {x:?} | {y:?}")),
                    }
                }
                None
            }
            (Array(a), Array(b)) if a.len() == b.len() => a
                .iter()
                .zip(b)
                .enumerate()
                .find_map(|(index, (x, y))| walk(&format!("{path}/{index}"), x, y)),
            (x, y) if x == y => None,
            (x, y) => Some(format!("{path}: {x} | {y}")),
        }
    }
    let payload = |bytes: &[u8]| -> serde_json::Value {
        let start = bytes.iter().position(|byte| *byte == b'\n').unwrap() + 33;
        serde_json::from_slice(&bytes[start..]).unwrap()
    };
    walk("", &payload(left), &payload(right)).unwrap_or_else(|| {
        // Equal as JSON values: show the bytes around the first difference.
        let at = left
            .iter()
            .zip(right)
            .enumerate()
            .position(|(index, (a, b))| index > 66 && a != b)
            .unwrap_or(left.len().min(right.len()));
        let show = |bytes: &[u8]| {
            let from = at.saturating_sub(160).max(80);
            String::from_utf8_lossy(&bytes[from..(at + 60).min(bytes.len())]).into_owned()
        };
        format!("bytes at {at}: {} | {}", show(left), show(right))
    })
}

#[test]
fn every_kind_of_edit_on_a_large_document_is_decided_as_from_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("project.vwb");
    let mut store = fixture(&path);
    assert!(store.project().canonical_bytes().unwrap().len() > 700_000);

    // Refused edits in between must leave nothing behind in what is kept.
    let refused = |store: &mut ProjectStore, n: u64| {
        let before = store.checkpoint_bytes().unwrap();
        for (index, kinds) in [
            vec![moved(OBJECT + STROKES + 500, 1.0)],
            vec![layer_update(3, |u| u.opacity = Some(1.5))],
            vec![
                moved(OBJECT + 7, 9.0),
                layer_update(9, |u| u.visible = Some(false)),
            ],
            vec![
                Kind::UpdateDocument(pb::UpdateDocument {
                    document_id: uid(2),
                    title: "Never".into(),
                }),
                Kind::DeleteObject(pb::DeleteObject {
                    object_id: uid(OBJECT + STROKES + 500),
                }),
            ],
        ]
        .into_iter()
        .enumerate()
        {
            let txn = transaction(store, &device(1), 5000 + n * 10 + index as u64, kinds);
            assert!(store.validate_transaction(&txn, &device(1), 20).is_err());
            assert!(store.commit(&txn, &device(1), 20).is_err());
        }
        assert!(
            store.checkpoint_bytes().unwrap() == before,
            "refusals at {n}"
        );
        assert_eq!(store.checkpoint_len(), before.len(), "refusals at {n}");
    };

    let mut history = Vec::<(pb::Revision, Id)>::new();
    let mut undone = 0;
    let mut conflicts = 0;
    for (n, step) in steps().into_iter().enumerate() {
        let n = n as u64 + 100;
        let (name, author, txn) = match step {
            Step::Edit(name, kinds) => {
                undone = 0;
                (name, device(1), transaction(&store, &device(1), n, kinds))
            }
            Step::Undo => {
                // The newest edit by the host that is not undone yet.
                undone += 1;
                let (_, target) = &history[history.len() - (2 * undone - 1)];
                let kinds = vec![Kind::UndoTransaction(pb::UndoTransaction {
                    target_txn_id: Some(target.to_proto()),
                })];
                ("undo", device(1), transaction(&store, &device(1), n, kinds))
            }
            Step::Stale(name, back, kinds) => {
                undone = 0;
                let mut txn = transaction(&store, &device(2), n, kinds);
                if back > 0 {
                    txn.base_revision = Some(history[history.len() - back as usize].0.clone());
                }
                (name, device(2), txn)
            }
        };
        if n.is_multiple_of(3) {
            refused(&mut store, n);
        }
        let before = store.checkpoint_bytes().unwrap();
        let base = store.revision().unwrap();

        // Trying it first changes nothing and answers what the commit will.
        let tried = store.validate_transaction(&txn, &author, 30).unwrap();
        assert!(store.checkpoint_bytes().unwrap() == before, "{name}: trial");

        let accepted = store.commit(&txn, &author, 30).unwrap();
        assert_eq!(tried.ack, accepted.ack, "{name}");
        assert_eq!(tried.conflicts, accepted.conflicts, "{name}");
        conflicts += accepted.conflicts.len();

        // From nothing: a host read back from the checkpoint before the edit.
        let mut twin = HostSequencer::from_checkpoint_bytes(&before).unwrap();
        let decided = twin.submit(txn.clone(), &author, 30).unwrap();
        assert_eq!(
            decided.ack.encode_to_vec(),
            accepted.ack.encode_to_vec(),
            "{name}: acknowledgement"
        );
        assert_eq!(decided.conflicts, accepted.conflicts, "{name}: conflicts");
        assert!(
            twin.checkpoint_bytes().unwrap() == store.checkpoint_bytes().unwrap(),
            "{name}: sequencing state"
        );
        // The size of a checkpoint is known without writing one.
        assert_eq!(
            store.checkpoint_len(),
            store.checkpoint_bytes().unwrap().len(),
            "{name}: checkpoint length"
        );
        // The hash carried forward is the hash of the whole project.
        let whole = store.project().state_hash().unwrap();
        assert_eq!(store.state_hash().unwrap(), &whole, "{name}");
        assert_eq!(accepted.ack.state_hash, whole.bytes(), "{name}");
        store.project().validate().unwrap();

        history.push((base, Id::from_proto(txn.txn_id.as_ref()).unwrap()));

        // Opening compares every stored row with a full rebuild and decides
        // the log again.
        if n.is_multiple_of(4) {
            let state = store.checkpoint_bytes().unwrap();
            drop(store);
            store = ProjectStore::open(&path).unwrap();
            let reopened = store.checkpoint_bytes().unwrap();
            assert!(
                reopened == state,
                "{name}: reopened: {}",
                difference(&state, &reopened)
            );
        }
    }
    let state = store.checkpoint_bytes().unwrap();
    drop(store);
    let store = ProjectStore::open(&path).unwrap();
    assert!(store.checkpoint_bytes().unwrap() == state);
    store.integrity_check().unwrap();
    // The stale edits were settled, not just accepted.
    assert!(conflicts >= 3, "{conflicts}");
    assert_eq!(store.conflicts().unwrap().len(), conflicts);
}
