use super::*;
use std::collections::BTreeSet;
use vw_model::{DeviceId, Id};
use vw_proto::v1::{self as pb, envelope::Body, input_event::Event};
fn id(n: u64) -> Result<Id> {
    Ok(Id::from_parts(1_700_000_000_000 + n, [7; 10])?)
}
fn local(n: u8, remote: bool) -> LocalHello {
    LocalHello {
        device: DeviceId::from_bytes([n; 16]),
        platform: pb::Platform::Windows,
        app_version: "remote-protocol-fixture".into(),
        label: String::new(),
        capabilities: if remote {
            BTreeSet::from(["remote_edit_v1".into(), "agent_capture_status_v1".into()])
        } else {
            BTreeSet::new()
        },
    }
}
fn sessions(remote: bool) -> Result<(Session, Session)> {
    let a = local(1, remote);
    let b = local(2, remote);
    let connection = id(99)?;
    let (host, ack) = Session::accept(
        &b,
        &AuthenticatedPeer::for_simulation(a.device.clone()),
        &a.hello(&connection),
    )?;
    Ok((
        Session::finish(
            &a,
            &AuthenticatedPeer::for_simulation(b.device.clone()),
            connection,
            &ack,
        )?,
        host,
    ))
}
fn scope() -> Result<pb::RemoteScope> {
    Ok(pb::RemoteScope {
        connection_epoch: 1,
        capture_session_id: Some(id(2)?.to_proto()),
        source_generation: 1,
        target_token: Some(id(3)?.to_proto()),
        geometry_revision: 1,
    })
}
fn pen(flags: Option<u32>, scoped: bool) -> Result<pb::InputEvent> {
    let f = flags.unwrap_or(0);
    Ok(pb::InputEvent {
        input_session_id: Some(id(1)?.to_proto()),
        capture_session_id: Some(id(2)?.to_proto()),
        geometry_revision: 1,
        input_seq: 1,
        remote_scope: if scoped { Some(scope()?) } else { None },
        request_nonce: 0,
        event: Some(Event::Pen(pb::PenEvent {
            phase: pb::PenPhase::Down as i32,
            x: 4.0,
            y: 5.0,
            pressure: 0.5,
            barrel: f & 1 != 0,
            eraser: f & 4 != 0,
            native_pen_flags: flags,
            ..Default::default()
        })),
    })
}
fn control() -> Result<Body> {
    Ok(Body::RemoteControl(pb::RemoteControl {
        scope: Some(scope()?),
        sequence: 1,
        action: "request_control".into(),
        ..Default::default()
    }))
}
fn config() -> Result<Body> {
    Ok(Body::RemoteVideoConfig(pb::RemoteVideoConfig {
        scope: Some(scope()?),
        generation: 1,
        visible_width: 639,
        visible_height: 479,
        coded_width: 640,
        coded_height: 480,
        vps: vec![1],
        sps: vec![2],
        pps: vec![3],
        encoder_capabilities_json: b"{}".to_vec(),
    }))
}
fn video() -> Result<Body> {
    Ok(Body::VideoFrame(pb::VideoFrame {
        capture_session_id: Some(id(2)?.to_proto()),
        frame_id: 1,
        codec: "hevc".into(),
        keyframe: true,
        pts_ns: 1000,
        annexb: vec![1],
        remote_scope: Some(scope()?),
        config_generation: 1,
        coded_width: 640,
        coded_height: 480,
        visible_width: 639,
        visible_height: 479,
        captured_qpc_100ns: 10,
        ..Default::default()
    }))
}
fn wire(body: Body) -> Result<Frame> {
    let channel = channel_for(&body);
    Ok(Frame {
        channel,
        envelope: pb::Envelope {
            channel: channel as i32,
            connection_id: Some(id(99)?.to_proto()),
            seq: 1,
            body: Some(body),
        },
    })
}

