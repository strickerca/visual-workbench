//! Undo history follows host chronology and survives partial receipt delivery.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use vw_model::{DeviceId, Document, Id, Project};
use vw_ops::{Acceptance, HostSequencer, OpsError, UndoManager};
use vw_proto::v1 as pb;

fn id(number: u8) -> Id {
    Id::from_parts(1_700_000_000_000 + u64::from(number), [number; 10]).unwrap()
}

fn device() -> DeviceId {
    DeviceId::from_bytes([2; 16])
}

fn host() -> HostSequencer {
    let mut project = Project::new(id(1), "Undo receipt fixture".into(), device());
    project.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(2).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                title: "Before".into(),
                schema_version: 1,
                ..Default::default()
            },
            pages: Vec::new(),
            created_at_ms: 1,
        },
    );
    HostSequencer::new(project, DeviceId::from_bytes([1; 16])).unwrap()
}

fn transaction(host: &HostSequencer, number: u8) -> pb::Transaction {
    pb::Transaction {
        txn_id: Some(id(number).to_proto()),
        project_id: Some(id(1).to_proto()),
        device_id: device().to_string(),
        base_revision: Some(host.revision().unwrap()),
        created_at_wall_ms: i64::from(number),
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: device().to_string(),
                lamport: u64::from(number),
            }),
            kind: Some(pb::op::Kind::UpdateDocument(pb::UpdateDocument {
                document_id: Some(id(2).to_proto()),
                title: format!("Edit {number}"),
            })),
        }],
        ..Default::default()
    }
}

fn inverse(host: &HostSequencer, number: u8, target: u8) -> pb::Transaction {
    let mut value = transaction(host, number);
    value.ops[0].kind = Some(pb::op::Kind::UndoTransaction(pb::UndoTransaction {
        target_txn_id: Some(id(target).to_proto()),
    }));
    value
}

fn target(history: &UndoManager, redo: bool) -> Option<Id> {
    history
        .operation(redo, 1000)
        .ok()
        .and_then(|operation| match operation.kind {
            Some(pb::op::Kind::UndoTransaction(inverse)) => {
                Id::from_proto(inverse.target_txn_id.as_ref()).ok()
            }
            _ => None,
        })
}

fn deliver(
    history: &mut UndoManager,
    index: usize,
    txns: &[pb::Transaction; 3],
    acks: &[Acceptance; 3],
) {
    match index {
        0 => history.record(&txns[index], &acks[index]).unwrap(),
        1 => history
            .acknowledge(false, &txns[index], &acks[index])
            .unwrap(),
        _ => history
            .acknowledge(true, &txns[index], &acks[index])
            .unwrap(),
    }
}

fn offline_chain() -> ([pb::Transaction; 3], [Acceptance; 3]) {
    let mut host = host();
    let edit = transaction(&host, 10);
    let undo = inverse(&host, 11, 10);
    let redo = inverse(&host, 12, 11);
    let first = host.submit(edit.clone(), &device(), 100).unwrap();
    let second = host.submit(undo.clone(), &device(), 101).unwrap();
    let third = host.submit(redo.clone(), &device(), 102).unwrap();
    assert_eq!(host.project().documents[&id(2)].definition.title, "Edit 10");
    ([edit, undo, redo], [first, second, third])
}

#[test]
fn reversed_ordinary_acknowledgements_keep_latest_host_edit_on_top() {
    let mut host = host();
    let first = transaction(&host, 10);
    let ack_first = host.submit(first.clone(), &device(), 100).unwrap();
    let second = transaction(&host, 11);
    let ack_second = host.submit(second.clone(), &device(), 101).unwrap();
    let mut history = UndoManager::new(device());
    history.record(&second, &ack_second).unwrap();
    let retry_first = host.submit(first.clone(), &device(), 102).unwrap();
    assert!(retry_first.duplicate);
    history.record(&first, &retry_first).unwrap();
    history.record(&first, &ack_first).unwrap();
    assert_eq!(target(&history, false), Some(id(11)));
    let mut undo = transaction(&host, 12);
    undo.ops = vec![history.operation(false, 12).unwrap()];
    let ack = host.submit(undo.clone(), &device(), 103).unwrap();
    history.acknowledge(false, &undo, &ack).unwrap();
    assert_eq!(host.project().documents[&id(2)].definition.title, "Edit 10");
    assert_eq!(target(&history, false), Some(id(10)));
}

#[test]
fn late_original_ack_does_not_resurrect_an_already_undone_edit() {
    let mut host = host();
    let edit = transaction(&host, 10);
    let edit_ack = host.submit(edit.clone(), &device(), 100).unwrap();
    let undo = inverse(&host, 11, 10);
    let undo_ack = host.submit(undo.clone(), &device(), 101).unwrap();
    let mut history = UndoManager::new(device());
    history.acknowledge(false, &undo, &undo_ack).unwrap();
    assert_eq!(target(&history, false), None);
    history.record(&edit, &edit_ack).unwrap();
    assert_eq!(target(&history, false), None);
    assert_eq!(target(&history, true), Some(id(11)));
    history.record(&edit, &edit_ack).unwrap();
    history.acknowledge(false, &undo, &undo_ack).unwrap();
    assert_eq!(target(&history, false), None);
    assert_eq!(target(&history, true), Some(id(11)));
}

