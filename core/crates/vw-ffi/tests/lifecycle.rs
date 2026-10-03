#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use support::*;
use vw_core::*;

#[tokio::test]
async fn shutdown_releases_capacity_while_closed_gesture_handles_remain_alive() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    let initial = project.info().await.unwrap();
    let mut active = Vec::new();
    for _ in 0..4 {
        active.push(project.clone().begin_stroke(stroke()).await.unwrap());
    }
    assert!(matches!(
        project.clone().begin_stroke(stroke()).await,
        Err(CoreError::Backpressure)
    ));
    let mut retained = Vec::new();
    for n in 0..32u8 {
        let previous = active.remove(0);
        previous.shutdown().await.unwrap();
        previous.shutdown().await.unwrap();
        assert!(previous.append_samples(batch()).await.is_err());
        retained.push(previous);
        let mut options = stroke();
        options.gesture_id = id(20 + n);
        options.transaction_id = id(60 + n);
        options.object_id = id(100 + n);
        active.push(project.clone().begin_stroke(options).await.unwrap());
        assert!(matches!(
            project.clone().begin_stroke(stroke()).await,
            Err(CoreError::Backpressure)
        ));
    }
    for gesture in active {
        gesture.shutdown().await.unwrap();
        retained.push(gesture);
    }
    assert_eq!(retained.len(), 36);
    assert_eq!(project.info().await.unwrap(), initial);
    project.close().await.unwrap();
}

#[tokio::test]
async fn shutdown_after_commit_preserves_the_durable_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    let gesture = project.clone().begin_stroke(stroke()).await.unwrap();
    gesture.append_samples(batch()).await.unwrap();
    let committed = gesture.commit(cancel()).await.unwrap();
    gesture.shutdown().await.unwrap();
    assert_eq!(gesture.commit(cancel()).await.unwrap(), committed);
    assert_eq!(project.info().await.unwrap(), committed);
    project.close().await.unwrap();
}

#[tokio::test]
async fn create_stroke_export_reopen_undo_redo_is_durable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("project");
    let project = create_image_project(create(&path), cancel()).await.unwrap();
    let export = draw(&project).await;
    assert_eq!(export.revision.host_seq, 1);
    assert!(export.revision.can_undo);
    let snapshot = project.document_snapshot(id(2), cancel()).await.unwrap();
    assert_eq!(snapshot.render.items.len(), 1);
    assert!(!snapshot.render.items[0].stroke_contours.x.is_empty());
    project.close().await.unwrap();
    assert!(matches!(project.info().await, Err(CoreError::Closed)));
    drop(project);
    let project = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    assert_eq!(
        project.info().await.unwrap().state_hash,
        export.revision.state_hash
    );
    project
        .undo_redo(edit(10, 2), false, cancel())
        .await
        .unwrap();
    assert!(
        project
            .document_snapshot(id(2), cancel())
            .await
            .unwrap()
            .render
            .items
            .is_empty()
    );
    project.close().await.unwrap();
    drop(project);
    let project = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    assert!(project.info().await.unwrap().can_redo);
    project
        .undo_redo(edit(11, 3), true, cancel())
        .await
        .unwrap();
    assert_eq!(
        project
            .document_snapshot(id(2), cancel())
            .await
            .unwrap()
            .render
            .items
            .len(),
        1
    );
    project.close().await.unwrap();
}
#[tokio::test]
async fn prediction_retry_cancel_and_malformed_batch_leave_history_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    let initial = project.info().await.unwrap();
    let gesture = project.clone().begin_stroke(stroke()).await.unwrap();
    let mut malformed = batch();
    malformed.y.pop();
    assert!(gesture.append_samples(malformed).await.is_err());
    let update = gesture.append_samples(batch()).await.unwrap();
    assert_eq!(update, gesture.append_samples(batch()).await.unwrap());
    let predicted = SampleBatch {
        sequence: 2,
        x: vec![40.0],
        y: vec![20.0],
        t_ms: vec![24],
        pressure: vec![0.5],
        tilt: vec![],
        orientation: vec![],
    };
    gesture.preview_samples(predicted.clone()).await.unwrap();
    let real = gesture.append_samples(predicted).await.unwrap();
    assert_eq!(real.sample_count, 4);
    let mut collision = batch();
    collision.sequence = 2;
    assert!(gesture.append_samples(collision).await.is_err());
    gesture.cancel().unwrap();
    assert!(gesture.commit(cancel()).await.is_err());
    assert_eq!(initial, project.info().await.unwrap());
    project.close().await.unwrap();
}
#[tokio::test]
async fn hundred_thousand_samples_cross_bounded_batches_without_prediction_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    let gesture = project.clone().begin_stroke(stroke()).await.unwrap();
    let mut total = 0;
    for sequence in 1..=196 {
        let n = 512.min(100_000 - total);
        let input = SampleBatch {
            sequence,
            x: vec![10.0; n],
            y: vec![10.0; n],
            t_ms: (total..total + n).map(|n| n as u32).collect(),
            pressure: vec![0.5; n],
            tilt: vec![],
            orientation: vec![],
        };
        let update = gesture.append_samples(input).await.unwrap();
        total += n;
        assert_eq!(update.sample_count, total as u64);
    }
    let mut overflow = batch();
    overflow.sequence = 197;
    assert!(gesture.append_samples(overflow).await.is_err());
    assert_eq!(gesture.commit(cancel()).await.unwrap().host_seq, 1);
    assert_eq!(gesture.commit(cancel()).await.unwrap().host_seq, 1);
    project.close().await.unwrap();
}
#[tokio::test]
async fn highlighter_uses_atomic_multiply_layer_and_cancelled_calls_do_not_commit() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    let mut brush = stroke();
    brush.family = "highlighter".into();
    let gesture = project.clone().begin_stroke(brush).await.unwrap();
    gesture.append_samples(batch()).await.unwrap();
    let cancelled = cancel();
    cancelled.cancel();
    assert!(matches!(
        gesture.commit(cancelled).await,
        Err(CoreError::Cancelled)
    ));
    assert_eq!(project.info().await.unwrap().host_seq, 0);
    let info = gesture.commit(cancel()).await.unwrap();
    assert_eq!(info.next_lamport, 4);
    assert_eq!(
        project
            .document_snapshot(id(2), cancel())
            .await
            .unwrap()
            .render
            .items[0]
            .layer_blend,
        "multiply"
    );
    project.export_image(options(), cancel()).await.unwrap();
    project.close().await.unwrap();
}
#[test]
fn camera_keeps_f64_and_rejects_unbounded_batches() {
    let camera = Camera {
        center: Point { x: 1e8, y: -1e8 },
        scale: 2.25,
        rotation: 0.4,
        viewport_width: 1920.0,
        viewport_height: 1080.0,
    };
    let p = Point {
        x: 1e8 + 0.125,
        y: -1e8 + 0.5,
    };
    let q = camera_map_points(camera, false, vec![p]).unwrap();
    let r = camera_map_points(camera, true, q).unwrap();
    assert!((r[0].x - p.x).abs() < 1e-6);
    assert!((r[0].y - p.y).abs() < 1e-6);
    assert!(camera_map_points(camera, false, vec![p; 513]).is_err());
}

