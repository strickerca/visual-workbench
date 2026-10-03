use std::collections::BTreeSet;
use vw_model::{AssetId, DeviceId, Id, Project};
use vw_net::{
    AuthenticatedPeer, Frame, HostSync, LocalHello, NetError, Receive, Session, SyncReceiver,
    decode_frame, encode_frame,
};
use vw_ops::{HostSequencer, Replica};
use vw_proto::v1::{self as pb, envelope::Body, op::Kind, property_value::Value};
const CLOCK: u64 = 1_700_000_000_000;
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub seed: u64,
    pub accepted: usize,
    pub duplicate_receipts: usize,
    pub dropped: usize,
    pub duplicated: usize,
    pub delayed: usize,
    pub reconnects: usize,
    pub stale_input_discarded: usize,
    pub final_hash: Vec<u8>,
}
#[derive(Debug, thiserror::Error)]
pub enum SimError {
    #[error(transparent)]
    Net(#[from] NetError),
    #[error(transparent)]
    Ops(#[from] vw_ops::OpsError),
    #[error(transparent)]
    Model(#[from] vw_model::ModelError),
    #[error("simulation invariant: {0}")]
    Invariant(&'static str),
    #[error(
        "convergence failed: accepted={accepted}, host_seq={host_seq}, pending={pending:?}, replica_pending={replica_pending:?}, replica_seq={replica_seq:?}"
    )]
    Convergence {
        accepted: usize,
        host_seq: u64,
        pending: [usize; 2],
        replica_pending: [usize; 2],
        replica_seq: [u64; 2],
    },
    #[error("seed {seed}: {source}")]
    Seed { seed: u64, source: Box<SimError> },
}
type Result<T> = std::result::Result<T, SimError>;
struct Random(u64);
impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 ^ (self.0 >> 29)
    }
}
fn id(value: u64) -> Result<Id> {
    Ok(Id::from_parts(CLOCK + value, [0x61; 10])?)
}
fn uid(value: u64) -> Result<Option<pb::Uuid>> {
    Ok(Some(id(value)?.to_proto()))
}
fn local(device: DeviceId) -> LocalHello {
    LocalHello {
        device,
        platform: pb::Platform::Windows,
        app_version: "simulation".into(),
        label: "deterministic peer".into(),
        capabilities: BTreeSet::from(["ghost_ink".into(), "remote_input".into()]),
    }
}
fn sessions(client: &DeviceId, host: &DeviceId, nonce: u64) -> Result<(Session, Session)> {
    let a = local(client.clone());
    let b = local(host.clone());
    let connection = id(nonce)?;
    let (server, ack) = Session::accept(
        &b,
        &AuthenticatedPeer::for_simulation(client.clone()),
        &a.hello(&connection),
    )?;
    let client = Session::finish(
        &a,
        &AuthenticatedPeer::for_simulation(host.clone()),
        connection,
        &ack,
    )?;
    Ok((client, server))
}
fn txn(owner: &DeviceId, base: &pb::Revision, n: u64, kinds: Vec<Kind>) -> Result<pb::Transaction> {
    Ok(pb::Transaction {
        txn_id: uid(10_000 + n)?,
        project_id: uid(1)?,
        device_id: owner.to_string(),
        base_revision: Some(base.clone()),
        created_at_wall_ms: CLOCK as i64 + n as i64,
        gesture_id: None,
        ops: kinds
            .into_iter()
            .enumerate()
            .map(|(index, kind)| pb::Op {
                op_id: Some(pb::OpId {
                    device_id: owner.to_string(),
                    lamport: n * 100 + index as u64 + 1,
                }),
                kind: Some(kind),
            })
            .collect(),
    })
}
fn fixture(owner: &DeviceId) -> Result<HostSequencer> {
    let mut host = HostSequencer::new(
        Project::new(id(1)?, "Network simulation".into(), owner.clone()),
        owner.clone(),
    )?;
    let bytes = b"simulation immutable original";
    let asset = AssetId::hash(bytes).to_string();
    let setup = txn(
        owner,
        &host.revision()?,
        1,
        vec![
            Kind::AddAsset(pb::AddAsset {
                asset_id: asset.clone(),
                format: "png".into(),
                width: 128,
                height: 64,
                orientation: 1,
                bit_depth: 8,
                color_space: "sRGB".into(),
                byte_size: bytes.len() as u64,
                source: "import".into(),
                metadata_json: "{}".into(),
                ..Default::default()
            }),
            Kind::CreateDocument(pb::CreateDocument {
                document_id: uid(2)?,
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Document".into(),
                primary_asset_id: asset,
                capture: None,
            }),
            Kind::CreateLayer(pb::CreateLayer {
                layer_id: uid(3)?,
                document_id: uid(2)?,
                page_index: -1,
                name: "Annotations".into(),
                kind: "annotation".into(),
                order_key: "V".into(),
            }),
            Kind::CreateObject(pb::CreateObject {
                document_id: uid(2)?,
                state: Some(pb::ObjectState {
                    object_id: uid(4)?,
                    layer_id: uid(3)?,
                    order_key: "V".into(),
                    transform: Some(pb::Affine {
                        a: 1.0,
                        d: 1.0,
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
                    shape: Some(pb::object_state::Shape::Rect(pb::RectD {
                        x: 0.0,
                        y: 0.0,
                        w: 10.0,
                        h: 10.0,
                    })),
                    ..Default::default()
                }),
            }),
        ],
    )?;
    host.submit(setup, owner, CLOCK as i64)?;
    Ok(host)
}
struct Packet {
    at: u64,
    client: usize,
    to_host: bool,
    bytes: Vec<u8>,
}
fn transmit(
    random: &mut Random,
    network: &mut Vec<Packet>,
    frame: Frame,
    route: (usize, bool, u64, bool),
    report: &mut Report,
) -> Result<()> {
    let (client, to_host, tick, healthy) = route;
    let bytes = encode_frame(&frame)?;
    if !healthy && random.next().is_multiple_of(5) {
        report.dropped += 1;
        return Ok(());
    }
    let delay = if healthy { 0 } else { random.next() % 11 };
    if delay != 0 {
        report.delayed += 1;
    }
    if !healthy && random.next().is_multiple_of(4) {
        network.push(Packet {
            at: tick + delay + 1,
            client,
            to_host,
            bytes: bytes.clone(),
        });
        report.duplicated += 1;
    }
    network.push(Packet {
        at: tick + delay,
        client,
        to_host,
        bytes,
    });
    Ok(())
}
/// Two offline optimistic writers, packet loss/duplication/reordering/delay,
/// partition, reconnect with packets still in flight, lost receipts, eventual
/// fair delivery, and stale INPUT replay attempts on the new session.
pub fn run(seed: u64) -> Result<Report> {
    let host_device = DeviceId::from_bytes([1; 16]);
    let clients = [DeviceId::from_bytes([2; 16]), DeviceId::from_bytes([3; 16])];
    let initial = fixture(&host_device)?;
    let mut replicas = [
        Replica::new(clients[0].clone(), initial.snapshot()?)?,
        Replica::new(clients[1].clone(), initial.snapshot()?)?,
    ];
    let mut receivers = [
        SyncReceiver::new(initial.clone()),
        SyncReceiver::new(initial.clone()),
    ];
    let mut host = HostSync::new(initial)?;
    let mut peers = [
        sessions(&clients[0], &host_device, 100)?,
        sessions(&clients[1], &host_device, 101)?,
    ];
    let mut pending: [Vec<pb::Transaction>; 2] = std::array::from_fn(|_| Vec::new());
    for client in 0..2 {
        for edit in 0..6 {
            let value = 1.0 + ((seed.wrapping_add(edit).wrapping_add(client as u64)) % 50) as f64;
            let transaction = txn(
                &clients[client],
                replicas[client].revision(),
                10 + client as u64 * 10 + edit,
                vec![Kind::SetProperty(pb::SetProperty {
                    object_id: uid(4)?,
                    property: "style.width".into(),
                    value: Some(pb::PropertyValue {
                        value: Some(Value::Number(value)),
                    }),
                })],
            )?;
            replicas[client].queue(transaction.clone())?;
            pending[client].push(transaction);
        }
    }
    let old_grant = id(200)?;
    let capture = id(201)?;
    peers[0]
        .1
        .input
        .grant(old_grant.clone(), capture.clone(), 1, 0)?;
    let stale_input = pb::InputEvent {
        input_session_id: Some(old_grant.to_proto()),
        capture_session_id: Some(capture.to_proto()),
        geometry_revision: 1,
        input_seq: 1,
        event: Some(pb::input_event::Event::Key(pb::KeyEvent {
            ..Default::default()
        })),
    };
    peers[0].0.enqueue(Body::InputEvent(stale_input.clone()))?;
    let stale_frame = peers[0]
        .0
        .next_frame()?
        .ok_or(SimError::Invariant("input packet"))?;
    let mut report = Report {
        seed,
        ..Default::default()
    };
    let mut random = Random(seed);
    let mut network = Vec::new();
    for tick in 0_u64..160 {
        let healthy = tick >= 90;
        if tick == 45 {
            for client in 0..2 {
                peers[client].0.disconnect();
                peers[client].1.disconnect();
                peers[client] = sessions(&clients[client], &host_device, 300 + client as u64)?;
                report.reconnects += 1;
            }
            peers[0].1.input.grant(id(400)?, capture.clone(), 1, tick)?;
            if peers[0].1.receive(stale_frame.clone(), tick)? != Receive::Discard {
                return Err(SimError::Invariant("old connection input replay"));
            }
            peers[0].0.enqueue(Body::InputEvent(stale_input.clone()))?;
            let fresh_frame = peers[0]
                .0
                .next_frame()?
                .ok_or(SimError::Invariant("stale grant packet"))?;
            if peers[0].1.receive(fresh_frame, tick)? != Receive::Discard {
                return Err(SimError::Invariant("old grant input replay"));
            }
            report.stale_input_discarded += 2;
        }
        if tick.is_multiple_of(7) || healthy {
            for client in 0..2 {
                for transaction in &pending[client] {
                    peers[client].0.enqueue(Body::Txn(transaction.clone()))?;
                }
                peers[client]
                    .0
                    .enqueue(Body::SyncRequest(receivers[client].request()?))?;
                while let Some(frame) = peers[client].0.next_frame()? {
                    transmit(
                        &mut random,
                        &mut network,
                        frame,
                        (client, true, tick, healthy),
                        &mut report,
                    )?;
                }
            }
        }
        if (20..40).contains(&tick) {
            continue;
        }
        // Stable extraction matters during the healthy recovery period.
        // swap_remove would reorder every batch even after faults stop; a
        // SyncRequest could then permanently overtake its preceding retry and
        // cause the Session's reliable sequence guard to discard that edit.
        let (mut due, in_flight): (Vec<_>, Vec<_>) =
            network.into_iter().partition(|packet| packet.at <= tick);
        network = in_flight;
        if !healthy && random.next().is_multiple_of(2) {
            due.reverse();
        }
        for packet in due {
            let client = packet.client;
            let frame = decode_frame(&packet.bytes)?;
            let received = if packet.to_host {
                peers[client].1.receive(frame, tick)?
            } else {
                peers[client].0.receive(frame, tick)?
            };
            if let Receive::Deliver(body) = received {
                match body {
                    Body::Txn(transaction) => {
                        let accepted =
                            host.submit(transaction, &clients[client], CLOCK as i64 + tick as i64)?;
                        if accepted.duplicate {
                            report.duplicate_receipts += 1;
                        } else {
                            report.accepted += 1;
                        }
                        peers[client].1.enqueue(Body::TxnAck(accepted.ack))?;
                    }
                    Body::SyncRequest(request) => {
                        let response = host.respond(&request)?;
                        if response.checkpoint.is_some() {
                            return Err(SimError::Invariant("unexpected valid-base checkpoint"));
                        }
                        peers[client].1.enqueue(Body::SyncBatch(response.batch))?;
                    }
                    Body::SyncBatch(batch) => {
                        match receivers[client].receive(batch, &host_device) {
                            Ok(Some(snapshot)) => {
                                replicas[client].receive(snapshot)?;
                                pending[client].retain(|txn| {
                                    Id::from_proto(txn.txn_id.as_ref()).map_or(true, |id| {
                                        receivers[client].host().accepted_transaction(&id).is_none()
                                    })
                                });
                            }
                            Err(NetError::Resync) => {}
                            Err(error) => return Err(error.into()),
                            Ok(None) => return Err(SimError::Invariant("unexpected manifest")),
                        }
                    }
                    Body::TxnAck(_) => {} // Receipt alone never removes durable work.
                    _ => return Err(SimError::Invariant("unexpected wire body")),
                }
            }
        }
        for (client, peer) in peers.iter_mut().enumerate() {
            while let Some(frame) = peer.1.next_frame()? {
                transmit(
                    &mut random,
                    &mut network,
                    frame,
                    (client, false, tick + 1, healthy),
                    &mut report,
                )?;
            }
        }
        let current = host.host().revision()?;
        if healthy
            && pending.iter().all(Vec::is_empty)
            && replicas.iter().all(|r| r.revision() == &current)
        {
            break;
        }
    }
    let revision = host.host().revision()?;
    if report.accepted != 12
        || revision.host_seq != 13
        || pending.iter().any(|p| !p.is_empty())
        || replicas
            .iter()
            .any(|r| !r.pending().is_empty() || r.revision() != &revision)
    {
        return Err(SimError::Convergence {
            accepted: report.accepted,
            host_seq: revision.host_seq,
            pending: std::array::from_fn(|n| pending[n].len()),
            replica_pending: std::array::from_fn(|n| replicas[n].pending().len()),
            replica_seq: std::array::from_fn(|n| replicas[n].revision().host_seq),
        });
    }
    report.final_hash = revision.state_hash;
    Ok(report)
}
#[cfg(test)]
mod tests {
    #[test]
    fn seed_one_recovery_does_not_starve_the_last_pending_edit() -> Result<(), super::SimError> {
        let report = super::run(1)?;
        assert_eq!(report.accepted, 12);
        assert_eq!(report.stale_input_discarded, 2);
        assert!(report.dropped > 0 && report.delayed > 0 && report.reconnects == 2);
        Ok(())
    }
    #[test]
    fn ten_thousand_deterministic_fault_seeds_converge() -> Result<(), super::SimError> {
        let mut dropped = 0;
        let mut duplicates = 0;
        let mut retry_receipts = 0;
        for seed in 0..10_000 {
            let report = super::run(seed).map_err(|source| super::SimError::Seed {
                seed,
                source: Box::new(source),
            })?;
            dropped += report.dropped;
            duplicates += report.duplicated;
            retry_receipts += report.duplicate_receipts;
            assert_eq!(report.accepted, 12);
            assert_eq!(report.stale_input_discarded, 2);
            if (seed + 1) % 100 == 0 {
                eprintln!("simulation progress: {}/10000 seeds verified", seed + 1);
            }
        }
        assert!(dropped > 0 && duplicates > 0 && retry_receipts > 0);
        eprintln!(
            "PASS seeds=10000 accepted=120000 stale_input_discarded=20000 dropped_packets={dropped} duplicated_packets={duplicates} duplicate_receipts={retry_receipts}"
        );
        Ok(())
    }
}
