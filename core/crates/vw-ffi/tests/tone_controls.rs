//! Canonical-operation and immutable-original regressions; run by the coordinator.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use std::sync::Arc;
use support::*;
use vw_core::*;
use vw_model::{AssetId, DeviceId};

const BUDGET: u64 = 256 * 1024 * 1024;
static TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn temp() -> tempfile::TempDir {
    if cfg!(target_os = "android") {
        tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()
    } else {
        tempfile::tempdir().unwrap()
    }
}
fn metadata(n: u8, info: &ProjectInfo) -> WorkflowMetadata {
    WorkflowMetadata {
        transaction_id: id(n),
        device_id: info.device_id.clone(),
        first_lamport: info.next_lamport,
        created_at_ms: TIME + i64::from(n),
    }
}
fn settings() -> ImageToneSettings {
    ImageToneSettings {
        brightness: -0.3,
        contrast: 0.1,
        input_black: 0.1,
        input_white: 0.9,
        gamma: 1.2,
        output_black: 0.1,
        output_white: 0.8,
    }
}
fn set() -> ToneControlCommand {
    ToneControlCommand::Set {
        layer_id: id(30),
        object_id: id(31),
        settings: settings(),
        enabled: true,
        opacity: 0.5,
    }
}
fn region() -> SelectionRegion {
    SelectionRegion {
        x: 3,
        y: 5,
        width: 17,
        height: 13,
    }
}
async fn snapshot(project: &Arc<ProjectSession>) -> ToneControlSnapshot {
    project
        .tone_controls_snapshot(id(2), BUDGET, cancel())
        .await
        .unwrap()
}
async fn prepare(
    project: &Arc<ProjectSession>,
    n: u8,
    command: ToneControlCommand,
) -> Arc<WorkflowPlan> {
    let view = snapshot(project).await;
    project
        .clone()
        .prepare_tone_control(
            view.binding,
            metadata(n, &project.info().await.unwrap()),
            command,
            BUDGET,
            cancel(),
        )
        .await
        .unwrap()
}
async fn apply(project: &Arc<ProjectSession>, n: u8, command: ToneControlCommand) {
    let plan = prepare(project, n, command).await;
    plan.commit(cancel()).await.unwrap();
    plan.dispose();
}
async fn preview(
    project: &Arc<ProjectSession>,
    binding: WorkflowBinding,
    command: Option<ToneControlCommand>,
) -> ToneControlPreview {
    project
        .preview_tone_control(binding, command, region(), true, BUDGET, cancel())
        .await
        .unwrap()
}
async fn control(project: &Arc<ProjectSession>, n: u8, command: DocumentControlCommand) {
    let view = project
        .document_controls_snapshot(id(2), BUDGET, cancel())
        .await
        .unwrap();
    let plan = project
        .clone()
        .prepare_document_control(
            view.binding,
            metadata(n, &view.revision),
            command,
            BUDGET,
            cancel(),
        )
        .await
        .unwrap();
    plan.commit(cancel()).await.unwrap();
    plan.dispose();
}
fn layer_lock(locked: bool) -> DocumentControlCommand {
    DocumentControlCommand::UpdateLayer {
        update: LayerControlUpdate {
            layer_id: id(30),
            name: None,
            visible: None,
            locked: Some(locked),
            opacity: None,
            blend: None,
        },
    }
}

