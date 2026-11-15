//! A phone mirrors a project the PC owns. Every sync update the PC sends is
//! decided again on the phone, stored, and shown with the phone's own edits
//! that are still waiting on top. These tests drive that through the real
//! worker, store and replica, and measure what one update costs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::replication as sync;
use crate::{ProjectSession, project::SessionPermit};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use vw_model::{DeviceId, Document, Id, Layer, Project};
use vw_net::HostSync;
use vw_ops::HostSequencer;
use vw_proto::v1::{self as pb, op::Kind};
use vw_store::{BlobStore, ProjectStore};

const CLOCK: u64 = 1_700_000_000_000;
const SAMPLES: usize = 48;
fn id(n: u64) -> Id {
    Id::from_parts(CLOCK + n, [0x73; 10]).unwrap()
}
fn uid(n: u64) -> Option<pb::Uuid> {
    Some(id(n).to_proto())
}
/// The PC.
fn pc() -> DeviceId {
    DeviceId::from_bytes([1; 16])
}
/// The phone.
fn phone() -> DeviceId {
    DeviceId::from_bytes([2; 16])
}
fn project() -> Project {
    let mut value = Project::new(id(1), "Mirror fixture".into(), pc());
    value.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: uid(2),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Mirror".into(),
                ..Default::default()
            },
            pages: vec![],
            created_at_ms: 0,
        },
    );
    value.layers.insert(
        id(3),
        Layer {
            definition: pb::CreateLayer {
                layer_id: uid(3),
                document_id: uid(2),
                page_index: -1,
                name: "Ink".into(),
                kind: "annotation".into(),
                order_key: "V".into(),
            },
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: "normal".into(),
        },
    );
    value
}
fn stroke(n: u64, author: &DeviceId) -> Kind {
    let at = |i: usize| i as f64;
    Kind::CreateObject(pb::CreateObject {
        document_id: uid(2),
        state: Some(pb::ObjectState {
            object_id: uid(1_000_000 + n),
            layer_id: uid(3),
            order_key: "V".into(),
            transform: Some(pb::Affine {
                a: 1.0,
                d: 1.0,
                ..Default::default()
            }),
            style: Some(pb::Style {
                stroke: Some(pb::Color { rgba: 0x203040ff }),
                width: 4.0,
                ..Default::default()
            }),
            role: pb::Role::None as i32,
            created_by: author.to_string(),
            created_at_ms: CLOCK as i64,
            shape: Some(pb::object_state::Shape::Stroke(pb::Stroke {
                x: (0..SAMPLES).map(|i| (n % 900) as f64 + at(i)).collect(),
                y: (0..SAMPLES)
                    .map(|i| (n % 700) as f64 + at(i) * 0.5)
                    .collect(),
                t_ms: (0..SAMPLES).map(|i| (i * 4) as u32).collect(),
                pressure: vec![0.6; SAMPLES],
                tilt: vec![],
                orientation: vec![],
                brush: Some(pb::Brush {
                    family: "pen".into(),
                    algorithm_version: 1,
                    base_width: 4.0,
                    pressure_curve: vec![0.0, 0.0, 0.25, 0.25, 0.75, 0.75, 1.0, 1.0],
                    stabilization: 0.0,
                }),
            })),
            ..Default::default()
        }),
    })
}
fn moved(n: u64, by: f64) -> Kind {
    Kind::SetProperty(pb::SetProperty {
        object_id: uid(1_000_000 + n),
        property: "transform".into(),
        value: Some(pb::PropertyValue {
            value: Some(pb::property_value::Value::Transform(pb::Affine {
                a: 1.0,
                d: 1.0,
                e: by,
                ..Default::default()
            })),
        }),
    })
}
fn transaction(base: pb::Revision, author: &DeviceId, n: u64, kinds: Vec<Kind>) -> pb::Transaction {
    pb::Transaction {
        txn_id: uid(10_000_000 + n),
        project_id: uid(1),
        device_id: author.to_string(),
        base_revision: Some(base),
        created_at_wall_ms: 1000 + n as i64,
        ops: kinds
            .into_iter()
            .enumerate()
            .map(|(index, kind)| pb::Op {
                op_id: Some(pb::OpId {
                    device_id: author.to_string(),
                    lamport: n * 64 + index as u64 + 1,
                }),
                kind: Some(kind),
            })
            .collect(),
        gesture_id: Some(id(20_000_000 + n).to_proto()),
    }
}

