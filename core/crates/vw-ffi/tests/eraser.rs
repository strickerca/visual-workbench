#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use std::{path::Path, sync::Arc};
use support::*;
use vw_core::*;
use vw_model::{DeviceId, Id};
use vw_proto::{Message, v1 as pb};
static TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
async fn line(project: &Arc<ProjectSession>, family: &str) -> ProjectInfo {
    let mut setup = stroke();
    setup.family = family.into();
    setup.width = 20.0;
    setup.rgba = 0xe0306090;
    setup.stabilization = 0.65;
    let gesture = project.clone().begin_stroke(setup).await.unwrap();
    gesture
        .append_samples(SampleBatch {
            sequence: 1,
            x: vec![8.0, 16.0, 24.0, 32.0, 40.0, 48.0, 56.0],
            y: vec![32.0; 7],
            t_ms: vec![0, 160, 320, 480, 640, 800, 960],
            pressure: vec![1.0; 7],
            tilt: vec![],
            orientation: vec![],
        })
        .await
        .unwrap();
    let revision = gesture.commit(cancel()).await.unwrap();
    gesture.shutdown().await.unwrap();
    revision
}
async fn request(
    project: &ProjectSession,
    n: u8,
    original: u8,
    replacement: u8,
) -> VectorEraseOptions {
    let info = project.info().await.unwrap();
    VectorEraseOptions {
        binding: WorkflowBinding {
            project_id: info.project_id,
            document_id: id(2),
            host_seq: info.host_seq,
            state_hash: info.state_hash,
        },
        metadata: WorkflowMetadata {
            transaction_id: id(n),
            device_id: device(),
            first_lamport: info.next_lamport,
            created_at_ms: TIME + i64::from(n),
        },
        targets: vec![VectorEraseTarget {
            object_id: id(original),
            replacement_id: id(replacement),
        }],
        centers: vec![Point { x: 32.0, y: -10.0 }, Point { x: 32.0, y: 74.0 }],
        radius: 3.0,
        memory_budget_bytes: 256 * 1024 * 1024,
        max_output_vertices: 8192,
        max_work_units: 64_000_000,
    }
}
async fn pixels(project: &ProjectSession) -> Vec<u8> {
    let bytes = project
        .export_image(options(), cancel())
        .await
        .unwrap()
        .bytes;
    let image = vw_raster::decode(&bytes, vw_raster::DecodeLimits::default()).unwrap();
    assert_eq!((image.width, image.height), (64, 64));
    match image.pixels {
        vw_raster::Pixels::Rgba8(bytes) => bytes,
        _ => panic!("expected PNG8 fixture"),
    }
}
async fn object(project: &ProjectSession, n: u8) -> pb::ObjectState {
    let view = project.document_snapshot(id(2), cancel()).await.unwrap();
    let item = view
        .render
        .items
        .iter()
        .find(|item| item.object_id == id(n))
        .unwrap();
    pb::ObjectState::decode(item.object_protobuf.as_slice()).unwrap()
}
fn pixel(bytes: &[u8], x: usize, y: usize) -> &[u8] {
    &bytes[(y * 64 + x) * 4..(y * 64 + x + 1) * 4]
}
fn assert_outside_equal(before: &[u8], after: &[u8]) {
    for y in 0..64 {
        for x in 0..64 {
            if !(26..=38).contains(&x) {
                for (a, b) in pixel(before, x, y).iter().zip(pixel(after, x, y)) {
                    assert!(a.abs_diff(*b) <= 1, "outside coverage changed at ({x},{y})");
                }
            }
        }
    }
}
fn checkpoint(path: &Path) -> vw_ops::HostSequencer {
    let store = vw_store::ProjectStore::open(path).unwrap();
    vw_ops::HostSequencer::from_checkpoint_bytes(&store.checkpoint_bytes().unwrap()).unwrap()
}
fn typed_commit(store: &mut vw_store::ProjectStore, n: u8, lamport: u64, kinds: Vec<pb::op::Kind>) {
    let transaction = pb::Transaction {
        txn_id: Some(Id::try_from(id(n)).unwrap().to_proto()),
        project_id: Some(store.project().id.to_proto()),
        device_id: device(),
        base_revision: Some(store.revision().unwrap()),
        created_at_wall_ms: TIME + i64::from(n),
        gesture_id: None,
        ops: kinds
            .into_iter()
            .enumerate()
            .map(|(index, kind)| pb::Op {
                op_id: Some(pb::OpId {
                    device_id: device(),
                    lamport: lamport + index as u64,
                }),
                kind: Some(kind),
            })
            .collect(),
    };
    store
        .commit(
            &transaction,
            &DeviceId::try_from(device()).unwrap(),
            TIME + i64::from(n),
        )
        .unwrap();
}

