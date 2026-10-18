#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;
use common::*;
use std::sync::{Arc, Barrier};
use vw_model::DeviceId;
use vw_proto::{Message, v1};
use vw_stream::*;

const FRAME_BYTES: u64 = 8 * 8 * 4 + 4096;
const WORK: u64 = 128 * 1024 * 1024;

fn shared_ring(pool: &SessionBudget) -> FrameRing {
    FrameRing::new_budgeted(
        id(1),
        geometry(8, 8, 1000),
        8,
        8 * FRAME_BYTES,
        pool.clone(),
    )
    .unwrap()
}
fn packet(pool: &SessionBudget, original: &OriginalFrame) -> EncodedTiles {
    encode_tiles_budgeted(original, None, true, 85, pool, &Cancellation::default()).unwrap()
}
fn decoded(pool: &SessionBudget, original: &OriginalFrame) -> DisplayFrame {
    decode_tiles_budgeted(
        &packet(pool, original),
        None,
        pool,
        &Cancellation::default(),
    )
    .unwrap()
}

#[test]
fn explicit_endpoint_caps_and_overflow_refuse_without_changing_admission() {
    assert_eq!(PHONE_SESSION_BYTES, 256 * 1024 * 1024);
    assert_eq!(DESKTOP_SESSION_BYTES, 512 * 1024 * 1024);
    for invalid in [0, DESKTOP_SESSION_BYTES + 1, u64::MAX] {
        assert!(matches!(
            SessionBudget::new(invalid),
            Err(StreamError::Limit)
        ));
    }
    assert!(SessionBudget::new(DESKTOP_SESSION_BYTES).is_ok());
    let pool = SessionBudget::new(1).unwrap();
    let held = pool.reserve(MemoryKind::Foreign, 1).unwrap();
    assert!(matches!(
        pool.reserve(MemoryKind::Transfer, u64::MAX),
        Err(StreamError::Limit)
    ));
    assert!(matches!(
        pool.reserve(MemoryKind::Transfer, 1),
        Err(StreamError::Limit)
    ));
    assert_eq!(pool.used_bytes(), 1);
    let empty = pool.reserve(MemoryKind::Transfer, 0).unwrap();
    drop(empty);
    assert_eq!(pool.used_bytes(), 1);
    drop(held);
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn every_memory_category_and_clone_competes_for_one_pool() {
    let pool = SessionBudget::new(7).unwrap();
    let clone = pool.clone();
    assert!(clone.same_pool(&pool));
    assert!(!SessionBudget::new(7).unwrap().same_pool(&pool));
    let kinds = [
        MemoryKind::RingOriginal,
        MemoryKind::CodecWorkspace,
        MemoryKind::EncodedTiles,
        MemoryKind::DisplayFrame,
        MemoryKind::VerifiedOriginal,
        MemoryKind::Transfer,
        MemoryKind::Foreign,
    ];
    let mut owners = Vec::new();
    for (n, kind) in kinds.into_iter().enumerate() {
        let charge = clone.reserve(kind, 1).unwrap();
        assert_eq!(charge.kind(), kind);
        assert!(charge.belongs_to(&pool));
        owners.push(charge);
        assert_eq!(pool.used_bytes(), n as u64 + 1);
    }
    assert!(pool.reserve(MemoryKind::CodecWorkspace, 1).is_err());
    owners.pop();
    let last = pool.reserve(MemoryKind::CodecWorkspace, 1).unwrap();
    assert_eq!(clone.used_bytes(), clone.limit_bytes());
    drop(owners);
    assert_eq!(pool.used_bytes(), 1);
    drop(last);
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn concurrent_admission_and_release_never_oversubscribe_or_lose_credit() {
    let pool = SessionBudget::new(16).unwrap();
    let gate = Arc::new(Barrier::new(3));
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 0..2 {
            let shared = pool.clone();
            let gate = gate.clone();
            workers.push(scope.spawn(move || {
                for _ in 0..128 {
                    let charge = shared.reserve(MemoryKind::CodecWorkspace, 8).unwrap();
                    gate.wait();
                    gate.wait();
                    drop(charge);
                    gate.wait();
                    gate.wait();
                }
            }));
        }
        for _ in 0..128 {
            gate.wait();
            assert_eq!(pool.used_bytes(), 16);
            assert!(pool.reserve(MemoryKind::Foreign, 1).is_err());
            gate.wait();
            gate.wait();
            assert_eq!(pool.used_bytes(), 0);
            gate.wait();
        }
        for worker in workers {
            worker.join().unwrap();
        }
    });
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn evicted_and_closed_ring_originals_stay_charged_until_the_final_pin() {
    let pool = SessionBudget::new(8 * FRAME_BYTES).unwrap();
    let mut ring = shared_ring(&pool);
    let mut pins = Vec::new();
    for n in 1..=8 {
        pins.push(frame(&mut ring, n, 8, 8, [n as u8, 2, 3, 255]));
    }
    assert!(matches!(
        ring.begin_frame(identity(9, 8, 8), &Cancellation::default()),
        Err(StreamError::Limit)
    ));
    assert_eq!(pool.used_bytes(), 8 * FRAME_BYTES);
    assert_eq!(pins[0].rgba()[0], 1);
    drop(ring);
    assert_eq!(pool.used_bytes(), 8 * FRAME_BYTES);
    let final_pin = pins.pop().unwrap();
    let extra_reader = final_pin.clone();
    drop(pins);
    drop(final_pin);
    assert_eq!(pool.used_bytes(), FRAME_BYTES);
    drop(extra_reader);
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn foreign_pressure_refuses_capture_before_allocating_and_cancelled_readback_settles() {
    let pool = SessionBudget::new(FRAME_BYTES).unwrap();
    let mut ring = shared_ring(&pool);
    let foreign = pool.reserve(MemoryKind::Foreign, 1).unwrap();
    assert!(matches!(
        ring.reserve_frame(identity(1, 8, 8), &Cancellation::default()),
        Err(StreamError::Limit)
    ));
    assert_eq!(pool.used_bytes(), 1);
    drop(foreign);
    let pending = ring
        .reserve_frame(identity(1, 8, 8), &Cancellation::default())
        .unwrap();
    assert_eq!(pool.used_bytes(), FRAME_BYTES);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(matches!(
        pending.adopt([1, 2, 3, 255].repeat(64), &cancel),
        Err(StreamError::Cancelled)
    ));
    assert_eq!(pool.used_bytes(), 0);
    assert_eq!(ring.retained_frames(), 0);
}

#[test]
fn aggregate_real_ring_codec_output_display_original_transfer_and_foreign_pressure() {
    let pool = SessionBudget::new(WORK).unwrap();
    let mut ring = shared_ring(&pool);
    let original = frame(&mut ring, 1, 8, 8, [17, 33, 66, 255]);
    let tiles = packet(&pool, &original);
    let display = decode_tiles_budgeted(&tiles, None, &pool, &Cancellation::default()).unwrap();
    let png =
        encode_original_budgeted(&original, 64 * 1024, &pool, &Cancellation::default()).unwrap();
    let verified = png.verify(&pool, &Cancellation::default()).unwrap();
    assert_eq!(verified.rgba(), original.rgba());
    let copy = display
        .copy_into_transfer(&pool, &Cancellation::default())
        .unwrap();
    let foreign =
        BudgetedBytes::allocate(&pool, MemoryKind::Foreign, 32, &Cancellation::default()).unwrap();
    let used = FRAME_BYTES
        + tiles.charge().bytes()
        + display.charged_bytes()
        + png.charge().bytes()
        + verified.charged_bytes()
        + copy.charge().bytes()
        + foreign.charge().bytes();
    assert_eq!(pool.used_bytes(), used);
    let pressure = pool
        .reserve(MemoryKind::CodecWorkspace, WORK - used)
        .unwrap();
    assert_eq!(pool.used_bytes(), WORK);
    assert!(matches!(
        encode_tiles_budgeted(&original, None, true, 85, &pool, &Cancellation::default()),
        Err(StreamError::Limit)
    ));
    assert!(matches!(
        decode_tiles_budgeted(&tiles, None, &pool, &Cancellation::default()),
        Err(StreamError::Limit)
    ));
    assert!(matches!(
        png.verify(&pool, &Cancellation::default()),
        Err(StreamError::Limit)
    ));
    assert!(matches!(
        verified.copy_into_transfer(&pool, &Cancellation::default()),
        Err(StreamError::Limit)
    ));
    assert_eq!(pool.used_bytes(), WORK);
    assert_eq!(original.rgba(), [17, 33, 66, 255].repeat(64));
    drop(pressure);
    drop((ring, original, tiles, display, png, verified, copy, foreign));
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn move_to_transfer_preserves_allocation_and_never_opens_admission_credit() {
    let pool = SessionBudget::new(WORK).unwrap();
    let mut ring = shared_ring(&pool);
    let original = frame(&mut ring, 1, 8, 8, [11, 22, 33, 255]);
    let encoded =
        encode_original_budgeted(&original, 64 * 1024, &pool, &Cancellation::default()).unwrap();
    let identity = encoded.identity().clone();
    let asset = encoded.asset_id().clone();
    let pointer = encoded.bytes().as_ptr();
    let before = pool.used_bytes();
    let wire = encoded.into_transfer().unwrap();
    assert_eq!(wire.bytes().as_ptr(), pointer);
    assert_eq!(wire.charge().kind(), MemoryKind::Transfer);
    assert_eq!(pool.used_bytes(), before);
    let verified =
        verify_original_budgeted(&identity, &asset, &wire, &pool, &Cancellation::default())
            .unwrap();
    assert_eq!(verified.rgba(), original.rgba());
    let tiles = packet(&pool, &original);
    let before = pool.used_bytes();
    let pointer = tiles.packet().tiles[0].data.as_ptr();
    let tiles = tiles.into_transfer();
    assert_eq!(tiles.charge().kind(), MemoryKind::Transfer);
    assert_eq!(tiles.packet().tiles[0].data.as_ptr(), pointer);
    assert_eq!(pool.used_bytes(), before);
    assert!(decode_tiles_budgeted(&tiles, None, &pool, &Cancellation::default()).is_ok());
    drop((ring, original, wire, verified, tiles));
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn physical_transfer_copy_requires_second_reservation_and_shared_readers_do_not() {
    let pool = SessionBudget::new(16).unwrap();
    let payload = Arc::new(
        BudgetedBytes::copy_from(
            &pool,
            MemoryKind::Foreign,
            &[7; 16],
            &Cancellation::default(),
        )
        .unwrap(),
    );
    let reader = payload.clone();
    assert_eq!(pool.used_bytes(), 16);
    assert!(matches!(
        BudgetedBytes::copy_from(
            &pool,
            MemoryKind::Transfer,
            payload.bytes(),
            &Cancellation::default()
        ),
        Err(StreamError::Limit)
    ));
    drop(payload);
    assert_eq!(pool.used_bytes(), 16);
    assert_eq!(reader.bytes(), &[7; 16]);
    drop(reader);
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn queued_drawing_and_frozen_frames_share_charge_until_last_platform_reader() {
    let pool = SessionBudget::new(WORK).unwrap();
    let mut ring = shared_ring(&pool);
    let first = frame(&mut ring, 1, 8, 8, [11, 22, 33, 255]);
    let display = Arc::new(decoded(&pool, &first));
    let display_charge = display.charged_bytes();
    let mut shown = PresentedFrames::new_budgeted(7, pool.clone()).unwrap();
    shown.offer(display.clone()).unwrap();
    let (token, platform_reader) = shown.begin_presentation().unwrap().unwrap();
    let second = frame(&mut ring, 2, 8, 8, [44, 55, 66, 255]);
    let second_display = Arc::new(decoded(&pool, &second));
    shown.offer(second_display).unwrap();
    shown.presented(&token).unwrap();
    let mut freeze =
        FreezeMachine::new_budgeted(7, StreamMode::LiveAnnotate, pool.clone()).unwrap();
    let request = freeze
        .first_contact(id(2), &shown, true, 10)
        .unwrap()
        .unwrap();
    assert_eq!(request.frame, *first.identity());
    shown.invalidate();
    drop(display);
    drop((ring, first, second));
    assert_eq!(pool.used_bytes(), display_charge);
    freeze.cancel_resume();
    assert_eq!(pool.used_bytes(), display_charge);
    assert_eq!(platform_reader.identity(), &request.frame);
    drop(platform_reader);
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn accepted_lossless_arc_stays_charged_and_late_cancelled_settlement_cannot_replace_draft() {
    let pool = SessionBudget::new(WORK).unwrap();
    let mut ring = shared_ring(&pool);
    let original = frame(&mut ring, 1, 8, 8, [11, 22, 33, 255]);
    let display = Arc::new(decoded(&pool, &original));
    let png =
        encode_original_budgeted(&original, 64 * 1024, &pool, &Cancellation::default()).unwrap();
    let verified = Arc::new(png.verify(&pool, &Cancellation::default()).unwrap());
    let verified_charge = verified.charged_bytes();
    let mut shown = PresentedFrames::new_budgeted(9, pool.clone()).unwrap();
    shown.offer(display).unwrap();
    let (token, platform) = shown.begin_presentation().unwrap().unwrap();
    shown.presented(&token).unwrap();
    let mut freeze =
        FreezeMachine::new_budgeted(9, StreamMode::LiveAnnotate, pool.clone()).unwrap();
    let request = freeze
        .first_contact(id(2), &shown, true, 0)
        .unwrap()
        .unwrap();
    assert!(matches!(
        freeze.lossless(&request, &verified),
        Err(StreamError::Invalid)
    ));
    assert!(
        freeze
            .lossless_budgeted(&request, verified.clone())
            .unwrap()
    );
    assert!(Arc::ptr_eq(
        freeze.draft().unwrap().lossless_original().unwrap(),
        &verified
    ));
    assert_eq!(freeze.phase(), FreezePhase::FrozenDraft);
    freeze.cancel_resume();
    assert!(
        !freeze
            .lossless_budgeted(&request, verified.clone())
            .unwrap()
    );
    assert!(freeze.draft().is_none());
    shown.invalidate();
    drop((platform, ring, original, png));
    assert_eq!(pool.used_bytes(), verified_charge);
    drop(verified);
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn failed_renderer_retirement_remains_quarantined_and_late_draw_arc_retains_charge() {
    struct Renderer {
        quarantine: Option<Arc<BudgetedBytes>>,
    }
    impl Renderer {
        fn retire(&mut self, platform_confirmed: bool) -> Result<()> {
            if !platform_confirmed {
                return Err(StreamError::Unavailable);
            }
            self.quarantine = None;
            Ok(())
        }
    }
    let pool = SessionBudget::new(16).unwrap();
    let foreign = Arc::new(
        BudgetedBytes::allocate(&pool, MemoryKind::Foreign, 16, &Cancellation::default()).unwrap(),
    );
    let late_draw = foreign.clone();
    let mut renderer = Renderer {
        quarantine: Some(foreign),
    };
    assert!(renderer.retire(false).is_err());
    assert_eq!(pool.used_bytes(), 16);
    assert!(pool.reserve(MemoryKind::DisplayFrame, 1).is_err());
    renderer.retire(true).unwrap();
    assert_eq!(pool.used_bytes(), 16);
    drop(late_draw);
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn stale_platform_receipt_cannot_credit_an_in_flight_owner() {
    let pool = SessionBudget::new(WORK).unwrap();
    let mut ring = shared_ring(&pool);
    let original = frame(&mut ring, 1, 8, 8, [11, 22, 33, 255]);
    let display = Arc::new(decoded(&pool, &original));
    let charge = display.charged_bytes();
    let mut shown = PresentedFrames::new_budgeted(1, pool.clone()).unwrap();
    shown.offer(display.clone()).unwrap();
    let (token, platform) = shown.begin_presentation().unwrap().unwrap();
    let mut other_epoch = PresentedFrames::new_budgeted(2, pool.clone()).unwrap();
    other_epoch.offer(display).unwrap();
    let (wrong, wrong_platform) = other_epoch.begin_presentation().unwrap().unwrap();
    let before = pool.used_bytes();
    assert!(matches!(
        shown.not_presented(&wrong),
        Err(StreamError::Stale)
    ));
    assert_eq!(pool.used_bytes(), before);
    shown.not_presented(&token).unwrap();
    other_epoch.invalidate();
    drop((ring, original, wrong_platform));
    assert_eq!(pool.used_bytes(), charge);
    drop(platform);
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn production_entrypoints_reject_unbudgeted_or_different_pool_inputs() {
    let pool = SessionBudget::new(WORK).unwrap();
    let other_pool = SessionBudget::new(WORK).unwrap();
    let mut ring = shared_ring(&pool);
    let original = frame(&mut ring, 1, 8, 8, [11, 22, 33, 255]);
    assert!(matches!(
        encode_tiles_budgeted(
            &original,
            None,
            true,
            85,
            &other_pool,
            &Cancellation::default()
        ),
        Err(StreamError::Invalid)
    ));
    assert!(matches!(
        encode_original_budgeted(&original, 1024, &other_pool, &Cancellation::default()),
        Err(StreamError::Invalid)
    ));
    let packet = packet(&pool, &original);
    assert!(matches!(
        decode_tiles_budgeted(&packet, None, &other_pool, &Cancellation::default()),
        Err(StreamError::Invalid)
    ));
    let unbudgeted =
        Arc::new(decode_tiles(packet.packet(), None, WORK, &Cancellation::default()).unwrap());
    let mut shown = PresentedFrames::new_budgeted(1, pool.clone()).unwrap();
    assert!(matches!(
        shown.offer(unbudgeted.clone()),
        Err(StreamError::Invalid)
    ));
    let mut isolated_shown = PresentedFrames::new(1).unwrap();
    isolated_shown.offer(unbudgeted).unwrap();
    let (token, _) = isolated_shown.begin_presentation().unwrap().unwrap();
    isolated_shown.presented(&token).unwrap();
    let mut freeze =
        FreezeMachine::new_budgeted(1, StreamMode::LiveAnnotate, pool.clone()).unwrap();
    assert!(matches!(
        freeze.first_contact(id(2), &isolated_shown, true, 0),
        Err(StreamError::Invalid)
    ));
    assert_eq!(other_pool.used_bytes(), 0);
}

#[test]
fn codec_refusal_and_cancelled_outputs_release_workspace_but_keep_existing_payloads() {
    let pool = SessionBudget::new(WORK).unwrap();
    let mut ring = shared_ring(&pool);
    let original = frame(&mut ring, 1, 8, 8, [11, 22, 33, 255]);
    let before = pool.used_bytes();
    assert!(encode_original_budgeted(&original, 1, &pool, &Cancellation::default()).is_err());
    assert!(
        encode_tiles_budgeted(&original, None, true, 1, &pool, &Cancellation::default()).is_err()
    );
    assert_eq!(pool.used_bytes(), before);
    let packet = packet(&pool, &original);
    let png = encode_original_budgeted(&original, 1024, &pool, &Cancellation::default()).unwrap();
    let before = pool.used_bytes();
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(matches!(
        encode_tiles_budgeted(&original, None, true, 85, &pool, &cancel),
        Err(StreamError::Cancelled)
    ));
    assert!(matches!(
        decode_tiles_budgeted(&packet, None, &pool, &cancel),
        Err(StreamError::Cancelled)
    ));
    assert!(matches!(
        png.verify(&pool, &cancel),
        Err(StreamError::Cancelled)
    ));
    assert_eq!(pool.used_bytes(), before);
    drop((ring, original, packet, png));
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn inbound_packet_reservation_rejects_unadmitted_capacity_and_cancelled_parse() {
    let pool = SessionBudget::new(WORK).unwrap();
    let mut ring = shared_ring(&pool);
    let original = frame(&mut ring, 1, 8, 8, [11, 22, 33, 255]);
    let mut raw = encode_tiles(&original, None, true, 85, WORK, &Cancellation::default()).unwrap();
    raw.tiles[0].data.reserve_exact(MAX_TILE_BYTES);
    let before = pool.used_bytes();
    let reserved = EncodedTiles::reserve(&pool, &Cancellation::default()).unwrap();
    assert!(matches!(
        reserved.adopt(raw, &Cancellation::default()),
        Err(StreamError::Limit)
    ));
    assert_eq!(pool.used_bytes(), before);
    let raw = encode_tiles(&original, None, true, 85, WORK, &Cancellation::default()).unwrap();
    let reserved = EncodedTiles::reserve(&pool, &Cancellation::default()).unwrap();
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(matches!(
        reserved.adopt(raw, &cancel),
        Err(StreamError::Cancelled)
    ));
    assert_eq!(pool.used_bytes(), before);
}

#[test]
fn impossible_allocation_and_precancelled_transfer_refuse_without_payload_allocation() {
    let pool = SessionBudget::new(16).unwrap();
    assert!(matches!(
        BudgetedBytes::allocate(
            &pool,
            MemoryKind::Transfer,
            usize::MAX,
            &Cancellation::default()
        ),
        Err(StreamError::Limit)
    ));
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(matches!(
        BudgetedBytes::allocate(&pool, MemoryKind::Foreign, 8, &cancel),
        Err(StreamError::Cancelled)
    ));
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn budgeted_delta_preserves_old_frame_and_strict_jpeg_admission() {
    let pool = SessionBudget::new(WORK).unwrap();
    let mut ring = shared_ring(&pool);
    let first = frame(&mut ring, 1, 8, 8, [11, 22, 33, 255]);
    let old = decoded(&pool, &first);
    let second = frame(&mut ring, 2, 8, 8, [11, 22, 33, 255]);
    let delta = encode_tiles_budgeted(
        &second,
        Some(&first),
        false,
        85,
        &pool,
        &Cancellation::default(),
    )
    .unwrap();
    assert!(delta.packet().tiles.is_empty());
    let next = decode_tiles_budgeted(&delta, Some(&old), &pool, &Cancellation::default()).unwrap();
    assert_eq!(old.rgba(), next.rgba());
    assert_eq!(next.identity(), second.identity());
    let key = packet(&pool, &first);
    let jpeg = &key.packet().tiles[0].data;
    assert!(validate_jpeg_tile_header(jpeg, 8, 8).is_ok());
    assert!(validate_jpeg_tile_header(jpeg, 7, 8).is_err());
    assert!(validate_jpeg_tile_header(jpeg, 0, 8).is_err());
    assert!(validate_jpeg_tile_header(jpeg, TILE_SIDE + 1, 8).is_err());
    let mut metadata = jpeg.clone();
    metadata.splice(2..2, [0xff, 0xe2, 0, 6, b'I', b'C', b'C', 0]);
    assert!(validate_jpeg_tile_header(&metadata, 8, 8).is_err());
}

#[test]
fn wire_serialization_charges_physical_copy_before_writing_and_preserves_packet() {
    let pool = SessionBudget::new(WORK).unwrap();
    let mut ring = shared_ring(&pool);
    let original = frame(&mut ring, 1, 8, 8, [11, 22, 33, 255]);
    let packet = packet(&pool, &original);
    let before = pool.used_bytes();
    let wire = packet
        .to_transfer_bytes(&pool, &Cancellation::default())
        .unwrap();
    assert_eq!(wire.charge().kind(), MemoryKind::Transfer);
    assert_eq!(pool.used_bytes(), before + wire.bytes().len() as u64);
    assert_eq!(
        v1::FrameTiles::decode(wire.bytes()).unwrap(),
        *packet.packet()
    );
    let pressure = pool
        .reserve(MemoryKind::Foreign, WORK - pool.used_bytes())
        .unwrap();
    assert!(matches!(
        packet.to_transfer_bytes(&pool, &Cancellation::default()),
        Err(StreamError::Limit)
    ));
    assert_eq!(pool.used_bytes(), WORK);
    drop(pressure);
    drop((ring, original, packet));
    assert_eq!(pool.used_bytes(), wire.bytes().len() as u64);
    drop(wire);
    assert_eq!(pool.used_bytes(), 0);
}

#[test]
fn budgeted_approved_crop_preserves_original_identity_pixels_and_physical_geometry() {
    let pool = SessionBudget::new(WORK).unwrap();
    let mut ring = shared_ring(&pool);
    let original = frame(&mut ring, 1, 8, 8, [11, 22, 33, 255]);
    let peer = AuthenticatedPeer {
        device: DeviceId::from_bytes([1; 16]),
        epoch: 1,
    };
    let mut consent = RegionConsent::new(peer.clone()).unwrap();
    consent.request(&peer, 1, id(2), 0).unwrap();
    let offer = consent.approve(&id(2), id(3), original.clone(), 1).unwrap();
    let region = consent
        .accept(
            &peer,
            &RegionChoice {
                request_id: id(2),
                offer_id: id(3),
                frame: offer.frame,
                rectangle: TileRect {
                    x: 2,
                    y: 3,
                    width: 4,
                    height: 2,
                },
            },
            2,
        )
        .unwrap();
    let png = encode_region_budgeted(&region, 4096, &pool, &Cancellation::default()).unwrap();
    let verified = png.verify(&pool, &Cancellation::default()).unwrap();
    assert!(verified.belongs_to(&pool));
    assert_eq!(
        verified.identity().timestamp_ns,
        original.identity().timestamp_ns
    );
    assert_eq!(verified.identity().frame_id, original.identity().frame_id);
    assert_eq!(verified.identity().session, original.identity().session);
    let rect = verified
        .identity()
        .geometry
        .wire()
        .client_rect_host
        .as_ref()
        .unwrap();
    assert_eq!((rect.x, rect.y, rect.w, rect.h), (-1918, -253, 4, 2));
    assert_eq!(verified.rgba(), [11, 22, 33, 255].repeat(8));
    drop((ring, original, consent, region, png, verified));
    assert_eq!(pool.used_bytes(), 0);
}