#[test]
fn exact_flags_preserve_inverted_eraser_and_combined_values_over_protobuf() -> Result<()> {
    let (mut tx, mut rx) = sessions(true)?;
    rx.input.grant(id(1)?, id(2)?, 1, 0)?;
    for flags in 0..=7 {
        let mut event = pen(Some(flags), true)?;
        event.input_seq = u64::from(flags) + 1;
        tx.enqueue(Body::InputEvent(event))?;
        let frame = tx
            .next_frame()?
            .ok_or(NetError::Invalid("missing fixture frame"))?;
        let decoded = decode_frame(&encode_frame(&frame)?)?;
        let Receive::Deliver(Body::InputEvent(value)) = rx.receive(decoded, u64::from(flags))?
        else {
            return Err(NetError::Invalid("not delivered"));
        };
        let Some(Event::Pen(value)) = value.event else {
            return Err(NetError::Invalid("not pen"));
        };
        assert_eq!(value.native_pen_flags, Some(flags));
        assert_eq!(value.barrel, flags & 1 != 0);
        assert_eq!(value.eraser, flags & 4 != 0);
    }
    Ok(())
}
#[test]
fn scoped_pen_requires_exact_presence_and_rejects_projection_conflicts() -> Result<()> {
    let (mut tx, _) = sessions(true)?;
    assert!(tx.enqueue(Body::InputEvent(pen(None, true)?)).is_err());
    for flags in [8, u32::MAX] {
        assert!(
            tx.enqueue(Body::InputEvent(pen(Some(flags), true)?))
                .is_err()
        );
    }
    for bit in [1, 4] {
        let mut event = pen(Some(bit), true)?;
        if let Some(Event::Pen(p)) = &mut event.event {
            if bit == 1 {
                p.barrel = false
            } else {
                p.eraser = false
            }
        }
        assert!(tx.enqueue(Body::InputEvent(event)).is_err());
    }
    Ok(())
}
#[test]
fn legacy_pen_boolean_semantics_stay_intact_and_new_flags_need_scope() -> Result<()> {
    let (mut tx, mut rx) = sessions(false)?;
    rx.input.grant(id(1)?, id(2)?, 1, 0)?;
    let mut legacy = pen(None, false)?;
    if let Some(Event::Pen(p)) = &mut legacy.event {
        p.eraser = true;
        p.barrel = true;
    }
    tx.enqueue(Body::InputEvent(legacy))?;
    let frame = tx
        .next_frame()?
        .ok_or(NetError::Invalid("legacy fixture"))?;
    let Receive::Deliver(Body::InputEvent(event)) = rx.receive(frame, 1)? else {
        return Err(NetError::Invalid("legacy not delivered"));
    };
    let Some(Event::Pen(p)) = event.event else {
        return Err(NetError::Invalid("legacy not pen"));
    };
    assert!(p.eraser && p.barrel);
    assert_eq!(p.native_pen_flags, None);
    assert!(tx.enqueue(Body::InputEvent(pen(Some(0), false)?)).is_err());
    Ok(())
}
#[test]
fn remote_control_config_media_and_input_require_bilateral_capability() -> Result<()> {
    for body in [
        control()?,
        config()?,
        video()?,
        Body::InputEvent(pen(Some(0), true)?),
    ] {
        let (mut tx, mut rx) = sessions(false)?;
        assert!(tx.enqueue(body.clone()).is_err());
        assert!(rx.receive(wire(body)?, 0).is_err());
    }
    Ok(())
}
#[test]
fn remote_messages_use_existing_authenticated_control_media_and_input_channels() -> Result<()> {
    for (body, expected) in [
        (control()?, pb::Channel::Control),
        (config()?, pb::Channel::Control),
        (video()?, pb::Channel::Media),
        (Body::InputEvent(pen(Some(0), true)?), pb::Channel::Input),
    ] {
        assert_eq!(channel_for(&body), expected);
        let (mut tx, _) = sessions(true)?;
        tx.enqueue(body)?;
        let frame = tx
            .next_frame()?
            .ok_or(NetError::Invalid("remote fixture"))?;
        assert_eq!(frame.channel, expected);
        assert_eq!(decode_frame(&encode_frame(&frame)?)?, frame);
    }
    Ok(())
}
#[test]
fn malformed_scope_config_and_unscoped_remote_metadata_are_refused() -> Result<()> {
    let (mut tx, _) = sessions(true)?;
    let Body::RemoteControl(mut c) = control()? else {
        return Err(NetError::Invalid("fixture"));
    };
    if let Some(s) = &mut c.scope {
        s.connection_epoch = 2
    };
    assert!(tx.enqueue(Body::RemoteControl(c)).is_err());
    let Body::RemoteVideoConfig(mut c) = config()? else {
        return Err(NetError::Invalid("fixture"));
    };
    c.coded_width = 642;
    assert!(tx.enqueue(Body::RemoteVideoConfig(c)).is_err());
    let Body::RemoteVideoConfig(mut c) = config()? else {
        return Err(NetError::Invalid("fixture"));
    };
    c.vps = vec![0; 8192];
    assert!(tx.enqueue(Body::RemoteVideoConfig(c)).is_err());
    let Body::VideoFrame(mut v) = video()? else {
        return Err(NetError::Invalid("fixture"));
    };
    v.remote_scope = None;
    assert!(tx.enqueue(Body::VideoFrame(v)).is_err());
    Ok(())
}
#[test]
fn held_keys_and_unscoped_finite_commands_are_refused_only_on_new_route() -> Result<()> {
    let (mut tx, _) = sessions(true)?;
    let mut event = pen(Some(0), true)?;
    event.event = Some(Event::Key(pb::KeyEvent {
        virtual_key: 65,
        down: true,
        ..Default::default()
    }));
    assert!(tx.enqueue(Body::InputEvent(event)).is_err());
    let mut event = pen(None, false)?;
    event.event = Some(Event::Finite(pb::RemoteFiniteInput {
        kind: "click".into(),
        button: 1,
        ..Default::default()
    }));
    assert!(tx.enqueue(Body::InputEvent(event)).is_err());
    Ok(())
}
#[test]
fn agent_capture_status_survives_minor_six_and_disconnect_discards_old_grant() -> Result<()> {
    let (mut tx, mut rx) = sessions(true)?;
    rx.input.grant(id(1)?, id(2)?, 1, 0)?;
    let status = Body::AgentCaptureStatus(pb::AgentCaptureStatus {
        probe: true,
        nonce: 1,
        ..Default::default()
    });
    tx.enqueue(status.clone())?;
    let frame = tx.next_frame()?.ok_or(NetError::Invalid("agent fixture"))?;
    assert_eq!(rx.receive(frame, 1)?, Receive::Deliver(status));
    tx.enqueue(Body::InputEvent(pen(Some(0), true)?))?;
    let frame = tx
        .next_frame()?
        .ok_or(NetError::Invalid("old grant fixture"))?;
    rx.disconnect();
    assert_eq!(rx.receive(frame, 2)?, Receive::Discard);
    Ok(())
}

