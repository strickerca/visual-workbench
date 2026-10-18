//! Synthetic persistence, rollback, recovery and projection integration tests.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::{fs, path::Path};
use tempfile::TempDir;
use vw_model::{
    AssetId, DeviceId, Document, Group, Id, Instruction, Layer, Object, PdfPage, Project,
    ResultCandidate, SemanticSnapshot,
};
use vw_proto::{Message, v1 as pb};
use vw_store::{CommitStage, ProjectStore, StoreError};

fn temporary() -> TempDir {
    let mut builder = tempfile::Builder::new();
    builder.prefix("vw-database-test-");
    #[cfg(target_os = "android")]
    {
        builder
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap()
    }
    #[cfg(not(target_os = "android"))]
    {
        builder.tempdir().unwrap()
    }
}
fn id(number: u64) -> Id {
    Id::from_parts(1_700_000_000_000 + number, [7; 10]).unwrap()
}
fn device(number: u8) -> DeviceId {
    DeviceId::from_bytes([number; 16])
}
fn project() -> Project {
    let mut value = Project::new(id(1), "Database fixture".into(), device(1));
    value.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(2).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Before".into(),
                ..Default::default()
            },
            pages: vec![],
            created_at_ms: 0,
        },
    );
    value.layers.insert(
        id(5),
        Layer {
            definition: pb::CreateLayer {
                layer_id: Some(id(5).to_proto()),
                document_id: Some(id(2).to_proto()),
                page_index: -1,
                name: "Annotations".into(),
                kind: "annotation".into(),
                order_key: "V".into(),
            },
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: "normal".into(),
        },
    );
    value.objects.insert(
        id(6),
        Object {
            document_id: id(2),
            state: pb::ObjectState {
                object_id: Some(id(6).to_proto()),
                layer_id: Some(id(5).to_proto()),
                order_key: "V".into(),
                transform: Some(pb::Affine {
                    a: 1.0,
                    d: 1.0,
                    ..Default::default()
                }),
                style: Some(pb::Style {
                    stroke: Some(pb::Color { rgba: 0x123456ff }),
                    width: 2.0,
                    ..Default::default()
                }),
                role: pb::Role::None as i32,
                created_by: device(1).to_string(),
                shape: Some(pb::object_state::Shape::Rect(pb::RectD {
                    x: 0.0,
                    y: 0.0,
                    w: 10.0,
                    h: 10.0,
                })),
                ..Default::default()
            },
        },
    );
    value
}
fn create(path: &Path) -> ProjectStore {
    ProjectStore::create(path, project(), device(1), 0).unwrap()
}
fn transaction(
    revision: pb::Revision,
    number: u64,
    who: u8,
    kind: pb::op::Kind,
) -> pb::Transaction {
    pb::Transaction {
        txn_id: Some(id(number).to_proto()),
        project_id: Some(id(1).to_proto()),
        device_id: device(who).to_string(),
        base_revision: Some(revision),
        created_at_wall_ms: number as i64,
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: device(who).to_string(),
                lamport: number,
            }),
            kind: Some(kind),
        }],
        ..Default::default()
    }
}
fn title(revision: pb::Revision, number: u64) -> pb::Transaction {
    transaction(
        revision,
        number,
        1,
        pb::op::Kind::UpdateDocument(pb::UpdateDocument {
            document_id: Some(id(2).to_proto()),
            title: format!("Edit {number}"),
        }),
    )
}
fn width(revision: pb::Revision, number: u64, who: u8, value: f64) -> pb::Transaction {
    transaction(
        revision,
        number,
        who,
        pb::op::Kind::SetProperty(pb::SetProperty {
            object_id: Some(id(6).to_proto()),
            property: "style.width".into(),
            value: Some(pb::PropertyValue {
                value: Some(pb::property_value::Value::Number(value)),
            }),
        }),
    )
}
fn query(path: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open(path.join("project.sqlite")).unwrap()
}

