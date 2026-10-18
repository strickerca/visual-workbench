#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use std::{io::Cursor, path::Path, sync::Arc};
use support::*;
use vw_core::*;
use vw_model::AssetId;

const MEMORY: u64 = 256 * 1024 * 1024;
fn temp() -> tempfile::TempDir {
    if cfg!(target_os = "android") {
        tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()
    } else {
        tempfile::tempdir().unwrap()
    }
}
async fn fixture(path: &Path) -> Arc<ProjectSession> {
    create_image_project(create(path), cancel()).await.unwrap()
}
fn rectangle(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    combine: SelectionCombine,
) -> SelectionOperation {
    SelectionOperation::Rectangle {
        rectangle: QueryRect {
            x,
            y,
            width,
            height,
        },
        combine,
    }
}
async fn request(
    project: &ProjectSession,
    n: u8,
    target: SelectionTarget,
    operation: SelectionOperation,
) -> SelectionEdit {
    let doc = project.selection_document(id(2), cancel()).await.unwrap();
    SelectionEdit {
        binding: doc.binding,
        transaction_id: id(n),
        device_id: device(),
        lamport: doc.revision.next_lamport,
        created_at_ms: TIME + i64::from(n),
        target,
        operation,
        memory_budget_bytes: MEMORY,
    }
}
async fn new_rectangle(
    project: &ProjectSession,
    n: u8,
    object: u8,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> SelectionReceipt {
    let options = request(
        project,
        n,
        SelectionTarget::New {
            object_id: id(object),
            layer_id: id(20),
        },
        rectangle(x, y, w, h, SelectionCombine::Add),
    )
    .await;
    project.apply_selection(options, cancel()).await.unwrap()
}
async fn refine(
    project: &ProjectSession,
    n: u8,
    selection: SelectionVersion,
    operation: SelectionOperation,
) -> SelectionReceipt {
    let options = request(
        project,
        n,
        SelectionTarget::Existing { selection },
        operation,
    )
    .await;
    project.apply_selection(options, cancel()).await.unwrap()
}
async fn pixels(project: &ProjectSession, receipt: &SelectionReceipt) -> Vec<u8> {
    let tile = project
        .selection_tiles(
            receipt.snapshot.binding.clone(),
            receipt.snapshot.selection.clone(),
            vec![SelectionRegion {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            }],
            MEMORY,
            cancel(),
        )
        .await
        .unwrap();
    tile.tiles[0].coverage.clone()
}
fn output(
    dir: &Path,
    receipt: &SelectionReceipt,
    kind: SelectionExportKind,
    name: &str,
) -> SelectionExportOptions {
    SelectionExportOptions {
        binding: receipt.snapshot.binding.clone(),
        selection: receipt.snapshot.selection.clone(),
        kind,
        region: None,
        work_directory: dir.to_string_lossy().into(),
        output_path: dir.join(name).to_string_lossy().into(),
        memory_budget_bytes: MEMORY,
        max_encoded_bytes: 64 * 1024 * 1024,
    }
}
fn decode_gray(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    let mut out = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut out).unwrap();
    assert_eq!(info.color_type, png::ColorType::Grayscale);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    out.truncate(info.buffer_size());
    (info.width, info.height, out)
}

