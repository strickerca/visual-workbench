//! Synthetic loopback integration. Test trust lives only in fixture memory;
//! production entry points use DPAPI or the platform protection callback.
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use std::time::Duration;
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
    let mut store = vw_store::ProjectStore::open(&path).unwrap();
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

fn listen(carrier: AppCarrier) -> SessionEndpoint {
    SessionEndpoint {
        carrier,
        address: "127.0.0.1:0".into(),
    }
}
fn addresses(endpoints: &[SessionEndpoint]) -> Vec<String> {
    endpoints
        .iter()
        .map(|endpoint| endpoint.address.clone())
        .collect()
}
fn receive_options(path: &std::path::Path, hints: Vec<SessionEndpoint>) -> ReceiveProjectOptions {
    ReceiveProjectOptions {
        path: path.to_string_lossy().into(),
        peer_device_id: device(),
        endpoints: hints,
        expected_project_id: Some(id(1)),
        max_total_blob_bytes: 16 * 1024 * 1024,
    }
}
fn linked(status: &SessionStatus) -> bool {
    matches!(status.status, SyncStatus::Syncing | SyncStatus::Synced)
}
/// Both ends authenticated on `carrier`. The status never names a carrier
/// before the pinned handshake and Hello have completed on it.
async fn on_carrier(host: &LiveSession, client: &LiveSession, carrier: AppCarrier) {
    let reached = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let (a, b) = (host.status(), client.status());
            if linked(&a) && linked(&b) && a.carrier == Some(carrier) && b.carrier == Some(carrier)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    let (a, b) = (host.status(), client.status());
    assert!(
        reached.is_ok(),
        "carrier deadline for {carrier:?}: host={:?}/{:?}/{:?}; client={:?}/{:?}/{:?}",
        a.status,
        a.carrier,
        a.failure,
        b.status,
        b.carrier,
        b.failure
    );
}
/// The client receives the host's revision `host_seq` over the connection it
/// already has: the link never leaves `carrier` while waiting.
async fn arrived(client: &LiveSession, host_seq: u64, carrier: AppCarrier) {
    let reached = tokio::time::timeout(Duration::from_secs(20), async {
        while client.status().host_seq < host_seq {
            let status = client.status();
            assert!(linked(&status) && status.carrier == Some(carrier));
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(
        reached.is_ok(),
        "revision {host_seq} did not arrive: {:?}/{:?}",
        client.status().status,
        client.status().failure
    );
}
async fn failed_with(link: &LiveSession, code: &str) {
    let reached = tokio::time::timeout(Duration::from_secs(20), async {
        while link.status().failure.as_deref() != Some(code) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(
        reached.is_ok(),
        "expected {code}, found {:?}",
        link.status().failure
    );
}
fn recorded(after: u64, area: &str, cause: &str) -> bool {
    core_diagnostics(after).events.iter().any(|event| {
        event.kind == CoreDiagnosticKind::Failure && event.area == area && event.name == cause
    })
}

#[tokio::test]
async fn a_paired_device_reconnects_with_no_address_supplied_and_a_hint_is_never_trusted() {
    let _serial = SERIAL.lock().await;
    let temp = temp();
    let (host_service, client_service) = paired().await;
    let host = create_image_project(create(&temp.path().join("host")), cancel())
        .await
        .unwrap();
    draw(&host).await;
    let client_id = client_service.local_device().await.unwrap().device_id;
    // The computer listens by itself; nobody chose an address or a port.
    assert!(matches!(
        host_service
            .host_project_automatic(host.clone(), client_id.clone(), vec![])
            .await,
        Err(SessionError::Invalid)
    ));
    let server = host_service
        .host_project_automatic(
            host.clone(),
            client_id.clone(),
            vec![listen(AppCarrier::QuicWifi)],
        )
        .await
        .unwrap();
    let listening = server.endpoints();
    assert_eq!(listening.len(), 1);
    // Asking for what is already bound changes nothing.
    assert_eq!(
        addresses(&server.listen_on(listening.clone()).await.unwrap()),
        addresses(&listening)
    );
    assert!(client_service.known_routes(device()).unwrap().is_empty());
    // First contact: the one address pairing already handed over, as a hint.
    let client_path = temp.path().join("client");
    let client = client_service
        .receive_project_automatic(receive_options(&client_path, listening.clone()), cancel())
        .await
        .unwrap();
    assert_eq!(
        addresses(&client_service.known_routes(device()).unwrap()),
        addresses(&listening)
    );

    // From here on no call supplies an address.
    let link = client_service
        .connect_project_automatic(client.clone(), device(), vec![])
        .await
        .unwrap();
    converged(&server, &link).await;
    assert_eq!(addresses(&link.endpoints()), addresses(&listening));
    assert_eq!(link.status().peer_device_id, device());
    assert!(matches!(
        link.listen_on(vec![]).await,
        Err(SessionError::Invalid)
    ));
    assert!(matches!(
        server.offer_routes(vec![]),
        Err(SessionError::Invalid)
    ));
    link.close().await.unwrap();
    drop(link);
    let mut add = edit(20, client.info().await.unwrap().next_lamport);
    add.device_id = client_id.clone();
    client
        .apply_edit(add, vec![rectangle(24)], cancel())
        .await
        .unwrap();
    host.apply_edit(
        edit(30, host.info().await.unwrap().next_lamport),
        vec![rectangle(31)],
        cancel(),
    )
    .await
    .unwrap();
    let link = client_service
        .connect_project_automatic(client.clone(), device(), vec![])
        .await
        .unwrap();
    converged(&server, &link).await;
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
    drop(link);

    // Nothing remembered and nothing offered: the session waits offline and
    // says why. It never reports a carrier it has not authenticated.
    client_service.forget_routes(device()).unwrap();
    let before = core_diagnostics(0).latest_sequence;
    let link = client_service
        .connect_project_automatic(client.clone(), device(), vec![])
        .await
        .unwrap();
    failed_with(&link, "no_known_route").await;
    assert!(!linked(&link.status()) && link.status().carrier.is_none());
    assert!(link.endpoints().is_empty());
    assert!(recorded(before, "session", "no known route"));

    // A device with the same DeviceId but another certificate listens on the
    // preferred carrier. Its address is only a hint.
    let (impostor_service, _) = paired().await;
    let impostor_project = create_image_project(create(&temp.path().join("impostor")), cancel())
        .await
        .unwrap();
    let impostor = impostor_service
        .host_project_automatic(
            impostor_project.clone(),
            client_id.clone(),
            vec![listen(AppCarrier::QuicTether)],
        )
        .await
        .unwrap();
    link.offer_routes(impostor.endpoints()).unwrap();
    failed_with(&link, "connection").await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(!linked(&link.status()) && link.status().carrier.is_none());
    assert!(!linked(&impostor.status()) && impostor.status().carrier.is_none());
    assert!(recorded(
        before,
        "session",
        "route did not prove the paired device"
    ));
    assert!(client_service.known_routes(device()).unwrap().is_empty());

    // Discovery: the computer advertises, the phone's platform browser reports
    // what it resolved, and the app offers those addresses as hints.
    //
    // A catalog fed by a platform browser is the phone's path (Android), and
    // it can be driven here on any host that is not Windows. On Windows the
    // core runs its own mDNS on named interfaces instead: advertising and
    // browsing start a real multicast responder, which a build machine does
    // not promise to carry, and browsing on no interface is refused. There
    // the test checks that refusal and then offers the listening address the
    // way a browser would have reported it.
    #[cfg(not(windows))]
    let discovered = {
        let advertised = host_service
            .create_discovery(addresses(&listening))
            .await
            .unwrap();
        let advertisement = advertised.advertisement().unwrap();
        let browsing = client_service.browse_discovery(vec![]).await.unwrap();
        browsing
            .observe(
                advertisement.service_type.clone(),
                advertisement.properties.clone(),
                addresses(&listening),
            )
            .await
            .unwrap();
        let found = browsing.snapshot().await.unwrap();
        assert_eq!(found.len(), 1);
        browsing.close().await.unwrap();
        advertised.close().await.unwrap();
        found[0].endpoints.clone()
    };
    #[cfg(windows)]
    let discovered = {
        assert!(matches!(
            client_service.browse_discovery(vec![]).await,
            Err(SessionError::Invalid)
        ));
        addresses(&listening)
    };
    let mut hints = impostor.endpoints();
    hints.extend(discovered.iter().map(|address| SessionEndpoint {
        carrier: AppCarrier::QuicWifi,
        address: address.clone(),
    }));
    link.offer_routes(hints).unwrap();
    converged(&server, &link).await;
    assert_eq!(link.status().carrier, Some(AppCarrier::QuicWifi));
    assert_eq!(addresses(&link.endpoints()), addresses(&listening));
    assert!(!linked(&impostor.status()));
    // Only the address that authenticated is remembered.
    assert_eq!(
        addresses(&client_service.known_routes(device()).unwrap()),
        addresses(&listening)
    );
    link.close().await.unwrap();
    drop(link);

    // Hints saved by an earlier run are accepted for a paired device only,
    // and only as numeric local addresses.
    client_service.forget_routes(device()).unwrap();
    assert!(matches!(
        client_service
            .restore_routes(DeviceId::from_bytes([9; 16]).to_string(), listening.clone())
            .await,
        Err(SessionError::Authentication)
    ));
    assert!(matches!(
        client_service
            .restore_routes(
                device(),
                vec![SessionEndpoint {
                    carrier: AppCarrier::QuicWifi,
                    address: "192.0.2.7:47000".into(),
                }]
            )
            .await,
        Err(SessionError::Invalid)
    ));
    client_service
        .restore_routes(device(), listening.clone())
        .await
        .unwrap();
    let link = client_service
        .connect_project_automatic(client.clone(), device(), vec![])
        .await
        .unwrap();
    converged(&server, &link).await;
    link.close().await.unwrap();
    drop(link);

    // Revocation still wins: the hints go, and a remembered address no longer
    // connects.
    client_service.revoke_device(device()).await.unwrap();
    assert!(client_service.known_routes(device()).unwrap().is_empty());
    assert!(matches!(
        client_service
            .restore_routes(device(), listening.clone())
            .await,
        Err(SessionError::Authentication)
    ));
    let link = client_service
        .connect_project_automatic(client.clone(), device(), listening.clone())
        .await
        .unwrap();
    failed_with(&link, "untrusted_or_revoked").await;
    assert!(!linked(&link.status()) && link.status().carrier.is_none());
    link.close().await.unwrap();
    impostor.close().await.unwrap();
    server.close().await.unwrap();
    impostor_project.close().await.unwrap();
    client.close().await.unwrap();
    host.close().await.unwrap();
    let persisted = vw_store::ProjectStore::open(&client_path).unwrap();
    assert!(persisted.pending().unwrap().is_empty());
    assert_eq!(
        persisted
            .accepted_transactions()
            .filter(|(txn, _)| txn.txn_id.as_ref()
                == Some(&vw_model::Id::try_from(id(20)).unwrap().to_proto()))
            .count(),
        1
    );
}

/// Stands in for one network path on loopback: while it is up, datagrams
/// pass both ways; once cut they vanish and neither end is told. It runs on a
/// thread of its own, so a packet never waits behind this test's own work.
struct Cable {
    address: String,
    up: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Conversations opened before this number are silenced; later ones pass.
    quiet_before: std::sync::Arc<std::sync::atomic::AtomicU64>,
    opened: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Dropping it ends the relay thread.
    _stop: tokio::sync::oneshot::Sender<()>,
}
impl Cable {
    fn to(target: &str) -> Self {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicU64, Ordering},
        };
        let target: std::net::SocketAddr = target.parse().unwrap();
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let address = socket.local_addr().unwrap().to_string();
        let up = Arc::new(AtomicBool::new(true));
        let quiet_before = Arc::new(AtomicU64::new(0));
        let opened = Arc::new(AtomicU64::new(0));
        let (live, quiet, count) = (up.clone(), quiet_before.clone(), opened.clone());
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let relay = async move {
                let front = Arc::new(tokio::net::UdpSocket::from_std(socket).unwrap());
                let mut flows =
                    std::collections::HashMap::<_, (u64, Arc<tokio::net::UdpSocket>)>::new();
                let mut buffer = vec![0_u8; 65536];
                loop {
                    // A datagram sent earlier to a port that has since closed
                    // can come back as an error on a later receive (always on
                    // Windows). The path is still there: carry on.
                    let Ok((size, from)) = front.recv_from(&mut buffer).await else {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                        continue;
                    };
                    if !live.load(Ordering::Acquire) {
                        continue;
                    }
                    let (flow, back) = match flows.get(&from) {
                        Some(known) => known.clone(),
                        None => {
                            let flow = count.fetch_add(1, Ordering::AcqRel);
                            let back =
                                Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
                            back.connect(target).await.unwrap();
                            flows.insert(from, (flow, back.clone()));
                            let (front, live, quiet, replies) =
                                (front.clone(), live.clone(), quiet.clone(), back.clone());
                            tokio::spawn(async move {
                                let mut buffer = vec![0_u8; 65536];
                                loop {
                                    let Ok(size) = replies.recv(&mut buffer).await else {
                                        tokio::time::sleep(Duration::from_millis(1)).await;
                                        continue;
                                    };
                                    if live.load(Ordering::Acquire)
                                        && flow >= quiet.load(Ordering::Acquire)
                                    {
                                        let _ = front.send_to(&buffer[..size], from).await;
                                    }
                                }
                            });
                            (flow, back)
                        }
                    };
                    if flow >= quiet.load(Ordering::Acquire) {
                        let _ = back.send(&buffer[..size]).await;
                    }
                }
            };
            runtime.block_on(async move {
                tokio::select! {
                    _ = relay => {}
                    _ = stopped => {}
                }
            });
        });
        Self {
            address,
            up,
            quiet_before,
            opened,
            _stop: stop,
        }
    }
    fn plug(&self, connected: bool) {
        self.up
            .store(connected, std::sync::atomic::Ordering::Release);
    }
    /// The far end stops talking on what is already open (as during a long
    /// local operation) but still answers anyone who asks anew.
    fn busy(&self, silent: bool) {
        use std::sync::atomic::Ordering;
        let before = if silent {
            self.opened.load(Ordering::Acquire)
        } else {
            0
        };
        self.quiet_before.store(before, Ordering::Release);
    }
}

/// When the session behind an automatic host ends, nothing keeps listening:
/// the paired device must never reach a host that accepts and then says
/// nothing.
#[tokio::test]
async fn an_automatic_host_stops_listening_when_its_session_ends() {
    let _serial = SERIAL.lock().await;
    let temp = temp();
    let (host_service, client_service) = paired().await;
    let host = create_image_project(create(&temp.path().join("host")), cancel())
        .await
        .unwrap();
    let client_id = client_service.local_device().await.unwrap().device_id;
    let server = host_service
        .host_project_automatic(host.clone(), client_id, vec![listen(AppCarrier::QuicWifi)])
        .await
        .unwrap();
    let listening = server.endpoints();
    assert_eq!(listening.len(), 1);

    // The project closes under the session: its loop ends without the app
    // having closed the session object yet.
    host.close().await.unwrap();
    let retired = tokio::time::timeout(Duration::from_secs(20), async {
        while !server.endpoints().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(retired.is_ok(), "the listeners outlived the session");
    assert!(!linked(&server.status()) && server.status().carrier.is_none());
    // Nothing can be made to listen again on a session that is over.
    assert!(matches!(
        server.listen_on(listening.clone()).await,
        Err(SessionError::Closed)
    ));
    assert!(server.endpoints().is_empty());
    // The address is free again: nothing of the session still holds it.
    let address: std::net::SocketAddr = listening[0].address.parse().unwrap();
    let freed = tokio::time::timeout(Duration::from_secs(20), async {
        while std::net::UdpSocket::bind(address).is_err() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(freed.is_ok(), "the listening address was still held");
    server.close().await.unwrap();
}

/// The newest diagnostics sequence number: later records belong to what the
/// test does next.
fn diagnostics_mark() -> u64 {
    core_diagnostics(u64::MAX).latest_sequence
}
/// A duration the core measured itself and recorded after `mark`.
fn measured(mark: u64, name: &str) -> Option<Duration> {
    core_diagnostics(mark)
        .events
        .iter()
        .rev()
        .find(|event| {
            event.kind == CoreDiagnosticKind::DurationMicros
                && event.area == "session"
                && event.name == name
        })
        .map(|event| Duration::from_micros(event.value))
}
fn counted(mark: u64, name: &str) -> bool {
    core_diagnostics(mark).events.iter().any(|event| {
        event.kind == CoreDiagnosticKind::Count && event.area == "session" && event.name == name
    })
}

/// PERF-007 in simulation: loopback only, no radio, cable or phone. Both
/// routes cross the same kind of relay, so a loaded machine delays them alike
/// and the test compares the routes, not this machine's spare time.
#[tokio::test]
async fn a_better_carrier_takes_over_and_a_dead_route_fails_over_without_losing_or_repeating_edits()
{
    let _serial = SERIAL.lock().await;
    let temp = temp();
    let (host_service, client_service) = paired().await;
    let host_path = temp.path().join("host");
    let host = create_image_project(create(&host_path), cancel())
        .await
        .unwrap();
    draw(&host).await;
    let client_id = client_service.local_device().await.unwrap().device_id;
    let server = host_service
        .host_project_automatic(
            host.clone(),
            client_id.clone(),
            vec![listen(AppCarrier::QuicTether), listen(AppCarrier::QuicWifi)],
        )
        .await
        .unwrap();
    let bound = |carrier| {
        server
            .endpoints()
            .into_iter()
            .find(|endpoint| endpoint.carrier == carrier)
            .unwrap()
    };
    let listening_wifi = bound(AppCarrier::QuicWifi);
    let listening_usb = bound(AppCarrier::QuicTether);
    let air = Cable::to(&listening_wifi.address);
    let cable = Cable::to(&listening_usb.address);
    let over = |carrier, path: &Cable| SessionEndpoint {
        carrier,
        address: path.address.clone(),
    };
    let wifi = over(AppCarrier::QuicWifi, &air);
    let usb = over(AppCarrier::QuicTether, &cable);
    let both = vec![usb.clone(), wifi.clone()];

    // The phone starts on Wi-Fi: no cable yet.
    let client = client_service
        .receive_project_automatic(
            receive_options(&temp.path().join("client"), vec![wifi.clone()]),
            cancel(),
        )
        .await
        .unwrap();
    let link = client_service
        .connect_project_automatic(client.clone(), device(), vec![])
        .await
        .unwrap();
    on_carrier(&server, &link, AppCarrier::QuicWifi).await;
    converged(&server, &link).await;
    let mark = |transaction: u8, object: u8, lamport: u64| {
        let mut options = edit(transaction, lamport);
        options.device_id = client_id.clone();
        (options, vec![rectangle(object)])
    };

    // USB tethering comes up: the link moves to it by itself.
    let (options, commands) = mark(20, 24, client.info().await.unwrap().next_lamport);
    client
        .apply_edit(options, commands, cancel())
        .await
        .unwrap();
    let offered_mark = diagnostics_mark();
    let offered = std::time::Instant::now();
    link.offer_routes(both.clone()).unwrap();
    on_carrier(&server, &link, AppCarrier::QuicTether).await;
    let upgrade = offered.elapsed();
    let upgrade_by_core =
        measured(offered_mark, "route upgrade").expect("the core times its own move");
    converged(&server, &link).await;

    // The computer goes quiet without going away, as during a long local
    // operation. Its address still answers, so the link is left where it is
    // instead of being torn down and restarted on Wi-Fi.
    let quiet_mark = diagnostics_mark();
    cable.busy(true);
    let stays = || {
        for status in [server.status(), link.status()] {
            assert!(linked(&status), "a busy peer must not be dropped");
            assert_eq!(status.carrier, Some(AppCarrier::QuicTether));
        }
    };
    // How soon the silence is noticed depends on how free this machine is
    // (silence is counted only while the link loop could read), so the wait
    // is for the core's own record that it asked the route in use and was
    // answered, not for a fixed time.
    let asked = tokio::time::timeout(Duration::from_secs(10), async {
        while !counted(quiet_mark, "silent route still answers") {
            stays();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(asked.is_ok(), "the silent route was never asked");
    // It is asked again every second, and each time it stays where it is.
    let quiet = std::time::Instant::now();
    while quiet.elapsed() < Duration::from_millis(1500) {
        stays();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    cable.busy(false);
    // The same connection carries on: a new edit crosses it.
    let resumed = host
        .apply_edit(
            edit(33, host.info().await.unwrap().next_lamport),
            vec![rectangle(34)],
            cancel(),
        )
        .await
        .unwrap();
    arrived(&link, resumed.host_seq, AppCarrier::QuicTether).await;
    converged(&server, &link).await;
    assert_eq!(link.status().carrier, Some(AppCarrier::QuicTether));

    // The cable is pulled while both sides have unsent work.
    host.apply_edit(
        edit(30, host.info().await.unwrap().next_lamport),
        vec![rectangle(31)],
        cancel(),
    )
    .await
    .unwrap();
    let pulled_mark = diagnostics_mark();
    cable.plug(false);
    let pulled = std::time::Instant::now();
    let (options, commands) = mark(21, 25, client.info().await.unwrap().next_lamport);
    client
        .apply_edit(options, commands, cancel())
        .await
        .unwrap();
    on_carrier(&server, &link, AppCarrier::QuicWifi).await;
    let failover = pulled.elapsed();
    let rescue_by_core =
        measured(pulled_mark, "route rescue").expect("the core times its own rescue");
    converged(&server, &link).await;

    // The phone's tethering comes back before the computer has an address on
    // it. The app offers what it knows, and the core starts asking the old
    // address, which stays dead.
    link.offer_routes(both.clone()).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (options, commands) = mark(22, 26, client.info().await.unwrap().next_lamport);
    client
        .apply_edit(options, commands, cancel())
        .await
        .unwrap();
    // The computer is back on the cable under a new address. This offer
    // arrives while the attempt at the old address is still in flight, and
    // must not wait for that attempt to run out.
    let cable_again = Cable::to(&listening_usb.address);
    let returned_mark = diagnostics_mark();
    let returned = std::time::Instant::now();
    link.offer_routes(vec![
        over(AppCarrier::QuicTether, &cable_again),
        wifi.clone(),
    ])
    .unwrap();
    on_carrier(&server, &link, AppCarrier::QuicTether).await;
    let comeback = returned.elapsed();
    let comeback_by_core =
        measured(returned_mark, "route upgrade").expect("the core times its own return");
    converged(&server, &link).await;
    println!(
        "carrier switch on loopback: to better carrier {upgrade:?} (core {upgrade_by_core:?}), off dead route {failover:?} (core, after noticing {rescue_by_core:?}), back to better carrier {comeback:?} (core {comeback_by_core:?})"
    );
    // PERF-007: sync resumes within two seconds of the better carrier
    // becoming available. Measured from outside (offer to both ends linked on
    // it) and by the core itself (offer to an authenticated connection).
    let budget = Duration::from_secs(2);
    assert!(upgrade < budget, "upgrade took {upgrade:?}");
    assert!(upgrade_by_core < budget, "upgrade took {upgrade_by_core:?}");
    assert!(comeback < budget, "return took {comeback:?}");
    assert!(
        comeback_by_core < budget,
        "return took {comeback_by_core:?}"
    );
    // Leaving a dead route. The core's own interval, from having seen a
    // second of silence to an authenticated connection on the other route,
    // keeps the same two seconds.
    assert!(rescue_by_core < budget, "rescue took {rescue_by_core:?}");
    // Seen from outside it also holds that second of silence and both ends
    // resuming. Silence is counted only while the link loop was free to read,
    // in slices of at most 50 ms, so on a loaded machine that second
    // stretches. NOTICING allows for this machine's load alone; it is not a
    // budget for the product.
    const NOTICING: Duration = Duration::from_secs(6);
    assert!(failover < NOTICING + budget, "failover took {failover:?}");

    // The listeners can change under a live connection without dropping it:
    // one is retired, another address is added later.
    assert_eq!(
        addresses(
            &server
                .listen_on(vec![listening_wifi.clone()])
                .await
                .unwrap()
        ),
        vec![listening_wifi.address.clone()]
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(linked(&server.status()) && linked(&link.status()));
    assert_eq!(server.status().carrier, Some(AppCarrier::QuicTether));
    let widened = server
        .listen_on(vec![listen(AppCarrier::QuicTether), listening_wifi.clone()])
        .await
        .unwrap();
    assert_eq!(widened.len(), 2);
    assert_eq!(server.endpoints().len(), 2);
    converged(&server, &link).await;

    for project in [&host, &client] {
        assert_eq!(
            project
                .document_snapshot(id(2), cancel())
                .await
                .unwrap()
                .render
                .items
                .len(),
            6
        );
    }
    assert_eq!(
        host.info().await.unwrap().state_hash,
        client.info().await.unwrap().state_hash
    );
    link.close().await.unwrap();
    server.close().await.unwrap();
    client.close().await.unwrap();
    host.close().await.unwrap();
    let persisted = vw_store::ProjectStore::open(&host_path).unwrap();
    for transaction in [20, 21, 22, 30, 33] {
        assert_eq!(
            persisted
                .accepted_transactions()
                .filter(|(txn, _)| txn.txn_id.as_ref()
                    == Some(&vw_model::Id::try_from(id(transaction)).unwrap().to_proto()))
                .count(),
            1,
            "transaction {transaction} must be accepted exactly once"
        );
    }
}
