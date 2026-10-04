#![allow(clippy::unwrap_used, clippy::panic)]
use super::*;
use vw_model::{DeviceId, Document, Layer, Project};

fn id(n: u8) -> Id {
    Id::from_parts(1_700_000_000_000, [n; 10]).unwrap()
}
fn pending(marker: Option<u8>) -> Pending {
    Pending {
        binding: WorkflowBinding {
            project_id: id(1).to_string(),
            document_id: id(2).to_string(),
            host_seq: 7,
            state_hash: "ab".repeat(32),
        },
        marker: marker.map(id),
    }
}
fn queue(hub: &FocusHub, epoch: u64, value: Pending) -> WorkflowResult<()> {
    hub.queue(epoch, hub.issue(epoch)?, value)
}
#[test]
fn minor_three_control_roundtrip_has_a_bounded_exact_wire_binding() {
    assert_eq!(vw_net::PROTOCOL_MINOR, 4);
    let value = wire(&pending(Some(4)), 1).unwrap();
    assert!(value.encoded_len() <= MAX_BYTES);
    assert_eq!(
        vw_net::channel_for(&Body::MarkerFocus(value.clone())),
        pb::Channel::Control
    );
    let decoded = pb::MarkerFocus::decode(value.encode_to_vec().as_slice()).unwrap();
    let result = parse(&id(1), decoded).unwrap();
    assert_eq!(result.binding, pending(Some(4)).binding);
    assert_eq!(result.marker, Some(id(4)));
}
#[test]
fn ten_thousand_inputs_have_one_latest_outbound_and_no_lifetime_identity_cap() {
    let hub = FocusHub::new(id(1));
    let _epoch = hub.activate(1).unwrap();
    let start = Instant::now();
    for i in 1..=10_000u64 {
        let mut value = pending(Some(4));
        value.binding.host_seq = i;
        queue(&hub, 1, value).unwrap();
    }
    let mut sent = None;
    hub.flush_at(start + SEND_INTERVAL, |v| {
        sent = Some(v);
        Ok(())
    })
    .unwrap();
    let sent = sent.unwrap();
    assert_eq!(sent.sequence, 1);
    assert_eq!(sent.revision.unwrap().host_seq, 10_000);
    assert!(hub.state.lock().unwrap().outbound.is_none());
    for i in 1..=10_000u64 {
        let now = start + Duration::from_millis(i * 50);
        hub.receive_at(wire(&pending(Some(4)), i).unwrap(), 1, now)
            .unwrap();
    }
    assert_eq!(
        hub.snapshot(1, start + Duration::from_secs(500))
            .unwrap()
            .unwrap()
            .sequence,
        10_000
    );
}
#[test]
fn rate_limited_output_keeps_latest_and_control_backpressure_does_not_drop_it() {
    let hub = FocusHub::new(id(1));
    let _epoch = hub.activate(1).unwrap();
    let now = Instant::now();
    queue(&hub, 1, pending(Some(4))).unwrap();
    hub.flush_at(now, |_| Err(vw_net::NetError::Backpressure))
        .unwrap();
    assert!(hub.state.lock().unwrap().outbound.is_some());
    queue(&hub, 1, pending(None)).unwrap();
    hub.flush_at(now + Duration::from_millis(49), |_| panic!("too early"))
        .unwrap();
    hub.flush_at(now + SEND_INTERVAL, |v| {
        assert!(v.marker_id.is_none());
        assert_eq!(v.sequence, 1);
        Ok(())
    })
    .unwrap();
}
#[test]
fn stale_packets_are_identity_validated_and_cannot_clear_newer_selection() {
    let hub = FocusHub::new(id(1));
    let _epoch = hub.activate(1).unwrap();
    let now = Instant::now();
    hub.receive_at(wire(&pending(Some(4)), 2).unwrap(), 1, now)
        .unwrap();
    hub.receive_at(wire(&pending(None), 1).unwrap(), 1, now)
        .unwrap();
    assert_eq!(
        hub.snapshot(1, now).unwrap().unwrap().value.marker,
        Some(id(4))
    );
    let mut forged = wire(&pending(None), 1).unwrap();
    forged.project_id = Some(id(9).to_proto());
    assert!(matches!(
        hub.receive_at(forged, 1, now),
        Err(SessionError::Authentication)
    ));
    let mut malformed = wire(&pending(None), 1).unwrap();
    malformed.revision.as_mut().unwrap().state_hash.pop();
    assert!(matches!(
        hub.receive_at(malformed, 1, now),
        Err(SessionError::Invalid)
    ));
}
#[test]
fn clear_expiry_and_newer_signal_fence_late_queries() {
    let hub = FocusHub::new(id(1));
    let _epoch = hub.activate(1).unwrap();
    let now = Instant::now();
    hub.receive_at(wire(&pending(Some(4)), 1).unwrap(), 1, now)
        .unwrap();
    assert!(hub.unchanged(1, 1, now).unwrap());
    hub.receive_at(wire(&pending(None), 2).unwrap(), 1, now)
        .unwrap();
    assert!(!hub.unchanged(1, 1, now).unwrap());
    assert!(
        hub.snapshot(1, now)
            .unwrap()
            .unwrap()
            .value
            .marker
            .is_none()
    );
    let before = hub.signal.borrow().sequence;
    hub.flush_at(now + TTL, |_| panic!("no outbound")).unwrap();
    assert!(hub.signal.borrow().sequence > before);
    assert!(hub.snapshot(1, now + TTL).unwrap().is_none());
}
#[test]
fn reconnect_refuses_old_inflight_admission_and_resets_only_with_owned_epoch() {
    let hub = FocusHub::new(id(1));
    let first = hub.activate(1).unwrap();
    let captured = 1;
    queue(&hub, captured, pending(Some(4))).unwrap();
    drop(first);
    let _second = hub.activate(3).unwrap();
    assert!(matches!(
        queue(&hub, captured, pending(Some(5))),
        Err(WorkflowError::Stale)
    ));
    assert!(matches!(
        hub.snapshot(captured, Instant::now()),
        Err(WorkflowError::Stale)
    ));
    assert!(hub.state.lock().unwrap().outbound.is_none());
    assert!(!hub.signal.borrow().available || hub.signal.borrow().connection_epoch == 3);
    hub.stop();
    assert!(hub.activate(5).is_err());
    assert!(matches!(
        queue(&hub, 3, pending(None)),
        Err(WorkflowError::Closed)
    ));
}
#[test]
fn old_call_paused_before_worker_admission_cannot_replace_newer_local_selection() {
    let hub = FocusHub::new(id(1));
    let _epoch = hub.activate(1).unwrap();
    let old = hub.issue(1).unwrap();
    let newer = hub.issue(1).unwrap();
    hub.queue(1, newer, pending(Some(5))).unwrap();
    assert!(matches!(
        hub.queue(1, old, pending(Some(4))),
        Err(WorkflowError::Stale)
    ));
    assert_eq!(
        hub.state.lock().unwrap().outbound.as_ref().unwrap().marker,
        Some(id(5))
    );
}
#[test]
fn malformed_inputs_fail_but_lawful_delayed_bursts_keep_newest_without_disconnect() {
    let hub = FocusHub::new(id(1));
    let _epoch = hub.activate(1).unwrap();
    let now = Instant::now();
    let mut huge = wire(&pending(Some(4)), 1).unwrap();
    huge.marker_id.as_mut().unwrap().value.resize(1024, 0);
    assert!(matches!(
        hub.receive_at(huge, 1, now),
        Err(SessionError::Invalid)
    ));
    for i in 1..=40 {
        hub.receive_at(wire(&pending(Some(4)), i).unwrap(), 1, now)
            .unwrap();
    }
    let before = hub.signal.borrow().sequence;
    // A sender at 20/s can accumulate these messages during a carrier stall.
    for i in 41..=200 {
        hub.receive_at(wire(&pending(None), i).unwrap(), 1, now)
            .unwrap();
    }
    assert_eq!(hub.signal.borrow().sequence, before);
    assert_eq!(hub.snapshot(1, now).unwrap().unwrap().sequence, 200);
    assert!(hub.state.lock().unwrap().pending_signal);
    hub.flush_at(now + SEND_INTERVAL, |_| panic!("no outbound"))
        .unwrap();
    assert_eq!(hub.signal.borrow().sequence, before + 1);
    assert!(!hub.state.lock().unwrap().pending_signal);
    assert_eq!(
        hub.snapshot(1, now + SEND_INTERVAL)
            .unwrap()
            .unwrap()
            .sequence,
        200
    );
}
#[test]
fn snapshot_before_tick_publishes_exactly_one_expiry_even_with_deferred_notification() {
    let hub = FocusHub::new(id(1));
    let _epoch = hub.activate(1).unwrap();
    let now = Instant::now();
    for i in 1..=41 {
        hub.receive_at(wire(&pending(Some(4)), i).unwrap(), 1, now)
            .unwrap();
    }
    assert!(hub.state.lock().unwrap().pending_signal);
    let before = hub.signal.borrow().sequence;
    // Caller may discard/cancel the result: watch observers still see expiry.
    assert!(hub.snapshot(1, now + TTL).unwrap().is_none());
    assert_eq!(hub.signal.borrow().sequence, before + 1);
    assert!(!hub.state.lock().unwrap().pending_signal);
    hub.flush_at(now + TTL, |_| panic!("no outbound")).unwrap();
    assert!(hub.snapshot(1, now + TTL).unwrap().is_none());
    assert_eq!(hub.signal.borrow().sequence, before + 1);
}
fn fixture() -> vw_ops::HostSequencer {
    let device = DeviceId::from_bytes([1; 16]);
    let mut project = Project::new(id(1), "Focus fixture".into(), device.clone());
    project.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(2).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Canvas".into(),
                ..Default::default()
            },
            pages: vec![],
            created_at_ms: 1000,
        },
    );
    project.layers.insert(
        id(3),
        Layer {
            definition: pb::CreateLayer {
                layer_id: Some(id(3).to_proto()),
                document_id: Some(id(2).to_proto()),
                page_index: -1,
                name: "Marks".into(),
                kind: "annotation".into(),
                order_key: "V".into(),
            },
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: "normal".into(),
        },
    );
    let mut host = vw_ops::HostSequencer::new(project, device.clone()).unwrap();
    let view =
        vw_instructions::DocumentView::new(host.project(), &host.revision().unwrap(), &id(2))
            .unwrap();
    let plan = view
        .place_marker(
            vw_instructions::EditMetadata {
                transaction_id: id(8),
                device: device.clone(),
                first_lamport: 1,
                created_at_ms: 1001,
            },
            vw_instructions::MarkerPlacement {
                object_id: id(4),
                instruction_id: id(5),
                layer_id: id(3),
                point: pb::PointD { x: 10.0, y: 20.0 },
                bounds: None,
                element_eids: vec![],
                style: pb::Style {
                    stroke: Some(pb::Color { rgba: 0x112233ff }),
                    width: 2.0,
                    ..Default::default()
                },
                role: vw_instructions::Role::Change,
                text: "Literal fixture".into(),
                entry_method: vw_instructions::EntryMethod::PcKeyboard,
                language: "en-US".into(),
            },
        )
        .unwrap();
    plan.submit(&mut host, &device, 1002).unwrap();
    host
}
fn binding(project: &Project, host_seq: u64) -> WorkflowBinding {
    WorkflowBinding {
        project_id: project.id.to_string(),
        document_id: id(2).to_string(),
        host_seq,
        state_hash: project.state_hash().unwrap().to_string(),
    }
}
#[test]
fn target_uses_actual_canonical_marker_links_and_rejects_deleted_or_wrong_document() {
    let host = fixture();
    let original = host.project();
    let expected = binding(original, host.revision().unwrap().host_seq);
    assert_eq!(
        target(original, &expected, &id(4)).unwrap(),
        id(5).to_string()
    );
    assert!(target(original, &expected, &id(5)).is_err());
    let mut wrong = expected.clone();
    wrong.document_id = id(9).to_string();
    assert!(target(original, &wrong, &id(4)).is_err());
    let mut removed = original.clone();
    removed.objects.remove(&id(4));
    assert!(matches!(
        target(&removed, &expected, &id(4)),
        Err(WorkflowError::Stale)
    ));
    assert!(matches!(
        target(&removed, &binding(&removed, expected.host_seq), &id(4)),
        Err(WorkflowError::Missing)
    ));
    let mut duplicate = original.clone();
    let mut extra = duplicate.instructions[&id(5)].clone();
    extra.definition.instruction_id = Some(id(6).to_proto());
    duplicate.instructions.insert(id(6), extra);
    assert!(matches!(
        target(&duplicate, &binding(&duplicate, expected.host_seq), &id(4)),
        Err(WorkflowError::Reconciliation)
    ));
}
#[test]
fn focus_before_ops_remains_only_a_hint_until_exact_visible_revision_catches_up() {
    let host = fixture();
    let accepted = binding(host.project(), host.revision().unwrap().host_seq);
    let hub = FocusHub::new(id(1));
    let _epoch = hub.activate(1).unwrap();
    let now = Instant::now();
    hub.receive_at(
        wire(
            &Pending {
                binding: accepted.clone(),
                marker: Some(id(4)),
            },
            1,
        )
        .unwrap(),
        1,
        now,
    )
    .unwrap();
    let mut prior = accepted.clone();
    prior.host_seq -= 1;
    let pending = hub.snapshot(1, now).unwrap().unwrap();
    assert_ne!(pending.value.binding, prior);
    assert_eq!(pending.value.binding, accepted);
    assert_eq!(
        target(host.project(), &accepted, &id(4)).unwrap(),
        id(5).to_string()
    );
    let mut optimistic = accepted.clone();
    optimistic.state_hash = "cd".repeat(32);
    assert_ne!(pending.value.binding, optimistic);
    assert!(matches!(
        target(host.project(), &optimistic, &id(4)),
        Err(WorkflowError::Stale)
    ));
}