#[tokio::test]
async fn rectangle_is_exact_document_coverage_and_one_durable_transaction() {
    let dir = temp();
    let path = dir.path().join("project");
    let project = fixture(&path).await;
    let before = project.selection_document(id(2), cancel()).await.unwrap();
    let original = source();
    let receipt = new_rectangle(&project, 30, 21, 2.0, 3.0, 4.0, 5.0).await;
    assert_eq!(receipt.revision.host_seq, before.revision.host_seq + 1);
    assert_eq!(receipt.snapshot.selection.version, 1);
    let actual = pixels(&project, &receipt).await;
    for y in 0..64 {
        for x in 0..64 {
            assert_eq!(
                actual[y * 64 + x],
                if (2..6).contains(&x) && (3..8).contains(&y) {
                    255
                } else {
                    0
                }
            );
        }
    }
    assert_eq!(
        receipt.snapshot.nonzero_bounds,
        Some(SelectionRegion {
            x: 2,
            y: 3,
            width: 4,
            height: 5
        })
    );
    let blobs = vw_store::BlobStore::new(&path).unwrap();
    assert_eq!(blobs.read(&AssetId::hash(&original)).unwrap(), original);
    let mask_bytes = blobs
        .read(&AssetId::try_from(receipt.snapshot.selection.asset_id.clone()).unwrap())
        .unwrap();
    assert_eq!(decode_gray(&mask_bytes).2, actual);
    project.close().await.unwrap();
    let reopened = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let observed = reopened
        .selection_snapshot(receipt.snapshot.binding.clone(), id(21), MEMORY, cancel())
        .await
        .unwrap();
    assert_eq!(observed.selection, receipt.snapshot.selection);
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn lasso_matches_independent_axis_aligned_rectangle_answers() {
    let dir = temp();
    let project = fixture(&dir.path().join("project")).await;
    let options = request(
        &project,
        31,
        SelectionTarget::New {
            object_id: id(21),
            layer_id: id(20),
        },
        SelectionOperation::Lasso {
            points: vec![
                Point { x: 3.0, y: 5.0 },
                Point { x: 7.0, y: 5.0 },
                Point { x: 7.0, y: 8.0 },
                Point { x: 3.0, y: 8.0 },
            ],
            combine: SelectionCombine::Add,
        },
    )
    .await;
    let receipt = project.apply_selection(options, cancel()).await.unwrap();
    let actual = pixels(&project, &receipt).await;
    assert_eq!(actual.iter().filter(|&&v| v == 255).count(), 12);
    for y in 0..64 {
        for x in 0..64 {
            assert_eq!(
                actual[y * 64 + x],
                if (3..7).contains(&x) && (5..8).contains(&y) {
                    255
                } else {
                    0
                }
            );
        }
    }
    project.close().await.unwrap();
}

#[tokio::test]
async fn subtractive_paint_erases_mask_only_and_undo_restores_its_exact_asset() {
    let dir = temp();
    let project = fixture(&dir.path().join("project")).await;
    let initial = new_rectangle(&project, 30, 21, 0.0, 0.0, 64.0, 64.0).await;
    let erased = refine(
        &project,
        31,
        initial.snapshot.selection.clone(),
        SelectionOperation::Paint {
            points: vec![Point { x: 16.5, y: 16.5 }],
            radius: 2.0,
            opacity: 255,
            combine: SelectionCombine::Subtract,
        },
    )
    .await;
    let actual = pixels(&project, &erased).await;
    assert_eq!(actual[16 * 64 + 16], 0);
    assert_eq!(actual[0], 255);
    assert_eq!(erased.snapshot.selection.version, 2);
    let mut undo = edit(32, erased.revision.next_lamport);
    let revision = project
        .undo_redo(undo.clone(), false, cancel())
        .await
        .unwrap();
    let doc = project.selection_document(id(2), cancel()).await.unwrap();
    assert_eq!(doc.revision, revision);
    let restored = project
        .selection_snapshot(doc.binding, id(21), MEMORY, cancel())
        .await
        .unwrap();
    assert_eq!(restored.selection, initial.snapshot.selection);
    undo.transaction_id = id(33);
    undo.lamport = revision.next_lamport;
    let redone = project.undo_redo(undo, true, cancel()).await.unwrap();
    let doc = project.selection_document(id(2), cancel()).await.unwrap();
    let restored = project
        .selection_snapshot(doc.binding, id(21), MEMORY, cancel())
        .await
        .unwrap();
    assert_eq!(doc.revision, redone);
    assert_eq!(restored.selection, erased.snapshot.selection);
    project.close().await.unwrap();
}

#[tokio::test]
async fn feather_and_morphology_have_independent_disk_kernel_answers() {
    let dir = temp();
    let project = fixture(&dir.path().join("project")).await;
    let initial = new_rectangle(&project, 30, 21, 8.0, 8.0, 1.0, 1.0).await;
    let feather = refine(
        &project,
        31,
        initial.snapshot.selection,
        SelectionOperation::Feather { radius: 1 },
    )
    .await;
    let values = pixels(&project, &feather).await;
    for y in 0..64 {
        for x in 0..64 {
            let distance = (x as i32 - 8).abs() + (y as i32 - 8).abs();
            assert_eq!(values[y * 64 + x], if distance <= 1 { 51 } else { 0 });
        }
    }
    let square = new_rectangle(&project, 32, 22, 20.0, 20.0, 5.0, 5.0).await;
    let shrunk = refine(
        &project,
        33,
        square.snapshot.selection,
        SelectionOperation::Shrink { radius: 1 },
    )
    .await;
    assert_eq!(
        pixels(&project, &shrunk)
            .await
            .iter()
            .filter(|&&v| v == 255)
            .count(),
        9
    );
    let expanded = refine(
        &project,
        34,
        feather.snapshot.selection,
        SelectionOperation::Expand { radius: 1 },
    )
    .await;
    let values = pixels(&project, &expanded).await;
    assert_eq!(values[8 * 64 + 10], 51);
    assert_eq!(values[8 * 64 + 11], 0);
    project.close().await.unwrap();
}

#[tokio::test]
async fn boolean_operations_and_invert_keep_exact_coverage() {
    let dir = temp();
    let project = fixture(&dir.path().join("project")).await;
    let a = new_rectangle(&project, 30, 21, 0.0, 0.0, 8.0, 4.0).await;
    let b = new_rectangle(&project, 31, 22, 4.0, 0.0, 8.0, 4.0).await;
    let intersection = refine(
        &project,
        32,
        a.snapshot.selection,
        SelectionOperation::Combine {
            other: b.snapshot.selection,
            combine: SelectionCombine::Intersect,
        },
    )
    .await;
    assert_eq!(
        pixels(&project, &intersection)
            .await
            .iter()
            .filter(|&&v| v == 255)
            .count(),
        16
    );
    let invert = refine(
        &project,
        33,
        intersection.snapshot.selection,
        SelectionOperation::Invert,
    )
    .await;
    assert_eq!(
        pixels(&project, &invert)
            .await
            .iter()
            .filter(|&&v| v == 0)
            .count(),
        16
    );
    project.close().await.unwrap();
}

#[tokio::test]
async fn stale_revision_source_and_selection_version_never_publish_an_edit() {
    let dir = temp();
    let project = fixture(&dir.path().join("project")).await;
    let first = new_rectangle(&project, 30, 21, 1.0, 1.0, 4.0, 4.0).await;
    let stale = request(
        &project,
        31,
        SelectionTarget::Existing {
            selection: first.snapshot.selection.clone(),
        },
        SelectionOperation::Invert,
    )
    .await;
    let current = refine(
        &project,
        32,
        first.snapshot.selection.clone(),
        SelectionOperation::Expand { radius: 1 },
    )
    .await;
    assert!(matches!(
        project.apply_selection(stale, cancel()).await,
        Err(SelectionError::Conflict)
    ));
    let mut wrong = request(
        &project,
        33,
        SelectionTarget::Existing {
            selection: first.snapshot.selection,
        },
        SelectionOperation::Invert,
    )
    .await;
    assert!(matches!(
        project.apply_selection(wrong.clone(), cancel()).await,
        Err(SelectionError::Conflict)
    ));
    wrong.target = SelectionTarget::Existing {
        selection: current.snapshot.selection,
    };
    wrong.binding.source_asset_id = AssetId::hash(b"different source").to_string();
    assert!(matches!(
        project.apply_selection(wrong, cancel()).await,
        Err(SelectionError::Conflict)
    ));
    assert_eq!(project.info().await.unwrap(), current.revision);
    project.close().await.unwrap();
}

#[tokio::test]
async fn cancel_budget_and_bad_geometry_are_atomic_refusals() {
    let dir = temp();
    let project = fixture(&dir.path().join("project")).await;
    let initial = project.info().await.unwrap();
    let options = request(
        &project,
        30,
        SelectionTarget::New {
            object_id: id(21),
            layer_id: id(20),
        },
        rectangle(0.0, 0.0, 8.0, 8.0, SelectionCombine::Add),
    )
    .await;
    let stopped = cancel();
    stopped.cancel();
    assert!(matches!(
        project.apply_selection(options.clone(), stopped).await,
        Err(SelectionError::Cancelled)
    ));
    let mut small = options.clone();
    small.memory_budget_bytes = 1;
    assert!(matches!(
        project.apply_selection(small, cancel()).await,
        Err(SelectionError::Memory { .. })
    ));
    let mut invalid = options;
    invalid.operation = rectangle(f64::NAN, 0.0, 1.0, 1.0, SelectionCombine::Add);
    assert!(project.apply_selection(invalid, cancel()).await.is_err());
    assert_eq!(project.info().await.unwrap(), initial);
    project.close().await.unwrap();
}

#[tokio::test]
async fn transaction_ids_cannot_be_reused_even_with_a_fresh_binding() {
    let dir = temp();
    let project = fixture(&dir.path().join("project")).await;
    let first = new_rectangle(&project, 30, 21, 1.0, 1.0, 4.0, 4.0).await;
    let duplicate = request(
        &project,
        30,
        SelectionTarget::Existing {
            selection: first.snapshot.selection.clone(),
        },
        SelectionOperation::Invert,
    )
    .await;
    assert!(matches!(
        project.apply_selection(duplicate, cancel()).await,
        Err(SelectionError::ReusedTransaction)
    ));
    assert_eq!(project.info().await.unwrap(), first.revision);
    project.close().await.unwrap();
}

#[tokio::test]
async fn mask_export_is_exact_gray_png_and_never_overwrites_a_destination() {
    let dir = temp();
    let project = fixture(&dir.path().join("project")).await;
    let selected = new_rectangle(&project, 30, 21, 4.0, 5.0, 3.0, 4.0).await;
    let options = output(dir.path(), &selected, SelectionExportKind::Mask, "mask.png");
    let receipt = project
        .export_selection_file(options.clone(), cancel())
        .await
        .unwrap();
    let bytes = std::fs::read(&options.output_path).unwrap();
    assert_eq!(receipt.blake3, AssetId::hash(&bytes).to_string());
    assert_eq!(decode_gray(&bytes).2, pixels(&project, &selected).await);
    assert!(
        bytes
            .windows(receipt.binding.state_hash.len())
            .any(|s| s == receipt.binding.state_hash.as_bytes())
    );
    assert!(
        project
            .export_selection_file(options.clone(), cancel())
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(options.output_path).unwrap(), bytes);
    assert_eq!(project.info().await.unwrap(), selected.revision);
    project.close().await.unwrap();
}

#[tokio::test]
async fn export_refusals_leave_staging_and_source_unchanged() {
    let dir = temp();
    let project = fixture(&dir.path().join("project")).await;
    let selected = new_rectangle(&project, 30, 21, 4.0, 5.0, 3.0, 4.0).await;
    let stage = dir.path().join("stage");
    std::fs::create_dir(&stage).unwrap();
    let mut options = output(&stage, &selected, SelectionExportKind::Cutout, "cutout.png");
    options.max_encoded_bytes = 1;
    assert!(matches!(
        project
            .export_selection_file(options.clone(), cancel())
            .await,
        Err(SelectionError::EncodedLimit)
    ));
    options.max_encoded_bytes = 64 * 1024 * 1024;
    options.memory_budget_bytes = 1;
    assert!(matches!(
        project
            .export_selection_file(options.clone(), cancel())
            .await,
        Err(SelectionError::Memory { .. })
    ));
    options.memory_budget_bytes = MEMORY;
    options.output_path = dir.path().join("escape.png").to_string_lossy().into();
    assert!(matches!(
        project.export_selection_file(options, cancel()).await,
        Err(SelectionError::Invalid)
    ));
    assert_eq!(std::fs::read_dir(&stage).unwrap().count(), 0);
    assert!(!dir.path().join("escape.png").exists());
    assert_eq!(project.info().await.unwrap(), selected.revision);
    project.close().await.unwrap();
}

#[tokio::test]
async fn cutout_preserves_sixteen_bit_hidden_rgb_and_multiplies_alpha_once() {
    let dir = temp();
    let values = vec![
        12345u16, 23456, 34567, 0, 10101, 20202, 30303, 30000, 11111, 22222, 33333, 65535,
    ];
    let image = vw_raster::DecodedImage {
        width: 3,
        height: 1,
        pixels: vw_raster::Pixels::Rgba16(values.clone()),
        icc: None,
        source_asset: AssetId::hash(b"synthetic16"),
        original_available: true,
        orientation_applied: 1,
    };
    let encoded = vw_raster::export(
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
            memory_budget_bytes: MEMORY,
            capture_session: None,
            frame_id: None,
        },
    )
    .unwrap()
    .bytes;
    let mut creating = create(&dir.path().join("project"));
    creating.source = encoded;
    let project = create_image_project(creating, cancel()).await.unwrap();
    let options = request(
        &project,
        30,
        SelectionTarget::New {
            object_id: id(21),
            layer_id: id(20),
        },
        SelectionOperation::Paint {
            points: vec![Point { x: 1.5, y: 0.5 }],
            radius: 10.0,
            opacity: 128,
            combine: SelectionCombine::Add,
        },
    )
    .await;
    let selected = project.apply_selection(options, cancel()).await.unwrap();
    let options = output(
        dir.path(),
        &selected,
        SelectionExportKind::Cutout,
        "cutout.png",
    );
    let receipt = project
        .export_selection_file(options.clone(), cancel())
        .await
        .unwrap();
    assert_eq!(receipt.output_bit_depth, 16);
    let decoded = vw_raster::decode(
        &std::fs::read(options.output_path).unwrap(),
        vw_raster::DecodeLimits::default(),
    )
    .unwrap();
    let vw_raster::Pixels::Rgba16(result) = decoded.pixels else {
        panic!("lost source precision")
    };
    for (original, actual) in values
        .as_chunks::<4>()
        .0
        .iter()
        .zip(result.as_chunks::<4>().0)
    {
        assert_eq!(&actual[..3], &original[..3]);
        assert_eq!(
            u32::from(actual[3]),
            (u32::from(original[3]) * 128 + 127) / 255
        );
    }
    project.close().await.unwrap();
}

