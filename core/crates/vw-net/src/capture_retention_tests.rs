#![allow(clippy::unwrap_used)]
use crate::session::Session;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use vw_model::{DeviceId, Id};
use vw_proto::v1::{self as pb, envelope::Body};

struct Retained(Arc<AtomicUsize>);
impl Drop for Retained {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}
fn session() -> Session {
    Session::new(
        DeviceId::from_bytes([1; 16]),
        DeviceId::from_bytes([2; 16]),
        Id::from_parts(1_700_000_000_000, [3; 10]).unwrap(),
        ["capture_stream_v1".into()].into(),
    )
}
// Synthetic MEDIA exercises queue ownership, not codec or presentation truth.
fn media(delivery: u64) -> Body {
    Body::FrameTiles(pb::FrameTiles {
        capture_session_id: Some(
            Id::from_parts(1_700_000_000_000, [4; 10])
                .unwrap()
                .to_proto(),
        ),
        frame_id: delivery,
        delivery_sequence: delivery,
        source_generation: 1,
        keyframe: true,
        ..Default::default()
    })
}
#[test]
fn reservation_moves_from_queue_through_the_actual_write_owner() {
    let mut session = session();
    let drops = Arc::new(AtomicUsize::new(0));
    session
        .enqueue_retained(media(1), Box::new(Retained(drops.clone())))
        .unwrap();
    assert_eq!(drops.load(Ordering::Acquire), 0);
    let (frame, retained) = session
        .next_retained_frame_where(|_| true)
        .unwrap()
        .unwrap();
    assert_eq!(drops.load(Ordering::Acquire), 0);
    drop(frame);
    assert_eq!(drops.load(Ordering::Acquire), 0);
    drop(retained);
    assert_eq!(drops.load(Ordering::Acquire), 1);
}
#[test]
fn latest_replacement_and_disconnect_retire_exactly_their_queued_owners() {
    let mut session = session();
    let drops = Arc::new(AtomicUsize::new(0));
    session
        .enqueue_retained(media(1), Box::new(Retained(drops.clone())))
        .unwrap();
    session
        .enqueue_retained(media(2), Box::new(Retained(drops.clone())))
        .unwrap();
    assert_eq!(drops.load(Ordering::Acquire), 1);
    session.disconnect();
    assert_eq!(drops.load(Ordering::Acquire), 2);
    session.disconnect();
    assert_eq!(drops.load(Ordering::Acquire), 2);
}
#[test]
fn failed_admission_retires_its_owner_without_releasing_an_inflight_write() {
    let mut session = session();
    let drops = Arc::new(AtomicUsize::new(0));
    session
        .enqueue_retained(media(1), Box::new(Retained(drops.clone())))
        .unwrap();
    let (frame, retained) = session
        .next_retained_frame_where(|_| true)
        .unwrap()
        .unwrap();
    session.disconnect();
    assert!(
        session
            .enqueue_retained(media(2), Box::new(Retained(drops.clone())))
            .is_err()
    );
    assert_eq!(drops.load(Ordering::Acquire), 1);
    drop(frame);
    drop(retained);
    assert_eq!(drops.load(Ordering::Acquire), 2);
}

#[test]
fn capture_controls_and_scoped_blobs_require_the_negotiated_capability() {
    let mut plain = Session::new(
        DeviceId::from_bytes([1; 16]),
        DeviceId::from_bytes([2; 16]),
        Id::from_parts(1_700_000_000_000, [3; 10]).unwrap(),
        Default::default(),
    );
    assert!(
        plain
            .enqueue(Body::StreamControl(pb::StreamControl::default()))
            .is_err()
    );
    assert!(
        plain
            .enqueue(Body::StreamBlob(pb::StreamBlob::default()))
            .is_err()
    );
    assert!(plain.enqueue(media(1)).is_err());
    let mut scoped = session();
    let Body::FrameTiles(mut packet) = media(1) else {
        unreachable!()
    };
    packet.delivery_sequence = 0;
    assert!(scoped.enqueue(Body::FrameTiles(packet)).is_err());
}

#[test]
fn delivery_sequence_orders_a_repair_without_fabricating_a_new_capture() {
    let mut session = session();
    session.enqueue(media(1)).unwrap();
    drop(session.next_frame().unwrap());
    let Body::FrameTiles(mut repair) = media(2) else {
        unreachable!()
    };
    repair.frame_id = 1;
    session.enqueue(Body::FrameTiles(repair)).unwrap();
    let frame = session.next_frame().unwrap().unwrap();
    let Some(Body::FrameTiles(packet)) = frame.envelope.body else {
        unreachable!()
    };
    assert_eq!(packet.frame_id, 1);
    assert_eq!(packet.delivery_sequence, 2);
}

#[test]
fn media_preflight_refuses_repeated_amplification_duplicate_fields_and_groups() {
    let mut repeated = Vec::new();
    for _ in 0..129 {
        repeated.extend_from_slice(&[0x3a, 0]);
    }
    assert!(vw_proto::capture_admission::frame_tiles(&repeated).is_err());
    assert!(vw_proto::capture_admission::frame_tiles(&[0x10, 1, 0x10, 2]).is_err());
    assert!(vw_proto::capture_admission::frame_tiles(&[0x1b]).is_err());
}

#[test]
fn scoped_capture_ack_preempts_queued_bulk_blob_data() {
    let mut session = session();
    session
        .enqueue(Body::BlobChunk(pb::BlobChunk {
            asset_id: "a".repeat(64),
            offset: 0,
            total_size: 1,
            data: vec![1],
            last: true,
            ..Default::default()
        }))
        .unwrap();
    session
        .enqueue(Body::StreamBlob(pb::StreamBlob {
            key: None,
            body: Some(pb::stream_blob::Body::Ack(pb::BlobAck {
                asset_id: "b".repeat(64),
                next_offset: 0,
                verified: false,
            })),
        }))
        .unwrap();
    let first = session.next_frame().unwrap().unwrap();
    assert!(matches!(first.envelope.body, Some(Body::StreamBlob(_))));
}