#[tokio::test]
async fn typed_edits_are_atomic_exact_retries_and_text_keeps_identity() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    let text = EditCommand::Create {
        object_id: id(20),
        layer_id: id(3),
        shape: NewShape::Text {
            anchor: Point { x: 5.0, y: 10.0 },
            text: "Before".into(),
            font: "Inter".into(),
            size: 10.0,
        },
        style: ObjectStyle {
            rgba: 0x203040ff,
            width: 1.0,
            screen_constant_width: false,
            fill: None,
        },
        transform: Transform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        },
    };
    let first = project
        .apply_edit(edit(21, 1), vec![text.clone()], cancel())
        .await
        .unwrap();
    assert_eq!(
        project
            .apply_edit(edit(21, 1), vec![text], cancel())
            .await
            .unwrap(),
        first
    );
    let update = EditCommand::SetText {
        object_id: id(20),
        text: "After".into(),
        font: "Inter".into(),
        size: 12.0,
    };
    let info = project
        .apply_edit(edit(22, 2), vec![update.clone()], cancel())
        .await
        .unwrap();
    assert_eq!(info.next_lamport, 5);
    assert_eq!(
        project
            .apply_edit(edit(22, 2), vec![update], cancel())
            .await
            .unwrap(),
        info
    );
    assert!(
        project
            .apply_edit(
                edit(22, 2),
                vec![EditCommand::Delete { object_id: id(20) }],
                cancel()
            )
            .await
            .is_err()
    );
    assert_eq!(project.info().await.unwrap(), info);
    let snapshot = project.document_snapshot(id(2), cancel()).await.unwrap();
    assert_eq!(snapshot.render.items[0].object_id, id(20));
    assert!(matches!(&snapshot.render.items[0].shape,DrawShape::Text{text,..} if text=="After"));
    let styled = EditCommand::SetStyle {
        object_id: id(20),
        style: ObjectStyle {
            rgba: 0xff2040ff,
            width: 2.0,
            screen_constant_width: false,
            fill: None,
        },
    };
    let style_info = project
        .apply_edit(edit(25, 8), vec![styled.clone()], cancel())
        .await
        .unwrap();
    assert_eq!(
        project
            .apply_edit(edit(25, 8), vec![styled], cancel())
            .await
            .unwrap(),
        style_info
    );
    assert_eq!(
        project
            .document_snapshot(id(2), cancel())
            .await
            .unwrap()
            .render
            .items[0]
            .style
            .rgba,
        0xff2040ff
    );
    let deleted = project
        .apply_edit(
            edit(23, 9),
            vec![EditCommand::Delete { object_id: id(20) }],
            cancel(),
        )
        .await
        .unwrap();
    assert_eq!(
        project
            .apply_edit(
                edit(23, 9),
                vec![EditCommand::Delete { object_id: id(20) }],
                cancel()
            )
            .await
            .unwrap(),
        deleted
    );
    let undone = project
        .undo_redo(edit(24, 10), false, cancel())
        .await
        .unwrap();
    assert_eq!(
        project
            .undo_redo(edit(24, 10), false, cancel())
            .await
            .unwrap(),
        undone
    );
    assert!(
        project
            .undo_redo(edit(24, 10), true, cancel())
            .await
            .is_err()
    );
    project.close().await.unwrap();
}

