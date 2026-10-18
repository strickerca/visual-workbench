//! Canonical store/ops/pixel contracts. Source-only until coordinator validation.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use std::{io::Cursor, sync::Arc};
use support::*;
use vw_core::*;
use vw_model::{AssetId, DeviceId, Id};

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
fn settings() -> ImageFrameSettings {
    ImageFrameSettings {
        crop_x: 8,
        crop_y: 6,
        crop_width: 32,
        crop_height: 24,
        clockwise_degrees: 90,
        output_width: 12,
        output_height: 16,
    }
}
fn set() -> ImageFrameCommand {
    ImageFrameCommand::Set {
        layer_id: id(30),
        object_id: id(31),
        settings: settings(),
        enabled: true,
    }
}
fn full_output() -> SelectionRegion {
    SelectionRegion {
        x: 0,
        y: 0,
        width: 12,
        height: 16,
    }
}
fn original(depth: u8) -> Vec<u8> {
    let samples: Vec<u16> = (0..64u16)
        .flat_map(|y| (0..64u16).flat_map(move |x| [x * 997, y * 991, (x + y) * 509, 65535]))
        .collect();
    let image = vw_raster::DecodedImage {
        width: 64,
        height: 64,
        pixels: if depth == 16 {
            vw_raster::Pixels::Rgba16(samples)
        } else {
            vw_raster::Pixels::Rgba8(samples.iter().map(|v| (v / 257) as u8).collect())
        },
        icc: None,
        source_asset: AssetId::hash(b"synthetic-image-frame-fixture"),
        original_available: true,
        orientation_applied: 1,
    };
    vw_raster::export(
        &image,
        &vw_raster::ExportRequest {
            format: if depth == 16 {
                vw_raster::ExportFormat::Png16
            } else {
                vw_raster::ExportFormat::Png8
            },
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
    .bytes
}
async fn create_fixture(path: &std::path::Path, depth: u8) -> Arc<ProjectSession> {
    let mut value = create(path);
    value.source = original(depth);
    create_image_project(value, cancel()).await.unwrap()
}
async fn snapshot(project: &Arc<ProjectSession>) -> ImageFramesSnapshot {
    project
        .image_frames_snapshot(id(2), BUDGET, cancel())
        .await
        .unwrap()
}
async fn prepare(
    project: &Arc<ProjectSession>,
    n: u8,
    command: ImageFrameCommand,
) -> Arc<WorkflowPlan> {
    let view = snapshot(project).await;
    project
        .clone()
        .prepare_image_frame(
            view.binding,
            metadata(n, &project.info().await.unwrap()),
            command,
            BUDGET,
            cancel(),
        )
        .await
        .unwrap()
}
async fn apply(project: &Arc<ProjectSession>, n: u8, command: ImageFrameCommand) {
    let plan = prepare(project, n, command).await;
    plan.commit(cancel()).await.unwrap();
    plan.dispose();
}
async fn output(
    project: &Arc<ProjectSession>,
    command: Option<ImageFrameCommand>,
    marked: bool,
) -> ImageFramePreview {
    project
        .image_frame_preview(
            snapshot(project).await.binding,
            command,
            full_output(),
            marked,
            true,
            BUDGET,
            cancel(),
        )
        .await
        .unwrap()
}
async fn editing(project: &Arc<ProjectSession>, region: SelectionRegion) -> ImageEditingPreview {
    project
        .image_editing_preview(
            snapshot(project).await.binding,
            region,
            true,
            BUDGET,
            cancel(),
        )
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
fn locked(value: bool) -> DocumentControlCommand {
    DocumentControlCommand::UpdateLayer {
        update: LayerControlUpdate {
            layer_id: id(30),
            name: None,
            visible: None,
            locked: Some(value),
            opacity: None,
            blend: None,
        },
    }
}
async fn tone(project: &Arc<ProjectSession>, n: u8) {
    let view = snapshot(project).await;
    let plan = project
        .clone()
        .prepare_tone_control(
            view.binding,
            metadata(n, &project.info().await.unwrap()),
            ToneControlCommand::Set {
                layer_id: id(40),
                object_id: id(41),
                settings: ImageToneSettings {
                    brightness: -0.2,
                    contrast: 0.2,
                    input_black: 0.0,
                    input_white: 1.0,
                    gamma: 1.2,
                    output_black: 0.0,
                    output_white: 1.0,
                },
                enabled: true,
                opacity: 0.5,
            },
            BUDGET,
            cancel(),
        )
        .await
        .unwrap();
    plan.commit(cancel()).await.unwrap();
    plan.dispose();
}
async fn annotate(project: &Arc<ProjectSession>, n: u8) {
    let now = project.info().await.unwrap();
    let commands = [(50, 48.0, 48.0), (51, 12.0, 10.0)]
        .into_iter()
        .map(|(object, x, y)| EditCommand::Create {
            object_id: id(object),
            layer_id: id(3),
            shape: NewShape::Rectangle {
                rectangle: QueryRect {
                    x,
                    y,
                    width: 10.0,
                    height: 10.0,
                },
            },
            style: ObjectStyle {
                rgba: 0x991122ff,
                width: 1.0,
                screen_constant_width: false,
                fill: Some(0xee1122ff),
            },
            transform: Transform {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                e: 0.0,
                f: 0.0,
            },
        })
        .collect();
    project
        .apply_edit(edit(n, now.next_lamport), commands, cancel())
        .await
        .unwrap();
}
fn png_metadata(bytes: &[u8]) -> vw_raster::ExportMetadata {
    let reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    let text = reader
        .info()
        .utf8_text
        .iter()
        .find(|v| v.keyword == "VisualWorkbench")
        .unwrap()
        .get_text()
        .unwrap();
    serde_json::from_str(&text).unwrap()
}
fn tiff_metadata(bytes: &[u8]) -> vw_raster::ExportMetadata {
    assert_eq!(&bytes[..4], b"II*\0");
    let ifd = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let count = u16::from_le_bytes(bytes[ifd..ifd + 2].try_into().unwrap()) as usize;
    let at = (0..count)
        .map(|n| ifd + 2 + n * 12)
        .find(|&at| u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()) == 270)
        .unwrap();
    let length = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
    let offset = u32::from_le_bytes(bytes[at + 8..at + 12].try_into().unwrap()) as usize;
    assert_eq!(bytes[offset + length - 1], 0);
    serde_json::from_slice(&bytes[offset..offset + length - 1]).unwrap()
}

#[tokio::test]
async fn draft_prepare_commit_retry_undo_disable_and_remove_preserve_originals_and_d_extent() {
    let _serial = TESTS.lock().await;
    for depth in [8, 16] {
        let root = temp();
        let path = root.path().join("p");
        let project = create_fixture(&path, depth).await;
        let initial = project.info().await.unwrap();
        let view = snapshot(&project).await;
        assert_eq!(
            (view.source_width, view.source_height, view.source_bit_depth),
            (64, 64, depth)
        );
        assert!(view.frame.is_none());
        let clean = output(&project, None, true).await;
        let draft = output(&project, Some(set()), true).await;
        assert_eq!(draft.binding, view.binding);
        assert_eq!(draft.frame.as_ref(), Some(&settings()));
        assert_eq!((draft.output_width, draft.output_height), (12, 16));
        assert_ne!(draft.rgba, clean.rgba);
        assert_eq!(project.info().await.unwrap(), initial);
        assert!(snapshot(&project).await.frame.is_none());
        let plan = prepare(&project, 20, set()).await;
        assert_eq!(plan.describe().unwrap().operation_count, 3);
        assert_eq!(project.info().await.unwrap(), initial);
        let committed = plan.commit(cancel()).await.unwrap();
        let retry = plan.commit(cancel()).await.unwrap();
        assert!(!committed.duplicate);
        assert!(retry.duplicate);
        assert_eq!(committed.revision, retry.revision);
        plan.dispose();
        assert_eq!(output(&project, None, true).await.rgba, draft.rgba);
        let doc = project.document_snapshot(id(2), cancel()).await.unwrap();
        assert_eq!(
            (doc.width, doc.height, doc.bit_depth),
            (64, 64, u32::from(depth))
        );
        let now = project.info().await.unwrap();
        project
            .undo_redo(edit(21, now.next_lamport), false, cancel())
            .await
            .unwrap();
        assert!(snapshot(&project).await.frame.is_none());
        assert_eq!(output(&project, None, true).await.rgba, clean.rgba);
        let now = project.info().await.unwrap();
        project
            .undo_redo(edit(22, now.next_lamport), true, cancel())
            .await
            .unwrap();
        assert_eq!(output(&project, None, true).await.rgba, draft.rgba);
        let disabled = ImageFrameCommand::Set {
            layer_id: id(30),
            object_id: id(31),
            settings: settings(),
            enabled: false,
        };
        apply(&project, 23, disabled).await;
        assert!(!snapshot(&project).await.frame.unwrap().enabled);
        let inactive = output(&project, None, true).await;
        assert!(inactive.frame.is_none());
        assert_eq!((inactive.output_width, inactive.output_height), (64, 64));
        assert_eq!(inactive.rgba, clean.rgba);
        apply(&project, 24, set()).await;
        let remove = ImageFrameCommand::Remove {
            layer_id: id(30),
            object_id: id(31),
        };
        assert_eq!(
            output(&project, Some(remove.clone()), true).await.rgba,
            clean.rgba
        );
        assert!(snapshot(&project).await.frame.is_some());
        apply(&project, 25, remove).await;
        assert!(snapshot(&project).await.frame.is_none());
        project.close().await.unwrap();
        let store = vw_store::ProjectStore::open(&path).unwrap();
        assert_eq!(store.project().assets.len(), 1);
        assert!(store.project().objects.is_empty());
        let source_id = store.project().assets.keys().next().unwrap();
        assert_eq!(
            vw_store::BlobStore::new(&path)
                .unwrap()
                .read(source_id)
                .unwrap(),
            original(depth)
        );
        drop(store);
        let reopened = open_project(path.to_string_lossy().into(), cancel())
            .await
            .unwrap();
        assert!(snapshot(&reopened).await.frame.is_none());
        reopened.close().await.unwrap();
    }
}

#[tokio::test]
async fn editing_background_and_tiles_include_canonical_tones_annotations_and_d_outside_crop() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let path = root.path().join("p");
    let project = create_fixture(&path, 8).await;
    let outside = SelectionRegion {
        x: 46,
        y: 46,
        width: 16,
        height: 16,
    };
    let untouched = editing(&project, outside.clone()).await;
    tone(&project, 20).await;
    let toned = editing(&project, outside.clone()).await;
    assert_ne!(toned.rgba, untouched.rgba);
    annotate(&project, 21).await;
    let annotated = editing(&project, outside.clone()).await;
    assert_ne!(annotated.rgba, toned.rgba);
    apply(&project, 22, set()).await;
    let after = editing(&project, outside).await;
    assert_eq!(after.rgba, annotated.rgba);
    let current = snapshot(&project).await;
    let whole = project
        .image_editing_background(current.binding.clone(), true, BUDGET, cancel())
        .await
        .unwrap();
    assert_eq!(whole.binding, current.binding);
    assert_eq!(
        (whole.width, whole.height, whole.source_bit_depth),
        (64, 64, 8)
    );
    assert_eq!(whole.rgba.len(), 64 * 64 * 4);
    assert!(!whole.icc_profile.is_empty());
    for y in [0, 32] {
        for x in [0, 32] {
            let tile = editing(
                &project,
                SelectionRegion {
                    x,
                    y,
                    width: 32,
                    height: 32,
                },
            )
            .await;
            let expected: Vec<u8> = (y..y + 32)
                .flat_map(|row| {
                    let start = ((row * 64 + x) * 4) as usize;
                    whole.rgba[start..start + 32 * 4].iter().copied()
                })
                .collect();
            assert_eq!(tile.rgba, expected);
        }
    }
    assert!(matches!(
        project
            .image_frame_preview(
                current.binding,
                None,
                SelectionRegion {
                    x: 46,
                    y: 46,
                    width: 16,
                    height: 16
                },
                true,
                true,
                BUDGET,
                cancel()
            )
            .await,
        Err(WorkflowError::Invalid)
    ));
    assert_ne!(
        output(&project, None, false).await.rgba,
        output(&project, None, true).await.rgba
    );
    let rendered = project.render_list(id(2), None, cancel()).await.unwrap();
    let rectangle = rendered
        .items
        .iter()
        .find(|v| v.object_id == id(50))
        .unwrap();
    assert!(
        matches!(rectangle.shape, DrawShape::Rectangle { rectangle } if rectangle.x == 48.0 && rectangle.y == 48.0)
    );
    project.close().await.unwrap();
    let store = vw_store::ProjectStore::open(&path).unwrap();
    let object = &store.project().objects[&Id::try_from(id(50)).unwrap()].state;
    assert_eq!(object.transform.as_ref().unwrap().e, 0.0);
    assert!(
        matches!(object.shape, Some(vw_proto::v1::object_state::Shape::Rect(ref v)) if v.x == 48.0 && v.y == 48.0)
    );
    let source = store.project().assets.keys().next().unwrap();
    assert_eq!(
        vw_store::BlobStore::new(&path)
            .unwrap()
            .read(source)
            .unwrap(),
        original(8)
    );
}

#[tokio::test]
async fn png_and_tiff_exports_match_preview_and_embed_true_source_extent_and_output_map() {
    let _serial = TESTS.lock().await;
    for depth in [8, 16] {
        let root = temp();
        let path = root.path().join("p");
        let project = create_fixture(&path, depth).await;
        tone(&project, 20).await;
        annotate(&project, 21).await;
        apply(&project, 22, set()).await;
        let expected = output(&project, None, true).await;
        for format in [
            if depth == 16 {
                ImageFormat::Png16
            } else {
                ImageFormat::Png8
            },
            if depth == 16 {
                ImageFormat::Tiff16
            } else {
                ImageFormat::Tiff8
            },
        ] {
            let mut request = options();
            request.format = format.clone();
            request.convert_to_srgb = true;
            let result = project.export_image(request, cancel()).await.unwrap();
            let metadata: vw_raster::ExportMetadata =
                serde_json::from_str(&result.metadata_json).unwrap();
            assert_eq!(
                (
                    metadata.source_width,
                    metadata.source_height,
                    metadata.output_width,
                    metadata.output_height
                ),
                (64, 64, 12, 16)
            );
            assert_eq!(metadata.source_bit_depth, depth);
            assert_eq!(metadata.source_asset, expected.source_asset_id);
            let frame = metadata.image_output.unwrap();
            assert_eq!(
                (
                    frame.crop.x,
                    frame.crop.y,
                    frame.crop.width,
                    frame.crop.height,
                    frame.clockwise_degrees
                ),
                (8, 6, 32, 24, 90)
            );
            let embedded = if matches!(format, ImageFormat::Png8 | ImageFormat::Png16) {
                png_metadata(&result.bytes)
            } else {
                tiff_metadata(&result.bytes)
            };
            assert_eq!((embedded.source_width, embedded.source_height), (64, 64));
            assert_eq!(embedded.image_output, Some(frame));
            let decoded = if matches!(format, ImageFormat::Tiff8 | ImageFormat::Tiff16) {
                vw_formats::decode(&result.bytes, vw_formats::Limits::default(), &|| false)
                    .unwrap()
                    .image
            } else {
                vw_raster::decode(&result.bytes, vw_raster::DecodeLimits::default()).unwrap()
            };
            assert_eq!(
                (decoded.width, decoded.height, decoded.pixels.bit_depth()),
                (12, 16, depth)
            );
            assert_eq!(decoded.pixels.rgba8(), expected.rgba);
        }
        let mut bmp = options();
        bmp.format = ImageFormat::Bmp8;
        bmp.allow_depth_reduction = true;
        assert!(project.export_image(bmp, cancel()).await.is_err());
        project.close().await.unwrap();
    }
}

#[tokio::test]
async fn stale_invalid_cancelled_color_budget_and_locked_requests_cannot_change_state() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let project = create_fixture(&root.path().join("p"), 8).await;
    let stale = snapshot(&project).await.binding;
    apply(&project, 20, set()).await;
    let before = project.info().await.unwrap();
    assert!(matches!(
        project
            .image_frame_preview(
                stale.clone(),
                None,
                full_output(),
                true,
                true,
                BUDGET,
                cancel()
            )
            .await,
        Err(WorkflowError::Stale)
    ));
    assert!(matches!(
        project
            .image_editing_background(stale, true, BUDGET, cancel())
            .await,
        Err(WorkflowError::Stale)
    ));
    let binding = snapshot(&project).await.binding;
    let mut invalid = settings();
    invalid.crop_width = 0;
    assert!(matches!(
        project
            .clone()
            .prepare_image_frame(
                binding.clone(),
                metadata(21, &before),
                ImageFrameCommand::Set {
                    layer_id: id(30),
                    object_id: id(31),
                    settings: invalid,
                    enabled: true
                },
                BUDGET,
                cancel()
            )
            .await,
        Err(WorkflowError::Invalid)
    ));
    assert!(matches!(
        project
            .image_frame_preview(
                binding.clone(),
                None,
                SelectionRegion {
                    width: 513,
                    ..full_output()
                },
                true,
                true,
                BUDGET,
                cancel()
            )
            .await,
        Err(WorkflowError::Invalid)
    ));
    assert!(matches!(
        project
            .image_frame_preview(
                binding.clone(),
                None,
                full_output(),
                true,
                false,
                BUDGET,
                cancel()
            )
            .await,
        Err(WorkflowError::Invalid)
    ));
    assert!(matches!(
        project
            .image_editing_background(binding.clone(), false, BUDGET, cancel())
            .await,
        Err(WorkflowError::Invalid)
    ));
    assert!(matches!(
        project
            .image_frame_preview(
                binding.clone(),
                Some(set()),
                full_output(),
                true,
                true,
                1,
                cancel()
            )
            .await,
        Err(WorkflowError::Memory { .. })
    ));
    assert!(matches!(
        project
            .image_editing_background(binding.clone(), true, 1, cancel())
            .await,
        Err(WorkflowError::Memory { .. })
    ));
    let cancelled = cancel();
    cancelled.cancel();
    assert!(matches!(
        project
            .image_frame_preview(
                binding.clone(),
                Some(set()),
                full_output(),
                true,
                true,
                BUDGET,
                cancelled
            )
            .await,
        Err(WorkflowError::Cancelled)
    ));
    assert_eq!(project.info().await.unwrap(), before);
    control(&project, 22, locked(true)).await;
    let locked_view = snapshot(&project).await;
    assert!(locked_view.frame.unwrap().locked);
    for command in [
        set(),
        ImageFrameCommand::Remove {
            layer_id: id(30),
            object_id: id(31),
        },
    ] {
        assert!(matches!(
            project
                .image_frame_preview(
                    locked_view.binding.clone(),
                    Some(command.clone()),
                    full_output(),
                    true,
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
                .prepare_image_frame(
                    locked_view.binding.clone(),
                    metadata(23, &project.info().await.unwrap()),
                    command,
                    BUDGET,
                    cancel()
                )
                .await,
            Err(WorkflowError::Locked)
        ));
    }
    control(&project, 24, locked(false)).await;
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
    let object_lock = snapshot(&project).await;
    assert!(matches!(
        project
            .image_frame_preview(
                object_lock.binding,
                Some(set()),
                full_output(),
                true,
                true,
                BUDGET,
                cancel()
            )
            .await,
        Err(WorkflowError::Locked)
    ));
    project.close().await.unwrap();
}

#[tokio::test]
async fn offline_draft_reads_visible_frames_and_never_persists_its_private_host() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let host_path = root.path().join("host");
    let host = create_fixture(&host_path, 8).await;
    host.close().await.unwrap();
    let store = vw_store::ProjectStore::open(&host_path).unwrap();
    let accepted = store.revision().unwrap();
    let checkpoint = store.checkpoint_bytes().unwrap();
    let device = store.host_device().clone();
    drop(store);
    let path = root.path().join("client");
    drop(
        vw_store::ProjectStore::create_authenticated_checkpoint(
            &path,
            &checkpoint,
            &device,
            &DeviceId::from_bytes([9; 16]),
            TIME,
        )
        .unwrap(),
    );
    vw_store::BlobStore::new(&path)
        .unwrap()
        .put(&original(8))
        .unwrap();
    let project = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    apply(&project, 20, set()).await;
    let before = project.info().await.unwrap();
    let configured = output(&project, None, true).await;
    let removed = output(
        &project,
        Some(ImageFrameCommand::Remove {
            layer_id: id(30),
            object_id: id(31),
        }),
        true,
    )
    .await;
    assert_ne!(configured.rgba, removed.rgba);
    assert_eq!((removed.output_width, removed.output_height), (64, 64));
    assert_eq!(project.info().await.unwrap(), before);
    project.close().await.unwrap();
    let store = vw_store::ProjectStore::open(&path).unwrap();
    let pending = store.pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].base_revision.as_ref(), Some(&accepted));
    assert_eq!(store.accepted_transactions().count(), 0);
}