#[tokio::test]
async fn corrupt_mask_blob_is_refused_and_never_replaced() {
    let dir = temp();
    let path = dir.path().join("project");
    let project = fixture(&path).await;
    let selected = new_rectangle(&project, 30, 21, 0.0, 0.0, 3.0, 3.0).await;
    let blobs = vw_store::BlobStore::new(&path).unwrap();
    let blob_path = blobs
        .path(&AssetId::try_from(selected.snapshot.selection.asset_id.clone()).unwrap())
        .unwrap();
    let mut bytes = std::fs::read(&blob_path).unwrap();
    bytes[0] ^= 1;
    std::fs::write(&blob_path, &bytes).unwrap();
    assert!(matches!(
        project
            .selection_snapshot(selected.snapshot.binding, id(21), MEMORY, cancel())
            .await,
        Err(SelectionError::Corrupt)
    ));
    assert_eq!(std::fs::read(blob_path).unwrap(), bytes);
    assert_eq!(project.info().await.unwrap(), selected.revision);
    project.close().await.unwrap();
}

#[tokio::test]
async fn deleting_a_selection_is_whole_object_erase_and_undo_restores_it() {
    let dir = temp();
    let project = fixture(&dir.path().join("project")).await;
    let selected = new_rectangle(&project, 30, 21, 0.0, 0.0, 3.0, 3.0).await;
    let erased = project
        .apply_edit_at(
            edit(31, selected.revision.next_lamport),
            vec![EditCommand::Delete { object_id: id(21) }],
            Some(EditPrecondition {
                host_seq: selected.revision.host_seq,
                state_hash: selected.revision.state_hash,
            }),
            None,
            cancel(),
        )
        .await
        .unwrap();
    let doc = project.selection_document(id(2), cancel()).await.unwrap();
    assert!(matches!(
        project
            .selection_snapshot(doc.binding, id(21), MEMORY, cancel())
            .await,
        Err(SelectionError::Conflict)
    ));
    let reuse = request(
        &project,
        34,
        SelectionTarget::New {
            object_id: id(21),
            layer_id: id(20),
        },
        rectangle(0.0, 0.0, 3.0, 3.0, SelectionCombine::Add),
    )
    .await;
    assert!(matches!(
        project.apply_selection(reuse, cancel()).await,
        Err(SelectionError::Conflict)
    ));
    assert_eq!(project.info().await.unwrap(), erased);
    project
        .undo_redo(edit(32, erased.next_lamport), false, cancel())
        .await
        .unwrap();
    let doc = project.selection_document(id(2), cancel()).await.unwrap();
    let restored = project
        .selection_snapshot(doc.binding, id(21), MEMORY, cancel())
        .await
        .unwrap();
    assert_eq!(restored.selection, selected.snapshot.selection);
    project.close().await.unwrap();
}

#[tokio::test]
async fn content_identical_imported_gray_original_reuses_its_existing_metadata() {
    let dir = temp();
    let path = dir.path().join("project");
    let size = vw_mask::Size::new(64, 64).unwrap();
    let gray = vw_mask::Mask::rectangle(
        size,
        vw_mask::Rect {
            x: 0.0,
            y: 0.0,
            width: 64.0,
            height: 64.0,
        },
    )
    .unwrap()
    .encode_mask_png(vw_mask::Region::full(size))
    .unwrap();
    let mut options = create(&path);
    options.source = gray.clone();
    let project = create_image_project(options, cancel()).await.unwrap();
    let selected = new_rectangle(&project, 30, 21, 0.0, 0.0, 64.0, 64.0).await;
    assert_eq!(
        selected.snapshot.selection.asset_id,
        AssetId::hash(&gray).to_string()
    );
    assert!(pixels(&project, &selected).await.iter().all(|&v| v == 255));
    let blobs = vw_store::BlobStore::new(&path).unwrap();
    assert_eq!(blobs.read(&AssetId::hash(&gray)).unwrap(), gray);
    project.close().await.unwrap();
}
