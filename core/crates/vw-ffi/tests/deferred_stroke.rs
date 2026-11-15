//! A stroke begun without reading the project (`begin_stroke_deferred`) must
//! save exactly what `begin_stroke` saves, must be able to start while the
//! project is busy, and must be refused at commit for the same reasons
//! `begin_stroke` refuses a stroke at its start.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use std::sync::Arc;
use support::*;
use vw_core::*;
use vw_model::Id;

const BUDGET: u64 = 256 * 1024 * 1024;

fn wide_id(kind: u8, number: u32) -> String {
    let mut entropy = [kind; 10];
    entropy[6..].copy_from_slice(&number.to_be_bytes());
    Id::from_parts(TIME as u64, entropy).unwrap().to_string()
}
async fn numbered(project: &Arc<ProjectSession>, number: u32) -> StrokeOptions {
    let mut options = stroke();
    options.gesture_id = wide_id(0x41, number);
    options.transaction_id = wide_id(0x42, number);
    options.object_id = wide_id(0x60, number);
    options.lamport = project.info().await.unwrap().next_lamport;
    options.created_at_ms = TIME + 10 + i64::from(number);
    options
}

#[tokio::test]
async fn a_deferred_stroke_saves_byte_for_byte_what_a_checked_stroke_saves() {
    let dir = tempfile::tempdir().unwrap();
    let checked = create_image_project(create(&dir.path().join("checked")), cancel())
        .await
        .unwrap();
    let deferred = create_image_project(create(&dir.path().join("deferred")), cancel())
        .await
        .unwrap();
    let a = checked.clone().begin_stroke(stroke()).await.unwrap();
    let b = deferred.clone().begin_stroke_deferred(stroke()).unwrap();
    let first = a.append_samples(batch()).await.unwrap();
    let second = b.append_samples(batch()).await.unwrap();
    assert_eq!(first, second);
    let saved_a = a.commit(cancel()).await.unwrap();
    let saved_b = b.commit(cancel()).await.unwrap();
    // Same revision, same state hash: the stored transaction is identical.
    assert_eq!(saved_a, saved_b);
    let export_a = checked.export_image(options(), cancel()).await.unwrap();
    let export_b = deferred.export_image(options(), cancel()).await.unwrap();
    assert_eq!(export_a.blake3, export_b.blake3);
    assert_eq!(export_a.bytes, export_b.bytes);
    // An exact retry of a finished commit returns the same receipt.
    assert_eq!(b.commit(cancel()).await.unwrap(), saved_b);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    checked.close().await.unwrap();
    deferred.close().await.unwrap();
}

#[tokio::test]
async fn a_deferred_stroke_begun_before_another_save_is_saved_on_top_of_it() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    // The second stroke starts and takes ink before the first is saved: this
    // is the pen going down again while the previous stroke is still saving.
    let first = project
        .clone()
        .begin_stroke(numbered(&project, 1).await)
        .await
        .unwrap();
    first.append_samples(batch()).await.unwrap();
    let mut later = numbered(&project, 2).await;
    later.lamport += 3;
    let second = project.clone().begin_stroke_deferred(later).unwrap();
    second.append_samples(batch()).await.unwrap();
    let saved_first = first.commit(cancel()).await.unwrap();
    let saved_second = second.commit(cancel()).await.unwrap();
    assert_eq!(saved_second.host_seq, saved_first.host_seq + 1);
    let items = project
        .render_list(id(2), None, cancel())
        .await
        .unwrap()
        .items;
    assert_eq!(
        items
            .iter()
            .map(|i| i.object_id.clone())
            .collect::<Vec<_>>(),
        vec![wide_id(0x60, 1), wide_id(0x60, 2)]
    );
    first.shutdown().await.unwrap();
    second.shutdown().await.unwrap();
    project.close().await.unwrap();
}