#[tokio::test]
async fn huge_output_does_not_truncate_d_tiles_and_can_be_removed_in_a_small_draft() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let project = create_fixture(&root.path().join("p"), 8).await;
    let mut huge = settings();
    huge.output_width = 8192;
    huge.output_height = 6000;
    apply(
        &project,
        20,
        ImageFrameCommand::Set {
            layer_id: id(30),
            object_id: id(31),
            settings: huge,
            enabled: true,
        },
    )
    .await;
    let binding = snapshot(&project).await.binding;
    assert!(matches!(
        project
            .image_frame_preview(
                binding.clone(),
                None,
                SelectionRegion {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1
                },
                true,
                true,
                BUDGET,
                cancel()
            )
            .await,
        Err(WorkflowError::Memory { .. })
    ));
    let tile = editing(
        &project,
        SelectionRegion {
            x: 46,
            y: 46,
            width: 16,
            height: 16,
        },
    )
    .await;
    assert_eq!(tile.rgba.len(), 16 * 16 * 4);
    let before = project.info().await.unwrap();
    let remove = output(
        &project,
        Some(ImageFrameCommand::Remove {
            layer_id: id(30),
            object_id: id(31),
        }),
        true,
    )
    .await;
    assert_eq!((remove.output_width, remove.output_height), (64, 64));
    assert_eq!(project.info().await.unwrap(), before);
    project.close().await.unwrap();
}

#[tokio::test]
async fn changed_original_hash_is_rejected_before_decode_without_modifying_the_revision() {
    let _serial = TESTS.lock().await;
    let root = temp();
    let path = root.path().join("p");
    let project = create_fixture(&path, 8).await;
    let view = snapshot(&project).await;
    let before = project.info().await.unwrap();
    let blobs = vw_store::BlobStore::new(&path).unwrap();
    let source_id = AssetId::try_from(view.source_asset_id).unwrap();
    let path = blobs.path(&source_id).unwrap();
    let bytes = original(8);
    let mut corrupt = bytes.clone();
    corrupt[0] ^= 1;
    std::fs::write(&path, &corrupt).unwrap();
    assert!(matches!(
        project
            .image_frame_preview(
                view.binding.clone(),
                Some(set()),
                full_output(),
                true,
                true,
                BUDGET,
                cancel()
            )
            .await,
        Err(WorkflowError::Missing)
    ));
    assert!(matches!(
        project
            .image_editing_background(view.binding, true, BUDGET, cancel())
            .await,
        Err(WorkflowError::Missing)
    ));
    assert_eq!(project.info().await.unwrap(), before);
    std::fs::write(path, bytes).unwrap();
    project.close().await.unwrap();
}
