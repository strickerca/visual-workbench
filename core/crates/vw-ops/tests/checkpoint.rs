//! Checkpoints retain private ordering/undo guards as well as visible state.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::Value as Json;
use vw_model::{AssetId, DeviceId, Document, Id, Layer, Object, Project};
use vw_ops::HostSequencer;
use vw_proto::{
    Message,
    v1::{self as pb, object_state::Shape, op::Kind, property_value::Value},
};

const PREFIX: &[u8] = b"VisualWorkbench.HostCheckpoint.v1\n";
const CLOCK: i64 = 1_700_000_000_000;

fn id(n: u64) -> Id {
    Id::from_parts(CLOCK as u64 + n, [0x43; 10]).unwrap()
}
fn uid(n: u64) -> Option<pb::Uuid> {
    Some(id(n).to_proto())
}
fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
fn fixture() -> HostSequencer {
    let mut project = Project::new(id(1), "Checkpoint fixture".into(), device(1));
    project.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: uid(2),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Synthetic image".into(),
                ..Default::default()
            },
            pages: Vec::new(),
            created_at_ms: CLOCK,
        },
    );
    project.layers.insert(
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
    project.objects.insert(
        id(4),
        Object {
            document_id: id(2),
            state: pb::ObjectState {
                object_id: uid(4),
                layer_id: uid(3),
                order_key: "V".into(),
                created_by: device(1).to_string(),
                created_at_ms: CLOCK,
                role: pb::Role::None as i32,
                transform: Some(pb::Affine {
                    a: 1.0,
                    d: 1.0,
                    ..Default::default()
                }),
                style: Some(pb::Style {
                    stroke: Some(pb::Color { rgba: 0x123456ff }),
                    width: 1.0,
                    ..Default::default()
                }),
                shape: Some(Shape::Rect(pb::RectD {
                    x: 0.10000000000000002,
                    y: -0.0,
                    w: 12.0,
                    h: 9.0,
                })),
                ..Default::default()
            },
        },
    );
    HostSequencer::new(project, device(1)).unwrap()
}
fn width(value: f64) -> Kind {
    Kind::SetProperty(pb::SetProperty {
        object_id: uid(4),
        property: "style.width".into(),
        value: Some(pb::PropertyValue {
            value: Some(Value::Number(value)),
        }),
    })
}
fn transaction(
    host: &HostSequencer,
    who: u8,
    n: u64,
    wall: i64,
    kinds: Vec<Kind>,
) -> pb::Transaction {
    pb::Transaction {
        txn_id: uid(100 + n),
        project_id: uid(1),
        device_id: device(who).to_string(),
        base_revision: Some(host.revision().unwrap()),
        created_at_wall_ms: wall,
        gesture_id: None,
        ops: kinds
            .into_iter()
            .enumerate()
            .map(|(index, kind)| pb::Op {
                op_id: Some(pb::OpId {
                    device_id: device(who).to_string(),
                    lamport: n * 100 + index as u64,
                }),
                kind: Some(kind),
            })
            .collect(),
    }
}
fn current_width(host: &HostSequencer) -> f64 {
    host.project().objects[&id(4)].state.style.unwrap().width
}
fn restore(host: &HostSequencer) -> HostSequencer {
    HostSequencer::from_checkpoint_bytes(&host.checkpoint_bytes().unwrap()).unwrap()
}
fn payload(bytes: &[u8]) -> Json {
    serde_json::from_slice(&bytes[PREFIX.len() + 32..]).unwrap()
}
fn envelope(value: &Json) -> Vec<u8> {
    let payload = serde_json::to_vec(value).unwrap();
    let mut bytes = PREFIX.to_vec();
    bytes.extend_from_slice(&AssetId::hash(&payload).bytes());
    bytes.extend_from_slice(&payload);
    bytes
}

#[test]
fn checkpoint_roundtrip_is_exact_for_revision_zero_and_float_payloads() {
    let host = fixture();
    let restored = restore(&host);
    assert_eq!(
        host.checkpoint_bytes().unwrap(),
        restored.checkpoint_bytes().unwrap()
    );
    assert_eq!(host.project(), restored.project());
    assert_eq!(host.host_device(), restored.host_device());
    assert_eq!(host.revision().unwrap(), restored.revision().unwrap());
    assert!(restored.accepted_transaction(&id(101)).is_none());
    assert!(restored.accepted_at(&id(101)).is_none());
}