fn injection_status(state: &str, qpc: u64, reason: &str) -> Result<Body> {
    Ok(Body::InputStatus(pb::InputStatus {
        input_session_id: Some(id(1)?.to_proto()),
        state: state.into(),
        reason: reason.into(),
        remote_scope: Some(scope()?),
        input_seq: 7,
        accepted_qpc_100ns: qpc,
        editor_action: 1,
        request_nonce: 9,
    }))
}
#[test]
fn scoped_injection_partial_and_refusal_receipts_roundtrip_with_exact_correlation() -> Result<()> {
    for body in [
        injection_status("injected", 101, "")?,
        injection_status("refused", 0, "native_refused")?,
        injection_status("sealed_partial", 0, "partial_input")?,
    ] {
        let (mut tx, mut rx) = sessions(true)?;
        tx.enqueue(body.clone())?;
        let encoded = tx
            .next_frame()?
            .ok_or(NetError::Invalid("receipt absent"))?;
        assert_eq!(channel_for(&body), pb::Channel::Input);
        assert!(
            matches!(rx.receive(decode_frame(&encode_frame(&encoded)?)?,0)?,Receive::Deliver(v) if v==body)
        );
    }
    Ok(())
}
#[test]
fn status_cannot_claim_injection_from_refusal_or_bad_reason_ordinal() -> Result<()> {
    for body in [
        injection_status("injected", 0, "")?,
        injection_status("injected", 1, "native_refused")?,
        injection_status("refused", 1, "native_refused")?,
        injection_status("sealed_partial", 0, "native_refused")?,
        injection_status("refused", 0, "arbitrary message")?,
    ] {
        let (mut tx, _) = sessions(true)?;
        assert!(tx.enqueue(body).is_err());
    }
    for action in [0, 17, u32::MAX] {
        let Body::InputStatus(mut v) = injection_status("refused", 0, "native_refused")? else {
            return Err(NetError::Invalid("fixture"));
        };
        v.editor_action = action;
        let (mut tx, _) = sessions(true)?;
        assert!(tx.enqueue(Body::InputStatus(v)).is_err());
    }
    Ok(())
}
#[test]
fn command_request_nonce_is_finite_and_not_admitted_on_other_control_actions() -> Result<()> {
    let make = |action: &str, nonce: u64, ordinal: u32| -> Result<Body> {
        Ok(Body::RemoteControl(pb::RemoteControl {
            scope: Some(scope()?),
            sequence: 1,
            action: action.into(),
            request_nonce: nonce,
            editor_action: ordinal,
            ..Default::default()
        }))
    };
    for body in [
        make("command_request", 0, 1)?,
        make("command_request", 1, 0)?,
        make("command_request", 1, 17)?,
        make("grant", 1, 1)?,
    ] {
        let (mut tx, _) = sessions(true)?;
        assert!(tx.enqueue(body).is_err());
    }
    let (mut tx, _) = sessions(true)?;
    tx.enqueue(make("command_request", 1, 1)?)?;
    Ok(())
}
#[test]
fn reason_codes_are_closed_and_remote_receipts_require_capability_and_scope() -> Result<()> {
    let allowed = [
        "",
        "owner_pause",
        "background",
        "surface_lost",
        "decoder_failure",
        "input_expired",
        "video_owner_retired",
        "input_owner_retired",
        "command_timeout",
        "partial_input",
        "native_refused",
        "target_changed",
        "current_state_unknown",
        "peer_background",
        "host_revoked",
        "connection_retired",
    ];
    for reason in allowed {
        assert!(remote_reason_allowed(reason));
    }
    for reason in ["unknown", "OWNER_PAUSE", "background\n", "host_revoked "] {
        assert!(!remote_reason_allowed(reason));
    }
    let (mut tx, _) = sessions(false)?;
    assert!(tx.enqueue(injection_status("injected", 1, "")?).is_err());
    let Body::InputStatus(mut v) = injection_status("injected", 1, "")? else {
        return Err(NetError::Invalid("fixture"));
    };
    v.remote_scope = None;
    let (mut tx, _) = sessions(true)?;
    assert!(tx.enqueue(Body::InputStatus(v)).is_err());
    Ok(())
}

