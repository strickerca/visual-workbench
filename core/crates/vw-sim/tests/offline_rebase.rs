//! Scale/reconnect regressions; executed only in the coordinator's final batch.
#![allow(clippy::unwrap_used)]
use vw_model::{DeviceId, Document, Id, Project};
use vw_net::{HostSync, SyncReceiver};
use vw_ops::{HostSequencer, Replica};
use vw_proto::{Message, v1 as pb};

fn id(n: u64) -> Id {
    Id::from_parts(1_700_000_000_000 + n, [0x63; 10]).unwrap()
}
fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
fn fixture() -> HostSequencer {
    let mut project = Project::new(id(1), "scale recovery".into(), device(1));
    for document in [2, 3] {
        project.documents.insert(
            id(document),
            Document {
                definition: pb::CreateDocument {
                    document_id: Some(id(document).to_proto()),
                    kind: pb::DocumentKind::Image as i32,
                    schema_version: 1,
                    title: "initial".into(),
                    ..Default::default()
                },
                pages: vec![],
                created_at_ms: 0,
            },
        );
    }
    HostSequencer::new(project, device(1)).unwrap()
}
fn transaction(base: &pb::Revision, n: u64, who: u8, document: u64) -> pb::Transaction {
    pb::Transaction {
        txn_id: Some(id(n).to_proto()),
        project_id: Some(id(1).to_proto()),
        device_id: device(who).to_string(),
        base_revision: Some(base.clone()),
        created_at_wall_ms: n as i64,
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: device(who).to_string(),
                lamport: n,
            }),
            kind: Some(pb::op::Kind::UpdateDocument(pb::UpdateDocument {
                document_id: Some(id(document).to_proto()),
                title: format!("edit {n}"),
            })),
        }],
        ..Default::default()
    }
}
fn catch_up(host: &HostSync, receiver: &mut SyncReceiver, replica: &mut Replica) {
    while receiver.host().revision().unwrap() != host.host().revision().unwrap() {
        let response = host.respond(&receiver.request().unwrap()).unwrap();
        let snapshot = if let Some(bytes) = response.checkpoint {
            assert!(
                receiver
                    .receive(response.batch, &device(1))
                    .unwrap()
                    .is_none()
            );
            receiver.install_checkpoint(&bytes, &device(1)).unwrap()
        } else {
            receiver
                .receive(response.batch, &device(1))
                .unwrap()
                .unwrap()
        };
        replica.receive(snapshot).unwrap();
    }
}

#[test]
fn five_thousand_exact_offline_transactions_rebase_across_interruption_and_lost_receipts() {
    let initial = fixture();
    let base = initial.revision().unwrap();
    let mut host = HostSync::new(initial.clone()).unwrap();
    let mut receiver = SyncReceiver::new(initial.clone());
    let mut replica = Replica::new(device(2), initial.snapshot().unwrap()).unwrap();
    let outbox: Vec<_> = (100..5100).map(|n| transaction(&base, n, 2, 2)).collect();
    let bytes: Vec<_> = outbox.iter().map(Message::encode_to_vec).collect();
    replica.queue_recovered_batch(outbox.clone()).unwrap();
    assert_eq!(replica.pending().len(), 5000);
    // An independent PC edit advances the authoritative base while the phone
    // retains its old transaction bases. Existing conflict cases cover same-field writes.
    host.submit(transaction(&base, 6000, 1, 3), &device(1), 6000)
        .unwrap();
    for (index, txn) in outbox.iter().enumerate() {
        let accepted = host
            .submit(txn.clone(), &device(2), 7000 + index as i64)
            .unwrap();
        assert!(!accepted.duplicate);
        if index.is_multiple_of(257) {
            // A lost receipt retries the original transaction, not a new ID/base.
            let duplicate = host
                .submit(txn.clone(), &device(2), 20_000 + index as i64)
                .unwrap();
            assert!(duplicate.duplicate);
            assert_eq!(duplicate.ack, accepted.ack);
        }
        if index == 2499 {
            catch_up(&host, &mut receiver, &mut replica);
            assert_eq!(replica.pending().len(), 2500);
            let remaining: Vec<_> = replica
                .pending()
                .iter()
                .map(|p| p.transaction.clone())
                .collect();
            // Process reconstruction uses the verified host and original outbox.
            replica = Replica::new(device(2), receiver.host().snapshot().unwrap()).unwrap();
            replica.queue_recovered_batch(remaining).unwrap();
            assert_eq!(
                replica
                    .pending()
                    .iter()
                    .map(|p| p.transaction.encode_to_vec())
                    .collect::<Vec<_>>(),
                bytes[2500..]
            );
        }
    }
    catch_up(&host, &mut receiver, &mut replica);
    assert!(replica.pending().is_empty());
    assert_eq!(host.host().revision().unwrap().host_seq, 5001);
    assert_eq!(
        replica.project().state_hash().unwrap(),
        host.host().project().state_hash().unwrap()
    );
    assert_eq!(receiver.host().project(), host.host().project());
    for (txn, original) in outbox.iter().zip(bytes) {
        let key = Id::from_proto(txn.txn_id.as_ref()).unwrap();
        let (accepted, _) = host.host().accepted_transaction(&key).unwrap();
        assert_eq!(accepted.encode_to_vec(), original);
    }
    // Stale delivery cannot replace the final base or resurrect acknowledged edits.
    let before = replica.project().clone();
    assert!(replica.receive(initial.snapshot().unwrap()).is_err());
    assert_eq!(replica.project(), &before);
    replica
        .receive(receiver.host().snapshot().unwrap())
        .unwrap();
    assert!(replica.pending().is_empty());
}

#[test]
fn existing_authenticated_fault_simulation_still_converges_after_batch_recovery_change() {
    for seed in [1, 7, 42] {
        let report = vw_sim::run(seed).unwrap();
        assert_eq!(report.accepted, 12);
        assert_eq!(report.reconnects, 2);
        assert_eq!(report.stale_input_discarded, 2);
    }
}
