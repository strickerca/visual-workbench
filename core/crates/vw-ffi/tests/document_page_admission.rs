#![allow(clippy::unwrap_used)] // Deterministic native admission fixtures.
mod support;
use support::*;
use vw_core::*;

async fn expected(project: &ProjectSession, page: Option<u32>) -> EditPrecondition {
    let info = project.info().await.unwrap();
    EditPrecondition {
        host_seq: info.host_seq,
        state_hash: info.state_hash,
        page_index: page,
    }
}

#[tokio::test]
async fn same_revision_same_size_pages_cannot_delete_or_erase_each_others_objects() {
    let directory = tempfile::tempdir().unwrap();
    // Parsing is deliberately outside this test; these are already-admitted
    // immutable source bytes and validated page geometry.
    let source = b"%PDF-synthetic-two-page-admission".to_vec();
    let project = create_editable_document(
        CreateEditableDocument {
            path: directory.path().join("project").to_string_lossy().into(),
            project_id: id(1),
            document_id: id(2),
            device_id: device(),
            title: "two same-sized pages".into(),
            source_blake3: vw_model::AssetId::hash(&source).to_string(),
            source,
            kind: EditableDocumentKind::Pdf,
            now_ms: TIME,
            pages: (0..2)
                .map(|page| DocumentPageGeometry {
                    page,
                    width: 64.0,
                    height: 64.0,
                    crop: vec![0.0, 0.0, 64.0, 64.0],
                    quarter_turns: 0,
                    layer_id: id(3 + page as u8),
                })
                .collect(),
        },
        cancel(),
    )
    .await
    .unwrap();
    for page in 0..2 {
        let info = project.info().await.unwrap();
        let mut options = stroke();
        options.layer_id = id(3 + page as u8);
        options.object_id = id(10 + page as u8);
        options.gesture_id = id(20 + page as u8);
        options.transaction_id = id(30 + page as u8);
        options.lamport = info.next_lamport;
        let gesture = project.clone().begin_stroke(options).await.unwrap();
        gesture
            .append_samples(SampleBatch {
                sequence: 1,
                x: vec![8.0, 24.0, 48.0],
                y: vec![24.0; 3],
                t_ms: vec![0, 16, 32],
                pressure: vec![1.0; 3],
                tilt: vec![],
                orientation: vec![],
            })
            .await
            .unwrap();
        gesture.commit(cancel()).await.unwrap();
        gesture.shutdown().await.unwrap();
    }
    let first = project
        .editable_document_page(id(2), 0, cancel())
        .await
        .unwrap();
    let second = project
        .editable_document_page(id(2), 1, cancel())
        .await
        .unwrap();
    assert_eq!((first.width, first.height), (second.width, second.height));
    assert_eq!(first.render.revision, second.render.revision);
    assert_eq!(first.render.items[0].object_id, id(10));
    assert_eq!(second.render.items[0].object_id, id(11));
    let before = project.info().await.unwrap();
    for page in [Some(1), None] {
        assert!(matches!(
            project
                .apply_edit_at(
                    edit(40, before.next_lamport),
                    vec![EditCommand::Delete { object_id: id(10) }],
                    Some(expected(&project, page).await),
                    None,
                    cancel()
                )
                .await,
            Err(CoreError::Invalid)
        ));
        assert_eq!(project.info().await.unwrap(), before);
    }
    for page in [Some(1), None] {
        assert!(
            project
                .erase_vector_strokes(
                    VectorEraseOptions {
                        binding: WorkflowBinding {
                            project_id: id(1),
                            document_id: id(2),
                            host_seq: before.host_seq,
                            state_hash: before.state_hash.clone()
                        },
                        metadata: WorkflowMetadata {
                            transaction_id: id(41),
                            device_id: device(),
                            first_lamport: before.next_lamport,
                            created_at_ms: TIME + 41
                        },
                        targets: vec![VectorEraseTarget {
                            object_id: id(10),
                            replacement_id: id(50)
                        }],
                        centers: vec![Point { x: 24.0, y: 24.0 }],
                        radius: 8.0,
                        memory_budget_bytes: 256 * 1024 * 1024,
                        max_output_vertices: 8192,
                        max_work_units: 64_000_000,
                        page_index: page,
                    },
                    cancel()
                )
                .await
                .is_err()
        );
        assert_eq!(project.info().await.unwrap(), before);
    }
    // Positive control uses the selected current page with the identical
    // source/revision: page admission must not reject every vector erase.
    let erased = project
        .erase_vector_strokes(
            VectorEraseOptions {
                binding: WorkflowBinding {
                    project_id: id(1),
                    document_id: id(2),
                    host_seq: before.host_seq,
                    state_hash: before.state_hash.clone(),
                },
                metadata: WorkflowMetadata {
                    transaction_id: id(43),
                    device_id: device(),
                    first_lamport: before.next_lamport,
                    created_at_ms: TIME + 43,
                },
                targets: vec![VectorEraseTarget {
                    object_id: id(11),
                    replacement_id: id(51),
                }],
                centers: vec![Point { x: 24.0, y: 24.0 }],
                radius: 8.0,
                memory_budget_bytes: 256 * 1024 * 1024,
                max_output_vertices: 8192,
                max_work_units: 64_000_000,
                page_index: Some(1),
            },
            cancel(),
        )
        .await
        .unwrap();
    assert_eq!(erased.changed.len(), 1);
    assert_eq!(erased.changed[0].original_id, id(11));
    let remaining = erased.changed[0].outline_id.clone().unwrap();
    let before = project.info().await.unwrap();
    let options = edit(42, before.next_lamport);
    let condition = expected(&project, Some(1)).await;
    let commands = vec![EditCommand::Delete {
        object_id: remaining,
    }];
    let saved = project
        .apply_edit_at(
            options.clone(),
            commands.clone(),
            Some(condition.clone()),
            None,
            cancel(),
        )
        .await
        .unwrap();
    assert_eq!(
        project
            .apply_edit_at(options, commands, Some(condition), None, cancel())
            .await
            .unwrap(),
        saved
    );
    assert_eq!(
        project
            .editable_document_page(id(2), 0, cancel())
            .await
            .unwrap()
            .render
            .items[0]
            .object_id,
        id(10)
    );
    assert!(
        project
            .editable_document_page(id(2), 1, cancel())
            .await
            .unwrap()
            .render
            .items
            .is_empty()
    );
    project.close().await.unwrap();
}