#[test]
fn reopen_retains_exact_retry_ack_undo_and_conflict_records() {
    let temporary = temporary();
    let path = temporary.path().join("project.vwb");
    let mut store = create(&path);
    let baseline = store.revision().unwrap();
    let host = width(baseline.clone(), 10, 1, 3.0);
    store.commit(&host, &device(1), 100).unwrap();
    let offline = width(baseline, 11, 2, 4.0);
    let accepted = store.commit(&offline, &device(2), 101).unwrap();
    assert_eq!(accepted.conflicts.len(), 1);
    assert_eq!(store.conflicts().unwrap(), accepted.conflicts);
    let undo = transaction(
        store.revision().unwrap(),
        12,
        2,
        pb::op::Kind::UndoTransaction(pb::UndoTransaction {
            target_txn_id: offline.txn_id.clone(),
        }),
    );
    let undo_ack = store.commit(&undo, &device(2), 102).unwrap();
    assert_eq!(
        store.project().objects[&id(6)]
            .state
            .style
            .as_ref()
            .unwrap()
            .width,
        3.0
    );
    let expected = store.project().state_hash().unwrap();
    store.snapshot_now(103).unwrap();
    drop(store);
    let mut restored = ProjectStore::open(&path).unwrap();
    assert_eq!(restored.replayed_transactions(), 0);
    assert_eq!(restored.project().state_hash().unwrap(), expected);
    assert_eq!(restored.conflicts().unwrap(), accepted.conflicts);
    let retry = restored.commit(&offline, &device(2), 104).unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.ack, accepted.ack);
    assert!(retry.conflicts.is_empty());
    let retry = restored.commit(&undo, &device(2), 105).unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.ack, undo_ack.ack);
    assert_eq!(restored.project().state_hash().unwrap(), expected);
    restored.integrity_check().unwrap();
    drop(restored);
    temporary.close().unwrap();
}

#[test]
fn pending_queue_survives_restart_in_order_and_deletes_only_exact_acked_bytes() {
    let temporary = temporary();
    let path = temporary.path().join("project.vwb");
    let mut store = create(&path);
    let first = title(store.revision().unwrap(), 10);
    let second = title(store.revision().unwrap(), 11);
    let sequence = store.enqueue_pending(&first).unwrap();
    assert_eq!(store.enqueue_pending(&first).unwrap(), sequence);
    assert!(store.enqueue_pending(&second).unwrap() > sequence);
    let mut collision = first.clone();
    collision.created_at_wall_ms += 1;
    assert!(store.enqueue_pending(&collision).is_err());
    drop(store);
    let mut restored = ProjectStore::open(&path).unwrap();
    assert_eq!(
        restored.pending().unwrap(),
        vec![first.clone(), second.clone()]
    );
    let ack = restored.commit(&first, &device(1), 100).unwrap();
    assert!(!restored.acknowledge_pending(&collision, &ack.ack).unwrap());
    assert_eq!(restored.pending().unwrap().len(), 2);
    assert!(restored.acknowledge_pending(&first, &ack.ack).unwrap());
    assert!(!restored.acknowledge_pending(&first, &ack.ack).unwrap());
    let mut invalid = ack.ack;
    invalid.txn_id = second.txn_id.clone();
    invalid.state_hash.clear();
    assert!(restored.acknowledge_pending(&second, &invalid).is_err());
    drop(restored);
    let restored = ProjectStore::open(&path).unwrap();
    assert_eq!(restored.pending().unwrap(), vec![second]);
    drop(restored);
    temporary.close().unwrap();
}

