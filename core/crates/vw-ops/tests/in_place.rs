//! The host edits its project in place. These tests pin down what that must
//! not change: a transaction that is refused, or whose persistence fails,
//! leaves the host byte-identical; `check` accepts nothing; and the hash the
//! host carries forward is the hash of the whole project.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use vw_model::{AssetId, DeviceId, Id, Project};
use vw_ops::{HostSequencer, OpsError, Touched};
use vw_proto::{
    Message,
    v1::{self as pb, object_state::Shape, op::Kind, property_value::Value},
};

const CLOCK: u64 = 1_700_000_000_000;
fn id(n: u64) -> Id {
    Id::from_parts(CLOCK + n, [0x31; 10]).unwrap()
}
fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
fn uid(n: u64) -> Option<pb::Uuid> {
    Some(id(n).to_proto())
}
fn asset() -> pb::AddAsset {
    pb::AddAsset {
        asset_id: AssetId::hash(b"immutable fixture original").to_string(),
        format: "png".into(),
        width: 128,
        height: 64,
        orientation: 1,
        bit_depth: 8,
        color_space: "sRGB".into(),
        byte_size: 26,
        source: "import".into(),
        metadata_json: "{}".into(),
        ..Default::default()
    }
}
fn layer(n: u64) -> Kind {
    Kind::CreateLayer(pb::CreateLayer {
        layer_id: uid(n),
        document_id: uid(2),
        page_index: -1,
        name: "Fixture layer".into(),
        kind: "annotation".into(),
        order_key: "V".into(),
    })
}
fn create_in(n: u64, layer: u64, owner: &DeviceId) -> Kind {
    Kind::CreateObject(pb::CreateObject {
        document_id: uid(2),
        state: Some(pb::ObjectState {
            object_id: uid(n),
            layer_id: uid(layer),
            order_key: "V".into(),
            transform: Some(pb::Affine {
                a: 1.0,
                d: 1.0,
                e: n as f64,
                ..Default::default()
            }),
            style: Some(pb::Style {
                stroke: Some(pb::Color { rgba: 0x112233ff }),
                width: 1.0,
                ..Default::default()
            }),
            role: pb::Role::None as i32,
            created_by: owner.to_string(),
            created_at_ms: CLOCK as i64,
            shape: Some(Shape::Rect(pb::RectD {
                x: 0.25,
                y: 1.5,
                w: 12.0,
                h: 9.0,
            })),
            ..Default::default()
        }),
    })
}
fn property(n: u64, name: &str, value: Value) -> Kind {
    Kind::SetProperty(pb::SetProperty {
        object_id: uid(n),
        property: name.into(),
        value: Some(pb::PropertyValue { value: Some(value) }),
    })
}
fn undo(target: u64) -> Kind {
    Kind::UndoTransaction(pb::UndoTransaction {
        target_txn_id: uid(10_000 + target),
    })
}
fn transaction(
    base: pb::Revision,
    owner: &DeviceId,
    n: u64,
    wall: i64,
    kinds: Vec<Kind>,
) -> pb::Transaction {
    pb::Transaction {
        txn_id: uid(10_000 + n),
        project_id: uid(1),
        device_id: owner.to_string(),
        base_revision: Some(base),
        created_at_wall_ms: wall,
        ops: kinds
            .into_iter()
            .enumerate()
            .map(|(i, kind)| pb::Op {
                op_id: Some(pb::OpId {
                    device_id: owner.to_string(),
                    lamport: n * 100 + i as u64 + 1,
                }),
                kind: Some(kind),
            })
            .collect(),
        gesture_id: n.is_multiple_of(2).then(|| id(20_000 + n).to_proto()),
    }
}
fn fixture() -> HostSequencer {
    let owner = device(1);
    let mut host = HostSequencer::new(
        Project::new(id(1), "In-place fixture".into(), owner.clone()),
        owner.clone(),
    )
    .unwrap();
    let setup = transaction(
        host.revision().unwrap(),
        &owner,
        1,
        CLOCK as i64,
        vec![
            Kind::AddAsset(asset()),
            Kind::CreateDocument(pb::CreateDocument {
                document_id: uid(2),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Fixture document".into(),
                primary_asset_id: asset().asset_id,
                capture: None,
            }),
            layer(3),
            create_in(4, 3, &owner),
            create_in(5, 3, &owner),
        ],
    );
    host.submit(setup, &owner, CLOCK as i64).unwrap();
    host
}

