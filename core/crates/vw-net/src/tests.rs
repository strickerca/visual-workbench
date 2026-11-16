#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use std::{collections::BTreeSet, io::Cursor};
use vw_model::{AssetId, DeviceId, Id, Project};
use vw_ops::HostSequencer;
use vw_proto::v1::{self as pb, envelope::Body, op::Kind};
fn id(n: u64) -> Id {
    Id::from_parts(1_700_000_000_000 + n, [0x44; 10]).unwrap()
}
fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
fn temporary() -> tempfile::TempDir {
    #[cfg(target_os = "android")]
    {
        tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()
    }
    #[cfg(not(target_os = "android"))]
    {
        tempfile::tempdir().unwrap()
    }
}
fn local(n: u8) -> LocalHello {
    LocalHello {
        device: device(n),
        platform: pb::Platform::Windows,
        app_version: "test".into(),
        label: "test peer".into(),
        capabilities: BTreeSet::from(["remote_input".into(), "frame_tiles".into()]),
    }
}
fn sessions(n: u64) -> (Session, Session) {
    let a = local(2);
    let b = local(1);
    let connection = id(n);
    let (host, ack) = Session::accept(
        &b,
        &AuthenticatedPeer::for_simulation(device(2)),
        &a.hello(&connection),
    )
    .unwrap();
    (
        Session::finish(
            &a,
            &AuthenticatedPeer::for_simulation(device(1)),
            connection,
            &ack,
        )
        .unwrap(),
        host,
    )
}
fn frame(body: Body, seq: u64) -> Frame {
    let channel = channel_for(&body);
    Frame {
        channel,
        envelope: pb::Envelope {
            channel: channel as i32,
            seq,
            connection_id: Some(id(99).to_proto()),
            body: Some(body),
        },
    }
}
fn media(n: u64) -> Body {
    Body::FrameTiles(pb::FrameTiles {
        capture_session_id: Some(id(44).to_proto()),
        frame_id: n,
        keyframe: true,
        ..Default::default()
    })
}
fn initial() -> HostSequencer {
    HostSequencer::new(
        Project::new(id(1), "Transport fixture".into(), device(1)),
        device(1),
    )
    .unwrap()
}
fn transaction(host: &HostSequencer, n: u64) -> pb::Transaction {
    let data = n.to_le_bytes();
    pb::Transaction {
        txn_id: Some(id(100 + n).to_proto()),
        project_id: Some(id(1).to_proto()),
        device_id: device(2).to_string(),
        base_revision: Some(host.revision().unwrap()),
        created_at_wall_ms: 1000 + n as i64,
        gesture_id: None,
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: device(2).to_string(),
                lamport: n + 1,
            }),
            kind: Some(Kind::AddAsset(pb::AddAsset {
                asset_id: AssetId::hash(&data).to_string(),
                format: "png".into(),
                width: 1,
                height: 1,
                orientation: 1,
                bit_depth: 8,
                byte_size: data.len() as u64,
                color_space: "sRGB".into(),
                source: "import".into(),
                metadata_json: "{}".into(),
                ..Default::default()
            })),
        }],
    }
}
#[test]
fn all_channels_round_trip_and_header_lies_fail_closed() {
    for body in [
        Body::Ping(pb::Ping::default()),
        Body::SyncRequest(pb::SyncRequest::default()),
        Body::CursorUpdate(pb::CursorUpdate {
            document_id: Some(id(1).to_proto()),
            ..Default::default()
        }),
        Body::BlobAck(pb::BlobAck::default()),
        media(1),
        Body::InputStatus(pb::InputStatus::default()),
    ] {
        let original = frame(body, 1);
        let bytes = encode_frame(&original).unwrap();
        assert_eq!(decode_frame(&bytes).unwrap(), original);
        let mut bad = bytes.clone();
        bad[5] ^= 2;
        assert!(decode_frame(&bad).is_err());
        let mut bad = bytes.clone();
        bad[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode_frame(&bad).is_err());
        let mut bad = original;
        bad.envelope.channel = 0;
        assert!(encode_frame(&bad).is_err());
    }
}
#[test]
fn handshake_rejects_wrong_peer_major_minor_and_ack_nonce() {
    let local = local(1);
    let peer = AuthenticatedPeer::for_simulation(device(2));
    let hello = super::tests::local(2).hello(&id(1));
    let mut bad = hello.clone();
    bad.device_id = device(3).to_string();
    assert!(Session::accept(&local, &peer, &bad).is_err());
    let mut bad = hello.clone();
    bad.protocol_major += 1;
    assert!(Session::accept(&local, &peer, &bad).is_err());
    for minor in 0..PROTOCOL_MINOR {
        let mut bad = hello.clone();
        bad.protocol_minor = minor;
        assert!(Session::accept(&local, &peer, &bad).is_err());
    }
    let (_, mut ack) = Session::accept(&local, &peer, &hello).unwrap();
    ack.connection_id = Some(id(2).to_proto());
    assert!(Session::finish(&local, &peer, id(1), &ack).is_err());
}
#[test]
fn latest_media_and_ephemeral_replacement_keeps_control_responsive() {
    let (mut a, mut b) = sessions(99);
    for n in 1..=100 {
        a.enqueue(media(n)).unwrap();
        a.enqueue(Body::CursorUpdate(pb::CursorUpdate {
            document_id: Some(id(1).to_proto()),
            tool: n.to_string(),
            ..Default::default()
        }))
        .unwrap();
    }
    a.enqueue(Body::Ping(pb::Ping {
        nonce: 7,
        t_sent_ns: 10,
    }))
    .unwrap();
    assert!(matches!(
        a.next_frame().unwrap().unwrap().envelope.body,
        Some(Body::Ping(_))
    ));
    let cursor = a.next_frame().unwrap().unwrap();
    assert!(
        matches!(cursor.envelope.body.as_ref(), Some(Body::CursorUpdate(v)) if v.tool == "100")
    );
    let latest = a.next_frame().unwrap().unwrap();
    assert!(
        matches!(latest.envelope.body.as_ref(), Some(Body::FrameTiles(v)) if v.frame_id == 100)
    );
    assert!(matches!(
        b.receive(latest.clone(), 0).unwrap(),
        Receive::Deliver(_)
    ));
    assert_eq!(b.receive(latest, 0).unwrap(), Receive::Discard);
    assert!(a.next_frame().unwrap().is_none());
}
#[test]
fn bulk_fairness_and_visible_blob_priority() {
    let (mut a, _) = sessions(99);
    for priority in [10, 0, 3] {
        a.enqueue(Body::BlobRequest(pb::BlobRequest {
            asset_id: AssetId::hash(&[priority as u8]).to_string(),
            priority,
            ..Default::default()
        }))
        .unwrap();
    }
    a.enqueue(media(1)).unwrap();
    assert!(
        matches!(a.next_frame().unwrap().unwrap().envelope.body, Some(Body::BlobRequest(v)) if v.priority == 0)
    );
    assert_eq!(a.next_frame().unwrap().unwrap().channel, pb::Channel::Media);
    assert!(
        matches!(a.next_frame().unwrap().unwrap().envelope.body, Some(Body::BlobRequest(v)) if v.priority == 3)
    );
}
#[test]
fn predictive_media_replacement_requests_recovery_keyframe() {
    let (mut session, _) = sessions(99);
    session.enqueue(media(1)).unwrap();
    session.next_frame().unwrap();
    let delta = |n| {
        Body::FrameTiles(pb::FrameTiles {
            capture_session_id: Some(id(44).to_proto()),
            frame_id: n,
            keyframe: false,
            ..Default::default()
        })
    };
    session.enqueue(delta(2)).unwrap();
    assert!(matches!(
        session.enqueue(delta(3)),
        Err(NetError::KeyframeRequired)
    ));
    assert!(session.next_frame().unwrap().is_none());
    assert!(matches!(
        session.enqueue(delta(4)),
        Err(NetError::KeyframeRequired)
    ));
    session.enqueue(media(5)).unwrap();
    assert!(session.next_frame().unwrap().is_some());
}
#[test]
fn sender_media_watermark_ignores_late_frames_and_recovers_predictive_gaps() {
    let (mut sender, _) = sessions(99);
    sender.enqueue(media(100)).unwrap();
    sender.enqueue(media(90)).unwrap();
    sender.enqueue(media(100)).unwrap();
    assert!(
        matches!(sender.next_frame().unwrap().unwrap().envelope.body,Some(Body::FrameTiles(v)) if v.frame_id==100)
    );
    let delta = |n| {
        Body::FrameTiles(pb::FrameTiles {
            capture_session_id: Some(id(44).to_proto()),
            frame_id: n,
            keyframe: false,
            ..Default::default()
        })
    };
    sender.enqueue(delta(101)).unwrap();
    sender.next_frame().unwrap();
    sender.enqueue(delta(99)).unwrap();
    assert!(sender.next_frame().unwrap().is_none());
    assert!(matches!(
        sender.enqueue(delta(103)),
        Err(NetError::KeyframeRequired)
    ));
    assert!(matches!(
        sender.enqueue(delta(104)),
        Err(NetError::KeyframeRequired)
    ));
    sender.enqueue(media(104)).unwrap(); // The rejected frame can be regenerated independently.
    assert!(
        matches!(sender.next_frame().unwrap().unwrap().envelope.body,Some(Body::FrameTiles(v)) if v.frame_id==104&&v.keyframe)
    );
    sender.enqueue(delta(105)).unwrap();
    sender.next_frame().unwrap();
    let other = Body::FrameTiles(pb::FrameTiles {
        capture_session_id: Some(id(45).to_proto()),
        frame_id: 1,
        keyframe: true,
        ..Default::default()
    });
    sender.enqueue(other).unwrap();
    assert!(sender.next_frame().unwrap().is_some());
}
#[test]
fn receiver_predictive_gap_requires_keyframe_without_reviving_old_frames() {
    let (_, mut receiver) = sessions(99);
    assert!(matches!(
        receiver.receive(frame(media(100), 1), 0).unwrap(),
        Receive::Deliver(_)
    ));
    let delta = |n| {
        Body::FrameTiles(pb::FrameTiles {
            capture_session_id: Some(id(44).to_proto()),
            frame_id: n,
            keyframe: false,
            ..Default::default()
        })
    };
    assert!(matches!(
        receiver.receive(frame(delta(102), 2), 0),
        Err(NetError::KeyframeRequired)
    ));
    assert!(matches!(
        receiver.receive(frame(delta(103), 3), 0),
        Err(NetError::KeyframeRequired)
    ));
    assert_eq!(
        receiver.receive(frame(media(99), 4), 0).unwrap(),
        Receive::Discard
    );
    assert!(matches!(
        receiver.receive(frame(media(103), 5), 0).unwrap(),
        Receive::Deliver(_)
    ));
    assert!(matches!(
        receiver.receive(frame(delta(104), 6), 0).unwrap(),
        Receive::Deliver(_)
    ));
    assert_eq!(
        receiver.receive(frame(media(103), 7), 0).unwrap(),
        Receive::Discard
    );
}
#[test]
fn long_session_receive_retires_identities_and_preserves_channel_floors() {
    let (_, mut receiver) = sessions(99);
    assert!(matches!(
        receiver.receive(frame(media(1), 1), 0).unwrap(),
        Receive::Deliver(_)
    ));
    let gesture = |n| {
        Body::GestureUpdate(Box::new(pb::GestureUpdate {
            gesture_id: Some(id(n).to_proto()),
            ..Default::default()
        }))
    };
    for seq in 1000..11_000 {
        assert!(matches!(
            receiver.receive(frame(gesture(seq), seq), 0).unwrap(),
            Receive::Deliver(_)
        ));
    }
    assert_eq!(
        receiver.receive(frame(gesture(1000), 1000), 0).unwrap(),
        Receive::Discard
    );
    let mut delta = media(2);
    if let Body::FrameTiles(value) = &mut delta {
        value.keyframe = false;
    }
    assert!(matches!(
        receiver.receive(frame(delta, 2), 0),
        Err(NetError::KeyframeRequired)
    ));
    assert!(matches!(
        receiver.receive(frame(media(2), 3), 0).unwrap(),
        Receive::Deliver(_)
    ));
    assert_eq!(
        receiver.receive(frame(media(1), 1), 0).unwrap(),
        Receive::Discard
    );
}
#[test]
fn sender_retires_consumed_media_histories_and_requires_independent_restart() {
    let (mut sender, _) = sessions(99);
    for identity in 1..=1000 {
        sender
            .enqueue(Body::FrameTiles(pb::FrameTiles {
                capture_session_id: Some(id(identity).to_proto()),
                frame_id: 1,
                keyframe: true,
                ..Default::default()
            }))
            .unwrap();
        assert!(sender.next_frame().unwrap().is_some());
    }
    let restarted = |keyframe| {
        Body::FrameTiles(pb::FrameTiles {
            capture_session_id: Some(id(1).to_proto()),
            frame_id: 2,
            keyframe,
            ..Default::default()
        })
    };
    assert!(matches!(
        sender.enqueue(restarted(false)),
        Err(NetError::KeyframeRequired)
    ));
    assert!(sender.next_frame().unwrap().is_none());
    sender.enqueue(restarted(true)).unwrap();
    assert!(sender.next_frame().unwrap().is_some());
}
#[test]
fn blocked_carrier_lanes_leave_latest_media_replaceable_and_control_available() {
    let (mut session, _) = sessions(99);
    session.enqueue(media(1)).unwrap();
    assert!(
        session
            .next_frame_where(|channel| channel != pb::Channel::Media)
            .unwrap()
            .is_none()
    );
    session.enqueue(media(2)).unwrap();
    session
        .enqueue(Body::Ping(pb::Ping {
            nonce: 71,
            t_sent_ns: 1,
        }))
        .unwrap();
    assert!(
        matches!(session.next_frame_where(|channel|channel!=pb::Channel::Media).unwrap().unwrap().envelope.body,Some(Body::Ping(v))if v.nonce==71)
    );
    assert!(
        matches!(session.next_frame().unwrap().unwrap().envelope.body,Some(Body::FrameTiles(v))if v.frame_id==2)
    );
}
#[test]
fn input_requires_fresh_grant_geometry_sequence_and_activity() {
    let mut guard = InputGuard::default();
    let mut event = pb::InputEvent {
        input_session_id: Some(id(1).to_proto()),
        capture_session_id: Some(id(2).to_proto()),
        geometry_revision: 3,
        input_seq: 1,
        remote_scope: None,
        request_nonce: 0,
        event: Some(pb::input_event::Event::Mouse(pb::MouseEvent::default())),
    };
    assert!(!guard.accept(&event, 0).unwrap());
    guard.grant(id(1), id(2), 3, 0).unwrap();
    assert!(guard.accept(&event, 1).unwrap());
    assert!(!guard.accept(&event, 2).unwrap());
    event.input_seq = 2;
    event.geometry_revision = 4;
    assert!(!guard.accept(&event, 3).unwrap());
    event.geometry_revision = 3;
    assert!(!guard.accept(&event, 600001).unwrap());
    assert!(guard.grant(id(1), id(2), 3, 600002).is_err());
}
#[test]
fn reconnect_discards_volatile_queues_and_rejects_old_connection() {
    let (mut old, mut remote) = sessions(90);
    old.enqueue(media(1)).unwrap();
    let packet = old.next_frame().unwrap().unwrap();
    old.enqueue(media(2)).unwrap();
    old.disconnect();
    remote.disconnect();
    assert!(old.next_frame().unwrap().is_none());
    let (_, mut new) = sessions(91);
    assert_eq!(new.receive(packet, 0).unwrap(), Receive::Discard);
    let mut state = ConnectionMachine::default();
    state.transition(ConnectionEvent::Discover).unwrap();
    state.transition(ConnectionEvent::Handshake).unwrap();
    state
        .transition(ConnectionEvent::Connected(pb::Carrier::TcpAdb))
        .unwrap();
    assert!(
        state
            .transition(ConnectionEvent::Lost { now_ms: 10 })
            .unwrap()
    );
    state
        .transition(ConnectionEvent::Tick {
            now_ms: 2010,
            pending: 7,
        })
        .unwrap();
    assert_eq!(state.state(), &ConnectionState::Offline { pending: 7 });
}
#[test]
fn blobs_window_duplicates_resume_and_hash_integrity() {
    let data = vec![0x7a; 700_000];
    let asset = AssetId::hash(&data);
    let mut sender = BlobSender::resume(
        Cursor::new(data.clone()),
        asset.clone(),
        data.len() as u64,
        0,
    )
    .unwrap();
    let mut receiver = BlobReceiver::resume(
        Cursor::new(Vec::new()),
        asset.clone(),
        data.len() as u64,
        0,
        1_000_000,
    )
    .unwrap();
    let mut chunks = Vec::new();
    while let Some(chunk) = sender.next_chunk().unwrap() {
        chunks.push(chunk);
    }
    assert_eq!(chunks.len(), 4);
    for chunk in &chunks {
        let ack = receiver.receive(chunk).unwrap();
        receiver.receive(chunk).unwrap();
        sender.acknowledge(&ack).unwrap();
    }
    let offset = receiver.offset();
    let prefix = data[..offset as usize].to_vec();
    let mut receiver = BlobReceiver::resume(
        Cursor::new(prefix),
        asset.clone(),
        data.len() as u64,
        offset,
        1_000_000,
    )
    .unwrap();
    let mut sender =
        BlobSender::resume(Cursor::new(data.clone()), asset, data.len() as u64, offset).unwrap();
    while let Some(chunk) = sender.next_chunk().unwrap() {
        let ack = receiver.receive(&chunk).unwrap();
        sender.acknowledge(&ack).unwrap();
    }
    assert!(sender.is_verified());
    assert_eq!(receiver.finish().unwrap().into_inner(), data);
}
#[test]
fn corrupted_blob_and_trailing_resume_bytes_are_rejected() {
    let asset = AssetId::hash(b"abc");
    assert!(BlobReceiver::resume(Cursor::new(b"ab".to_vec()), asset.clone(), 3, 1, 10).is_err());
    let mut receiver =
        BlobReceiver::resume(Cursor::new(Vec::new()), asset.clone(), 3, 0, 10).unwrap();
    let chunk = pb::BlobChunk {
        asset_id: asset.to_string(),
        offset: 0,
        data: b"abd".to_vec(),
        total_size: 3,
        last: true,
    };
    assert!(matches!(receiver.receive(&chunk), Err(NetError::Integrity)));
}
#[test]
fn cumulative_blob_acks_allow_reordering_without_rewinding_or_unverifying() {
    let data = vec![0x91; 300_000];
    let asset = AssetId::hash(&data);
    let mut sender = BlobSender::resume(
        Cursor::new(data.clone()),
        asset.clone(),
        data.len() as u64,
        0,
    )
    .unwrap();
    let mut receiver = BlobReceiver::resume(
        Cursor::new(Vec::new()),
        asset.clone(),
        data.len() as u64,
        0,
        400_000,
    )
    .unwrap();
    let mut acknowledgements = Vec::new();
    while let Some(chunk) = sender.next_chunk().unwrap() {
        acknowledgements.push(receiver.receive(&chunk).unwrap());
    }
    assert_eq!(acknowledgements.len(), 4);
    for ack in acknowledgements.iter().rev() {
        sender.acknowledge(ack).unwrap();
    }
    let final_chunk = sender.next_chunk().unwrap().unwrap();
    let final_ack = receiver.receive(&final_chunk).unwrap();
    assert!(final_ack.verified);
    sender.acknowledge(&final_ack).unwrap();
    sender.acknowledge(&acknowledgements[0]).unwrap();
    sender.acknowledge(&final_ack).unwrap();
    assert!(sender.is_verified());
    assert!(sender.next_chunk().unwrap().is_none());
    let mut wrong = acknowledgements[0].clone();
    wrong.asset_id = AssetId::hash(b"different").to_string();
    assert!(sender.acknowledge(&wrong).is_err());
    let mut wrong = acknowledgements[0].clone();
    wrong.verified = true;
    assert!(sender.acknowledge(&wrong).is_err());
    let mut wrong = final_ack.clone();
    wrong.next_offset += 1;
    assert!(sender.acknowledge(&wrong).is_err());
    assert!(sender.is_verified());
    assert_eq!(receiver.finish().unwrap().into_inner(), data);
}
#[test]
fn sync_exact_ack_retry_and_tamper_are_atomic() {
    let initial = initial();
    let mut receive = SyncReceiver::new(initial.clone());
    let mut host = HostSync::new(initial).unwrap();
    let transaction = transaction(host.host(), 1);
    let first = host.submit(transaction.clone(), &device(2), 1002).unwrap();
    let retry = host.submit(transaction, &device(2), 9999).unwrap();
    assert!(retry.duplicate);
    assert_eq!(first.ack, retry.ack);
    let response = host.respond(&receive.request().unwrap()).unwrap();
    let before = receive.host().revision().unwrap();
    let mut bad = response.batch.clone();
    bad.txns[0].ack.as_mut().unwrap().state_hash[0] ^= 1;
    assert!(receive.receive(bad, &device(1)).is_err());
    assert_eq!(receive.host().revision().unwrap(), before);
    let snapshot = receive
        .receive(response.batch, &device(1))
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.revision, host.host().revision().unwrap());
}
#[test]
fn checkpoint_manifest_is_peer_project_and_digest_bound() {
    let initial = initial();
    let mut receive = SyncReceiver::new(initial.clone());
    let mut host = HostSync::new(initial).unwrap();
    host.submit(transaction(host.host(), 1), &device(2), 1002)
        .unwrap();
    let mut request = receive.request().unwrap();
    request.state_hash[0] ^= 1;
    let response = host.respond(&request).unwrap();
    let mut bytes = response.checkpoint.unwrap();
    assert!(receive.receive(response.batch.clone(), &device(3)).is_err());
    assert!(
        receive
            .receive(response.batch, &device(1))
            .unwrap()
            .is_none()
    );
    bytes[0] ^= 1;
    assert!(receive.install_checkpoint(&bytes, &device(1)).is_err());
    bytes[0] ^= 1;
    assert_eq!(
        receive
            .install_checkpoint(&bytes, &device(1))
            .unwrap()
            .revision,
        host.host().revision().unwrap()
    );
    assert!(receive.install_checkpoint(&bytes, &device(1)).is_err());
}
#[test]
fn persisted_outbox_survives_disconnect_and_host_ack_loss() {
    let directory = temporary();
    let project = initial().project().clone();
    let path = directory.path().join("client");
    let mut client =
        vw_store::ProjectStore::create(&path, project.clone(), device(1), 1000).unwrap();
    let mut host_store =
        vw_store::ProjectStore::create(&directory.path().join("host"), project, device(1), 1000)
            .unwrap();
    let mut host = HostSync::from_store(&host_store).unwrap();
    let transaction = transaction(host.host(), 1);
    let (mut session, _) = sessions(99);
    session.disconnect();
    assert!(
        session
            .enqueue_transaction(&mut client, transaction.clone())
            .is_err()
    );
    drop(client);
    let mut client = vw_store::ProjectStore::open(&path).unwrap();
    assert_eq!(client.pending().unwrap(), vec![transaction.clone()]);
    let accepted = host
        .commit_persisted(&mut host_store, transaction.clone(), &device(2), 1002)
        .unwrap();
    assert!(!accepted.duplicate);
    let (mut session, _) = sessions(100);
    session.resend_pending(&client).unwrap();
    assert!(matches!(
        session.next_frame().unwrap().unwrap().envelope.body,
        Some(Body::Txn(_))
    ));
    let retry = host
        .commit_persisted(&mut host_store, transaction, &device(2), 1003)
        .unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.ack, accepted.ack);
    let mut receive = SyncReceiver::new(initial());
    let response = host.respond(&receive.request().unwrap()).unwrap();
    receive.receive(response.batch, &device(1)).unwrap();
    assert_eq!(receive.acknowledge_pending(&mut client).unwrap(), 1);
    assert!(client.pending().unwrap().is_empty());
    drop(client);
    let client = vw_store::ProjectStore::open(&path).unwrap();
    assert_eq!(client.project(), host_store.project());
    assert_eq!(client.revision().unwrap(), host_store.revision().unwrap());
    assert!(client.pending().unwrap().is_empty());
    drop(client);
    drop(host_store);
    directory.close().unwrap();
}
#[test]
fn reconnect_resends_only_pending_transactions_owned_by_the_authenticated_device() {
    let directory = temporary();
    let mut client = vw_store::ProjectStore::create(
        &directory.path().join("client"),
        initial().project().clone(),
        device(1),
        0,
    )
    .unwrap();
    let own = transaction(&initial(), 1);
    let mut foreign = transaction(&initial(), 2);
    foreign.device_id = device(3).to_string();
    foreign.ops[0].op_id.as_mut().unwrap().device_id = device(3).to_string();
    client.enqueue_pending(&foreign).unwrap();
    client.enqueue_pending(&own).unwrap();
    let (mut session, _) = sessions(99);
    session.resend_pending(&client).unwrap();
    assert!(
        matches!(session.next_frame().unwrap().unwrap().envelope.body,Some(Body::Txn(v))if v==own)
    );
    assert!(session.next_frame().unwrap().is_none());
    assert_eq!(client.pending().unwrap(), vec![foreign, own]);
    drop(client);
    directory.close().unwrap();
}
#[test]
fn sync_checkpoint_refuses_forgotten_cancellation_and_changed_receipt_history() {
    let mut baseline = initial();
    baseline.cancel_gesture(device(2), id(77)).unwrap();
    let mut receive = SyncReceiver::new(baseline);
    let remote = HostSync::new(initial()).unwrap();
    let mut request = receive.request().unwrap();
    request.state_hash[0] ^= 1;
    let response = remote.respond(&request).unwrap();
    receive.receive(response.batch, &device(1)).unwrap();
    assert!(
        receive
            .install_checkpoint(&response.checkpoint.unwrap(), &device(1))
            .is_err()
    );
    let mut local = initial();
    let txn = transaction(&local, 1);
    local.submit(txn.clone(), &device(2), 1002).unwrap();
    let mut receive = SyncReceiver::new(local.clone());
    let mut alternate = initial();
    alternate.submit(txn, &device(2), 1003).unwrap();
    let alternate = HostSync::new(alternate).unwrap();
    let mut request = receive.request().unwrap();
    request.state_hash[0] ^= 1;
    let response = alternate.respond(&request).unwrap();
    receive.receive(response.batch, &device(1)).unwrap();
    assert!(
        receive
            .install_checkpoint(&response.checkpoint.unwrap(), &device(1))
            .is_err()
    );
    assert_eq!(
        receive.host().checkpoint_bytes().unwrap(),
        local.checkpoint_bytes().unwrap()
    );
}
/// A project with one layer, an object the phone made, a move of it by the
/// host, and the phone's undo of its object, which the host answers with a
/// notice: the move by the other device is in the way.
fn history_with_a_notice() -> (HostSequencer, HostSync, Id) {
    use vw_model::{Document, Layer};
    let mut project = Project::new(id(1), "Notice fixture".into(), device(1));
    project.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(2).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Fixture".into(),
                ..Default::default()
            },
            pages: Vec::new(),
            created_at_ms: 0,
        },
    );
    project.layers.insert(
        id(3),
        Layer {
            definition: pb::CreateLayer {
                layer_id: Some(id(3).to_proto()),
                document_id: Some(id(2).to_proto()),
                page_index: -1,
                name: "Layer".into(),
                kind: "annotation".into(),
                order_key: "V".into(),
            },
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: "normal".into(),
        },
    );
    let initial = HostSequencer::new(project, device(1)).unwrap();
    let mut host = HostSync::new(initial.clone()).unwrap();
    let transform = |e: f64| pb::Affine {
        a: 1.0,
        d: 1.0,
        e,
        ..Default::default()
    };
    let by = |author: u8, n: u64, kind: Kind, host: &HostSync| pb::Transaction {
        txn_id: Some(id(200 + n).to_proto()),
        project_id: Some(id(1).to_proto()),
        device_id: device(author).to_string(),
        base_revision: Some(host.host().revision().unwrap()),
        created_at_wall_ms: 1000 + n as i64,
        gesture_id: None,
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: device(author).to_string(),
                lamport: n,
            }),
            kind: Some(kind),
        }],
    };
    let created = by(
        2,
        1,
        Kind::CreateObject(pb::CreateObject {
            document_id: Some(id(2).to_proto()),
            state: Some(pb::ObjectState {
                object_id: Some(id(4).to_proto()),
                layer_id: Some(id(3).to_proto()),
                order_key: "V".into(),
                transform: Some(transform(0.0)),
                style: Some(pb::Style {
                    stroke: Some(pb::Color { rgba: 0x112233ff }),
                    width: 1.0,
                    ..Default::default()
                }),
                role: pb::Role::None as i32,
                created_by: device(2).to_string(),
                created_at_ms: 1,
                shape: Some(pb::object_state::Shape::Rect(pb::RectD {
                    x: 0.0,
                    y: 0.0,
                    w: 4.0,
                    h: 3.0,
                })),
                ..Default::default()
            }),
        }),
        &host,
    );
    host.submit(created, &device(2), 2001).unwrap();
    let moved = by(
        1,
        2,
        Kind::SetProperty(pb::SetProperty {
            object_id: Some(id(4).to_proto()),
            property: "transform".into(),
            value: Some(pb::PropertyValue {
                value: Some(pb::property_value::Value::Transform(transform(5.0))),
            }),
        }),
        &host,
    );
    host.submit(moved, &device(1), 2002).unwrap();
    let undo = by(
        2,
        3,
        Kind::UndoTransaction(pb::UndoTransaction {
            target_txn_id: Some(id(201).to_proto()),
        }),
        &host,
    );
    let answered = host.submit(undo, &device(2), 2003).unwrap();
    assert_eq!(answered.ack.notices.len(), 1, "{:?}", answered.ack.notices);
    (initial, host, id(203))
}