#[tokio::test]
async fn highlighter_split_keeps_alpha_outside_and_raw_samples_are_exactly_undoable() {
    let _serial = TESTS.lock().await;
    let dir = temp();
    let project = fixture(&dir.path().join("project")).await;
    let before = line(&project, "highlighter").await;
    let raw = object(&project, 6).await;
    let original = pixels(&project).await;
    let result = project
        .erase_vector_strokes(request(&project, 30, 6, 21).await, cancel())
        .await
        .unwrap();
    assert_eq!(result.revision.host_seq, before.host_seq + 1);
    assert_eq!(result.changed.len(), 1);
    assert_eq!(result.changed[0].outline_id, Some(id(21)));
    assert!(result.changed[0].contour_count >= 2);
    let outline = object(&project, 21).await;
    assert!(matches!(
        outline.shape,
        Some(pb::object_state::Shape::Polygon(_))
    ));
    assert_eq!(outline.layer_id, raw.layer_id);
    assert_eq!(outline.transform, raw.transform);
    assert_eq!(outline.order_key, raw.order_key);
    assert_eq!(outline.style.as_ref().unwrap().width, 0.0);
    assert!(outline.style.as_ref().unwrap().has_fill);
    let erased = pixels(&project).await;
    assert_outside_equal(&original, &erased);
    assert_eq!(pixel(&erased, 32, 32), &[255, 255, 255, 255]);
    assert_ne!(pixel(&original, 32, 32), &[255, 255, 255, 255]);
    let undone = project
        .undo_redo(edit(31, result.revision.next_lamport), false, cancel())
        .await
        .unwrap();
    assert_eq!(object(&project, 6).await, raw);
    assert_eq!(pixels(&project).await, original);
    project
        .undo_redo(edit(32, undone.next_lamport), true, cancel())
        .await
        .unwrap();
    assert_eq!(pixels(&project).await, erased);
    project.close().await.unwrap();
}

#[tokio::test]
async fn repeated_outline_erase_preserves_the_first_hole() {
    let _serial = TESTS.lock().await;
    let dir = temp();
    let project = fixture(&dir.path().join("project")).await;
    line(&project, "marker").await;
    let mut first = request(&project, 30, 6, 21).await;
    first.centers = vec![Point { x: 24.0, y: 32.0 }];
    first.radius = 2.0;
    project.erase_vector_strokes(first, cancel()).await.unwrap();
    let mut second = request(&project, 31, 21, 22).await;
    second.centers = vec![Point { x: 40.0, y: 32.0 }];
    second.radius = 2.0;
    let result = project
        .erase_vector_strokes(second, cancel())
        .await
        .unwrap();
    assert_eq!(result.changed.len(), 1);
    let image = pixels(&project).await;
    assert_eq!(pixel(&image, 24, 32), &[255, 255, 255, 255]);
    assert_eq!(pixel(&image, 40, 32), &[255, 255, 255, 255]);
    assert_ne!(pixel(&image, 32, 32), &[255, 255, 255, 255]);
    project.close().await.unwrap();
}

#[tokio::test]
async fn complete_erasure_deletes_once_without_an_invalid_empty_polygon() {
    let _serial = TESTS.lock().await;
    let dir = temp();
    let project = fixture(&dir.path().join("project")).await;
    let before = line(&project, "pen").await;
    let mut edit = request(&project, 30, 6, 21).await;
    edit.centers = vec![Point { x: 32.0, y: 32.0 }];
    edit.radius = 64.0;
    let result = project.erase_vector_strokes(edit, cancel()).await.unwrap();
    assert_eq!(result.revision.host_seq, before.host_seq + 1);
    assert_eq!(result.changed[0].outline_id, None);
    assert_eq!(result.changed[0].contour_count, 0);
    assert!(
        project
            .document_snapshot(id(2), cancel())
            .await
            .unwrap()
            .render
            .items
            .is_empty()
    );
    assert!(pixels(&project).await.iter().all(|value| *value == 255));
    project.close().await.unwrap();
}

