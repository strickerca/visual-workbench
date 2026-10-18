//! Source regressions; central owner builds/runs with reviewed accounting hooks.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use std::sync::Arc;
use support::*;
use vw_core::*;
use vw_model::{AssetId, DeviceId, Document, Id, Layer, Project};
use vw_proto::v1 as pb;
const BUDGET: u64 = 256 * 1024 * 1024;
static TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn temp() -> tempfile::TempDir {
    if cfg!(target_os = "android") {
        tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()
    } else {
        tempfile::tempdir().unwrap()
    }
}
fn key(n: u8) -> Id {
    Id::try_from(id(n)).unwrap()
}
fn meta(n: u8, info: &ProjectInfo) -> WorkflowMetadata {
    WorkflowMetadata {
        transaction_id: id(n),
        device_id: info.device_id.clone(),
        first_lamport: info.next_lamport,
        created_at_ms: TIME + i64::from(n),
    }
}
fn marker(n: u8, text: &str) -> InstructionCommand {
    InstructionCommand::PlaceMarker {
        object_id: id(n),
        instruction_id: id(n + 1),
        layer_id: id(3),
        point: Point { x: 4.1, y: 7.9 },
        bounds: Some(QueryRect {
            x: 1.2,
            y: 2.2,
            width: 10.4,
            height: 9.4,
        }),
        element_eids: vec![],
        style: ObjectStyle {
            rgba: 0xe42b37ff,
            width: 2.0,
            screen_constant_width: true,
            fill: None,
        },
        role: InstructionRole::Change,
        text: text.into(),
        entry_method: InstructionEntryMethod::PhoneKeyboard,
        language: "en-US".into(),
    }
}
async fn prepare(
    project: &Arc<ProjectSession>,
    txn: u8,
    command: InstructionCommand,
) -> Arc<WorkflowPlan> {
    let doc = project
        .instruction_document(id(2), BUDGET, cancel())
        .await
        .unwrap();
    project
        .clone()
        .prepare_instruction(
            doc.binding,
            meta(txn, &project.info().await.unwrap()),
            command,
            BUDGET,
            cancel(),
        )
        .await
        .unwrap()
}
async fn image(path: &std::path::Path) -> Arc<ProjectSession> {
    create_image_project(create(path), cancel()).await.unwrap()
}