/// Two builds word a notice differently, or one gives a notice where the
/// other gives none. What the host decided is the revision and the state
/// hash; the device that receives the transaction decides those again and
/// keeps the host's notices as they came.
#[test]
fn sync_does_not_depend_on_how_the_other_build_words_its_notices() {
    let (initial, host, undo) = history_with_a_notice();
    let response = host
        .respond(&SyncReceiver::new(initial.clone()).request().unwrap())
        .unwrap();
    assert_eq!(response.batch.txns.len(), 3);
    let own = response.batch.txns[2].ack.as_ref().unwrap().notices.clone();
    // As this build words it, the batch is accepted as it always was.
    let mut same = SyncReceiver::new(initial.clone());
    same.receive(response.batch.clone(), &device(1))
        .unwrap()
        .unwrap();

    type Reword = fn(&mut pb::SyncBatch);
    let rewordings: [(&str, Reword); 4] = [
        ("another wording", |batch| {
            batch.txns[2].ack.as_mut().unwrap().notices =
                vec!["Left the object where the other device put it".into()];
        }),
        ("no notice where this build gives one", |batch| {
            batch.txns[2].ack.as_mut().unwrap().notices.clear();
        }),
        ("two notices where this build gives one", |batch| {
            batch.txns[2].ack.as_mut().unwrap().notices = vec!["First".into(), "Second".into()];
        }),
        ("a notice where this build gives none", |batch| {
            batch.txns[0].ack.as_mut().unwrap().notices = vec!["Welcome".into()];
        }),
    ];
    for (name, reword) in rewordings {
        let mut batch = response.batch.clone();
        reword(&mut batch);
        let mut receive = SyncReceiver::new(initial.clone());
        let snapshot = receive
            .receive(batch.clone(), &device(1))
            .unwrap_or_else(|error| panic!("{name}: {error}"))
            .unwrap();
        // The same project at the same revision as the host.
        assert_eq!(snapshot.revision, host.host().revision().unwrap(), "{name}");
        assert_eq!(snapshot.project, *host.host().project(), "{name}");
        // The host's acknowledgements, word for word: in what the replica is
        // given, in what the mirror answers a retry with and in what it stores.
        for entry in &batch.txns {
            let txn = entry.txn.as_ref().unwrap();
            let key = Id::from_proto(txn.txn_id.as_ref()).unwrap();
            let sent = entry.ack.as_ref().unwrap();
            assert_eq!(&snapshot.accepted[&key].1, sent, "{name}");
            assert_eq!(
                receive.host().accepted_transaction(&key).unwrap().1,
                sent,
                "{name}"
            );
        }
        let stored =
            HostSequencer::from_checkpoint_bytes(&receive.host().checkpoint_bytes().unwrap())
                .unwrap();
        assert_eq!(
            stored.accepted_transaction(&undo).unwrap().1,
            batch.txns[2].ack.as_ref().unwrap(),
            "{name}"
        );
        // A second device that gets the same batch agrees with the first.
        let mut other = SyncReceiver::new(initial.clone());
        other.receive(batch, &device(1)).unwrap().unwrap();
        assert!(
            other.host().checkpoint_bytes().unwrap() == receive.host().checkpoint_bytes().unwrap(),
            "{name}"
        );
    }

    // What matters for integrity is still compared, whatever the wording.
    type Tamper = fn(&mut pb::SyncBatch);
    let tampered: [(&str, Tamper); 10] = [
        ("state hash in the acknowledgement", |batch| {
            batch.txns[2].ack.as_mut().unwrap().state_hash[0] ^= 1;
        }),
        ("state hash of the entry", |batch| {
            batch.txns[2].state_hash[0] ^= 1
        }),
        ("both state hashes", |batch| {
            batch.txns[2].state_hash[0] ^= 1;
            batch.txns[2].ack.as_mut().unwrap().state_hash[0] ^= 1;
        }),
        ("revision in the acknowledgement", |batch| {
            batch.txns[2].ack.as_mut().unwrap().host_seq += 1;
        }),
        ("transaction named by the acknowledgement", |batch| {
            batch.txns[2].ack.as_mut().unwrap().txn_id = Some(id(999).to_proto());
        }),
        ("no acknowledgement", |batch| batch.txns[2].ack = None),
        ("a notice no acknowledgement may hold", |batch| {
            batch.txns[2].ack.as_mut().unwrap().notices = vec!["x".repeat(1025)];
        }),
        ("an empty notice", |batch| {
            batch.txns[2].ack.as_mut().unwrap().notices = vec![String::new()];
        }),
        ("a notice with a control character", |batch| {
            batch.txns[2].ack.as_mut().unwrap().notices = vec!["Skipped\u{1b}[2J".into()];
        }),
        ("more notices than are kept", |batch| {
            batch.txns[2].ack.as_mut().unwrap().notices = vec!["Skipped".into(); 257];
        }),
    ];
    for (name, tamper) in tampered {
        let mut batch = response.batch.clone();
        batch.txns[2].ack.as_mut().unwrap().notices = vec!["Another wording".into()];
        tamper(&mut batch);
        let mut receive = SyncReceiver::new(initial.clone());
        let before = receive.host().checkpoint_bytes().unwrap();
        assert!(receive.receive(batch, &device(1)).is_err(), "{name}");
        assert!(
            receive.host().checkpoint_bytes().unwrap() == before,
            "{name}"
        );
    }
    assert_eq!(own.len(), 1);
}
fn tls_pair() -> (carrier::PinnedTls, carrier::PinnedTls) {
    let a = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let b = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let a_identity = carrier::Identity {
        certificate: a.cert.der().clone(),
        key: rustls::pki_types::PrivatePkcs8KeyDer::from(a.signing_key.serialize_der()).into(),
    };
    let b_identity = carrier::Identity {
        certificate: b.cert.der().clone(),
        key: rustls::pki_types::PrivatePkcs8KeyDer::from(b.signing_key.serialize_der()).into(),
    };
    (
        carrier::PinnedTls::new(
            a_identity,
            carrier::TrustedPeer {
                device: device(2),
                certificate: b.cert.der().clone(),
                server_name: "localhost".into(),
            },
        )
        .unwrap(),
        carrier::PinnedTls::new(
            b_identity,
            carrier::TrustedPeer {
                device: device(1),
                certificate: a.cert.der().clone(),
                server_name: "localhost".into(),
            },
        )
        .unwrap(),
    )
}
#[tokio::test(flavor = "current_thread")]
async fn mutually_pinned_tls_tcp_round_trip() {
    let (server_tls, client_tls) = tls_pair();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (server, client) = tokio::join!(
        carrier::TcpCarrier::accept(&listener, &server_tls),
        carrier::TcpCarrier::connect(address, &client_tls)
    );
    let mut server = server.unwrap();
    let mut client = client.unwrap();
    assert_eq!(server.peer().device(), &device(2));
    assert_eq!(client.peer().device(), &device(1));
    assert_eq!(
        server.peer().channel_binding(),
        client.peer().channel_binding()
    );
    let expected = frame(
        Body::Ping(pb::Ping {
            nonce: 4,
            t_sent_ns: 10,
        }),
        1,
    );
    let (sent, received) = tokio::join!(client.send(&expected), server.receive());
    sent.unwrap();
    assert_eq!(received.unwrap(), expected);
}
#[tokio::test(flavor = "current_thread")]
async fn complete_secure_connection_handshake_then_control() {
    let (server_tls, client_tls) = tls_pair();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (server, client) = tokio::join!(
        carrier::TcpCarrier::accept(&listener, &server_tls),
        carrier::TcpCarrier::connect(address, &client_tls)
    );
    let server_local = local(1);
    let client_local = local(2);
    let (server, client) = tokio::join!(
        SecureConnection::server(CarrierIo::Tcp(server.unwrap()), &server_local),
        SecureConnection::client(CarrierIo::Tcp(client.unwrap()), &client_local)
    );
    let mut server = server.unwrap();
    let mut client = client.unwrap();
    client
        .session_mut()
        .enqueue(Body::Ping(pb::Ping {
            nonce: 81,
            t_sent_ns: 123,
        }))
        .unwrap();
    let (sent, received) = tokio::join!(client.send_next(), server.receive(0));
    assert!(sent.unwrap());
    assert!(matches!(received.unwrap(), Receive::Deliver(Body::Ping(value)) if value.nonce == 81));
}
#[tokio::test(flavor = "current_thread")]
async fn tls_rejects_untrusted_certificates_before_application_handshake() {
    let (server_tls, _) = tls_pair();
    let (_, wrong_client_tls) = tls_pair();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (server, client) = tokio::join!(
        carrier::TcpCarrier::accept(&listener, &server_tls),
        carrier::TcpCarrier::connect(address, &wrong_client_tls)
    );
    assert!(server.is_err());
    assert!(client.is_err());
}
#[tokio::test(flavor = "current_thread")]
async fn quic_channels_datagrams_and_clean_blob_fin() {
    let (server_tls, client_tls) = tls_pair();
    let listener = carrier::QuicListener::bind("127.0.0.1:0".parse().unwrap(), server_tls).unwrap();
    let address = listener.local_addr().unwrap();
    let (server, client) = tokio::join!(
        listener.accept(),
        carrier::QuicCarrier::connect("127.0.0.1:0".parse().unwrap(), address, &client_tls)
    );
    let mut server = server.unwrap();
    let mut client = client.unwrap();
    assert_eq!(
        server.peer().channel_binding(),
        client.peer().channel_binding()
    );
    for body in [
        Body::Ping(pb::Ping::default()),
        Body::SyncRequest(pb::SyncRequest::default()),
        Body::InputStatus(pb::InputStatus::default()),
        media(1),
        Body::CursorUpdate(pb::CursorUpdate {
            document_id: Some(id(1).to_proto()),
            ..Default::default()
        }),
        Body::BlobAck(pb::BlobAck::default()),
        Body::BlobAck(pb::BlobAck::default()),
    ] {
        let expected = frame(body, 1);
        let (sent, received) = tokio::join!(client.send(&expected), server.receive());
        sent.unwrap();
        assert_eq!(received.unwrap(), expected);
    }
}
