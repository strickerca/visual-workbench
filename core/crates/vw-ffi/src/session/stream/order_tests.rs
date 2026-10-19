#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::order::{Arrival, Delivery, MediaQueue, Source};

#[test]
fn source_label_is_bounded_display_data_and_cannot_relabel_an_existing_generation() {
    let mut control = source(1).control("start");
    control.source_name = "x".repeat(257);
    assert!(Source::parse(&control).is_err());
    control.source_name = "source\ncontrol".into();
    assert!(Source::parse(&control).is_err());
    let mut queue = queue();
    control = source(1).control("start");
    control.source_name = "Different chosen label".into();
    assert!(queue.open(Source::parse(&control).unwrap()).is_err());
}
use super::transfer::{Receiver, Sender};
use super::wire::{OriginalOffer, Scope};
use std::io::Cursor;
use vw_model::{AssetId, Id};
use vw_proto::v1 as pb;
use vw_stream::{FrameIdentity, StreamGeometry};

fn id(n: u64) -> Id {
    Id::from_parts(1_700_000_000_000 + n, [0x63; 10]).unwrap()
}
fn identity(n: u64) -> FrameIdentity {
    let wire = pb::CaptureGeometry {
        source_kind: "window".into(),
        window_handle: 7,
        monitor_id: String::new(),
        client_rect_host: Some(pb::RectI {
            x: -1920,
            y: 30,
            w: 16,
            h: 9,
        }),
        dpi_scale: 1.75,
        geometry_revision: 1,
        timestamp_ns: n as i64 * 1000,
    };
    FrameIdentity {
        session: id(1),
        frame_id: n,
        geometry: StreamGeometry::new(&wire, 16, 9).unwrap(),
        timestamp_ns: n as i64 * 1000,
    }
}
fn source(generation: u64) -> Source {
    Source {
        session: id(1),
        generation,
        geometry: identity(1).geometry,
        fps: 30,
        interaction: "live_annotate".into(),
        platform: "windows".into(),
        name: "Synthetic selected window".into(),
    }
}
fn frame(n: u64, delivery: u64, key: bool) -> pb::FrameTiles {
    let value = identity(n);
    pb::FrameTiles {
        capture_session_id: Some(value.session.to_proto()),
        frame_id: n,
        geometry: Some(value.geometry.wire().clone()),
        width: 16,
        height: 9,
        keyframe: key,
        tiles: Vec::new(),
        timestamp_ns: value.timestamp_ns,
        delivery_sequence: delivery,
        source_generation: 1,
        last_input_seq_applied: 0,
        captured_at_ms: 1_700_000_000_000 + n as i64,
    }
}
fn queue() -> MediaQueue {
    let mut q = MediaQueue::new(1).unwrap();
    q.open(source(1)).unwrap();
    q
}
fn decoded(q: &mut MediaQueue, n: u64, delivery: u64) {
    assert_eq!(q.push(frame(n, delivery, true)).unwrap(), Arrival::Queued);
    let next = q.take().unwrap().unwrap();
    assert!(q.settle(&next.ticket, true));
}

