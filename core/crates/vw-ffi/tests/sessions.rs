//! Synthetic loopback integration. Test trust lives only in fixture memory;
//! production entry points use DPAPI or the platform protection callback.
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use support::*;
use vw_core::*;
use vw_model::DeviceId;
use vw_net::pairing::TrustState;
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn temp() -> tempfile::TempDir {
    if cfg!(target_os = "android") {
        tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()
    } else {
        tempfile::tempdir().unwrap()
    }
}

#[derive(Clone, Default)]
struct FixtureTrust(Arc<Mutex<Option<Vec<u8>>>>);
impl ProtectedTrustCallback for FixtureTrust {
    fn load(&self) -> SessionResult<Option<Vec<u8>>> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn clear_loaded_plaintext(&self) {}
    fn compare_exchange(
        &self,
        expected: u64,
        replacement: u64,
        bytes: Vec<u8>,
    ) -> SessionResult<bool> {
        let decoded = TrustState::decode_unprotected(&bytes).map_err(|_| SessionError::Storage)?;
        if decoded.revision() != replacement || expected.checked_add(1) != Some(replacement) {
            return Err(SessionError::Storage);
        }
        let mut stored = self.0.lock().unwrap();
        let revision = stored
            .as_ref()
            .map(|value| TrustState::decode_unprotected(value).unwrap().revision())
            .unwrap_or(0);
        if revision != expected {
            return Ok(false);
        }
        *stored = Some(bytes);
        Ok(true)
    }
}
async fn service(id: String, fixture: FixtureTrust) -> Arc<SessionService> {
    open_callback_session_service(id, Box::new(fixture))
        .await
        .unwrap()
}
async fn paired() -> (Arc<SessionService>, Arc<SessionService>) {
    let host = service(device(), FixtureTrust::default()).await;
    let client = service(
        DeviceId::from_bytes([8; 16]).to_string(),
        FixtureTrust::default(),
    )
    .await;
    let listener = host.listen_pairing("127.0.0.1:0".into()).await.unwrap();
    let offer = listener.offer(false).await.unwrap();
    let details = client.inspect_qr(offer.qr.clone()).unwrap();
    assert_eq!(details.peer_device_id, device());
    assert_eq!(details.endpoints, vec![offer.endpoint.clone()]);
    assert_eq!(details.expires_at_ms, offer.expires_at_ms);
    assert!(client.inspect_qr(vec![0; 4097]).is_err());
    let (accepted, joined) = tokio::join!(
        listener.accept(cancel()),
        client.join_qr(offer.qr, offer.endpoint)
    );
    assert_eq!(
        accepted.unwrap().paired_device_id,
        Some(client.local_device().await.unwrap().device_id)
    );
    assert_eq!(joined.unwrap(), device());
    listener.close().await.unwrap();
    (host, client)
}
async fn converged(host: &LiveSession, client: &LiveSession) {
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let a = host.status();
            let b = client.status();
            if a.status == SyncStatus::Synced
                && b.status == SyncStatus::Synced
                && a.host_seq == b.host_seq
                && a.state_hash == b.state_hash
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    let a = host.status();
    let b = client.status();
    assert!(
        result.is_ok(),
        "convergence deadline: host={:?}/seq{}/pending{}/blocked{}/failure{:?}; client={:?}/seq{}/pending{}/blocked{}/failure{:?}; same_hash={}",
        a.status,
        a.host_seq,
        a.pending,
        a.blocked,
        a.failure,
        b.status,
        b.host_seq,
        b.pending,
        b.blocked,
        b.failure,
        a.state_hash == b.state_hash,
    );
}
fn rectangle(object: u8) -> EditCommand {
    EditCommand::Create {
        object_id: id(object),
        layer_id: id(3),
        shape: NewShape::Rectangle {
            rectangle: QueryRect {
                x: 5.0,
                y: 6.0,
                width: 12.0,
                height: 13.0,
            },
        },
        style: ObjectStyle {
            rgba: 0x123456ff,
            width: 2.0,
            screen_constant_width: false,
            fill: None,
        },
        transform: Transform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        },
    }
}

