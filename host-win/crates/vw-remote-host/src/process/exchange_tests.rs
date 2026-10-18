use super::*;
use std::cell::Cell;
use vw_remote::{Binding, Rect, Scope, Target, wire::FiniteAction};

fn binding() -> Binding {
    Binding {
        scope: Scope {
            connection_epoch: 1,
            capture_session_id: "00000000-0000-7000-8000-000000000001".into(),
            source_generation: 1,
            target_token: "00000000-0000-7000-8000-000000000002".into(),
            geometry_revision: 1,
        },
        input_session_id: "00000000-0000-7000-8000-000000000003".into(),
    }
}
fn finite_request() -> Command {
    Command::Finite {
        sequence: 2,
        input_seq: 10,
        action: FiniteAction::Click {
            x: 10,
            y: 10,
            button: 1,
        },
    }
}
fn injected() -> Packet {
    Packet {
        header: Header::Injected {
            sequence: 2,
            binding: binding(),
            input_seq: 10,
            accepted_qpc_100ns: 100,
        },
        payload: Vec::new(),
    }
}
fn finite_fence() -> Result<finite::Fence> {
    let mut fence = finite::Fence::with_binding(Some(binding()));
    fence.begin(2, 10)?;
    Ok(fence)
}
fn target() -> Target {
    let rect = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 100,
    };
    Target {
        token: binding().scope.target_token,
        window: 1,
        process_id: 2,
        thread_id: 3,
        process_created: 4,
        window_rect: rect,
        frame_rect: rect,
        client_rect: rect,
        dpi: 96,
        integrity: 0,
    }
}

#[test]
fn only_bound_paint_provider_commands_receive_the_larger_parent_deadline() {
    let paint_digest = "a".repeat(64);
    let wrong_digest = "c".repeat(64);
    let shortcut = Command::Finite {
        sequence: 2,
        input_seq: 10,
        action: FiniteAction::Shortcut {
            profile_digest: "a".repeat(64),
            action: 3,
            focus_runtime_hash: "b".repeat(64),
        },
    };
    let refresh = Command::RefreshAuthority {
        sequence: 2,
        profile_digest: "a".repeat(64),
    };
    for request in [shortcut, refresh] {
        assert_eq!(
            exchange_timeout(&request, Some(&paint_digest)),
            Duration::from_millis(1000)
        );
        for known in [None, Some(wrong_digest.as_str())] {
            assert_eq!(
                exchange_timeout(&request, known),
                Duration::from_millis(250)
            );
        }
    }
    let wheel = Command::Finite {
        sequence: 2,
        input_seq: 10,
        action: FiniteAction::Wheel {
            x: 10,
            y: 10,
            delta: 120,
        },
    };
    let pen = Command::Pen {
        sequence: 2,
        input_seq: 10,
        sample: wire::PenSample {
            phase: wire::Phase::Up,
            x: 10,
            y: 10,
            pressure: 0,
            tilt_x: 0,
            tilt_y: 0,
            rotation: 0,
            pen_flags: 0,
        },
    };
    for request in [finite_request(), wheel, pen, Command::Check { sequence: 2 }] {
        for known in [None, Some(paint_digest.as_str())] {
            assert_eq!(
                exchange_timeout(&request, known),
                Duration::from_millis(250)
            );
        }
    }
    for request in [
        Command::Poll {
            sequence: 2,
            force_keyframe: true,
        },
        Command::Stop { sequence: 2 },
    ] {
        assert_eq!(
            exchange_timeout(&request, Some(&paint_digest)),
            Duration::from_millis(6000)
        );
    }
}