/// A history that reaches objects, layers, groups, instructions, deletions
/// with tombstones, a stale write that loses, and undo and redo.
fn script() -> Vec<(DeviceId, i64, Vec<Kind>)> {
    let (owner, peer) = (device(1), device(2));
    vec![
        (owner.clone(), 100, vec![create_in(6, 3, &owner)]),
        (
            owner.clone(),
            110,
            vec![property(4, "style.width", Value::Number(5.0))],
        ),
        (owner.clone(), 120, vec![layer(7), create_in(8, 7, &owner)]),
        (
            peer.clone(),
            130,
            vec![property(4, "hidden", Value::Flag(true))],
        ),
        (
            owner.clone(),
            140,
            vec![Kind::Group(pb::Group {
                group_id: uid(9),
                object_ids: vec![id(4).to_proto(), id(5).to_proto()],
            })],
        ),
        (
            owner.clone(),
            150,
            vec![Kind::SetInstruction(pb::SetInstruction {
                instruction_id: uid(10),
                document_id: uid(2),
                target_object_ids: vec![id(4).to_proto()],
                role: pb::Role::Change as i32,
                text: "Synthetic instruction".into(),
                entry_method: "pc_keyboard".into(),
                language: "en".into(),
            })],
        ),
        (
            owner.clone(),
            160,
            vec![Kind::DeleteObject(pb::DeleteObject { object_id: uid(6) })],
        ),
        // Undo the deletion (transaction 8), then redo it by undoing that.
        (owner.clone(), 170, vec![undo(8)]),
        (owner.clone(), 180, vec![undo(9)]),
        (
            owner.clone(),
            190,
            vec![Kind::UpdateLayer(pb::UpdateLayer {
                layer_id: uid(7),
                opacity: Some(0.5),
                ..Default::default()
            })],
        ),
        (
            owner.clone(),
            200,
            vec![Kind::DeleteLayer(pb::DeleteLayer { layer_id: uid(7) })],
        ),
        (owner.clone(), 210, vec![undo(12)]),
        (
            owner,
            220,
            vec![Kind::Ungroup(pb::Ungroup { group_id: uid(9) })],
        ),
    ]
}

#[test]
fn a_failed_persist_or_a_check_leaves_the_host_byte_identical() {
    let mut host = fixture();
    for (step, (author, wall, kinds)) in script().into_iter().enumerate() {
        let n = step as u64 + 2;
        let txn = transaction(host.revision().unwrap(), &author, n, wall, kinds);
        let before = host.checkpoint_bytes().unwrap();
        let length = host.canonical_len();
        // A twin that never sees the failed attempts.
        let mut twin = HostSequencer::from_checkpoint_bytes(&before).unwrap();

        let (checked, checked_length) = host.check(txn.clone(), &author, CLOCK as i64).unwrap();
        assert_eq!(host.checkpoint_bytes().unwrap(), before, "check at {step}");

        let mut seen = None;
        let failed = host.submit_with(txn.clone(), &author, CLOCK as i64, |next, ack, touched| {
            // The callback sees the transaction applied.
            seen = Some((next.revision().unwrap(), ack.ack.clone(), touched.clone()));
            Err::<(), OpsError>(OpsError::NotFound)
        });
        assert!(matches!(failed, Err(OpsError::NotFound)), "step {step}");
        let (revision, ack, touched) = seen.unwrap();
        assert_eq!(revision.host_seq, ack.host_seq);
        assert_eq!(revision.state_hash, ack.state_hash);
        assert_ne!(touched, Touched::default(), "step {step} touched nothing");
        assert_eq!(host.checkpoint_bytes().unwrap(), before, "revert at {step}");
        assert_eq!(host.canonical_len(), length);
        assert_eq!(
            host.project().state_hash().unwrap().bytes().to_vec(),
            host.revision().unwrap().state_hash
        );

        let accepted = host.submit(txn.clone(), &author, CLOCK as i64).unwrap();
        let reference = twin.submit(txn, &author, CLOCK as i64).unwrap();
        assert_eq!(accepted.ack.encode_to_vec(), reference.ack.encode_to_vec());
        assert_eq!(accepted.ack.encode_to_vec(), checked.ack.encode_to_vec());
        assert_eq!(accepted.ack.encode_to_vec(), ack.encode_to_vec());
        assert_eq!(accepted.conflicts, checked.conflicts);
        assert_eq!(host.canonical_len(), checked_length);
        assert_eq!(
            host.canonical_len(),
            host.project().canonical_bytes().unwrap().len()
        );
        assert_eq!(
            host.checkpoint_bytes().unwrap(),
            twin.checkpoint_bytes().unwrap(),
            "step {step} differs from a host that never failed"
        );
        assert_eq!(
            host.project().state_hash().unwrap().bytes().to_vec(),
            accepted.ack.state_hash,
            "carried hash at {step}"
        );
    }
    // The script ended where it meant to: the layer is back, the group gone.
    assert!(host.project().layers.contains_key(&id(7)));
    assert!(host.project().groups.is_empty());
}

