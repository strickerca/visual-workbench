#![allow(clippy::unwrap_used)]
use vw_model::{DeviceId, Document, Id, Project};
use vw_ops::{HostSequencer, Replica};
use vw_proto::v1 as pb;
fn id(n: u8) -> Id {
    Id::from_parts(1_700_000_000_000, [n; 10]).unwrap()
}
fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
fn title(base: pb::Revision, n: u8, who: u8, text: &str) -> pb::Transaction {
    pb::Transaction {
        txn_id: Some(id(n).to_proto()),
        project_id: Some(id(1).to_proto()),
        device_id: device(who).to_string(),
        base_revision: Some(base),
        created_at_wall_ms: i64::from(n),
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: device(who).to_string(),
                lamport: u64::from(n),
            }),
            kind: Some(pb::op::Kind::UpdateDocument(pb::UpdateDocument {
                document_id: Some(id(2).to_proto()),
                title: text.into(),
            })),
        }],
        ..Default::default()
    }
}
#[test]
fn recovery_keeps_original_base_and_replays_over_an_advanced_host() {
    let mut project = Project::new(id(1), "recovery".into(), device(1));
    project.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(2).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "initial".into(),
                ..Default::default()
            },
            pages: vec![],
            created_at_ms: 0,
        },
    );
    let mut host = HostSequencer::new(project, device(1)).unwrap();
    let initial = host.revision().unwrap();
    let offline = title(initial.clone(), 10, 2, "offline");
    host.submit(title(initial, 11, 1, "host"), &device(1), 11)
        .unwrap();
    let mut replica = Replica::new(device(2), host.snapshot().unwrap()).unwrap();
    assert!(replica.queue(offline.clone()).is_err());
    replica.queue_recovered(offline.clone()).unwrap();
    replica.queue_recovered(offline.clone()).unwrap();
    assert_eq!(replica.pending().len(), 1);
    assert_eq!(replica.pending()[0].transaction, offline);
    assert_eq!(
        replica.project().documents[&id(2)].definition.title,
        "offline"
    );
    let before = replica.project().state_hash().unwrap();
    let mut forged = offline.clone();
    forged.device_id = device(3).to_string();
    assert!(replica.queue_recovered(forged).is_err());
    let mut collision = offline;
    collision.created_at_wall_ms += 1;
    assert!(replica.queue_recovered(collision).is_err());
    assert_eq!(replica.project().state_hash().unwrap(), before);
}

fn fixture() -> HostSequencer {
    let mut project = Project::new(id(1), "batch recovery".into(), device(1));
    project.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(2).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "initial".into(),
                ..Default::default()
            },
            pages: vec![],
            created_at_ms: 0,
        },
    );
    HostSequencer::new(project, device(1)).unwrap()
}
fn inverse(base: pb::Revision, n: u8, target: u8) -> pb::Transaction {
    let mut txn = title(base, n, 2, "unused");
    txn.ops[0].kind = Some(pb::op::Kind::UndoTransaction(pb::UndoTransaction {
        target_txn_id: Some(id(target).to_proto()),
    }));
    txn
}
fn pending_bytes(replica: &Replica) -> Vec<(Vec<u8>, Option<String>)> {
    use vw_proto::Message;
    replica
        .pending()
        .iter()
        .map(|pending| {
            (
                pending.transaction.encode_to_vec(),
                pending.blocked_reason.clone(),
            )
        })
        .collect()
}
fn assert_same(before: &Replica, after: &Replica) {
    assert_eq!(before.project(), after.project());
    assert_eq!(before.revision(), after.revision());
    assert_eq!(pending_bytes(before), pending_bytes(after));
    assert_eq!(
        before.workspace_estimate_bytes(u64::MAX).unwrap(),
        after.workspace_estimate_bytes(u64::MAX).unwrap()
    );
}

#[test]
fn batch_preserves_exact_order_duplicates_and_pending_inverse_chain() {
    let mut host = fixture();
    let base = host.revision().unwrap();
    let change = title(base.clone(), 10, 2, "offline");
    let undo = inverse(base.clone(), 11, 10);
    let redo = inverse(base.clone(), 12, 11);
    host.submit(title(base, 20, 1, "host"), &device(1), 20)
        .unwrap();
    let mut batch = Replica::new(device(2), host.snapshot().unwrap()).unwrap();
    let mut single = batch.clone();
    let original = vec![change.clone(), undo.clone(), redo.clone()];
    for txn in &original {
        single.queue_recovered(txn.clone()).unwrap();
    }
    batch
        .queue_recovered_batch(vec![change.clone(), change, undo, redo])
        .unwrap();
    assert_same(&single, &batch);
    assert_eq!(
        batch.project().documents[&id(2)].definition.title,
        "offline"
    );
    assert!(batch.pending().iter().all(|p| p.blocked_reason.is_none()));
    let before = batch.clone();
    batch.queue_recovered_batch(original).unwrap();
    batch.queue_recovered_batch(vec![]).unwrap();
    assert_same(&before, &batch);
    batch
        .queue(inverse(batch.revision().clone(), 13, 12))
        .unwrap();
    assert_eq!(batch.project().documents[&id(2)].definition.title, "host");
}

