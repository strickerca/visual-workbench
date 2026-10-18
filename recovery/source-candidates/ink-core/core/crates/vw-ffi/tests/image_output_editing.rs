//! The source editing view recognizes only the typed global tone convention.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use support::*;
use vw_core::*;
use std::sync::Arc;
const BUDGET: u64 = 256 * 1024 * 1024;
fn temp() -> tempfile::TempDir {
    if cfg!(target_os = "android") { tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap() }
    else { tempfile::tempdir().unwrap() }
}
async fn tone(project: &Arc<ProjectSession>) -> WorkflowReceipt {
    let view = project.image_output_snapshot(id(2), BUDGET, cancel()).await.unwrap();
    let info = project.info().await.unwrap();
    let plan = project.clone().prepare_image_output(view.binding, WorkflowMetadata {
        transaction_id: id(20), device_id: device(), first_lamport: info.next_lamport, created_at_ms: TIME + 20,
    }, ImageOutputCommand::SetTone {
        layer_id: id(30), object_id: id(31), enabled: true, opacity: 1.0,
        settings: ImageToneSettings { brightness: -1.0, contrast: 0.0, input_black: 0.0,
            input_white: 1.0, gamma: 1.0, output_black: 0.0, output_white: 1.0 },
    }, BUDGET, cancel()).await.unwrap();
    let receipt = plan.commit(cancel()).await.unwrap();
    plan.dispose(); receipt
}

#[tokio::test]
async fn known_tone_is_output_metadata_after_reopen_and_remains_in_export_and_undo() {
    let root = temp(); let path = root.path().join("project");
    let project = create_image_project(create(&path), cancel()).await.unwrap();
    let initial = project.info().await.unwrap();
    let accepted = tone(&project).await;
    assert!(project.render_list(id(2), None, cancel()).await.unwrap().items.is_empty());
    let result = project.export_image(options(), cancel()).await.unwrap();
    let image = vw_raster::decode(&result.bytes, vw_raster::DecodeLimits::default()).unwrap();
    assert_eq!(&image.pixels.rgba8()[..4], &[0, 0, 0, 255]);
    project.close().await.unwrap();
    let project = open_project(path.to_string_lossy().into(), cancel()).await.unwrap();
    assert!(project.render_list(id(2), None, cancel()).await.unwrap().items.is_empty());
    assert_eq!(project.image_output_snapshot(id(2), BUDGET, cancel()).await.unwrap().tones.len(), 1);
    project.undo_redo(edit(21, accepted.revision.next_lamport), false, cancel()).await.unwrap();
    assert_eq!(project.info().await.unwrap().state_hash, initial.state_hash);
    project.close().await.unwrap();
}

#[tokio::test]
async fn unrelated_transformed_adjustment_is_not_silently_hidden_from_editor_refusal() {
    let root = temp(); let project = create_image_project(create(&root.path().join("project")), cancel()).await.unwrap();
    let accepted = tone(&project).await;
    project.apply_edit(edit(21, accepted.revision.next_lamport), vec![EditCommand::Transform {
        object_id: id(31), transform: Transform { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 1.0, f: 0.0 },
    }], cancel()).await.unwrap();
    let list = project.render_list(id(2), None, cancel()).await.unwrap();
    assert_eq!(list.items.len(), 1);
    assert!(matches!(list.items[0].shape, DrawShape::Adjustment));
    assert!(project.image_output_snapshot(id(2), BUDGET, cancel()).await.is_err());
    project.close().await.unwrap();
}