/// The PC, in memory, with `strokes` strokes, every other one by the phone.
fn pc_with(strokes: u64) -> HostSync {
    let mut host = HostSync::new(HostSequencer::new(project(), pc()).unwrap()).unwrap();
    for n in 0..strokes {
        let author = if n % 2 == 0 { pc() } else { phone() };
        let txn = transaction(
            host.host().revision().unwrap(),
            &author,
            n,
            vec![stroke(n, &author)],
        );
        host.submit(txn, &author, 2000 + n as i64).unwrap();
    }
    host
}
/// The phone's mirror of what the PC holds now, opened as the apps open it.
fn mirror_of(host: &HostSync, path: &Path) -> Arc<ProjectSession> {
    let store = ProjectStore::create_authenticated_checkpoint(
        path,
        &host.host().checkpoint_bytes().unwrap(),
        &pc(),
        &phone(),
        1,
    )
    .unwrap();
    ProjectSession::from_store(
        store,
        BlobStore::new(path).unwrap(),
        SessionPermit::acquire().unwrap(),
    )
    .unwrap()
}
/// What the PC answers the phone's next request with.
async fn batch(host: &HostSync, mirror: &ProjectSession) -> pb::SyncBatch {
    let request = sync::request(mirror).await.unwrap();
    let response = host.respond(&request).unwrap();
    assert!(response.checkpoint.is_none());
    response.batch
}
/// The phone's own edit, shown at once and waiting for the PC.
async fn queue(mirror: &ProjectSession, txn: pb::Transaction) {
    mirror
        .worker
        .call(move |state| {
            let device = phone();
            state.commit(&txn, &device, txn.created_at_wall_ms)
        })
        .await
        .unwrap();
}
async fn revision(mirror: &ProjectSession) -> pb::Revision {
    sync::state(mirror).await.unwrap().revision
}

/// One update of each ordinary kind: a stroke by the PC, a move of an early
/// stroke by the PC, the phone's own stroke coming back acknowledged, and a
/// PC stroke while the phone has one waiting. Returns how long `receive`
/// took for each.
async fn updates(
    host: &mut HostSync,
    mirror: &ProjectSession,
    first: u64,
    rounds: u64,
) -> [Vec<Duration>; 4] {
    let mut times = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for round in 0..rounds {
        let n = first + round * 8;
        let time = async |host: &mut HostSync, slot: usize, times: &mut [Vec<Duration>; 4]| {
            let update = batch(host, mirror).await;
            let started = Instant::now();
            assert!(sync::receive(mirror, update, pc()).await.unwrap());
            times[slot].push(started.elapsed());
        };
        let txn = transaction(
            host.host().revision().unwrap(),
            &pc(),
            n,
            vec![stroke(n, &pc())],
        );
        host.submit(txn, &pc(), 3000 + n as i64).unwrap();
        time(host, 0, &mut times).await;

        let txn = transaction(
            host.host().revision().unwrap(),
            &pc(),
            n + 1,
            vec![moved(round * 2, n as f64)],
        );
        host.submit(txn, &pc(), 3001 + n as i64).unwrap();
        time(host, 1, &mut times).await;

        let own = transaction(
            revision(mirror).await,
            &phone(),
            n + 2,
            vec![stroke(n + 2, &phone())],
        );
        queue(mirror, own.clone()).await;
        host.submit(own, &phone(), 3002 + n as i64).unwrap();
        time(host, 2, &mut times).await;

        let waiting = transaction(
            revision(mirror).await,
            &phone(),
            n + 3,
            vec![stroke(n + 3, &phone())],
        );
        queue(mirror, waiting.clone()).await;
        let txn = transaction(
            host.host().revision().unwrap(),
            &pc(),
            n + 4,
            vec![stroke(n + 4, &pc())],
        );
        host.submit(txn, &pc(), 3004 + n as i64).unwrap();
        time(host, 3, &mut times).await;
        host.submit(waiting, &phone(), 3005 + n as i64).unwrap();
        let update = batch(host, mirror).await;
        assert!(sync::receive(mirror, update, pc()).await.unwrap());
    }
    times
}