#[test]
fn start_and_epoch_are_admission_authority_not_media() {
    assert!(MediaQueue::new(0).is_err());
    let mut q = MediaQueue::new(1).unwrap();
    assert_eq!(q.push(frame(1, 1, true)).unwrap(), Arrival::Ignored);
    assert!(q.source().is_none());
    q.open(source(1)).unwrap();
    assert_eq!(q.push(frame(1, 2, false)).unwrap(), Arrival::Repair);
    assert!(q.needs_repair());
}
#[test]
fn duplicate_open_does_not_clear_the_decoded_base() {
    let mut q = queue();
    decoded(&mut q, 10, 1);
    assert!(!q.open(source(1)).unwrap());
    assert_eq!(q.push(frame(10, 2, true)).unwrap(), Arrival::Ignored);
    assert_eq!(q.push(frame(11, 3, false)).unwrap(), Arrival::Queued);
}
#[test]
fn late_repaired_original_cannot_regress_pixels_or_change_its_timestamp() {
    let mut q = queue();
    decoded(&mut q, 10, 1);
    assert_eq!(q.push(frame(9, 2, true)).unwrap(), Arrival::Ignored);
    let mut mismatch = frame(10, 3, true);
    mismatch.timestamp_ns += 1;
    mismatch.geometry.as_mut().unwrap().timestamp_ns += 1;
    assert!(q.push(mismatch).is_err());
}
#[test]
fn pending_delta_replacement_refuses_broken_chain_without_lifetime_failure() {
    let mut q = queue();
    decoded(&mut q, 1, 1);
    assert_eq!(q.push(frame(2, 2, false)).unwrap(), Arrival::Queued);
    assert_eq!(q.push(frame(3, 3, false)).unwrap(), Arrival::Repair);
    assert!(q.take().unwrap().is_none());
    assert!(q.needs_repair());
    decoded(&mut q, 3, 4);
    assert!(!q.needs_repair());
}
#[test]
fn older_repair_during_newer_decode_cannot_queue_a_pixel_regression() {
    let mut q = queue();
    q.push(frame(10, 1, true)).unwrap();
    let newer = q.take().unwrap().unwrap();
    assert_eq!(q.push(frame(9, 2, true)).unwrap(), Arrival::Ignored);
    assert!(q.settle(&newer.ticket, true));
    assert!(q.take().unwrap().is_none());
    assert!(!q.needs_repair());
    assert_eq!(q.push(frame(11, 3, false)).unwrap(), Arrival::Queued);
}
#[test]
fn one_pending_suffix_may_wait_behind_its_owned_decoder() {
    let mut q = queue();
    q.push(frame(1, 1, true)).unwrap();
    let first = q.take().unwrap().unwrap();
    assert_eq!(q.push(frame(1, 2, true)).unwrap(), Arrival::Ignored);
    // Repair stays true until the first independently decodable image settles.
    assert_eq!(q.push(frame(2, 3, false)).unwrap(), Arrival::Repair);
    assert!(q.settle(&first.ticket, true));
    decoded(&mut q, 2, 4);
    q.push(frame(3, 5, false)).unwrap();
    let third = q.take().unwrap().unwrap();
    assert_eq!(q.push(frame(4, 6, false)).unwrap(), Arrival::Queued);
    assert!(q.take().unwrap().is_none());
    assert!(q.settle(&third.ticket, true));
    let fourth = q.take().unwrap().unwrap();
    assert_eq!(fourth.ticket.frame.frame_id, 4);
}
#[test]
fn close_and_new_source_retain_old_worker_until_actual_settlement() {
    let mut q = queue();
    q.push(frame(1, 1, true)).unwrap();
    let old = q.take().unwrap().unwrap();
    assert!(q.close(&source(1).control("source_closed")).unwrap());
    assert!(!q.open(source(1)).unwrap());
    q.open(source(2)).unwrap();
    let mut replacement = frame(1, 1, true);
    replacement.source_generation = 2;
    q.push(replacement).unwrap();
    assert!(q.take().unwrap().is_none());
    assert!(!q.settle(&old.ticket, true));
    let fresh = q.take().unwrap().unwrap();
    assert_eq!(fresh.ticket.generation, 2);
    assert!(q.settle(&fresh.ticket, true));
    assert!(!q.settle(&old.ticket, true));
}
#[test]
fn malformed_future_and_foreign_generation_never_create_source() {
    let mut q = queue();
    let mut future = frame(100, 999, true);
    future.source_generation = 2;
    assert_eq!(q.push(future).unwrap(), Arrival::Ignored);
    decoded(&mut q, 1, 1);
    let mut malformed = frame(2, 2, true);
    malformed.width = u32::MAX;
    assert!(q.push(malformed).is_err());
    let mut altered = source(1);
    altered.fps = 60;
    assert!(q.open(altered).is_err());
}
#[test]
fn stamp_preserves_real_identity_across_keyframe_replay() {
    let mut sender = Delivery::new(source(1));
    let mut a = frame(30, 99, true);
    let before = identity(30);
    sender.stamp(&mut a).unwrap();
    assert_eq!(a.delivery_sequence, 1);
    sender.stamp(&mut a).unwrap();
    assert_eq!(a.delivery_sequence, 2);
    assert_eq!(a.frame_id, before.frame_id);
    assert_eq!(a.timestamp_ns, before.timestamp_ns);
    assert_eq!(a.geometry.as_ref(), Some(before.geometry.wire()));
}
#[test]
fn source_metadata_is_exact_and_does_not_imply_an_input_grant() {
    let initial = source(1);
    assert_eq!(Source::parse(&initial.control("start")).unwrap(), initial);
    let mut bad = initial.control("start");
    bad.window_handle += 1;
    assert!(Source::parse(&bad).is_err());
    bad = initial.control("start");
    bad.interaction = "remote_input".into();
    assert!(Source::parse(&bad).is_err());
    bad = initial.control("start");
    bad.source_generation = 0;
    assert!(Source::parse(&bad).is_err());
}