#[test]
fn offline_edit_undo_redo_survives_every_ack_delivery_permutation() {
    let (txns, acks) = offline_chain();
    for delivery in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut history = UndoManager::new(device());
        history.record_pending(&txns[0]).unwrap();
        assert_eq!(target(&history, false), Some(id(10)));
        history.record_pending_inverse(false, &txns[1]).unwrap();
        assert_eq!(target(&history, false), None);
        assert_eq!(target(&history, true), Some(id(11)));
        history.record_pending_inverse(true, &txns[2]).unwrap();
        assert_eq!(target(&history, false), Some(id(12)));
        assert_eq!(target(&history, true), None);
        // Duplicate local delivery and later acknowledgements cannot advance twice.
        history.record_pending(&txns[0]).unwrap();
        history.record_pending_inverse(false, &txns[1]).unwrap();
        history.record_pending_inverse(true, &txns[2]).unwrap();
        for index in delivery {
            deliver(&mut history, index, &txns, &acks);
            deliver(&mut history, index, &txns, &acks);
            assert_eq!(target(&history, false), Some(id(12)));
            assert_eq!(target(&history, true), None);
        }
        history.record_pending(&txns[0]).unwrap();
        assert_eq!(target(&history, false), Some(id(12)));
    }
}

#[test]
fn receipts_without_local_registration_converge_in_every_delivery_order() {
    let (txns, acks) = offline_chain();
    for delivery in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut history = UndoManager::new(device());
        for index in delivery {
            deliver(&mut history, index, &txns, &acks);
        }
        assert_eq!(target(&history, false), Some(id(12)));
        assert_eq!(target(&history, true), None);
    }
}

#[test]
fn later_known_local_edit_remains_latest_while_earlier_receipt_is_missing() {
    let mut host = host();
    let first = transaction(&host, 10);
    let second = transaction(&host, 11);
    let mut history = UndoManager::new(device());
    history.record_pending(&first).unwrap();
    history.record_pending(&second).unwrap();
    let first_ack = host.submit(first.clone(), &device(), 100).unwrap();
    let second_ack = host.submit(second.clone(), &device(), 101).unwrap();
    history.record(&second, &second_ack).unwrap();
    assert_eq!(target(&history, false), Some(id(11)));
    history.record(&first, &first_ack).unwrap();
    assert_eq!(target(&history, false), Some(id(11)));
}

#[test]
fn missing_middle_inverse_receipt_is_replayed_in_host_order() {
    let mut host = host();
    let first = transaction(&host, 10);
    let first_ack = host.submit(first.clone(), &device(), 100).unwrap();
    let second = transaction(&host, 11);
    let second_ack = host.submit(second.clone(), &device(), 101).unwrap();
    let undo_second = inverse(&host, 12, 11);
    let undo_second_ack = host.submit(undo_second.clone(), &device(), 102).unwrap();
    let undo_first = inverse(&host, 13, 10);
    let undo_first_ack = host.submit(undo_first.clone(), &device(), 103).unwrap();
    let mut history = UndoManager::new(device());
    history.record(&first, &first_ack).unwrap();
    history.record(&second, &second_ack).unwrap();
    history
        .acknowledge(false, &undo_first, &undo_first_ack)
        .unwrap();
    history
        .acknowledge(false, &undo_second, &undo_second_ack)
        .unwrap();
    assert_eq!(target(&history, false), None);
    assert_eq!(target(&history, true), Some(id(13)));
    assert_eq!(host.project().documents[&id(2)].definition.title, "Before");
}

#[test]
fn host_order_overrides_conflicting_provisional_order_of_independent_edits() {
    let mut host = host();
    let txns = [
        transaction(&host, 10),
        transaction(&host, 11),
        transaction(&host, 12),
    ];
    let mut history = UndoManager::new(device());
    for txn in &txns {
        history.record_pending(txn).unwrap();
    }
    let third_ack = host.submit(txns[2].clone(), &device(), 100).unwrap();
    let second_ack = host.submit(txns[1].clone(), &device(), 101).unwrap();
    let first_ack = host.submit(txns[0].clone(), &device(), 102).unwrap();
    history.record(&txns[2], &third_ack).unwrap();
    history.record(&txns[0], &first_ack).unwrap();
    history.record(&txns[1], &second_ack).unwrap();
    assert_eq!(target(&history, false), Some(id(10)));
}

#[test]
fn rejected_pending_inverse_restores_cursor_and_discards_its_redo() {
    let (txns, _) = offline_chain();
    let mut history = UndoManager::new(device());
    history.record_pending(&txns[0]).unwrap();
    history.record_pending_inverse(false, &txns[1]).unwrap();
    history.record_pending_inverse(true, &txns[2]).unwrap();
    assert_eq!(
        history.discard_pending(&id(11)).unwrap(),
        vec![id(11), id(12)]
    );
    assert_eq!(target(&history, false), Some(id(10)));
    assert_eq!(target(&history, true), None);
    assert_eq!(history.discard_pending(&id(10)).unwrap(), vec![id(10)]);
    assert_eq!(target(&history, false), None);
}