#[test]
fn invalid_late_item_rolls_back_every_observable_and_inverse_owner() {
    let host = fixture();
    let base = host.revision().unwrap();
    let mut replica = Replica::new(device(2), host.snapshot().unwrap()).unwrap();
    replica
        .queue(title(base.clone(), 10, 2, "retained"))
        .unwrap();
    let valid = title(base.clone(), 11, 2, "must not appear");
    let mut invalid = Vec::new();
    let mut bad = title(base.clone(), 12, 3, "foreign author");
    invalid.push(bad.clone());
    bad = title(base.clone(), 12, 2, "foreign project");
    bad.project_id = Some(id(99).to_proto());
    invalid.push(bad);
    bad = title(base.clone(), 12, 2, "future base");
    bad.base_revision.as_mut().unwrap().host_seq += 1;
    invalid.push(bad);
    bad = title(base.clone(), 12, 2, "different current base hash");
    bad.base_revision.as_mut().unwrap().state_hash[0] ^= 1;
    invalid.push(bad);
    bad = title(base.clone(), 12, 2, "missing ID");
    bad.txn_id = None;
    invalid.push(bad);
    bad = title(base.clone(), 12, 2, "missing base");
    bad.base_revision = None;
    invalid.push(bad);
    bad = title(base.clone(), 12, 2, "negative time");
    bad.created_at_wall_ms = -1;
    invalid.push(bad);
    bad = title(base.clone(), 12, 2, "counter reuse");
    bad.ops.push(bad.ops[0].clone());
    invalid.push(bad);
    bad = title(base.clone(), 12, 2, "zero counter");
    bad.ops[0].op_id.as_mut().unwrap().lamport = 0;
    invalid.push(bad);
    bad = title(base.clone(), 12, 2, "empty");
    bad.ops.clear();
    invalid.push(bad);
    bad = title(base.clone(), 12, 2, "too many operations");
    bad.ops = vec![bad.ops[0].clone(); 4097];
    invalid.push(bad);
    for bad in invalid {
        let before = replica.clone();
        assert!(
            replica
                .queue_recovered_batch(vec![valid.clone(), bad])
                .is_err()
        );
        assert_same(&before, &replica);
        // Probe the private inverse owner through the ordinary public path.
        let mut probe = replica.clone();
        probe.queue(inverse(base.clone(), 13, 10)).unwrap();
        assert_eq!(
            probe.project().documents[&id(2)].definition.title,
            "initial"
        );
        assert!(probe.queue(inverse(base.clone(), 14, 11)).is_err());
    }
}

#[test]
fn collisions_within_batch_existing_pending_and_accepted_history_are_atomic() {
    let mut host = fixture();
    let base = host.revision().unwrap();
    let accepted = title(base.clone(), 9, 2, "accepted");
    host.submit(accepted.clone(), &device(2), 9).unwrap();
    let mut replica = Replica::new(device(2), host.snapshot().unwrap()).unwrap();
    let first = title(base.clone(), 10, 2, "first");
    let mut collision = first.clone();
    collision.created_at_wall_ms += 1;
    let before = replica.clone();
    assert!(
        replica
            .queue_recovered_batch(vec![first.clone(), collision.clone()])
            .is_err()
    );
    assert_same(&before, &replica);
    assert!(
        replica
            .queue_recovered_batch(vec![first.clone(), accepted])
            .is_err()
    );
    assert_same(&before, &replica);
    replica.queue_recovered(first.clone()).unwrap();
    let before = replica.clone();
    assert!(
        replica
            .queue_recovered_batch(vec![title(base, 11, 2, "new"), collision])
            .is_err()
    );
    assert_same(&before, &replica);
    replica.queue_recovered_batch(vec![first]).unwrap();
    assert_same(&before, &replica);
}

