//! Deterministic review/recovery fixtures; no network, identity provisioning or providers.
#![allow(clippy::unwrap_used, clippy::panic)]
use tempfile::TempDir;
use vw_model::{DeviceId, Document, Id, Project};
use vw_ops::{HostSequencer, Replica};
use vw_proto::{Message, v1 as pb};
use vw_store::{
    ConflictReviewChoice as Choice, ConflictReviewIdentity as Identity,
    ConflictReviewStatus as Status, ProjectStore,
};

fn id(n: u64) -> Id {
    Id::from_parts(1_700_000_000_000 + n, [0x43; 10]).unwrap()
}
fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
fn project() -> Project {
    let mut project = Project::new(id(1), "Review fixture".into(), device(1));
    project.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(2).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "initial".into(),
                ..Default::default()
            },
            pages: vec![],
            created_at_ms: 0,
        },
    );
    project
}
fn transaction(
    base: pb::Revision,
    n: u64,
    who: u8,
    wall: i64,
    kind: pb::op::Kind,
) -> pb::Transaction {
    pb::Transaction {
        txn_id: Some(id(n).to_proto()),
        project_id: Some(id(1).to_proto()),
        device_id: device(who).to_string(),
        base_revision: Some(base),
        created_at_wall_ms: wall,
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: device(who).to_string(),
                lamport: n,
            }),
            kind: Some(kind),
        }],
        ..Default::default()
    }
}
fn title(base: pb::Revision, n: u64, who: u8, wall: i64, text: &str) -> pb::Transaction {
    transaction(
        base,
        n,
        who,
        wall,
        pb::op::Kind::UpdateDocument(pb::UpdateDocument {
            document_id: Some(id(2).to_proto()),
            title: text.into(),
        }),
    )
}
fn identity() -> Identity {
    Identity {
        transaction: id(100),
        lamport: 100,
        wall_ms: 1_000,
    }
}
fn host(temp: &TempDir) -> (ProjectStore, Id) {
    let mut store =
        ProjectStore::create(&temp.path().join("host"), project(), device(1), 0).unwrap();
    let base = store.revision().unwrap();
    store
        .commit(&title(base.clone(), 10, 1, 100, "host"), &device(1), 110)
        .unwrap();
    store
        .commit(&title(base, 11, 2, 90, "offline"), &device(2), 111)
        .unwrap();
    let conflict = Id::from_proto(store.conflicts().unwrap()[0].conflict_id.as_ref()).unwrap();
    (store, conflict)
}
fn stage(store: &mut ProjectStore, conflict: &Id) {
    store
        .stage_conflict_review(
            conflict,
            &store.revision().unwrap(),
            Choice::SwapOther,
            identity(),
        )
        .unwrap();
}
fn db(temp: &TempDir, name: &str) -> rusqlite::Connection {
    rusqlite::Connection::open(temp.path().join(name).join("project.sqlite")).unwrap()
}

#[test]
fn staged_draft_survives_interruption_without_applying_and_can_resume_once() {
    let temp = TempDir::new().unwrap();
    let (mut store, conflict) = host(&temp);
    let before = store.checkpoint_bytes().unwrap();
    stage(&mut store, &conflict);
    let draft = store.conflict_review(&conflict).unwrap();
    assert_eq!(draft.status, Status::Draft);
    let exact = draft.transaction.unwrap().encode_to_vec();
    assert_eq!(store.checkpoint_bytes().unwrap(), before);
    drop(store);
    let mut store = ProjectStore::open(&temp.path().join("host")).unwrap();
    assert_eq!(
        store
            .conflict_review(&conflict)
            .unwrap()
            .transaction
            .unwrap()
            .encode_to_vec(),
        exact
    );
    assert_eq!(
        store.submit_conflict_review(&conflict, 1_001).unwrap(),
        Status::Applied
    );
    assert_eq!(
        store.project().documents[&id(2)].definition.title,
        "offline"
    );
    let after = store.checkpoint_bytes().unwrap();
    assert_eq!(
        store.submit_conflict_review(&conflict, 2_000).unwrap(),
        Status::Applied
    );
    assert_eq!(store.checkpoint_bytes().unwrap(), after);
    assert_eq!(store.conflicts().unwrap().len(), 1);
}

