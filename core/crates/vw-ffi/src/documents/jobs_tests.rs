#![allow(clippy::unwrap_used)]
use super::*;
fn svg_request() -> String {
    serde_json::to_string(&Request {
        operation: Operation::SvgInfo,
        password: None,
        limits: vw_document_protocol::JobLimits {
            input_bytes: 64 * 1024 * 1024,
            output_bytes: 128 * 1024 * 1024,
            memory_bytes: 512 * 1024 * 1024,
        },
    })
    .unwrap()
}
fn svg_reply() -> DocumentJobResult {
    DocumentJobResult {
        source_blake3: blake3::hash(b"<svg/>").to_hex().to_string(),
        metadata_json:
            r#"{"kind":"svg_info","width":24.5,"height":31,"notices":["resource_denied"]}"#.into(),
        bytes: vec![],
        output_blake3: blake3::hash(b"").to_hex().to_string(),
    }
}
#[test]
fn valid_typed_metadata_is_retained_after_payload_hash_check() {
    let reply = svg_reply();
    assert!(
        validate_document_job_result(svg_request(), reply.source_blake3.clone(), reply).is_ok()
    );
}
#[test]
fn a_reply_from_another_original_is_refused() {
    assert!(matches!(
        validate_document_job_result(
            svg_request(),
            blake3::hash(b"different").to_hex().to_string(),
            svg_reply()
        ),
        Err(DocumentJobError::Verification)
    ));
}
#[test]
fn noncanonical_unknown_metadata_is_refused() {
    let mut reply = svg_reply();
    reply.metadata_json =
        r#"{"kind":"svg_info","width":24.5,"height":31,"notices":[],"command":"exec"}"#.into();
    assert!(
        validate_document_job_result(svg_request(), reply.source_blake3.clone(), reply).is_err()
    );
}
#[test]
fn info_cannot_smuggle_payload_bytes() {
    let mut reply = svg_reply();
    reply.bytes = b"hidden".to_vec();
    reply.output_blake3 = blake3::hash(&reply.bytes).to_hex().to_string();
    assert!(matches!(
        validate_document_job_result(svg_request(), reply.source_blake3.clone(), reply),
        Err(DocumentJobError::Verification)
    ));
}
#[test]
fn a_changed_private_output_fails_before_adoption() {
    let mut reply = svg_reply();
    reply.output_blake3 = blake3::hash(b"changed").to_hex().to_string();
    assert!(
        validate_document_job_result(svg_request(), reply.source_blake3.clone(), reply).is_err()
    );
}
#[test]
fn request_source_bounds_apply_before_parsing() {
    assert!(matches!(
        request(&svg_request(), &[]),
        Err(DocumentJobError::Limit)
    ));
    assert!(matches!(
        request(&" ".repeat(65537), b"<svg/>"),
        Err(DocumentJobError::Limit)
    ));
}
#[test]
fn metadata_kind_must_match_the_exact_operation() {
    let mut reply = svg_reply();
    reply.metadata_json = r#"{"kind":"pdf_info","pages":1,"encrypted":false,"notices":[]}"#.into();
    assert!(
        validate_document_job_result(svg_request(), reply.source_blake3.clone(), reply).is_err()
    );
}

#[test]
fn output_hashing_accepts_empty_information_payloads_without_enlarging_source_requests() {
    assert_eq!(
        document_source_hash(vec![]).unwrap(),
        blake3::hash(b"").to_hex().to_string()
    );
    assert!(matches!(
        request(&svg_request(), &[]),
        Err(DocumentJobError::Limit)
    ));
}
#[test]
fn svg_source_limit_is_independent_of_the_shared_output_hash_limit() {
    assert!(matches!(
        request(&svg_request(), &vec![b' '; 4 * 1024 * 1024 + 1]),
        Err(DocumentJobError::Limit)
    ));
}