#[test]
fn a_rejected_transaction_leaves_the_host_byte_identical() {
    let mut host = fixture();
    let owner = device(1);
    let before = host.checkpoint_bytes().unwrap();
    let rejected: Vec<Vec<Kind>> = vec![
        // The second operation fails after the first already edited the project.
        vec![
            create_in(6, 3, &owner),
            property(99, "hidden", Value::Flag(true)),
        ],
        // The second operation fails after the first removed a layer and
        // every object in it.
        vec![
            Kind::DeleteLayer(pb::DeleteLayer { layer_id: uid(3) }),
            create_in(6, 3, &owner),
        ],
        // A layer change fails after an object change.
        vec![
            property(4, "style.width", Value::Number(9.0)),
            Kind::UpdateLayer(pb::UpdateLayer {
                layer_id: uid(42),
                opacity: Some(0.5),
                ..Default::default()
            }),
        ],
        vec![undo(77)],
    ];
    for (n, kinds) in rejected.into_iter().enumerate() {
        let txn = transaction(host.revision().unwrap(), &owner, 50 + n as u64, 100, kinds);
        assert!(host.check(txn.clone(), &owner, CLOCK as i64).is_err());
        assert!(host.submit(txn, &owner, CLOCK as i64).is_err(), "case {n}");
        assert_eq!(host.checkpoint_bytes().unwrap(), before, "case {n}");
    }
    // The same identifiers are still free to use.
    let txn = transaction(
        host.revision().unwrap(),
        &owner,
        50,
        100,
        vec![create_in(6, 3, &owner)],
    );
    host.submit(txn, &owner, CLOCK as i64).unwrap();
}

#[test]
fn touched_names_exactly_the_entities_a_transaction_changed() {
    let mut host = fixture();
    let owner = device(1);
    let touched_by = |host: &mut HostSequencer, n: u64, kinds: Vec<Kind>| {
        let txn = transaction(host.revision().unwrap(), &owner, n, 100, kinds);
        host.submit_with(txn, &owner, CLOCK as i64, |_, _, touched| {
            Ok::<_, OpsError>(touched.clone())
        })
        .unwrap()
        .1
        .unwrap()
    };
    let created = touched_by(&mut host, 2, vec![create_in(6, 3, &owner)]);
    assert!(created.only_objects());
    assert_eq!(created.objects.into_iter().collect::<Vec<_>>(), [id(6)]);

    let layered = touched_by(&mut host, 3, vec![layer(7), create_in(8, 7, &owner)]);
    assert!(!layered.only_objects());
    assert_eq!(layered.layers.into_iter().collect::<Vec<_>>(), [id(7)]);
    assert_eq!(layered.objects.into_iter().collect::<Vec<_>>(), [id(8)]);

    let removed = touched_by(
        &mut host,
        4,
        vec![Kind::DeleteLayer(pb::DeleteLayer { layer_id: uid(7) })],
    );
    assert_eq!(removed.layers.into_iter().collect::<Vec<_>>(), [id(7)]);
    assert_eq!(removed.objects.into_iter().collect::<Vec<_>>(), [id(8)]);
    assert!(!host.project().objects.contains_key(&id(8)));

    // A retry is answered from the record and persists nothing.
    let txn = transaction(host.revision().unwrap(), &owner, 5, 100, vec![layer(9)]);
    host.submit(txn.clone(), &owner, CLOCK as i64).unwrap();
    let (retry, persisted) = host
        .submit_with(txn, &owner, CLOCK as i64, |_, _, _| {
            Ok::<_, OpsError>("persist must not run for a retry")
        })
        .unwrap();
    assert!(retry.duplicate && persisted.is_none());
}