#[test]
fn interruption_after_commit_before_review_metadata_uses_exact_journal_receipt() {
    let temp = TempDir::new().unwrap();
    let (mut store, conflict) = host(&temp);
    stage(&mut store, &conflict);
    let transaction = store
        .conflict_review(&conflict)
        .unwrap()
        .transaction
        .unwrap();
    // The durable edit won, then the process stopped before finish_review.
    let ack = store.commit(&transaction, &device(1), 1_001).unwrap().ack;
    drop(store);
    let mut store = ProjectStore::open(&temp.path().join("host")).unwrap();
    assert_eq!(
        store.conflict_review(&conflict).unwrap().status,
        Status::Applied
    );
    assert_eq!(
        store.submit_conflict_review(&conflict, 2_000).unwrap(),
        Status::Applied
    );
    assert_eq!(
        store.commit(&transaction, &device(1), 3_000).unwrap().ack,
        ack
    );
    assert_eq!(store.revision().unwrap().host_seq, 3);
}

#[test]
fn stale_unsubmitted_review_is_preserved_and_never_overwrites_newer_edits() {
    let temp = TempDir::new().unwrap();
    let (mut store, conflict) = host(&temp);
    stage(&mut store, &conflict);
    let saved = store.conflict_review(&conflict).unwrap().transaction;
    store
        .commit(
            &title(store.revision().unwrap(), 20, 1, 1_100, "new edit"),
            &device(1),
            1_101,
        )
        .unwrap();
    let before = store.checkpoint_bytes().unwrap();
    assert!(store.submit_conflict_review(&conflict, 1_200).is_err());
    assert_eq!(store.checkpoint_bytes().unwrap(), before);
    assert_eq!(store.conflict_review(&conflict).unwrap().transaction, saved);
    store.cancel_conflict_review(&conflict).unwrap();
    assert_eq!(
        store.conflict_review(&conflict).unwrap().status,
        Status::Unreviewed
    );
}

#[test]
fn pending_local_edits_block_staging_and_never_get_replaced_by_review() {
    let temp = TempDir::new().unwrap();
    let (mut store, conflict) = host(&temp);
    let pending = title(store.revision().unwrap(), 20, 1, 1_100, "local draft");
    store.enqueue_pending(&pending).unwrap();
    let revision = store.revision().unwrap();
    assert!(
        store
            .stage_conflict_review(&conflict, &revision, Choice::SwapOther, identity())
            .is_err()
    );
    assert!(
        store
            .keep_conflict_current(&conflict, &revision, 1_200)
            .is_err()
    );
    assert_eq!(store.pending().unwrap(), vec![pending]);
    assert_eq!(
        store.conflict_review(&conflict).unwrap().status,
        Status::Unreviewed
    );
}

#[test]
fn mirror_reconnect_duplicate_delivery_and_stale_replay_preserve_drafts_and_history() {
    let temp = TempDir::new().unwrap();
    let (mut host, conflict) = host(&temp);
    let mut mirror = ProjectStore::create_authenticated_checkpoint(
        &temp.path().join("mirror"),
        &host.checkpoint_bytes().unwrap(),
        &device(1),
        &device(2),
        200,
    )
    .unwrap();
    stage(&mut mirror, &conflict);
    let review = mirror
        .conflict_review(&conflict)
        .unwrap()
        .transaction
        .unwrap();
    assert_eq!(
        mirror.submit_conflict_review(&conflict, 1_001).unwrap(),
        Status::Queued
    );
    drop(mirror); // Interruption after enqueue, before send.
    let mut mirror = ProjectStore::open(&temp.path().join("mirror")).unwrap();
    assert_eq!(
        mirror.submit_conflict_review(&conflict, 1_002).unwrap(),
        Status::Queued
    );
    assert!(mirror.cancel_conflict_review(&conflict).is_err());
    assert_eq!(mirror.pending().unwrap(), vec![review.clone()]);
    let mut replica = Replica::new(
        device(2),
        HostSequencer::from_checkpoint_bytes(&mirror.checkpoint_bytes().unwrap())
            .unwrap()
            .snapshot()
            .unwrap(),
    )
    .unwrap();
    replica.queue_recovered(review.clone()).unwrap();
    assert_eq!(
        replica.project().documents[&id(2)].definition.title,
        "offline"
    );
    let stale = mirror.checkpoint_bytes().unwrap();
    let ack = host.commit(&review, &device(2), 1_003).unwrap().ack;
    assert_eq!(host.commit(&review, &device(2), 1_004).unwrap().ack, ack);
    let current = host.checkpoint_bytes().unwrap();
    assert!(
        mirror
            .install_authenticated_checkpoint(&current, &device(3), 1_005)
            .is_err()
    );
    assert_eq!(mirror.pending().unwrap(), vec![review.clone()]);
    assert_eq!(
        mirror
            .install_authenticated_checkpoint(&current, &device(1), 1_006)
            .unwrap(),
        1
    );
    assert_eq!(
        mirror
            .install_authenticated_checkpoint(&current, &device(1), 1_007)
            .unwrap(),
        0
    );
    assert_eq!(
        mirror.conflict_review(&conflict).unwrap().status,
        Status::Applied
    );
    assert_eq!(
        mirror.submit_conflict_review(&conflict, 1_008).unwrap(),
        Status::Applied
    );
    assert!(
        mirror
            .install_authenticated_checkpoint(&stale, &device(1), 1_009)
            .is_err()
    );
    assert!(mirror.pending().unwrap().is_empty());
    assert_eq!(
        mirror
            .conflict_review(&conflict)
            .unwrap()
            .transaction
            .unwrap(),
        review
    );
}

