//! Replica, local history delivery and ephemeral gesture regressions.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use vw_model::{DeviceId, Document, Id, Project};
use vw_ops::{GestureStore, HostSequencer, Replica, UndoManager};
use vw_proto::v1 as pb;

fn id(n: u8) -> Id {
    Id::from_parts(1_700_000_000_000 + u64::from(n), [n; 10]).unwrap()
}
fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
fn host() -> HostSequencer {
    let mut project = Project::new(id(1), "Replica fixture".into(), device(1));
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
    HostSequencer::new(project, device(1)).unwrap()
}
fn txn(host: &HostSequencer, who: u8, number: u8, title: &str) -> pb::Transaction {
    pb::Transaction {
        txn_id: Some(id(number).to_proto()),
        project_id: Some(id(1).to_proto()),
        device_id: device(who).to_string(),
        base_revision: Some(host.revision().unwrap()),
        created_at_wall_ms: i64::from(number),
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: device(who).to_string(),
                lamport: u64::from(number),
            }),
            kind: Some(pb::op::Kind::UpdateDocument(pb::UpdateDocument {
                document_id: Some(id(2).to_proto()),
                title: title.into(),
            })),
        }],
        ..Default::default()
    }
}

#[test]
fn two_replicas_rebase_interleaved_pending_and_acknowledgements() {
    let mut host = host();
    let mut a = Replica::new(device(1), host.snapshot().unwrap()).unwrap();
    let mut b = Replica::new(device(2), host.snapshot().unwrap()).unwrap();
    let first = txn(&host, 1, 10, "Host edit");
    let second = txn(&host, 2, 11, "Phone edit");
    let third = txn(&host, 2, 12, "Later phone edit");
    a.queue(first.clone()).unwrap();
    b.queue(second.clone()).unwrap();
    b.queue(third.clone()).unwrap();
    assert_eq!(
        b.project().documents[&id(2)].definition.title,
        "Later phone edit"
    );
    host.submit(first, &device(1), 100).unwrap();
    a.receive(host.snapshot().unwrap()).unwrap();
    b.receive(host.snapshot().unwrap()).unwrap();
    assert_eq!(a.pending().len(), 0);
    assert_eq!(b.pending().len(), 2);
    host.submit(second, &device(2), 101).unwrap();
    b.receive(host.snapshot().unwrap()).unwrap();
    assert_eq!(b.pending().len(), 1);
    assert_eq!(
        b.project().documents[&id(2)].definition.title,
        "Later phone edit"
    );
    host.submit(third, &device(2), 102).unwrap();
    a.receive(host.snapshot().unwrap()).unwrap();
    b.receive(host.snapshot().unwrap()).unwrap();
    assert_eq!(
        a.project().state_hash().unwrap(),
        b.project().state_hash().unwrap()
    );
    assert_eq!(
        b.project().state_hash().unwrap(),
        host.project().state_hash().unwrap()
    );
    assert!(b.pending().is_empty());
}

#[test]
fn replica_preserves_blocked_pending_and_rejects_forged_ack_atomically() {
    let host = host();
    let mut replica = Replica::new(device(2), host.snapshot().unwrap()).unwrap();
    let edit = txn(&host, 2, 10, "Offline");
    replica.queue(edit.clone()).unwrap();
    let mut snapshot = host.snapshot().unwrap();
    snapshot.project.documents.clear();
    snapshot.revision.host_seq = 1;
    snapshot.revision.state_hash = snapshot.project.state_hash().unwrap().bytes().to_vec();
    replica.receive(snapshot.clone()).unwrap();
    assert_eq!(replica.pending().len(), 1);
    assert!(replica.pending()[0].blocked_reason.is_some());
    assert!(replica.project().documents.is_empty());
    let before = replica.project().state_hash().unwrap();
    let mut forged = edit.clone();
    forged.created_at_wall_ms += 1;
    snapshot.accepted.insert(
        id(10),
        (
            forged,
            pb::TxnAck {
                txn_id: edit.txn_id,
                host_seq: 1,
                state_hash: snapshot.revision.state_hash.clone(),
                notices: vec![],
            },
        ),
    );
    assert!(replica.receive(snapshot).is_err());
    assert_eq!(replica.project().state_hash().unwrap(), before);
    assert_eq!(replica.pending().len(), 1);
}