#[test]
fn appended_objects_carry_the_whole_project_hash() {
    let mut host = fixture();
    let owner = device(1);
    for n in 0..300_u64 {
        let txn = transaction(
            host.revision().unwrap(),
            &owner,
            100 + n,
            100,
            vec![create_in(1000 + n, 3, &owner)],
        );
        let accepted = host.submit(txn, &owner, CLOCK as i64).unwrap();
        if n % 37 == 0 || n == 299 {
            assert_eq!(
                accepted.ack.state_hash,
                host.project().state_hash().unwrap().bytes().to_vec(),
                "object {n}"
            );
        }
    }
    // Change an early object, then a late one, then remove one in the middle.
    for (n, kinds) in [
        (2000, vec![property(1003, "hidden", Value::Flag(true))]),
        (2001, vec![property(1290, "hidden", Value::Flag(true))]),
        (
            2002,
            vec![Kind::DeleteObject(pb::DeleteObject {
                object_id: uid(1150),
            })],
        ),
    ] {
        let txn = transaction(host.revision().unwrap(), &owner, n, 100, kinds);
        let accepted = host.submit(txn, &owner, CLOCK as i64).unwrap();
        assert_eq!(
            accepted.ack.state_hash,
            host.project().state_hash().unwrap().bytes().to_vec()
        );
    }
    let restored = HostSequencer::from_checkpoint_bytes(&host.checkpoint_bytes().unwrap()).unwrap();
    assert_eq!(restored.revision().unwrap(), host.revision().unwrap());
    assert_eq!(restored.canonical_len(), host.canonical_len());
}