#[tokio::test]
async fn no_hit_cancel_and_all_admission_failures_leave_the_exact_revision() {
    let _serial = TESTS.lock().await;
    let dir = temp();
    let project = fixture(&dir.path().join("project")).await;
    let before = line(&project, "pen").await;
    let request = request(&project, 30, 6, 21).await;
    let stopped = cancel();
    stopped.cancel();
    assert!(matches!(
        project.erase_vector_strokes(request.clone(), stopped).await,
        Err(WorkflowError::Cancelled)
    ));
    let mut small = request.clone();
    small.memory_budget_bytes = 1;
    assert!(matches!(
        project.erase_vector_strokes(small, cancel()).await,
        Err(WorkflowError::Memory { .. })
    ));
    let mut work = request.clone();
    work.max_work_units = 1;
    assert!(matches!(
        project.erase_vector_strokes(work, cancel()).await,
        Err(WorkflowError::Limit)
    ));
    let mut vertices = request.clone();
    vertices.max_output_vertices = 3;
    assert!(matches!(
        project.erase_vector_strokes(vertices, cancel()).await,
        Err(WorkflowError::Limit)
    ));
    let mut no_hit = request;
    no_hit.centers = vec![Point { x: 500.0, y: 500.0 }];
    let result = project
        .erase_vector_strokes(no_hit, cancel())
        .await
        .unwrap();
    assert!(result.changed.is_empty());
    assert_eq!(result.revision, before);
    assert_eq!(project.info().await.unwrap(), before);
    project.close().await.unwrap();
}

#[tokio::test]
async fn stale_visible_binding_and_reused_transaction_cannot_replay_different_intent() {
    let _serial = TESTS.lock().await;
    let dir = temp();
    let project = fixture(&dir.path().join("project")).await;
    line(&project, "marker").await;
    let stale = request(&project, 30, 6, 21).await;
    let fresh = request(&project, 31, 6, 22).await;
    let accepted = project.erase_vector_strokes(fresh, cancel()).await.unwrap();
    assert!(matches!(
        project.erase_vector_strokes(stale, cancel()).await,
        Err(WorkflowError::Stale)
    ));
    let reused = request(&project, 31, 22, 23).await;
    assert!(matches!(
        project.erase_vector_strokes(reused, cancel()).await,
        Err(WorkflowError::ReusedTransaction)
    ));
    assert_eq!(project.info().await.unwrap(), accepted.revision);
    project.close().await.unwrap();
}

#[tokio::test]
async fn missing_later_target_is_an_atomic_failure_for_the_whole_gesture() {
    let _serial = TESTS.lock().await;
    let dir = temp();
    let project = fixture(&dir.path().join("project")).await;
    let before = line(&project, "pen").await;
    let original = pixels(&project).await;
    let mut request = request(&project, 30, 6, 21).await;
    request.targets.push(VectorEraseTarget {
        object_id: id(90),
        replacement_id: id(91),
    });
    assert!(matches!(
        project.erase_vector_strokes(request, cancel()).await,
        Err(WorkflowError::Missing)
    ));
    assert_eq!(project.info().await.unwrap(), before);
    assert_eq!(pixels(&project).await, original);
    project.close().await.unwrap();
}

#[tokio::test]
async fn transformed_stroke_is_erased_in_document_space_without_rewriting_its_transform() {
    let _serial = TESTS.lock().await;
    let dir = temp();
    let project = fixture(&dir.path().join("project")).await;
    let info = line(&project, "marker").await;
    let transform = Transform {
        a: 0.5,
        b: 0.0,
        c: 0.0,
        d: 1.5,
        e: 16.0,
        f: -16.0,
    };
    project
        .apply_edit(
            edit(29, info.next_lamport),
            vec![EditCommand::Transform {
                object_id: id(6),
                transform,
            }],
            cancel(),
        )
        .await
        .unwrap();
    let raw = object(&project, 6).await;
    let mut edit = request(&project, 30, 6, 21).await;
    edit.centers = vec![Point { x: 32.0, y: 32.0 }];
    edit.radius = 3.0;
    project.erase_vector_strokes(edit, cancel()).await.unwrap();
    assert_eq!(object(&project, 21).await.transform, raw.transform);
    let image = pixels(&project).await;
    assert_eq!(pixel(&image, 32, 32), &[255, 255, 255, 255]);
    assert_ne!(pixel(&image, 24, 32), &[255, 255, 255, 255]);
    project.close().await.unwrap();
}