#[tokio::test]
async fn draft_and_prepare_are_read_only_and_committed_preview_matches_undo_redo() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let path = root.path().join("p");
    let original = source();
    let project = create_image_project(create(&path), cancel()).await.unwrap();
    let initial = project.info().await.unwrap();
    let view = snapshot(&project).await;
    assert!(view.tones.is_empty());
    assert!(
        !project
            .document_snapshot(id(2), cancel())
            .await
            .unwrap()
            .has_tone_adjustments
    );
    assert_eq!(
        (view.source_width, view.source_height, view.source_bit_depth),
        (64, 64, 8)
    );
    let clean = preview(&project, view.binding.clone(), None).await;
    let draft = preview(&project, view.binding.clone(), Some(set())).await;
    assert_eq!(draft.binding, view.binding);
    assert_eq!(
        (draft.x, draft.y, draft.width, draft.height),
        (3, 5, 17, 13)
    );
    assert_eq!(draft.rgba.len(), 17 * 13 * 4);
    assert_ne!(draft.rgba, clean.rgba);
    assert_eq!(project.info().await.unwrap(), initial);
    assert!(snapshot(&project).await.tones.is_empty());
    let plan = prepare(&project, 20, set()).await;
    assert_eq!(plan.describe().unwrap().operation_count, 3);
    assert_eq!(project.info().await.unwrap(), initial);
    let committed = plan.commit(cancel()).await.unwrap();
    assert!(!committed.duplicate);
    let retry = plan.commit(cancel()).await.unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.revision, committed.revision);
    plan.dispose();
    let accepted = snapshot(&project).await;
    assert_eq!(accepted.tones.len(), 1);
    let editing = project.document_snapshot(id(2), cancel()).await.unwrap();
    assert!(editing.has_tone_adjustments);
    assert!(
        !editing
            .render
            .items
            .iter()
            .any(|item| matches!(&item.shape, DrawShape::Adjustment))
    );
    assert_eq!(accepted.tones[0].settings, settings());
    assert_eq!(accepted.tones[0].opacity, 0.5);
    assert_eq!(
        preview(&project, accepted.binding.clone(), None).await.rgba,
        draft.rgba
    );
    let now = project.info().await.unwrap();
    project
        .undo_redo(edit(21, now.next_lamport), false, cancel())
        .await
        .unwrap();
    let undone = snapshot(&project).await;
    assert!(undone.tones.is_empty());
    assert_eq!(
        preview(&project, undone.binding, None).await.rgba,
        clean.rgba
    );
    let now = project.info().await.unwrap();
    project
        .undo_redo(edit(22, now.next_lamport), true, cancel())
        .await
        .unwrap();
    let redone = snapshot(&project).await;
    assert_eq!(
        preview(&project, redone.binding.clone(), None).await.rgba,
        draft.rgba
    );
    let removal = ToneControlCommand::Remove {
        layer_id: id(30),
        object_id: id(31),
    };
    let removed_draft = preview(&project, redone.binding, Some(removal.clone())).await;
    assert_eq!(removed_draft.rgba, clean.rgba);
    assert_eq!(snapshot(&project).await.tones.len(), 1);
    apply(&project, 23, removal).await;
    assert!(snapshot(&project).await.tones.is_empty());
    project.close().await.unwrap();
    let store = vw_store::ProjectStore::open(&path).unwrap();
    assert_eq!(store.project().assets.len(), 1);
    assert!(store.project().objects.is_empty());
    let asset = store.project().assets.keys().next().unwrap();
    assert_eq!(
        vw_store::BlobStore::new(&path)
            .unwrap()
            .read(asset)
            .unwrap(),
        original
    );
    drop(store);
    let reopened = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    assert!(snapshot(&reopened).await.tones.is_empty());
    assert!(
        !reopened
            .document_snapshot(id(2), cancel())
            .await
            .unwrap()
            .has_tone_adjustments
    );
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn stale_binding_and_layer_or_object_locks_refuse_both_prepare_and_draft() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let project = create_image_project(create(&root.path().join("p")), cancel())
        .await
        .unwrap();
    let stale = snapshot(&project).await.binding;
    apply(&project, 20, set()).await;
    let info = project.info().await.unwrap();
    assert!(matches!(
        project
            .preview_tone_control(stale.clone(), Some(set()), region(), true, BUDGET, cancel())
            .await,
        Err(WorkflowError::Stale)
    ));
    assert!(matches!(
        project
            .clone()
            .prepare_tone_control(stale, metadata(21, &info), set(), BUDGET, cancel())
            .await,
        Err(WorkflowError::Stale)
    ));
    control(&project, 22, layer_lock(true)).await;
    for command in [
        set(),
        ToneControlCommand::Remove {
            layer_id: id(30),
            object_id: id(31),
        },
    ] {
        let view = snapshot(&project).await;
        assert!(view.tones[0].locked);
        assert!(matches!(
            project
                .preview_tone_control(
                    view.binding.clone(),
                    Some(command.clone()),
                    region(),
                    true,
                    BUDGET,
                    cancel()
                )
                .await,
            Err(WorkflowError::Locked)
        ));
        assert!(matches!(
            project
                .clone()
                .prepare_tone_control(
                    view.binding,
                    metadata(23, &project.info().await.unwrap()),
                    command,
                    BUDGET,
                    cancel()
                )
                .await,
            Err(WorkflowError::Locked)
        ));
    }
    control(&project, 24, layer_lock(false)).await;
    control(
        &project,
        25,
        DocumentControlCommand::SetObjectLocks {
            targets: DocumentControlTargets::Objects {
                object_ids: vec![id(31)],
            },
            locked: true,
        },
    )
    .await;
    let locked = snapshot(&project).await;
    let before = project.info().await.unwrap();
    assert!(locked.tones[0].locked);
    assert!(matches!(
        project
            .preview_tone_control(
                locked.binding.clone(),
                Some(set()),
                region(),
                true,
                BUDGET,
                cancel()
            )
            .await,
        Err(WorkflowError::Locked)
    ));
    assert!(matches!(
        project
            .clone()
            .prepare_tone_control(
                locked.binding,
                metadata(26, &before),
                set(),
                BUDGET,
                cancel()
            )
            .await,
        Err(WorkflowError::Locked)
    ));
    assert_eq!(project.info().await.unwrap(), before);
    project.close().await.unwrap();
}

