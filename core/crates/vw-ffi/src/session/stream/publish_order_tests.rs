#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::super::receiver_tests::{BUDGET, id, packet, source};
use super::*;

fn prepared(n: u64, budget: &ByteBudget) -> PreparedFrame {
    let mut value = packet(n, 1);
    value.delivery_sequence = 0;
    value.source_generation = 0;
    PreparedFrame::decode(value.encode_to_vec(), budget).unwrap()
}
fn request(n: u64) -> pb::FreezeRequest {
    pb::FreezeRequest {
        capture_session_id: Some(id(1).to_proto()),
        frame_id: n,
        request_id: Some(id(8).to_proto()),
        geometry_revision: 1,
        source_generation: 1,
    }
}

#[test]
fn local_packet_admission_refuses_network_authority_spare_capacity_and_malformed_repeated_fields() {
    let budget = ByteBudget::new(BUDGET).unwrap();
    let value = packet(1, 1);
    assert!(PreparedFrame::decode(value.encode_to_vec(), &budget).is_err());
    assert_eq!(budget.used(), 0);
    let mut bytes = Vec::with_capacity(vw_proto::capture_admission::MAX_MEDIA_BYTES + 1);
    bytes.push(0);
    assert!(PreparedFrame::decode(bytes, &budget).is_err());
    assert_eq!(budget.used(), 0);
    assert!(PreparedFrame::decode(vec![0x3a, 0xff, 0xff, 0xff, 0xff, 0xff], &budget).is_err());
    assert_eq!(budget.used(), 0);
    let frame = prepared(1, &budget);
    assert!(budget.used() > 0);
    drop(frame);
    assert_eq!(budget.used(), 0);
}
#[test]
fn source_open_precedes_media_and_declined_admission_preserves_captured_identity() {
    let budget = ByteBudget::new(BUDGET).unwrap();
    let mut order = PublishOrder::new(source(1), &budget).unwrap();
    assert!(
        !order
            .publish(prepared(1, &budget), |_| Err(
                vw_net::NetError::Backpressure
            ))
            .unwrap()
    );
    assert_eq!(order.delivery, 0);
    let mut output = Vec::new();
    assert!(
        order
            .publish(prepared(1, &budget), |body| {
                output.push(body);
                Ok(())
            })
            .unwrap()
    );
    assert!(matches!(&output[0],Body::StreamControl(value)if value.action=="start"));
    let Body::FrameTiles(first) = &output[1] else {
        panic!("missing media")
    };
    assert_eq!(
        (
            first.frame_id,
            first.delivery_sequence,
            first.source_generation
        ),
        (1, 1, 1)
    );
    assert!(
        !order
            .publish(prepared(2, &budget), |_| Err(
                vw_net::NetError::Backpressure
            ))
            .unwrap()
    );
    output.clear();
    assert!(
        order
            .publish(prepared(2, &budget), |body| {
                output.push(body);
                Ok(())
            })
            .unwrap()
    );
    let Body::FrameTiles(repair) = &output[0] else {
        panic!("missing repair")
    };
    assert_eq!((repair.frame_id, repair.delivery_sequence), (2, 3));
    assert_eq!(repair.timestamp_ns, packet(2, 1).timestamp_ns);
    assert_eq!(repair.captured_at_ms, packet(2, 1).captured_at_ms);
}
#[test]
fn same_frame_repair_cannot_change_provenance_geometry_or_capture_pixels_mode() {
    let budget = ByteBudget::new(BUDGET).unwrap();
    let mut order = PublishOrder::new(source(1), &budget).unwrap();
    order.publish(prepared(1, &budget), |_| Ok(())).unwrap();
    for change in 0..3 {
        let mut value = packet(1, 1);
        value.delivery_sequence = 0;
        value.source_generation = 0;
        match change {
            0 => value.captured_at_ms += 1,
            1 => {
                value
                    .geometry
                    .as_mut()
                    .unwrap()
                    .client_rect_host
                    .as_mut()
                    .unwrap()
                    .x += 1
            }
            _ => value.keyframe = false,
        }
        let prepared = PreparedFrame::decode(value.encode_to_vec(), &budget).unwrap();
        assert!(
            order
                .publish(prepared, |_| panic!("invalid packet reached queue"))
                .is_err()
        );
    }
    assert_eq!(order.history.len(), 1);
}
#[test]
fn only_admitted_recent_frames_and_exact_source_can_create_one_immutable_freeze_ticket() {
    let budget = ByteBudget::new(BUDGET).unwrap();
    let mut order = PublishOrder::new(source(1), &budget).unwrap();
    order.publish(prepared(1, &budget), |_| Ok(())).unwrap();
    order
        .publish(prepared(2, &budget), |_| {
            Err(vw_net::NetError::Backpressure)
        })
        .unwrap();
    assert!(matches!(
        order.freeze(&request(2), 0).unwrap(),
        FreezeAdmission::Unavailable(_)
    ));
    let mut foreign = request(1);
    foreign.source_generation = 2;
    assert!(matches!(
        order.freeze(&foreign, 0).unwrap(),
        FreezeAdmission::Ignored
    ));
    assert!(order.request().is_none());
    let FreezeAdmission::Request(ticket) = order.freeze(&request(1), 0).unwrap() else {
        panic!("no ticket")
    };
    assert!(order.current_request(&ticket));
    assert!(order.paused());
    let FreezeAdmission::Request(duplicate) = order.freeze(&request(1), 1).unwrap() else {
        panic!("no idempotent ticket")
    };
    assert_eq!(ticket, duplicate);
    let mut other = request(1);
    other.request_id = Some(id(9).to_proto());
    assert!(matches!(
        order.freeze(&other, 2).unwrap(),
        FreezeAdmission::Unavailable(_)
    ));
    order.control(&source(1).control("resume")).unwrap();
    assert!(!order.current_request(&ticket));
    assert!(!order.paused());
    assert!(matches!(
        order.freeze(&other, 100).unwrap(),
        FreezeAdmission::Unavailable(_)
    ));
    assert!(!order.closed());
    let FreezeAdmission::Request(next) = order.freeze(&other, 250).unwrap() else {
        panic!("next request absent")
    };
    assert!(next.sequence > ticket.sequence);
    order.stop();
    assert!(!order.current_request(&next));
    assert!(matches!(
        order.freeze(&other, 500).unwrap(),
        FreezeAdmission::Ignored
    ));
}
#[test]
fn bounded_sent_history_never_authorizes_an_arbitrary_project_asset_or_future_frame() {
    let budget = ByteBudget::new(BUDGET).unwrap();
    let mut order = PublishOrder::new(source(1), &budget).unwrap();
    for n in 1..=64 {
        order.publish(prepared(n, &budget), |_| Ok(())).unwrap();
    }
    assert_eq!(order.history.len(), 32);
    assert_eq!(budget.used(), HISTORY_BYTES);
    for n in [1, 32, 65, u64::MAX] {
        assert!(matches!(
            order.freeze(&request(n), 1000).unwrap(),
            FreezeAdmission::Unavailable(_)
        ));
    }
    assert!(matches!(
        order.freeze(&request(64), 1000).unwrap(),
        FreezeAdmission::Request(_)
    ));
    drop(order);
    assert_eq!(budget.used(), 0);
}
