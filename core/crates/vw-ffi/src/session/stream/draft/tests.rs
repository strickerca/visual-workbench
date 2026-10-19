#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::super::{
    hub::{Command, Hub},
    memory::ByteBudget,
    receiver_tests::{BUDGET, WORKERS, id, owned},
    verified, verified_tests,
};
use super::*;
use std::{
    future::{Future, poll_fn},
    task::Poll,
    time::Duration,
};

struct Fixture {
    draft: Arc<CaptureDraft>,
    _stream: Arc<CaptureStream>,
    target: Arc<ProjectSession>,
    compressed: Arc<CaptureBackground>,
    original: Arc<CaptureBackground>,
    endpoint: Arc<super::super::hub::Endpoint>,
    temporary: tempfile::TempDir,
}
impl Fixture {
    async fn new() -> Self {
        let temporary = verified_tests::root();
        let hub = Hub::new();
        let (endpoint, mut commands) = hub.activate(1).unwrap();
        let mut opening = Box::pin(hub.open(1, BUDGET));
        assert!(poll_fn(|cx| Poll::Ready(opening.as_mut().poll(cx).is_pending())).await);
        let Command::Enable { lease, reply, .. } = commands.recv().await.unwrap() else {
            panic!("enable");
        };
        let lease = lease.upgrade().unwrap();
        reply.send(Ok(())).unwrap();
        let stream = opening.await.unwrap();
        let memory = ByteBudget::new(BUDGET).unwrap();
        let frame = owned(1, &memory);
        let request = vw_stream::FreezeRequest {
            draft_id: id(8),
            frame: frame.identity().clone(),
            epoch: 1,
        };
        let compressed =
            CaptureBackground::new(lease.clone(), &request, frame.clone(), None).unwrap();
        let (offer, file) = verified_tests::fixture(temporary.path());
        let actual = Arc::new(
            verified::verify(offer, file, memory, &vw_stream::Cancellation::default()).unwrap(),
        );
        let original = CaptureBackground::new(lease, &request, frame, Some(actual)).unwrap();
        let bytes = pixels::preview_png(&compressed, &Cancellation::new()).unwrap();
        let asset = pixels::asset(&bytes, &compressed.info(), &Cancellation::new()).unwrap();
        let device = DeviceId::from_bytes([3; 16]);
        let mut model = Project::new(id(40), "Live paired project".into(), device.clone());
        model.assets.insert(
            vw_model::AssetId::try_from(asset.asset_id.clone()).unwrap(),
            asset.clone(),
        );
        model.documents.insert(
            id(41),
            Document {
                definition: pb::CreateDocument {
                    document_id: Some(id(41).to_proto()),
                    kind: pb::DocumentKind::Image as i32,
                    schema_version: 1,
                    title: "Existing".into(),
                    primary_asset_id: asset.asset_id,
                    capture: None,
                },
                pages: vec![],
                created_at_ms: 1,
            },
        );
        let path = temporary.path().join("live");
        let (store, blobs) = crate::creation::create_complete(
            &path,
            crate::creation::InitialProject {
                project: model,
                device,
                time: 1,
            },
            &bytes,
            &vw_model::AssetId::hash(&bytes),
            &Cancellation::new(),
            |_| Ok(()),
        )
        .unwrap();
        let target =
            ProjectSession::from_store(store, blobs, SessionPermit::acquire().unwrap()).unwrap();
        let source = compressed.clone();
        let create = create(
            target.clone(),
            stream.clone(),
            CaptureDraftOptions {
                document_id: id(10).to_string(),
                layer_id: id(11).to_string(),
                memory_budget_bytes: MAX_MEMORY,
            },
            Arc::new(Cancellation::new()),
        );
        let serve = async {
            let Command::Background { reply, .. } = commands.recv().await.unwrap() else {
                panic!("background");
            };
            reply
                .send(Ok(Some(source)))
                .unwrap_or_else(|_| panic!("background delivery"));
        };
        let (draft, ()) = tokio::join!(create, serve);
        Self {
            draft: draft.unwrap(),
            _stream: stream,
            target,
            compressed,
            original,
            endpoint,
            temporary,
        }
    }
    async fn source(&self) -> WorkflowBinding {
        binding(&self.draft.life.project, id(10)).await
    }
    async fn target(&self) -> WorkflowBinding {
        binding(&self.target, id(41)).await
    }
    fn metadata(&self) -> WorkflowMetadata {
        WorkflowMetadata {
            transaction_id: id(20).to_string(),
            device_id: DeviceId::from_bytes([3; 16]).to_string(),
            first_lamport: 1,
            created_at_ms: 2000,
        }
    }
    async fn prepare(&self) -> WorkflowResult<Arc<CaptureDraftSave>> {
        self.draft
            .prepare_save(
                self.source().await,
                self.target().await,
                self.metadata(),
                self.original.clone(),
                MAX_MEMORY,
                Arc::new(Cancellation::new()),
            )
            .await
    }
    async fn close(self) {
        self.draft.close_draft().await.unwrap();
        self.target.close().await.unwrap();
        self.endpoint.stop();
        self.endpoint.done.send_replace(Some(Ok(())));
    }
}
async fn binding(project: &ProjectSession, document: Id) -> WorkflowBinding {
    let info = project.info().await.unwrap();
    WorkflowBinding {
        project_id: info.project_id,
        document_id: document.to_string(),
        host_seq: info.host_seq,
        state_hash: info.state_hash,
    }
}
async fn hold(project: &ProjectSession) -> std::sync::mpsc::Sender<()> {
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    project
        .worker
        .enqueue(Box::new(move |_| {
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
        }))
        .unwrap();
    ready.await.unwrap();
    release
}
#[tokio::test]
async fn late_lossless_original_is_required_and_exact_save_is_durable_retryable() {
    let _serial = WORKERS.lock().await;
    let fixture = Fixture::new().await;
    let missing = fixture
        .draft
        .prepare_save(
            fixture.source().await,
            fixture.target().await,
            fixture.metadata(),
            fixture.compressed.clone(),
            MAX_MEMORY,
            Arc::new(Cancellation::new()),
        )
        .await;
    assert!(matches!(missing, Err(WorkflowError::Missing)));
    assert!(!fixture.draft.life.sealed.load(Ordering::Acquire));
    let plan = fixture.prepare().await.unwrap();
    let expected = plan.describe().unwrap();
    assert!(fixture.draft.life.sealed.load(Ordering::Acquire));
    let receipt = plan.commit(Arc::new(Cancellation::new())).await.unwrap();
    assert!(!receipt.duplicate);
    assert_eq!(receipt.revision.host_seq, 1);
    let repeated = plan.commit(Arc::new(Cancellation::new())).await.unwrap();
    assert!(repeated.duplicate);
    assert_eq!(repeated.revision.host_seq, 1);
    fixture
        .target
        .worker
        .call(move |state| {
            let document = &state.project()?.documents[&id(10)].definition;
            assert_eq!(document.primary_asset_id, expected.saved_asset_id);
            assert!(document.capture.as_ref().unwrap().lossless);
            assert!(!document.capture.as_ref().unwrap().degraded);
            let hash = vw_model::AssetId::try_from(expected.saved_asset_id)?;
            assert!(state.blobs.verify(&hash)? > 0);
            assert_eq!(state.project()?.documents.len(), 2);
            Ok(())
        })
        .await
        .unwrap();
    assert!(!fixture.endpoint.stop.load(Ordering::Acquire));
    plan.dispose();
    assert!(fixture.draft.life.sealed.load(Ordering::Acquire));
    fixture.close().await;
}
#[tokio::test]
async fn failed_preparation_and_plan_drop_restore_draft_edit_admission() {
    let _serial = WORKERS.lock().await;
    let fixture = Fixture::new().await;
    let mut stale = fixture.source().await;
    stale.host_seq += 1;
    assert!(matches!(
        fixture
            .draft
            .prepare_save(
                stale,
                fixture.target().await,
                fixture.metadata(),
                fixture.original.clone(),
                MAX_MEMORY,
                Arc::new(Cancellation::new())
            )
            .await,
        Err(WorkflowError::Stale)
    ));
    assert!(!fixture.draft.life.sealed.load(Ordering::Acquire));
    let plan = fixture.prepare().await.unwrap();
    assert!(fixture.draft.life.sealed.load(Ordering::Acquire));
    drop(plan);
    assert!(!fixture.draft.life.sealed.load(Ordering::Acquire));
    fixture.close().await;
}
#[tokio::test]
async fn retired_link_refuses_new_save_without_redirecting_or_mutating_target() {
    let _serial = WORKERS.lock().await;
    let fixture = Fixture::new().await;
    let plan = fixture.prepare().await.unwrap();
    let prior = fixture.target.info().await.unwrap();
    fixture.endpoint.stop();
    assert!(matches!(
        plan.commit(Arc::new(Cancellation::new())).await,
        Err(WorkflowError::Closed)
    ));
    assert_eq!(
        fixture.target.info().await.unwrap().state_hash,
        prior.state_hash
    );
    drop(plan);
    fixture.close().await;
}
#[tokio::test]
async fn cancellation_of_queued_save_keeps_snapshot_sealed_until_actual_settlement() {
    let _serial = WORKERS.lock().await;
    let fixture = Fixture::new().await;
    let plan = fixture.prepare().await.unwrap();
    let release = hold(&fixture.target).await;
    let cancel = Arc::new(Cancellation::new());
    let mut saving = Box::pin(plan.commit(cancel.clone()));
    assert!(poll_fn(|cx| Poll::Ready(saving.as_mut().poll(cx).is_pending())).await);
    cancel.cancel();
    assert!(fixture.draft.life.sealed.load(Ordering::Acquire));
    release.send(()).unwrap();
    assert!(matches!(saving.await, Err(WorkflowError::Cancelled)));
    assert_eq!(fixture.target.info().await.unwrap().host_seq, 0);
    drop(plan);
    assert!(!fixture.draft.life.sealed.load(Ordering::Acquire));
    fixture.close().await;
}
#[tokio::test]
async fn central_commit_refuses_a_queued_edit_after_save_preparation_seals() {
    let _serial = WORKERS.lock().await;
    let fixture = Fixture::new().await;
    let source = fixture.source().await;
    let target = fixture.target().await;
    let release = hold(&fixture.draft.life.project).await;
    let mut preparing = Box::pin(fixture.draft.prepare_save(
        source,
        target,
        fixture.metadata(),
        fixture.original.clone(),
        MAX_MEMORY,
        Arc::new(Cancellation::new()),
    ));
    assert!(poll_fn(|cx| Poll::Ready(preparing.as_mut().poll(cx).is_pending())).await);
    let mut edit = Box::pin(fixture.draft.life.project.worker.call(|state| {
        let txn = pb::Transaction {
            txn_id: Some(id(55).to_proto()),
            project_id: Some(id(8).to_proto()),
            device_id: DeviceId::from_bytes([3; 16]).to_string(),
            base_revision: Some(state.store()?.revision()?),
            created_at_wall_ms: 2000,
            ops: vec![pb::Op {
                op_id: Some(pb::OpId {
                    device_id: DeviceId::from_bytes([3; 16]).to_string(),
                    lamport: 1,
                }),
                kind: Some(pb::op::Kind::UpdateDocument(pb::UpdateDocument {
                    document_id: Some(id(10).to_proto()),
                    title: "Must not write".into(),
                })),
            }],
            ..Default::default()
        };
        state.commit(&txn, &DeviceId::from_bytes([3; 16]), 2000)
    }));
    assert!(poll_fn(|cx| Poll::Ready(edit.as_mut().poll(cx).is_pending())).await);
    release.send(()).unwrap();
    let plan = preparing.await.unwrap();
    assert!(matches!(edit.await, Err(crate::CoreError::Backpressure)));
    assert_eq!(fixture.draft.life.project.info().await.unwrap().host_seq, 0);
    drop(plan);
    fixture.close().await;
}
#[tokio::test]
async fn joined_disposal_waits_for_real_worker_and_removes_only_owned_draft() {
    let _serial = WORKERS.lock().await;
    let fixture = Fixture::new().await;
    let path = fixture
        .draft
        .life
        .project
        .worker
        .call(|state| {
            let probe = state.store()?.transfer_file()?;
            Ok(probe.path().ancestors().nth(3).unwrap().to_path_buf())
        })
        .await
        .unwrap();
    assert!(path.join("project.sqlite").exists());
    let keep_borrow = fixture.draft.project().unwrap();
    let release = hold(&keep_borrow).await;
    let mut close = Box::pin(fixture.draft.close_draft());
    assert!(poll_fn(|cx| Poll::Ready(close.as_mut().poll(cx).is_pending())).await);
    assert!(path.exists());
    drop(close);
    release.send(()).unwrap();
    fixture.draft.close_draft().await.unwrap();
    assert!(!path.exists());
    assert!(
        fixture
            .temporary
            .path()
            .join("live/project.sqlite")
            .exists()
    );
    assert!(matches!(
        keep_borrow.info().await,
        Err(crate::CoreError::Closed)
    ));
    fixture.close().await;
}
#[tokio::test]
async fn annotation_geometry_survives_atomic_save_undo_redo_and_reopen() {
    let _serial = WORKERS.lock().await;
    let fixture = Fixture::new().await;
    let edit = crate::EditOptions {
        transaction_id: id(50).to_string(),
        document_id: id(10).to_string(),
        device_id: DeviceId::from_bytes([3; 16]).to_string(),
        lamport: 1,
        created_at_ms: 2000,
    };
    fixture
        .draft
        .life
        .project
        .apply_edit(
            edit,
            vec![crate::EditCommand::Create {
                object_id: id(51).to_string(),
                layer_id: id(11).to_string(),
                shape: crate::NewShape::Rectangle {
                    rectangle: crate::QueryRect {
                        x: 1.0,
                        y: 2.0,
                        width: 4.0,
                        height: 3.0,
                    },
                },
                style: crate::ObjectStyle {
                    rgba: 0x10203080,
                    width: 2.5,
                    screen_constant_width: false,
                    fill: Some(0x44556670),
                },
                transform: crate::Transform {
                    a: 1.5,
                    b: 0.25,
                    c: -0.5,
                    d: 1.0,
                    e: 3.0,
                    f: 2.0,
                },
            }],
            Arc::new(Cancellation::new()),
        )
        .await
        .unwrap();
    let expected = fixture
        .draft
        .life
        .project
        .worker
        .call(|state| Ok(state.project()?.objects[&id(51)].clone()))
        .await
        .unwrap();
    let plan = fixture.prepare().await.unwrap();
    plan.commit(Arc::new(Cancellation::new())).await.unwrap();
    fixture
        .target
        .worker
        .call(move |state| {
            assert_eq!(&state.project()?.objects[&id(51)], &expected);
            Ok(())
        })
        .await
        .unwrap();
    let saved_hash = fixture.target.info().await.unwrap().state_hash;
    for (n, redo, count) in [(60, false, 0), (61, true, 1)] {
        let info = fixture.target.info().await.unwrap();
        fixture
            .target
            .undo_redo(
                crate::EditOptions {
                    transaction_id: id(n).to_string(),
                    document_id: id(41).to_string(),
                    device_id: info.device_id,
                    lamport: info.next_lamport,
                    created_at_ms: 3000,
                },
                redo,
                Arc::new(Cancellation::new()),
            )
            .await
            .unwrap();
        fixture
            .target
            .worker
            .call(move |state| {
                assert_eq!(state.project()?.objects.len(), count);
                Ok(())
            })
            .await
            .unwrap();
    }
    assert_eq!(fixture.target.info().await.unwrap().state_hash, saved_hash);
    plan.dispose();
    fixture.draft.close_draft().await.unwrap();
    fixture.target.close().await.unwrap();
    let reopened = crate::open_project(
        fixture
            .temporary
            .path()
            .join("live")
            .to_string_lossy()
            .into(),
        Arc::new(Cancellation::new()),
    )
    .await
    .unwrap();
    assert_eq!(reopened.info().await.unwrap().state_hash, saved_hash);
    reopened.close().await.unwrap();
    fixture.close().await;
}
#[tokio::test]
async fn cancelled_prepare_future_keeps_edit_seal_until_the_queued_worker_drops_its_capture() {
    let _serial = WORKERS.lock().await;
    let fixture = Fixture::new().await;
    let source = fixture.source().await;
    let target = fixture.target().await;
    let release = hold(&fixture.draft.life.project).await;
    let mut prepare = Box::pin(fixture.draft.prepare_save(
        source,
        target,
        fixture.metadata(),
        fixture.original.clone(),
        MAX_MEMORY,
        Arc::new(Cancellation::new()),
    ));
    assert!(poll_fn(|cx| Poll::Ready(prepare.as_mut().poll(cx).is_pending())).await);
    assert!(fixture.draft.life.sealed.load(Ordering::Acquire));
    drop(prepare);
    assert!(fixture.draft.life.sealed.load(Ordering::Acquire));
    release.send(()).unwrap();
    fixture
        .draft
        .life
        .project
        .worker
        .call(|_| Ok(()))
        .await
        .unwrap();
    assert!(!fixture.draft.life.sealed.load(Ordering::Acquire));
    fixture.close().await;
}
#[tokio::test]
async fn draft_disposal_joins_a_dropped_save_waiter_on_the_other_project_actor() {
    let _serial = WORKERS.lock().await;
    let fixture = Fixture::new().await;
    let path = fixture
        .draft
        .life
        .project
        .worker
        .call(|state| {
            let probe = state.store()?.transfer_file()?;
            Ok(probe.path().ancestors().nth(3).unwrap().to_path_buf())
        })
        .await
        .unwrap();
    let plan = fixture.prepare().await.unwrap();
    let release = hold(&fixture.target).await;
    let mut save = Box::pin(plan.commit(Arc::new(Cancellation::new())));
    assert!(poll_fn(|cx| Poll::Ready(save.as_mut().poll(cx).is_pending())).await);
    let mut closing = Box::pin(fixture.draft.close_draft());
    assert!(poll_fn(|cx| Poll::Ready(closing.as_mut().poll(cx).is_pending())).await);
    drop(save);
    drop(plan);
    assert!(path.exists());
    assert!(poll_fn(|cx| Poll::Ready(closing.as_mut().poll(cx).is_pending())).await);
    release.send(()).unwrap();
    closing.await.unwrap();
    assert!(!path.exists());
    assert_eq!(fixture.target.info().await.unwrap().host_seq, 0);
    fixture.close().await;
}
#[tokio::test]
async fn actual_saved_frame_can_resume_after_disposal_without_reviving_a_closed_source() {
    use super::super::presentation::{Freeze, Presentation};
    let _serial = WORKERS.lock().await;
    for source_closed in [false, true] {
        let memory = ByteBudget::new(BUDGET).unwrap();
        let mut shown = Presentation::new(1, &memory).unwrap();
        shown.offer(owned(1, &memory)).unwrap();
        let draw = shown.begin().unwrap().unwrap();
        shown.settle(&draw.token, true).unwrap();
        drop(draw);
        let mut frozen = Freeze::new(1, vw_stream::StreamMode::LiveAnnotate, &memory).unwrap();
        let request = frozen.contact(id(8), &shown, true, 0).unwrap().unwrap();
        let temporary = verified_tests::root();
        let (offer, file) = verified_tests::fixture(temporary.path());
        let original =
            verified::verify(offer, file, memory, &vw_stream::Cancellation::default()).unwrap();
        assert!(
            frozen
                .lossless(&request, original.pixels().clone())
                .unwrap()
        );
        frozen.saved(&request).unwrap();
        if source_closed {
            frozen.source_closed();
        }
        assert_eq!(frozen.cancel_resume(), Some(id(8)));
        assert!(frozen.request().is_none());
        let next = frozen.contact(id(9), &shown, true, 10);
        assert_eq!(next.is_err(), source_closed);
    }
}