#[test]
fn blocked_edits_and_their_dependents_remain_exact_while_other_work_replays() {
    let host = fixture();
    let base = host.revision().unwrap();
    let mut blocked = title(base.clone(), 10, 2, "missing document");
    if let Some(pb::op::Kind::UpdateDocument(value)) = &mut blocked.ops[0].kind {
        value.document_id = Some(id(99).to_proto());
    }
    let dependent = inverse(base.clone(), 11, 10);
    let valid = title(base, 12, 2, "independent");
    let mut replica = Replica::new(device(2), host.snapshot().unwrap()).unwrap();
    replica
        .queue_recovered_batch(vec![blocked.clone(), dependent.clone(), valid.clone()])
        .unwrap();
    assert_eq!(replica.pending().len(), 3);
    assert_eq!(replica.pending()[0].transaction, blocked);
    assert_eq!(replica.pending()[1].transaction, dependent);
    assert_eq!(replica.pending()[2].transaction, valid);
    assert!(replica.pending()[0].blocked_reason.is_some());
    assert!(replica.pending()[1].blocked_reason.is_some());
    assert!(replica.pending()[2].blocked_reason.is_none());
    assert_eq!(
        replica.project().documents[&id(2)].definition.title,
        "independent"
    );
}

#[test]
fn five_thousand_capacity_is_finite_and_exact_retries_work_when_full() {
    let host = fixture();
    let base = host.revision().unwrap();
    let mut replica = Replica::new(device(2), host.snapshot().unwrap()).unwrap();
    let transactions: Vec<_> = (0..5000u64)
        .map(|n| {
            let mut txn = title(base.clone(), 10, 2, "bounded");
            txn.txn_id = Some(
                Id::from_parts(1_700_000_001_000 + n, [0x55; 10])
                    .unwrap()
                    .to_proto(),
            );
            txn.ops[0].op_id.as_mut().unwrap().lamport = n + 100;
            txn
        })
        .collect();
    replica
        .queue_recovered_batch(transactions[..4999].to_vec())
        .unwrap();
    replica.queue(transactions[4999].clone()).unwrap();
    assert_eq!(replica.pending().len(), 5000);
    let before = replica.clone();
    replica.queue_recovered_batch(transactions.clone()).unwrap();
    assert_same(&before, &replica);
    replica.queue(transactions[0].clone()).unwrap();
    assert!(
        replica
            .queue(title(base.clone(), 11, 2, "overflow"))
            .is_err()
    );
    assert!(
        replica
            .queue_recovered_batch(vec![title(base, 12, 2, "overflow")])
            .is_err()
    );
    assert_same(&before, &replica);
    let mut empty = Replica::new(device(2), host.snapshot().unwrap()).unwrap();
    let prior = empty.clone();
    let mut oversized = transactions;
    oversized.push(oversized[0].clone());
    assert!(empty.queue_recovered_batch(oversized).is_err());
    assert_same(&prior, &empty);
}

#[test]
fn borrowed_byte_admission_refuses_oversized_blocked_input_before_retaining_it() {
    let host = fixture();
    let base = host.revision().unwrap();
    let mut replica = Replica::new(device(2), host.snapshot().unwrap()).unwrap();
    replica
        .queue(title(base.clone(), 10, 2, "retained"))
        .unwrap();
    let before = replica.clone();
    let oversized = title(base, 11, 2, &"x".repeat(64 * 1024 * 1024));
    assert!(
        replica
            .queue_recovered_batch(vec![oversized.clone()])
            .is_err()
    );
    assert_same(&before, &replica);
    assert!(replica.queue_recovered(oversized.clone()).is_err());
    assert!(replica.queue(oversized).is_err());
    assert_same(&before, &replica);
}

#[test]
fn combined_retained_and_raw_batch_bytes_are_bounded_even_when_ids_are_duplicates() {
    let host = fixture();
    let base = host.revision().unwrap();
    let large = title(base.clone(), 10, 2, &"x".repeat(33 * 1024 * 1024));
    let another = title(base, 11, 2, &"y".repeat(33 * 1024 * 1024));
    let mut replica = Replica::new(device(2), host.snapshot().unwrap()).unwrap();
    let before = replica.clone();
    assert!(
        replica
            .queue_recovered_batch(vec![large.clone(), large.clone()])
            .is_err()
    );
    assert_same(&before, &replica);
    replica.queue_recovered(large.clone()).unwrap();
    // Content rejected by the normal planner remains a bounded blocked edit.
    assert!(replica.pending()[0].blocked_reason.is_some());
    let before = replica.clone();
    assert!(
        replica
            .queue_recovered_batch(vec![another.clone()])
            .is_err()
    );
    assert!(replica.queue(another).is_err());
    assert_same(&before, &replica);
    replica.queue_recovered(large).unwrap();
    assert_same(&before, &replica);
}
