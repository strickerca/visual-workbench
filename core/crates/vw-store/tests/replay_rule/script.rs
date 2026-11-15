//! The transactions of the pinned logs: two devices and every way the engine
//! can decide what they send.
//!
//! This file uses nothing a build of the store from October 2026 lacks. It is
//! copied unchanged into a checkout of an older build to produce the stored
//! logs in `tests/fixtures`; see the README there.
use std::path::Path;
use vw_model::{DeviceId, Document, Id, Layer, Project};
use vw_proto::v1::{self as pb, object_state::Shape, op::Kind, property_value::Value};
use vw_store::ProjectStore;

pub const CLOCK: u64 = 1_700_000_000_000;
pub fn id(n: u64) -> Id {
    Id::from_parts(CLOCK + n, [0x52; 10]).unwrap()
}
fn uid(n: u64) -> Option<pb::Uuid> {
    Some(id(n).to_proto())
}
/// Device 1 is the host; device 2 is the other device.
pub fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
pub fn project() -> Project {
    let mut value = Project::new(id(1), "Replay rule fixture".into(), device(1));
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

fn object(n: u64, layer: u64, by: u8) -> Kind {
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
            created_by: device(by).to_string(),
            created_at_ms: CLOCK as i64,
            shape: Some(Shape::Rect(pb::RectD {
                x: 0.25,
                y: 1.5,
                w: 12.0,
                h: 9.0,
            })),
            ..Default::default()
        }),
    })
}
fn property(n: u64, name: &str, value: Value) -> Kind {
    Kind::SetProperty(pb::SetProperty {
        object_id: uid(n),
        property: name.into(),
        value: Some(pb::PropertyValue { value: Some(value) }),
    })
}
fn width(n: u64, value: f64) -> Kind {
    property(n, "style.width", Value::Number(value))
}
fn colour(n: u64, rgba: u32) -> Kind {
    property(n, "style.stroke", Value::Color(pb::Color { rgba }))
}
fn delete(n: u64) -> Kind {
    Kind::DeleteObject(pb::DeleteObject { object_id: uid(n) })
}
fn undo(step: u64) -> Kind {
    Kind::UndoTransaction(pb::UndoTransaction {
        target_txn_id: uid(10_000 + step),
    })
}

/// One transaction. Its number is its place in the script, which is also the
/// revision it produces: every one of them is accepted.
pub struct Step {
    pub device: u8,
    /// The revision it was made at, when that is not the newest: a device
    /// that had not yet seen what the other one did.
    pub base: Option<u64>,
    /// The sender's clock, which decides between two edits of one property.
    pub wall: i64,
    pub gesture: Option<u64>,
    pub kinds: Vec<Kind>,
}
fn step(device: u8, base: Option<u64>, wall: i64, kinds: Vec<Kind>) -> Step {
    Step {
        device,
        base,
        wall,
        gesture: None,
        kinds,
    }
}

pub fn steps() -> Vec<Step> {
    let mut steps = vec![
        // 1: the host draws.
        step(
            1,
            None,
            10,
            vec![
                object(10, 3, 1),
                object(11, 3, 1),
                object(12, 3, 1),
                object(13, 3, 1),
            ],
        ),
        // 2..=4: both change one property without having seen the other.
        // The later clock wins: first the host's, then the other device's.
        step(2, None, 100, vec![width(10, 3.0)]),
        step(1, Some(1), 200, vec![width(10, 7.0)]),
        step(2, Some(2), 250, vec![width(10, 9.0)]),
        // 5..=7: equal clocks. The host wins, in either order of arrival.
        step(2, None, 300, vec![width(11, 2.0)]),
        step(1, Some(4), 300, vec![width(11, 4.0)]),
        step(2, Some(5), 300, vec![width(11, 6.0)]),
        // 8..=11: the host deletes an object; the other device, which has
        // not seen that, edits it, moves it and deletes it.
        step(1, None, 400, vec![delete(12)]),
        step(2, Some(7), 410, vec![width(12, 8.0)]),
        step(
            2,
            Some(7),
            420,
            vec![Kind::Reorder(pb::Reorder {
                object_id: uid(12),
                layer_id: uid(3),
                order_key: "k".into(),
            })],
        ),
        step(2, Some(7), 430, vec![delete(12), width(10, 1.5)]),
        // 12..=13: the other device edits an object; the host, which has not
        // seen that, deletes it. The deletion wins and the edit is kept for
        // review.
        step(2, None, 500, vec![width(13, 5.0), colour(13, 0x445566ff)]),
        step(1, Some(11), 490, vec![delete(13)]),
        // 14..=16: the host changes two properties in one edit, the other
        // device changes one of them again, and the host undoes its edit:
        // only the property nobody else touched goes back.
        step(1, None, 600, vec![width(10, 20.0), colour(10, 0x778899ff)]),
        step(2, None, 610, vec![width(10, 30.0)]),
        step(1, None, 620, vec![undo(14)]),
        // 17..=18: the host undoes that undo, and the deletion of object 13.
        step(1, None, 630, vec![undo(16)]),
        step(1, None, 640, vec![undo(13)]),
        // 19..=22: the other device's own work, a layer, objects and a group,
        // then undone and redone.
        step(
            2,
            None,
            700,
            vec![
                Kind::CreateLayer(pb::CreateLayer {
                    layer_id: uid(4),
                    document_id: uid(2),
                    page_index: -1,
                    name: "From the other device".into(),
                    kind: "annotation".into(),
                    order_key: "k".into(),
                }),
                object(20, 4, 2),
                object(21, 4, 2),
            ],
        ),
        step(
            2,
            None,
            710,
            vec![Kind::Group(pb::Group {
                group_id: uid(30),
                object_ids: vec![id(20).to_proto(), id(21).to_proto()],
            })],
        ),
        step(2, None, 720, vec![undo(20)]),
        step(2, None, 730, vec![undo(21)]),
        // 23..=25: the host renames the layer and the document while the
        // other device, on an older revision, renames the layer too.
        step(
            1,
            None,
            800,
            vec![Kind::UpdateLayer(pb::UpdateLayer {
                layer_id: uid(4),
                name: Some("Renamed by the host".into()),
                ..Default::default()
            })],
        ),
        step(
            2,
            Some(22),
            790,
            vec![Kind::UpdateLayer(pb::UpdateLayer {
                layer_id: uid(4),
                name: Some("Renamed by the other device".into()),
                opacity: Some(0.5),
                ..Default::default()
            })],
        ),
        step(
            1,
            None,
            810,
            vec![Kind::UpdateDocument(pb::UpdateDocument {
                document_id: uid(2),
                title: "After".into(),
            })],
        ),
        // 26..=28: a stroke of the other device as one gesture, an edit of it
        // by the host, and the other device's undo of its creation.
        step(2, None, 900, vec![object(22, 3, 2)]),
        step(1, None, 910, vec![width(22, 2.5)]),
        step(2, None, 920, vec![undo(26)]),
        // 29..=30: the other device, on an old revision but with the later
        // clock, sets a property to the value it already has. Nothing
        // changes and nothing is in conflict, yet the property is now the
        // other device's, and the host's undo of its own edit leaves it.
        step(2, Some(5), 950, vec![width(11, 4.0)]),
        step(1, None, 960, vec![undo(6)]),
    ];
    steps[25].gesture = Some(40);
    steps
}