#[test]
fn keep_is_local_durable_metadata_and_does_not_remove_losing_values() {
    let temp = TempDir::new().unwrap();
    let (mut store, conflict) = host(&temp);
    let bytes = store.checkpoint_bytes().unwrap();
    let records = store.conflicts().unwrap();
    store
        .keep_conflict_current(&conflict, &store.revision().unwrap(), 1_000)
        .unwrap();
    assert_eq!(store.checkpoint_bytes().unwrap(), bytes);
    drop(store);
    let store = ProjectStore::open(&temp.path().join("host")).unwrap();
    assert_eq!(
        store.conflict_review(&conflict).unwrap().status,
        Status::Kept
    );
    assert_eq!(store.conflicts().unwrap(), records);
    let ids = store.conflict_review_ids(0, 1).unwrap();
    assert_eq!(ids[0].1, conflict);
    assert!(store.conflict_review_ids(ids[0].0, 1).unwrap().is_empty());
    assert!(store.conflict_review_ids(0, 129).is_err());
}

#[test]
fn draft_replacement_collision_and_tampering_fail_without_mutation() {
    let temp = TempDir::new().unwrap();
    let (mut store, conflict) = host(&temp);
    stage(&mut store, &conflict);
    assert!(
        store
            .stage_conflict_review(
                &conflict,
                &store.revision().unwrap(),
                Choice::SwapOther,
                identity()
            )
            .is_err()
    );
    let mut transaction = store
        .conflict_review(&conflict)
        .unwrap()
        .transaction
        .unwrap();
    transaction.created_at_wall_ms += 1;
    store.commit(&transaction, &device(1), 1_001).unwrap();
    assert!(store.submit_conflict_review(&conflict, 1_002).is_err());
    assert!(store.conflict_review(&conflict).is_err());
}

#[test]
fn changed_private_draft_cannot_submit_a_different_edit() {
    let temp = TempDir::new().unwrap();
    let (mut store, conflict) = host(&temp);
    stage(&mut store, &conflict);
    let connection = db(&temp, "host");
    let key = format!("offline-review/v1/{conflict}");
    let encoded: String = connection
        .query_row("SELECT value FROM settings WHERE key=?1", [&key], |r| {
            r.get(0)
        })
        .unwrap();
    let mut draft: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    draft["transaction"]["ops"][0]["kind"]["UpdateDocument"]["title"] = "unreviewed".into();
    connection
        .execute(
            "UPDATE settings SET value=?1 WHERE key=?2",
            rusqlite::params![draft.to_string(), key],
        )
        .unwrap();
    assert!(store.submit_conflict_review(&conflict, 1_001).is_err());
    assert_eq!(store.project().documents[&id(2)].definition.title, "host");
}