fn median(values: &mut [Duration]) -> f64 {
    values.sort();
    values[values.len() / 2].as_secs_f64() * 1000.0
}

/// What one sync update costs the phone as the project grows.
/// Run: cargo test -p vw-ffi --lib mirror_update_cost -- --ignored --nocapture
/// (VW_BENCH_SIZES=2000,8000 by default).
#[tokio::test]
#[ignore = "measurement; prints a table"]
async fn mirror_update_cost() {
    let sizes: Vec<u64> = std::env::var("VW_BENCH_SIZES")
        .ok()
        .map(|sizes| {
            sizes
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect()
        })
        .unwrap_or_else(|| vec![2000, 8000]);
    println!(
        "strokes | a stroke by the PC | the PC moves an early stroke | the phone's own stroke back | a PC stroke over a waiting one"
    );
    for strokes in sizes {
        let temp = tempfile::tempdir().unwrap();
        let mut host = pc_with(strokes);
        let mirror = mirror_of(&host, &temp.path().join("phone"));
        let [mut a, mut b, mut c, mut d] = updates(&mut host, &mirror, 100_000, 7).await;
        println!(
            "{strokes:>7} | {:>15.1} ms | {:>25.1} ms | {:>24.1} ms | {:>28.1} ms",
            median(&mut a),
            median(&mut b),
            median(&mut c),
            median(&mut d)
        );
        mirror.close().await.unwrap();
    }
}

fn cancel() -> Arc<crate::Cancellation> {
    Arc::new(crate::Cancellation::new())
}
fn options(n: u64, info: &crate::ProjectInfo) -> crate::EditOptions {
    crate::EditOptions {
        transaction_id: id(5_000_000 + n).to_string(),
        document_id: id(2).to_string(),
        device_id: phone().to_string(),
        lamport: info.next_lamport,
        created_at_ms: 4000 + n as i64,
    }
}
/// The phone moves one of its strokes, through the call the app makes.
async fn phone_moves(mirror: &ProjectSession, n: u64, stroke: u64, by: f64) {
    let info = mirror.info().await.unwrap();
    mirror
        .apply_edit(
            options(n, &info),
            vec![crate::EditCommand::Transform {
                object_id: id(1_000_000 + stroke).to_string(),
                transform: crate::Transform {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    e: by,
                    f: 0.0,
                },
            }],
            cancel(),
        )
        .await
        .unwrap();
}
/// The phone undoes or redoes its last edit, through the call the app makes.
async fn phone_undoes(mirror: &ProjectSession, n: u64, redo: bool) {
    let info = mirror.info().await.unwrap();
    mirror
        .undo_redo(options(n, &info), redo, cancel())
        .await
        .unwrap();
}
/// The phone's work still waiting, oldest first.
async fn outbox(mirror: &ProjectSession) -> Vec<pb::Transaction> {
    mirror
        .worker
        .call(|state| Ok(state.store()?.pending()?))
        .await
        .unwrap()
}
/// The PC accepts the phone's oldest waiting work, `count` of it.
async fn pc_accepts(host: &mut HostSync, mirror: &ProjectSession, count: usize, at: i64) {
    for (index, txn) in outbox(mirror).await.into_iter().take(count).enumerate() {
        host.submit(txn, &phone(), at + index as i64).unwrap();
    }
}
fn pc_edits(host: &mut HostSync, n: u64, kinds: Vec<Kind>) {
    let txn = transaction(host.host().revision().unwrap(), &pc(), n, kinds);
    host.submit(txn, &pc(), 6000 + n as i64).unwrap();
}