#[test]
fn replica_rejects_bad_operation_identity_and_reused_transaction_payload() {
    let host = host();
    let mut replica = Replica::new(device(2), host.snapshot().unwrap()).unwrap();
    let edit = txn(&host, 2, 10, "Offline");
    let mut invalid = edit.clone();
    invalid.ops[0].op_id.as_mut().unwrap().device_id = device(1).to_string();
    assert!(replica.queue(invalid).is_err());
    assert!(replica.pending().is_empty());
    replica.queue(edit.clone()).unwrap();
    replica.queue(edit.clone()).unwrap();
    assert_eq!(replica.pending().len(), 1);
    let mut invalid = edit;
    invalid.created_at_wall_ms += 1;
    assert!(replica.queue(invalid).is_err());
}

#[test]
fn undo_manager_handles_lost_ack_and_repeated_local_delivery() {
    let mut host = host();
    let mut history = UndoManager::new(device(2));
    let edit = txn(&host, 2, 10, "Changed");
    let ack = host.submit(edit.clone(), &device(2), 100).unwrap();
    let retry = host.submit(edit.clone(), &device(2), 101).unwrap();
    assert!(retry.duplicate);
    history.record(&edit, &retry).unwrap();
    history.record(&edit, &ack).unwrap();
    let mut inverse = txn(&host, 2, 11, "unused");
    inverse.ops = vec![history.operation(false, 11).unwrap()];
    let ack = host.submit(inverse.clone(), &device(2), 102).unwrap();
    let retry = host.submit(inverse.clone(), &device(2), 103).unwrap();
    history.acknowledge(false, &inverse, &retry).unwrap();
    history.acknowledge(false, &inverse, &ack).unwrap();
    assert!(history.operation(false, 12).is_err());
    let mut redo = txn(&host, 2, 12, "unused");
    redo.ops = vec![history.operation(true, 12).unwrap()];
    let ack = host.submit(redo.clone(), &device(2), 104).unwrap();
    history.acknowledge(true, &redo, &ack).unwrap();
    assert_eq!(host.project().documents[&id(2)].definition.title, "Changed");
    assert!(history.operation(true, 13).is_err());
}

#[test]
fn replica_optimistically_undoes_then_rebases_around_new_peer_write() {
    let mut host = host();
    let mut history = UndoManager::new(device(2));
    let edit = txn(&host, 2, 10, "Phone edit");
    let ack = host.submit(edit.clone(), &device(2), 100).unwrap();
    history.record(&edit, &ack).unwrap();
    let mut replica = Replica::new(device(2), host.snapshot().unwrap()).unwrap();
    let mut inverse = txn(&host, 2, 11, "unused");
    inverse.ops = vec![history.operation(false, 11).unwrap()];
    replica.queue(inverse.clone()).unwrap();
    assert_eq!(
        replica.project().documents[&id(2)].definition.title,
        "Before"
    );
    let peer = txn(&host, 1, 12, "Peer edit");
    host.submit(peer, &device(1), 101).unwrap();
    replica.receive(host.snapshot().unwrap()).unwrap();
    assert_eq!(
        replica.project().documents[&id(2)].definition.title,
        "Peer edit"
    );
    assert!(replica.pending()[0].blocked_reason.is_none());
    let ack = host.submit(inverse, &device(2), 102).unwrap();
    assert_eq!(ack.ack.notices.len(), 1);
    replica.receive(host.snapshot().unwrap()).unwrap();
    assert!(replica.pending().is_empty());
    assert_eq!(
        replica.project().state_hash().unwrap(),
        host.project().state_hash().unwrap()
    );
}

#[test]
fn replica_offline_pending_undo_redo_survives_rebase_and_reconnect() {
    let mut host = host();
    let mut replica = Replica::new(device(2), host.snapshot().unwrap()).unwrap();
    let mut history = UndoManager::new(device(2));
    let edit = txn(&host, 2, 10, "Offline edit");
    replica.queue(edit.clone()).unwrap();
    history.record_pending(&edit).unwrap();
    let mut undo = txn(&host, 2, 11, "unused");
    undo.ops = vec![history.operation(false, 11).unwrap()];
    replica.queue(undo.clone()).unwrap();
    history.record_pending_inverse(false, &undo).unwrap();
    assert_eq!(
        replica.project().documents[&id(2)].definition.title,
        "Before"
    );
    let mut redo = txn(&host, 2, 12, "unused");
    redo.ops = vec![history.operation(true, 12).unwrap()];
    replica.queue(redo.clone()).unwrap();
    history.record_pending_inverse(true, &redo).unwrap();
    assert_eq!(
        replica.project().documents[&id(2)].definition.title,
        "Offline edit"
    );
    replica.receive(host.snapshot().unwrap()).unwrap();
    assert_eq!(replica.pending().len(), 3);
    for (index, transaction) in [edit, undo, redo].into_iter().enumerate() {
        let acceptance = host
            .submit(transaction.clone(), &device(2), 100 + index as i64)
            .unwrap();
        match index {
            0 => history.record(&transaction, &acceptance).unwrap(),
            1 => history
                .acknowledge(false, &transaction, &acceptance)
                .unwrap(),
            _ => history
                .acknowledge(true, &transaction, &acceptance)
                .unwrap(),
        }
        replica.receive(host.snapshot().unwrap()).unwrap();
        assert!(replica.pending().iter().all(|p| p.blocked_reason.is_none()));
    }
    assert!(replica.pending().is_empty());
    assert_eq!(
        replica.project().state_hash().unwrap(),
        host.project().state_hash().unwrap()
    );
    assert_eq!(
        host.project().documents[&id(2)].definition.title,
        "Offline edit"
    );
}