fn object_conflict(temp: &TempDir, deleting: bool) -> (ProjectStore, Id) {
    let (mut store, _) = host(temp);
    let layer = pb::op::Kind::CreateLayer(pb::CreateLayer {
        layer_id: Some(id(3).to_proto()),
        document_id: Some(id(2).to_proto()),
        page_index: -1,
        name: "fixture".into(),
        kind: "annotation".into(),
        order_key: "V".into(),
    });
    store
        .commit(
            &transaction(store.revision().unwrap(), 30, 1, 200, layer),
            &device(1),
            201,
        )
        .unwrap();
    let object = pb::ObjectState {
        object_id: Some(id(4).to_proto()),
        layer_id: Some(id(3).to_proto()),
        order_key: "V".into(),
        transform: Some(pb::Affine {
            a: 1.0,
            d: 1.0,
            ..Default::default()
        }),
        style: Some(pb::Style {
            width: 1.0,
            stroke: Some(pb::Color { rgba: 0x112233ff }),
            ..Default::default()
        }),
        created_by: device(1).to_string(),
        created_at_ms: 210,
        shape: Some(pb::object_state::Shape::Rect(pb::RectD {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        })),
        ..Default::default()
    };
    store
        .commit(
            &transaction(
                store.revision().unwrap(),
                31,
                1,
                210,
                pb::op::Kind::CreateObject(pb::CreateObject {
                    document_id: Some(id(2).to_proto()),
                    state: Some(object),
                }),
            ),
            &device(1),
            211,
        )
        .unwrap();
    let base = store.revision().unwrap();
    let width = |v| {
        pb::op::Kind::SetProperty(pb::SetProperty {
            object_id: Some(id(4).to_proto()),
            property: "style.width".into(),
            value: Some(pb::PropertyValue {
                value: Some(pb::property_value::Value::Number(v)),
            }),
        })
    };
    store
        .commit(
            &transaction(base.clone(), 32, 1, 300, width(5.0)),
            &device(1),
            301,
        )
        .unwrap();
    let other = if deleting {
        pb::op::Kind::DeleteObject(pb::DeleteObject {
            object_id: Some(id(4).to_proto()),
        })
    } else {
        width(2.0)
    };
    store
        .commit(&transaction(base, 33, 2, 290, other), &device(2), 302)
        .unwrap();
    let conflict = Id::from_proto(
        store
            .conflicts()
            .unwrap()
            .last()
            .unwrap()
            .conflict_id
            .as_ref(),
    )
    .unwrap();
    (store, conflict)
}

#[test]
fn object_property_swap_uses_normal_typed_operations_and_remains_undoable() {
    let temp = TempDir::new().unwrap();
    let (mut store, conflict) = object_conflict(&temp, false);
    let before = store.project().state_hash().unwrap();
    stage(&mut store, &conflict);
    let review = store
        .conflict_review(&conflict)
        .unwrap()
        .transaction
        .unwrap();
    assert_eq!(
        store.submit_conflict_review(&conflict, 1_001).unwrap(),
        Status::Applied
    );
    assert_eq!(
        store.project().objects[&id(4)].state.style.unwrap().width,
        2.0
    );
    let undo = transaction(
        store.revision().unwrap(),
        101,
        1,
        1_100,
        pb::op::Kind::UndoTransaction(pb::UndoTransaction {
            target_txn_id: review.txn_id,
        }),
    );
    store.commit(&undo, &device(1), 1_101).unwrap();
    assert_eq!(store.project().state_hash().unwrap(), before);
}

#[test]
fn deletion_restores_an_explicit_new_copy_and_never_resurrects_tombstone_identity() {
    let temp = TempDir::new().unwrap();
    let (mut store, conflict) = object_conflict(&temp, true);
    let revision = store.revision().unwrap();
    assert!(
        store
            .stage_conflict_review(&conflict, &revision, Choice::SwapOther, identity())
            .is_err()
    );
    assert!(
        store
            .stage_conflict_review(
                &conflict,
                &revision,
                Choice::RestoreCopy {
                    document: id(2),
                    object: id(4)
                },
                identity()
            )
            .is_err()
    );
    let records = store.conflicts().unwrap();
    store
        .stage_conflict_review(
            &conflict,
            &revision,
            Choice::RestoreCopy {
                document: id(2),
                object: id(5),
            },
            identity(),
        )
        .unwrap();
    store.submit_conflict_review(&conflict, 1_001).unwrap();
    assert!(!store.project().objects.contains_key(&id(4)));
    assert_eq!(
        store.project().objects[&id(5)].state.style.unwrap().width,
        5.0
    );
    assert_eq!(
        store.project().objects[&id(5)].state.created_by,
        device(1).as_str()
    );
    assert_eq!(store.conflicts().unwrap(), records);
    assert!(store.cancel_conflict_review(&conflict).is_err());
}

