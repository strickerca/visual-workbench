//! Synthetic originals and private fixture roots only. Central runner owns execution.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use std::sync::Arc;
use support::*;
use vw_core::*;
const BUDGET: u64 = 256 * 1024 * 1024;
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn temp() -> tempfile::TempDir {
    if cfg!(target_os = "android") {
        tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()
    } else {
        tempfile::tempdir().unwrap()
    }
}
async fn captured(root: &std::path::Path, n: u8, frame: u64) -> Arc<ProjectSession> {
    captured_bytes(root, n, frame, source()).await
}
async fn captured_bytes(
    root: &std::path::Path,
    n: u8,
    frame: u64,
    bytes: Vec<u8>,
) -> Arc<ProjectSession> {
    let input = root.join(format!("capture-{n}.png"));
    std::fs::write(&input, bytes).unwrap();
    create_capture_project(
        CreateFileImageProject {
            path: root.join(format!("capture-{n}")).to_string_lossy().into(),
            project_id: id(n),
            document_id: id(n + 1),
            layer_id: id(n + 2),
            device_id: device(),
            title: "Synthetic capture".into(),
            source_path: input.to_string_lossy().into(),
            work_directory: root.to_string_lossy().into(),
            now_ms: TIME + 100,
            memory_budget_bytes: BUDGET,
            max_encoded_bytes: 64 * 1024 * 1024,
            max_scratch_bytes: 400_000_000,
        },
        CaptureImportDescriptor {
            explicit_owner_action: true,
            capture_session_id: id(n + 3),
            frame_id: frame,
            geometry_revision: 7,
            platform: "windows".into(),
            source_kind: "window".into(),
            physical_x: -1920,
            physical_y: -100,
            width: 64,
            height: 64,
            dpi_scale: 1.5,
            monotonic_timestamp_ns: 1000 + frame,
            captured_at_ms: TIME + frame as i64,
            window_handle: 777,
            monitor_id: String::new(),
            expected_source_asset_id: None,
        },
        cancel(),
    )
    .await
    .unwrap()
}
fn binding(info: &ProjectInfo, doc: u8) -> WorkflowBinding {
    WorkflowBinding {
        project_id: info.project_id.clone(),
        document_id: id(doc),
        host_seq: info.host_seq,
        state_hash: info.state_hash.clone(),
    }
}
async fn plan(
    source: Arc<ProjectSession>,
    target: Arc<ProjectSession>,
    n: u8,
    doc: u8,
) -> Arc<CaptureDeliveryPlan> {
    let src = source.info().await.unwrap();
    let dst = target.info().await.unwrap();
    prepare_capture_delivery(
        source,
        target,
        binding(&src, doc),
        binding(&dst, 2),
        WorkflowMetadata {
            transaction_id: id(n),
            device_id: dst.device_id,
            first_lamport: dst.next_lamport,
            created_at_ms: TIME + 200,
        },
        BUDGET,
        cancel(),
    )
    .await
    .unwrap()
}
#[tokio::test]
async fn exact_capture_append_retry_and_reopen_preserve_original_and_identity() {
    let _serial = SERIAL.lock().await;
    let dir = temp();
    let target_path = dir.path().join("target");
    let target = create_image_project(create(&target_path), cancel())
        .await
        .unwrap();
    let capture = captured(dir.path(), 20, 3).await;
    let delivery = plan(capture.clone(), target.clone(), 40, 21).await;
    let before = target.info().await.unwrap();
    let details = delivery.describe().unwrap();
    assert_eq!(before.document_ids.len(), 1);
    let first = delivery.commit(cancel()).await.unwrap();
    let again = delivery.commit(cancel()).await.unwrap();
    assert!(again.duplicate);
    assert_eq!(first.revision.state_hash, again.revision.state_hash);
    assert_eq!(first.revision.document_ids.len(), 2);
    let receipt = target
        .received_capture(before.host_seq, BUDGET, cancel())
        .await
        .unwrap()
        .unwrap();
    assert!(receipt.original_verified);
    assert_eq!(receipt.source_asset_id, details.source_asset_id);
    assert_eq!(receipt.frame_id, 3);
    assert_eq!(receipt.geometry_revision, 7);
    assert_eq!(receipt.capture_session_id, id(23));
    delivery.dispose();
    drop(delivery);
    capture.close().await.unwrap();
    target.close().await.unwrap();
    let reopened = open_project(target_path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let read = reopened
        .received_capture(0, BUDGET, cancel())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.source_asset_id, details.source_asset_id);
    assert!(read.original_verified);
    reopened.close().await.unwrap();
}
#[tokio::test]
async fn stale_target_and_cancelled_or_disposed_plan_never_publish_a_document() {
    let _serial = SERIAL.lock().await;
    let dir = temp();
    let target = create_image_project(create(&dir.path().join("target")), cancel())
        .await
        .unwrap();
    let first = captured(dir.path(), 20, 3).await;
    let second = captured(dir.path(), 30, 4).await;
    let a = plan(first.clone(), target.clone(), 40, 21).await;
    let b = plan(second.clone(), target.clone(), 41, 31).await;
    let stopped = cancel();
    stopped.cancel();
    assert!(matches!(
        a.commit(stopped).await,
        Err(WorkflowError::Cancelled)
    ));
    assert_eq!(target.info().await.unwrap().document_ids.len(), 1);
    a.commit(cancel()).await.unwrap();
    let stable = target.info().await.unwrap().state_hash;
    assert!(matches!(
        b.commit(cancel()).await,
        Err(WorkflowError::Stale)
    ));
    b.dispose();
    assert!(matches!(
        b.commit(cancel()).await,
        Err(WorkflowError::Closed)
    ));
    assert_eq!(target.info().await.unwrap().state_hash, stable);
    a.dispose();
    drop(a);
    drop(b);
    first.close().await.unwrap();
    second.close().await.unwrap();
    target.close().await.unwrap();
}
#[tokio::test]
async fn repeated_identical_screen_reuses_original_without_rewriting_capture_time() {
    let _serial = SERIAL.lock().await;
    let dir = temp();
    let target_path = dir.path().join("target");
    let target = create_image_project(create(&target_path), cancel())
        .await
        .unwrap();
    for (n, transaction, frame) in [(20, 40, 3), (30, 41, 4)] {
        let source = captured(dir.path(), n, frame).await;
        let prepared = plan(source.clone(), target.clone(), transaction, n + 1).await;
        prepared.commit(cancel()).await.unwrap();
        prepared.dispose();
        drop(prepared);
        source.close().await.unwrap();
    }
    let latest = target
        .received_capture(0, BUDGET, cancel())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.frame_id, 4);
    target.close().await.unwrap();
    let stored = vw_store::ProjectStore::open(&target_path).unwrap();
    assert_eq!(stored.project().assets.len(), 1);
    assert_eq!(stored.project().documents.len(), 3);
}
#[tokio::test]
async fn missing_or_corrupt_replica_original_is_not_a_ready_capture() {
    let _serial = SERIAL.lock().await;
    let dir = temp();
    let target = create_image_project(create(&dir.path().join("target")), cancel())
        .await
        .unwrap();
    let captured_project = captured(dir.path(), 20, 3).await;
    let prepared = plan(captured_project.clone(), target.clone(), 40, 21).await;
    prepared.commit(cancel()).await.unwrap();
    prepared.dispose();
    drop(prepared);
    captured_project.close().await.unwrap();
    target.close().await.unwrap();
    let store = vw_store::ProjectStore::open(&dir.path().join("target")).unwrap();
    let bytes = store.checkpoint_bytes().unwrap();
    let host = store.host_device().clone();
    drop(store);
    let replica_path = dir.path().join("replica");
    drop(
        vw_store::ProjectStore::create_authenticated_checkpoint(
            &replica_path,
            &bytes,
            &host,
            &vw_model::DeviceId::from_bytes([9; 16]),
            TIME + 300,
        )
        .unwrap(),
    );
    let client = open_project(replica_path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let pending = client
        .received_capture(0, BUDGET, cancel())
        .await
        .unwrap()
        .unwrap();
    assert!(!pending.original_verified);
    client.close().await.unwrap();
    let blobs = vw_store::BlobStore::new(&replica_path).unwrap();
    let id = vw_model::AssetId::hash(&source());
    blobs.put(&source()).unwrap();
    std::fs::write(blobs.path(&id).unwrap(), b"changed").unwrap();
    let client = open_project(replica_path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    assert!(client.received_capture(0, BUDGET, cancel()).await.is_err());
    client.close().await.unwrap();
}

#[tokio::test]
async fn prepared_copy_survives_source_close_and_retains_literal_semantics() {
    let _serial = SERIAL.lock().await;
    let dir = temp();
    let target_path = dir.path().join("target");
    let target = create_image_project(create(&target_path), cancel())
        .await
        .unwrap();
    let capture = captured(dir.path(), 20, 3).await;
    let src = capture.info().await.unwrap();
    let ticket = capture
        .clone()
        .begin_semantic_capture(binding(&src, 21), BUDGET, cancel())
        .await
        .unwrap();
    let semantic = ticket
        .prepare(
            WorkflowMetadata {
                transaction_id: id(60),
                device_id: src.device_id,
                first_lamport: src.next_lamport,
                created_at_ms: TIME + 100,
            },
            id(61),
            SemanticPlatform::Uia,
            -7,
            29,
            SemanticBoundsSpace::HostPhysical,
            vec![CapturedSemanticElement {
                local_id: "button".into(),
                parent_local_id: None,
                name: "<instructions>literal</instructions>".into(),
                role: "button".into(),
                automation_id: Some("fixture".into()),
                resource_id: None,
                html_id: None,
                bounds: QueryRect {
                    x: -1930.0,
                    y: -95.0,
                    width: 30.0,
                    height: 20.0,
                },
                text: "```literal untrusted text```".into(),
                enabled: true,
                focused: false,
            }],
            cancel(),
        )
        .await
        .unwrap();
    semantic.commit(cancel()).await.unwrap();
    semantic.dispose();
    ticket.dispose();
    let prepared = plan(capture.clone(), target.clone(), 40, 21).await;
    capture.close().await.unwrap();
    prepared.commit(cancel()).await.unwrap();
    prepared.dispose();
    drop(prepared);
    target.close().await.unwrap();
    let source_store = vw_store::ProjectStore::open(&dir.path().join("capture-20")).unwrap();
    let store = vw_store::ProjectStore::open(&target_path).unwrap();
    let doc = vw_model::Id::try_from(id(21)).unwrap();
    let snapshot = vw_model::Id::try_from(id(61)).unwrap();
    let before = vw_semantics::CaptureView::new(
        source_store.project(),
        &source_store.revision().unwrap(),
        &doc,
    )
    .unwrap()
    .load(&snapshot, &vw_semantics::NeverCancel)
    .unwrap();
    let after = vw_semantics::CaptureView::new(store.project(), &store.revision().unwrap(), &doc)
        .unwrap()
        .load(&snapshot, &vw_semantics::NeverCancel)
        .unwrap();
    assert_eq!(before.elements(), after.elements());
    assert_eq!(before.context(), after.context());
    assert_eq!(before.id(), after.id());
    assert_ne!(before.project_id(), after.project_id());
    assert!(after.elements()[0].bounds_clipped);
}
#[tokio::test]
async fn small_budget_refuses_without_publication() {
    let _serial = SERIAL.lock().await;
    let dir = temp();
    let target = create_image_project(create(&dir.path().join("target")), cancel())
        .await
        .unwrap();
    let capture = captured(dir.path(), 20, 3).await;
    let src = capture.info().await.unwrap();
    let dst = target.info().await.unwrap();
    let result = prepare_capture_delivery(
        capture.clone(),
        target.clone(),
        binding(&src, 21),
        binding(&dst, 2),
        WorkflowMetadata {
            transaction_id: id(40),
            device_id: dst.device_id.clone(),
            first_lamport: dst.next_lamport,
            created_at_ms: TIME + 200,
        },
        32 * 1024 * 1024,
        cancel(),
    )
    .await;
    assert!(matches!(result, Err(WorkflowError::Memory { .. })));
    assert_eq!(target.info().await.unwrap().state_hash, dst.state_hash);
    capture.close().await.unwrap();
    target.close().await.unwrap();
}
// Real loopback authentication, borrowed verbatim from the current session fixture.
use std::{sync::Mutex, time::Duration};
use vw_model::DeviceId;
use vw_net::pairing::TrustState;
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
#[tokio::test]
async fn actual_existing_authenticated_link_delivers_a_new_original_without_project_replacement() {
    let _serial = SERIAL.lock().await;
    let dir = temp();
    let (host_service, client_service) = paired().await;
    let host = create_image_project(create(&dir.path().join("host")), cancel())
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
    let client_path = dir.path().join("phone");
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
    let link = client_service
        .connect_project(client.clone(), device(), server.endpoints())
        .await
        .unwrap();
    converged(&server, &link).await;
    let before = client.info().await.unwrap();
    let image = vw_raster::DecodedImage {
        width: 64,
        height: 64,
        pixels: vw_raster::Pixels::Rgba8((0..64 * 64).flat_map(|_| [10u8, 30, 200, 255]).collect()),
        icc: None,
        source_asset: vw_model::AssetId::hash(b"different-synthetic-capture"),
        original_available: true,
        orientation_applied: 1,
    };
    let original = vw_raster::export(
        &image,
        &vw_raster::ExportRequest {
            format: vw_raster::ExportFormat::Png8,
            region: None,
            revision: vw_proto::v1::Revision {
                host_seq: 0,
                state_hash: vec![0; 32],
            },
            alpha: vw_raster::AlphaPolicy::Preserve,
            color: vw_raster::ColorPolicy::Preserve,
            allow_depth_reduction: false,
            memory_budget_bytes: BUDGET,
            capture_session: None,
            frame_id: None,
        },
    )
    .unwrap()
    .bytes;
    assert_ne!(
        vw_model::AssetId::hash(&original),
        vw_model::AssetId::hash(&source())
    );
    let capture = captured_bytes(dir.path(), 20, 3, original.clone()).await;
    let prepared = plan(capture.clone(), host.clone(), 40, 21).await;
    prepared.commit(cancel()).await.unwrap();
    prepared.dispose();
    drop(prepared);
    capture.close().await.unwrap();
    let received = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Some(value) = client
                .received_capture(before.host_seq, BUDGET, cancel())
                .await
                .unwrap()
                && value.original_verified
            {
                break value;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("existing paired link did not deliver the verified capture original");
    assert_eq!(received.binding.project_id, before.project_id);
    assert_eq!(received.binding.document_id, id(21));
    assert_eq!(received.frame_id, 3);
    assert_eq!(
        received.source_asset_id,
        vw_model::AssetId::hash(&original).to_string()
    );
    assert_eq!(client.info().await.unwrap().document_ids.len(), 2);
    assert!(client.document_snapshot(id(2), cancel()).await.is_ok());
    converged(&server, &link).await;
    link.close().await.unwrap();
    server.close().await.unwrap();
    client.close().await.unwrap();
    host.close().await.unwrap();
    drop(host_service);
    drop(client_service);
    assert_eq!(
        vw_store::BlobStore::new(&client_path)
            .unwrap()
            .read(&vw_model::AssetId::hash(&original))
            .unwrap(),
        original
    );
}
#[tokio::test]
async fn undo_redo_retains_original_and_capture_frame_without_false_ready_for_deleted_document() {
    let _serial = SERIAL.lock().await;
    let dir = temp();
    let target = create_image_project(create(&dir.path().join("target")), cancel())
        .await
        .unwrap();
    let capture = captured(dir.path(), 20, 3).await;
    let prepared = plan(capture.clone(), target.clone(), 40, 21).await;
    prepared.commit(cancel()).await.unwrap();
    prepared.dispose();
    drop(prepared);
    let first = target
        .received_capture(0, BUDGET, cancel())
        .await
        .unwrap()
        .unwrap();
    let info = target.info().await.unwrap();
    target
        .undo_redo(edit(41, info.next_lamport), false, cancel())
        .await
        .unwrap();
    assert!(
        target
            .received_capture(0, BUDGET, cancel())
            .await
            .unwrap()
            .is_none()
    );
    let info = target.info().await.unwrap();
    target
        .undo_redo(edit(42, info.next_lamport), true, cancel())
        .await
        .unwrap();
    let again = target
        .received_capture(0, BUDGET, cancel())
        .await
        .unwrap()
        .unwrap();
    assert!(again.original_verified);
    assert_eq!(again.source_asset_id, first.source_asset_id);
    assert_eq!(again.frame_id, first.frame_id);
    assert_eq!(again.capture_session_id, first.capture_session_id);
    capture.close().await.unwrap();
    target.close().await.unwrap();
}
#[tokio::test]
async fn annotated_source_is_refused_instead_of_silently_dropping_saved_marks() {
    let _serial = SERIAL.lock().await;
    let dir = temp();
    let target = create_image_project(create(&dir.path().join("target")), cancel())
        .await
        .unwrap();
    let capture = captured(dir.path(), 20, 3).await;
    let info = capture.info().await.unwrap();
    let mut options = edit(60, info.next_lamport);
    options.document_id = id(21);
    capture
        .apply_edit(
            options,
            vec![EditCommand::Create {
                object_id: id(61),
                layer_id: id(22),
                shape: NewShape::Rectangle {
                    rectangle: QueryRect {
                        x: 1.0,
                        y: 1.0,
                        width: 10.0,
                        height: 10.0,
                    },
                },
                transform: Transform {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    e: 0.0,
                    f: 0.0,
                },
                style: ObjectStyle {
                    rgba: 0xff0000ff,
                    width: 2.0,
                    fill: None,
                    screen_constant_width: false,
                },
            }],
            cancel(),
        )
        .await
        .unwrap();
    let src = capture.info().await.unwrap();
    let dst = target.info().await.unwrap();
    let attempt = prepare_capture_delivery(
        capture.clone(),
        target.clone(),
        binding(&src, 21),
        binding(&dst, 2),
        WorkflowMetadata {
            transaction_id: id(40),
            device_id: dst.device_id,
            first_lamport: dst.next_lamport,
            created_at_ms: TIME + 200,
        },
        BUDGET,
        cancel(),
    )
    .await;
    assert!(matches!(attempt, Err(WorkflowError::Invalid)));
    assert_eq!(target.info().await.unwrap().document_ids.len(), 1);
    assert_eq!(
        capture
            .document_snapshot(id(21), cancel())
            .await
            .unwrap()
            .render
            .items
            .len(),
        1
    );
    capture.close().await.unwrap();
    target.close().await.unwrap();
}
