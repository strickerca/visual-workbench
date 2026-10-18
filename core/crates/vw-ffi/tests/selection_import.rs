//! Source-bound proposal import uses the ordinary selection transaction/undo path.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use support::*;
use vw_core::*;
use vw_model::AssetId;

static FIXTURES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const MEMORY: u64 = 256 * 1024 * 1024;

fn temporary() -> tempfile::TempDir {
    if cfg!(target_os = "android") {
        tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()
    } else {
        tempfile::tempdir().unwrap()
    }
}
fn target() -> SelectionTarget {
    SelectionTarget::New {
        object_id: id(21),
        layer_id: id(20),
    }
}
fn coverage(first: u8, second: u8, combine: SelectionCombine) -> SelectionOperation {
    let mut bytes = vec![0; 64 * 64];
    bytes[0] = first;
    bytes[1] = second;
    SelectionOperation::ImportCoverage {
        width: 64,
        height: 64,
        coverage: bytes,
        combine,
    }
}
async fn request(
    project: &ProjectSession,
    transaction: u8,
    target: SelectionTarget,
    operation: SelectionOperation,
) -> SelectionEdit {
    let doc = project.selection_document(id(2), cancel()).await.unwrap();
    SelectionEdit {
        binding: doc.binding,
        transaction_id: id(transaction),
        device_id: device(),
        lamport: doc.revision.next_lamport,
        created_at_ms: TIME + i64::from(transaction),
        target,
        operation,
        memory_budget_bytes: MEMORY,
    }
}
async fn pixels(project: &ProjectSession, receipt: &SelectionReceipt) -> Vec<u8> {
    let tiles = project
        .selection_tiles(
            receipt.snapshot.binding.clone(),
            receipt.snapshot.selection.clone(),
            vec![SelectionRegion {
                x: 0,
                y: 0,
                width: 2,
                height: 1,
            }],
            MEMORY,
            cancel(),
        )
        .await
        .unwrap();
    tiles.tiles[0].coverage.clone()
}

#[tokio::test]
async fn imported_coverage_is_one_undoable_version_and_never_replaces_original_pixels() {
    let _serial = FIXTURES.lock().await;
    let dir = temporary();
    let path = dir.path().join("project");
    let project = create_image_project(create(&path), cancel()).await.unwrap();
    let before = project.info().await.unwrap();
    let options = request(
        &project,
        30,
        target(),
        coverage(128, 255, SelectionCombine::Add),
    )
    .await;
    let receipt = project
        .apply_selection(options.clone(), cancel())
        .await
        .unwrap();
    assert_eq!(receipt.transaction_id, options.transaction_id);
    assert_eq!(receipt.snapshot.selection.version, 1);
    assert_eq!(receipt.revision.host_seq, before.host_seq + 1);
    assert_eq!(pixels(&project, &receipt).await, [128, 255]);
    // The transaction identity is consumed exactly once, never reinterpreted as
    // a new proposal against the now-current source/version.
    let mut reused = options;
    reused.binding = receipt.snapshot.binding.clone();
    assert!(matches!(
        project.apply_selection(reused, cancel()).await,
        Err(SelectionError::ReusedTransaction)
    ));
    let undone = project
        .undo_redo(edit(31, receipt.revision.next_lamport), false, cancel())
        .await
        .unwrap();
    assert_eq!(undone.state_hash, before.state_hash);
    let redone = project
        .undo_redo(edit(32, undone.next_lamport), true, cancel())
        .await
        .unwrap();
    assert_eq!(redone.state_hash, receipt.revision.state_hash);
    let original = source();
    assert_eq!(
        vw_store::BlobStore::new(&path)
            .unwrap()
            .read(&AssetId::hash(&original))
            .unwrap(),
        original
    );
    project.close().await.unwrap();
}

#[tokio::test]
async fn import_combines_with_exact_existing_version_using_existing_mask_math() {
    let _serial = FIXTURES.lock().await;
    for (combine, expected) in [
        (SelectionCombine::Add, [192, 255]),
        (SelectionCombine::Subtract, [0, 191]),
        (SelectionCombine::Intersect, [128, 64]),
    ] {
        let dir = temporary();
        let project = create_image_project(create(&dir.path().join("project")), cancel())
            .await
            .unwrap();
        let initial = request(
            &project,
            30,
            target(),
            coverage(128, 255, SelectionCombine::Add),
        )
        .await;
        let first = project.apply_selection(initial, cancel()).await.unwrap();
        let update = request(
            &project,
            31,
            SelectionTarget::Existing {
                selection: first.snapshot.selection.clone(),
            },
            coverage(192, 64, combine),
        )
        .await;
        let second = project.apply_selection(update, cancel()).await.unwrap();
        assert_eq!(second.snapshot.selection.version, 2);
        assert_eq!(pixels(&project, &second).await, expected);
        let mut stale = request(
            &project,
            32,
            SelectionTarget::Existing {
                selection: first.snapshot.selection,
            },
            coverage(255, 255, SelectionCombine::Add),
        )
        .await;
        assert!(matches!(
            project.apply_selection(stale.clone(), cancel()).await,
            Err(SelectionError::Conflict)
        ));
        stale.target = SelectionTarget::Existing {
            selection: second.snapshot.selection,
        };
        // Ordinary manual correction follows the imported canonical version.
        stale.operation = SelectionOperation::Paint {
            points: vec![Point { x: 0.5, y: 0.5 }],
            radius: 1.0,
            opacity: 255,
            combine: SelectionCombine::Subtract,
        };
        let corrected = project.apply_selection(stale, cancel()).await.unwrap();
        assert_eq!(corrected.snapshot.selection.version, 3);
        assert_eq!(pixels(&project, &corrected).await[0], 0);
        project.close().await.unwrap();
    }
}

#[tokio::test]
async fn invalid_byte_counts_dimensions_budget_and_cancel_leave_revision_unchanged() {
    let _serial = FIXTURES.lock().await;
    let dir = temporary();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    let before = project.info().await.unwrap();
    for (width, height, bytes) in [(64, 64, 4095), (32, 128, 4096), (0, 64, 0)] {
        let options = request(
            &project,
            30,
            target(),
            SelectionOperation::ImportCoverage {
                width,
                height,
                coverage: vec![255; bytes],
                combine: SelectionCombine::Add,
            },
        )
        .await;
        assert!(matches!(
            project.apply_selection(options, cancel()).await,
            Err(SelectionError::Invalid)
        ));
    }
    let mut limited = request(
        &project,
        30,
        target(),
        coverage(128, 255, SelectionCombine::Add),
    )
    .await;
    limited.memory_budget_bytes = 64 * 64 * 20 + 16 * 1024 * 1024 - 1;
    assert!(matches!(
        project.apply_selection(limited, cancel()).await,
        Err(SelectionError::Memory { .. })
    ));
    let mut stale = request(
        &project,
        30,
        target(),
        coverage(128, 255, SelectionCombine::Add),
    )
    .await;
    stale.binding.source_asset_id = AssetId::hash(b"other source").to_string();
    assert!(matches!(
        project.apply_selection(stale, cancel()).await,
        Err(SelectionError::Conflict)
    ));
    let options = request(
        &project,
        30,
        target(),
        coverage(128, 255, SelectionCombine::Add),
    )
    .await;
    let cancellation = cancel();
    cancellation.cancel();
    assert!(matches!(
        project.apply_selection(options, cancellation).await,
        Err(SelectionError::Cancelled)
    ));
    let after = project.info().await.unwrap();
    assert_eq!(after.host_seq, before.host_seq);
    assert_eq!(after.state_hash, before.state_hash);
    project.close().await.unwrap();
}