#[test]
fn selected_destination_is_bounded_display_metadata_only_on_start() -> Result<()> {
    let make = |action: &str, label: &str| -> Result<Body> {
        Ok(Body::RemoteControl(pb::RemoteControl {
            scope: Some(scope()?),
            sequence: 1,
            action: action.into(),
            selected_destination_label: label.into(),
            ..Default::default()
        }))
    };
    let body = make("start", "Untitled - Krita · krita.exe")?;
    let (mut tx, mut rx) = sessions(true)?;
    tx.enqueue(body.clone())?;
    let frame = tx
        .next_frame()?
        .ok_or(NetError::Invalid("metadata fixture"))?;
    assert_eq!(rx.receive(frame, 0)?, Receive::Deliver(body));
    for body in [
        make("start", &"x".repeat(2049))?,
        make("start", "forged\nstatus")?,
        make("grant", "Krita")?,
    ] {
        let (mut tx, _) = sessions(true)?;
        assert!(tx.enqueue(body).is_err());
    }
    Ok(())
}

#[test]
fn authority_update_requires_exact_nonzero_finite_correlation() -> Result<()> {
    let make = |action: &str, seq: u64, nonce: u64, ordinal: u32| -> Result<Body> {
        Ok(Body::RemoteControl(pb::RemoteControl {
            scope: Some(scope()?),
            sequence: 1,
            action: action.into(),
            command_input_seq: seq,
            request_nonce: nonce,
            editor_action: ordinal,
            ..Default::default()
        }))
    };
    for body in [
        make("authority", 0, 0, 1)?,
        make("authority", 7, 0, 0)?,
        make("authority", 7, 0, 17)?,
        make("grant", 7, 0, 0)?,
        make("command_refused", 0, 0, 1)?,
    ] {
        let (mut tx, _) = sessions(true)?;
        assert!(tx.enqueue(body).is_err());
    }
    for body in [
        make("authority", 7, 0, 1)?,
        make("authority", 7, 5, 1)?,
        make("command_refused", 0, 5, 1)?,
    ] {
        let (mut tx, _) = sessions(true)?;
        tx.enqueue(body)?;
    }
    Ok(())
}