#[test]
fn paint_parent_budget_requires_native_ready_target_catalog_and_identity() -> Result<()> {
    use vw_remote::profile::{
        Authority, AuthorityRoutes, CanvasProof, ImageIdentity, PackagedProfile,
    };
    let digest = "a".repeat(64);
    let target = target();
    let authority = Authority {
        identity: ImageIdentity {
            executable_name: "mspaint.exe".into(),
            executable_blake3: "b2a7f30ffa5f4864fbe63b49c64153f1d75a278f5771f14e59ffb6be4f65792f"
                .into(),
            executable_bytes: 4_272_128,
            file_version: Some("11.2605.81.0".into()),
            package_full_name: Some(vw_remote::profile::essential::PAINT_PACKAGE.into()),
            package_version: Some("11.2605.81.0".into()),
        },
        profile_digest: digest.clone(),
        verified_actions: vec![3],
        routes: AuthorityRoutes::Essential,
        focus: Some(CanvasProof {
            runtime_id_hash: "b".repeat(64),
            process_id: target.process_id,
            sampled_qpc_100ns: 1,
            class_name: "NamedContainerAutomationPeer".into(),
            control_type: 50026,
            canvas_rect: target.client_rect,
            profile_digest: digest.clone(),
        }),
        selected_canvas: None,
    };
    authority.focused_canvas()?;
    let request = Command::OpenInput {
        sequence: 1,
        target: target.clone(),
        binding: binding(),
        owner_pid: 99,
        // This unit boundary begins after the pinned helper has validated the
        // catalog bytes; only its exact digest and native Ready are adopted here.
        profile: Some(PackagedProfile {
            digest: digest.clone(),
            json: String::new(),
        }),
    };
    let ready = |value| Packet {
        header: Header::Ready {
            sequence: 1,
            capabilities: None,
            input_authority: value,
        },
        payload: Vec::new(),
    };
    assert_eq!(
        paint_profile_digest(&request, &ready(Some(authority.clone()))),
        Some(digest)
    );
    assert_eq!(paint_profile_digest(&request, &ready(None)), None);
    assert_eq!(
        paint_profile_digest(&finite_request(), &ready(Some(authority.clone()))),
        None
    );
    for mutation in 0..7 {
        let mut changed = authority.clone();
        match mutation {
            0 => changed.routes = AuthorityRoutes::Legacy,
            1 => changed.profile_digest = "c".repeat(64),
            2 => changed.identity.executable_name = "krita.exe".into(),
            3 => changed.identity.executable_bytes += 1,
            4 => changed.identity.package_version = Some("11.2605.82.0".into()),
            5 => {
                if let Some(canvas) = changed.focus.as_mut() {
                    canvas.process_id += 1;
                }
            }
            _ => {
                changed.selected_canvas = changed.focus.take();
            }
        }
        assert_eq!(paint_profile_digest(&request, &ready(Some(changed))), None);
    }
    let mut no_catalog = request;
    if let Command::OpenInput { profile, .. } = &mut no_catalog {
        *profile = None;
    }
    assert_eq!(
        paint_profile_digest(&no_catalog, &ready(Some(authority))),
        None
    );
    Ok(())
}

#[test]
fn receive_slice_shrinks_without_extending_the_original_deadline() -> Result<()> {
    let start = Instant::now();
    let deadline = start + Duration::from_millis(250);
    let cancel = Cancellation::default();
    assert_eq!(
        exchange_wait(deadline, start, &cancel)?,
        Duration::from_millis(5)
    );
    assert_eq!(
        exchange_wait(deadline, deadline - Duration::from_millis(1), &cancel)?,
        Duration::from_millis(1)
    );
    assert_eq!(
        exchange_wait(deadline, deadline - Duration::from_nanos(1), &cancel)?,
        Duration::from_nanos(1)
    );
    assert_eq!(
        exchange_wait(deadline, deadline, &cancel),
        Err(Error::Timeout)
    );
    assert_eq!(
        exchange_wait(deadline, deadline + Duration::from_nanos(1), &cancel),
        Err(Error::Timeout)
    );
    Ok(())
}

#[test]
fn cancellation_refuses_another_wait_even_before_the_deadline() {
    let now = Instant::now();
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        exchange_wait(now + Duration::from_secs(1), now, &cancel),
        Err(Error::Cancelled)
    );
}

#[test]
fn on_time_injected_reply_is_accepted_and_clears_real_finite_uncertainty() -> Result<()> {
    let now = Instant::now();
    let mut fence = finite_fence()?;
    admit_reply(
        &finite_request(),
        &injected(),
        |p| fence.observe(&p.header),
        || {
            exchange_wait(
                now + Duration::from_millis(250),
                now,
                &Cancellation::default(),
            )
            .map(|_| ())
        },
    )?;
    assert!(!fence.pending());
    Ok(())
}