/// After the update, what the phone holds is what it would hold had it
/// installed a checkpoint of the PC and built everything from it: the stored
/// sequencing state, the replica with the work still waiting, and the undo
/// history.
async fn sync_and_compare(host: &HostSync, mirror: &ProjectSession, step: &str) {
    let update = batch(host, mirror).await;
    assert!(sync::receive(mirror, update, pc()).await.unwrap(), "{step}");
    let expected = host.host().checkpoint_bytes().unwrap();
    let step = step.to_owned();
    mirror
        .worker
        .call(move |state| {
            let store = state.store()?;
            assert!(
                store.checkpoint_bytes()? == expected,
                "{step}: stored state"
            );
            let replica = state.replica.as_ref().unwrap();
            let rebuilt = sync::rebuild_replica(state).unwrap();
            assert_eq!(replica.revision(), rebuilt.revision(), "{step}: revision");
            assert!(
                replica.project() == rebuilt.project(),
                "{step}: shown project"
            );
            assert_eq!(
                replica.state_hash(),
                rebuilt.state_hash(),
                "{step}: shown hash"
            );
            assert_eq!(
                replica.state_hash(),
                &replica.project().state_hash().unwrap(),
                "{step}: shown hash is the project's"
            );
            let waiting = |replica: &vw_ops::Replica| {
                replica
                    .pending()
                    .iter()
                    .map(|pending| (pending.transaction.clone(), pending.blocked_reason.clone()))
                    .collect::<Vec<_>>()
            };
            assert_eq!(waiting(replica), waiting(&rebuilt), "{step}: waiting work");
            let restored = crate::editor::History::restore(store).unwrap();
            let history = &state.history;
            assert_eq!(
                history.next_lamport, restored.next_lamport,
                "{step}: counter"
            );
            for redo in [false, true] {
                let lamport = history.next_lamport;
                assert_eq!(
                    history.undo.operation(redo, lamport).ok(),
                    restored.undo.operation(redo, lamport).ok(),
                    "{step}: {} offered",
                    if redo { "redo" } else { "undo" }
                );
            }
            Ok(())
        })
        .await
        .unwrap();
}

