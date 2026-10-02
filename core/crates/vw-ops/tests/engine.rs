// Failed fixture assumptions should fail this test binary immediately.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use proptest::prelude::*;
use vw_model::{AssetId, DeviceId, Id, Project};
use vw_ops::{Acceptance, HostSequencer, OpsError, UndoManager};
use vw_proto::{
    Message,
    v1::{self as pb, object_state::Shape, op::Kind, property_value::Value},
};

const CLOCK: u64 = 1_700_000_000_000;
fn id(n: u64) -> Id {
    Id::from_parts(CLOCK + n, [0x31; 10]).unwrap()
}
fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
fn uid(n: u64) -> Option<pb::Uuid> {
    Some(id(n).to_proto())
}
fn asset() -> pb::AddAsset {
    pb::AddAsset {
        asset_id: AssetId::hash(b"immutable fixture original").to_string(),
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
fn layer(n: u64) -> Kind {
    Kind::CreateLayer(pb::CreateLayer {
        layer_id: uid(n),
        document_id: uid(2),
        page_index: -1,
        name: "Fixture layer".into(),
        kind: "annotation".into(),
        order_key: "V".into(),
    })
}
fn object(n: u64, owner: &DeviceId) -> pb::ObjectState {
    pb::ObjectState {
        object_id: uid(n),
        layer_id: uid(3),
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
        created_by: owner.to_string(),
        created_at_ms: CLOCK as i64,
        shape: Some(Shape::Rect(pb::RectD {
            x: 0.25,
            y: 1.5,
            w: 12.0,
            h: 9.0,
        })),
        ..Default::default()
    }
}
fn create(n: u64, owner: &DeviceId) -> Kind {
    Kind::CreateObject(pb::CreateObject {
        document_id: uid(2),
        state: Some(object(n, owner)),
    })
}
fn property(n: u64, name: &str, value: Value) -> Kind {
    Kind::SetProperty(pb::SetProperty {
        object_id: uid(n),
        property: name.into(),
        value: Some(pb::PropertyValue { value: Some(value) }),
    })
}
fn width(v: f64) -> Kind {
    property(4, "style.width", Value::Number(v))
}
fn hidden(v: bool) -> Kind {
    property(4, "hidden", Value::Flag(v))
}
fn delete(n: u64) -> Kind {
    Kind::DeleteObject(pb::DeleteObject { object_id: uid(n) })
}
fn group(n: u64, members: &[u64]) -> Kind {
    Kind::Group(pb::Group {
        group_id: uid(n),
        object_ids: members.iter().map(|n| id(*n).to_proto()).collect(),
    })
}
fn transaction(
    host: &HostSequencer,
    owner: &DeviceId,
    n: u64,
    wall: i64,
    kinds: Vec<Kind>,
) -> pb::Transaction {
    pb::Transaction {
        txn_id: uid(10_000 + n),
        project_id: uid(1),
        device_id: owner.to_string(),
        base_revision: Some(host.revision().unwrap()),
        created_at_wall_ms: wall,
        ops: kinds
            .into_iter()
            .enumerate()
            .map(|(i, kind)| pb::Op {
                op_id: Some(pb::OpId {
                    device_id: owner.to_string(),
                    lamport: n * 100 + i as u64 + 1,
                }),
                kind: Some(kind),
            })
            .collect(),
        gesture_id: None,
    }
}
fn accept(
    host: &mut HostSequencer,
    owner: &DeviceId,
    n: u64,
    wall: i64,
    kinds: Vec<Kind>,
) -> Acceptance {
    let txn = transaction(host, owner, n, wall, kinds);
    host.submit(txn, owner, CLOCK as i64 + n as i64).unwrap()
}
fn fixture() -> HostSequencer {
    let owner = device(1);
    let mut host = HostSequencer::new(
        Project::new(id(1), "Operation fixture".into(), owner.clone()),
        owner.clone(),
    )
    .unwrap();
    accept(
        &mut host,
        &owner,
        1,
        CLOCK as i64,
        vec![
            Kind::AddAsset(asset()),
            Kind::CreateDocument(pb::CreateDocument {
                document_id: uid(2),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Fixture document".into(),
                primary_asset_id: asset().asset_id,
                capture: None,
            }),
            layer(3),
            create(4, &owner),
            create(5, &owner),
        ],
    );
    host
}
fn object_width(host: &HostSequencer) -> f64 {
    host.project().objects[&id(4)].state.style.unwrap().width
}
fn decode_value(value: &Option<pb::PropertyValue>) -> serde_json::Value {
    let Some(Value::Raw(bytes)) = value.as_ref().and_then(|v| v.value.as_ref()) else {
        panic!("conflict must preserve canonical typed value");
    };
    serde_json::from_slice(bytes).unwrap()
}
fn inverse(host: &mut HostSequencer, owner: &DeviceId, n: u64, target: &pb::Uuid) -> Acceptance {
    accept(
        host,
        owner,
        n,
        CLOCK as i64 + n as i64,
        vec![Kind::UndoTransaction(pb::UndoTransaction {
            target_txn_id: Some(target.clone()),
        })],
    )
}

#[test]
fn duplicate_ack_is_byte_identical_and_payload_reuse_is_rejected() {
    let mut host = fixture();
    let owner = device(1);
    let txn = transaction(&host, &owner, 2, 100, vec![width(5.0)]);
    let first = host.submit(txn.clone(), &owner, CLOCK as i64).unwrap();
    let revision = host.revision().unwrap();
    let duplicate = host
        .submit(txn.clone(), &owner, CLOCK as i64 + 999)
        .unwrap();
    assert!(duplicate.duplicate);
    assert_eq!(first.ack.encode_to_vec(), duplicate.ack.encode_to_vec());
    assert_eq!(host.revision().unwrap(), revision);
    assert!(duplicate.conflicts.is_empty());
    let mut collision = txn;
    collision.created_at_wall_ms += 1;
    assert!(matches!(
        host.submit(collision, &owner, CLOCK as i64),
        Err(OpsError::IdCollision)
    ));
    assert_eq!(host.revision().unwrap(), revision);
}

#[test]
fn rejection_rolls_back_state_counters_and_gesture_reservation() {
    let mut host = fixture();
    let owner = device(1);
    let before = host.snapshot().unwrap();
    let mut txn = transaction(
        &host,
        &owner,
        2,
        100,
        vec![width(7.0), property(4, "unknown", Value::Flag(true))],
    );
    txn.gesture_id = uid(800);
    assert!(host.submit(txn.clone(), &owner, CLOCK as i64).is_err());
    assert_eq!(host.project(), &before.project);
    assert_eq!(host.revision().unwrap(), before.revision);
    txn.ops.pop();
    let accepted = host.submit(txn, &owner, CLOCK as i64).unwrap();
    assert_eq!(accepted.ack.host_seq, before.revision.host_seq + 1);
    assert_eq!(object_width(&host), 7.0);
}

#[test]
fn authenticated_device_op_identity_base_hash_and_creator_are_validated() {
    let mut host = fixture();
    let owner = device(1);
    let peer = device(2);
    let before = host.revision().unwrap();
    let original = transaction(&host, &owner, 2, 100, vec![width(2.0)]);
    assert!(matches!(
        host.submit(original.clone(), &peer, CLOCK as i64),
        Err(OpsError::DeviceMismatch)
    ));
    let mut wrong_op = original.clone();
    wrong_op.ops[0].op_id.as_mut().unwrap().device_id = peer.to_string();
    assert!(host.submit(wrong_op, &owner, CLOCK as i64).is_err());
    let mut bad_hash = original.clone();
    bad_hash.base_revision.as_mut().unwrap().state_hash[0] ^= 1;
    assert!(matches!(
        host.submit(bad_hash, &owner, CLOCK as i64),
        Err(OpsError::RevisionMismatch)
    ));
    let mut future = original.clone();
    future.base_revision.as_mut().unwrap().host_seq += 1;
    assert!(matches!(
        host.submit(future, &owner, CLOCK as i64),
        Err(OpsError::RevisionMismatch)
    ));
    let forged = transaction(&host, &owner, 2, 100, vec![create(100, &peer)]);
    assert!(matches!(
        host.submit(forged, &owner, CLOCK as i64),
        Err(OpsError::DeviceMismatch)
    ));
    assert_eq!(host.revision().unwrap(), before);
    host.submit(original, &owner, CLOCK as i64).unwrap();
}

#[test]
fn live_host_order_wins_even_when_wall_clock_is_earlier() {
    let mut host = fixture();
    let owner = device(1);
    let peer = device(2);
    accept(&mut host, &owner, 2, 900, vec![width(2.0)]);
    let later = accept(&mut host, &peer, 3, 100, vec![width(3.0)]);
    assert_eq!(object_width(&host), 3.0);
    assert!(later.conflicts.is_empty());
}

#[test]
fn stale_writes_use_later_wall_clock_and_ties_favor_host_in_both_orders() {
    for (first_is_host, incoming_wall, incoming_wins) in [
        (true, 201, true),
        (true, 199, false),
        (true, 200, false),
        (false, 201, true),
        (false, 199, false),
        (false, 200, true),
    ] {
        let mut host = fixture();
        let first = device(if first_is_host { 1 } else { 2 });
        let next = device(if first_is_host { 2 } else { 1 });
        let mut delayed = transaction(&host, &next, 3, incoming_wall, vec![width(3.0)]);
        let base = host.revision().unwrap();
        accept(&mut host, &first, 2, 200, vec![width(2.0)]);
        delayed.base_revision = Some(base);
        let accepted = host
            .submit(delayed.clone(), &next, CLOCK as i64 + 3)
            .unwrap();
        assert_eq!(accepted.conflicts.len(), 1);
        let conflict = &accepted.conflicts[0];
        let expected = if incoming_wins { 3.0 } else { 2.0 };
        let losing = if incoming_wins { 2.0 } else { 3.0 };
        assert_eq!(object_width(&host), expected);
        assert_eq!(
            decode_value(&conflict.kept_value),
            serde_json::json!(expected)
        );
        assert_eq!(
            decode_value(&conflict.other_value),
            serde_json::json!(losing)
        );
        assert_eq!(
            conflict.kept_device,
            if incoming_wins {
                next.to_string()
            } else {
                first.to_string()
            }
        );
        assert!(!conflict.delete_vs_edit);
        assert_eq!(host.conflicts().len(), 1);
        let retry = host.submit(delayed, &next, CLOCK as i64 + 99).unwrap();
        assert!(retry.duplicate);
        assert!(retry.conflicts.is_empty());
        assert_eq!(host.conflicts().len(), 1);
    }
}

#[test]
fn disjoint_stale_properties_merge_without_a_conflict() {
    let mut host = fixture();
    let owner = device(1);
    let peer = device(2);
    let delayed = transaction(&host, &peer, 3, 10, vec![hidden(true)]);
    accept(&mut host, &owner, 2, 100, vec![width(4.0)]);
    let accepted = host.submit(delayed, &peer, CLOCK as i64).unwrap();
    assert!(accepted.conflicts.is_empty());
    assert_eq!(object_width(&host), 4.0);
    assert!(host.project().objects[&id(4)].state.hidden);
}

#[test]
fn value_equal_write_still_owns_the_property_for_conflicts_and_undo() {
    let mut host = fixture();
    let owner = device(1);
    let peer = device(2);
    let first = accept(&mut host, &owner, 2, 100, vec![width(2.0)]);
    let delayed = transaction(&host, &owner, 4, 150, vec![width(3.0)]);
    let before = host.project().state_hash().unwrap();
    accept(&mut host, &peer, 3, 200, vec![width(2.0)]);
    assert_eq!(before, host.project().state_hash().unwrap());
    let accepted = host.submit(delayed, &owner, CLOCK as i64).unwrap();
    assert_eq!(object_width(&host), 2.0);
    assert_eq!(accepted.conflicts.len(), 1);
    assert_eq!(accepted.conflicts[0].kept_device, peer.to_string());
    let undo = inverse(&mut host, &owner, 5, first.ack.txn_id.as_ref().unwrap());
    assert_eq!(object_width(&host), 2.0);
    assert_eq!(undo.ack.notices.len(), 1);
}

#[test]
fn delete_beats_edit_in_either_arrival_order_and_retains_edited_object() {
    for delete_first in [false, true] {
        let mut host = fixture();
        let owner = device(1);
        let peer = device(2);
        let deleting = transaction(&host, &owner, 2, 1, vec![delete(4)]);
        let editing = transaction(&host, &peer, 3, 999, vec![width(7.25)]);
        let final_acceptance = if delete_first {
            host.submit(deleting, &owner, CLOCK as i64).unwrap();
            host.submit(editing, &peer, CLOCK as i64 + 1).unwrap()
        } else {
            host.submit(editing, &peer, CLOCK as i64).unwrap();
            host.submit(deleting, &owner, CLOCK as i64 + 1).unwrap()
        };
        assert!(!host.project().objects.contains_key(&id(4)));
        assert_eq!(final_acceptance.conflicts.len(), 1);
        let record = &final_acceptance.conflicts[0];
        assert!(record.delete_vs_edit);
        assert_eq!(record.kept_device, owner.to_string());
        assert_eq!(record.other_device, peer.to_string());
        assert_eq!(
            record.edited_object.as_ref().unwrap().style.unwrap().width,
            7.25
        );
    }
}

#[test]
fn deleted_object_reorder_is_preserved_as_a_conflict_without_resurrection() {
    let mut host = fixture();
    let owner = device(1);
    let peer = device(2);
    let editing = transaction(
        &host,
        &peer,
        3,
        999,
        vec![Kind::Reorder(pb::Reorder {
            object_id: uid(4),
            layer_id: uid(3),
            order_key: "b".into(),
        })],
    );
    accept(&mut host, &owner, 2, 100, vec![delete(4)]);
    let accepted = host.submit(editing, &peer, CLOCK as i64).unwrap();
    assert!(!host.project().objects.contains_key(&id(4)));
    assert_eq!(accepted.conflicts.len(), 1);
    assert!(accepted.conflicts[0].delete_vs_edit);
    assert_eq!(
        accepted.conflicts[0]
            .edited_object
            .as_ref()
            .unwrap()
            .order_key,
        "b"
    );
    let recreate = transaction(&host, &owner, 4, 1000, vec![create(4, &owner)]);
    assert!(matches!(
        host.submit(recreate, &owner, CLOCK as i64),
        Err(OpsError::AlreadyExists)
    ));
}

#[test]
fn undo_skips_peer_properties_but_restores_independent_properties() {
    let mut host = fixture();
    let owner = device(1);
    let peer = device(2);
    let first = accept(&mut host, &owner, 2, 100, vec![width(2.0), hidden(true)]);
    accept(&mut host, &peer, 3, 200, vec![width(9.0)]);
    let undone = inverse(&mut host, &owner, 4, first.ack.txn_id.as_ref().unwrap());
    assert_eq!(object_width(&host), 9.0);
    assert!(!host.project().objects[&id(4)].state.hidden);
    assert_eq!(undone.ack.notices.len(), 1);
    let redone = inverse(&mut host, &owner, 5, undone.ack.txn_id.as_ref().unwrap());
    assert_eq!(object_width(&host), 9.0);
    assert!(host.project().objects[&id(4)].state.hidden);
    assert!(redone.ack.notices.is_empty());
}

#[test]
fn undo_remembers_intervening_peer_write_after_an_own_overwrite() {
    let mut host = fixture();
    let owner = device(1);
    let peer = device(2);
    let first = accept(&mut host, &owner, 2, 100, vec![width(2.0)]);
    accept(&mut host, &peer, 3, 200, vec![width(3.0)]);
    accept(&mut host, &owner, 4, 300, vec![width(4.0)]);
    let undone = inverse(&mut host, &owner, 5, first.ack.txn_id.as_ref().unwrap());
    assert_eq!(object_width(&host), 4.0);
    assert_eq!(undone.ack.notices.len(), 1);
}

#[test]
fn structural_undo_preserves_parent_of_peer_edited_object_and_reports_notice() {
    let mut host = fixture();
    let owner = device(1);
    let peer = device(2);
    let mut child = object(40, &owner);
    child.layer_id = uid(30);
    let creating = accept(
        &mut host,
        &owner,
        2,
        100,
        vec![
            layer(30),
            Kind::CreateObject(pb::CreateObject {
                document_id: uid(2),
                state: Some(child),
            }),
        ],
    );
    accept(
        &mut host,
        &peer,
        3,
        200,
        vec![property(40, "hidden", Value::Flag(true))],
    );
    let undone = inverse(&mut host, &owner, 4, creating.ack.txn_id.as_ref().unwrap());
    assert!(host.project().layers.contains_key(&id(30)));
    assert!(host.project().objects[&id(40)].state.hidden);
    assert_eq!(undone.ack.notices.len(), 2);
    host.project().validate().unwrap();
}

#[test]
fn another_device_cannot_undo_a_transaction() {
    let mut host = fixture();
    let owner = device(1);
    let peer = device(2);
    let first = accept(&mut host, &owner, 2, 100, vec![width(2.0)]);
    let before = host.revision().unwrap();
    let txn = transaction(
        &host,
        &peer,
        3,
        200,
        vec![Kind::UndoTransaction(pb::UndoTransaction {
            target_txn_id: first.ack.txn_id,
        })],
    );
    assert!(matches!(
        host.submit(txn, &peer, CLOCK as i64),
        Err(OpsError::UndoOwnership)
    ));
    assert_eq!(host.revision().unwrap(), before);
}

#[test]
fn group_cycles_and_cross_document_reorders_reject_atomically() {
    let mut host = fixture();
    let owner = device(1);
    accept(
        &mut host,
        &owner,
        2,
        100,
        vec![group(60, &[4]), group(61, &[60])],
    );
    let before = host.revision().unwrap();
    let cycle = transaction(&host, &owner, 3, 200, vec![group(60, &[61])]);
    assert!(host.submit(cycle, &owner, CLOCK as i64).is_err());
    assert_eq!(host.revision().unwrap(), before);
    let mut other_layer = match layer(80) {
        Kind::CreateLayer(v) => v,
        _ => unreachable!(),
    };
    other_layer.document_id = uid(70);
    accept(
        &mut host,
        &owner,
        4,
        300,
        vec![
            Kind::CreateDocument(pb::CreateDocument {
                document_id: uid(70),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Other".into(),
                ..Default::default()
            }),
            Kind::CreateLayer(other_layer),
        ],
    );
    let before = host.revision().unwrap();
    let reorder = transaction(
        &host,
        &owner,
        5,
        400,
        vec![Kind::Reorder(pb::Reorder {
            object_id: uid(4),
            layer_id: uid(80),
            order_key: "c".into(),
        })],
    );
    assert!(host.submit(reorder, &owner, CLOCK as i64).is_err());
    assert_eq!(host.revision().unwrap(), before);
}

#[test]
fn nested_group_changes_respect_locked_descendants_and_layers() {
    for lock_layer in [false, true] {
        let mut host = fixture();
        let owner = device(1);
        accept(
            &mut host,
            &owner,
            2,
            100,
            vec![group(60, &[4]), group(61, &[60])],
        );
        let lock = if lock_layer {
            Kind::UpdateLayer(pb::UpdateLayer {
                layer_id: uid(3),
                locked: Some(true),
                ..Default::default()
            })
        } else {
            property(4, "locked", Value::Flag(true))
        };
        accept(&mut host, &owner, 3, 200, vec![lock]);
        let before = host.revision().unwrap();
        for (n, kind) in [
            (4, group(62, &[61])),
            (5, Kind::Ungroup(pb::Ungroup { group_id: uid(61) })),
        ] {
            let txn = transaction(&host, &owner, n, 300, vec![kind]);
            assert!(matches!(
                host.submit(txn, &owner, CLOCK as i64),
                Err(OpsError::Locked)
            ));
            assert_eq!(host.revision().unwrap(), before);
        }
    }
}

#[test]
fn immutable_assets_reject_metadata_replacement_and_duplicate_metadata_is_a_noop() {
    let mut host = fixture();
    let owner = device(1);
    let before = host.project().state_hash().unwrap();
    accept(&mut host, &owner, 2, 100, vec![Kind::AddAsset(asset())]);
    assert_eq!(before, host.project().state_hash().unwrap());
    let mut changed = asset();
    changed.width = 1024;
    let txn = transaction(&host, &owner, 3, 200, vec![Kind::AddAsset(changed)]);
    assert!(matches!(
        host.submit(txn, &owner, CLOCK as i64),
        Err(OpsError::AlreadyExists)
    ));
    assert_eq!(before, host.project().state_hash().unwrap());
}

#[test]
fn instructions_semantics_results_and_mask_versions_are_journaled_and_reversible() {
    let mut host = fixture();
    let owner = device(1);
    let before = host.project().state_hash().unwrap();
    let selection = Shape::SelectionVector(pb::Polyline {
        points: vec![
            pb::PointD { x: 0.0, y: 0.0 },
            pb::PointD { x: 10.0, y: 0.0 },
            pb::PointD { x: 0.0, y: 10.0 },
        ],
        closed: true,
    });
    let change = accept(
        &mut host,
        &owner,
        2,
        100,
        vec![
            Kind::SetInstruction(pb::SetInstruction {
                instruction_id: uid(90),
                document_id: uid(2),
                target_object_ids: vec![id(4).to_proto()],
                role: pb::Role::Change as i32,
                text: "Fixture instruction".into(),
                entry_method: "pc_keyboard".into(),
                language: "en".into(),
            }),
            Kind::AddSemanticSnapshot(pb::AddSemanticSnapshot {
                snapshot_id: uid(91),
                document_id: uid(2),
                platform: "uia".into(),
                frame_delta_ms: 12,
                elements_json_zstd: vec![1, 2, 3],
            }),
            Kind::AddResult(pb::AddResult {
                result_id: uid(92),
                document_id: uid(2),
                provider: "fixture".into(),
                model: "fixture".into(),
                request_json: "{}".into(),
                cost_estimate_usd: 0.125,
                ..Default::default()
            }),
            Kind::UpdateResult(pb::UpdateResult {
                result_id: uid(92),
                status: "error".into(),
                ..Default::default()
            }),
            property(
                4,
                "shape",
                Value::Raw(serde_json::to_vec(&selection).unwrap()),
            ),
            Kind::MaskOp(pb::MaskOp {
                output_object_id: uid(4),
                op: "feather".into(),
                amount: 2.5,
                input_object_ids: vec![id(4).to_proto()],
            }),
        ],
    );
    assert_eq!(host.project().instructions.len(), 1);
    assert!(host.project().semantic_snapshots[&id(91)].text_is_untrusted());
    assert_eq!(host.project().results[&id(92)].status, "error");
    assert_eq!(host.project().mask_versions[&id(4)].len(), 1);
    inverse(&mut host, &owner, 3, change.ack.txn_id.as_ref().unwrap());
    assert_eq!(before, host.project().state_hash().unwrap());
}

fn fixed_log() -> HostSequencer {
    let mut host = fixture();
    let owner = device(1);
    let peer = device(2);
    let delayed = transaction(&host, &peer, 3, 300, vec![width(7.5)]);
    accept(
        &mut host,
        &owner,
        2,
        200,
        vec![width(2.25), group(60, &[4, 5])],
    );
    host.submit(delayed, &peer, CLOCK as i64 + 3).unwrap();
    accept(
        &mut host,
        &owner,
        4,
        400,
        vec![
            property(
                4,
                "transform",
                Value::Transform(pb::Affine {
                    a: 1.25,
                    b: 0.125,
                    c: -0.25,
                    d: 0.75,
                    e: 1024.5,
                    f: -32.25,
                }),
            ),
            hidden(true),
        ],
    );
    accept(
        &mut host,
        &peer,
        5,
        500,
        vec![Kind::Reorder(pb::Reorder {
            object_id: uid(5),
            layer_id: uid(3),
            order_key: "m".into(),
        })],
    );
    let deleting = accept(&mut host, &owner, 6, 600, vec![delete(5)]);
    inverse(&mut host, &owner, 7, deleting.ack.txn_id.as_ref().unwrap());
    host
}

#[test]
fn fixed_op_log_has_identical_cross_platform_canonical_hash() {
    let first = fixed_log();
    let second = fixed_log();
    assert_eq!(
        first.project().canonical_bytes().unwrap(),
        second.project().canonical_bytes().unwrap()
    );
    assert_eq!(first.conflicts(), second.conflicts());
    let hash = first.project().state_hash().unwrap().to_string();
    println!("T1.02_FIXED_OP_LOG_STATE_HASH={hash}");
    assert_eq!(
        hash,
        "3120769b42f577ffbedf8c0ac10cc176ae21b1f643ace65f625e979914952ee0"
    );
}

static COMPLETED_SEQUENCES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

proptest! {
    #![proptest_config(ProptestConfig { cases:500, rng_seed: proptest::test_runner::RngSeed::Fixed(0x565701020500), failure_persistence:None, ..ProptestConfig::default() })]
    #[test]
    fn five_hundred_random_sequences_undo_and_redo_identical_hashes(actions in prop::collection::vec((0u8..8,0u16..4096),1..25)) {
        let mut host=fixture(); let owner=device(1); let mut history=UndoManager::new(owner.clone());
        let original=host.project().state_hash().unwrap(); let originals=host.project().assets.clone(); let mut n=10u64;
        for (index,(kind,value)) in actions.iter().enumerate() {
            let v=f64::from(*value)/16.0;
            let op=match kind {
                0=>width(v),
                1=>hidden(value%2==0),
                2=>property(4,"transform",Value::Transform(pb::Affine{a:1.0,d:1.0,e:v,f:-v,..Default::default()})),
                3=>property(4,"style.stroke",Value::Color(pb::Color{rgba:u32::from(*value)*257})),
                4=>property(4,"shape",Value::Raw(serde_json::to_vec(&Shape::Ellipse(pb::RectD{x:v,y:0.5,w:12.0,h:8.0})).unwrap())),
                5=>Kind::Reorder(pb::Reorder{object_id:uid(4),layer_id:uid(3),order_key:if value%2==0{"W"}else{"Y"}.into()}),
                6=>if host.project().groups.contains_key(&id(60)){Kind::Ungroup(pb::Ungroup{group_id:uid(60)})}else{group(60,&[4,5])},
                _=>create(1000+index as u64,&owner),
            };
            let txn=transaction(&host,&owner,n,CLOCK as i64+n as i64,vec![op]); let accepted=host.submit(txn.clone(),&owner,CLOCK as i64+n as i64).unwrap(); history.record(&txn,&accepted).unwrap(); n+=1;
        }
        let final_hash=host.project().state_hash().unwrap();
        for redo in [false,true] {
            for _ in &actions {
                let op=history.operation(redo,n*100+1).unwrap();
                let mut txn=transaction(&host,&owner,n,CLOCK as i64+n as i64,Vec::new()); txn.ops=vec![op];
                let accepted=host.submit(txn.clone(),&owner,CLOCK as i64+n as i64).unwrap();
                prop_assert!(accepted.ack.notices.is_empty()); history.acknowledge(redo,&txn,&accepted).unwrap(); n+=1;
            }
            prop_assert_eq!(host.project().state_hash().unwrap(),if redo{final_hash.clone()}else{original.clone()});
            prop_assert_eq!(&host.project().assets,&originals);
        }
        let completed = COMPLETED_SEQUENCES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if completed.is_multiple_of(50) {
            eprintln!("T1.02_PROPERTY_SEQUENCES_COMPLETED={completed}/500 seed=0x565701020500");
        }
    }
}

#[test]
fn latest_own_leaf_write_supersedes_older_opposing_shape_write() {
    let mut host = fixture();
    let owner = device(1);
    let peer = device(2);
    let text = Shape::Text(pb::TextObject {
        text: "Initial".into(),
        font_family: "Fixture".into(),
        font_size: 12.0,
        anchor: Some(pb::PointD { x: 0.0, y: 0.0 }),
    });
    accept(
        &mut host,
        &owner,
        2,
        900,
        vec![property(
            4,
            "shape",
            Value::Raw(serde_json::to_vec(&text).unwrap()),
        )],
    );
    let first = transaction(
        &host,
        &peer,
        3,
        100,
        vec![property(4, "text.text", Value::Text("First".into()))],
    );
    let second = transaction(
        &host,
        &peer,
        4,
        101,
        vec![property(4, "text.text", Value::Text("Second".into()))],
    );
    host.submit(first, &peer, CLOCK as i64).unwrap();
    let accepted = host.submit(second, &peer, CLOCK as i64 + 1).unwrap();
    assert!(accepted.conflicts.is_empty());
    let Some(Shape::Text(text)) = &host.project().objects[&id(4)].state.shape else {
        panic!("expected text")
    };
    assert_eq!(text.text, "Second");
}

#[test]
fn equal_instruction_result_and_group_writes_preserve_peer_ownership() {
    let mut host = fixture();
    let owner = device(1);
    let peer = device(2);
    let instruction = pb::SetInstruction {
        instruction_id: uid(90),
        document_id: uid(2),
        target_object_ids: vec![id(4).to_proto()],
        role: pb::Role::Change as i32,
        text: "Initial".into(),
        entry_method: "pc_keyboard".into(),
        language: "en".into(),
    };
    accept(
        &mut host,
        &owner,
        2,
        100,
        vec![
            Kind::SetInstruction(instruction.clone()),
            Kind::AddResult(pb::AddResult {
                result_id: uid(92),
                document_id: uid(2),
                provider: "fixture".into(),
                model: "fixture".into(),
                request_json: "{}".into(),
                ..Default::default()
            }),
            group(60, &[4]),
        ],
    );
    let mut changed_instruction = instruction.clone();
    changed_instruction.text = "Delayed".into();
    let delayed_instruction = transaction(
        &host,
        &owner,
        4,
        150,
        vec![Kind::SetInstruction(changed_instruction)],
    );
    let delayed_result = transaction(
        &host,
        &owner,
        5,
        150,
        vec![Kind::UpdateResult(pb::UpdateResult {
            result_id: uid(92),
            status: "error".into(),
            ..Default::default()
        })],
    );
    let delayed_group = transaction(&host, &owner, 6, 150, vec![group(61, &[4])]);
    accept(
        &mut host,
        &peer,
        3,
        200,
        vec![
            Kind::SetInstruction(instruction),
            Kind::UpdateResult(pb::UpdateResult {
                result_id: uid(92),
                status: "pending".into(),
                ..Default::default()
            }),
            group(60, &[4]),
        ],
    );
    let acceptance = host
        .submit(delayed_instruction, &owner, CLOCK as i64)
        .unwrap();
    assert!(
        acceptance
            .conflicts
            .iter()
            .any(|c| c.property.ends_with("definition.text"))
    );
    assert_eq!(
        host.project().instructions[&id(90)].definition.text,
        "Initial"
    );
    let acceptance = host
        .submit(delayed_result, &owner, CLOCK as i64 + 1)
        .unwrap();
    assert!(
        acceptance
            .conflicts
            .iter()
            .any(|c| c.property == "results.status")
    );
    assert_eq!(host.project().results[&id(92)].status, "pending");
    let acceptance = host
        .submit(delayed_group, &owner, CLOCK as i64 + 2)
        .unwrap();
    assert!(
        acceptance
            .conflicts
            .iter()
            .any(|c| c.property.ends_with("state.group_id"))
    );
    assert_eq!(host.project().objects[&id(4)].state.group_id, uid(60));
}

#[test]
fn full_shape_write_stamps_equal_sibling_fields_in_the_same_variant() {
    let mut host = fixture();
    let owner = device(1);
    let peer = device(2);
    let rectangle = |w, h| {
        property(
            4,
            "shape",
            Value::Raw(
                serde_json::to_vec(&Shape::Rect(pb::RectD {
                    x: 0.25,
                    y: 1.5,
                    w,
                    h,
                }))
                .unwrap(),
            ),
        )
    };
    accept(&mut host, &owner, 2, 100, vec![rectangle(12.0, 10.0)]);
    let delayed = transaction(&host, &owner, 4, 150, vec![rectangle(12.0, 30.0)]);
    accept(&mut host, &peer, 3, 200, vec![rectangle(20.0, 10.0)]);
    let accepted = host.submit(delayed, &owner, CLOCK as i64).unwrap();
    assert_eq!(accepted.conflicts.len(), 2);
    let Some(Shape::Rect(rect)) = &host.project().objects[&id(4)].state.shape else {
        panic!("expected rectangle")
    };
    assert_eq!((rect.w, rect.h), (20.0, 10.0));
}

#[test]
fn delete_retains_peer_edit_despite_newer_own_write_to_another_property() {
    let mut host = fixture();
    let owner = device(1);
    let peer = device(2);
    let own_hidden = transaction(&host, &peer, 3, 200, vec![hidden(true)]);
    let own_delete = transaction(&host, &peer, 4, 300, vec![delete(4)]);
    accept(&mut host, &owner, 2, 100, vec![width(9.0)]);
    host.submit(own_hidden, &peer, CLOCK as i64).unwrap();
    let accepted = host.submit(own_delete, &peer, CLOCK as i64 + 1).unwrap();
    assert!(!host.project().objects.contains_key(&id(4)));
    assert_eq!(accepted.conflicts.len(), 1);
    let record = &accepted.conflicts[0];
    assert!(record.delete_vs_edit);
    assert_eq!(record.other_device, owner.to_string());
    let edited = record.edited_object.as_ref().unwrap();
    assert_eq!(edited.style.unwrap().width, 9.0);
    assert!(edited.hidden);
}

#[test]
fn delete_ignores_peer_value_fully_overwritten_by_later_own_write() {
    let mut host = fixture();
    let owner = device(1);
    let peer = device(2);
    let own_width = transaction(&host, &peer, 3, 200, vec![width(8.0)]);
    let own_hidden = transaction(&host, &peer, 4, 210, vec![hidden(true)]);
    let own_delete = transaction(&host, &peer, 5, 300, vec![delete(4)]);
    accept(&mut host, &owner, 2, 100, vec![width(9.0)]);
    host.submit(own_width, &peer, CLOCK as i64).unwrap();
    host.submit(own_hidden, &peer, CLOCK as i64 + 1).unwrap();
    let accepted = host.submit(own_delete, &peer, CLOCK as i64 + 2).unwrap();
    assert!(!host.project().objects.contains_key(&id(4)));
    assert!(accepted.conflicts.is_empty());
}

#[test]
fn atomic_shape_replacement_checks_peer_owned_leaf_after_newer_own_sibling_write() {
    let mut host = fixture();
    let owner = device(1);
    let peer = device(2);
    let text = Shape::Text(pb::TextObject {
        text: "Initial".into(),
        font_family: "Fixture".into(),
        font_size: 12.0,
        anchor: Some(pb::PointD { x: 0.0, y: 0.0 }),
    });
    accept(
        &mut host,
        &owner,
        2,
        50,
        vec![property(
            4,
            "shape",
            Value::Raw(serde_json::to_vec(&text).unwrap()),
        )],
    );
    let own_font = transaction(
        &host,
        &peer,
        4,
        400,
        vec![property(4, "text.font_size", Value::Number(20.0))],
    );
    let ellipse = Shape::Ellipse(pb::RectD {
        x: 0.0,
        y: 0.0,
        w: 10.0,
        h: 10.0,
    });
    let own_shape = transaction(
        &host,
        &peer,
        5,
        150,
        vec![property(
            4,
            "shape",
            Value::Raw(serde_json::to_vec(&ellipse).unwrap()),
        )],
    );
    accept(
        &mut host,
        &owner,
        3,
        300,
        vec![property(
            4,
            "text.text",
            Value::Text("Keep this peer text".into()),
        )],
    );
    host.submit(own_font, &peer, CLOCK as i64).unwrap();
    let accepted = host.submit(own_shape, &peer, CLOCK as i64 + 1).unwrap();
    assert_eq!(accepted.conflicts.len(), 1);
    let Some(Shape::Text(text)) = &host.project().objects[&id(4)].state.shape else {
        panic!("peer text must win")
    };
    assert_eq!(text.text, "Keep this peer text");
    assert_eq!(text.font_size, 20.0);
}