#[tokio::test]
async fn invalid_settings_regions_color_consent_and_cancelled_work_leave_no_changes() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let project = create_image_project(create(&root.path().join("p")), cancel())
        .await
        .unwrap();
    let view = snapshot(&project).await;
    let initial = project.info().await.unwrap();
    let mut invalid = Vec::new();
    for value in [f64::NAN, f64::INFINITY, -1.01, 1.01] {
        let mut s = settings();
        s.brightness = value;
        invalid.push(s);
    }
    for value in [0.0, -1.0, f64::NAN] {
        let mut s = settings();
        s.gamma = value;
        invalid.push(s);
    }
    let mut s = settings();
    s.input_white = s.input_black;
    invalid.push(s);
    let mut s = settings();
    s.output_white = s.output_black - 0.1;
    invalid.push(s);
    for (index, settings) in invalid.into_iter().enumerate() {
        let command = ToneControlCommand::Set {
            layer_id: id(30),
            object_id: id(31),
            settings,
            enabled: true,
            opacity: 1.0,
        };
        assert!(matches!(
            project
                .clone()
                .prepare_tone_control(
                    view.binding.clone(),
                    metadata(40 + index as u8, &initial),
                    command.clone(),
                    BUDGET,
                    cancel()
                )
                .await,
            Err(WorkflowError::Invalid)
        ));
        assert!(matches!(
            project
                .preview_tone_control(
                    view.binding.clone(),
                    Some(command),
                    region(),
                    true,
                    BUDGET,
                    cancel()
                )
                .await,
            Err(WorkflowError::Invalid)
        ));
    }
    for invalid_region in [
        SelectionRegion {
            width: 513,
            ..region()
        },
        SelectionRegion { x: 63, ..region() },
        SelectionRegion {
            width: 0,
            ..region()
        },
    ] {
        assert!(matches!(
            project
                .preview_tone_control(
                    view.binding.clone(),
                    None,
                    invalid_region,
                    true,
                    BUDGET,
                    cancel()
                )
                .await,
            Err(WorkflowError::Invalid)
        ));
    }
    assert!(matches!(
        project
            .preview_tone_control(
                view.binding.clone(),
                None,
                region(),
                false,
                BUDGET,
                cancel()
            )
            .await,
        Err(WorkflowError::Invalid)
    ));
    assert!(matches!(
        project
            .preview_tone_control(view.binding.clone(), None, region(), true, 1, cancel())
            .await,
        Err(WorkflowError::Memory { .. })
    ));
    let token = cancel();
    token.cancel();
    assert!(matches!(
        project
            .preview_tone_control(view.binding, Some(set()), region(), true, BUDGET, token)
            .await,
        Err(WorkflowError::Cancelled)
    ));
    assert_eq!(project.info().await.unwrap(), initial);
    assert!(snapshot(&project).await.tones.is_empty());
    project.close().await.unwrap();
}

#[tokio::test]
async fn unsupported_hidden_adjustment_fails_explicitly_instead_of_becoming_an_empty_catalog() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let project = create_image_project(create(&root.path().join("p")), cancel())
        .await
        .unwrap();
    apply(&project, 20, set()).await;
    control(
        &project,
        21,
        DocumentControlCommand::SetObjectHidden {
            targets: DocumentControlTargets::Objects {
                object_ids: vec![id(31)],
            },
            hidden: true,
        },
    )
    .await;
    let info = project.info().await.unwrap();
    let binding = WorkflowBinding {
        project_id: info.project_id.clone(),
        document_id: id(2),
        host_seq: info.host_seq,
        state_hash: info.state_hash.clone(),
    };
    assert!(matches!(
        project
            .tone_controls_snapshot(id(2), BUDGET, cancel())
            .await,
        Err(WorkflowError::Invalid)
    ));
    assert!(matches!(
        project
            .preview_tone_control(binding.clone(), None, region(), true, BUDGET, cancel())
            .await,
        Err(WorkflowError::Invalid)
    ));
    assert!(matches!(
        project
            .clone()
            .prepare_tone_control(binding, metadata(22, &info), set(), BUDGET, cancel())
            .await,
        Err(WorkflowError::Invalid)
    ));
    assert_eq!(project.info().await.unwrap(), info);
    project.close().await.unwrap();
}

