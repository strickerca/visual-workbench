#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
fn send(hub: &CaptureStatusHub, now: Instant) -> Option<pb::AgentCaptureStatus> {
    let mut result = None;
    hub.flush_at(now, |value| {
        result = Some(value);
        Ok(())
    })
    .unwrap();
    result
}
fn active(role: Role) -> (Arc<CaptureStatusHub>, CaptureStatusEpoch) {
    let hub = CaptureStatusHub::with_role(role);
    let epoch = hub.activate(1, true).unwrap();
    (hub, epoch)
}
fn response(nonce: u64, sequence: u64, count: u32) -> pb::AgentCaptureStatus {
    pb::AgentCaptureStatus {
        probe: false,
        nonce,
        sequence,
        known: true,
        active_grant_count: count,
        active_capture: false,
        remaining_ms: if count == 0 { 0 } else { 60_000 },
    }
}
fn view(hub: &CaptureStatusHub, now: Instant) -> AgentCaptureDisplay {
    let mut state = hub.state.lock().unwrap();
    hub.expire(&mut state, now);
    hub.display(&state, now)
}
#[test]
fn inactive_requires_current_receiver_probe_and_fresh_explicit_host_snapshot() {
    let (phone, _phone_epoch) = active(Role::Receiver);
    let (host, _host_epoch) = active(Role::Publisher);
    let now = Instant::now();
    phone.receive_at(response(1, 1, 0), 1, now).unwrap();
    assert!(!view(&phone, now).known);
    let probe = send(&phone, now).unwrap();
    host.receive_at(probe, 1, now).unwrap();
    let unknown = send(&host, now).unwrap();
    assert!(!unknown.known);
    phone.receive_at(unknown, 1, now).unwrap();
    assert!(!view(&phone, now).known);
    host.set(1, 0, false, 0, now).unwrap();
    let reply = send(&host, now + UPDATE_INTERVAL).unwrap();
    phone.receive_at(reply, 1, now + UPDATE_INTERVAL).unwrap();
    let actual = view(&phone, now + UPDATE_INTERVAL);
    assert!(actual.known);
    assert_eq!(actual.active_grant_count, 0);
}
#[test]
fn replacement_revoke_and_capture_cleanup_are_full_scalar_snapshots() {
    let (phone, _epoch) = active(Role::Receiver);
    let now = Instant::now();
    let probe = send(&phone, now).unwrap();
    phone
        .receive_at(response(probe.nonce, 1, 3), 1, now)
        .unwrap();
    assert_eq!(view(&phone, now).active_grant_count, 3);
    let mut capture = response(probe.nonce, 2, 1);
    capture.active_capture = true;
    phone.receive_at(capture, 1, now).unwrap();
    assert_eq!(view(&phone, now).active_grant_count, 1);
    let mut revoked = response(probe.nonce, 3, 0);
    revoked.active_capture = true;
    phone.receive_at(revoked, 1, now).unwrap();
    let actual = view(&phone, now);
    assert!(actual.active_capture);
    assert_eq!(actual.active_grant_count, 0);
    phone
        .receive_at(response(probe.nonce, 4, 0), 1, now)
        .unwrap();
    assert!(!view(&phone, now).active_capture);
}
#[test]
fn delayed_inactive_cannot_replace_active_from_an_expired_probe() {
    let (phone, _epoch) = active(Role::Receiver);
    let now = Instant::now();
    let first = send(&phone, now).unwrap();
    phone
        .receive_at(response(first.nonce, 1, 2), 1, now)
        .unwrap();
    phone
        .receive_at(response(first.nonce, 2, 0), 1, now + TTL)
        .unwrap();
    assert!(!view(&phone, now + TTL).known);
    let next = send(&phone, now + TTL).unwrap();
    phone
        .receive_at(response(first.nonce, 100, 0), 1, now + TTL)
        .unwrap();
    assert!(!view(&phone, now + TTL).known);
    phone
        .receive_at(response(next.nonce, 2, 1), 1, now + TTL)
        .unwrap();
    assert_eq!(view(&phone, now + TTL).active_grant_count, 1);
}
#[test]
fn stale_and_duplicate_wire_sequences_do_not_overwrite_newer_active_state() {
    let (phone, _epoch) = active(Role::Receiver);
    let now = Instant::now();
    let probe = send(&phone, now).unwrap();
    phone
        .receive_at(response(probe.nonce, 7, 4), 1, now)
        .unwrap();
    for sequence in [1, 6, 7] {
        phone
            .receive_at(response(probe.nonce, sequence, 0), 1, now)
            .unwrap();
    }
    assert_eq!(view(&phone, now).active_grant_count, 4);
}
#[test]
fn disconnect_and_new_epoch_refuse_retired_source_and_received_updates() {
    let (host, epoch) = active(Role::Publisher);
    let now = Instant::now();
    host.set(1, 2, false, 50_000, now).unwrap();
    drop(epoch);
    assert!(!view(&host, now).known);
    let _next = host.activate(3, true).unwrap();
    assert!(!view(&host, now).known);
    assert!(host.set(1, 0, false, 0, now).is_err());
    assert!(
        host.receive_at(
            pb::AgentCaptureStatus {
                probe: true,
                nonce: 1,
                ..Default::default()
            },
            1,
            now
        )
        .is_err()
    );
    host.set(3, 2, false, 40_000, now).unwrap();
    assert_eq!(view(&host, now).active_grant_count, 2);
}
#[test]
fn receiver_cannot_publish_or_spoof_authority_and_host_rejects_status_from_receiver() {
    let (phone, _p) = active(Role::Receiver);
    let (host, _h) = active(Role::Publisher);
    let now = Instant::now();
    assert!(matches!(
        phone.set(1, 0, false, 0, now),
        Err(SessionError::Authentication)
    ));
    assert!(matches!(
        host.receive_at(response(1, 1, 0), 1, now),
        Err(SessionError::Authentication)
    ));
    assert!(matches!(
        phone.receive_at(
            pb::AgentCaptureStatus {
                probe: true,
                nonce: 1,
                ..Default::default()
            },
            1,
            now
        ),
        Err(SessionError::Authentication)
    ));
}
#[test]
fn expired_grant_or_stalled_host_callback_becomes_unknown_never_inactive() {
    let (host, _h) = active(Role::Publisher);
    let now = Instant::now();
    host.set(1, 2, false, 100, now).unwrap();
    assert!(!view(&host, now + Duration::from_millis(100)).known);
    host.set(1, 0, false, 0, now).unwrap();
    assert!(!view(&host, now + LOCAL_TTL).known);
    let (phone, _p) = active(Role::Receiver);
    let phone_now = Instant::now();
    let probe = send(&phone, phone_now).unwrap();
    let mut value = response(probe.nonce, 1, 1);
    value.remaining_ms = 20;
    phone.receive_at(value, 1, phone_now).unwrap();
    assert!(!view(&phone, phone_now + Duration::from_millis(20)).known);
}
#[test]
fn bounds_unknown_shape_and_wrong_direction_are_refused_before_state_change() {
    for value in [
        response(0, 1, 1),
        response(1, 0, 1),
        response(1, 1, 65),
        pb::AgentCaptureStatus {
            remaining_ms: 600_001,
            ..response(1, 1, 1)
        },
        pb::AgentCaptureStatus {
            known: false,
            ..response(1, 1, 1)
        },
        pb::AgentCaptureStatus {
            probe: true,
            ..response(1, 1, 1)
        },
    ] {
        assert!(validate_wire(&value).is_err());
    }
    let (phone, _p) = active(Role::Receiver);
    let now = Instant::now();
    send(&phone, now);
    assert!(!view(&phone, now).known);
    assert!(phone.receive_at(response(1, 1, 65), 1, now).is_err());
    assert!(!view(&phone, now).known);
}
#[test]
fn backpressure_retains_latest_snapshot_and_lawful_burst_has_bounded_notifications() {
    let (host, _h) = active(Role::Publisher);
    let now = Instant::now();
    host.set(1, 3, false, 60_000, now).unwrap();
    host.receive_at(
        pb::AgentCaptureStatus {
            probe: true,
            nonce: 1,
            ..Default::default()
        },
        1,
        now,
    )
    .unwrap();
    host.flush_at(now, |_| Err(vw_net::NetError::Backpressure))
        .unwrap();
    host.set(1, 0, false, 0, now).unwrap();
    let value = send(&host, now).unwrap();
    assert_eq!(value.sequence, 1);
    assert_eq!(value.active_grant_count, 0);
    let (phone, _p) = active(Role::Receiver);
    let phone_now = Instant::now();
    let probe = send(&phone, phone_now).unwrap();
    let before = phone.signal.borrow().sequence;
    for sequence in 1..=200 {
        phone
            .receive_at(response(probe.nonce, sequence, 1), 1, phone_now)
            .unwrap();
    }
    assert!(phone.signal.borrow().sequence <= before + 1);
    assert_eq!(phone.state.lock().unwrap().received, 200);
    send(&phone, phone_now + UPDATE_INTERVAL);
    assert!(phone.signal.borrow().known);
}
#[test]
fn unsupported_capability_and_snapshot_expiry_stay_observably_unknown() {
    let hub = CaptureStatusHub::with_role(Role::Receiver);
    let _epoch = hub.activate(1, false).unwrap();
    assert!(send(&hub, Instant::now()).is_none());
    assert!(!hub.snapshot().unwrap().known);
    let (phone, _p) = active(Role::Receiver);
    let now = Instant::now();
    let probe = send(&phone, now).unwrap();
    phone
        .receive_at(response(probe.nonce, 1, 1), 1, now)
        .unwrap();
    let before = phone.signal.borrow().sequence;
    view(&phone, now + TTL);
    let retired = phone.signal.borrow().clone();
    assert!(!retired.known);
    assert!(retired.sequence > before);
    view(&phone, now + TTL);
    assert_eq!(phone.signal.borrow().sequence, retired.sequence);
}