#[tokio::test]
async fn what_begin_refuses_at_the_start_a_deferred_stroke_is_refused_at_commit() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    // Malformed requests are refused at once, as by begin_stroke.
    let mut bad = stroke();
    bad.lamport = 0;
    assert!(matches!(
        project.clone().begin_stroke_deferred(bad),
        Err(CoreError::Invalid)
    ));
    let mut highlighter = stroke();
    highlighter.family = "highlighter".into();
    assert!(matches!(
        project.clone().begin_stroke_deferred(highlighter),
        Err(CoreError::Unsupported)
    ));

    // A layer that does not exist: begin_stroke refuses, the deferred stroke
    // takes ink and is refused when saved. Nothing is stored either way.
    let mut nowhere = numbered(&project, 1).await;
    nowhere.layer_id = wide_id(0x70, 1);
    assert!(matches!(
        project.clone().begin_stroke(nowhere.clone()).await,
        Err(CoreError::Invalid)
    ));
    let gesture = project.clone().begin_stroke_deferred(nowhere).unwrap();
    gesture.append_samples(batch()).await.unwrap();
    let before = project.info().await.unwrap();
    assert!(matches!(
        gesture.commit(cancel()).await,
        Err(CoreError::Invalid)
    ));
    assert_eq!(project.info().await.unwrap(), before);
    gesture.shutdown().await.unwrap();

    // An object that already exists.
    let existing = numbered(&project, 2).await;
    let saved = project
        .clone()
        .begin_stroke(existing.clone())
        .await
        .unwrap();
    saved.append_samples(batch()).await.unwrap();
    saved.commit(cancel()).await.unwrap();
    saved.shutdown().await.unwrap();
    let mut duplicate = numbered(&project, 3).await;
    duplicate.object_id = existing.object_id.clone();
    let gesture = project.clone().begin_stroke_deferred(duplicate).unwrap();
    gesture.append_samples(batch()).await.unwrap();
    let before = project.info().await.unwrap();
    assert!(matches!(
        gesture.commit(cancel()).await,
        Err(CoreError::Invalid)
    ));
    assert_eq!(project.info().await.unwrap(), before);
    gesture.shutdown().await.unwrap();

    // A layer locked after the pen went down: refused at commit.
    let gesture = project
        .clone()
        .begin_stroke_deferred(numbered(&project, 4).await)
        .unwrap();
    gesture.append_samples(batch()).await.unwrap();
    let view = project
        .document_controls_snapshot(id(2), BUDGET, cancel())
        .await
        .unwrap();
    let plan = project
        .clone()
        .prepare_document_control(
            view.binding,
            WorkflowMetadata {
                transaction_id: wide_id(0x45, 1),
                device_id: view.revision.device_id.clone(),
                first_lamport: view.revision.next_lamport + 8,
                created_at_ms: TIME + 600,
            },
            DocumentControlCommand::UpdateLayer {
                update: LayerControlUpdate {
                    layer_id: id(3),
                    name: None,
                    visible: None,
                    locked: Some(true),
                    opacity: None,
                    blend: None,
                },
            },
            BUDGET,
            cancel(),
        )
        .await
        .unwrap();
    plan.commit(cancel()).await.unwrap();
    plan.dispose();
    let before = project.info().await.unwrap();
    assert!(matches!(
        gesture.commit(cancel()).await,
        Err(CoreError::Invalid)
    ));
    assert_eq!(project.info().await.unwrap(), before);
    gesture.shutdown().await.unwrap();
    project.close().await.unwrap();
}

/// The point of the call: it returns while the project's own thread is
/// occupied, where `begin_stroke` has to wait for it.
#[tokio::test]
async fn a_deferred_stroke_starts_and_takes_ink_while_the_project_is_busy() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    // Occupy the project thread with a queue of exports.
    let busy: Vec<_> = (0..6)
        .map(|_| {
            let project = project.clone();
            tokio::spawn(async move { project.export_image(options(), cancel()).await.is_ok() })
        })
        .collect();
    tokio::task::yield_now().await;
    let gesture = project.clone().begin_stroke_deferred(stroke()).unwrap();
    let update = gesture.append_samples(batch()).await.unwrap();
    assert_eq!(update.sample_count, 3);
    // The ink is there whether or not the exports have finished; then the
    // save takes its turn behind them.
    let saved = gesture.commit(cancel()).await.unwrap();
    assert_eq!(saved.host_seq, 1);
    for task in busy {
        assert!(task.await.unwrap());
    }
    gesture.shutdown().await.unwrap();
    project.close().await.unwrap();
}
