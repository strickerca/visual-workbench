use std::io::Cursor;
use vw_capture::{
    Error, Rect,
    picker::{MonitorTarget, PickerSource},
    stream::StreamRequest,
    stream_process::validate_reply,
    stream_wire::{self, Command, Header, MAX_HEADER_BYTES},
};
use vw_stream::{Cancellation, SessionBudget};

fn request() -> StreamRequest {
    StreamRequest {
        explicit_owner_action: true,
        owner_process_id: 9,
        capture_session_id: vw_model::Id::from_parts(1_700_000_000_000, [1; 10])
            .expect("id")
            .to_string(),
        connection_epoch: 13,
        source_generation: 17,
        source: PickerSource::Monitor {
            target: MonitorTarget {
                monitor: 1,
                device: "fixture-monitor".into(),
                physical: Rect {
                    x: -512,
                    y: 20,
                    width: 512,
                    height: 512,
                },
                dpi: 96,
                rotation: 0,
                primary: true,
                observed_ns: 100,
            },
        },
        geometry_revision: 2,
        target_fps: 15,
        endpoint_bytes: 128 * 1024 * 1024,
        ring_bytes: 16 * 1024 * 1024,
    }
}
#[test]
fn bounded_pipe_rejects_oversize_truncation_and_unknown_command() {
    let budget = SessionBudget::new(128 * 1024 * 1024).expect("budget");
    let cancel = Cancellation::default();
    let mut oversized = Cursor::new(((MAX_HEADER_BYTES + 1) as u32).to_le_bytes().to_vec());
    assert!(matches!(
        stream_wire::read_command(&mut oversized, &budget, &cancel),
        Err(Error::Limit)
    ));
    assert_eq!(0, budget.used_bytes());
    let mut truncated = Cursor::new([10u32.to_le_bytes().as_slice(), b"abc"].concat());
    assert!(matches!(
        stream_wire::read_command(&mut truncated, &budget, &cancel),
        Err(Error::Platform)
    ));
    assert_eq!(0, budget.used_bytes());
    let bad = br#"{"command":"poll","sequence":1,"connection_epoch":13,"source_generation":17,"feedback":null,"repair_keyframe":false,"new_grant":true}"#;
    let mut bytes = (bad.len() as u32).to_le_bytes().to_vec();
    bytes.extend_from_slice(bad);
    assert!(matches!(
        stream_wire::read_command(&mut Cursor::new(bytes), &budget, &cancel),
        Err(Error::Invalid)
    ));
}
#[test]
fn epoch_source_sequence_and_payload_are_not_self_issued_grants() {
    let request = request();
    let budget = SessionBudget::new(request.endpoint_bytes).expect("budget");
    let cancel = Cancellation::default();
    let command = Command::Poll {
        sequence: 4,
        connection_epoch: 13,
        source_generation: 17,
        feedback: None,
        repair_keyframe: false,
    };
    let mut wire = Vec::new();
    stream_wire::write_command(&mut wire, &command, &budget, &cancel).expect("write command");
    let decoded =
        stream_wire::read_command(&mut Cursor::new(wire), &budget, &cancel).expect("read command");
    assert_eq!(decoded.command.sequence(), Some(4));
    drop(decoded);
    let good = Header::Idle {
        sequence: 4,
        connection_epoch: 13,
        source_generation: 17,
        retained_frames: 2,
        retained_bytes: 4096,
    };
    let mut reply = Vec::new();
    stream_wire::write_packet(&mut reply, &good, &[], &budget, &cancel).expect("write reply");
    let packet =
        stream_wire::read_packet(&mut Cursor::new(reply), &budget, &cancel).expect("read reply");
    validate_reply(&request, &command, &packet).expect("exact reply");
    drop(packet);
    for wrong in [
        Header::Idle {
            sequence: 5,
            connection_epoch: 13,
            source_generation: 17,
            retained_frames: 2,
            retained_bytes: 4096,
        },
        Header::Idle {
            sequence: 4,
            connection_epoch: 14,
            source_generation: 17,
            retained_frames: 2,
            retained_bytes: 4096,
        },
        Header::Idle {
            sequence: 4,
            connection_epoch: 13,
            source_generation: 18,
            retained_frames: 2,
            retained_bytes: 4096,
        },
        Header::Idle {
            sequence: 4,
            connection_epoch: 13,
            source_generation: 17,
            retained_frames: 9,
            retained_bytes: 4096,
        },
    ] {
        let mut bytes = Vec::new();
        stream_wire::write_packet(&mut bytes, &wrong, &[], &budget, &cancel).expect("write");
        let packet =
            stream_wire::read_packet(&mut Cursor::new(bytes), &budget, &cancel).expect("read");
        assert!(matches!(
            validate_reply(&request, &command, &packet),
            Err(Error::Invalid)
        ));
    }
    let mut no_payload = Vec::new();
    assert!(matches!(
        stream_wire::write_packet(&mut no_payload, &good, b"unexpected", &budget, &cancel),
        Err(Error::Invalid)
    ));
    let oversized_media = Header::Tiles {
        sequence: 4,
        connection_epoch: 13,
        source_generation: 17,
        frame_id: 1,
        timestamp_ns: 200,
        captured_at_ms: 1_700_000_000_000,
        keyframe: true,
        payload_bytes: (vw_stream::MAX_TILE_BYTES + 64 * 1024 + 1) as u32,
    };
    assert!(matches!(oversized_media.payload_len(), Err(Error::Limit)));
    assert_eq!(0, budget.used_bytes());
}
