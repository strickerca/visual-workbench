//! Register in session.rs after the coordinator adopts the batch startup hook.
//! These are synthetic real-worker/store/replication tests, with no live provider or carrier.
#![allow(clippy::unwrap_used)]
use super::replication as sync;
use crate::{Cancellation, EditOptions, ProjectSession, project::SessionPermit};
use std::{io::Write, path::Path, sync::Arc};
use vw_model::{AssetId, DeviceId, Document, Id, Project};
use vw_proto::{Message, v1 as pb};
use vw_store::{BlobStore, ProjectStore};

fn id(n: u64) -> Id {
    Id::from_parts(1_700_000_000_000 + n, [0x72; 10]).unwrap()
}
fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
fn project() -> Project {
    let mut project = Project::new(id(1), "offline session fixture".into(), device(1));
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
    project
}
fn title(base: &pb::Revision, n: u64) -> pb::Transaction {
    pb::Transaction {
        txn_id: Some(id(n).to_proto()),
        project_id: Some(id(1).to_proto()),
        device_id: device(2).to_string(),
        base_revision: Some(base.clone()),
        created_at_wall_ms: n as i64,
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: device(2).to_string(),
                lamport: n,
            }),
            kind: Some(pb::op::Kind::UpdateDocument(pb::UpdateDocument {
                document_id: Some(id(2).to_proto()),
                title: format!("offline {n}"),
            })),
        }],
        ..Default::default()
    }
}
fn open(path: &Path) -> Arc<ProjectSession> {
    ProjectSession::from_store(
        ProjectStore::open(path).unwrap(),
        BlobStore::new(path).unwrap(),
        SessionPermit::acquire().unwrap(),
    )
    .unwrap()
}
fn mirror(path: &Path, checkpoint: &[u8]) -> ProjectStore {
    ProjectStore::create_authenticated_checkpoint(path, checkpoint, &device(1), &device(2), 1)
        .unwrap()
}

#[tokio::test]
async fn real_startup_restores_five_thousand_exact_rows_and_counter_floor_after_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let host = ProjectStore::create(&temp.path().join("host"), project(), device(1), 0).unwrap();
    let path = temp.path().join("phone");
    let mut phone = mirror(&path, &host.checkpoint_bytes().unwrap());
    let base = phone.revision().unwrap();
    let transactions: Vec<_> = (100..5100).map(|n| title(&base, n)).collect();
    for txn in &transactions {
        phone.enqueue_pending(txn).unwrap();
    }
    drop(phone);
    let session = open(&path);
    let info = session.info().await.unwrap();
    assert_eq!(info.next_lamport, 5100);
    assert!(info.can_undo);
    assert_eq!(sync::state(&session).await.unwrap().pending, 5000);
    assert_eq!(sync::state(&session).await.unwrap().blocked, 0);
    assert_eq!(sync::pending(&session).await.unwrap(), transactions);
    session
        .worker
        .call(|state| {
            assert_eq!(
                state.project()?.documents[&id(2)].definition.title,
                "offline 5099"
            );
            assert_eq!(state.history.next_lamport, 5100);
            Ok(())
        })
        .await
        .unwrap();
    session.close().await.unwrap();
    let reopened = open(&path);
    let after = reopened.info().await.unwrap();
    assert_eq!(after.state_hash, info.state_hash);
    assert_eq!(after.next_lamport, info.next_lamport);
    assert_eq!(sync::pending(&reopened).await.unwrap(), transactions);
    reopened.close().await.unwrap();
    ProjectStore::open(&path)
        .unwrap()
        .integrity_check()
        .unwrap();
}