#[test]
fn checkpoint_retains_retries_conflicts_tombstones_undo_and_gesture_guards() {
    let mut host = fixture();
    let delayed = transaction(&host, 2, 2, 200, vec![width(3.0)]);
    let mut first = transaction(&host, 1, 1, 100, vec![width(2.0)]);
    first.gesture_id = uid(700);
    let first_ack = host.submit(first.clone(), &device(1), CLOCK + 1).unwrap();
    let delayed_ack = host.submit(delayed.clone(), &device(2), CLOCK + 2).unwrap();
    assert_eq!(delayed_ack.conflicts.len(), 1);
    let ghost = transaction(&host, 2, 4, 400, vec![width(9.0)]);
    let deleting = transaction(
        &host,
        1,
        3,
        300,
        vec![Kind::DeleteObject(pb::DeleteObject { object_id: uid(4) })],
    );
    host.submit(deleting.clone(), &device(1), CLOCK + 3)
        .unwrap();
    let ghost_ack = host.submit(ghost.clone(), &device(2), CLOCK + 4).unwrap();
    assert!(ghost_ack.conflicts[0].delete_vs_edit);
    host.cancel_gesture(device(1), id(800)).unwrap();
    let mut recovered = restore(&host);
    assert_eq!(host.conflicts(), recovered.conflicts());
    assert_eq!(
        host.checkpoint_bytes().unwrap(),
        recovered.checkpoint_bytes().unwrap()
    );
    assert!(!recovered.project().objects.contains_key(&id(4)));
    let retry = recovered
        .submit(delayed.clone(), &device(2), CLOCK + 99)
        .unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.ack, delayed_ack.ack);
    assert!(retry.conflicts.is_empty());
    let (stored, ack) = recovered.accepted_transaction(&id(101)).unwrap();
    assert_eq!(stored.encode_to_vec(), first.encode_to_vec());
    assert_eq!(ack, &first_ack.ack);
    assert_eq!(recovered.accepted_at(&id(101)), Some(CLOCK + 1));
    for (n, gesture) in [(5, 700), (6, 800)] {
        let mut txn = transaction(&recovered, 1, n, 500, vec![width(1.0)]);
        txn.gesture_id = uid(gesture);
        assert!(recovered.submit(txn, &device(1), CLOCK + 10).is_err());
    }
    let mut repeated_op = transaction(&recovered, 1, 7, 500, vec![width(1.0)]);
    repeated_op.ops[0].op_id = first.ops[0].op_id.clone();
    assert!(
        recovered
            .submit(repeated_op, &device(1), CLOCK + 10)
            .is_err()
    );
    let inverse = transaction(
        &recovered,
        1,
        8,
        600,
        vec![Kind::UndoTransaction(pb::UndoTransaction {
            target_txn_id: deleting.txn_id,
        })],
    );
    let undone = recovered.submit(inverse, &device(1), CLOCK + 11).unwrap();
    assert!(undone.ack.notices.is_empty());
    assert_eq!(current_width(&recovered), 3.0);
    assert_eq!(
        restore(&recovered).checkpoint_bytes().unwrap(),
        recovered.checkpoint_bytes().unwrap()
    );
}

#[test]
fn restored_write_ownership_makes_identical_future_conflict_and_undo_decisions() {
    let mut host = fixture();
    let first = transaction(
        &host,
        1,
        1,
        100,
        vec![
            width(2.0),
            Kind::SetProperty(pb::SetProperty {
                object_id: uid(4),
                property: "hidden".into(),
                value: Some(pb::PropertyValue {
                    value: Some(Value::Flag(true)),
                }),
            }),
        ],
    );
    host.submit(first.clone(), &device(1), CLOCK + 1).unwrap();
    let delayed = transaction(&host, 1, 3, 150, vec![width(8.0)]);
    let peer = transaction(&host, 2, 2, 200, vec![width(4.0)]);
    host.submit(peer, &device(2), CLOCK + 2).unwrap();
    let mut recovered = restore(&host);
    let live = host.submit(delayed.clone(), &device(1), CLOCK + 3).unwrap();
    let after_restart = recovered.submit(delayed, &device(1), CLOCK + 3).unwrap();
    assert_eq!(live.ack, after_restart.ack);
    assert_eq!(live.conflicts, after_restart.conflicts);
    let inverse = transaction(
        &host,
        1,
        4,
        300,
        vec![Kind::UndoTransaction(pb::UndoTransaction {
            target_txn_id: first.txn_id,
        })],
    );
    let a = host.submit(inverse.clone(), &device(1), CLOCK + 4).unwrap();
    let b = recovered.submit(inverse, &device(1), CLOCK + 4).unwrap();
    assert_eq!(a.ack, b.ack);
    assert_eq!(a.ack.notices.len(), 1);
    assert_eq!(current_width(&recovered), 4.0);
    assert!(!recovered.project().objects[&id(4)].state.hidden);
    assert_eq!(
        host.checkpoint_bytes().unwrap(),
        recovered.checkpoint_bytes().unwrap()
    );
}

