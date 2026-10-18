#![allow(clippy::unwrap_used)] // Malformed fixture setup must fail its test immediately.
use std::io::Cursor;
use vw_document_protocol::*;

fn request() -> Request {
    Request {
        limits: JobLimits {
            input_bytes: 64 * 1024 * 1024,
            output_bytes: 64 * 1024 * 1024,
            memory_bytes: 128 * 1024 * 1024,
        },
        operation: Operation::PdfInfo,
        password: None,
        export_identity: None,
    }
}
fn binding(source: &[u8]) -> JobBinding {
    JobBinding {
        nonce: [7; 16],
        source_blake3: *blake3::hash(source).as_bytes(),
    }
}
fn job_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    write_job(&mut bytes, binding(b"%PDF-1.7"), &request(), b"%PDF-1.7").unwrap();
    bytes
}
fn reply_bytes(data: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut out = ReplyWriter::new(&mut bytes, binding(b"input"), 1024 * 1024).unwrap();
    out.metadata(&serde_json::json!({"kind":"synthetic"}))
        .unwrap();
    out.data(data).unwrap();
    out.finish().unwrap();
    bytes
}
fn receive(bytes: &[u8], out: &mut Vec<u8>) -> ProtocolResult<Completion> {
    receive_reply(
        &mut Cursor::new(bytes),
        out,
        binding(b"input"),
        1024 * 1024,
        &mut |metadata| {
            if serde_json::from_slice::<serde_json::Value>(metadata)
                .map_err(|_| ProtocolError::Invalid)?
                != serde_json::json!({"kind":"synthetic"})
            {
                return Err(ProtocolError::Invalid);
            }
            Ok(())
        },
        &mut |_| {},
    )
}

#[test]
fn bounded_source_roundtrips_and_debug_never_prints_a_password() {
    let mut req = request();
    req.password = Some("synthetic-secret-password".to_owned());
    let mut bytes = Vec::new();
    write_job(&mut bytes, binding(b"%PDF-1.7"), &req, b"%PDF-1.7").unwrap();
    let parsed = read_job(&mut Cursor::new(bytes)).unwrap();
    assert_eq!(parsed.request, req);
    assert_eq!(parsed.source, b"%PDF-1.7");
    assert!(!format!("{parsed:?} {:?}", parsed.request).contains("synthetic-secret-password"));
}

#[test]
fn interrupted_final_pipe_read_preserves_complete_source_admission() {
    struct InterruptedAtEof {
        bytes: Cursor<Vec<u8>>,
        interrupted: bool,
    }
    impl std::io::Read for InterruptedAtEof {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            if !self.interrupted && self.bytes.position() == self.bytes.get_ref().len() as u64 {
                self.interrupted = true;
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            std::io::Read::read(&mut self.bytes, output)
        }
    }
    let mut reader = InterruptedAtEof {
        bytes: Cursor::new(job_bytes()),
        interrupted: false,
    };
    let job = read_job(&mut reader).unwrap();
    assert!(reader.interrupted);
    assert_eq!(job.source, b"%PDF-1.7");
    assert_eq!(job.binding, binding(&job.source));
}

#[test]
fn huge_declared_metadata_or_source_is_refused_from_only_the_fixed_header() {
    let original = job_bytes();
    let mut metadata = original[..68].to_vec();
    metadata[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        read_job(&mut Cursor::new(metadata)),
        Err(ProtocolError::Limit)
    ));
    let mut source = original[..68].to_vec();
    source[12..20].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(matches!(
        read_job(&mut Cursor::new(source)),
        Err(ProtocolError::Limit)
    ));
}

#[test]
fn truncated_tampered_and_trailing_inputs_never_reach_the_decoder() {
    let original = job_bytes();
    assert!(read_job(&mut Cursor::new(&original[..original.len() - 1])).is_err());
    let mut changed = original.clone();
    *changed.last_mut().unwrap() ^= 1;
    assert!(matches!(
        read_job(&mut Cursor::new(changed)),
        Err(ProtocolError::Integrity)
    ));
    let mut trailing = original;
    trailing.push(0);
    assert!(read_job(&mut Cursor::new(trailing)).is_err());
}