#[tokio::test]
async fn stroke_style_width_rebuilds_geometry_and_undo_restores_it() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    draw(&project).await;
    let before = project
        .document_snapshot(id(2), cancel())
        .await
        .unwrap()
        .render
        .items[0]
        .stroke_contours
        .clone();
    let command = EditCommand::SetStyle {
        object_id: id(6),
        style: ObjectStyle {
            rgba: 0x203040ff,
            width: 8.0,
            screen_constant_width: false,
            fill: None,
        },
    };
    let after = project
        .apply_edit(edit(20, 2), vec![command.clone()], cancel())
        .await
        .unwrap();
    assert_eq!(after.next_lamport, 4);
    assert_eq!(
        project
            .apply_edit(edit(20, 2), vec![command], cancel())
            .await
            .unwrap(),
        after
    );
    assert_ne!(
        before,
        project
            .document_snapshot(id(2), cancel())
            .await
            .unwrap()
            .render
            .items[0]
            .stroke_contours
    );
    project
        .undo_redo(edit(21, 4), false, cancel())
        .await
        .unwrap();
    assert_eq!(
        before,
        project
            .document_snapshot(id(2), cancel())
            .await
            .unwrap()
            .render
            .items[0]
            .stroke_contours
    );
    project.close().await.unwrap();
}

#[tokio::test]
async fn source_and_retained_region_pixels_obey_the_callers_export_budget() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    let before = project.info().await.unwrap();
    let mut request = options();
    request.marked = false;
    request.memory_budget_bytes = 1;
    assert!(matches!(
        project.export_image(request.clone(), cancel()).await,
        Err(CoreError::Raster)
    ));
    // The codec's 1px region estimate alone fits, but the retained 64x64 RGBA
    // source must also fit before the codec starts.
    request.region = Some(QueryRect {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    });
    request.memory_budget_bytes = 32 * 1024 * 1024 + 64;
    assert!(matches!(
        project.export_image(request.clone(), cancel()).await,
        Err(CoreError::Raster)
    ));
    request.memory_budget_bytes += 64 * 64 * 4;
    assert!(
        !project
            .export_image(request, cancel())
            .await
            .unwrap()
            .bytes
            .is_empty()
    );
    assert_eq!(project.info().await.unwrap(), before);
    project.close().await.unwrap();
}

#[tokio::test]
async fn absolute_edit_precondition_is_atomic_but_exact_retry_survives_a_new_revision() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    draw(&project).await;
    let before = project.info().await.unwrap();
    let expected = EditPrecondition {
        host_seq: before.host_seq,
        state_hash: before.state_hash.clone(),
    };
    let change = EditCommand::Transform {
        object_id: id(6),
        transform: Transform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 5.0,
            f: 6.0,
        },
    };
    let accepted = project
        .apply_edit_at(
            edit(30, 2),
            vec![change.clone()],
            Some(expected.clone()),
            Some(id(29)),
            cancel(),
        )
        .await
        .unwrap();
    assert_eq!(
        project
            .apply_edit_at(
                edit(30, 2),
                vec![change.clone()],
                Some(expected.clone()),
                Some(id(29)),
                cancel()
            )
            .await
            .unwrap(),
        accepted
    );
    assert!(
        project
            .apply_edit_at(
                edit(31, 3),
                vec![change],
                Some(expected),
                Some(id(28)),
                cancel()
            )
            .await
            .is_err()
    );
    assert_eq!(project.info().await.unwrap(), accepted);
    project.close().await.unwrap();
}
