#![allow(clippy::unwrap_used)] // Synthetic protocol fixtures fail immediately.
use vw_document_protocol::{
    CopyIdentity, CopyKind, JobBinding, JobLimits, Operation, Request, read_job, write_job,
};

fn request(source: &[u8]) -> Request {
    Request {
        limits: JobLimits {
            input_bytes: 1024,
            output_bytes: 1024,
            memory_bytes: 32 * 1024 * 1024,
        },
        operation: Operation::PdfCopy {
            copy: CopyKind::Original,
            overlays: vec![],
        },
        password: None,
        export_identity: Some(CopyIdentity {
            schema_version: 1,
            host_seq: u64::MAX,
            state_hash: "b".repeat(64),
            source_asset: blake3::hash(source).to_hex().to_string(),
            capture_session: Some("018bcfe5-6800-7000-8000-000000000001".into()),
            frame_id: Some(u64::MAX),
        }),
    }
}
#[test]
fn copy_identity_roundtrips_exact_full_hashes_and_unsigned_maxima() {
    let source = b"%PDF-identity-fixture";
    let request = request(source);
    assert_eq!(
        request.require_copy_identity().unwrap().revision().unwrap(),
        "r18446744073709551615-bbbbbbbb"
    );
    let binding = JobBinding {
        nonce: [9; 16],
        source_blake3: *blake3::hash(source).as_bytes(),
    };
    let mut wire = vec![];
    write_job(&mut wire, binding, &request, source).unwrap();
    assert_eq!(read_job(&mut wire.as_slice()).unwrap().request, request);
}
#[test]
fn another_source_identity_is_refused_before_any_pipe_write() {
    let source = b"%PDF-current";
    let request = request(b"%PDF-other");
    let binding = JobBinding {
        nonce: [9; 16],
        source_blake3: *blake3::hash(source).as_bytes(),
    };
    let mut wire = vec![];
    assert!(write_job(&mut wire, binding, &request, source).is_err());
    assert!(wire.is_empty());
}
#[test]
fn incomplete_noncanonical_and_unknown_identity_fields_are_refused() {
    for key in [
        "schema_version",
        "state_hash",
        "source_asset",
        "capture_session",
        "frame_id",
    ] {
        let mut identity =
            serde_json::to_value(request(b"input").export_identity.unwrap()).unwrap();
        identity[key] = match key {
            "schema_version" => 2.into(),
            "frame_id" => serde_json::Value::Null,
            "capture_session" => "NOT-A-UUID".into(),
            _ => "B".repeat(64).into(),
        };
        let admitted = serde_json::from_value::<CopyIdentity>(identity)
            .is_ok_and(|value| value.validate().is_ok());
        assert!(!admitted, "accepted invalid {key}");
    }
    let mut identity = serde_json::to_value(request(b"input").export_identity.unwrap()).unwrap();
    identity["path"] = "/private/path".into();
    assert!(serde_json::from_value::<CopyIdentity>(identity).is_err());
}
#[test]
fn noncopy_cannot_claim_export_identity_and_unbound_copy_is_not_publishable() {
    let mut request = request(b"input");
    request.operation = Operation::PdfInfo;
    assert!(request.validate().is_err());
    request.operation = Operation::SvgCopy { overlays: vec![] };
    request.export_identity = None;
    assert!(request.validate().is_ok()); // Legacy low-level framing compatibility only.
    assert!(request.require_copy_identity().is_err());
}