#[tokio::test]
async fn recovered_pending_inverse_chain_supports_ordinary_undo_without_counter_reuse() {
    let temp = tempfile::tempdir().unwrap();
    let host = ProjectStore::create(&temp.path().join("host"), project(), device(1), 0).unwrap();
    let path = temp.path().join("phone");
    let mut phone = mirror(&path, &host.checkpoint_bytes().unwrap());
    let base = phone.revision().unwrap();
    let change = title(&base, 100);
    let mut undo = title(&base, 101);
    undo.ops[0].kind = Some(pb::op::Kind::UndoTransaction(pb::UndoTransaction {
        target_txn_id: Some(id(100).to_proto()),
    }));
    let mut redo = title(&base, 102);
    redo.ops[0].kind = Some(pb::op::Kind::UndoTransaction(pb::UndoTransaction {
        target_txn_id: Some(id(101).to_proto()),
    }));
    for txn in [&change, &undo, &redo] {
        phone.enqueue_pending(txn).unwrap();
    }
    drop(phone);
    let session = open(&path);
    let info = session.info().await.unwrap();
    assert_eq!(info.next_lamport, 103);
    assert!(info.can_undo);
    let result = session
        .undo_redo(
            EditOptions {
                transaction_id: id(103).to_string(),
                document_id: id(2).to_string(),
                device_id: device(2).to_string(),
                lamport: info.next_lamport,
                created_at_ms: 103,
            },
            false,
            Arc::new(Cancellation::new()),
        )
        .await
        .unwrap();
    assert_eq!(result.next_lamport, 104);
    session
        .worker
        .call(|state| {
            assert_eq!(
                state.project()?.documents[&id(2)].definition.title,
                "initial"
            );
            let pending = state.store()?.pending()?;
            assert_eq!(pending.len(), 4);
            assert_eq!(pending[3].ops[0].op_id.as_ref().unwrap().lamport, 103);
            Ok(())
        })
        .await
        .unwrap();
    session.close().await.unwrap();
    let reopened = open(&path);
    assert_eq!(reopened.info().await.unwrap().next_lamport, 104);
    assert!(reopened.info().await.unwrap().can_redo);
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn saved_phone_result_uses_real_transaction_bound_asset_and_acknowledgement_callers() {
    let temp = tempfile::tempdir().unwrap();
    let host_path = temp.path().join("host");
    let store = ProjectStore::create(&host_path, project(), device(1), 0).unwrap();
    let checkpoint = store.checkpoint_bytes().unwrap();
    let host = ProjectSession::from_store(
        store,
        BlobStore::new(&host_path).unwrap(),
        SessionPermit::acquire().unwrap(),
    )
    .unwrap();
    let phone_path = temp.path().join("phone");
    let store = mirror(&phone_path, &checkpoint);
    let base = store.revision().unwrap();
    drop(store);
    let phone = open(&phone_path);
    let raw = b"synthetic saved Result output".to_vec();
    let composite = b"synthetic saved Result composite".to_vec();
    let mut txn = title(&base, 100);
    let mut kinds: Vec<_> = [&raw, &composite]
        .into_iter()
        .map(|bytes| {
            pb::op::Kind::AddAsset(pb::AddAsset {
                asset_id: AssetId::hash(bytes).to_string(),
                format: "png".into(),
                width: 1,
                height: 1,
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
        result_id: Some(id(20).to_proto()),
        document_id: Some(id(2).to_proto()),
        provider: "offline_mock".into(),
        model: "transport-fixture".into(),
        request_json: "{}".into(),
        output_asset_id: AssetId::hash(&raw).to_string(),
        composite_asset_id: AssetId::hash(&composite).to_string(),
        proof_json: "{\"changed_outside\":0}".into(),
        metrics_json: "{}".into(),
        ..Default::default()
    }));
    kinds.push(pb::op::Kind::UpdateResult(pb::UpdateResult {
        result_id: Some(id(20).to_proto()),
        status: "ready".into(),
        acceptance_mask_asset_id: String::new(),
    }));
    txn.ops = kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| pb::Op {
            op_id: Some(pb::OpId {
                device_id: device(2).to_string(),
                lamport: 100 + index as u64,
            }),
            kind: Some(kind),
        })
        .collect();
    let saved = txn.clone();
    let data = vec![raw.clone(), composite.clone()];
    phone
        .worker
        .call(move |state| {
            for bytes in data {
                state.blobs.put(&bytes)?;
            }
            state.commit(&saved, &device(2), 100)?;
            Ok(())
        })
        .await
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_millis(100),
        phone.network_changed.notified(),
    )
    .await
    .unwrap();
    assert_eq!(
        sync::pending(&phone).await.unwrap()[0].encode_to_vec(),
        txn.encode_to_vec()
    );
    assert!(
        sync::transaction_missing(&host, txn.clone(), device(3))
            .await
            .is_err()
    );
    assert_eq!(
        sync::transaction_missing(&host, txn.clone(), device(2))
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(sync::submit(&host, txn.clone(), device(2)).await.is_err());
    assert_eq!(host.info().await.unwrap().host_seq, 0);
    assert_eq!(sync::pending(&phone).await.unwrap(), vec![txn.clone()]);
    for bytes in [raw.clone(), composite.clone()] {
        let asset = AssetId::hash(&bytes);
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&bytes).unwrap();
        sync::install_transaction_asset(
            &host,
            txn.clone(),
            device(2),
            asset,
            file,
            bytes.len() as u64,
        )
        .await
        .unwrap();
    }
    assert!(
        sync::transaction_missing(&host, txn.clone(), device(2))
            .await
            .unwrap()
            .is_empty()
    );
    let accepted = sync::submit(&host, txn.clone(), device(2)).await.unwrap();
    let duplicate = sync::submit(&host, txn.clone(), device(2)).await.unwrap();
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.ack, accepted.ack);
    assert_eq!(sync::pending(&phone).await.unwrap().len(), 1);
    let response = sync::respond(&host, sync::request(&phone).await.unwrap())
        .await
        .unwrap();
    assert!(response.checkpoint.is_none());
    assert!(
        sync::receive(&phone, response.batch.clone(), device(3))
            .await
            .is_err()
    );
    assert_eq!(sync::pending(&phone).await.unwrap().len(), 1);
    assert!(
        sync::receive(&phone, response.batch.clone(), device(1))
            .await
            .unwrap()
    );
    assert!(sync::pending(&phone).await.unwrap().is_empty());
    assert!(
        sync::receive(&phone, response.batch, device(1))
            .await
            .is_err()
    );
    let info = phone.info().await.unwrap();
    assert_eq!(info.state_hash, host.info().await.unwrap().state_hash);
    assert_eq!(info.next_lamport, 104);
    assert!(info.can_undo);
    phone.close().await.unwrap();
    let reopened = open(&phone_path);
    assert_eq!(reopened.info().await.unwrap().state_hash, info.state_hash);
    assert_eq!(reopened.info().await.unwrap().next_lamport, 104);
    reopened
        .worker
        .call(move |state| {
            assert_eq!(state.store()?.accepted_transactions().count(), 1);
            assert_eq!(state.project()?.results[&id(20)].status, "ready");
            assert_eq!(state.blobs.read(&AssetId::hash(&raw))?, raw);
            assert_eq!(state.blobs.read(&AssetId::hash(&composite))?, composite);
            Ok(())
        })
        .await
        .unwrap();
    reopened.close().await.unwrap();
    host.close().await.unwrap();
}
