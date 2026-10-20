#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::{
    memory::{ByteBudget, OwnedFrame},
    order::{DecodeTicket, DecodeWork, Source},
    receiver::StreamReceiver,
};
use std::sync::Arc;
use vw_model::Id;
use vw_proto::v1 as pb;
use vw_stream::{Cancellation, FrameIdentity, FrameRing, StreamGeometry};

pub(super) static WORKERS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
pub(super) const BUDGET: u64 = 128 * 1024 * 1024;
pub(super) fn id(n: u8) -> Id {
    Id::from_parts(1_700_000_000_000, [n; 10]).unwrap()
}
pub(super) fn identity(n: u64) -> FrameIdentity {
    FrameIdentity {
        session: id(1),
        frame_id: n,
        timestamp_ns: n as i64 * 1000,
        geometry: StreamGeometry::new(
            &pb::CaptureGeometry {
                source_kind: "window".into(),
                window_handle: 7,
                monitor_id: String::new(),
                client_rect_host: Some(pb::RectI {
                    x: -20,
                    y: 10,
                    w: 16,
                    h: 9,
                }),
                dpi_scale: 1.75,
                geometry_revision: 1,
                timestamp_ns: n as i64 * 1000,
            },
            16,
            9,
        )
        .unwrap(),
    }
}
pub(super) fn source(generation: u64) -> Source {
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
pub(super) fn stamp(n: u64) -> super::wire::CaptureStamp {
    super::wire::CaptureStamp::new(1_700_000_000_000 + n as i64, "windows").unwrap()
}
pub(super) fn packet(n: u64, generation: u64) -> pb::FrameTiles {
    let frame = identity(n);
    let mut ring = FrameRing::default_for(frame.session.clone(), frame.geometry.clone()).unwrap();
    let mut pixels = ring.begin_frame(frame, &Cancellation::default()).unwrap();
    for rgba in pixels.rgba_mut().as_chunks_mut::<4>().0 {
        rgba.copy_from_slice(&[n as u8, 120, 160, 255]);
    }
    let original = pixels.finish(&Cancellation::default()).unwrap();
    let mut packet =
        vw_stream::encode_tiles(&original, None, true, 85, BUDGET, &Cancellation::default())
            .unwrap();
    packet.delivery_sequence = n;
    packet.source_generation = generation;
    packet.captured_at_ms = stamp(n).captured_at_ms;
    packet
}
pub(super) fn work(n: u64) -> DecodeWork {
    DecodeWork {
        ticket: DecodeTicket {
            epoch: 1,
            serial: n,
            delivery: n,
            generation: 1,
            frame: identity(n),
            stamp: stamp(n),
        },
        packet: packet(n, 1),
    }
}
pub(super) fn owned(n: u64, budget: &ByteBudget) -> Arc<OwnedFrame> {
    let cancel = Cancellation::default();
    let packet = vw_stream::EncodedTiles::reserve(budget.pool(), &cancel)
        .unwrap()
        .adopt(packet(n, 1), &cancel)
        .unwrap();
    let frame = vw_stream::decode_tiles_budgeted(&packet, None, budget.pool(), &cancel).unwrap();
    OwnedFrame::new(frame, stamp(n), budget.reserve(0).unwrap()).unwrap()
}
async fn decoded(receiver: &mut StreamReceiver, n: u64) {
    receiver.media(packet(n, 1)).unwrap();
    assert!(receiver.start_decode().unwrap());
    assert!(receiver.complete_decode().await.unwrap());
}

#[tokio::test]
async fn decode_is_not_presentation_and_first_contact_waits_for_exact_draw_receipt() {
    let _guard = WORKERS.lock().await;
    let mut receiver = StreamReceiver::new(1, BUDGET).unwrap();
    receiver.open(&source(1).control("start")).unwrap();
    decoded(&mut receiver, 1).await;
    assert!(receiver.contact(id(8), true, 0).is_err());
    let draw = receiver.begin_draw().unwrap().unwrap();
    assert!(receiver.contact(id(8), true, 0).is_err());
    receiver.draw_receipt(1, &draw.token, true).unwrap();
    decoded(&mut receiver, 2).await;
    let frozen = receiver.contact(id(8), true, 10).unwrap().unwrap();
    assert_eq!(frozen.frame.frame_id, 1);
    assert!(receiver.begin_draw().unwrap().is_none());
    assert!(receiver.tick(2010));
    assert_eq!(
        receiver.frozen().unwrap().quality(),
        Some(&vw_stream::FreezeQuality::DegradedCompressed)
    );
    receiver.shutdown().await.unwrap();
}
#[tokio::test]
async fn source_loss_allows_late_actual_receipt_preserves_pixels_and_never_substitutes_a_draft() {
    let _guard = WORKERS.lock().await;
    let mut receiver = StreamReceiver::new(1, BUDGET).unwrap();
    receiver.open(&source(1).control("start")).unwrap();
    decoded(&mut receiver, 1).await;
    let draw = receiver.begin_draw().unwrap().unwrap();
    receiver
        .control(&source(1).control("source_closed"))
        .unwrap();
    receiver.draw_receipt(1, &draw.token, true).unwrap();
    assert!(receiver.contact(id(8), true, 0).is_err());
    assert_eq!(draw.frame.identity().frame_id, 1);
    receiver.open(&source(2).control("start")).unwrap();
    assert!(receiver.draw_receipt(1, &draw.token, true).is_err());
    receiver.shutdown().await.unwrap();
}
#[tokio::test]
async fn new_source_and_cancelled_old_decode_do_not_publish_old_pixels() {
    let _guard = WORKERS.lock().await;
    let mut receiver = StreamReceiver::new(1, BUDGET).unwrap();
    receiver.open(&source(1).control("start")).unwrap();
    receiver.media(packet(1, 1)).unwrap();
    receiver.start_decode().unwrap();
    receiver.open(&source(2).control("start")).unwrap();
    receiver.media(packet(2, 2)).unwrap();
    assert!(!receiver.start_decode().unwrap());
    assert!(!receiver.complete_decode().await.unwrap());
    assert!(receiver.start_decode().unwrap());
    assert!(receiver.complete_decode().await.unwrap());
    let draw = receiver.begin_draw().unwrap().unwrap();
    assert_eq!(draw.frame.identity().frame_id, 2);
    receiver.shutdown().await.unwrap();
}
#[tokio::test]
async fn frozen_draft_refuses_remote_source_replacement_and_explicit_dispose_resumes_with_repair() {
    let _guard = WORKERS.lock().await;
    let mut receiver = StreamReceiver::new(1, BUDGET).unwrap();
    receiver.open(&source(1).control("start")).unwrap();
    decoded(&mut receiver, 1).await;
    let draw = receiver.begin_draw().unwrap().unwrap();
    receiver.draw_receipt(1, &draw.token, true).unwrap();
    let request = receiver.contact(id(8), true, 0).unwrap().unwrap();
    assert!(receiver.open(&source(2).control("start")).is_err());
    assert_eq!(receiver.frozen().unwrap().request(), Some(&request));
    assert_eq!(receiver.cancel_resume_after_draft_disposed(), Some(id(8)));
    assert_eq!(receiver.repair_control().unwrap().action, "keyframe");
    receiver.shutdown().await.unwrap();
}
#[tokio::test]
async fn remote_edit_never_requests_a_freeze() {
    let _guard = WORKERS.lock().await;
    let mut receiver = StreamReceiver::new(1, BUDGET).unwrap();
    let mut value = source(1);
    value.interaction = "remote_edit".into();
    receiver.open(&value.control("start")).unwrap();
    decoded(&mut receiver, 1).await;
    let draw = receiver.begin_draw().unwrap().unwrap();
    receiver.draw_receipt(1, &draw.token, true).unwrap();
    assert!(receiver.contact(id(8), true, 0).unwrap().is_none());
    assert!(receiver.frozen().unwrap().request().is_none());
    receiver.shutdown().await.unwrap();
}
#[test]
fn coalesced_and_escaped_frames_keep_actual_byte_credit_through_freeze_and_disposal() {
    use super::presentation::{Freeze, Presentation};
    let budget = ByteBudget::new(BUDGET).unwrap();
    let charge = 16 * 9 * 4 + 4096;
    let mut shown = Presentation::new(1, &budget).unwrap();
    shown.offer(owned(1, &budget)).unwrap();
    let first = shown.begin().unwrap().unwrap();
    shown.settle(&first.token, true).unwrap();
    drop(first);
    shown.offer(owned(2, &budget)).unwrap();
    let actual = shown.begin().unwrap().unwrap();
    shown.offer(owned(3, &budget)).unwrap();
    shown.offer(owned(4, &budget)).unwrap();
    assert_eq!(budget.used(), charge * 3);
    shown.settle(&actual.token, true).unwrap();
    assert_eq!(budget.used(), charge * 2);
    let mut frozen = Freeze::new(1, vw_stream::StreamMode::LiveAnnotate, &budget).unwrap();
    assert_eq!(
        frozen
            .contact(id(8), &shown, true, 0)
            .unwrap()
            .unwrap()
            .frame
            .frame_id,
        2
    );
    drop(shown);
    assert_eq!(budget.used(), charge);
    drop(frozen);
    assert_eq!(budget.used(), charge);
    drop(actual);
    assert_eq!(budget.used(), 0);
}
#[test]
fn packet_spare_capacity_cannot_bypass_the_queued_reservation() {
    let mut packet = packet(1, 1);
    packet.tiles[0].format = String::with_capacity(vw_stream::MAX_TILE_BYTES + 128 * 1024);
    packet.tiles[0].format.push_str("jpeg");
    assert!(super::wire::frame_identity(&packet).is_err());
}
#[test]
fn byte_reservation_overflow_and_failed_admission_do_not_change_used_bytes() {
    let budget = ByteBudget::new(1024).unwrap();
    let reserved = budget.reserve(512).unwrap();
    assert!(budget.reserve(u64::MAX).is_err());
    assert!(budget.reserve(513).is_err());
    assert_eq!(budget.used(), 512);
    drop(reserved);
    assert_eq!(budget.used(), 0);
}