#[test]
fn a_replica_always_knows_the_hash_of_what_it_shows() {
    let mut host = fixture();
    let peer = device(2);
    let mut replica = vw_ops::Replica::new(peer.clone(), host.snapshot().unwrap()).unwrap();
    let shown = |replica: &vw_ops::Replica| {
        assert_eq!(
            replica.state_hash(),
            &replica.project().state_hash().unwrap(),
            "the replica's carried hash is not the hash of its visible project"
        );
    };
    shown(&replica);
    // Queue edits against the same base: objects, a property, an undo.
    let base = host.revision().unwrap();
    let mut queued = Vec::new();
    for (n, kinds) in [
        (30, vec![create_in(60, 3, &peer)]),
        (31, vec![create_in(61, 3, &peer), layer(8)]),
        (32, vec![property(60, "hidden", Value::Flag(true))]),
        (33, vec![undo(32)]),
        (34, vec![create_in(62, 8, &peer)]),
    ] {
        let txn = transaction(base.clone(), &peer, n, 100 + n as i64, kinds);
        replica.queue(txn.clone()).unwrap();
        shown(&replica);
        queued.push(txn);
    }
    assert!(!replica.project().objects[&id(60)].state.hidden);
    // A refused edit changes neither the project nor its hash.
    let before = (replica.project().clone(), replica.state_hash().clone());
    let refused = transaction(
        base.clone(),
        &peer,
        35,
        200,
        vec![
            create_in(63, 3, &peer),
            property(999, "hidden", Value::Flag(true)),
        ],
    );
    assert!(replica.queue(refused).is_err());
    assert_eq!(
        (replica.project(), replica.state_hash()),
        (&before.0, &before.1)
    );
    // The host accepts the first two; the rest are replayed on its snapshot.
    for txn in queued.iter().take(2) {
        host.submit(txn.clone(), &peer, CLOCK as i64).unwrap();
        replica.receive(host.snapshot().unwrap()).unwrap();
        shown(&replica);
    }
    assert_eq!(replica.pending().len(), 3);
    assert!(replica.pending().iter().all(|p| p.blocked_reason.is_none()));
    // Abandon the property edit: its undo goes with it.
    let removed = replica.discard_pending(&id(10_032)).unwrap();
    assert_eq!(removed.len(), 2);
    shown(&replica);
    // An edit authored before the base advanced is recovered, not refused.
    let late = transaction(base, &peer, 36, 300, vec![create_in(64, 3, &peer)]);
    replica.queue_recovered(late).unwrap();
    shown(&replica);
    assert!(replica.project().objects.contains_key(&id(64)));
    // Once everything is accepted the replica shows the host's own state.
    let rest = replica
        .pending()
        .iter()
        .map(|p| p.transaction.clone())
        .collect::<Vec<_>>();
    for txn in rest {
        host.submit(txn, &peer, CLOCK as i64).unwrap();
    }
    replica.receive(host.snapshot().unwrap()).unwrap();
    assert!(replica.pending().is_empty());
    assert_eq!(
        replica.state_hash().bytes().to_vec(),
        host.revision().unwrap().state_hash
    );
}

/// A replica moved on step by step from a wrong list of what the host's
/// transactions touched is not the host's project: with nothing waiting
/// that is seen in its hash, and the step is refused with the replica left
/// at its old revision, where it refuses new work.
#[test]
fn a_replica_advanced_from_a_wrong_touched_set_is_refused() {
    let owner = device(1);
    let phone = device(2);
    let mut host = fixture();
    let snapshot = host.snapshot().unwrap();
    let mut replica = vw_ops::Replica::new(phone.clone(), snapshot).unwrap();
    let before = replica.revision().clone();
    let txn = transaction(
        host.revision().unwrap(),
        &owner,
        7,
        CLOCK as i64 + 7,
        vec![create_in(70, 3, &owner)],
    );
    let (_, touched) = host
        .submit_with(txn.clone(), &owner, CLOCK as i64, |_, _, touched| {
            Ok::<_, OpsError>(touched.clone())
        })
        .unwrap();
    let touched = touched.unwrap();
    assert!(touched.objects.contains(&id(70)));
    // Told nothing was touched, the shown project lacks the new object.
    let wrong = Touched::default();
    let refused = replica.advance(
        host.project(),
        host.revision().unwrap(),
        &[&txn],
        &wrong,
        Default::default(),
    );
    assert!(matches!(refused, Err(OpsError::RevisionMismatch)));
    assert_eq!(replica.revision(), &before);
    // Behind its host, it refuses work made on the host's revision.
    let own = transaction(
        host.revision().unwrap(),
        &phone,
        8,
        CLOCK as i64 + 8,
        vec![create_in(80, 3, &phone)],
    );
    assert!(replica.queue(own).is_err());
    // From the right list, a fresh replica takes the step, and shows what a
    // snapshot shows.
    let mut fresh = vw_ops::Replica::new(phone.clone(), {
        let earlier = fixture();
        earlier.snapshot().unwrap()
    })
    .unwrap();
    fresh
        .advance(
            host.project(),
            host.revision().unwrap(),
            &[&txn],
            &touched,
            Default::default(),
        )
        .unwrap();
    let rebuilt = vw_ops::Replica::new(phone, host.snapshot().unwrap()).unwrap();
    assert!(fresh.project() == rebuilt.project());
    assert_eq!(fresh.state_hash(), rebuilt.state_hash());
    assert_eq!(fresh.revision(), rebuilt.revision());
}