#[test]
fn rejecting_pending_ordinary_removes_dependent_inverses_but_preserves_other_edits() {
    let (txns, _) = offline_chain();
    let mut history = UndoManager::new(device());
    history.record_pending(&txns[0]).unwrap();
    history.record_pending_inverse(false, &txns[1]).unwrap();
    history.record_pending_inverse(true, &txns[2]).unwrap();
    let unrelated = transaction(&host(), 13);
    history.record_pending(&unrelated).unwrap();
    assert_eq!(
        history.discard_pending(&id(10)).unwrap(),
        vec![id(10), id(11), id(12)]
    );
    assert_eq!(target(&history, false), Some(id(13)));
    assert_eq!(target(&history, true), None);
}

#[test]
fn rejection_cannot_remove_accepted_history_or_accepted_inverse_dependencies() {
    let (txns, acks) = offline_chain();
    let mut history = UndoManager::new(device());
    history.record_pending(&txns[0]).unwrap();
    history.record_pending_inverse(false, &txns[1]).unwrap();
    history.acknowledge(false, &txns[1], &acks[1]).unwrap();
    assert!(history.discard_pending(&id(10)).is_err());
    assert_eq!(target(&history, true), Some(id(11)));
    history.record(&txns[0], &acks[0]).unwrap();
    assert!(history.discard_pending(&id(10)).is_err());
    assert!(history.discard_pending(&id(80)).is_err());
    assert_eq!(target(&history, false), None);
    assert_eq!(target(&history, true), Some(id(11)));
}

#[test]
fn malformed_pending_registration_and_payload_collisions_are_atomic() {
    let host = host();
    let edit = transaction(&host, 10);
    let later = transaction(&host, 11);
    let mut history = UndoManager::new(device());
    history.record_pending(&edit).unwrap();
    history.record_pending(&later).unwrap();
    let stale_inverse = inverse(&host, 12, 10);
    assert!(
        history
            .record_pending_inverse(false, &stale_inverse)
            .is_err()
    );
    assert_eq!(target(&history, false), Some(id(11)));
    for change in 0..5 {
        let mut invalid = transaction(&host, 15);
        match change {
            0 => invalid.ops.clear(),
            1 => invalid.project_id = Some(id(80).to_proto()),
            2 => invalid.device_id = DeviceId::from_bytes([4; 16]).to_string(),
            3 => invalid.ops[0].op_id.as_mut().unwrap().lamport = 0,
            _ => invalid.txn_id = None,
        }
        assert!(history.record_pending(&invalid).is_err());
        assert_eq!(target(&history, false), Some(id(11)));
    }
    let mut collision = later.clone();
    collision.created_at_wall_ms += 1;
    assert!(matches!(
        history.record_pending(&collision),
        Err(OpsError::IdCollision)
    ));
    assert_eq!(target(&history, false), Some(id(11)));
    history.record_pending(&later).unwrap();
    let correct = inverse(&host, 12, 11);
    history.record_pending_inverse(false, &correct).unwrap();
    assert_eq!(target(&history, false), Some(id(10)));
    assert_eq!(target(&history, true), Some(id(12)));
}

#[test]
fn acknowledgement_collisions_identity_and_impossible_inverse_order_are_atomic() {
    let (txns, acks) = offline_chain();
    let mut history = UndoManager::new(device());
    history.record(&txns[0], &acks[0]).unwrap();
    let mut collision = acks[0].clone();
    collision.ack.state_hash[0] ^= 1;
    assert!(matches!(
        history.record(&txns[0], &collision),
        Err(OpsError::IdCollision)
    ));
    let mut bad_identity = acks[1].clone();
    bad_identity.ack.txn_id = Some(id(90).to_proto());
    assert!(history.acknowledge(false, &txns[1], &bad_identity).is_err());
    let mut bad_sequence = acks[1].clone();
    bad_sequence.ack.host_seq = acks[0].ack.host_seq;
    assert!(history.acknowledge(false, &txns[1], &bad_sequence).is_err());
    let mut bad_hash = acks[1].clone();
    bad_hash.ack.state_hash.pop();
    assert!(history.acknowledge(false, &txns[1], &bad_hash).is_err());
    assert!(history.acknowledge(true, &txns[1], &acks[1]).is_err());
    assert_eq!(target(&history, false), Some(id(10)));
    history.acknowledge(false, &txns[1], &acks[1]).unwrap();
    assert_eq!(target(&history, false), None);
    assert_eq!(target(&history, true), Some(id(11)));

    let mut deferred = UndoManager::new(device());
    deferred.acknowledge(false, &txns[1], &acks[1]).unwrap();
    let mut future_original = acks[0].clone();
    future_original.ack.host_seq = 50;
    assert!(deferred.record(&txns[0], &future_original).is_err());
    deferred.record(&txns[0], &acks[0]).unwrap();
    assert_eq!(target(&deferred, false), None);
    assert_eq!(target(&deferred, true), Some(id(11)));
}
