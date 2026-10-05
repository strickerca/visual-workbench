use vw_remote::{
    applied::{Applied, Receipt},
    profile::{Action, CanvasSelector, ImageIdentity, Profile},
    *,
};
fn id(n: u32) -> String {
    format!("01900000-0000-7000-8000-{n:012x}")
}
fn scope() -> Scope {
    Scope {
        connection_epoch: 1,
        capture_session_id: id(1),
        source_generation: 1,
        target_token: id(2),
        geometry_revision: 1,
    }
}
fn binding() -> Binding {
    Binding {
        scope: scope(),
        input_session_id: id(3),
    }
}
fn nal(kind: u8, body: &[u8]) -> Vec<u8> {
    [&[0, 0, 0, 1, kind << 1, 1][..], body].concat()
}
fn sps(width: u32, height: u32) -> Vec<u8> {
    fn bits(out: &mut Vec<bool>, value: u32, n: usize) {
        for b in (0..n).rev() {
            out.push(value & (1 << b) != 0)
        }
    }
    fn ue(out: &mut Vec<bool>, value: u32) {
        let v = value + 1;
        let n = (32 - v.leading_zeros()) as usize;
        out.extend(std::iter::repeat_n(false, n - 1));
        bits(out, v, n)
    }
    let mut b = Vec::new();
    bits(&mut b, 0, 4);
    bits(&mut b, 0, 3);
    bits(&mut b, 1, 1);
    bits(&mut b, 0, 2);
    bits(&mut b, 0, 1);
    bits(&mut b, 1, 5);
    bits(&mut b, 0, 32);
    bits(&mut b, 0, 24);
    bits(&mut b, 0, 24);
    bits(&mut b, 120, 8);
    ue(&mut b, 0);
    ue(&mut b, 1);
    ue(&mut b, width);
    ue(&mut b, height);
    bits(&mut b, 0, 1);
    ue(&mut b, 0);
    ue(&mut b, 0);
    b.push(true);
    let mut bytes = Vec::new();
    let mut zeros = 0;
    for c in b.chunks(8) {
        let mut byte = 0;
        for (n, bit) in c.iter().enumerate() {
            if *bit {
                byte |= 1 << (7 - n)
            }
        }
        if zeros >= 2 && byte <= 3 {
            bytes.push(3);
            zeros = 0
        }
        bytes.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    nal(33, &bytes)
}
fn config() -> VideoConfig {
    VideoConfig {
        scope: scope(),
        generation: 1,
        visible_width: 800,
        visible_height: 600,
        coded_width: 800,
        coded_height: 600,
        vps: nal(32, &[1]),
        sps: sps(800, 600),
        pps: nal(34, &[1]),
        encoder: EncoderCapabilities {
            name: "fixture".into(),
            vendor_attribute: Some("VEN_8086".into()),
            adapter_vendor: 0x8086,
            hardware_enumerated: true,
            intel_vendor_confirmed: true,
            low_latency_control_accepted: true,
            zero_b_control_accepted: true,
            cbr_control_accepted: true,
            maximum_bitrate_control_accepted: true,
            bitrate: BITRATE,
            one_frame_in_flight: 1,
        },
    }
}
fn frame(keyframe: bool) -> Frame {
    Frame {
        scope: scope(),
        config_generation: 1,
        frame_id: 1,
        pts_100ns: 100,
        captured_qpc_100ns: 100,
        input_session_id: None,
        last_input_seq_applied: 0,
        keyframe,
        annexb: nal(if keyframe { 19 } else { 1 }, &[0x80]),
    }
}
#[test]
fn inline_changed_sps_is_rejected_on_delta_and_idr() -> Result<()> {
    let c = config();
    c.validate()?;
    for keyframe in [false, true] {
        let mut f = frame(keyframe);
        f.validate(&c)?;
        let mut changed = c.sps.clone();
        if let Some(last) = changed.last_mut() {
            *last ^= 1
        }
        f.annexb = [changed, f.annexb].concat();
        assert_eq!(f.validate(&c), Err(Error::Invalid));
    }
    Ok(())
}
#[test]
fn matching_inline_parameters_preserve_frame_identity() -> Result<()> {
    let c = config();
    let mut f = frame(true);
    f.annexb = [c.vps.clone(), c.sps.clone(), c.pps.clone(), f.annexb].concat();
    f.validate(&c)
}
#[test]
fn changed_generation_and_invented_ack_are_rejected() {
    let c = config();
    let mut f = frame(true);
    f.config_generation = 2;
    assert_eq!(f.validate(&c), Err(Error::Invalid));
    f.config_generation = 1;
    f.last_input_seq_applied = 7;
    assert_eq!(f.validate(&c), Err(Error::Invalid));
}
#[test]
fn main_sps_dimension_and_depth_bounds_are_enforced() {
    assert_eq!(annexb::sps(&sps(800, 600), 802, 600), Err(Error::Invalid));
    assert_eq!(annexb::sps(&sps(800, 600), 800, 600), Ok(()));
}
#[test]
fn mixed_irap_and_repeated_parameter_sets_are_refused() {
    let f = [nal(19, &[1]), nal(1, &[1])].concat();
    assert!(annexb::access_unit(&f).is_err());
    let f = [nal(32, &[1]), nal(32, &[2]), nal(19, &[1])].concat();
    assert!(annexb::access_unit(&f).is_err());
}
#[test]
fn older_queued_frame_cannot_ack_newer_injection() -> Result<()> {
    let mut l = Applied::new(binding())?;
    l.record(Receipt {
        binding: binding(),
        input_seq: 1,
        accepted_qpc_100ns: 100,
    })?;
    l.record(Receipt {
        binding: binding(),
        input_seq: 2,
        accepted_qpc_100ns: 200,
    })?;
    assert_eq!(l.for_frame(&scope(), 150), (Some(id(3)), 1));
    assert_eq!(l.for_frame(&scope(), 50), (Some(id(3)), 0));
    assert_eq!(l.for_frame(&scope(), 200), (Some(id(3)), 2));
    Ok(())
}
#[test]
fn transport_and_keepalive_have_no_implicit_ack() -> Result<()> {
    let l = Applied::new(binding())?;
    assert_eq!(l.for_frame(&scope(), u64::MAX), (Some(id(3)), 0));
    Ok(())
}
#[test]
fn wrong_grant_seals_ack_ledger() -> Result<()> {
    let mut l = Applied::new(binding())?;
    let mut b = binding();
    b.input_session_id = id(4);
    assert_eq!(
        l.record(Receipt {
            binding: b,
            input_seq: 1,
            accepted_qpc_100ns: 100
        }),
        Err(Error::Invalid)
    );
    assert_eq!(l.for_frame(&scope(), u64::MAX), (None, 0));
    Ok(())
}
#[test]
fn old_epoch_geometry_and_retired_grant_have_no_ack() -> Result<()> {
    let mut l = Applied::new(binding())?;
    l.record(Receipt {
        binding: binding(),
        input_seq: 1,
        accepted_qpc_100ns: 100,
    })?;
    let mut s = scope();
    s.connection_epoch = 3;
    assert_eq!(l.for_frame(&s, 200), (None, 0));
    s = scope();
    s.geometry_revision = 2;
    assert_eq!(l.for_frame(&s, 200), (None, 0));
    l.retire();
    assert_eq!(l.for_frame(&scope(), 200), (None, 0));
    Ok(())
}
#[test]
fn overlong_ipc_header_refuses_before_payload_allocation() {
    let size = 65537u32.to_le_bytes();
    let mut bytes = &size[..];
    assert!(matches!(wire::read_command(&mut bytes), Err(Error::Limit)));
}
fn profile() -> Profile {
    let i = ImageIdentity {
        executable_name: "krita.exe".into(),
        executable_blake3: "a".repeat(64),
        executable_bytes: 4096,
        file_version: Some("5.2.0.0".into()),
        package_full_name: None,
        package_version: None,
    };
    Profile {
        schema_version: 1,
        acceptance_receipt_digest: "b".repeat(64),
        guard_lifecycle_receipt_digest: "c".repeat(64),
        executable_name: i.executable_name,
        executable_blake3: i.executable_blake3,
        executable_bytes: i.executable_bytes,
        file_version: i.file_version,
        package_full_name: i.package_full_name,
        package_version: i.package_version,
        tool_id: "observed fixture".into(),
        settings_digest: "d".repeat(64),
        canvas_selector: CanvasSelector {
            framework_id: "Qt".into(),
            class_name: "fixtureCanvas".into(),
            automation_id: String::new(),
            control_type: 50033,
            require_keyboard_focus: true,
        },
        actions: vec![Action {
            action: 1,
            keys: vec![0x11, 0x5a],
            native_batch_digest: "e".repeat(64),
            effect_receipt_digest: "f".repeat(64),
        }],
    }
}
#[test]
fn profile_shape_never_admits_global_or_held_key_routes() {
    let mut p = profile();
    assert_eq!(p.validate(), Ok(()));
    p.actions[0].keys.push(0x5b);
    assert_eq!(p.validate(), Err(Error::Invalid));
    p.actions[0].keys = vec![0x11, 0x11];
    assert_eq!(p.validate(), Err(Error::Invalid));
}
#[test]
fn unknown_canvas_focus_and_wrong_editor_semantics_refuse_profile() {
    let mut p = profile();
    p.canvas_selector.require_keyboard_focus = false;
    assert_eq!(p.validate(), Err(Error::Invalid));
    p.canvas_selector.require_keyboard_focus = true;
    p.actions[0].action = 5;
    assert_eq!(p.validate(), Err(Error::Invalid));
    p.executable_name = "unknown.exe".into();
    assert_eq!(p.validate(), Err(Error::Unavailable));
}

#[test]
fn finite_shortcut_ipc_roundtrip_preserves_action_and_rejects_unknown_fields() -> Result<()> {
    let command = wire::Command::Finite {
        sequence: 2,
        input_seq: 1,
        action: wire::FiniteAction::Shortcut {
            profile_digest: "a".repeat(64),
            action: 3,
            focus_runtime_hash: "b".repeat(64),
        },
    };
    let bytes = serde_json::to_vec(&command).map_err(|_| Error::Invalid)?;
    let decoded: wire::Command = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
    match decoded {
        wire::Command::Finite {
            sequence,
            input_seq,
            action:
                wire::FiniteAction::Shortcut {
                    action,
                    profile_digest,
                    focus_runtime_hash,
                },
        } => {
            assert_eq!((sequence, input_seq, action), (2, 1, 3));
            assert_eq!(profile_digest, "a".repeat(64));
            assert_eq!(focus_runtime_hash, "b".repeat(64));
        }
        _ => return Err(Error::Invalid),
    }
    let mut value = serde_json::to_value(&command).map_err(|_| Error::Invalid)?;
    assert_eq!(value["action"]["kind"], "Shortcut");
    value["action"]["unexpected"] = true.into();
    assert!(serde_json::from_value::<wire::Command>(value).is_err());
    Ok(())
}
