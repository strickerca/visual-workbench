//! Actual mutual-TLS/QUIC sockets, with a deliberately blocked MEDIA lane.
#![allow(clippy::unwrap_used, clippy::panic)]
use super::*;
use crate::{CarrierIo, LocalHello, Receive, SecureConnection};
use std::collections::BTreeSet;
use vw_model::Id;

fn device(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 16])
}
fn id(n: u64) -> Id {
    Id::from_parts(1_700_000_000_000 + n, [4; 10]).unwrap()
}
fn local(n: u8) -> LocalHello {
    LocalHello {
        device: device(n),
        platform: v1::Platform::Windows,
        app_version: "fixture".into(),
        label: "synthetic".into(),
        capabilities: BTreeSet::new(),
    }
}
fn tls_pair() -> (PinnedTls, PinnedTls) {
    let a = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let b = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let identity = |value: &rcgen::CertifiedKey<rcgen::KeyPair>| Identity {
        certificate: value.cert.der().clone(),
        key: rustls::pki_types::PrivatePkcs8KeyDer::from(value.signing_key.serialize_der()).into(),
    };
    (
        PinnedTls::new(
            identity(&a),
            TrustedPeer {
                device: device(2),
                certificate: b.cert.der().clone(),
                server_name: "localhost".into(),
            },
        )
        .unwrap(),
        PinnedTls::new(
            identity(&b),
            TrustedPeer {
                device: device(1),
                certificate: a.cert.der().clone(),
                server_name: "localhost".into(),
            },
        )
        .unwrap(),
    )
}
fn media(n: u64) -> Body {
    Body::FrameTiles(v1::FrameTiles {
        capture_session_id: Some(id(4).to_proto()),
        frame_id: n,
        keyframe: true,
        ..Default::default()
    })
}
async fn quick<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(3), future)
        .await
        .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn duplex_accepts_more_than_256_consumed_gesture_identities() {
    let (server_tls, client_tls) = tls_pair();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (server, client) = tokio::join!(
        TcpCarrier::accept(&listener, &server_tls),
        TcpCarrier::connect(listener.local_addr().unwrap(), &client_tls)
    );
    let a = local(1);
    let b = local(2);
    let (server, client) = tokio::join!(
        SecureConnection::server(CarrierIo::Tcp(server.unwrap()), &a),
        SecureConnection::client(CarrierIo::Tcp(client.unwrap()), &b)
    );
    let (server_send, mut server_receive) = server.unwrap().into_duplex().unwrap();
    let (client_send, _client_receive) = client.unwrap().into_duplex().unwrap();
    for n in 1..=600 {
        client_send
            .enqueue(Body::GestureUpdate(Box::new(v1::GestureUpdate {
                gesture_id: Some(id(n).to_proto()),
                ..Default::default()
            })))
            .unwrap();
        assert!(matches!(quick(server_receive.receive(0)).await.unwrap(),
            Receive::Deliver(Body::GestureUpdate(value)) if value.gesture_id == Some(id(n).to_proto())));
    }
    client_send.close();
    server_send.close();
}