#[tokio::test]
async fn sixteen_bit_original_remains_exact_and_same_length_blob_corruption_is_detected() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let path = root.path().join("p");
    let image = vw_raster::DecodedImage {
        width: 64,
        height: 64,
        pixels: vw_raster::Pixels::Rgba16([16_384, 32_768, 49_152, 65_535].repeat(64 * 64)),
        icc: None,
        source_asset: AssetId::hash(b"synthetic-16-bit-tonal-fixture"),
        original_available: true,
        orientation_applied: 1,
    };
    let original = vw_raster::export(
        &image,
        &vw_raster::ExportRequest {
            format: vw_raster::ExportFormat::Png16,
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
    let mut options = create(&path);
    options.source = original.clone();
    let project = create_image_project(options, cancel()).await.unwrap();
    let view = snapshot(&project).await;
    assert_eq!(view.source_bit_depth, 16);
    let clean = preview(&project, view.binding.clone(), None).await;
    let draft = preview(&project, view.binding, Some(set())).await;
    assert_ne!(clean.rgba, draft.rgba);
    apply(&project, 20, set()).await;
    let blobs = vw_store::BlobStore::new(&path).unwrap();
    let source_id = AssetId::try_from(view.source_asset_id).unwrap();
    assert_eq!(blobs.read(&source_id).unwrap(), original);
    let blob_path = blobs.path(&source_id).unwrap();
    let mut damaged = original.clone();
    damaged[0] ^= 1;
    std::fs::write(&blob_path, &damaged).unwrap();
    let before = project.info().await.unwrap();
    assert!(matches!(
        project
            .preview_tone_control(
                snapshot(&project).await.binding,
                None,
                region(),
                true,
                BUDGET,
                cancel()
            )
            .await,
        Err(WorkflowError::Missing)
    ));
    assert_eq!(project.info().await.unwrap(), before);
    std::fs::write(blob_path, &original).unwrap();
    project.close().await.unwrap();
}

#[tokio::test]
async fn large_source_metadata_is_refused_before_decoding_even_for_a_one_pixel_tile() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let seed_path = root.path().join("seed");
    let seed = create_image_project(create(&seed_path), cancel())
        .await
        .unwrap();
    seed.close().await.unwrap();
    let seed_store = vw_store::ProjectStore::open(&seed_path).unwrap();
    let mut model = seed_store.project().clone();
    drop(seed_store);
    let asset = model.assets.values_mut().next().unwrap();
    asset.width = 4096;
    asset.height = 4096;
    let path = root.path().join("large");
    drop(
        vw_store::ProjectStore::create(&path, model, DeviceId::try_from(device()).unwrap(), TIME)
            .unwrap(),
    );
    // No original is installed: a successful memory refusal proves that this
    // request is rejected before opening or decoding the source blob.
    let project = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let view = snapshot(&project).await;
    assert!(matches!(
        project
            .preview_tone_control(
                view.binding,
                Some(set()),
                SelectionRegion {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1
                },
                true,
                BUDGET,
                cancel()
            )
            .await,
        Err(WorkflowError::Memory { .. })
    ));
    project.close().await.unwrap();
}

#[tokio::test]
async fn offline_draft_uses_the_visible_project_and_never_enqueues_a_preview_transaction() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let host_path = root.path().join("host");
    let host = create_image_project(create(&host_path), cancel())
        .await
        .unwrap();
    host.close().await.unwrap();
    let host_store = vw_store::ProjectStore::open(&host_path).unwrap();
    let accepted = host_store.revision().unwrap();
    let checkpoint = host_store.checkpoint_bytes().unwrap();
    let host_device = host_store.host_device().clone();
    drop(host_store);
    let path = root.path().join("client");
    drop(
        vw_store::ProjectStore::create_authenticated_checkpoint(
            &path,
            &checkpoint,
            &host_device,
            &DeviceId::from_bytes([9; 16]),
            TIME,
        )
        .unwrap(),
    );
    vw_store::BlobStore::new(&path)
        .unwrap()
        .put(&source())
        .unwrap();
    let project = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    apply(&project, 20, set()).await;
    let view = snapshot(&project).await;
    let before = project.info().await.unwrap();
    assert_eq!(before.host_seq, accepted.host_seq);
    let current = preview(&project, view.binding.clone(), None).await;
    let remove = preview(
        &project,
        view.binding.clone(),
        Some(ToneControlCommand::Remove {
            layer_id: id(30),
            object_id: id(31),
        }),
    )
    .await;
    assert_ne!(current.rgba, remove.rgba);
    assert_eq!(project.info().await.unwrap(), before);
    assert_eq!(snapshot(&project).await.binding, view.binding);
    project.close().await.unwrap();
    let store = vw_store::ProjectStore::open(&path).unwrap();
    let pending = store.pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].base_revision.as_ref(), Some(&accepted));
    assert_eq!(store.accepted_transactions().count(), 0);
}
