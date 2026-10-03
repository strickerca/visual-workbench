#![allow(clippy::unwrap_used)]
use vw_model::{DeviceId, Id, Project};
use vw_ops::HostSequencer;
use vw_proto::v1 as pb;
use vw_store::ProjectStore;
fn id(n: u8) -> Id {
    Id::from_parts(1_700_000_000_000, [n; 10]).unwrap()
}
fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
fn transaction(base: pb::Revision, n: u8, author: u8, kind: pb::op::Kind) -> pb::Transaction {
    pb::Transaction {
        txn_id: Some(id(n).to_proto()),
        project_id: Some(id(1).to_proto()),
        device_id: device(author).to_string(),
        base_revision: Some(base),
        created_at_wall_ms: i64::from(n),
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: device(author).to_string(),
                lamport: u64::from(n),
            }),
            kind: Some(kind),
        }],
        ..Default::default()
    }
}
#[test]
fn authenticated_bootstrap_preserves_exact_history_and_local_role_after_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let mut host = HostSequencer::new(
        Project::new(id(1), "bootstrap".into(), device(1)),
        device(1),
    )
    .unwrap();
    let txn = transaction(
        host.revision().unwrap(),
        10,
        1,
        pb::op::Kind::CreateDocument(pb::CreateDocument {
            document_id: Some(id(2).to_proto()),
            kind: pb::DocumentKind::Image as i32,
            schema_version: 1,
            title: "source".into(),
            ..Default::default()
        }),
    );
    let accepted = host.submit(txn.clone(), &device(1), 123).unwrap();
    let bytes = host.checkpoint_bytes().unwrap();
    let path = temp.path().join("mirror");
    let mut store =
        ProjectStore::create_authenticated_checkpoint(&path, &bytes, &device(1), &device(2), 200)
            .unwrap();
    assert_eq!(store.checkpoint_bytes().unwrap(), bytes);
    assert_eq!(store.local_device().unwrap(), device(2));
    assert_eq!(
        store.accepted_transactions().next().unwrap(),
        (&txn, &accepted.ack)
    );
    let pending = transaction(
        store.revision().unwrap(),
        11,
        2,
        pb::op::Kind::UpdateDocument(pb::UpdateDocument {
            document_id: Some(id(2).to_proto()),
            title: "offline".into(),
        }),
    );
    store.enqueue_pending(&pending).unwrap();
    drop(store);
    let reopened = ProjectStore::open(&path).unwrap();
    assert_eq!(reopened.local_device().unwrap(), device(2));
    assert_eq!(reopened.host_device(), &device(1));
    assert_eq!(reopened.pending().unwrap(), vec![pending]);
    assert_eq!(reopened.checkpoint_bytes().unwrap(), bytes);
    reopened.integrity_check().unwrap();
    assert!(
        ProjectStore::create_authenticated_checkpoint(&path, &bytes, &device(1), &device(2), 200)
            .is_err()
    );
    assert_eq!(reopened.checkpoint_bytes().unwrap(), bytes);
}
#[test]
fn forged_peer_and_corrupt_checkpoints_never_create_a_candidate_directory() {
    let temp = tempfile::tempdir().unwrap();
    let host = HostSequencer::new(
        Project::new(id(1), "bootstrap".into(), device(1)),
        device(1),
    )
    .unwrap();
    let bytes = host.checkpoint_bytes().unwrap();
    let wrong = temp.path().join("wrong-peer");
    assert!(
        ProjectStore::create_authenticated_checkpoint(&wrong, &bytes, &device(3), &device(2), 0)
            .is_err()
    );
    assert!(!wrong.exists());
    let corrupt = temp.path().join("corrupt");
    let mut bad = bytes;
    bad[0] ^= 1;
    assert!(
        ProjectStore::create_authenticated_checkpoint(&corrupt, &bad, &device(1), &device(2), 0)
            .is_err()
    );
    assert!(!corrupt.exists());
}

#[test]
fn bootstrap_keeps_undone_original_inventory_and_transfer_files_are_project_private() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("mirror");
    let original = b"retained immutable original";
    let asset = vw_model::AssetId::hash(original);
    let definition = pb::AddAsset {
        asset_id: asset.to_string(),
        format: "png".into(),
        width: 1,
        height: 1,
        orientation: 1,
        bit_depth: 8,
        color_space: "sRGB".into(),
        byte_size: original.len() as u64,
        source: "import".into(),
        metadata_json: "{}".into(),
        ..Default::default()
    };
    let mut host =
        HostSequencer::new(Project::new(id(1), "retained".into(), device(1)), device(1)).unwrap();
    host.submit(
        transaction(
            host.revision().unwrap(),
            10,
            1,
            pb::op::Kind::AddAsset(definition),
        ),
        &device(1),
        100,
    )
    .unwrap();
    host.submit(
        transaction(
            host.revision().unwrap(),
            11,
            1,
            pb::op::Kind::UndoTransaction(pb::UndoTransaction {
                target_txn_id: Some(id(10).to_proto()),
            }),
        ),
        &device(1),
        101,
    )
    .unwrap();
    assert!(host.project().assets.is_empty());
    let mut store = ProjectStore::create_authenticated_checkpoint(
        &path,
        &host.checkpoint_bytes().unwrap(),
        &device(1),
        &device(2),
        200,
    )
    .unwrap();
    assert_eq!(
        store.retained_assets().unwrap().get(&asset),
        Some(&(original.len() as u64))
    );
    let blobs = vw_store::BlobStore::new(&path).unwrap();
    assert_eq!(blobs.put(original).unwrap(), asset);
    let scratch_path = {
        let mut scratch = store.transfer_file().unwrap();
        use std::io::Write;
        scratch.write_all(b"partial transfer").unwrap();
        let owned = scratch.path().to_path_buf();
        assert!(
            owned.starts_with(
                path.canonicalize()
                    .unwrap()
                    .join("cache/session-transfers-v1")
            )
        );
        assert!(owned.is_file());
        owned
    };
    assert!(!scratch_path.exists());
    assert_eq!(blobs.read(&asset).unwrap(), original);
    let redo = transaction(
        store.revision().unwrap(),
        12,
        1,
        pb::op::Kind::UndoTransaction(pb::UndoTransaction {
            target_txn_id: Some(id(11).to_proto()),
        }),
    );
    store.commit(&redo, &device(1), 201).unwrap();
    assert!(store.project().assets.contains_key(&asset));
    drop(store);
    let reopened = ProjectStore::open(&path).unwrap();
    assert_eq!(
        reopened.retained_assets().unwrap().get(&asset),
        Some(&(original.len() as u64))
    );
    assert_eq!(blobs.read(&asset).unwrap(), original);
}