#[tokio::test]
async fn locked_source_is_refused_and_equal_order_keys_cannot_change_alpha_order() {
    let _serial = TESTS.lock().await;
    let dir = temp();
    let path = dir.path().join("project");
    let project = fixture(&path).await;
    let info = line(&project, "pen").await;
    let source = object(&project, 6).await;
    project.close().await.unwrap();
    {
        let mut store = vw_store::ProjectStore::open(&path).unwrap();
        typed_commit(
            &mut store,
            29,
            info.next_lamport,
            vec![pb::op::Kind::SetProperty(pb::SetProperty {
                object_id: source.object_id.clone(),
                property: "locked".into(),
                value: Some(pb::PropertyValue {
                    value: Some(pb::property_value::Value::Flag(true)),
                }),
            })],
        );
    }
    let project = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let before = project.info().await.unwrap();
    assert!(matches!(
        project
            .erase_vector_strokes(request(&project, 30, 6, 21).await, cancel())
            .await,
        Err(WorkflowError::Locked)
    ));
    assert_eq!(project.info().await.unwrap(), before);
    project.close().await.unwrap();
    {
        let mut store = vw_store::ProjectStore::open(&path).unwrap();
        let mut peer = source.clone();
        peer.object_id = Some(Id::try_from(id(10)).unwrap().to_proto());
        typed_commit(
            &mut store,
            31,
            before.next_lamport,
            vec![
                pb::op::Kind::SetProperty(pb::SetProperty {
                    object_id: source.object_id.clone(),
                    property: "locked".into(),
                    value: Some(pb::PropertyValue {
                        value: Some(pb::property_value::Value::Flag(false)),
                    }),
                }),
                pb::op::Kind::CreateObject(pb::CreateObject {
                    document_id: Some(Id::try_from(id(2)).unwrap().to_proto()),
                    state: Some(peer),
                }),
            ],
        );
    }
    let project = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let before = project.info().await.unwrap();
    assert!(matches!(
        project
            .erase_vector_strokes(request(&project, 32, 6, 21).await, cancel())
            .await,
        Err(WorkflowError::Invalid)
    ));
    assert_eq!(project.info().await.unwrap(), before);
    project.close().await.unwrap();
}

#[tokio::test]
async fn durable_reopen_checkpoint_replica_replay_and_undo_preserve_outline_and_original() {
    let _serial = TESTS.lock().await;
    let dir = temp();
    let path = dir.path().join("project");
    let project = fixture(&path).await;
    line(&project, "marker").await;
    let raw = object(&project, 6).await;
    project.close().await.unwrap();
    let before = checkpoint(&path);
    let mut replica = vw_ops::Replica::new(
        DeviceId::try_from(device()).unwrap(),
        before.snapshot().unwrap(),
    )
    .unwrap();
    let project = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let result = project
        .erase_vector_strokes(request(&project, 30, 6, 21).await, cancel())
        .await
        .unwrap();
    let image = pixels(&project).await;
    project.close().await.unwrap();
    let after = checkpoint(&path);
    let mut accepted = after
        .accepted_transactions()
        .filter(|(_, ack)| ack.host_seq > before.revision().unwrap().host_seq)
        .collect::<Vec<_>>();
    accepted.sort_by_key(|(_, ack)| ack.host_seq);
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].0.ops.len(), 2);
    let mut replay = before.clone();
    for (transaction, _) in accepted {
        replay
            .submit(
                transaction.clone(),
                &DeviceId::try_from(device()).unwrap(),
                transaction.created_at_wall_ms,
            )
            .unwrap();
    }
    assert_eq!(replay.project(), after.project());
    assert_eq!(replay.revision().unwrap(), after.revision().unwrap());
    replica.receive(after.snapshot().unwrap()).unwrap();
    assert_eq!(replica.project(), after.project());
    assert_eq!(replica.revision(), &after.revision().unwrap());
    let project = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    assert_eq!(project.info().await.unwrap(), result.revision);
    assert_eq!(pixels(&project).await, image);
    project
        .undo_redo(edit(31, result.revision.next_lamport), false, cancel())
        .await
        .unwrap();
    assert_eq!(object(&project, 6).await, raw);
    project.close().await.unwrap();
}

#[test]
fn borrowed_new_transaction_admission_counts_many_small_values_and_stops_at_limit() {
    let mut transaction = pb::Transaction::default();
    let empty = vw_ops::HostSequencer::transaction_workspace_estimate_bytes(&transaction, u64::MAX)
        .unwrap();
    transaction.ops = vec![pb::Op::default(); 1000];
    let many = vw_ops::HostSequencer::transaction_workspace_estimate_bytes(&transaction, u64::MAX)
        .unwrap();
    assert!(many > empty + 256_000);
    assert_eq!(
        vw_ops::HostSequencer::transaction_workspace_estimate_bytes(&transaction, 8192).unwrap(),
        8193
    );
}