#[test]
fn unsupported_entity_conflict_stays_reviewable_and_can_be_kept() {
    let temp = TempDir::new().unwrap();
    let (mut store, _) = host(&temp);
    store
        .commit(
            &transaction(
                store.revision().unwrap(),
                30,
                1,
                200,
                pb::op::Kind::CreateLayer(pb::CreateLayer {
                    layer_id: Some(id(3).to_proto()),
                    document_id: Some(id(2).to_proto()),
                    page_index: -1,
                    name: "before".into(),
                    kind: "annotation".into(),
                    order_key: "V".into(),
                }),
            ),
            &device(1),
            201,
        )
        .unwrap();
    let base = store.revision().unwrap();
    store
        .commit(
            &transaction(
                base.clone(),
                31,
                1,
                210,
                pb::op::Kind::UpdateLayer(pb::UpdateLayer {
                    layer_id: Some(id(3).to_proto()),
                    name: Some("edited".into()),
                    ..Default::default()
                }),
            ),
            &device(1),
            211,
        )
        .unwrap();
    store
        .commit(
            &transaction(
                base,
                32,
                2,
                205,
                pb::op::Kind::DeleteLayer(pb::DeleteLayer {
                    layer_id: Some(id(3).to_proto()),
                }),
            ),
            &device(2),
            212,
        )
        .unwrap();
    let conflict = Id::from_proto(
        store
            .conflicts()
            .unwrap()
            .last()
            .unwrap()
            .conflict_id
            .as_ref(),
    )
    .unwrap();
    let before = store.checkpoint_bytes().unwrap();
    assert!(
        store
            .stage_conflict_review(
                &conflict,
                &store.revision().unwrap(),
                Choice::SwapOther,
                identity()
            )
            .is_err()
    );
    assert_eq!(store.checkpoint_bytes().unwrap(), before);
    assert_eq!(
        store.conflict_review(&conflict).unwrap().status,
        Status::Unreviewed
    );
    store
        .keep_conflict_current(&conflict, &store.revision().unwrap(), 1_001)
        .unwrap();
    assert_eq!(
        store.conflict_review(&conflict).unwrap().status,
        Status::Kept
    );
}

#[test]
fn review_fence_requires_the_full_revision_hash_and_preserves_locked_objects() {
    let temp = TempDir::new().unwrap();
    let (mut store, conflict) = object_conflict(&temp, false);
    let mut forged = store.revision().unwrap();
    forged.state_hash[0] ^= 1;
    assert!(
        store
            .stage_conflict_review(&conflict, &forged, Choice::SwapOther, identity())
            .is_err()
    );
    let lock = pb::op::Kind::SetProperty(pb::SetProperty {
        object_id: Some(id(4).to_proto()),
        property: "locked".into(),
        value: Some(pb::PropertyValue {
            value: Some(pb::property_value::Value::Flag(true)),
        }),
    });
    store
        .commit(
            &transaction(store.revision().unwrap(), 40, 1, 400, lock),
            &device(1),
            401,
        )
        .unwrap();
    let before = store.checkpoint_bytes().unwrap();
    assert!(
        store
            .stage_conflict_review(
                &conflict,
                &store.revision().unwrap(),
                Choice::SwapOther,
                identity()
            )
            .is_err()
    );
    assert_eq!(store.checkpoint_bytes().unwrap(), before);
    assert_eq!(
        store.conflict_review(&conflict).unwrap().status,
        Status::Unreviewed
    );
}

#[test]
fn saved_review_reserves_its_counter_across_reopen_before_entering_history() {
    let temp = TempDir::new().unwrap();
    let (mut store, conflict) = host(&temp);
    assert_eq!(store.conflict_review_lamport_floor().unwrap(), 1);
    stage(&mut store, &conflict);
    assert_eq!(store.conflict_review_lamport_floor().unwrap(), 101);
    assert!(store.pending().unwrap().is_empty());
    drop(store);
    let store = ProjectStore::open(&temp.path().join("host")).unwrap();
    assert_eq!(store.conflict_review_lamport_floor().unwrap(), 101);
    assert_eq!(store.revision().unwrap().host_seq, 2);
}

