#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use support::*;
use vw_core::*;
use vw_model::AssetId;

// Real native fixture admission is global in this process (four startup slots).
// Serializing these tests avoids testing accidental libtest scheduling pressure.
static FIXTURES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const MEMORY: u64 = 256 * 1024 * 1024;
fn temporary() -> tempfile::TempDir {
    if cfg!(target_os = "android") { tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap() }
    else { tempfile::tempdir().unwrap() }
}
async fn request(project: &ProjectSession, transaction: u8, target: SelectionTarget, operation: SelectionOperation) -> SelectionEdit {
    let doc = project.selection_document(id(2), cancel()).await.unwrap();
    SelectionEdit { binding: doc.binding, transaction_id: id(transaction), device_id: device(),
        lamport: doc.revision.next_lamport, created_at_ms: TIME + i64::from(transaction),
        target, operation, memory_budget_bytes: MEMORY }
}
fn ellipse() -> SelectionOperation {
    SelectionOperation::Ellipse { rectangle: QueryRect { x: 5.0, y: 7.0, width: 2.0, height: 2.0 }, combine: SelectionCombine::Add }
}
fn polygon(combine: SelectionCombine) -> SelectionOperation {
    SelectionOperation::Polygon { vertices: vec![Point { x: 5.0, y: 7.0 }, Point { x: 6.0, y: 7.0 },
        Point { x: 6.0, y: 9.0 }, Point { x: 5.0, y: 9.0 }], combine }
}
fn new_target() -> SelectionTarget { SelectionTarget::New { object_id: id(21), layer_id: id(20) } }
async fn tile(project: &ProjectSession, receipt: &SelectionReceipt) -> Vec<u8> {
    let tiles = project.selection_tiles(receipt.snapshot.binding.clone(), receipt.snapshot.selection.clone(),
        vec![SelectionRegion { x: 4, y: 6, width: 4, height: 4 }], MEMORY, cancel()).await.unwrap();
    tiles.tiles[0].coverage.clone()
}

#[tokio::test]
async fn ellipse_has_exact_native_golden_and_undo_redo_reopens_without_touching_source() {
    let _serial = FIXTURES.lock().await;
    let dir = temporary(); let path = dir.path().join("project");
    let project = create_image_project(create(&path), cancel()).await.unwrap();
    let before = project.info().await.unwrap();
    let options = request(&project, 30, new_target(), ellipse()).await;
    let receipt = project.apply_selection(options, cancel()).await.unwrap();
    assert_eq!(receipt.revision.host_seq, before.host_seq + 1);
    assert_eq!(tile(&project, &receipt).await, [0, 0, 0, 0, 0, 200, 200, 0, 0, 200, 200, 0, 0, 0, 0, 0]);
    let undone = project.undo_redo(edit(31, receipt.revision.next_lamport), false, cancel()).await.unwrap();
    assert_eq!(undone.state_hash, before.state_hash);
    let redone = project.undo_redo(edit(32, undone.next_lamport), true, cancel()).await.unwrap();
    assert_eq!(redone.state_hash, receipt.revision.state_hash);
    let original = source(); let blobs = vw_store::BlobStore::new(&path).unwrap();
    assert_eq!(blobs.read(&AssetId::hash(&original)).unwrap(), original);
    project.close().await.unwrap();
    let reopened = open_project(path.to_string_lossy().into(), cancel()).await.unwrap();
    assert_eq!(reopened.info().await.unwrap().state_hash, redone.state_hash);
    let binding = reopened.selection_document(id(2), cancel()).await.unwrap().binding;
    let snapshot = reopened.selection_snapshot(binding, id(21), MEMORY, cancel()).await.unwrap();
    assert_eq!(snapshot.selection, receipt.snapshot.selection);
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn polygon_subtraction_and_intersection_share_existing_mask_versions() {
    let _serial = FIXTURES.lock().await;
    for combine in [SelectionCombine::Subtract, SelectionCombine::Intersect] {
        let dir = temporary(); let project = create_image_project(create(&dir.path().join("project")), cancel()).await.unwrap();
        let options = request(&project, 30, new_target(), ellipse()).await;
        let first = project.apply_selection(options, cancel()).await.unwrap();
        let options = request(&project, 31, SelectionTarget::Existing { selection: first.snapshot.selection.clone() }, polygon(combine)).await;
        let second = project.apply_selection(options, cancel()).await.unwrap();
        assert_eq!(second.snapshot.selection.version, first.snapshot.selection.version + 1);
        let (left, right) = match combine { SelectionCombine::Subtract => (0, 200), _ => (200, 0) };
        assert_eq!(tile(&project, &second).await, [0, 0, 0, 0, 0, left, right, 0, 0, left, right, 0, 0, 0, 0, 0]);
        let current = project.undo_redo(edit(32, second.revision.next_lamport), false, cancel()).await.unwrap();
        assert_eq!(current.state_hash, first.revision.state_hash);
        project.close().await.unwrap();
    }
}

#[tokio::test]
async fn stale_shape_request_cancel_and_vertex_admission_are_atomic() {
    let _serial = FIXTURES.lock().await;
    let dir = temporary(); let project = create_image_project(create(&dir.path().join("project")), cancel()).await.unwrap();
    let options = request(&project, 30, new_target(), ellipse()).await;
    let initial = project.info().await.unwrap();
    let cancelled = cancel(); cancelled.cancel();
    assert!(matches!(project.apply_selection(options.clone(), cancelled).await, Err(SelectionError::Cancelled)));
    let mut bad = options.clone();
    bad.operation = SelectionOperation::Polygon { vertices: vec![Point { x: 0.0, y: 0.0 }; 16_385], combine: SelectionCombine::Add };
    assert!(matches!(project.apply_selection(bad, cancel()).await, Err(SelectionError::Limit)));
    assert_eq!(project.info().await.unwrap(), initial);
    let first = project.apply_selection(options, cancel()).await.unwrap();
    let stale = request(&project, 31, SelectionTarget::Existing { selection: first.snapshot.selection.clone() }, polygon(SelectionCombine::Subtract)).await;
    let current = request(&project, 32, SelectionTarget::Existing { selection: first.snapshot.selection }, SelectionOperation::Invert).await;
    let accepted = project.apply_selection(current, cancel()).await.unwrap();
    assert!(matches!(project.apply_selection(stale, cancel()).await, Err(SelectionError::Conflict)));
    assert_eq!(project.info().await.unwrap(), accepted.revision);
    project.close().await.unwrap();
}
