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