/// Every ordinary kind of update, each compared with building the phone's
/// state from a checkpoint of the PC.
#[tokio::test]
async fn an_update_taken_in_step_by_step_equals_one_installed_from_a_checkpoint() {
    let temp = tempfile::tempdir().unwrap();
    let mut host = pc_with(60);
    let path = temp.path().join("phone");
    let mirror = mirror_of(&host, &path);
    let mut n = 500;
    let mut next = || {
        n += 1;
        n
    };

    let k = next();
    pc_edits(&mut host, k, vec![stroke(k, &pc()), moved(0, 3.0)]);
    sync_and_compare(&host, &mirror, "an edit by the PC").await;

    phone_moves(&mirror, next(), 1, 4.0).await;
    sync_and_compare(&host, &mirror, "nothing new while the phone waits").await;
    pc_accepts(&mut host, &mirror, 1, 7000).await;
    sync_and_compare(&host, &mirror, "the phone's move comes back").await;

    phone_undoes(&mirror, next(), false).await;
    pc_accepts(&mut host, &mirror, 1, 7010).await;
    sync_and_compare(&host, &mirror, "its undo comes back").await;

    // A redo waits while the PC moves the same stroke.
    phone_undoes(&mirror, next(), true).await;
    let k = next();
    pc_edits(&mut host, k, vec![moved(1, 9.0)]);
    sync_and_compare(&host, &mirror, "a PC edit under a waiting redo").await;
    pc_accepts(&mut host, &mirror, 1, 7020).await;
    sync_and_compare(&host, &mirror, "the redo comes back").await;

    // The PC deletes a stroke the phone has a move of waiting.
    phone_moves(&mirror, next(), 3, 2.0).await;
    let k = next();
    pc_edits(
        &mut host,
        k,
        vec![Kind::DeleteObject(pb::DeleteObject {
            object_id: uid(1_000_003),
        })],
    );
    sync_and_compare(&host, &mirror, "the moved stroke is deleted").await;
    pc_accepts(&mut host, &mirror, 1, 7030).await;
    sync_and_compare(&host, &mirror, "the move of a deleted stroke comes back").await;

    // Two edits wait; one comes back with a PC edit, then the other.
    phone_moves(&mirror, next(), 5, 1.0).await;
    phone_moves(&mirror, next(), 7, 1.0).await;
    pc_accepts(&mut host, &mirror, 1, 7040).await;
    let k = next();
    pc_edits(&mut host, k, vec![stroke(k, &pc())]);
    sync_and_compare(&host, &mirror, "one of two back, with a PC edit").await;
    pc_accepts(&mut host, &mirror, 1, 7050).await;
    sync_and_compare(&host, &mirror, "the second back").await;

    // An edit and its undo wait; the edit comes back alone, so the undo now
    // undoes accepted work.
    phone_moves(&mirror, next(), 9, 6.0).await;
    phone_undoes(&mirror, next(), false).await;
    pc_accepts(&mut host, &mirror, 1, 7060).await;
    sync_and_compare(&host, &mirror, "an edit back under its waiting undo").await;
    pc_accepts(&mut host, &mirror, 1, 7070).await;
    sync_and_compare(&host, &mirror, "the undo back").await;

    // An edit and its undo come back together.
    phone_moves(&mirror, next(), 11, 6.0).await;
    phone_undoes(&mirror, next(), false).await;
    pc_accepts(&mut host, &mirror, 2, 7080).await;
    sync_and_compare(&host, &mirror, "an edit and its undo back together").await;

    // The PC locks the layer under a waiting move, which is then refused
    // and shown as it was, and unlocks it again.
    phone_moves(&mirror, next(), 15, 8.0).await;
    let lock = |locked: bool| {
        vec![Kind::UpdateLayer(pb::UpdateLayer {
            layer_id: uid(3),
            locked: Some(locked),
            ..Default::default()
        })]
    };
    let k = next();
    pc_edits(&mut host, k, lock(true));
    sync_and_compare(&host, &mirror, "the layer locked under a waiting move").await;
    let k = next();
    pc_edits(&mut host, k, lock(false));
    sync_and_compare(&host, &mirror, "the layer unlocked again").await;
    pc_accepts(&mut host, &mirror, 1, 7085).await;
    sync_and_compare(&host, &mirror, "the move back after all").await;

    // Layers, the title and several PC transactions in one update.
    let k = next();
    pc_edits(
        &mut host,
        k,
        vec![
            Kind::UpdateLayer(pb::UpdateLayer {
                layer_id: uid(3),
                name: Some("Renamed".into()),
                opacity: Some(0.75),
                ..Default::default()
            }),
            Kind::UpdateDocument(pb::UpdateDocument {
                document_id: uid(2),
                title: "After".into(),
            }),
        ],
    );
    phone_moves(&mirror, next(), 13, 2.5).await;
    for _ in 0..5 {
        let k = next();
        pc_edits(&mut host, k, vec![stroke(k, &pc()), moved(k % 40, 1.25)]);
    }
    sync_and_compare(&host, &mirror, "six PC transactions under a waiting move").await;
    pc_accepts(&mut host, &mirror, 1, 7090).await;
    sync_and_compare(&host, &mirror, "the move back").await;

    // Undo of older accepted work, whose preview the replica asks the
    // store for when the undo is made.
    phone_undoes(&mirror, next(), false).await;
    phone_undoes(&mirror, next(), false).await;
    sync_and_compare(&host, &mirror, "two undos of accepted work waiting").await;
    pc_accepts(&mut host, &mirror, 2, 7100).await;
    sync_and_compare(&host, &mirror, "both back").await;

    // Reopened, the store's rows equal a full rebuild of them.
    let state = host.host().checkpoint_bytes().unwrap();
    mirror.close().await.unwrap();
    let store = ProjectStore::open(&path).unwrap();
    assert!(store.checkpoint_bytes().unwrap() == state);
    store.integrity_check().unwrap();
}