#[tokio::test(flavor = "current_thread")]
async fn blocked_quic_media_preserves_bidirectional_control_input_and_ops() {
    let (server_tls, client_tls) = tls_pair();
    let listener = QuicListener::bind("127.0.0.1:0".parse().unwrap(), server_tls).unwrap();
    let (server, client) = tokio::join!(
        listener.accept(),
        QuicCarrier::connect(
            "127.0.0.1:0".parse().unwrap(),
            listener.local_addr().unwrap(),
            &client_tls
        )
    );
    let client = client.unwrap();
    let blocked_lane = client.send.clone();
    let a = local(1);
    let b = local(2);
    let (server, client) = tokio::join!(
        SecureConnection::server(CarrierIo::Quic(server.unwrap()), &a),
        SecureConnection::client(CarrierIo::Quic(client), &b)
    );
    let SendIo::Quic { streams, .. } = &*blocked_lane.io else {
        panic!("QUIC fixture");
    };
    let media_guard = streams
        .get(&(v1::Channel::Media as i32))
        .unwrap()
        .lock()
        .await;
    let (server_send, mut server_receive) = server.unwrap().into_duplex().unwrap();
    let (client_send, mut client_receive) = client.unwrap().into_duplex().unwrap();
    client_send.enqueue(media(1)).unwrap();
    tokio::task::yield_now().await;
    for n in 2..=10_000 {
        client_send.enqueue(media(n)).unwrap();
    }
    client_send
        .enqueue(Body::InputStatus(v1::InputStatus::default()))
        .unwrap();
    client_send
        .enqueue(Body::SyncRequest(v1::SyncRequest::default()))
        .unwrap();
    client_send
        .enqueue(Body::Ping(v1::Ping {
            nonce: 42,
            ..Default::default()
        }))
        .unwrap();
    let mut received = BTreeSet::new();
    for _ in 0..3 {
        let Receive::Deliver(body) = quick(server_receive.receive(0)).await.unwrap() else {
            panic!("delivery");
        };
        received.insert(crate::channel_for(&body) as i32);
    }
    assert_eq!(
        received,
        [v1::Channel::Control, v1::Channel::Input, v1::Channel::Ops]
            .map(|v| v as i32)
            .into_iter()
            .collect()
    );
    server_send
        .enqueue(Body::Pong(v1::Pong::default()))
        .unwrap();
    assert!(matches!(
        quick(client_receive.receive(0)).await.unwrap(),
        Receive::Deliver(Body::Pong(_))
    ));
    drop(media_guard);
    // One MEDIA may already be in flight; only the newest remains queued.
    let mut latest = 0;
    for _ in 0..2 {
        if let Receive::Deliver(Body::FrameTiles(frame)) =
            quick(server_receive.receive(0)).await.unwrap()
        {
            latest = frame.frame_id;
            if latest == 10_000 {
                break;
            }
        }
    }
    assert_eq!(latest, 10_000);
    client_send.close();
    server_send.close();
}

#[tokio::test(flavor = "current_thread")]
async fn blocked_tcp_write_does_not_block_opposite_direction_receive() {
    let (server_tls, client_tls) = tls_pair();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (server, client) = tokio::join!(
        TcpCarrier::accept(&listener, &server_tls),
        TcpCarrier::connect(listener.local_addr().unwrap(), &client_tls)
    );
    let client = client.unwrap();
    let blocked_lane = client.send.clone();
    let a = local(1);
    let b = local(2);
    let (server, client) = tokio::join!(
        SecureConnection::server(CarrierIo::Tcp(server.unwrap()), &a),
        SecureConnection::client(CarrierIo::Tcp(client), &b)
    );
    let SendIo::Tcp(write) = &*blocked_lane.io else {
        panic!("TCP fixture");
    };
    let guard = write.lock().await;
    let (server_send, mut server_receive) = server.unwrap().into_duplex().unwrap();
    let (client_send, mut client_receive) = client.unwrap().into_duplex().unwrap();
    client_send.enqueue(media(1)).unwrap();
    tokio::task::yield_now().await;
    server_send
        .enqueue(Body::Ping(v1::Ping::default()))
        .unwrap();
    assert!(matches!(
        quick(client_receive.receive(0)).await.unwrap(),
        Receive::Deliver(Body::Ping(_))
    ));
    client_send
        .enqueue(Body::Pong(v1::Pong::default()))
        .unwrap();
    drop(guard);
    let mut got_control = false;
    let mut got_media = false;
    for _ in 0..2 {
        match quick(server_receive.receive(0)).await.unwrap() {
            Receive::Deliver(Body::Pong(_)) => got_control = true,
            Receive::Deliver(Body::FrameTiles(_)) => got_media = true,
            _ => panic!("unexpected body"),
        }
    }
    assert!(got_control && got_media);
    client_send.close();
    server_send.close();
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_duplex_receiver_revokes_sender_and_discards_input_queue() {
    let (server_tls, client_tls) = tls_pair();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (server, client) = tokio::join!(
        TcpCarrier::accept(&listener, &server_tls),
        TcpCarrier::connect(listener.local_addr().unwrap(), &client_tls)
    );
    let a = local(1);
    let b = local(2);
    let (server, client) = tokio::join!(
        SecureConnection::server(CarrierIo::Tcp(server.unwrap()), &a),
        SecureConnection::client(CarrierIo::Tcp(client.unwrap()), &b)
    );
    let (_server_send, mut server_receive) = server.unwrap().into_duplex().unwrap();
    let (client_send, client_receive) = client.unwrap().into_duplex().unwrap();
    drop(client_receive);
    assert!(
        client_send
            .enqueue(Body::Ping(v1::Ping::default()))
            .is_err()
    );
    assert!(quick(server_receive.receive(0)).await.is_err());
}