#[test]
fn unknown_or_duplicate_metadata_fields_refuse() {
    let original = job_bytes();
    let length = u32::from_le_bytes(original[8..12].try_into().unwrap()) as usize;
    let json = &original[68..68 + length];
    for prefix in [b"{\"unknown\":1,".as_slice(), b"{\"password\":null,"] {
        let metadata = [prefix, &json[1..]].concat();
        let mut modified = original[..68].to_vec();
        modified[8..12].copy_from_slice(&(metadata.len() as u32).to_le_bytes());
        modified.extend(metadata);
        modified.extend_from_slice(&original[68 + length..]);
        assert!(matches!(
            read_job(&mut Cursor::new(modified)),
            Err(ProtocolError::Invalid)
        ));
    }
}

#[test]
fn result_is_not_complete_until_hash_identity_and_eof_agree() {
    let bytes = reply_bytes(&vec![42; MAX_DATA + 17]);
    let mut output = Vec::new();
    let complete = receive(&bytes, &mut output).unwrap();
    assert_eq!(output, vec![42; MAX_DATA + 17]);
    assert_eq!(complete.bytes, output.len() as u64);
    let mut appended = bytes.clone();
    appended.push(0);
    assert!(receive(&appended, &mut Vec::new()).is_err());
    assert!(receive(&bytes[..bytes.len() - 1], &mut Vec::new()).is_err());
}

#[test]
fn unexpected_nonce_and_reserved_header_bits_refuse_before_payload() {
    let bytes = reply_bytes(b"safe");
    let mut bad = bytes.clone();
    bad[12] ^= 1;
    assert!(receive(&bad, &mut Vec::new()).is_err());
    let mut bad = bytes;
    bad[5] = 1;
    assert!(receive(&bad, &mut Vec::new()).is_err());
}

#[test]
fn metadata_refusal_precedes_private_destination_writes() {
    let bytes = reply_bytes(b"safe");
    let mut output = Vec::new();
    assert!(
        receive_reply(
            &mut Cursor::new(bytes),
            &mut output,
            binding(b"input"),
            100,
            &mut |_| Err(ProtocolError::Invalid),
            &mut |_| {}
        )
        .is_err()
    );
    assert!(output.is_empty());
}

#[test]
fn a_failed_tracker_cannot_skip_bad_frames_and_later_complete() {
    let bytes = reply_bytes(b"safe");
    let mut cursor = Cursor::new(bytes);
    let mut reader = ReplyReader::new(&mut cursor, [7; 16]).unwrap();
    let mut tracker = ReplyTracker::new(binding(b"input"), 100).unwrap();
    tracker
        .accept(&reader.next_frame().unwrap().unwrap())
        .unwrap();
    assert!(
        tracker
            .accept(&Frame {
                kind: FrameKind::Metadata,
                payload: b"{}".to_vec()
            })
            .is_err()
    );
    assert!(
        tracker
            .accept(&reader.next_frame().unwrap().unwrap())
            .is_err()
    );
    assert!(tracker.finish_eof().is_err());
}

#[test]
fn writer_cannot_finish_after_an_ignored_output_limit_failure() {
    let mut bytes = Vec::new();
    let mut out = ReplyWriter::new(&mut bytes, binding(b"input"), 2).unwrap();
    out.metadata(&serde_json::json!({"kind":"synthetic"}))
        .unwrap();
    assert!(out.data(b"too much").is_err());
    assert!(out.finish().is_err());
    assert!(receive(&bytes, &mut Vec::new()).is_err());
}

#[test]
fn typed_fault_is_terminal_even_after_private_partial_bytes() {
    let mut bytes = Vec::new();
    let mut out = ReplyWriter::new(&mut bytes, binding(b"input"), 100).unwrap();
    out.metadata(&serde_json::json!({"kind":"synthetic"}))
        .unwrap();
    out.data(b"partial").unwrap();
    out.fault(FaultCode::Verification).unwrap();
    let mut output = Vec::new();
    assert!(matches!(
        receive(&bytes, &mut output),
        Err(ProtocolError::Worker(FaultCode::Verification))
    ));
    assert_eq!(output, b"partial"); // caller retains/disposes this private staging
}

#[test]
fn progress_cannot_change_total_repeat_or_regress() {
    for bad in [
        Progress {
            completed: 1,
            total: 3,
        },
        Progress {
            completed: 1,
            total: 2,
        },
        Progress {
            completed: 0,
            total: 2,
        },
    ] {
        let mut bytes = Vec::new();
        let mut out = ReplyWriter::new(&mut bytes, binding(b"input"), 100).unwrap();
        out.metadata(&serde_json::json!({"kind":"synthetic"}))
            .unwrap();
        out.progress(Progress {
            completed: 1,
            total: 2,
        })
        .unwrap();
        assert!(out.progress(bad).is_err());
        assert!(out.finish().is_err());
    }
}

