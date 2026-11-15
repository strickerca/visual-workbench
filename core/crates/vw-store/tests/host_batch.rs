//! A mirror takes an ordinary sync update into its store step by step: it
//! ends exactly where installing a checkpoint of the host would, and a batch
//! the host could not have sent changes nothing.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use vw_model::{DeviceId, Document, Id, Layer, Project};
use vw_ops::HostSequencer;
use vw_proto::v1::{self as pb, op::Kind};
use vw_store::{ProjectStore, StoreError};

const CLOCK: u64 = 1_700_000_000_000;
fn id(n: u64) -> Id {
    Id::from_parts(CLOCK + n, [0x75; 10]).unwrap()
}
fn uid(n: u64) -> Option<pb::Uuid> {
    Some(id(n).to_proto())
}
fn pc() -> DeviceId {
    DeviceId::from_bytes([1; 16])
}
fn phone() -> DeviceId {
    DeviceId::from_bytes([2; 16])
}
fn project() -> Project {
    let mut value = Project::new(id(1), "Host batch fixture".into(), pc());
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
                name: "Ink".into(),
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
fn shape(n: u64, author: &DeviceId) -> Kind {
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
                ..Default::default()
            }),
            style: Some(pb::Style {
                stroke: Some(pb::Color { rgba: 0x112233ff }),
                width: 1.0,
                ..Default::default()
            }),
            role: pb::Role::None as i32,
            created_by: author.to_string(),
            created_at_ms: CLOCK as i64,
            shape: Some(pb::object_state::Shape::Rect(pb::RectD {
                x: 0.0,
                y: 0.0,
                w: 4.0,
                h: 3.0,
            })),
            ..Default::default()
        }),
    })
}
fn transaction(base: pb::Revision, author: &DeviceId, n: u64, kinds: Vec<Kind>) -> pb::Transaction {
    pb::Transaction {
        txn_id: uid(100_000 + n),
        project_id: uid(1),
        device_id: author.to_string(),
        base_revision: Some(base),
        created_at_wall_ms: 1000 + n as i64,
        ops: kinds
            .into_iter()
            .enumerate()
            .map(|(index, kind)| pb::Op {
                op_id: Some(pb::OpId {
                    device_id: author.to_string(),
                    lamport: n * 64 + index as u64 + 1,
                }),
                kind: Some(kind),
            })
            .collect(),
        gesture_id: None,
    }
}
/// The batch the host sends for everything after `base`.
fn batch(host: &HostSequencer, base: &pb::Revision) -> pb::SyncBatch {
    let mut entries = host
        .accepted_transactions()
        .filter(|(_, ack)| ack.host_seq > base.host_seq)
        .map(|(txn, ack)| pb::AckedTransaction {
            host_seq: ack.host_seq,
            state_hash: ack.state_hash.clone(),
            txn: Some(txn.clone()),
            ack: Some(ack.clone()),
            accepted_at_ms: host
                .accepted_at(&Id::from_proto(txn.txn_id.as_ref()).unwrap())
                .unwrap(),
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.host_seq);
    pb::SyncBatch {
        txns: entries,
        complete: true,
        project_id: uid(1),
        base: Some(base.clone()),
        revision: Some(host.revision().unwrap()),
        ..Default::default()
    }
}

#[test]
fn an_update_taken_in_equals_installing_the_host_and_a_bad_one_changes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let mut host = HostSequencer::new(project(), pc()).unwrap();
    for n in 0..4 {
        let author = if n % 2 == 0 { pc() } else { phone() };
        let txn = transaction(
            host.revision().unwrap(),
            &author,
            n,
            vec![shape(n, &author)],
        );
        host.submit(txn, &author, 2000 + n as i64).unwrap();
    }
    let path = temp.path().join("phone");
    let mut mirror = ProjectStore::create_authenticated_checkpoint(
        &path,
        &host.checkpoint_bytes().unwrap(),
        &pc(),
        &phone(),
        1,
    )
    .unwrap();
    // The phone queues two edits; the host accepts one of them and edits too.
    let base = mirror.revision().unwrap();
    let own = transaction(base.clone(), &phone(), 10, vec![shape(10, &phone())]);
    let later = transaction(base.clone(), &phone(), 11, vec![shape(11, &phone())]);
    mirror.enqueue_pending(&own).unwrap();
    mirror.enqueue_pending(&later).unwrap();
    host.submit(own.clone(), &phone(), 3000).unwrap();
    let edit = transaction(
        host.revision().unwrap(),
        &pc(),
        12,
        vec![
            shape(12, &pc()),
            Kind::UpdateLayer(pb::UpdateLayer {
                layer_id: uid(3),
                name: Some("Renamed".into()),
                ..Default::default()
            }),
        ],
    );
    host.submit(edit, &pc(), 3001).unwrap();
    let update = batch(&host, &base);

    // Batches the host could not have sent are refused, and nothing changes.
    let before = mirror.checkpoint_bytes().unwrap();
    let refused = |mirror: &mut ProjectStore, batch: pb::SyncBatch, peer: &DeviceId| {
        let result = mirror.receive_host_batch(&batch, peer, 5000);
        assert!(result.is_err());
        result.err()
    };
    refused(&mut mirror, update.clone(), &phone());
    let mut wrong = update.clone();
    wrong.base = Some(host.revision().unwrap());
    refused(&mut mirror, wrong, &pc());
    let mut wrong = update.clone();
    wrong.txns.remove(0);
    refused(&mut mirror, wrong, &pc());
    let mut wrong = update.clone();
    wrong.complete = false;
    refused(&mut mirror, wrong, &pc());
    let mut wrong = update.clone();
    wrong.txns[1].state_hash[0] ^= 1;
    refused(&mut mirror, wrong, &pc());
    let mut wrong = update.clone();
    wrong.checkpoint_size = 10;
    refused(&mut mirror, wrong, &pc());
    // Consistently changed, the hash is not what this engine decides: the
    // first entry is taken and then put back with everything else.
    let mut wrong = update.clone();
    wrong.txns[1].state_hash[0] ^= 1;
    wrong.txns[1].ack.as_mut().unwrap().state_hash[0] ^= 1;
    assert!(matches!(
        refused(&mut mirror, wrong, &pc()),
        Some(StoreError::EngineMismatch)
    ));
    assert!(mirror.checkpoint_bytes().unwrap() == before);
    assert_eq!(mirror.pending().unwrap(), vec![own.clone(), later.clone()]);

    // The real one: the store holds exactly what the host holds, the
    // acknowledged edit leaves the outbox and the other waits.
    let received = mirror.receive_host_batch(&update, &pc(), 5000).unwrap();
    assert_eq!(received.removed, 1);
    assert!(received.touched.objects.contains(&id(1010)));
    assert!(received.touched.layers.contains(&id(3)));
    assert!(mirror.checkpoint_bytes().unwrap() == host.checkpoint_bytes().unwrap());
    assert_eq!(mirror.pending().unwrap(), vec![later]);
    // The same batch again is refused: it no longer starts at the mirror.
    refused(&mut mirror, update, &pc());
    assert!(mirror.checkpoint_bytes().unwrap() == host.checkpoint_bytes().unwrap());
    // Reopened, every stored row equals a full rebuild and the log is the
    // host's, decided again.
    let state = host.checkpoint_bytes().unwrap();
    drop(mirror);
    let mirror = ProjectStore::open(&path).unwrap();
    assert!(mirror.checkpoint_bytes().unwrap() == state);
    mirror.integrity_check().unwrap();
}