#[test]
fn late_injected_reply_preserves_consumed_completion_but_cannot_succeed() -> Result<()> {
    let deadline = Instant::now();
    let mut fence = finite_fence()?;
    let result = admit_reply(
        &finite_request(),
        &injected(),
        |p| fence.observe(&p.header),
        || exchange_wait(deadline, deadline, &Cancellation::default()).map(|_| ()),
    );
    assert_eq!(result, Err(Error::Timeout));
    assert!(!fence.pending());
    Ok(())
}

#[test]
fn reply_observation_crossing_deadline_uses_completion_time_not_wake_time() -> Result<()> {
    let deadline = Instant::now();
    let clock = Cell::new(deadline - Duration::from_nanos(1));
    let cancel = Cancellation::default();
    exchange_wait(deadline, clock.get(), &cancel)?;
    let mut fence = finite_fence()?;
    let result = admit_reply(
        &finite_request(),
        &injected(),
        |p| {
            fence.observe(&p.header)?;
            clock.set(deadline + Duration::from_nanos(1));
            Ok(())
        },
        || exchange_wait(deadline, clock.get(), &cancel).map(|_| ()),
    );
    assert_eq!(result, Err(Error::Timeout));
    assert!(!fence.pending());
    Ok(())
}

#[test]
fn cancellation_during_reply_observation_retains_completion_and_refuses_success() -> Result<()> {
    let now = Instant::now();
    let cancel = Cancellation::default();
    let mut fence = finite_fence()?;
    let result = admit_reply(
        &finite_request(),
        &injected(),
        |p| {
            fence.observe(&p.header)?;
            cancel.cancel();
            Ok(())
        },
        || exchange_wait(now + Duration::from_millis(250), now, &cancel).map(|_| ()),
    );
    assert_eq!(result, Err(Error::Cancelled));
    assert!(!fence.pending());
    Ok(())
}

#[test]
fn late_pen_up_reply_cannot_infer_scoped_device_retirement() -> Result<()> {
    let request = Command::Pen {
        sequence: 2,
        input_seq: 10,
        sample: wire::PenSample {
            phase: wire::Phase::Up,
            x: 10,
            y: 10,
            pressure: 0,
            tilt_x: 0,
            tilt_y: 0,
            rotation: 0,
            pen_flags: 0,
        },
    };
    let mut finite = finite::Fence::with_binding(Some(binding()));
    let mut input = input_retirement::Fence::new(binding());
    let deadline = Instant::now();
    assert_eq!(
        admit_reply(
            &request,
            &injected(),
            |p| {
                finite.observe(&p.header)?;
                input.observe(&p.header);
                Ok(())
            },
            || exchange_wait(deadline, deadline, &Cancellation::default()).map(|_| ())
        ),
        Err(Error::Timeout)
    );
    assert!(!finite.pending());
    assert!(input.pending());
    assert!(input.witness().is_none());
    input.sent_stop(3)?;
    input.observe(&Header::Stopped {
        sequence: 3,
        pen_release: None,
    });
    assert!(!input.pending());
    Ok(())
}

#[test]
fn wrong_sequence_or_reply_kind_cannot_observe_or_clear_the_fence() -> Result<()> {
    let cases = [
        Header::Idle { sequence: 2 },
        Header::Injected {
            sequence: 3,
            binding: binding(),
            input_seq: 10,
            accepted_qpc_100ns: 100,
        },
    ];
    for header in cases {
        let observed = Cell::new(false);
        let checked = Cell::new(false);
        let mut fence = finite_fence()?;
        assert_eq!(
            admit_reply(
                &finite_request(),
                &Packet {
                    header,
                    payload: Vec::new()
                },
                |p| {
                    observed.set(true);
                    fence.observe(&p.header)
                },
                || {
                    checked.set(true);
                    Ok(())
                }
            ),
            Err(Error::Invalid)
        );
        assert!(!observed.get());
        assert!(!checked.get());
        assert!(fence.pending());
    }
    Ok(())
}

#[test]
fn malformed_injected_clock_keeps_actual_finite_owner_pending() -> Result<()> {
    let mut packet = injected();
    if let Header::Injected {
        accepted_qpc_100ns, ..
    } = &mut packet.header
    {
        *accepted_qpc_100ns = 0;
    }
    let mut fence = finite_fence()?;
    let checked = Cell::new(false);
    assert_eq!(
        admit_reply(
            &finite_request(),
            &packet,
            |p| fence.observe(&p.header),
            || {
                checked.set(true);
                Ok(())
            }
        ),
        Err(Error::Invalid)
    );
    assert!(fence.pending());
    assert!(!checked.get());
    Ok(())
}