#[test]
fn failed_sqlite_writer_lock_preserves_log_and_in_memory_host() {
    let temporary = temporary();
    let path = temporary.path().join("project.vwb");
    let mut store = create(&path);
    let before = store.revision().unwrap();
    let txn = width(before.clone(), 10, 1, 9.0);
    let database = query(&path);
    database.execute_batch("BEGIN IMMEDIATE").unwrap();
    let mut stages = Vec::new();
    assert!(
        store
            .commit_observed(&txn, &device(1), 100, |stage| stages.push(stage))
            .is_err()
    );
    assert_eq!(stages, vec![CommitStage::Prepared]);
    assert_eq!(store.revision().unwrap(), before);
    assert_eq!(
        database
            .query_row("SELECT COUNT(*) FROM op_log", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let state: Vec<u8> = database
        .query_row(
            "SELECT state FROM objects WHERE object_id=?1",
            [id(6).to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        pb::ObjectState::decode(state.as_slice())
            .unwrap()
            .style
            .unwrap()
            .width,
        2.0
    );
    database.execute_batch("ROLLBACK").unwrap();
    drop(database);
    assert_eq!(store.commit(&txn, &device(1), 101).unwrap().ack.host_seq, 1);
    drop(store);
    let restored = ProjectStore::open(&path).unwrap();
    assert_eq!(
        restored.project().objects[&id(6)]
            .state
            .style
            .as_ref()
            .unwrap()
            .width,
        9.0
    );
    drop(restored);
    temporary.close().unwrap();
}

#[test]
fn integer_overflow_rejections_leave_storage_and_retry_identity_unchanged() {
    let temporary = temporary();
    let path = temporary.path().join("project.vwb");
    let mut store = create(&path);
    let before = store.revision().unwrap();
    let mut txn = title(before.clone(), 10);
    txn.ops[0].op_id.as_mut().unwrap().lamport = u64::MAX;
    assert!(matches!(
        store.commit(&txn, &device(1), 100),
        Err(StoreError::Invalid("SQLite integer range"))
    ));
    assert!(store.enqueue_pending(&txn).is_err());
    assert!(
        store
            .register_device(&device(2), Some("Synthetic"), "android", u64::MAX)
            .is_err()
    );
    assert_eq!(store.revision().unwrap(), before);
    txn.ops[0].op_id.as_mut().unwrap().lamport = 10;
    let huge = pb::AddAsset {
        asset_id: AssetId::hash(b"huge synthetic metadata").to_string(),
        format: "mp4".into(),
        orientation: 1,
        byte_size: u64::MAX,
        source: "import".into(),
        metadata_json: "{}".into(),
        ..Default::default()
    };
    let asset = transaction(before.clone(), 11, 1, pb::op::Kind::AddAsset(huge));
    assert!(matches!(
        store.commit(&asset, &device(1), 101),
        Err(StoreError::Invalid("SQLite integer range"))
    ));
    assert_eq!(store.revision().unwrap(), before);
    assert_eq!(
        query(&path)
            .query_row("SELECT COUNT(*) FROM op_log", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(store.commit(&txn, &device(1), 102).unwrap().ack.host_seq, 1);
    drop(store);
    temporary.close().unwrap();
}

#[test]
fn writer_lock_is_exclusive_and_released_after_drop() {
    let temporary = temporary();
    let path = temporary.path().join("project.vwb");
    let store = create(&path);
    assert!(ProjectStore::open(&path).is_err());
    // WAL readers remain available while the API holds the single-writer lock.
    assert_eq!(
        query(&path)
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "2"
    );
    drop(store);
    let reopened = ProjectStore::open(&path).unwrap();
    reopened.integrity_check().unwrap();
    drop(reopened);
    temporary.close().unwrap();
}

#[test]
fn transaction_count_checkpoint_replays_only_the_three_transaction_tail() {
    let temporary = temporary();
    let path = temporary.path().join("project.vwb");
    let mut store = ProjectStore::create(
        &path,
        Project::new(id(1), "Snapshot counter".into(), device(1)),
        device(1),
        0,
    )
    .unwrap();
    // Document creation is transaction 1; all later transactions change its title.
    for index in 1..=503_u64 {
        let kind = if index == 1 {
            pb::op::Kind::CreateDocument(pb::CreateDocument {
                document_id: Some(id(2).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Counter".into(),
                ..Default::default()
            })
        } else {
            pb::op::Kind::UpdateDocument(pb::UpdateDocument {
                document_id: Some(id(2).to_proto()),
                title: format!("Counter {index}"),
            })
        };
        let txn = transaction(store.revision().unwrap(), 1000 + index, 1, kind);
        store.commit(&txn, &device(1), index as i64).unwrap();
    }
    let expected = store.revision().unwrap();
    assert_eq!(
        query(&path)
            .query_row("SELECT MAX(host_seq) FROM snapshots", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        500
    );
    drop(store);
    let restored = ProjectStore::open(&path).unwrap();
    assert_eq!(restored.replayed_transactions(), 3);
    assert_eq!(restored.revision().unwrap(), expected);
    drop(restored);
    temporary.close().unwrap();
}

#[test]
fn elapsed_time_and_explicit_idle_checkpoints_limit_replay() {
    let temporary = temporary();
    let path = temporary.path().join("project.vwb");
    let mut store = create(&path);
    for (number, time) in [(10, 299_999), (11, 300_000), (12, 300_001)] {
        let txn = title(store.revision().unwrap(), number);
        store.commit(&txn, &device(1), time).unwrap();
    }
    assert_eq!(
        query(&path)
            .query_row("SELECT MAX(host_seq) FROM snapshots", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    let expected = store.revision().unwrap();
    drop(store);
    let mut restored = ProjectStore::open(&path).unwrap();
    assert_eq!(restored.replayed_transactions(), 1);
    assert_eq!(restored.revision().unwrap(), expected);
    restored.snapshot_now(300_002).unwrap();
    drop(restored);
    let restored = ProjectStore::open(&path).unwrap();
    assert_eq!(restored.replayed_transactions(), 0);
    assert_eq!(restored.revision().unwrap(), expected);
    drop(restored);
    temporary.close().unwrap();
}

#[test]
fn cancellation_survives_a_checkpoint_without_mutating_visible_state() {
    let temporary = temporary();
    let path = temporary.path().join("project.vwb");
    let mut store = create(&path);
    let before = store.revision().unwrap();
    store.cancel_gesture(device(1), id(50), 100).unwrap();
    drop(store);
    let mut restored = ProjectStore::open(&path).unwrap();
    let mut txn = title(before.clone(), 10);
    txn.gesture_id = Some(id(50).to_proto());
    assert!(restored.commit(&txn, &device(1), 101).is_err());
    assert_eq!(restored.revision().unwrap(), before);
    drop(restored);
    temporary.close().unwrap();
}

#[test]
fn rich_document_projections_preserve_full_capture_identity_and_model_types() {
    let temporary = temporary();
    let path = temporary.path().join("project.vwb");
    let mut value = project();
    let capture = pb::CaptureInfo {
        capture_session_id: Some(id(30).to_proto()),
        frame_id: u64::MAX,
        geometry: Some(pb::CaptureGeometry {
            source_kind: "window".into(),
            window_handle: 1234,
            monitor_id: "synthetic-monitor".into(),
            client_rect_host: Some(pb::RectI {
                x: -10,
                y: 20,
                w: 600,
                h: 800,
            }),
            dpi_scale: 1.5,
            geometry_revision: u32::MAX,
            timestamp_ns: 100,
        }),
        platform: "windows".into(),
        app_name: "SyntheticApp".into(),
        window_title: "Synthetic capture".into(),
        lossless: true,
        degraded: false,
        captured_at_ms: 100,
    };
    value.documents.insert(
        id(3),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(3).to_proto()),
                kind: pb::DocumentKind::Capture as i32,
                schema_version: 1,
                title: "Capture".into(),
                capture: Some(capture.clone()),
                ..Default::default()
            },
            pages: vec![],
            created_at_ms: 100,
        },
    );
    value.documents.insert(
        id(4),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(4).to_proto()),
                kind: pb::DocumentKind::Pdf as i32,
                schema_version: 1,
                title: "PDF".into(),
                ..Default::default()
            },
            pages: vec![PdfPage {
                page_index: 0,
                crop_box: pb::RectD {
                    x: 0.0,
                    y: 0.0,
                    w: 612.0,
                    h: 792.0,
                },
                rotation_degrees: 90,
            }],
            created_at_ms: 100,
        },
    );
    value.groups.insert(
        id(7),
        Group {
            id: id(7),
            document_id: id(2),
            parent: None,
        },
    );
    value.objects.get_mut(&id(6)).unwrap().state.group_id = Some(id(7).to_proto());
    value.results.insert(
        id(8),
        ResultCandidate {
            definition: pb::AddResult {
                result_id: Some(id(8).to_proto()),
                document_id: Some(id(2).to_proto()),
                package_id: Some(id(9).to_proto()),
                provider: "synthetic".into(),
                model: "fixture".into(),
                request_json: "{}".into(),
                ..Default::default()
            },
            status: "pending".into(),
            acceptance_mask_asset_id: None,
            created_at_ms: 100,
        },
    );
    value.semantic_snapshots.insert(
        id(12),
        SemanticSnapshot {
            definition: pb::AddSemanticSnapshot {
                snapshot_id: Some(id(12).to_proto()),
                document_id: Some(id(3).to_proto()),
                platform: "uia".into(),
                frame_delta_ms: -2,
                elements_json_zstd: vec![1, 2, 3],
            },
            capture_session_id: Some(id(30)),
            frame_id: Some(u64::MAX),
            created_at_ms: 100,
        },
    );
    value.instructions.insert(
        id(13),
        Instruction {
            definition: pb::SetInstruction {
                instruction_id: Some(id(13).to_proto()),
                document_id: Some(id(2).to_proto()),
                target_object_ids: vec![id(6).to_proto()],
                role: pb::Role::Change as i32,
                text: "Synthetic instruction".into(),
                entry_method: "pc_keyboard".into(),
                language: "en".into(),
            },
            updated_at_ms: 100,
        },
    );
    value.mask_versions.insert(
        id(6),
        vec![pb::MaskOp {
            output_object_id: Some(id(6).to_proto()),
            op: "feather".into(),
            input_object_ids: vec![id(6).to_proto()],
            amount: 1.0,
        }],
    );
    let store = ProjectStore::create(&path, value.clone(), device(1), 100).unwrap();
    let database = query(&path);
    for table in [
        "captures",
        "pdf_pages",
        "groups",
        "results",
        "semantic_snapshots",
        "instructions",
        "mask_versions",
    ] {
        assert_eq!(
            database
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    for table in ["captures", "semantic_snapshots"] {
        let (kind, frame): (String, String) = database
            .query_row(
                &format!("SELECT typeof(frame_id),frame_id FROM {table}"),
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "text");
        assert_eq!(frame, u64::MAX.to_string());
    }
    drop(database);
    drop(store);
    let restored = ProjectStore::open(&path).unwrap();
    assert_eq!(restored.project(), &value);
    assert_eq!(
        restored.project().documents[&id(3)]
            .definition
            .capture
            .as_ref(),
        Some(&capture)
    );
    drop(restored);
    temporary.close().unwrap();
}

#[test]
fn corrupted_log_checkpoint_and_snapshot_bindings_are_rejected() {
    for (index, sql) in [
        "UPDATE snapshots SET checkpoint=x'00' WHERE host_seq=2",
        "UPDATE snapshots SET state_hash='0000000000000000000000000000000000000000000000000000000000000000' WHERE host_seq=2",
        "UPDATE op_log SET txn=x'ff' WHERE host_seq=1",
        "UPDATE op_log SET ack=x'ff' WHERE host_seq=1",
        "UPDATE op_log SET state_hash='0000000000000000000000000000000000000000000000000000000000000000' WHERE host_seq=1",
        "UPDATE op_log SET created_at_wall=created_at_wall+1 WHERE host_seq=1",
        "UPDATE op_log SET accepted_at=accepted_at+1 WHERE host_seq=1",
        "DELETE FROM op_log WHERE host_seq=1",
        "UPDATE snapshots SET state=x'7b7d' WHERE host_seq=2",
    ].into_iter().enumerate() {
        let temporary = temporary(); let path = temporary.path().join("project.vwb"); let mut store = create(&path);
        for number in [10,11] { let txn = title(store.revision().unwrap(), number); store.commit(&txn, &device(1), 100 + number as i64).unwrap(); }
        store.snapshot_now(200).unwrap(); drop(store);
        let database = query(&path); database.execute_batch(sql).unwrap(); drop(database);
        assert!(ProjectStore::open(&path).is_err(), "corruption case {index} accepted");
        assert!(path.join("project.sqlite").is_file()); temporary.close().unwrap();
    }
}

#[test]
fn oversized_and_malformed_pending_payloads_fail_without_decoding_unbounded_bytes() {
    let temporary = temporary();
    let path = temporary.path().join("project.vwb");
    let mut store = create(&path);
    let txn = title(store.revision().unwrap(), 10);
    store.enqueue_pending(&txn).unwrap();
    drop(store);
    let database = query(&path);
    database
        .execute("UPDATE pending_txns SET txn=x'ff'", [])
        .unwrap();
    drop(database);
    assert!(ProjectStore::open(&path).is_err());
    let database = query(&path);
    database
        .execute("UPDATE pending_txns SET txn=zeroblob(67108865)", [])
        .unwrap();
    drop(database);
    assert!(matches!(
        ProjectStore::open(&path),
        Err(StoreError::Corrupt("payload size"))
    ));
    temporary.close().unwrap();
}

#[test]
fn pending_column_mismatches_and_deleted_or_modified_projections_fail_open() {
    for sql in [
        "UPDATE pending_txns SET base_host_seq=base_host_seq+1",
        "UPDATE pending_txns SET created_at_wall=created_at_wall+1",
        "UPDATE pending_txns SET txn_id='018bcfe5-6863-7707-8707-070707070707'",
        "DELETE FROM objects",
        "UPDATE documents SET title='Corrupted synthetic title'",
        "UPDATE layers SET opacity=0.25",
    ] {
        let temporary = temporary();
        let path = temporary.path().join("project.vwb");
        let mut store = create(&path);
        let txn = title(store.revision().unwrap(), 10);
        store.enqueue_pending(&txn).unwrap();
        drop(store);
        let database = query(&path);
        database.execute_batch(sql).unwrap();
        drop(database);
        assert!(ProjectStore::open(&path).is_err());
        assert!(path.join("project.sqlite").is_file());
        temporary.close().unwrap();
    }
}

#[test]
fn source_selected_path_and_device_metadata_are_persisted_without_pairing_fields() {
    let temporary = temporary();
    let path = temporary.path().join("chosen-project.vwb");
    let mut store = create(&path);
    store
        .register_device(&device(2), Some("Synthetic phone"), "android", 25)
        .unwrap();
    store
        .register_device(&device(2), Some("Synthetic renamed phone"), "android", 5)
        .unwrap();
    assert!(
        store
            .register_device(&device(2), Some("Synthetic"), "unknown", 26)
            .is_err()
    );
    drop(store);
    let reopened = ProjectStore::open(&path).unwrap();
    let database = query(&path);
    let row: (String, i64) = database
        .query_row(
            "SELECT label,lamport FROM devices WHERE device_id=?1",
            [device(2).to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, ("Synthetic renamed phone".into(), 25));
    let columns: Vec<String> = database
        .prepare("PRAGMA table_info(devices)")
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(columns, vec!["device_id", "label", "platform", "lamport"]);
    assert!(fs::metadata(path.join("project.sqlite")).unwrap().is_file());
    drop(database);
    drop(reopened);
    temporary.close().unwrap();
}

fn project_with_asset() -> Project {
    let mut project = project();
    let asset = AssetId::hash(b"synthetic database asset bytes");
    project.assets.insert(
        asset.clone(),
        pb::AddAsset {
            asset_id: asset.to_string(),
            format: "png".into(),
            width: 2,
            height: 3,
            orientation: 1,
            bit_depth: 8,
            has_alpha: true,
            color_space: "sRGB".into(),
            icc_profile: vec![1, 2, 3],
            byte_size: 29,
            source: "import".into(),
            captured_at_ms: 123,
            metadata_json: "{}".into(),
        },
    );
    project
}

#[test]
fn asset_metadata_columns_must_match_the_immutable_protobuf_definition() {
    for sql in [
        "UPDATE assets SET width=width+1",
        "UPDATE assets SET format='jpeg'",
        "UPDATE assets SET icc_profile=x'0102'",
        "UPDATE assets SET captured_at=captured_at+1",
        "UPDATE assets SET definition=x'ff'",
    ] {
        let temporary = temporary();
        let path = temporary.path().join("project.vwb");
        let store = ProjectStore::create(&path, project_with_asset(), device(1), 0).unwrap();
        drop(store);
        let database = query(&path);
        database.execute_batch(sql).unwrap();
        drop(database);
        assert!(ProjectStore::open(&path).is_err());
        temporary.close().unwrap();
    }
}

#[test]
fn oversized_asset_definition_and_object_state_are_rejected_before_decoding() {
    // Each owned fixture is disposed before the next one: these checks never
    // require two 64 MiB malformed database payloads to be resident at once.
    for sql in [
        "UPDATE assets SET definition=zeroblob(67108865)",
        "UPDATE objects SET state=zeroblob(67108865)",
    ] {
        let temporary = temporary();
        let path = temporary.path().join("project.vwb");
        let store = ProjectStore::create(&path, project_with_asset(), device(1), 0).unwrap();
        drop(store);
        let database = query(&path);
        database.execute_batch(sql).unwrap();
        drop(database);
        assert!(matches!(
            ProjectStore::open(&path),
            Err(StoreError::Corrupt("payload size"))
        ));
        temporary.close().unwrap();
    }
}