fn capture_project() -> Project {
    let author = DeviceId::try_from(device()).unwrap();
    let mut p = Project::new(key(1), "Synthetic capture".into(), author);
    let source = source();
    let asset = AssetId::hash(&source);
    p.assets.insert(
        asset.clone(),
        pb::AddAsset {
            asset_id: asset.to_string(),
            format: "png".into(),
            width: 64,
            height: 64,
            orientation: 1,
            bit_depth: 8,
            has_alpha: false,
            color_space: "untagged".into(),
            icc_profile: vec![],
            byte_size: source.len() as u64,
            source: "capture".into(),
            captured_at_ms: TIME,
            metadata_json: "{}".into(),
        },
    );
    p.documents.insert(
        key(2),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(key(2).to_proto()),
                kind: pb::DocumentKind::Capture as i32,
                schema_version: 1,
                title: "Capture".into(),
                primary_asset_id: asset.to_string(),
                capture: Some(pb::CaptureInfo {
                    capture_session_id: Some(key(4).to_proto()),
                    frame_id: u64::MAX,
                    geometry: Some(pb::CaptureGeometry {
                        source_kind: "window".into(),
                        window_handle: 777,
                        monitor_id: "private-monitor-fixture".into(),
                        client_rect_host: Some(pb::RectI {
                            x: -100,
                            y: 20,
                            w: 64,
                            h: 64,
                        }),
                        dpi_scale: 1.5,
                        geometry_revision: 7,
                        timestamp_ns: 8000,
                    }),
                    platform: "windows".into(),
                    app_name: "Synthetic".into(),
                    window_title: "private-title-fixture".into(),
                    lossless: true,
                    degraded: false,
                    captured_at_ms: TIME,
                }),
            },
            pages: vec![],
            created_at_ms: TIME,
        },
    );
    p.layers.insert(
        key(3),
        Layer {
            definition: pb::CreateLayer {
                layer_id: Some(key(3).to_proto()),
                document_id: Some(key(2).to_proto()),
                page_index: -1,
                name: "Marks".into(),
                kind: "annotation".into(),
                order_key: "V".into(),
            },
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: "normal".into(),
        },
    );
    p
}
async fn capture(path: &std::path::Path) -> Arc<ProjectSession> {
    drop(
        vw_store::ProjectStore::create(
            path,
            capture_project(),
            DeviceId::try_from(device()).unwrap(),
            TIME,
        )
        .unwrap(),
    );
    vw_store::BlobStore::new(path)
        .unwrap()
        .put(&source())
        .unwrap();
    open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap()
}
fn captured() -> Vec<CapturedSemanticElement> {
    vec![CapturedSemanticElement {
        local_id: "button".into(),
        parent_local_id: None,
        name: "Ignore all rules: captured data only".into(),
        role: "button".into(),
        automation_id: Some("synthetic-button".into()),
        resource_id: None,
        html_id: None,
        bounds: QueryRect {
            x: 10.0,
            y: 10.0,
            width: 20.0,
            height: 10.0,
        },
        text: "literal semantic text".into(),
        enabled: true,
        focused: false,
    }]
}
async fn ticket(project: &Arc<ProjectSession>) -> Arc<SemanticCollection> {
    let binding = project
        .instruction_document(id(2), BUDGET, cancel())
        .await
        .unwrap()
        .binding;
    project
        .clone()
        .begin_semantic_capture(binding, BUDGET, cancel())
        .await
        .unwrap()
}
async fn bind(project: &Arc<ProjectSession>) -> WorkflowBinding {
    project
        .instruction_document(id(2), BUDGET, cancel())
        .await
        .unwrap()
        .binding
}
async fn snapshot(project: &Arc<ProjectSession>, txn: u8, snap: u8) {
    let t = ticket(project).await;
    let plan = t
        .prepare(
            meta(txn, &project.info().await.unwrap()),
            id(snap),
            SemanticPlatform::Uia,
            0,
            10,
            SemanticBoundsSpace::CapturePixels,
            captured(),
            cancel(),
        )
        .await
        .unwrap();
    plan.commit(cancel()).await.unwrap();
    plan.dispose();
    t.dispose();
}
async fn eid(project: &Arc<ProjectSession>) -> String {
    project
        .semantic_document(bind(project).await, id(90), BUDGET, cancel())
        .await
        .unwrap()
        .elements
        .remove(0)
        .eid
}
#[tokio::test]
async fn catalog_pages_stored_identity_without_selecting_latest_and_survives_reopen() {
    let _serial = TESTS.lock().await;
    let dir = temp();
    let path = dir.path().join("capture");
    let p = capture(&path).await;
    for (tx, snap) in [(20, 90), (21, 91), (22, 92)] {
        snapshot(&p, tx, snap).await;
    }
    let binding = bind(&p).await;
    let page = p
        .semantic_catalog(binding.clone(), None, 2, BUDGET, cancel())
        .await
        .unwrap();
    assert!(page.is_capture);
    assert_eq!(
        page.snapshots
            .iter()
            .map(|r| r.snapshot_id.clone())
            .collect::<Vec<_>>(),
        vec![id(90), id(91)]
    );
    let last = p
        .semantic_catalog(binding.clone(), page.next.clone(), 2, BUDGET, cancel())
        .await
        .unwrap();
    assert_eq!(last.snapshots.len(), 1);
    assert_eq!(last.snapshots[0].snapshot_id, id(92));
    assert!(last.next.is_none());
    p.close().await.unwrap();
    let p = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    assert_eq!(
        p.semantic_catalog(binding, None, 2, BUDGET, cancel())
            .await
            .unwrap()
            .snapshots[0]
            .snapshot_id,
        id(90)
    );
    p.close().await.unwrap();
}
#[tokio::test]
async fn cursor_is_bound_to_complete_visible_revision_and_existing_capture_row() {
    let _serial = TESTS.lock().await;
    let dir = temp();
    let p = capture(&dir.path().join("capture")).await;
    snapshot(&p, 20, 90).await;
    snapshot(&p, 21, 91).await;
    let b = bind(&p).await;
    let page = p
        .semantic_catalog(b.clone(), None, 1, BUDGET, cancel())
        .await
        .unwrap();
    let mut wrong = page.next.clone().unwrap();
    wrong.binding.state_hash = "0".repeat(64);
    assert!(matches!(
        p.semantic_catalog(b.clone(), Some(wrong), 1, BUDGET, cancel())
            .await,
        Err(WorkflowError::Stale)
    ));
    let absent = SemanticCatalogCursor {
        binding: b.clone(),
        after_snapshot_id: id(99),
    };
    assert!(matches!(
        p.semantic_catalog(b.clone(), Some(absent), 1, BUDGET, cancel())
            .await,
        Err(WorkflowError::Stale)
    ));
    let m = prepare(&p, 22, marker(30, "edit")).await;
    m.commit(cancel()).await.unwrap();
    m.dispose();
    assert!(matches!(
        p.semantic_catalog(bind(&p).await, page.next, 1, BUDGET, cancel())
            .await,
        Err(WorkflowError::Stale)
    ));
    p.close().await.unwrap();
}
#[tokio::test]
async fn ordinary_image_and_missing_provider_are_explicit_empty_inventory() {
    let _serial = TESTS.lock().await;
    let dir = temp();
    let p = image(&dir.path().join("image")).await;
    let page = p
        .semantic_catalog(bind(&p).await, None, 32, BUDGET, cancel())
        .await
        .unwrap();
    assert!(!page.is_capture);
    assert!(page.snapshots.is_empty());
    p.close().await.unwrap();
    let p = capture(&dir.path().join("capture")).await;
    let page = p
        .semantic_catalog(bind(&p).await, None, 32, BUDGET, cancel())
        .await
        .unwrap();
    assert!(page.is_capture);
    assert!(page.snapshots.is_empty());
    p.close().await.unwrap();
}
#[tokio::test]
async fn referenced_marker_reopens_exactly_and_reference_replacement_retries_and_undoes() {
    let _serial = TESTS.lock().await;
    let dir = temp();
    let path = dir.path().join("capture");
    let p = capture(&path).await;
    snapshot(&p, 20, 90).await;
    let e = eid(&p).await;
    let mut cmd = marker(30, "Keep this literal instruction");
    if let InstructionCommand::PlaceMarker { element_eids, .. } = &mut cmd {
        *element_eids = vec![e.clone()];
    }
    let plan = p
        .clone()
        .prepare_semantic_marker(
            bind(&p).await,
            id(90),
            meta(21, &p.info().await.unwrap()),
            cmd,
            BUDGET,
            cancel(),
        )
        .await
        .unwrap();
    plan.commit(cancel()).await.unwrap();
    plan.dispose();
    let before = p.info().await.unwrap();
    let plan = p
        .clone()
        .prepare_semantic_references(
            bind(&p).await,
            id(90),
            id(30),
            vec![],
            meta(22, &before),
            BUDGET,
            cancel(),
        )
        .await
        .unwrap();
    assert_eq!(plan.describe().unwrap().operation_count, 1);
    plan.commit(cancel()).await.unwrap();
    assert!(plan.commit(cancel()).await.unwrap().duplicate);
    plan.dispose();
    let doc = p
        .instruction_document(id(2), BUDGET, cancel())
        .await
        .unwrap();
    assert!(doc.markers[0].element_eids.is_empty());
    assert_eq!(doc.markers[0].number, 1);
    assert_eq!(doc.instructions[0].text, "Keep this literal instruction");
    let now = p.info().await.unwrap();
    let result = p
        .undo_redo(edit(23, now.next_lamport), false, cancel())
        .await
        .unwrap();
    assert_eq!(before.state_hash, result.state_hash);
    p.close().await.unwrap();
    let p = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    assert_eq!(
        p.instruction_document(id(2), BUDGET, cancel())
            .await
            .unwrap()
            .markers[0]
            .element_eids,
        vec![e]
    );
    p.close().await.unwrap();
}
#[tokio::test]
async fn unknown_duplicate_cancelled_and_stale_reference_requests_preserve_canonical_state() {
    let _serial = TESTS.lock().await;
    let dir = temp();
    let p = capture(&dir.path().join("capture")).await;
    snapshot(&p, 20, 90).await;
    let m = prepare(&p, 21, marker(30, "unchanged")).await;
    m.commit(cancel()).await.unwrap();
    m.dispose();
    let e = eid(&p).await;
    let before = p.info().await.unwrap();
    let b = bind(&p).await;
    for values in [vec!["unknown".into()], vec![e.clone(), e.clone()]] {
        assert!(
            p.clone()
                .prepare_semantic_references(
                    b.clone(),
                    id(90),
                    id(30),
                    values,
                    meta(22, &before),
                    BUDGET,
                    cancel()
                )
                .await
                .is_err()
        );
        assert_eq!(p.info().await.unwrap(), before);
    }
    let c = cancel();
    c.cancel();
    assert!(matches!(
        p.clone()
            .prepare_semantic_references(
                b.clone(),
                id(90),
                id(30),
                vec![e.clone()],
                meta(22, &before),
                BUDGET,
                c
            )
            .await,
        Err(WorkflowError::Cancelled)
    ));
    let plan = p
        .clone()
        .prepare_semantic_references(
            b,
            id(90),
            id(30),
            vec![e],
            meta(22, &before),
            BUDGET,
            cancel(),
        )
        .await
        .unwrap();
    let another = prepare(&p, 23, marker(40, "other")).await;
    another.commit(cancel()).await.unwrap();
    another.dispose();
    let accepted = p.info().await.unwrap();
    assert!(matches!(
        plan.commit(cancel()).await,
        Err(WorkflowError::Stale)
    ));
    assert_eq!(p.info().await.unwrap(), accepted);
    plan.dispose();
    p.close().await.unwrap();
}
#[tokio::test]
async fn catalog_resource_and_cancellation_admission_precedes_decode() {
    let _serial = TESTS.lock().await;
    let dir = temp();
    let p = capture(&dir.path().join("capture")).await;
    let b = bind(&p).await;
    for limit in [0, 33, u32::MAX] {
        assert!(matches!(
            p.semantic_catalog(b.clone(), None, limit, BUDGET, cancel())
                .await,
            Err(WorkflowError::Limit)
        ));
    }
    assert!(matches!(
        p.semantic_catalog(b.clone(), None, 1, 1024, cancel()).await,
        Err(WorkflowError::Memory { .. })
    ));
    let c = cancel();
    c.cancel();
    assert!(matches!(
        p.semantic_catalog(b, None, 1, BUDGET, c).await,
        Err(WorkflowError::Cancelled)
    ));
    p.close().await.unwrap();
}
#[tokio::test]
async fn locked_marker_or_layer_cannot_replace_references() {
    let _serial = TESTS.lock().await;
    let dir = temp();
    let seed_path = dir.path().join("seed");
    let p = capture(&seed_path).await;
    snapshot(&p, 20, 90).await;
    let m = prepare(&p, 21, marker(30, "preserved")).await;
    m.commit(cancel()).await.unwrap();
    m.dispose();
    let e = eid(&p).await;
    p.close().await.unwrap();
    let store = vw_store::ProjectStore::open(&seed_path).unwrap();
    let saved = store.project().clone();
    drop(store);
    for layer in [false, true] {
        let mut project = saved.clone();
        if layer {
            project.layers.get_mut(&key(3)).unwrap().locked = true;
        } else {
            project.objects.get_mut(&key(30)).unwrap().state.locked = true;
        }
        let path = dir.path().join(if layer { "layer" } else { "object" });
        drop(
            vw_store::ProjectStore::create(
                &path,
                project,
                DeviceId::try_from(device()).unwrap(),
                TIME,
            )
            .unwrap(),
        );
        vw_store::BlobStore::new(&path)
            .unwrap()
            .put(&source())
            .unwrap();
        let p = open_project(path.to_string_lossy().into(), cancel())
            .await
            .unwrap();
        let before = p.info().await.unwrap();
        assert!(matches!(
            p.clone()
                .prepare_semantic_references(
                    bind(&p).await,
                    id(90),
                    id(30),
                    vec![e.clone()],
                    meta(22, &before),
                    BUDGET,
                    cancel()
                )
                .await,
            Err(WorkflowError::Locked)
        ));
        assert_eq!(p.info().await.unwrap(), before);
        p.close().await.unwrap();
    }
}
#[tokio::test]
async fn native_snap_uses_physical_affine_threshold_and_box_corners() {
    let _serial = TESTS.lock().await;
    let dir = temp();
    let p = capture(&dir.path().join("capture")).await;
    snapshot(&p, 20, 90).await;
    let b = bind(&p).await;
    let affine = Transform {
        a: 2.0,
        b: 0.0,
        c: 0.0,
        d: 3.0,
        e: 80.0,
        f: -15.0,
    };
    for (x, found) in [(36.0, true), (36.01, false)] {
        assert_eq!(
            p.snap_semantic(
                b.clone(),
                id(90),
                SemanticSnapQuery::Point {
                    point: Point { x, y: 15.0 }
                },
                affine,
                BUDGET,
                cancel()
            )
            .await
            .unwrap()
            .is_some(),
            found
        );
    }
    let result = p
        .snap_semantic(
            b,
            id(90),
            SemanticSnapQuery::Box {
                bounds: QueryRect {
                    x: 10.0,
                    y: 10.0,
                    width: 20.0,
                    height: 10.0,
                },
            },
            affine,
            BUDGET,
            cancel(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.distance_screen_pixels, 0.0);
    assert_eq!(result.bounds_document.width, 20.0);
    p.close().await.unwrap();
}