#[test]
fn late_refusal_preserves_primary_error_and_real_complete_or_pending_state() -> Result<()> {
    for retirement in [
        wire::FiniteRetirement::Complete,
        wire::FiniteRetirement::Pending {
            held_count: 1,
            uncertain: false,
            error: Error::RetirementPending,
        },
    ] {
        let pending = !matches!(retirement, wire::FiniteRetirement::Complete);
        let mut fence = finite_fence()?;
        let checked = Cell::new(false);
        let packet = Packet {
            header: Header::Refused {
                sequence: 2,
                error: Error::PartialInput,
                finite_attempt: Some(wire::FiniteAttempt {
                    expected_count: 3,
                    accepted_count: 2,
                    accepted_qpc_100ns: Some(100),
                    error: Some(Error::PartialInput),
                    retirement,
                }),
            },
            payload: Vec::new(),
        };
        assert_eq!(
            admit_reply(
                &finite_request(),
                &packet,
                |p| fence.observe(&p.header),
                || {
                    checked.set(true);
                    Err(Error::Timeout)
                }
            ),
            Err(Error::PartialInput)
        );
        assert_eq!(fence.pending(), pending);
        assert!(!checked.get());
    }
    Ok(())
}

#[test]
fn late_stop_retains_actual_finite_and_pen_release_without_success_admission() -> Result<()> {
    let mut finite = finite_fence()?;
    finite.sent_stop(3)?;
    let mut input = input_retirement::Fence::new(binding());
    input.sent_stop(3)?;
    let packet = Packet {
        header: Header::Stopped {
            sequence: 3,
            pen_release: Some(wire::PenReleaseWitness {
                binding: binding(),
                held_before_release: true,
                release_started_qpc: 100,
                release_completed_qpc: 101,
                qpc_frequency: 10_000_000,
            }),
        },
        payload: Vec::new(),
    };
    let deadline = Instant::now();
    assert_eq!(
        admit_reply(
            &Command::Stop { sequence: 3 },
            &packet,
            |p| {
                finite.observe(&p.header)?;
                input.observe(&p.header);
                Ok(())
            },
            || exchange_wait(deadline, deadline, &Cancellation::default()).map(|_| ())
        ),
        Err(Error::Timeout)
    );
    assert!(!finite.pending());
    assert!(!input.pending());
    assert_eq!(input.witness().map(|w| w.release_completed_qpc), Some(101));
    Ok(())
}

#[test]
fn ready_check_poll_and_authority_replies_share_the_post_receive_deadline_gate() {
    let pairs = [
        (
            Command::OpenInput {
                sequence: 1,
                target: target(),
                binding: binding(),
                owner_pid: 5,
                profile: None,
            },
            Header::Ready {
                sequence: 1,
                capabilities: None,
                input_authority: None,
            },
        ),
        (
            Command::OpenVideo {
                sequence: 1,
                target: target(),
                scope: binding().scope,
                owner_pid: 5,
            },
            Header::Ready {
                sequence: 1,
                capabilities: None,
                input_authority: None,
            },
        ),
        (Command::Check { sequence: 2 }, Header::Idle { sequence: 2 }),
        (
            Command::Poll {
                sequence: 2,
                force_keyframe: false,
            },
            Header::Idle { sequence: 2 },
        ),
        (
            Command::RefreshAuthority {
                sequence: 2,
                profile_digest: "a".repeat(64),
            },
            Header::Authority {
                sequence: 2,
                binding: binding(),
                authority: None,
            },
        ),
    ];
    for (request, header) in pairs {
        let observed = Cell::new(false);
        let deadline = Instant::now();
        assert_eq!(
            admit_reply(
                &request,
                &Packet {
                    header,
                    payload: Vec::new()
                },
                |_| {
                    observed.set(true);
                    Ok(())
                },
                || exchange_wait(deadline, deadline, &Cancellation::default()).map(|_| ())
            ),
            Err(Error::Timeout)
        );
        assert!(observed.get());
    }
}
