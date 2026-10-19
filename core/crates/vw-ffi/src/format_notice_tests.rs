// Included in format_images::tests: the same parser used by original_format_info.
#[test]
fn legacy_tiff_without_an_explicit_page_notice_remains_conservative() {
    for metadata in ["{}", r#"{"description":"legacy TIFF"}"#] {
        let notices = stored_import_notices("tiff", metadata).unwrap();
        assert!(notices.first_tiff_page_only, "{metadata}");
        assert!(!notices.flattened_composite);
        assert!(!notices.raw_preview);
    }
}

#[test]
fn explicit_tiff_page_notice_preserves_both_true_and_false() {
    for expected in [true, false] {
        let metadata = format!(r#"{{"first_tiff_page_only":{expected}}}"#);
        let notices = stored_import_notices("tiff", &metadata).unwrap();
        assert_eq!(notices.first_tiff_page_only, expected);
        // Newly inspected/serialized single-page TIFFs retain their exact false.
        let serialized = serde_json::to_string(&ImportNotices {
            first_tiff_page_only: expected,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(serialized, metadata);
        assert_eq!(
            stored_import_notices("tiff", &serialized)
                .unwrap()
                .first_tiff_page_only,
            expected
        );
    }
}

#[test]
fn invalid_tiff_notice_metadata_never_suppresses_the_page_warning() {
    for metadata in [
        "",
        "{",
        "null",
        "true",
        "[]",
        "[false]",
        r#"{"first_tiff_page_only":null}"#,
        r#"{"first_tiff_page_only":"false"}"#,
        r#"{"first_tiff_page_only":0}"#,
        r#"{"first_tiff_page_only":true,"first_tiff_page_only":false}"#,
        r#"{"first_tiff_page_only":false,"first_tiff_page_only":false}"#,
        r#"{"first_tiff_page_only":false,"raw_preview":"invalid"}"#,
        r#"{"first_tiff_page_only":false,"flattened_composite":null}"#,
    ] {
        assert!(
            stored_import_notices("tiff", metadata)
                .unwrap()
                .first_tiff_page_only,
            "{metadata}"
        );
    }
    assert!(matches!(
        stored_import_notices("tiff", &" ".repeat(4097)),
        Err(WorkflowError::Limit)
    ));
}

#[test]
fn tiff_fallback_does_not_admit_inconsistent_composite_or_raw_notices() {
    for metadata in [
        r#"{"flattened_composite":true}"#,
        r#"{"raw_preview":true}"#,
        r#"{"first_tiff_page_only":false,"flattened_composite":true}"#,
        r#"{"first_tiff_page_only":false,"raw_preview":true}"#,
    ] {
        assert!(matches!(
            stored_import_notices("tiff", metadata),
            Err(WorkflowError::Invalid)
        ));
    }
}

#[test]
fn adjacent_composite_and_preview_formats_keep_explicit_notice_admission() {
    for format in ["psd", "psb", "dng", "cr2", "raw-preview"] {
        for invalid in ["", "{", "{}", "null", r#"{"first_tiff_page_only":true}"#] {
            assert!(
                stored_import_notices(format, invalid).is_err(),
                "{format}: {invalid}"
            );
        }
        let composite = matches!(format, "psd" | "psb");
        let metadata = if composite {
            r#"{"flattened_composite":true}"#
        } else {
            r#"{"raw_preview":true}"#
        };
        let notices = stored_import_notices(format, metadata).unwrap();
        assert!(!notices.first_tiff_page_only);
        assert_eq!(notices.flattened_composite, composite);
        assert_eq!(notices.raw_preview, !composite);
    }
}

#[test]
fn ordinary_legacy_format_fallback_and_import_serialization_remain_unchanged() {
    for format in ["png", "jpeg", "webp", "heic", "heif", "avif", "bmp"] {
        for metadata in ["{}", "", "{", r#"{"first_tiff_page_only":true}"#] {
            let notices = stored_import_notices(format, metadata).unwrap();
            assert!(!notices.first_tiff_page_only);
            assert!(!notices.flattened_composite);
            assert!(!notices.raw_preview);
        }
        assert!(matches!(
            stored_import_notices(format, &" ".repeat(4097)),
            Err(WorkflowError::Limit)
        ));
    }
    assert_eq!(
        serde_json::to_string(&ImportNotices::default()).unwrap(),
        r#"{"first_tiff_page_only":false}"#
    );
}