#[test]
fn reconnecting_a_width_review_does_not_rewrite_an_independent_remote_stroke_color() {
    let temp = TempDir::new().unwrap();
    let (mut host, conflict) = object_conflict(&temp, false);
    let mut mirror = ProjectStore::create_authenticated_checkpoint(
        &temp.path().join("mirror"),
        &host.checkpoint_bytes().unwrap(),
        &device(1),
        &device(2),
        500,
    )
    .unwrap();
    stage(&mut mirror, &conflict);
    mirror.submit_conflict_review(&conflict, 1_001).unwrap();
    let review = mirror.pending().unwrap().remove(0);
    let Some(pb::op::Kind::SetProperty(property)) = &review.ops[0].kind else {
        unreachable!()
    };
    assert_eq!(property.property, "style.width");
    let color = pb::op::Kind::SetProperty(pb::SetProperty {
        object_id: Some(id(4).to_proto()),
        property: "style.stroke".into(),
        value: Some(pb::PropertyValue {
            value: Some(pb::property_value::Value::Color(pb::Color {
                rgba: 0xabcdefef,
            })),
        }),
    });
    host.commit(
        &transaction(host.revision().unwrap(), 40, 1, 1_100, color),
        &device(1),
        1_101,
    )
    .unwrap();
    let conflicts = host.conflicts().unwrap();
    assert!(
        host.commit(&review, &device(2), 1_102)
            .unwrap()
            .conflicts
            .is_empty()
    );
    let style = host.project().objects[&id(4)].state.style.unwrap();
    assert_eq!(style.width, 2.0);
    assert_eq!(style.stroke.unwrap().rgba, 0xabcdefef);
    assert_eq!(host.conflicts().unwrap(), conflicts);
}

#[test]
fn color_only_fill_review_cannot_queue_a_setter_that_might_reenable_disabled_fill() {
    let temp = TempDir::new().unwrap();
    let (mut store, _) = object_conflict(&temp, false);
    let fill = |rgba| {
        pb::op::Kind::SetProperty(pb::SetProperty {
            object_id: Some(id(4).to_proto()),
            property: "style.fill".into(),
            value: Some(pb::PropertyValue {
                value: Some(pb::property_value::Value::Color(pb::Color { rgba })),
            }),
        })
    };
    store
        .commit(
            &transaction(store.revision().unwrap(), 40, 1, 400, fill(0x111111ff)),
            &device(1),
            401,
        )
        .unwrap();
    let base = store.revision().unwrap();
    store
        .commit(
            &transaction(base.clone(), 41, 1, 410, fill(0x222222ff)),
            &device(1),
            411,
        )
        .unwrap();
    store
        .commit(
            &transaction(base, 42, 2, 405, fill(0x333333ff)),
            &device(2),
            412,
        )
        .unwrap();
    let conflict = Id::from_proto(
        store
            .conflicts()
            .unwrap()
            .last()
            .unwrap()
            .conflict_id
            .as_ref(),
    )
    .unwrap();
    assert_eq!(
        store.conflict_review(&conflict).unwrap().record.property,
        "objects.state.style.fill.rgba"
    );
    // Refuse even while enabled; that prevents the unsafe queued operation.
    assert!(
        store
            .stage_conflict_review(
                &conflict,
                &store.revision().unwrap(),
                Choice::SwapOther,
                identity()
            )
            .is_err()
    );
    let disable = pb::op::Kind::SetProperty(pb::SetProperty {
        object_id: Some(id(4).to_proto()),
        property: "style.has_fill".into(),
        value: Some(pb::PropertyValue {
            value: Some(pb::property_value::Value::Flag(false)),
        }),
    });
    store
        .commit(
            &transaction(store.revision().unwrap(), 43, 1, 420, disable),
            &device(1),
            421,
        )
        .unwrap();
    let before = store.checkpoint_bytes().unwrap();
    let records = store.conflicts().unwrap();
    assert!(
        store
            .stage_conflict_review(
                &conflict,
                &store.revision().unwrap(),
                Choice::SwapOther,
                identity()
            )
            .is_err()
    );
    assert_eq!(store.checkpoint_bytes().unwrap(), before);
    assert!(
        !store.project().objects[&id(4)]
            .state
            .style
            .unwrap()
            .has_fill
    );
    assert_eq!(store.conflicts().unwrap(), records);
    assert_eq!(
        store.conflict_review(&conflict).unwrap().status,
        Status::Unreviewed
    );
    assert!(store.pending().unwrap().is_empty());
}