/// The revision at which the other device joins, and those at which it
/// catches up. It ends level with the host.
pub const JOINS: u64 = 6;
pub const CATCHES_UP: [u64; 3] = [13, 21, 30];
/// Revisions after which the host is asked to store a checkpoint.
pub const CHECKPOINTS: [u64; 2] = [9, 17];
/// The host cancels this gesture of the other device after revision 19.
pub const CANCELLED: u64 = 41;
pub const CANCELLED_AFTER: u64 = 19;

fn transaction(n: u64, step: &Step, base: pb::Revision) -> pb::Transaction {
    pb::Transaction {
        txn_id: uid(10_000 + n),
        project_id: uid(1),
        device_id: device(step.device).to_string(),
        base_revision: Some(base),
        created_at_wall_ms: step.wall,
        ops: step
            .kinds
            .iter()
            .enumerate()
            .map(|(index, kind)| pb::Op {
                op_id: Some(pb::OpId {
                    device_id: device(step.device).to_string(),
                    lamport: n * 100 + index as u64 + 1,
                }),
                kind: Some(kind.clone()),
            })
            .collect(),
        gesture_id: step.gesture.and_then(uid),
    }
}

/// Run the script: the host's project at `root/host.vwb`, the other device's
/// at `root/mirror.vwb`. `checkpoint` stores a checkpoint of the host now,
/// by whatever name the build under test has for that.
pub fn run(root: &Path, checkpoint: fn(&mut ProjectStore, i64)) -> (ProjectStore, ProjectStore) {
    let mut host = ProjectStore::create(&root.join("host.vwb"), project(), device(1), 0).unwrap();
    let mut mirror = None;
    let mut revisions = vec![host.revision().unwrap()];
    for (index, step) in steps().iter().enumerate() {
        let n = index as u64 + 1;
        let base = revisions[step.base.unwrap_or(n - 1) as usize].clone();
        let accepted = host
            .commit(
                &transaction(n, step, base),
                &device(step.device),
                1_000 + n as i64,
            )
            .unwrap_or_else(|error| panic!("step {n} was refused: {error:?}"));
        assert_eq!(accepted.ack.host_seq, n);
        revisions.push(host.revision().unwrap());
        if n == CANCELLED_AFTER {
            host.cancel_gesture(device(2), id(CANCELLED), 1_500 + n as i64)
                .unwrap();
        }
        if CHECKPOINTS.contains(&n) {
            checkpoint(&mut host, 2_000 + n as i64);
        }
        if n == JOINS {
            mirror = Some(
                ProjectStore::create_authenticated_checkpoint(
                    &root.join("mirror.vwb"),
                    &host.checkpoint_bytes().unwrap(),
                    &device(1),
                    &device(2),
                    3_000 + n as i64,
                )
                .unwrap(),
            );
        }
        if CATCHES_UP.contains(&n) {
            mirror
                .as_mut()
                .unwrap()
                .install_authenticated_checkpoint(
                    &host.checkpoint_bytes().unwrap(),
                    &device(1),
                    3_000 + n as i64,
                )
                .unwrap();
        }
    }
    (host, mirror.unwrap())
}