#[tokio::test]
async fn absent_stream_proof_retains_legacy_capture_canonical_json_and_hash() {
    let _serial = WORKERS.lock().await;
    let fixture = Fixture::new().await;
    fixture
        .draft
        .life
        .project
        .worker
        .call(|state| {
            let project = state.project()?;
            let bytes = project.canonical_bytes()?;
            let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert!(
                json["documents"][id(10).to_string()]["definition"]["capture"]
                    .get("stream_save")
                    .is_none()
            );
            let reopened = vw_model::Project::from_canonical_bytes(&bytes)?;
            assert_eq!(reopened.state_hash()?, project.state_hash()?);
            assert_eq!(reopened.canonical_bytes()?, bytes);
            Ok(())
        })
        .await
        .unwrap();
    fixture.close().await;
}
#[tokio::test]
async fn explicit_degraded_save_requires_exact_native_timeout_and_keeps_no_original_claim() {
    let _serial = WORKERS.lock().await;
    let fixture = Fixture::new().await;
    let source = fixture.source().await;
    let target = fixture.target().await;
    assert!(matches!(
        fixture
            .draft
            .prepare_degraded_save(
                source.clone(),
                target.clone(),
                fixture.metadata(),
                fixture.compressed.clone(),
                MAX_MEMORY,
                Arc::new(Cancellation::new())
            )
            .await,
        Err(WorkflowError::Stale)
    ));
    assert!(!fixture.draft.life.sealed.load(Ordering::Acquire));
    fixture.endpoint.signal.send_modify(|s| {
        s.stage = super::super::api::StreamStage::FrozenDegraded;
        s.source_generation = 1;
    });
    let plan = fixture
        .draft
        .prepare_degraded_save(
            source,
            target.clone(),
            fixture.metadata(),
            fixture.compressed.clone(),
            MAX_MEMORY,
            Arc::new(Cancellation::new()),
        )
        .await
        .unwrap();
    let info = plan.describe().unwrap();
    assert!(info.degraded);
    assert!(info.original_asset_id.is_none());
    assert_eq!(info.saved_asset_id, fixture.draft.info().preview_asset_id);
    let receipt = plan.commit(Arc::new(Cancellation::new())).await.unwrap();
    assert_eq!(receipt.revision.host_seq, 1);
    fixture
        .target
        .worker
        .call(move |state| {
            let project = state.project()?;
            let doc = &project.documents[&id(10)].definition;
            let capture = doc.capture.as_ref().unwrap();
            let proof = capture.stream_save.as_ref().unwrap();
            assert!(capture.degraded);
            assert!(!capture.lossless);
            assert!(proof.original_asset_id.is_empty());
            assert_eq!(proof.saved_asset_id, doc.primary_asset_id);
            assert_eq!(proof.target_state_hash, target.state_hash);
            assert_eq!(proof.target_project_id, Some(id(40).to_proto()));
            assert_eq!(proof.target_host_seq, 0);
            Ok(())
        })
        .await
        .unwrap();
    let retry = plan.commit(Arc::new(Cancellation::new())).await.unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.revision.state_hash, receipt.revision.state_hash);
    plan.dispose();
    fixture.close().await;
}
#[tokio::test]
async fn native_timeout_cannot_downgrade_a_verified_original_or_other_generation() {
    let _serial = WORKERS.lock().await;
    let fixture = Fixture::new().await;
    fixture.endpoint.signal.send_modify(|s| {
        s.stage = super::super::api::StreamStage::FrozenDegraded;
        s.source_generation = 2;
    });
    assert!(matches!(
        fixture
            .draft
            .prepare_degraded_save(
                fixture.source().await,
                fixture.target().await,
                fixture.metadata(),
                fixture.compressed.clone(),
                MAX_MEMORY,
                Arc::new(Cancellation::new())
            )
            .await,
        Err(WorkflowError::Stale)
    ));
    fixture
        .endpoint
        .signal
        .send_modify(|s| s.source_generation = 1);
    assert!(matches!(
        fixture
            .draft
            .prepare_degraded_save(
                fixture.source().await,
                fixture.target().await,
                fixture.metadata(),
                fixture.original.clone(),
                MAX_MEMORY,
                Arc::new(Cancellation::new())
            )
            .await,
        Err(WorkflowError::Stale)
    ));
    assert_eq!(fixture.target.info().await.unwrap().host_seq, 0);
    assert!(!fixture.draft.life.sealed.load(Ordering::Acquire));
    fixture.close().await;
}
#[tokio::test]
async fn saved_capture_output_receipt_binds_native_frame_asset_and_final_full_revision() {
    let _serial = WORKERS.lock().await;
    let fixture = Fixture::new().await;
    let plan = fixture.prepare().await.unwrap();
    let save = plan.describe().unwrap();
    plan.commit(Arc::new(Cancellation::new())).await.unwrap();
    let output = fixture
        .target
        .export_image(
            crate::ExportOptions {
                document_id: id(10).to_string(),
                format: crate::ImageFormat::Png8,
                marked: true,
                region: None,
                matte_rgb: None,
                convert_to_srgb: false,
                assume_untagged_srgb: false,
                allow_depth_reduction: false,
                memory_budget_bytes: MAX_MEMORY,
            },
            Arc::new(Cancellation::new()),
        )
        .await
        .unwrap();
    let proof: serde_json::Value =
        serde_json::from_str(output.capture_provenance_json.as_ref().unwrap()).unwrap();
    assert_eq!(proof["document_id"], id(10).to_string());
    assert_eq!(proof["host_seq"], output.revision.host_seq);
    assert_eq!(proof["state_hash"], output.revision.state_hash);
    assert_eq!(proof["source_asset"], save.saved_asset_id);
    assert_eq!(proof["capture"]["lossless"], true);
    assert_eq!(proof["capture"]["degraded"], false);
    let metadata: vw_raster::ExportMetadata = serde_json::from_str(&output.metadata_json).unwrap();
    let mut wrong = output.revision.clone();
    wrong.state_hash = "f".repeat(64);
    assert!(
        crate::capture_provenance::export_receipt(&metadata, &id(10).to_string(), &wrong).is_err()
    );
    let normal = fixture
        .target
        .export_image(
            crate::ExportOptions {
                document_id: id(41).to_string(),
                format: crate::ImageFormat::Png8,
                marked: false,
                region: None,
                matte_rgb: None,
                convert_to_srgb: false,
                assume_untagged_srgb: false,
                allow_depth_reduction: false,
                memory_budget_bytes: MAX_MEMORY,
            },
            Arc::new(Cancellation::new()),
        )
        .await
        .unwrap();
    assert!(normal.capture_provenance_json.is_none());
    let normal_metadata: serde_json::Value = serde_json::from_str(&normal.metadata_json).unwrap();
    assert!(
        normal_metadata["settings"]
            .get("capture_provenance")
            .is_none()
    );
    plan.dispose();
    fixture.close().await;
}
#[tokio::test]
async fn degraded_capture_quality_and_saved_hash_survive_atomic_undo_redo_and_reopen() {
    let _serial = WORKERS.lock().await;
    let fixture = Fixture::new().await;
    fixture.endpoint.signal.send_modify(|s| {
        s.stage = super::super::api::StreamStage::FrozenDegraded;
        s.source_generation = 1;
    });
    let plan = fixture
        .draft
        .prepare_degraded_save(
            fixture.source().await,
            fixture.target().await,
            fixture.metadata(),
            fixture.compressed.clone(),
            MAX_MEMORY,
            Arc::new(Cancellation::new()),
        )
        .await
        .unwrap();
    let info = plan.describe().unwrap();
    plan.commit(Arc::new(Cancellation::new())).await.unwrap();
    let saved = fixture.target.info().await.unwrap();
    for (n, redo, count) in [(70, false, 1), (71, true, 2)] {
        let now = fixture.target.info().await.unwrap();
        fixture
            .target
            .undo_redo(
                crate::EditOptions {
                    transaction_id: id(n).to_string(),
                    document_id: id(41).to_string(),
                    device_id: now.device_id,
                    lamport: now.next_lamport,
                    created_at_ms: 3000,
                },
                redo,
                Arc::new(Cancellation::new()),
            )
            .await
            .unwrap();
        fixture
            .target
            .worker
            .call(move |state| {
                assert_eq!(state.project()?.documents.len(), count);
                Ok(())
            })
            .await
            .unwrap();
    }
    assert_eq!(
        fixture.target.info().await.unwrap().state_hash,
        saved.state_hash
    );
    plan.dispose();
    fixture.draft.close_draft().await.unwrap();
    fixture.target.close().await.unwrap();
    let opened = crate::open_project(
        fixture
            .temporary
            .path()
            .join("live")
            .to_string_lossy()
            .into(),
        Arc::new(Cancellation::new()),
    )
    .await
    .unwrap();
    opened
        .worker
        .call(move |state| {
            let p = state.project()?;
            let c = p.documents[&id(10)].definition.capture.as_ref().unwrap();
            assert!(c.degraded);
            assert!(!c.lossless);
            assert_eq!(
                c.stream_save.as_ref().unwrap().saved_asset_id,
                info.saved_asset_id
            );
            Ok(())
        })
        .await
        .unwrap();
    opened.close().await.unwrap();
    fixture.close().await;
}
