#![allow(clippy::unwrap_used)]
use vw_document_protocol::{CopyIdentity, CopyKind, JobLimits, Operation, PageRegion, Request};
use vw_document_win::ResultMetadata;
fn identity() -> CopyIdentity {
    CopyIdentity {
        schema_version: 1,
        host_seq: u64::MAX,
        state_hash: "a".repeat(64),
        source_asset: "b".repeat(64),
        capture_session: Some("00000000-0000-0000-0000-000000000001".into()),
        frame_id: Some(u64::MAX),
    }
}
fn cases() -> Vec<(Operation, ResultMetadata)> {
    let region = PageRegion {
        page: 0,
        rectangle: [1., 1., 2., 2.],
    };
    vec![
        (
            Operation::PdfCopy {
                copy: CopyKind::Annotated,
                overlays: vec![],
            },
            ResultMetadata::PdfCopy {
                export_identity: identity(),
                copy: CopyKind::Annotated,
                pages: 1,
                bytes: 100,
                redacted: false,
            },
        ),
        (
            Operation::PdfCopy {
                copy: CopyKind::Flattened,
                overlays: vec![],
            },
            ResultMetadata::PdfCopy {
                export_identity: identity(),
                copy: CopyKind::Flattened,
                pages: 1,
                bytes: 100,
                redacted: false,
            },
        ),
        (
            Operation::SvgCopy { overlays: vec![] },
            ResultMetadata::SvgCopy {
                export_identity: identity(),
                bytes: 100,
                notices: vec![],
            },
        ),
        (
            Operation::PdfRasterRedaction {
                scale: 1.,
                explicitly_remove_encryption: false,
                regions: vec![region],
            },
            ResultMetadata::RasterRedaction {
                export_identity: identity(),
                pages: 1,
                regions: 1,
                bytes: 100,
                requested_scale: 1.,
                effective_scale: 1.,
                plan_blake3: "c".repeat(64),
            },
        ),
        (
            Operation::PdfPcRedaction {
                scale: 1.,
                explicitly_remove_encryption: false,
                regions: vec![region],
            },
            ResultMetadata::PcRedaction {
                export_identity: identity(),
                pages: 1,
                regions: 1,
                bytes: 100,
                requested_scale: 1.,
                effective_scale: 1.,
                plan_blake3: "c".repeat(64),
                removed_objects: 1,
                preserved_paths: 1,
                raster_tiles: 1,
            },
        ),
    ]
}
#[test]
fn every_copy_mode_requires_exact_full_identity_echo() {
    for (operation, metadata) in cases() {
        let mut request = Request {
            operation,
            password: None,
            export_identity: Some(identity()),
            limits: JobLimits {
                input_bytes: 64 * 1024 * 1024,
                output_bytes: 128 * 1024 * 1024,
                memory_bytes: 512 * 1024 * 1024,
            },
        };
        metadata.validate_for(&request).unwrap();
        request
            .export_identity
            .as_mut()
            .unwrap()
            .state_hash
            .replace_range(63..64, "f");
        // Same displayed hash8 still cannot authorize a different full state.
        assert!(metadata.validate_for(&request).is_err());
        request.export_identity = None;
        assert!(metadata.validate_for(&request).is_err());
    }
}
#[test]
fn final_file_json_uses_existing_identity_keys_and_exact_unsigned_maxima() {
    let json = ResultMetadata::identity_metadata(&identity()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["revision"], "r18446744073709551615-aaaaaaaa");
    assert_eq!(value["host_seq"].as_u64(), Some(u64::MAX));
    assert_eq!(value["frame_id"].as_u64(), Some(u64::MAX));
    assert_eq!(value["state_hash"], "a".repeat(64));
    assert_eq!(value["source_asset"], "b".repeat(64));
    assert_eq!(
        value["capture_session"],
        "00000000-0000-0000-0000-000000000001"
    );
}
#[test]
fn missing_or_changed_result_identity_is_rejected_before_payload_adoption() {
    for (operation, metadata) in cases() {
        let request = Request {
            operation,
            password: None,
            export_identity: Some(identity()),
            limits: JobLimits {
                input_bytes: 64 * 1024 * 1024,
                output_bytes: 128 * 1024 * 1024,
                memory_bytes: 512 * 1024 * 1024,
            },
        };
        let mut value = serde_json::to_value(&metadata).unwrap();
        value.as_object_mut().unwrap().remove("export_identity");
        assert!(ResultMetadata::parse_for(&serde_json::to_vec(&value).unwrap(), &request).is_err());
        value = serde_json::to_value(&metadata).unwrap();
        value["export_identity"]["source_asset"] = serde_json::json!("d".repeat(64));
        assert!(ResultMetadata::parse_for(&serde_json::to_vec(&value).unwrap(), &request).is_err());
    }
}
