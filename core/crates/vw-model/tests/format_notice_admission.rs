#![allow(clippy::unwrap_used)]
use vw_model::{AssetId, validate_asset};
use vw_proto::v1::AddAsset;

fn asset(format: &str, notice: &str, depth: u32) -> AddAsset {
    AddAsset {
        asset_id: AssetId::hash(b"synthetic-full-immutable-container").to_string(),
        format: format.into(),
        width: 2,
        height: 2,
        orientation: 1,
        bit_depth: depth,
        byte_size: 64,
        source: "import".into(),
        color_space: "sRGB".into(),
        metadata_json: notice.into(),
        ..Default::default()
    }
}

#[test]
fn bounded_composite_and_preview_labels_require_their_truthful_typed_notice() {
    for (format, notice, depths) in [
        ("psd", r#"{"flattened_composite":true}"#, &[8, 16][..]),
        ("psb", r#"{"flattened_composite":true}"#, &[8, 16][..]),
        ("dng", r#"{"raw_preview":true}"#, &[8][..]),
        ("cr2", r#"{"raw_preview":true}"#, &[8][..]),
        ("raw-preview", r#"{"raw_preview":true}"#, &[8][..]),
    ] {
        for depth in depths {
            validate_asset(&asset(format, notice, *depth)).unwrap();
        }
        assert!(validate_asset(&asset(format, "{}", 8)).is_err());
        for bad in [
            r#"{"flattened_composite":"true"}"#,
            r#"{"raw_preview":1}"#,
            r#"{"flattened_composite":true,"raw_preview":true}"#,
            r#"{"flattened_composite":true,"first_tiff_page_only":true}"#,
        ] {
            assert!(validate_asset(&asset(format, bad, 8)).is_err());
        }
    }
}

#[test]
fn notices_cannot_relabel_ordinary_pixels_or_claim_unsupported_preview_depth() {
    for format in ["png", "jpeg", "tiff", "bmp"] {
        validate_asset(&asset(format, "{}", 8)).unwrap();
        for notice in [r#"{"flattened_composite":true}"#, r#"{"raw_preview":true}"#] {
            assert!(validate_asset(&asset(format, notice, 8)).is_err());
        }
    }
    validate_asset(&asset("tiff", r#"{"first_tiff_page_only":true}"#, 16)).unwrap();
    assert!(validate_asset(&asset("png", r#"{"first_tiff_page_only":true}"#, 8)).is_err());
    for depth in [0, 16, 32, 64] {
        assert!(validate_asset(&asset("dng", r#"{"raw_preview":true}"#, depth)).is_err());
    }
    for depth in [0, 32, 64] {
        assert!(validate_asset(&asset("psd", r#"{"flattened_composite":true}"#, depth)).is_err());
    }
}
