//! Synthetic saved-Result transport/storage contracts, not provider or pixel-proof execution.
#![allow(clippy::unwrap_used)]
use std::io::{Cursor, Seek, SeekFrom};
use vw_model::{AssetId, DeviceId, Document, Id, Project};
use vw_net::{BLOB_CHUNK_BYTES, BlobReceiver, BlobSender, HostSync, SyncReceiver};
use vw_ops::{HostSequencer, Replica};
use vw_proto::{Message, v1 as pb};
use vw_store::{BlobStore, ProjectStore};

fn id(n: u64) -> Id {
    Id::from_parts(1_700_000_000_000 + n, [0x71; 10]).unwrap()
}
fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
fn project() -> Project {
    let mut project = Project::new(id(1), "phone Result sync fixture".into(), device(1));
    project.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(2).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "source preserved".into(),
                ..Default::default()
            },
            pages: vec![],
            created_at_ms: 0,
        },
    );
    project
}
fn result_transaction(base: pb::Revision, payloads: &[Vec<u8>]) -> pb::Transaction {
    let assets: Vec<_> = payloads.iter().map(|bytes| AssetId::hash(bytes)).collect();
    let mut kinds: Vec<_> = payloads
        .iter()
        .zip(&assets)
        .map(|(bytes, asset)| {
            pb::op::Kind::AddAsset(pb::AddAsset {
                asset_id: asset.to_string(),
                format: "png".into(),
                width: 16,
                height: 16,
                orientation: 1,
                bit_depth: 8,
                color_space: "sRGB".into(),
                byte_size: bytes.len() as u64,
                source: "ai_result".into(),
                metadata_json: "{}".into(),
                ..Default::default()
            })
        })
        .collect();
    kinds.push(pb::op::Kind::AddResult(pb::AddResult {
        result_id: Some(id(10).to_proto()),
        document_id: Some(id(2).to_proto()),
        provider: "offline_mock".into(),
        model: "transport-fixture".into(),
        request_json: "{\"origin\":\"synthetic_transport_fixture\"}".into(),
        output_asset_id: assets[0].to_string(),
        composite_asset_id: assets[1].to_string(),
        // Minimal model-valid fixture metadata. This test does not establish a pixel proof.
        proof_json: "{\"changed_outside\":0}".into(),
        metrics_json: "{}".into(),
        ..Default::default()
    }));
    kinds.push(pb::op::Kind::UpdateResult(pb::UpdateResult {
        result_id: Some(id(10).to_proto()),
        status: "partial".into(),
        acceptance_mask_asset_id: assets[2].to_string(),
    }));
    pb::Transaction {
        txn_id: Some(id(20).to_proto()),
        project_id: Some(id(1).to_proto()),
        device_id: device(2).to_string(),
        base_revision: Some(base),
        created_at_wall_ms: 20,
        ops: kinds
            .into_iter()
            .enumerate()
            .map(|(index, kind)| pb::Op {
                op_id: Some(pb::OpId {
                    device_id: device(2).to_string(),
                    lamport: 100 + index as u64,
                }),
                kind: Some(kind),
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn interrupted_phone_result_assets_reopen_and_sync_once_with_exact_journal_and_mask() {
    let temp = tempfile::tempdir().unwrap();
    let host_path = temp.path().join("host");
    let phone_path = temp.path().join("phone");
    let mut host = ProjectStore::create(&host_path, project(), device(1), 0).unwrap();
    let host_blobs = BlobStore::new(&host_path).unwrap();
    let initial = host.checkpoint_bytes().unwrap();
    let mut phone = ProjectStore::create_authenticated_checkpoint(
        &phone_path,
        &initial,
        &device(1),
        &device(2),
        1,
    )
    .unwrap();
    let phone_blobs = BlobStore::new(&phone_path).unwrap();
    // Opaque fake bytes exercise transfer, not an image decoder or external AI.
    let payloads = vec![vec![0x22; 150_000], vec![0x33; 130_000], vec![0xff; 256]];
    let txn = result_transaction(phone.revision().unwrap(), &payloads);
    let original = txn.encode_to_vec();
    for bytes in &payloads {
        phone_blobs.put(bytes).unwrap();
    }
    phone.enqueue_pending(&txn).unwrap();
    drop(phone);
    let mut phone = ProjectStore::open(&phone_path).unwrap();
    assert_eq!(phone.pending().unwrap()[0].encode_to_vec(), original);
    let mut replica = Replica::new(
        device(2),
        HostSequencer::from_checkpoint_bytes(&phone.checkpoint_bytes().unwrap())
            .unwrap()
            .snapshot()
            .unwrap(),
    )
    .unwrap();
    replica
        .queue_recovered_batch(phone.pending().unwrap())
        .unwrap();
    assert_eq!(replica.project().results[&id(10)].status, "partial");
    let before = host.revision().unwrap();
    for (index, bytes) in payloads.iter().enumerate() {
        let asset = AssetId::hash(bytes);
        let staging = host.transfer_file().unwrap();
        let mut sender = BlobSender::resume(
            Cursor::new(bytes.clone()),
            asset.clone(),
            bytes.len() as u64,
            0,
        )
        .unwrap();
        let mut receiver = BlobReceiver::resume(
            staging.reopen().unwrap(),
            asset.clone(),
            bytes.len() as u64,
            0,
            1_000_000,
        )
        .unwrap();
        let first = sender.next_chunk().unwrap().unwrap();
        let first_ack = receiver.receive(&first).unwrap();
        if index == 0 {
            assert_eq!(first_ack.next_offset, BLOB_CHUNK_BYTES as u64);
            assert!(!first_ack.verified);
            assert!(receiver.finish().is_err());
            // Neither a partial sink nor an interruption accepts or removes the edit.
            assert_eq!(host.revision().unwrap(), before);
            assert_eq!(phone.pending().unwrap()[0].encode_to_vec(), original);
            assert!(!host_blobs.path(&asset).unwrap().exists());
            receiver = BlobReceiver::resume(
                staging.reopen().unwrap(),
                asset.clone(),
                bytes.len() as u64,
                first_ack.next_offset,
                1_000_000,
            )
            .unwrap();
            sender = BlobSender::resume(
                Cursor::new(bytes.clone()),
                asset.clone(),
                bytes.len() as u64,
                first_ack.next_offset,
            )
            .unwrap();
            assert_eq!(receiver.receive(&first).unwrap(), first_ack);
            let mut damaged_duplicate = first.clone();
            damaged_duplicate.data[0] ^= 1;
            assert!(receiver.receive(&damaged_duplicate).is_err());
            assert_eq!(receiver.offset(), first_ack.next_offset);
        } else {
            sender.acknowledge(&first_ack).unwrap();
        }
        while let Some(chunk) = sender.next_chunk().unwrap() {
            let ack = receiver.receive(&chunk).unwrap();
            sender.acknowledge(&ack).unwrap();
        }
        assert!(sender.is_verified());
        let mut file = receiver.finish().unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        assert_eq!(host_blobs.put_reader(&mut file).unwrap(), asset);
        assert_eq!(host_blobs.read(&asset).unwrap(), *bytes);
    }
    let accepted = host.commit(&txn, &device(2), 30).unwrap();
    let retry = host.commit(&txn, &device(2), 31).unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.ack, accepted.ack);
    // A delivered receipt alone must not delete the only local durable copy.
    assert!(phone.acknowledge_pending(&txn, &accepted.ack).is_err());
    assert_eq!(phone.pending().unwrap().len(), 1);
    let host_sync = HostSync::from_store(&host).unwrap();
    let mut receiver = SyncReceiver::new(HostSequencer::from_checkpoint_bytes(&initial).unwrap());
    let response = host_sync.respond(&receiver.request().unwrap()).unwrap();
    assert!(response.checkpoint.is_none());
    let snapshot = receiver
        .receive(response.batch.clone(), &device(1))
        .unwrap()
        .unwrap();
    // Losing delivery before persistence keeps the original queued operation on reopen.
    drop(phone);
    let mut phone = ProjectStore::open(&phone_path).unwrap();
    assert_eq!(phone.pending().unwrap()[0].encode_to_vec(), original);
    assert_eq!(receiver.persist(&mut phone, 40).unwrap(), 1);
    assert_eq!(receiver.persist(&mut phone, 41).unwrap(), 0);
    replica.receive(snapshot).unwrap();
    assert!(replica.pending().is_empty());
    let retained = phone.project().results[&id(10)].clone();
    assert_eq!(retained.status, "partial");
    assert_eq!(
        retained.acceptance_mask_asset_id,
        Some(AssetId::hash(&payloads[2]))
    );
    assert_eq!(phone.project(), host.project());
    let current = receiver.host().revision().unwrap();
    assert!(receiver.receive(response.batch, &device(1)).is_err());
    assert_eq!(receiver.host().revision().unwrap(), current);
    drop(phone);
    let phone = ProjectStore::open(&phone_path).unwrap();
    assert!(phone.pending().unwrap().is_empty());
    assert_eq!(phone.project().results[&id(10)], retained);
    for bytes in &payloads {
        let asset = AssetId::hash(bytes);
        assert_eq!(phone_blobs.read(&asset).unwrap(), *bytes);
        assert_eq!(host_blobs.read(&asset).unwrap(), *bytes);
    }
    assert_eq!(phone.accepted_transactions().count(), 1);
    assert_eq!(host.accepted_transactions().count(), 1);
    phone.integrity_check().unwrap();
    host.integrity_check().unwrap();
}

#[test]
fn wrong_host_or_divergent_saved_result_checkpoint_never_acknowledges_phone_work() {
    let temp = tempfile::tempdir().unwrap();
    let mut host =
        ProjectStore::create(&temp.path().join("host"), project(), device(1), 0).unwrap();
    let initial = host.checkpoint_bytes().unwrap();
    let mut phone = ProjectStore::create_authenticated_checkpoint(
        &temp.path().join("phone"),
        &initial,
        &device(1),
        &device(2),
        1,
    )
    .unwrap();
    let payloads = vec![vec![1; 8], vec![2; 8], vec![3; 8]];
    let txn = result_transaction(phone.revision().unwrap(), &payloads);
    phone.enqueue_pending(&txn).unwrap();
    host.commit(&txn, &device(2), 30).unwrap();
    let response = HostSync::from_store(&host)
        .unwrap()
        .respond(
            &SyncReceiver::new(HostSequencer::from_checkpoint_bytes(&initial).unwrap())
                .request()
                .unwrap(),
        )
        .unwrap();
    let mut receiver = SyncReceiver::new(HostSequencer::from_checkpoint_bytes(&initial).unwrap());
    assert!(
        receiver
            .receive(response.batch.clone(), &device(3))
            .is_err()
    );
    assert_eq!(phone.pending().unwrap(), vec![txn.clone()]);
    let mut altered = response.batch;
    altered.txns[0].txn.as_mut().unwrap().created_at_wall_ms += 1;
    assert!(receiver.receive(altered, &device(1)).is_err());
    assert_eq!(phone.pending().unwrap(), vec![txn]);
    assert_eq!(phone.revision().unwrap().host_seq, 0);
}