#[test]
fn explicitly_discarding_pending_work_removes_its_inverse_chain_only() {
    let host = host();
    let mut replica = Replica::new(device(2), host.snapshot().unwrap()).unwrap();
    let mut history = UndoManager::new(device(2));
    let edit = txn(&host, 2, 10, "Discard this");
    replica.queue(edit.clone()).unwrap();
    history.record_pending(&edit).unwrap();
    let mut undo = txn(&host, 2, 11, "unused");
    undo.ops = vec![history.operation(false, 11).unwrap()];
    replica.queue(undo.clone()).unwrap();
    history.record_pending_inverse(false, &undo).unwrap();
    let mut redo = txn(&host, 2, 12, "unused");
    redo.ops = vec![history.operation(true, 12).unwrap()];
    replica.queue(redo.clone()).unwrap();
    history.record_pending_inverse(true, &redo).unwrap();
    let retained = txn(&host, 2, 13, "Keep this");
    replica.queue(retained.clone()).unwrap();
    history.record_pending(&retained).unwrap();
    let removed = replica.discard_pending(&id(10)).unwrap();
    assert_eq!(removed, vec![id(10), id(11), id(12)]);
    assert_eq!(history.discard_pending(&id(10)).unwrap(), removed);
    assert_eq!(replica.pending().len(), 1);
    assert_eq!(
        replica.project().documents[&id(2)].definition.title,
        "Keep this"
    );
    replica.discard_pending(&id(13)).unwrap();
    history.discard_pending(&id(13)).unwrap();
    assert_eq!(
        replica.project().state_hash().unwrap(),
        host.project().state_hash().unwrap()
    );
    assert!(history.operation(false, 14).is_err());
}

fn preview(seq: u32) -> pb::GestureUpdate {
    pb::GestureUpdate {
        gesture_id: Some(id(5).to_proto()),
        document_id: Some(id(2).to_proto()),
        seq,
        kind: pb::GestureKind::Slider as i32,
        slider_value: 0.5,
        ..Default::default()
    }
}
#[test]
fn gesture_expiry_retains_sequence_watermarks_and_cancel_suppresses_late_packets() {
    let mut store = GestureStore::new();
    assert!(store.update(device(2), preview(10), 0).unwrap());
    assert!(!store.update(device(2), preview(9), 999).unwrap());
    assert!(store.expire(999).is_empty());
    assert_eq!(store.expire(1000), vec![id(5)]);
    assert!(!store.update(device(2), preview(9), 1001).unwrap());
    assert!(store.get(&device(2), &id(5)).is_none());
    assert!(store.update(device(2), preview(11), 1002).unwrap());
    assert!(store.finish(device(2), id(5)).unwrap());
    assert!(!store.update(device(2), preview(12), 1003).unwrap());
}
#[test]
fn gesture_payloads_and_bindings_are_validated_without_mutation() {
    let mut store = GestureStore::new();
    store.update(device(2), preview(1), 0).unwrap();
    let mut invalid = preview(2);
    invalid.document_id = Some(id(3).to_proto());
    assert!(store.update(device(2), invalid, 1).is_err());
    let mut invalid = preview(2);
    invalid.handle_positions.push(pb::PointD {
        x: f64::NAN,
        y: 0.0,
    });
    assert!(store.update(device(2), invalid, 1).is_err());
    let mut invalid = preview(2);
    invalid.stroke_delta = Some(pb::Stroke::default());
    assert!(store.update(device(2), invalid, 1).is_err());
    let mut invalid = preview(2);
    invalid.preview_state = Some(pb::ObjectState::default());
    assert!(store.update(device(2), invalid, 1).is_err());
    assert_eq!(store.get(&device(2), &id(5)).unwrap().seq, 1);
}
