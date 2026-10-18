#![allow(clippy::unwrap_used)] // Malformed fixture setup must fail its test immediately.
use vw_document_protocol::{
    JobBinding, JobLimits, Operation, PageRegion, Request, read_job, write_job,
};
fn request() -> Request {
    Request {
        limits: JobLimits {
            input_bytes: 64 * 1024 * 1024,
            output_bytes: 64 * 1024 * 1024,
            memory_bytes: 512 * 1024 * 1024,
        },
        operation: Operation::PdfPcRedaction {
            scale: 1.2,
            explicitly_remove_encryption: false,
            regions: vec![PageRegion {
                page: 2,
                rectangle: [1.0, 2.0, 3.0, 4.0],
            }],
        },
        password: None,
        export_identity: None,
    }
}
#[test]
fn pc_operation_roundtrips_with_exact_source_binding() {
    let source = b"%PDF-synthetic-private-input";
    let binding = JobBinding {
        nonce: [9; 16],
        source_blake3: *blake3::hash(source).as_bytes(),
    };
    let mut bytes = Vec::new();
    write_job(&mut bytes, binding, &request(), source).unwrap();
    let result = read_job(&mut bytes.as_slice()).unwrap();
    assert_eq!(result.binding, binding);
    assert_eq!(result.request, request());
    assert_eq!(result.source, source);
}
#[test]
fn invalid_pc_scale_or_region_refuses_before_input_decode() {
    for scale in [f64::NAN, 0.0, 9.0] {
        let mut value = request();
        if let Operation::PdfPcRedaction { scale: current, .. } = &mut value.operation {
            *current = scale;
        }
        assert!(value.validate().is_err());
    }
    let mut value = request();
    if let Operation::PdfPcRedaction { regions, .. } = &mut value.operation {
        regions.clear();
    }
    assert!(value.validate().is_err());
}
#[test]
fn pc_request_cannot_smuggle_a_silent_fallback_or_library_path() {
    let mut value = serde_json::to_value(request()).unwrap();
    value["operation"]["automatic_raster_fallback"] = serde_json::Value::Bool(true);
    assert!(serde_json::from_value::<Request>(value).is_err());
    let mut value = serde_json::to_value(request()).unwrap();
    value["operation"]["qpdf_library"] = serde_json::Value::String("untrusted.dll".into());
    assert!(serde_json::from_value::<Request>(value).is_err());
}
