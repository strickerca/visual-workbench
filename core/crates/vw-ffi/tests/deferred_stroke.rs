//! A stroke begun without waiting for the project (`begin_stroke_deferred`)
//! must save exactly what `begin_stroke` saves, must be able to start while
//! the project is busy, must be refused for the same reasons `begin_stroke`
//! refuses a stroke, and must never be lost where that one is kept.
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

/// Changes the stroke fixture's layer from another edit, as the other device
/// would while the pen is down.
async fn update_layer(
    project: &Arc<ProjectSession>,
    number: u32,
    locked: Option<bool>,
    visible: Option<bool>,
) {
    let view = project
        .document_controls_snapshot(id(2), BUDGET, cancel())
        .await
        .unwrap();
    let plan = project
        .clone()
        .prepare_document_control(
            view.binding,
            WorkflowMetadata {
                transaction_id: wide_id(0x45, number),
                device_id: view.revision.device_id.clone(),
                first_lamport: view.revision.next_lamport + 8,
                created_at_ms: TIME + 600 + i64::from(number),
            },
            DocumentControlCommand::UpdateLayer {
                update: LayerControlUpdate {
                    layer_id: id(3),
                    name: None,
                    visible,
                    locked,
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
}

#[tokio::test]
async fn what_begin_refuses_a_deferred_stroke_is_refused_with_the_same_error_and_takes_no_more_ink()
{
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

    // A layer that does not exist: begin_stroke refuses. The deferred stroke
    // begins; once the project has answered it takes no more ink and its
    // commit is refused with the same error. Nothing is stored either way.
    let mut nowhere = numbered(&project, 1).await;
    nowhere.layer_id = wide_id(0x70, 1);
    let refused = project.clone().begin_stroke(nowhere.clone()).await;
    assert!(matches!(refused, Err(CoreError::Invalid)));
    let gesture = project.clone().begin_stroke_deferred(nowhere).unwrap();
    // Any later request to the project is answered after the stroke's own.
    let before = project.info().await.unwrap();
    assert!(matches!(
        gesture.append_samples(batch()).await,
        Err(CoreError::Invalid)
    ));
    assert!(matches!(
        gesture.preview_samples(batch()).await,
        Err(CoreError::Invalid)
    ));
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
    assert!(matches!(
        project.clone().begin_stroke(duplicate.clone()).await,
        Err(CoreError::Invalid)
    ));
    let gesture = project.clone().begin_stroke_deferred(duplicate).unwrap();
    let before = project.info().await.unwrap();
    assert!(matches!(
        gesture.append_samples(batch()).await,
        Err(CoreError::Invalid)
    ));
    assert!(matches!(
        gesture.commit(cancel()).await,
        Err(CoreError::Invalid)
    ));
    assert_eq!(project.info().await.unwrap(), before);
    gesture.shutdown().await.unwrap();

    // A layer that is locked, and then one that is hidden, when the pen goes down.
    update_layer(&project, 4, Some(true), None).await;
    for number in [14, 15] {
        let options = numbered(&project, number).await;
        assert!(matches!(
            project.clone().begin_stroke(options.clone()).await,
            Err(CoreError::Invalid)
        ));
        let gesture = project.clone().begin_stroke_deferred(options).unwrap();
        let before = project.info().await.unwrap();
        assert!(matches!(
            gesture.append_samples(batch()).await,
            Err(CoreError::Invalid)
        ));
        assert!(matches!(
            gesture.commit(cancel()).await,
            Err(CoreError::Invalid)
        ));
        assert_eq!(project.info().await.unwrap(), before);
        gesture.shutdown().await.unwrap();
        if number == 14 {
            update_layer(&project, 5, Some(false), None).await;
            update_layer(&project, 6, None, Some(false)).await;
        }
    }
    project.close().await.unwrap();
}

/// The layer is changed after the pen went down. Whatever a stroke begun with
/// `begin_stroke` ends as, a deferred stroke ends as the same: it is never
/// lost where the other is kept.
#[tokio::test]
async fn a_layer_changed_during_the_stroke_ends_the_same_for_both_ways_of_beginning() {
    for (name, locked, visible, kept) in [
        ("hidden", None, Some(false), true),
        ("locked", Some(true), None, false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut outcomes = Vec::new();
        for deferred in [false, true] {
            let path = dir.path().join(format!("{name}-{deferred}"));
            let project = create_image_project(create(&path), cancel()).await.unwrap();
            let options = numbered(&project, 1).await;
            let gesture = if deferred {
                project.clone().begin_stroke_deferred(options).unwrap()
            } else {
                project.clone().begin_stroke(options).await.unwrap()
            };
            gesture.append_samples(batch()).await.unwrap();
            update_layer(&project, 1, locked, visible).await;
            // Ink is still taken after the change, as before.
            let mut more = batch();
            more.sequence = 2;
            more.t_ms = vec![24, 32, 40];
            gesture.append_samples(more).await.unwrap();
            let saved = gesture.commit(cancel()).await;
            let info = project.info().await.unwrap();
            gesture.shutdown().await.unwrap();
            project.close().await.unwrap();
            outcomes.push((format!("{saved:?}"), info));
        }
        assert_eq!(outcomes[0], outcomes[1], "{name}");
        assert_eq!(outcomes[0].0.starts_with("Ok("), kept, "{name}");
    }
}

/// Conflict review compares a transaction with what other devices changed
/// after its base. A deferred stroke is based on the revision at pen-down,
/// exactly as a stroke begun with `begin_stroke`, also when something else was
/// saved while the pen was down: the stored transactions are identical.
#[tokio::test]
async fn a_deferred_stroke_is_stored_with_the_revision_at_pen_down_as_its_base() {
    let dir = tempfile::tempdir().unwrap();
    let mut stored = Vec::new();
    for deferred in [false, true] {
        let path = dir.path().join(format!("project-{deferred}"));
        let project = create_image_project(create(&path), cancel()).await.unwrap();
        let at_pen_down = project.info().await.unwrap();
        let mut options = numbered(&project, 1).await;
        options.lamport += 3;
        let gesture = if deferred {
            project
                .clone()
                .begin_stroke_deferred(options.clone())
                .unwrap()
        } else {
            project.clone().begin_stroke(options.clone()).await.unwrap()
        };
        gesture.append_samples(batch()).await.unwrap();
        // Another stroke is saved while this one is still being drawn.
        let other = project
            .clone()
            .begin_stroke(numbered(&project, 2).await)
            .await
            .unwrap();
        other.append_samples(batch()).await.unwrap();
        let between = other.commit(cancel()).await.unwrap();
        other.shutdown().await.unwrap();
        assert_eq!(between.host_seq, at_pen_down.host_seq + 1);
        let saved = gesture.commit(cancel()).await.unwrap();
        assert_eq!(saved.host_seq, at_pen_down.host_seq + 2);
        gesture.shutdown().await.unwrap();
        project.close().await.unwrap();
        let store = vw_store::ProjectStore::open(&path).unwrap();
        let wanted = Id::try_from(options.transaction_id).unwrap().to_proto();
        let (transaction, _) = store
            .accepted_transactions()
            .find(|(txn, _)| txn.txn_id.as_ref() == Some(&wanted))
            .unwrap();
        let base = transaction.base_revision.clone().unwrap();
        assert_eq!(base.host_seq, at_pen_down.host_seq);
        stored.push((transaction.clone(), saved));
    }
    assert_eq!(stored[0], stored[1]);
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