#[test]
fn corrupt_checksum_unknown_version_truncation_and_noncanonical_payloads_reject() {
    let host = fixture();
    let original = host.checkpoint_bytes().unwrap();
    let mut damaged = original.clone();
    *damaged.last_mut().unwrap() ^= 1;
    assert!(HostSequencer::from_checkpoint_bytes(&damaged).is_err());
    let mut version = original.clone();
    version[PREFIX.len() - 2] = b'2';
    assert!(HostSequencer::from_checkpoint_bytes(&version).is_err());
    for length in [0, PREFIX.len(), PREFIX.len() + 31] {
        assert!(HostSequencer::from_checkpoint_bytes(&original[..length]).is_err());
    }
    let mut unknown = payload(&original);
    unknown["unknown_field"] = Json::Bool(true);
    assert!(HostSequencer::from_checkpoint_bytes(&envelope(&unknown)).is_err());
    let mut duplicate_payload = br#"{"sequence":0,"#.to_vec();
    duplicate_payload.extend_from_slice(&original[PREFIX.len() + 33..]);
    let mut duplicate = PREFIX.to_vec();
    duplicate.extend_from_slice(&AssetId::hash(&duplicate_payload).bytes());
    duplicate.extend_from_slice(&duplicate_payload);
    assert!(HostSequencer::from_checkpoint_bytes(&duplicate).is_err());
    assert_eq!(host.checkpoint_bytes().unwrap(), original);
}

#[test]
fn recomputed_checksums_do_not_bypass_revision_record_or_index_validation() {
    let mut host = fixture();
    let mut txn = transaction(&host, 1, 1, 100, vec![width(2.0)]);
    txn.gesture_id = uid(700);
    host.submit(txn, &device(1), CLOCK + 1).unwrap();
    let original = host.checkpoint_bytes().unwrap();
    let base = payload(&original);
    let record_key = id(101).to_string();
    for scenario in 0..10 {
        let mut value = base.clone();
        match scenario {
            0 => value["project"]["title"] = Json::String("Tampered".into()),
            1 => value["sequence"] = Json::from(7),
            2 => {
                value["revisions"].as_object_mut().unwrap().remove("0");
            }
            3 => value["records"][&record_key]["ack"]["host_seq"] = Json::from(0),
            4 => {
                value["records"][&record_key]["transaction"]["ops"][0]["op_id"]["device_id"] =
                    Json::String(device(2).to_string())
            }
            5 => value["seen_ops"] = Json::Array(vec![]),
            6 => value["gestures"] = Json::Array(vec![]),
            7 => {
                let stamp = value["writes"]
                    .as_object_mut()
                    .unwrap()
                    .values_mut()
                    .next()
                    .unwrap();
                stamp["sequence"] = Json::from(9);
            }
            8 => {
                value["records"][&record_key]["changes"][0]["address"]["path"] =
                    serde_json::json!(["state", "unrecognized"])
            }
            _ => value["records"][&record_key]["changes"][0]["unexpected"] = Json::Bool(true),
        }
        assert!(
            HostSequencer::from_checkpoint_bytes(&envelope(&value)).is_err(),
            "scenario {scenario}"
        );
    }
    let mut invalid = base;
    invalid["cancelled"] = invalid["gestures"].clone();
    assert!(HostSequencer::from_checkpoint_bytes(&envelope(&invalid)).is_err());
    assert_eq!(restore(&host).revision().unwrap(), host.revision().unwrap());
}

#[test]
fn tombstones_and_conflict_payloads_remain_bound_to_accepted_history() {
    let mut host = fixture();
    let edit = transaction(&host, 2, 2, 200, vec![width(8.0)]);
    let delete = transaction(
        &host,
        1,
        1,
        100,
        vec![Kind::DeleteObject(pb::DeleteObject { object_id: uid(4) })],
    );
    host.submit(delete, &device(1), CLOCK + 1).unwrap();
    host.submit(edit, &device(2), CLOCK + 2).unwrap();
    let base = payload(&host.checkpoint_bytes().unwrap());
    let mut missing = base.clone();
    missing["tombstones"] = serde_json::json!({});
    assert!(HostSequencer::from_checkpoint_bytes(&envelope(&missing)).is_err());
    let mut wrong_id = base.clone();
    wrong_id["tombstones"][id(4).as_str()]["state"]["object_id"] =
        serde_json::to_value(uid(99)).unwrap();
    assert!(HostSequencer::from_checkpoint_bytes(&envelope(&wrong_id)).is_err());
    let mut removed = base.clone();
    removed["conflicts"] = Json::Array(vec![]);
    assert!(HostSequencer::from_checkpoint_bytes(&envelope(&removed)).is_err());
    let mut wrong_time = base;
    wrong_time["records"][id(102).as_str()]["conflicts"][0]["created_at_ms"] = Json::from(1);
    wrong_time["conflicts"][0]["created_at_ms"] = Json::from(1);
    assert!(HostSequencer::from_checkpoint_bytes(&envelope(&wrong_time)).is_err());
    assert!(restore(&host).cancel_gesture(device(1), id(900)).is_ok());
}
