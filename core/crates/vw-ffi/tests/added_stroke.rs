//! A stroke saved with `commit_added` must come back exactly as a read of its
//! document lists it, so that a caller holding the document at the revision
//! just before the save holds it at the revision after the save without
//! reading it again. Whenever that cannot be shown the save says so, and the
//! caller reads.
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
async fn numbered(project: &Arc<ProjectSession>, number: u32, family: &str) -> StrokeOptions {
    let info = project.info().await.unwrap();
    let mut options = stroke();
    options.gesture_id = wide_id(0x41, number);
    options.transaction_id = wide_id(0x42, number);
    options.object_id = wide_id(0x60, number);
    options.device_id = info.device_id;
    options.lamport = info.next_lamport;
    options.created_at_ms = TIME + 10 + i64::from(number);
    options.family = family.into();
    options
}
/// One stroke, saved with the call under test. A highlighter needs the
/// project before its first ink; every other stroke begins without it.
async fn save_stroke(project: &Arc<ProjectSession>, number: u32, family: &str) -> StrokeSave {
    let options = numbered(project, number, family).await;
    let gesture = if family == "highlighter" {
        project.clone().begin_stroke(options).await.unwrap()
    } else {
        project.clone().begin_stroke_deferred(options).unwrap()
    };
    let mut samples = batch();
    for x in &mut samples.x {
        *x += f64::from(number);
    }
    gesture.append_samples(samples).await.unwrap();
    let saved = gesture.commit_added(cancel()).await.unwrap();
    gesture.shutdown().await.unwrap();
    saved
}
async fn edit_once(project: &Arc<ProjectSession>, number: u32, commands: Vec<EditCommand>) {
    let options = EditOptions {
        transaction_id: wide_id(0x43, number),
        document_id: id(2),
        device_id: device(),
        lamport: project.info().await.unwrap().next_lamport,
        created_at_ms: TIME + 100 + i64::from(number),
    };
    project
        .apply_edit(options, commands, cancel())
        .await
        .unwrap();
}
async fn undo(project: &Arc<ProjectSession>, number: u32) {
    let options = EditOptions {
        transaction_id: wide_id(0x44, number),
        document_id: id(2),
        device_id: device(),
        lamport: project.info().await.unwrap().next_lamport,
        created_at_ms: TIME + 200 + i64::from(number),
    };
    project.undo_redo(options, false, cancel()).await.unwrap();
}
async fn update_layer(project: &Arc<ProjectSession>, number: u32, visible: bool) {
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
                    visible: Some(visible),
                    locked: None,
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
fn rectangle(number: u32) -> EditCommand {
    EditCommand::Create {
        object_id: wide_id(0x61, number),
        layer_id: id(3),
        shape: NewShape::Rectangle {
            rectangle: QueryRect {
                x: 5.0,
                y: 6.0,
                width: 12.0,
                height: 13.0,
            },
        },
        style: ObjectStyle {
            rgba: 0x123456ff,
            width: 2.0,
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
    }
}
async fn read(project: &Arc<ProjectSession>) -> PackedDocumentSnapshot {
    project
        .document_snapshot_packed(id(2), cancel())
        .await
        .unwrap()
}

/// The caller's side: the document it holds, and the only way it may change
/// that document without reading it.
struct Held {
    revision: ProjectInfo,
    layers: Vec<String>,
    items: Vec<PackedRenderItem>,
}
impl Held {
    fn of(snapshot: PackedDocumentSnapshot) -> Self {
        Self {
            revision: snapshot.render.revision,
            layers: snapshot.layers.into_iter().map(|layer| layer.id).collect(),
            items: snapshot.render.items,
        }
    }
    /// Adds the saved stroke when the save directly followed what is held.
    fn adopt(&mut self, saved: &StrokeSave) -> bool {
        let Some(added) = &saved.added else {
            return false;
        };
        if added.previous != self.revision {
            return false;
        }
        let order = |layer: &str| self.layers.iter().position(|id| id == layer);
        let Some(layer) = order(&added.item.layer_id) else {
            return false;
        };
        // After every item of its own layer and of the layers under it.
        let mut index = self.items.len();
        while index > 0 && order(&self.items[index - 1].layer_id) > Some(layer) {
            index -= 1;
        }
        self.items.insert(index, added.item.clone());
        self.revision = saved.revision.clone();
        true
    }
    /// What is held is, field for field, what a complete read returns now.
    async fn assert_is_a_read(&self, project: &Arc<ProjectSession>) {
        let now = read(project).await;
        assert_eq!(self.revision, now.render.revision);
        assert_eq!(
            self.layers,
            now.layers
                .iter()
                .map(|layer| layer.id.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(self.items.len(), now.render.items.len());
        for (held, listed) in self.items.iter().zip(&now.render.items) {
            assert_eq!(format!("{held:?}"), format!("{listed:?}"));
        }
    }
}

#[tokio::test]
async fn added_strokes_keep_a_held_document_equal_to_a_read_through_edits_and_undo() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    let mut held = Held::of(read(&project).await);
    let mut reads = 0;

    // Stroke after stroke, nothing is read.
    for number in 1..=3 {
        let before = project.info().await.unwrap();
        let saved = save_stroke(&project, number, if number == 2 { "marker" } else { "pen" }).await;
        let added = saved.added.as_ref().expect("a plain stroke is only added");
        assert_eq!(added.previous, before);
        assert_eq!(saved.revision, project.info().await.unwrap());
        assert_eq!(added.item.object_id, wide_id(0x60, number));
        assert!(held.adopt(&saved));
        held.assert_is_a_read(&project).await;
        // The digest and cost are the ones a changes read gives for it, so a
        // later changes read keeps the stroke instead of sending it again.
        let changes = project
            .document_snapshot_changes_packed(id(2), vec![], cancel())
            .await
            .unwrap();
        let entry = changes
            .items
            .iter()
            .find(|entry| entry.object_id == added.item.object_id)
            .unwrap();
        assert_eq!(entry.digest.as_ref(), Some(&added.digest));
        assert_eq!(entry.cost, added.cost);
        let kept = project
            .document_snapshot_changes_packed(
                id(2),
                vec![KnownStroke {
                    object_id: added.item.object_id.clone(),
                    digest: added.digest.clone(),
                    cost: added.cost,
                }],
                cancel(),
            )
            .await
            .unwrap();
        assert!(
            kept.items
                .iter()
                .find(|entry| entry.object_id == added.item.object_id)
                .unwrap()
                .item
                .is_none()
        );
    }

    // An edit the caller has not read yet: the next save does not follow what
    // it holds, so it must read. After that read it is in step again.
    edit_once(&project, 1, vec![rectangle(1)]).await;
    let saved = save_stroke(&project, 4, "pen").await;
    assert!(saved.added.is_some());
    assert!(!held.adopt(&saved));
    held = Held::of(read(&project).await);
    reads += 1;
    let saved = save_stroke(&project, 5, "pen").await;
    assert!(held.adopt(&saved));
    held.assert_is_a_read(&project).await;

    // Undo takes the last stroke out again; a stroke saved after it does not
    // follow the document that still shows the undone one.
    undo(&project, 1).await;
    let saved = save_stroke(&project, 6, "pen").await;
    assert!(!held.adopt(&saved));
    held = Held::of(read(&project).await);
    reads += 1;
    assert!(
        held.items
            .iter()
            .all(|item| item.object_id != wide_id(0x60, 5))
    );

    // Moving and deleting earlier strokes, read once, then strokes again.
    edit_once(
        &project,
        2,
        vec![
            EditCommand::Transform {
                object_id: wide_id(0x60, 1),
                transform: Transform {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    e: 7.0,
                    f: -3.0,
                },
            },
            EditCommand::Delete {
                object_id: wide_id(0x60, 2),
            },
        ],
    )
    .await;
    held = Held::of(read(&project).await);
    reads += 1;
    for number in 7..=9 {
        let saved = save_stroke(&project, number, "pen").await;
        assert!(held.adopt(&saved));
        held.assert_is_a_read(&project).await;
    }
    assert_eq!(reads, 3);
    project.close().await.unwrap();
}

#[tokio::test]
async fn a_stroke_under_another_layer_is_listed_where_a_read_lists_it() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    let mut held = Held::of(read(&project).await);
    // The first highlighter stroke makes its multiply layer: more than an
    // added stroke, so the caller is told to read.
    let first = save_stroke(&project, 1, "highlighter").await;
    assert!(first.added.is_none());
    assert!(!held.adopt(&first));
    held = Held::of(read(&project).await);
    assert_eq!(held.layers.len(), 2);
    // A pen stroke on the layer underneath goes before the highlighter mark.
    let pen = save_stroke(&project, 2, "pen").await;
    assert!(held.adopt(&pen));
    assert_eq!(
        held.items
            .iter()
            .map(|item| item.object_id.clone())
            .collect::<Vec<_>>(),
        vec![wide_id(0x60, 2), wide_id(0x60, 1)]
    );
    held.assert_is_a_read(&project).await;
    // A highlighter stroke on the layer that now exists is only added, on top.
    let second = save_stroke(&project, 3, "highlighter").await;
    let added = second.added.as_ref().unwrap();
    assert_eq!(added.item.layer_id, held.layers[1]);
    assert_eq!(added.item.layer_blend, "multiply");
    assert!(held.adopt(&second));
    held.assert_is_a_read(&project).await;
    let pen = save_stroke(&project, 4, "pen").await;
    assert!(held.adopt(&pen));
    held.assert_is_a_read(&project).await;
    project.close().await.unwrap();
}

#[tokio::test]
async fn a_stroke_a_read_does_not_list_and_a_repeated_save_are_not_added() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    // The layer is hidden after the pen went down: the stroke is saved, and a
    // read of the document does not list it.
    let mut options = numbered(&project, 1, "pen").await;
    options.lamport += 16;
    let gesture = project.clone().begin_stroke(options).await.unwrap();
    gesture.append_samples(batch()).await.unwrap();
    update_layer(&project, 1, false).await;
    let saved = gesture.commit_added(cancel()).await.unwrap();
    assert!(saved.added.is_none());
    assert_eq!(saved.revision, project.info().await.unwrap());
    assert!(read(&project).await.render.items.is_empty());
    gesture.shutdown().await.unwrap();
    update_layer(&project, 2, true).await;
    assert_eq!(read(&project).await.render.items.len(), 1);

    // An exact retry of a finished save returns its revision and nothing to
    // add: it cannot show what it followed.
    let gesture = project
        .clone()
        .begin_stroke_deferred(numbered(&project, 2, "pen").await)
        .unwrap();
    gesture.append_samples(batch()).await.unwrap();
    let saved = gesture.commit_added(cancel()).await.unwrap();
    assert!(saved.added.is_some());
    let again = gesture.commit_added(cancel()).await.unwrap();
    assert_eq!(again.revision, saved.revision);
    assert!(again.added.is_none());
    // The plain call is unchanged.
    assert_eq!(gesture.commit(cancel()).await.unwrap(), saved.revision);
    gesture.shutdown().await.unwrap();
    project.close().await.unwrap();
}

#[tokio::test]
async fn a_mirror_queues_its_stroke_and_is_told_to_read() {
    let dir = tempfile::tempdir().unwrap();
    let (host_service, client_service) = paired().await;
    let host = create_image_project(create(&dir.path().join("host")), cancel())
        .await
        .unwrap();
    let client_id = client_service.local_device().await.unwrap().device_id;
    let server = host_service
        .host_project(
            host.clone(),
            client_id.clone(),
            vec![SessionEndpoint {
                carrier: AppCarrier::TcpAdb,
                address: "127.0.0.1:0".into(),
            }],
        )
        .await
        .unwrap();
    let client = client_service
        .receive_project(
            ReceiveProjectOptions {
                path: dir.path().join("client").to_string_lossy().into(),
                peer_device_id: device(),
                endpoints: server.endpoints(),
                expected_project_id: Some(id(1)),
                max_total_blob_bytes: 16 * 1024 * 1024,
            },
            cancel(),
        )
        .await
        .unwrap();
    assert_eq!(client.info().await.unwrap().device_id, client_id);
    // The mirror shows its stroke at once but has not saved it: the host has
    // yet to accept it. That is not "the document plus one stroke".
    let saved = save_stroke(&client, 1, "pen").await;
    assert!(saved.added.is_none());
    assert_eq!(saved.revision, client.info().await.unwrap());
    assert_eq!(read(&client).await.render.items.len(), 1);
    // The host, which saves its own strokes, hands them back.
    assert!(save_stroke(&host, 2, "pen").await.added.is_some());
    server.close().await.unwrap();
    client.close().await.unwrap();
    host.close().await.unwrap();
}

#[tokio::test]
async fn many_strokes_may_wait_to_be_saved_and_are_saved_in_the_order_asked() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    let mut held = Held::of(read(&project).await);
    // Far more strokes than are ever drawn at once begin and take ink before
    // the first of them is saved.
    let first = project.info().await.unwrap().next_lamport;
    let mut waiting = Vec::new();
    for number in 0..MAX_ACTIVE_GESTURES as u32 {
        let mut options = numbered(&project, number + 1, "pen").await;
        options.lamport = first + u64::from(number) * 3;
        let gesture = project.clone().begin_stroke_deferred(options).unwrap();
        gesture.append_samples(batch()).await.unwrap();
        waiting.push(gesture);
    }
    // One more is refused until a stroke is shut down; nothing is lost.
    let mut extra = numbered(&project, 1_000, "pen").await;
    extra.lamport = first + 1_000;
    assert!(matches!(
        project.clone().begin_stroke_deferred(extra.clone()),
        Err(CoreError::Backpressure)
    ));
    assert!(matches!(
        project.clone().begin_stroke(extra).await,
        Err(CoreError::Backpressure)
    ));
    for (index, gesture) in waiting.iter().enumerate() {
        let saved = gesture.commit_added(cancel()).await.unwrap();
        assert!(held.adopt(&saved), "stroke {index} follows the one before");
    }
    held.assert_is_a_read(&project).await;
    assert_eq!(
        held.items
            .iter()
            .map(|item| item.object_id.clone())
            .collect::<Vec<_>>(),
        (1..=MAX_ACTIVE_GESTURES as u32)
            .map(|number| wide_id(0x60, number))
            .collect::<Vec<_>>()
    );
    for gesture in waiting {
        gesture.shutdown().await.unwrap();
    }
    let later = save_stroke(&project, 2_000, "pen").await;
    assert!(held.adopt(&later));
    held.assert_is_a_read(&project).await;
    project.close().await.unwrap();
}