#[test]
fn private_request_metadata_is_bounded_before_serialized_output() {
    let mut req = request();
    req.password = Some("x".repeat(1025));
    let mut output = Vec::new();
    assert!(write_job(&mut output, binding(b"x"), &req, b"x").is_err());
    assert!(output.is_empty());
    req.password = None;
    req.operation = Operation::PdfText {
        page: 0,
        start: u32::MAX,
        count: 4096,
    };
    assert!(req.validate().is_err());
}

fn raw_frame(output: &mut Vec<u8>, kind: FrameKind, payload: &[u8]) {
    let mut header = [0_u8; 28];
    header[..4].copy_from_slice(b"VWR1");
    header[4] = kind as u8;
    header[8..12].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    header[12..28].copy_from_slice(&[7; 16]);
    output.extend_from_slice(&header);
    output.extend_from_slice(payload);
}

#[test]
fn production_receiver_refuses_tiny_data_fragments_before_the_excess_payload() {
    let metadata = br#"{"kind":"synthetic"}"#;
    let initial = 28 + metadata.len();
    let accepted = (MAX_WIRE_OVERHEAD as usize - initial) / 28;
    let mut wire = Vec::new();
    raw_frame(&mut wire, FrameKind::Metadata, metadata);
    for _ in 0..=accepted {
        raw_frame(&mut wire, FrameKind::Data, b"x");
    }
    let mut input = Cursor::new(wire);
    let mut output = Vec::new();
    assert!(matches!(
        receive_reply(
            &mut input,
            &mut output,
            binding(b"input"),
            1024 * 1024,
            &mut |_| Ok(()),
            &mut |_| {}
        ),
        Err(ProtocolError::Limit)
    ));
    assert_eq!(output.len(), accepted);
    // The rejected frame's header was read, but its one-byte body was not.
    assert_eq!(input.position(), (initial + accepted * 29 + 28) as u64);
}

#[test]
fn production_receiver_charges_json_whitespace_as_wire_overhead() {
    let metadata = br#"{"kind":"synthetic"}"#;
    let initial = 28 + metadata.len();
    let accepted = (MAX_WIRE_OVERHEAD as usize - initial) / (28 + 4096);
    let mut wire = Vec::new();
    raw_frame(&mut wire, FrameKind::Metadata, metadata);
    for index in 0..=accepted {
        let mut payload = serde_json::to_vec(&Progress {
            completed: index as u64 + 1,
            total: 4096,
        })
        .unwrap();
        payload.resize(4096, b' ');
        raw_frame(&mut wire, FrameKind::Progress, &payload);
    }
    let mut input = Cursor::new(wire);
    let mut output = Vec::new();
    let mut updates = 0;
    assert!(matches!(
        receive_reply(
            &mut input,
            &mut output,
            binding(b"input"),
            1024 * 1024,
            &mut |_| Ok(()),
            &mut |_| updates += 1
        ),
        Err(ProtocolError::Limit)
    ));
    assert_eq!(updates, accepted);
    assert!(output.is_empty());
    assert_eq!(
        input.position(),
        (initial + accepted * (28 + 4096) + 28) as u64
    );
}

#[test]
fn direct_tracker_also_charges_fragment_overhead_and_poison_is_terminal() {
    let metadata = Frame {
        kind: FrameKind::Metadata,
        payload: b"{}".to_vec(),
    };
    let data = Frame {
        kind: FrameKind::Data,
        payload: b"x".to_vec(),
    };
    let mut tracker = ReplyTracker::new(binding(b"input"), 1024 * 1024).unwrap();
    tracker.accept(&metadata).unwrap();
    let accepted = (MAX_WIRE_OVERHEAD as usize - 30) / 28;
    for _ in 0..accepted {
        tracker.accept(&data).unwrap();
    }
    assert!(matches!(tracker.accept(&data), Err(ProtocolError::Limit)));
    assert!(matches!(tracker.accept(&data), Err(ProtocolError::Order)));
    assert!(tracker.finish_eof().is_err());
}
