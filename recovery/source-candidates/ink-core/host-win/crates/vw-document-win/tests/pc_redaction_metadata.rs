use vw_document_protocol::{JobLimits, Operation, PageRegion, Request};
use vw_document_win::ResultMetadata;
fn request() -> Request {
    Request { limits: JobLimits { input_bytes: 64 * 1024 * 1024, output_bytes: 64 * 1024 * 1024, memory_bytes: 512 * 1024 * 1024 },
        operation: Operation::PdfPcRedaction { scale: 1.2, explicitly_remove_encryption: false, regions: vec![PageRegion { page: 2, rectangle: [1.0, 2.0, 3.0, 4.0] }] }, password: None }
}
fn metadata() -> ResultMetadata {
    ResultMetadata::PcRedaction { pages: 3, regions: 1, bytes: 4096, requested_scale: 1.2, effective_scale: f64::from(1.2f32),
        plan_blake3: "b".repeat(64), removed_objects: 2, preserved_paths: 7, raster_tiles: 3 }
}
#[test] fn pc_reply_requires_matching_operation_and_effective_native_scale() {
    assert!(metadata().validate_for(&request()).is_ok());
    let mut wrong = metadata(); if let ResultMetadata::PcRedaction { effective_scale, .. } = &mut wrong { *effective_scale = 1.2; }
    assert!(wrong.validate_for(&request()).is_err());
}
#[test] fn ordinary_or_raster_copy_metadata_cannot_claim_pc_redaction() {
    let raster = ResultMetadata::RasterRedaction { pages: 3, regions: 1, bytes: 4096, requested_scale: 1.2, effective_scale: f64::from(1.2f32), plan_blake3: "b".repeat(64) };
    assert!(raster.validate_for(&request()).is_err());
    let mut value = request(); value.operation = Operation::PdfCopy { copy: vw_document_protocol::CopyKind::Flattened, overlays: vec![] };
    assert!(metadata().validate_for(&value).is_err());
}
#[test] fn mismatched_region_count_or_unbounded_output_refuses_before_data() {
    let mut wrong = metadata(); if let ResultMetadata::PcRedaction { regions, .. } = &mut wrong { *regions = 0; }
    assert!(wrong.validate_for(&request()).is_err());
    let mut wrong = metadata(); if let ResultMetadata::PcRedaction { bytes, .. } = &mut wrong { *bytes = 65 * 1024 * 1024; }
    assert!(wrong.validate_for(&request()).is_err());
}