#[tokio::test]
async fn authenticated_bootstrap_offline_undo_reopen_and_reconnect_are_durable() {
    let _serial = SERIAL.lock().await;
    let temp = temp();
    let (host_service, client_service) = paired().await;
    let host = create_image_project(create(&temp.path().join("host")), cancel())
        .await
        .unwrap();
    draw(&host).await;
    let client_id = client_service.local_device().await.unwrap().device_id;
    let server = host_service
        .host_project(
            host.clone(),
            client_id.clone(),
            vec![SessionEndpoint {
                carrier: AppCarrier::TcpAdb,
                address: "127.0.0.1:0".into(),
            }],
        )
        .await
        .unwrap();
    let client_path = temp.path().join("client");
    let client = client_service
        .receive_project(
            ReceiveProjectOptions {
                path: client_path.to_string_lossy().into(),
                peer_device_id: device(),
                endpoints: server.endpoints(),
                expected_project_id: Some(id(1)),
                max_total_blob_bytes: 16 * 1024 * 1024,
            },
            cancel(),
        )
        .await
        .unwrap();
    assert_eq!(client.info().await.unwrap().device_id, client_id);
    assert_eq!(
        host.info().await.unwrap().state_hash,
        client.info().await.unwrap().state_hash
    );
    let link = client_service
        .connect_project(client.clone(), device(), server.endpoints())
        .await
        .unwrap();
    converged(&server, &link).await;
    link.close().await.unwrap();
    drop(link);
    let mut add = edit(20, client.info().await.unwrap().next_lamport);
    add.device_id = client_id.clone();
    let pending = client
        .apply_edit(add.clone(), vec![rectangle(24)], cancel())
        .await
        .unwrap();
    assert_eq!(pending.host_seq, 1);
    let mut undo = edit(21, pending.next_lamport);
    undo.device_id = client_id.clone();
    let undone = client.undo_redo(undo, false, cancel()).await.unwrap();
    let mut redo = edit(22, undone.next_lamport);
    redo.device_id = client_id.clone();
    let redone = client.undo_redo(redo, true, cancel()).await.unwrap();
    assert_eq!(redone.state_hash, pending.state_hash);
    client.close().await.unwrap();
    drop(client);
    let client = open_project(client_path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    assert_eq!(client.info().await.unwrap().state_hash, redone.state_hash);
    assert_eq!(
        client
            .apply_edit(add, vec![rectangle(24)], cancel())
            .await
            .unwrap()
            .state_hash,
        redone.state_hash
    );
    host.apply_edit(
        edit(30, host.info().await.unwrap().next_lamport),
        vec![rectangle(31)],
        cancel(),
    )
    .await
    .unwrap();
    let link = client_service
        .connect_project(client.clone(), device(), server.endpoints())
        .await
        .unwrap();
    converged(&server, &link).await;
    assert_eq!(client.info().await.unwrap().host_seq, 5);
    assert_eq!(
        client
            .document_snapshot(id(2), cancel())
            .await
            .unwrap()
            .render
            .items
            .len(),
        3
    );
    link.close().await.unwrap();
    server.close().await.unwrap();
    client.close().await.unwrap();
    host.close().await.unwrap();
    let persisted = vw_store::ProjectStore::open(&client_path).unwrap();
    assert!(persisted.pending().unwrap().is_empty());
    assert_eq!(persisted.local_device().unwrap().as_str(), client_id);
    assert_eq!(
        persisted
            .accepted_transactions()
            .filter(|(txn, _)| txn.txn_id.as_ref()
                == Some(&vw_model::Id::try_from(id(20)).unwrap().to_proto()))
            .count(),
        1
    );
}

#[tokio::test]
async fn protected_identity_reopen_mismatch_and_cancelled_accept_fail_closed() {
    let _serial = SERIAL.lock().await;
    let fixture = FixtureTrust::default();
    let first = service(device(), fixture.clone()).await;
    let fingerprint = first.local_device().await.unwrap().fingerprint;
    drop(first);
    let reopened = service(device(), fixture.clone()).await;
    assert_eq!(
        reopened.local_device().await.unwrap().fingerprint,
        fingerprint
    );
    assert!(matches!(
        open_callback_session_service(DeviceId::from_bytes([9; 16]).to_string(), Box::new(fixture))
            .await,
        Err(SessionError::Authentication)
    ));
    let listener = reopened.listen_pairing("127.0.0.1:0".into()).await.unwrap();
    let token = cancel();
    token.cancel();
    assert!(matches!(
        listener.accept(token).await,
        Err(SessionError::Cancelled)
    ));
    let offer = listener.offer(false).await.unwrap();
    let client = service(
        DeviceId::from_bytes([8; 16]).to_string(),
        FixtureTrust::default(),
    )
    .await;
    let (a, b) = tokio::join!(
        listener.accept(cancel()),
        client.join_qr(offer.qr, offer.endpoint)
    );
    assert!(a.is_ok());
    assert!(b.is_ok());
    listener.close().await.unwrap();
}

#[tokio::test]
async fn code_pairing_requires_both_live_confirmation_handles() {
    let _serial = SERIAL.lock().await;
    let host = service(device(), FixtureTrust::default()).await;
    let client = service(
        DeviceId::from_bytes([8; 16]).to_string(),
        FixtureTrust::default(),
    )
    .await;
    let server = host.listen_pairing("127.0.0.1:0".into()).await.unwrap();
    let offer = server.offer(true).await.unwrap();
    let (a, b) = tokio::join!(
        server.accept(cancel()),
        client.join_code(offer.code, offer.endpoint, cancel())
    );
    let a = a.unwrap().confirmation.unwrap();
    let b = b.unwrap();
    assert_eq!(a.fingerprint(), b.fingerprint());
    assert!(host.paired_devices().await.unwrap().is_empty());
    assert!(client.paired_devices().await.unwrap().is_empty());
    let (a, b) = tokio::join!(a.confirm(a.fingerprint()), b.confirm(b.fingerprint()));
    assert!(a.is_ok());
    assert!(b.is_ok());
    assert_eq!(host.paired_devices().await.unwrap().len(), 1);
    server.close().await.unwrap();
}

#[tokio::test]
async fn bootstrap_and_reconnect_restore_undone_originals_before_redo() {
    let _serial = SERIAL.lock().await;
    let temp = temp();
    let (host_service, client_service) = paired().await;
    let host_path = temp.path().join("history-host");
    let initial = create_image_project(create(&host_path), cancel())
        .await
        .unwrap();
    initial.close().await.unwrap();
    drop(initial);
    let original = b"synthetic historical original";
    let asset = vw_model::AssetId::hash(original);
    {
        use vw_proto::v1 as pb;
        let mut store = vw_store::ProjectStore::open(&host_path).unwrap();
        let author = DeviceId::try_from(device()).unwrap();
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
        for (n, kind) in [
            (40, pb::op::Kind::AddAsset(definition)),
            (
                41,
                pb::op::Kind::UndoTransaction(pb::UndoTransaction {
                    target_txn_id: Some(vw_model::Id::try_from(id(40)).unwrap().to_proto()),
                }),
            ),
        ] {
            let txn = pb::Transaction {
                txn_id: Some(vw_model::Id::try_from(id(n)).unwrap().to_proto()),
                project_id: Some(store.project().id.to_proto()),
                device_id: author.to_string(),
                base_revision: Some(store.revision().unwrap()),
                created_at_wall_ms: TIME + i64::from(n),
                ops: vec![pb::Op {
                    op_id: Some(pb::OpId {
                        device_id: author.to_string(),
                        lamport: u64::from(n),
                    }),
                    kind: Some(kind),
                }],
                ..Default::default()
            };
            store.commit(&txn, &author, TIME + i64::from(n)).unwrap();
        }
        assert!(!store.project().assets.contains_key(&asset));
        assert_eq!(
            vw_store::BlobStore::new(&host_path)
                .unwrap()
                .put(original)
                .unwrap(),
            asset
        );
    }
    let host = open_project(host_path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let client_id = client_service.local_device().await.unwrap().device_id;
    let server = host_service
        .host_project(
            host.clone(),
            client_id,
            vec![SessionEndpoint {
                carrier: AppCarrier::TcpAdb,
                address: "127.0.0.1:0".into(),
            }],
        )
        .await
        .unwrap();
    let client_path = temp.path().join("history-client");
    let client = client_service
        .receive_project(
            ReceiveProjectOptions {
                path: client_path.to_string_lossy().into(),
                peer_device_id: device(),
                endpoints: server.endpoints(),
                expected_project_id: Some(id(1)),
                max_total_blob_bytes: 16 * 1024 * 1024,
            },
            cancel(),
        )
        .await
        .unwrap();
    let blobs = vw_store::BlobStore::new(&client_path).unwrap();
    assert_eq!(blobs.read(&asset).unwrap(), original);
    // Remove only the fixture's known immutable original to force reconnect's
    // retained-inventory acquisition path, while it is still hidden by undo.
    let owned = blobs.path(&asset).unwrap();
    assert!(owned.starts_with(client_path.canonicalize().unwrap()));
    std::fs::remove_file(&owned).unwrap();
    let link = client_service
        .connect_project(client.clone(), device(), server.endpoints())
        .await
        .unwrap();
    converged(&server, &link).await;
    assert_eq!(blobs.read(&asset).unwrap(), original);
    host.undo_redo(
        edit(42, host.info().await.unwrap().next_lamport),
        true,
        cancel(),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        while client.info().await.unwrap().host_seq != 3 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    converged(&server, &link).await;
    link.close().await.unwrap();
    server.close().await.unwrap();
    client.close().await.unwrap();
    host.close().await.unwrap();
    let persisted = vw_store::ProjectStore::open(&client_path).unwrap();
    assert!(persisted.project().assets.contains_key(&asset));
    assert_eq!(blobs.read(&asset).unwrap(), original);
}

#[tokio::test]
async fn phone_original_is_persisted_before_host_acceptance_and_survives_undo_reconnect() {
    let _serial = SERIAL.lock().await;
    let root = temp();
    let (host_service, client_service) = paired().await;
    let host_path = root.path().join("upload-host");
    let client_path = root.path().join("upload-client");
    let host = create_image_project(create(&host_path), cancel())
        .await
        .unwrap();
    let client_id = client_service.local_device().await.unwrap().device_id;
    assert!(
        host_service
            .project_role(host.clone())
            .await
            .unwrap()
            .is_host
    );
    assert!(matches!(
        client_service.project_role(host.clone()).await,
        Err(SessionError::Authentication)
    ));
    let server = host_service
        .host_project(
            host.clone(),
            client_id.clone(),
            vec![SessionEndpoint {
                carrier: AppCarrier::TcpAdb,
                address: "127.0.0.1:0".into(),
            }],
        )
        .await
        .unwrap();
    let client = client_service
        .receive_project(
            ReceiveProjectOptions {
                path: client_path.to_string_lossy().into(),
                peer_device_id: device(),
                endpoints: server.endpoints(),
                expected_project_id: Some(id(1)),
                max_total_blob_bytes: 16 * 1024 * 1024,
            },
            cancel(),
        )
        .await
        .unwrap();
    assert!(
        !client_service
            .project_role(client.clone())
            .await
            .unwrap()
            .is_host
    );
    let mut original = Vec::new();
    for (kind, payload) in [
        (b"ftyp", b"isom\0\0\0\0isommp42".to_vec()),
        (b"mdat", vec![37u8; 512 * 1024]),
        (b"moov", b"synthetic container".to_vec()),
    ] {
        original.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
        original.extend_from_slice(kind);
        original.extend_from_slice(&payload);
    }
    let path = root.path().join("source.mp4");
    std::fs::write(&path, &original).unwrap();
    let attached = client
        .attach_asset_file(
            AttachFileOptions {
                transaction_id: id(60),
                device_id: client_id.clone(),
                lamport: client.info().await.unwrap().next_lamport,
                now_ms: TIME + 60,
                source_path: path.to_string_lossy().into(),
                work_directory: root.path().to_string_lossy().into(),
                kind: FileAssetKind::Mp4,
                memory_budget_bytes: 256 * 1024 * 1024,
                max_encoded_bytes: 8 * 1024 * 1024,
                max_scratch_bytes: 16 * 1024 * 1024,
            },
            cancel(),
        )
        .await
        .unwrap();
    let asset = vw_model::AssetId::try_from(attached.asset_id).unwrap();
    let host_blobs = vw_store::BlobStore::new(&host_path).unwrap();
    assert!(!host_blobs.path(&asset).unwrap().exists());
    assert_eq!(host.info().await.unwrap().host_seq, 0);
    let link = client_service
        .connect_project(client.clone(), device(), server.endpoints())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if host.info().await.unwrap().host_seq == 1 {
                assert_eq!(host_blobs.read(&asset).unwrap(), original);
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    converged(&server, &link).await;
    link.close().await.unwrap();
    let mut undo = edit(61, client.info().await.unwrap().next_lamport);
    undo.device_id = client_id.clone();
    client.undo_redo(undo, false, cancel()).await.unwrap();
    client.close().await.unwrap();
    drop(client);
    let client = open_project(client_path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let link = client_service
        .connect_project(client.clone(), device(), server.endpoints())
        .await
        .unwrap();
    converged(&server, &link).await;
    let mut redo = edit(62, client.info().await.unwrap().next_lamport);
    redo.device_id = client_id;
    client.undo_redo(redo, true, cancel()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        while host.info().await.unwrap().host_seq != 3 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    converged(&server, &link).await;
    link.close().await.unwrap();
    server.close().await.unwrap();
    client.close().await.unwrap();
    host.close().await.unwrap();
    let store = vw_store::ProjectStore::open(&host_path).unwrap();
    assert_eq!(
        store.retained_assets().unwrap().get(&asset),
        Some(&(original.len() as u64))
    );
    assert!(store.project().assets.contains_key(&asset));
    assert_eq!(host_blobs.read(&asset).unwrap(), original);
}

#[tokio::test]
async fn original_acquisition_preflight_is_read_only_and_author_bound() {
    use vw_proto::v1 as pb;
    let _serial = SERIAL.lock().await;
    let root = temp();
    let path = root.path().join("preflight");
    let session = create_image_project(create(&path), cancel()).await.unwrap();
    session.close().await.unwrap();
    drop(session);
    let store = vw_store::ProjectStore::open(&path).unwrap();
    let before = store.checkpoint_bytes().unwrap();
    let author = DeviceId::from_bytes([8; 16]);
    let asset = vw_model::AssetId::hash(b"not acquired yet");
    let txn = pb::Transaction {
        txn_id: Some(vw_model::Id::try_from(id(64)).unwrap().to_proto()),
        project_id: Some(store.project().id.to_proto()),
        device_id: author.to_string(),
        base_revision: Some(store.revision().unwrap()),
        created_at_wall_ms: TIME + 64,
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: author.to_string(),
                lamport: 1,
            }),
            kind: Some(pb::op::Kind::AddAsset(pb::AddAsset {
                asset_id: asset.to_string(),
                format: "png".into(),
                width: 1,
                height: 1,
                orientation: 1,
                bit_depth: 8,
                color_space: "sRGB".into(),
                byte_size: 16,
                source: "import".into(),
                metadata_json: "{}".into(),
                ..Default::default()
            })),
        }],
        ..Default::default()
    };
    assert_eq!(
        store
            .validate_transaction(&txn, &author, TIME + 65)
            .unwrap()
            .ack
            .host_seq,
        1
    );
    assert!(
        store
            .validate_transaction(&txn, &DeviceId::try_from(device()).unwrap(), TIME + 65)
            .is_err()
    );
    assert_eq!(store.checkpoint_bytes().unwrap(), before);
    assert!(!store.retained_assets().unwrap().contains_key(&asset));
    assert!(!store.project().assets.contains_key(&asset));
}

fn scan_fixture_tree(root: &std::path::Path, secrets: &[zeroize::Zeroizing<Vec<u8>>]) -> usize {
    let mut pending = vec![root.to_path_buf()];
    let mut files = 0usize;
    let mut entries = 0usize;
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        assert!(!metadata.file_type().is_symlink());
        if metadata.is_dir() {
            for entry in std::fs::read_dir(path).unwrap() {
                entries += 1;
                assert!(entries < 256);
                pending.push(entry.unwrap().path());
            }
        } else {
            assert!(metadata.is_file());
            assert!(metadata.len() <= 16 * 1024 * 1024);
            let bytes = std::fs::read(path).unwrap();
            assert!(
                !secrets.iter().any(|secret| bytes
                    .windows(secret.len())
                    .any(|window| window == secret.as_slice())),
                "synthetic credential found in project/archive bytes"
            );
            files += 1;
        }
    }
    files
}

#[tokio::test]
async fn synchronized_projects_and_real_archive_exclude_generated_pairing_credentials() {
    let _serial = SERIAL.lock().await;
    let root = temp();
    let host_trust = FixtureTrust::default();
    let client_trust = FixtureTrust::default();
    let host_service = service(device(), host_trust.clone()).await;
    let client_service = service(
        DeviceId::from_bytes([8; 16]).to_string(),
        client_trust.clone(),
    )
    .await;
    let listener = host_service
        .listen_pairing("127.0.0.1:0".into())
        .await
        .unwrap();
    let offer = listener.offer(false).await.unwrap();
    let mut secrets = Vec::<zeroize::Zeroizing<Vec<u8>>>::new();
    for fixture in [&host_trust, &client_trust] {
        let bytes = zeroize::Zeroizing::new(fixture.0.lock().unwrap().clone().unwrap());
        let state = TrustState::decode_unprotected(&bytes).unwrap();
        let identity = state.identity().transport_identity().unwrap();
        let key = identity.key.secret_der().to_vec();
        assert!(key.len() >= 32);
        secrets.push(zeroize::Zeroizing::new(serde_json::to_vec(&key).unwrap()));
        secrets.push(zeroize::Zeroizing::new(key));
    }
    let qr: serde_json::Value = serde_json::from_slice(&offer.qr).unwrap();
    let qr_secret = qr["secret"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| u8::try_from(value.as_u64().unwrap()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(qr_secret.len(), 16);
    secrets.push(zeroize::Zeroizing::new(
        serde_json::to_vec(&qr_secret).unwrap(),
    ));
    secrets.push(zeroize::Zeroizing::new(qr_secret));
    secrets.push(zeroize::Zeroizing::new(offer.qr.clone()));
    let (accepted, joined) = tokio::join!(
        listener.accept(cancel()),
        client_service.join_qr(offer.qr, offer.endpoint)
    );
    assert!(accepted.unwrap().paired_device_id.is_some());
    assert_eq!(joined.unwrap(), device());
    let code = listener.offer(true).await.unwrap();
    assert_eq!(code.code.len(), 8);
    secrets.push(zeroize::Zeroizing::new(code.code.into_bytes()));
    listener.close().await.unwrap();
    let host_path = root.path().join("secret-free-host");
    let client_path = root.path().join("secret-free-client");
    let host = create_image_project(create(&host_path), cancel())
        .await
        .unwrap();
    draw(&host).await;
    let peer = client_service.local_device().await.unwrap().device_id;
    let server = host_service
        .host_project(
            host.clone(),
            peer.clone(),
            vec![SessionEndpoint {
                carrier: AppCarrier::TcpAdb,
                address: "127.0.0.1:0".into(),
            }],
        )
        .await
        .unwrap();
    let client = client_service
        .receive_project(
            ReceiveProjectOptions {
                path: client_path.to_string_lossy().into(),
                peer_device_id: device(),
                endpoints: server.endpoints(),
                expected_project_id: Some(id(1)),
                max_total_blob_bytes: 16 * 1024 * 1024,
            },
            cancel(),
        )
        .await
        .unwrap();
    let mut options = edit(70, client.info().await.unwrap().next_lamport);
    options.device_id = peer;
    client
        .apply_edit(options, vec![rectangle(71)], cancel())
        .await
        .unwrap();
    let link = client_service
        .connect_project(client.clone(), device(), server.endpoints())
        .await
        .unwrap();
    converged(&server, &link).await;
    link.close().await.unwrap();
    server.close().await.unwrap();
    client.close().await.unwrap();
    host.close().await.unwrap();
    let mut store = vw_store::ProjectStore::open(&host_path).unwrap();
    let archive = root.path().join("shared.vwbz");
    let report = vw_store::export_vwbz(&mut store, &archive).unwrap();
    assert!(report.entries >= 2);
    drop(store);
    assert!(scan_fixture_tree(&host_path, &secrets) > 0);
    assert!(scan_fixture_tree(&client_path, &secrets) > 0);
    assert_eq!(scan_fixture_tree(&archive, &secrets), 1);
    // Also inspect validated decompressed members, so a future archive codec
    // change cannot turn a raw-container scan into a false absence claim.
    let extracted = root.path().join("inspected-archive");
    let imported = vw_store::import_vwbz(
        &archive,
        &extracted,
        vw_store::ImportLimits {
            max_entries: 256,
            max_entry_bytes: 16 * 1024 * 1024,
            max_total_bytes: 32 * 1024 * 1024,
            max_compression_ratio: 100,
            max_metadata_bytes: 1024 * 1024,
        },
    )
    .unwrap();
    drop(imported);
    assert!(scan_fixture_tree(&extracted, &secrets) > 0);
}