fn offer(data: &[u8]) -> OriginalOffer {
    OriginalOffer {
        scope: Scope {
            session: id(1),
            frame: 1,
            geometry_revision: 1,
            request: id(2),
            transfer: id(3),
            generation: 1,
        },
        frame: identity(1),
        asset: AssetId::hash(data),
        bytes: data.len() as u64,
        stamp: super::wire::CaptureStamp::new(1_700_000_000_001, "windows").unwrap(),
    }
}
#[test]
fn scoped_blob_exact_window_roundtrip_and_final_then_old_ack() {
    let bytes = vec![7; 300_000];
    let value = offer(&bytes);
    let mut sender =
        Sender::new(value.clone(), &value.request(), Cursor::new(bytes.clone())).unwrap();
    let mut receiver = Receiver::new(value, Cursor::new(Vec::new())).unwrap();
    let mut first = None;
    let mut final_ack = None;
    while let Some(chunk) = sender.next().unwrap() {
        let ack = receiver.receive(&chunk).unwrap();
        if first.is_none() {
            first = Some(ack.clone());
        }
        sender.acknowledge(&ack).unwrap();
        final_ack = Some(ack);
    }
    assert!(sender.verified());
    sender.acknowledge(&first.unwrap()).unwrap();
    sender.acknowledge(&final_ack.unwrap()).unwrap();
    assert!(sender.verified());
    assert_eq!(receiver.finish().unwrap().1.into_inner(), bytes);
}
#[test]
fn scoped_request_and_chunk_cannot_acquire_another_original_or_generation() {
    let bytes = vec![5; 128];
    let value = offer(&bytes);
    let mut foreign = value.request();
    foreign.key.as_mut().unwrap().generation += 1;
    assert!(Sender::new(value.clone(), &foreign, Cursor::new(bytes.clone())).is_err());
    let mut sender = Sender::new(value.clone(), &value.request(), Cursor::new(bytes)).unwrap();
    let mut receiver = Receiver::new(value, Cursor::new(Vec::new())).unwrap();
    let mut chunk = sender.next().unwrap().unwrap();
    chunk.key.as_mut().unwrap().request_id = Some(id(99).to_proto());
    assert!(receiver.receive(&chunk).is_err());
    assert!(receiver.finish().is_err());
}
#[test]
fn corrupt_completion_consumes_receiver_and_held_chunk_does_not_advance() {
    let bytes = vec![8; 128];
    let value = offer(&bytes);
    let mut sender = Sender::new(value.clone(), &value.request(), Cursor::new(bytes)).unwrap();
    let first = sender.next().unwrap().unwrap();
    sender.hold(first.clone()).unwrap();
    assert_eq!(sender.next().unwrap().unwrap(), first);
    let mut receiver = Receiver::new(value, Cursor::new(Vec::new())).unwrap();
    let mut corrupt = first.clone();
    let Some(pb::stream_blob::Body::Chunk(c)) = &mut corrupt.body else {
        panic!()
    };
    c.data[0] ^= 1;
    assert!(receiver.receive(&corrupt).is_err());
    assert!(receiver.receive(&first).is_err());
    assert!(receiver.finish().is_err());
}
#[test]
fn captured_time_and_platform_cannot_change_under_one_source_or_frame() {
    let mut q = queue();
    let first = frame(1, 1, true);
    q.push(first.clone()).unwrap();
    let mut changed = first;
    changed.delivery_sequence = 2;
    changed.captured_at_ms += 1;
    assert!(q.push(changed).is_err());
    let pending = q.take().unwrap().unwrap();
    assert_eq!(pending.ticket.stamp.platform, "windows");
    assert!(q.settle(&pending.ticket, true));
    let mut changed = frame(1, 3, true);
    changed.captured_at_ms -= 1;
    assert!(q.push(changed).is_err());
    let mut different = source(1);
    different.platform = "android".into();
    assert!(q.open(different).is_err());
    for bad in [0, -1, super::wire::MAX_CAPTURED_AT_MS + 1] {
        let mut changed = frame(2, 4, true);
        changed.captured_at_ms = bad;
        assert!(q.push(changed).is_err());
    }
}
#[test]
fn late_lossless_offer_must_match_frozen_wall_time_and_generation() {
    let value = offer(&[1, 2, 3]);
    let wire = value.wire();
    assert!(
        OriginalOffer::parse(&wire, &value.frame, &value.stamp, &value.scope.request, 1)
            .unwrap()
            .is_some()
    );
    let mut changed = wire.clone();
    changed.captured_at_ms += 1;
    assert!(
        OriginalOffer::parse(
            &changed,
            &value.frame,
            &value.stamp,
            &value.scope.request,
            1
        )
        .is_err()
    );
    assert!(
        OriginalOffer::parse(&wire, &value.frame, &value.stamp, &value.scope.request, 2).is_err()
    );
    assert!(super::wire::CaptureStamp::new(value.stamp.captured_at_ms, "unknown").is_err());
}
