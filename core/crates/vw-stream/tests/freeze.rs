#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;
use common::*;
use std::sync::Arc;
use vw_stream::*;
const WORK: u64 = 128 * 1024 * 1024;

#[test]
fn first_pen_freezes_displayed_frame_not_newer_decode_or_capture() {
    let mut r = ring(16, 16);
    let a = frame(&mut r, 1, 16, 16, [1, 2, 3, 255]);
    let da = Arc::new(display(&a, None, true, None));
    let b = frame(&mut r, 2, 16, 16, [9, 8, 7, 255]);
    let db = Arc::new(display(&b, None, true, None));
    let mut shown = PresentedFrames::new(1).unwrap();
    shown.offer(da).unwrap();
    let (ta, _) = shown.begin_presentation().unwrap().unwrap();
    shown.presented(&ta).unwrap();
    shown.offer(db).unwrap();
    let mut state = FreezeMachine::new(1, StreamMode::LiveAnnotate).unwrap();
    let request = state
        .first_contact(id(8), &shown, true, 100)
        .unwrap()
        .unwrap();
    assert_eq!(request.frame.frame_id, 1);
    assert_eq!(state.phase(), FreezePhase::Freezing);
    assert_eq!(state.draft().unwrap().displayed.identity().frame_id, 1);
}
#[test]
fn queued_frames_cannot_be_acknowledged_before_their_owned_draw_and_old_epoch_refuses() {
    let mut r = ring(16, 16);
    let a = frame(&mut r, 1, 16, 16, [1, 2, 3, 255]);
    let b = frame(&mut r, 2, 16, 16, [1, 2, 3, 255]);
    let mut shown = PresentedFrames::new(1).unwrap();
    shown
        .offer(Arc::new(display(&a, None, true, None)))
        .unwrap();
    let (ta, _) = shown.begin_presentation().unwrap().unwrap();
    shown
        .offer(Arc::new(display(&b, None, true, None)))
        .unwrap();
    assert!(shown.begin_presentation().unwrap().is_none());
    assert!(shown.displayed().is_none());
    shown.presented(&ta).unwrap();
    let (tb, draw) = shown.begin_presentation().unwrap().unwrap();
    assert_eq!(draw.identity().frame_id, 2);
    assert!(shown.presented(&ta).is_err());
    shown.presented(&tb).unwrap();
    let mut foreign = FreezeMachine::new(2, StreamMode::LiveAnnotate).unwrap();
    assert!(matches!(
        foreign.first_contact(id(9), &shown, true, 10),
        Err(StreamError::Stale)
    ));
    shown.invalidate();
    assert!(shown.displayed().is_none());
    assert!(shown.presented(&tb).is_err());
}
#[test]
fn late_actual_b_presentation_survives_c_and_d_offers_and_freezes_b() {
    let mut r = ring(16, 16);
    let mut shown = PresentedFrames::new(1).unwrap();
    let a = frame(&mut r, 1, 16, 16, [1, 2, 3, 255]);
    shown
        .offer(Arc::new(display(&a, None, true, None)))
        .unwrap();
    let (ta, _) = shown.begin_presentation().unwrap().unwrap();
    shown.presented(&ta).unwrap();
    let b = frame(&mut r, 2, 16, 16, [9, 8, 7, 255]);
    shown
        .offer(Arc::new(display(&b, None, true, None)))
        .unwrap();
    let (tb, actual_draw) = shown.begin_presentation().unwrap().unwrap();
    // B was drawn; its callback is queued. New decodes C/D cannot replace it.
    for number in 3..=4 {
        let next = frame(&mut r, number, 16, 16, [0, 0, 0, 255]);
        shown
            .offer(Arc::new(display(&next, None, true, None)))
            .unwrap();
    }
    assert!(shown.begin_presentation().unwrap().is_none());
    assert_eq!(actual_draw.identity().frame_id, 2);
    shown.presented(&tb).unwrap();
    let mut freeze = FreezeMachine::new(1, StreamMode::LiveAnnotate).unwrap();
    assert_eq!(
        freeze
            .first_contact(id(8), &shown, true, 100)
            .unwrap()
            .unwrap()
            .frame
            .frame_id,
        2
    );
    let (td, next) = shown.begin_presentation().unwrap().unwrap();
    assert_eq!(next.identity().frame_id, 4);
    shown.presented(&td).unwrap();
    assert_eq!(shown.displayed().unwrap().identity().frame_id, 4);
    assert_eq!(freeze.draft().unwrap().request.frame.frame_id, 2);
}
#[test]
fn explicitly_never_drawn_frame_releases_only_its_inflight_slot() {
    let mut r = ring(16, 16);
    let mut shown = PresentedFrames::new(1).unwrap();
    let a = frame(&mut r, 1, 16, 16, [1, 2, 3, 255]);
    shown
        .offer(Arc::new(display(&a, None, true, None)))
        .unwrap();
    let (ta, _) = shown.begin_presentation().unwrap().unwrap();
    let b = frame(&mut r, 2, 16, 16, [9, 8, 7, 255]);
    shown
        .offer(Arc::new(display(&b, None, true, None)))
        .unwrap();
    shown.not_presented(&ta).unwrap();
    assert!(shown.displayed().is_none());
    let (tb, _) = shown.begin_presentation().unwrap().unwrap();
    assert!(shown.not_presented(&ta).is_err());
    shown.presented(&tb).unwrap();
    assert_eq!(shown.displayed().unwrap().identity().frame_id, 2);
}
#[test]
fn remote_edit_and_invalid_hover_contact_never_freeze() {
    let mut r = ring(16, 16);
    let a = frame(&mut r, 1, 16, 16, [1, 2, 3, 255]);
    let mut shown = PresentedFrames::new(1).unwrap();
    shown
        .offer(Arc::new(display(&a, None, true, None)))
        .unwrap();
    let (t, _) = shown.begin_presentation().unwrap().unwrap();
    shown.presented(&t).unwrap();
    for (mode, valid) in [
        (StreamMode::RemoteEdit, true),
        (StreamMode::RemoteEdit, false),
        (StreamMode::LiveAnnotate, false),
    ] {
        let mut state = FreezeMachine::new(1, mode).unwrap();
        assert!(
            state
                .first_contact(id(8), &shown, valid, 0)
                .unwrap()
                .is_none()
        );
        assert_eq!(state.phase(), FreezePhase::Live);
    }
}
#[test]
fn timeout_flags_degraded_without_discarding_draft_then_exact_lossless_can_swap() {
    let mut r = ring(16, 16);
    let a = frame(&mut r, 1, 16, 16, [1, 2, 3, 255]);
    let mut shown = PresentedFrames::new(1).unwrap();
    shown
        .offer(Arc::new(display(&a, None, true, None)))
        .unwrap();
    let (t, _) = shown.begin_presentation().unwrap().unwrap();
    shown.presented(&t).unwrap();
    let mut state = FreezeMachine::new(1, StreamMode::LiveAnnotate).unwrap();
    let request = state
        .first_contact(id(8), &shown, true, 100)
        .unwrap()
        .unwrap();
    assert!(!state.tick(2099));
    assert!(state.tick(2100));
    assert_eq!(
        state.draft().unwrap().quality,
        FreezeQuality::DegradedCompressed
    );
    let encoded = encode_original(&a, MAX_LOSSLESS_BYTES, WORK, &Cancellation::default()).unwrap();
    let verified = verify_original(
        &encoded.identity,
        &encoded.asset_id,
        encoded.bytes(),
        WORK,
        &Cancellation::default(),
    )
    .unwrap();
    assert!(state.lossless(&request, &verified).unwrap());
    assert_eq!(state.draft().unwrap().request.draft_id, id(8));
    assert!(!state.unavailable(&request));
    assert!(matches!(
        state.draft().unwrap().quality,
        FreezeQuality::Lossless { .. }
    ));
}
#[test]
fn cancel_then_new_contact_refuses_old_lossless_reply_and_saved_document_is_not_discarded() {
    let mut r = ring(16, 16);
    let a = frame(&mut r, 1, 16, 16, [1, 2, 3, 255]);
    let mut shown = PresentedFrames::new(1).unwrap();
    shown
        .offer(Arc::new(display(&a, None, true, None)))
        .unwrap();
    let (t, _) = shown.begin_presentation().unwrap().unwrap();
    shown.presented(&t).unwrap();
    let mut state = FreezeMachine::new(1, StreamMode::LiveAnnotate).unwrap();
    let old = state
        .first_contact(id(8), &shown, true, 0)
        .unwrap()
        .unwrap();
    assert_eq!(state.cancel_resume(), Some(id(8)));
    let next = state
        .first_contact(id(9), &shown, true, 10)
        .unwrap()
        .unwrap();
    let encoded = encode_original(&a, MAX_LOSSLESS_BYTES, WORK, &Cancellation::default()).unwrap();
    let verified = verify_original(
        &encoded.identity,
        &encoded.asset_id,
        encoded.bytes(),
        WORK,
        &Cancellation::default(),
    )
    .unwrap();
    assert!(!state.lossless(&old, &verified).unwrap());
    assert!(state.lossless(&next, &verified).unwrap());
    state.saved(&next).unwrap();
    assert_eq!(state.cancel_resume(), None);
    assert_eq!(state.phase(), FreezePhase::Saved);
}
#[test]
fn source_close_retains_current_frozen_draft_and_blocks_silent_substitution() {
    let mut r = ring(16, 16);
    let a = frame(&mut r, 1, 16, 16, [1, 2, 3, 255]);
    let mut shown = PresentedFrames::new(1).unwrap();
    shown
        .offer(Arc::new(display(&a, None, true, None)))
        .unwrap();
    let (t, _) = shown.begin_presentation().unwrap().unwrap();
    shown.presented(&t).unwrap();
    let mut state = FreezeMachine::new(1, StreamMode::LiveAnnotate).unwrap();
    state.first_contact(id(8), &shown, true, 0).unwrap();
    state.source_closed();
    assert_eq!(state.draft().unwrap().request.frame.frame_id, 1);
    assert!(!state.source_available());
    state.cancel_resume();
    assert!(matches!(
        state.first_contact(id(9), &shown, true, 10),
        Err(StreamError::Unavailable)
    ));
}
