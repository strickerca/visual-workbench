#![allow(clippy::unwrap_used)]
mod common;
use common::*;
use std::sync::Arc;
use vw_stream::*;
const WORK: u64 = 128 * 1024 * 1024;

#[test]
fn keyframe_cannot_rewind_the_clock_or_change_source_without_an_explicit_reset() {
    let mut ring = ring(16, 16);
    let first = frame(&mut ring, 1, 16, 16, [10, 20, 30, 255]);
    let previous = display(&first, None, true, None);
    let next = frame(&mut ring, 2, 16, 16, [90, 80, 70, 255]);
    let packet = encode_tiles(&next, None, true, 85, WORK, &Cancellation::default()).unwrap();
    let mut old_clock = packet.clone();
    old_clock.timestamp_ns = previous.identity().timestamp_ns;
    old_clock.geometry.as_mut().unwrap().timestamp_ns = old_clock.timestamp_ns;
    assert!(matches!(
        decode_tiles(&old_clock, Some(&previous), WORK, &Cancellation::default()),
        Err(StreamError::Stale)
    ));
    let mut changed_source = packet;
    changed_source
        .geometry
        .as_mut()
        .unwrap()
        .client_rect_host
        .as_mut()
        .unwrap()
        .x += 1;
    assert!(matches!(
        decode_tiles(
            &changed_source,
            Some(&previous),
            WORK,
            &Cancellation::default()
        ),
        Err(StreamError::Stale)
    ));
    // A fresh, explicitly reset source owner has no old frame to mutate/reuse.
    assert!(decode_tiles(&changed_source, None, WORK, &Cancellation::default()).is_ok());
    assert_eq!(previous.identity().frame_id, 1);
}

#[test]
fn presentation_cannot_accept_a_new_id_with_an_old_capture_timestamp() {
    let mut ring = ring(16, 16);
    let first = frame(&mut ring, 1, 16, 16, [10, 20, 30, 255]);
    let mut shown = PresentedFrames::new(1).unwrap();
    shown
        .offer(Arc::new(display(&first, None, true, None)))
        .unwrap();
    let (ticket, _) = shown.begin_presentation().unwrap().unwrap();
    shown.presented(&ticket).unwrap();
    let next = frame(&mut ring, 2, 16, 16, [90, 80, 70, 255]);
    let mut packet = encode_tiles(&next, None, true, 85, WORK, &Cancellation::default()).unwrap();
    packet.timestamp_ns = 1;
    packet.geometry.as_mut().unwrap().timestamp_ns = 1;
    let independent = decode_tiles(&packet, None, WORK, &Cancellation::default()).unwrap();
    assert!(matches!(
        shown.offer(Arc::new(independent)),
        Err(StreamError::Stale)
    ));
    assert_eq!(shown.displayed().unwrap().identity().frame_id, 1);
}

#[test]
fn freeze_uses_the_acknowledged_frame_and_swaps_only_its_verified_lossless_original() {
    let mut ring = ring(16, 16);
    let first = frame(&mut ring, 1, 16, 16, [10, 20, 30, 255]);
    let mut shown = PresentedFrames::new(7).unwrap();
    shown
        .offer(Arc::new(display(&first, None, true, None)))
        .unwrap();
    let (ticket, drawing) = shown.begin_presentation().unwrap().unwrap();
    let newer = frame(&mut ring, 2, 16, 16, [90, 80, 70, 255]);
    shown
        .offer(Arc::new(display(&newer, None, true, None)))
        .unwrap();
    shown.presented(&ticket).unwrap();
    let mut freeze = FreezeMachine::new(7, StreamMode::LiveAnnotate).unwrap();
    let request = freeze
        .first_contact(id(9), &shown, true, 100)
        .unwrap()
        .unwrap();
    assert_eq!(request.frame, *drawing.identity());
    let original = ring
        .pin(&request.frame.session, request.frame.frame_id)
        .unwrap();
    let encoded = encode_original(
        &original,
        MAX_LOSSLESS_BYTES,
        WORK,
        &Cancellation::default(),
    )
    .unwrap();
    let verified = verify_original(
        &request.frame,
        &encoded.asset_id,
        encoded.bytes(),
        WORK,
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(verified.rgba(), first.rgba());
    assert!(freeze.lossless(&request, &verified).unwrap());
    assert_eq!(freeze.draft().unwrap().request.draft_id, id(9));
    assert_eq!(freeze.draft().unwrap().request.frame.frame_id, 1);
    assert!(matches!(
        freeze.draft().unwrap().quality,
        FreezeQuality::Lossless { .. }
    ));
    assert_eq!(freeze.phase(), FreezePhase::FrozenDraft);
    // This pure boundary never invents durable save/publication or a draw receipt.
    assert_ne!(freeze.phase(), FreezePhase::Saved);
}
