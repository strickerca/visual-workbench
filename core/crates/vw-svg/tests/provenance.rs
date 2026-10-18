#![allow(clippy::unwrap_used)]
use vw_svg::{Limits, SvgDocument, verify_svg_copy_metadata};
fn identity() -> String {
    format!(
        r#"{{"schema_version":1,"host_seq":18446744073709551615,"state_hash":"aabbccdd{}","revision":"r18446744073709551615-aabbccdd","source_asset":"{}","capture_session":"00000000-0000-0000-0000-000000000001","frame_id":18446744073709551615,"escaped":"<>&"}}"#,
        "a".repeat(56),
        "b".repeat(64)
    )
}
fn document() -> SvgDocument {
    SvgDocument::parse(b"<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><rect width='8' height='8' fill='red'/></svg>",Limits::default(),&||false).unwrap()
}
#[test]
fn final_xml_metadata_uses_existing_namespace_and_preserves_identity_exactly() {
    let metadata = identity();
    let bytes = document()
        .export_svg_with_metadata(&[], &metadata, &|| false)
        .unwrap();
    verify_svg_copy_metadata(&bytes, &metadata, Limits::default(), &|| false).unwrap();
    let doc = usvg::roxmltree::Document::parse(std::str::from_utf8(&bytes).unwrap()).unwrap();
    let node = doc
        .descendants()
        .find(|n| n.has_tag_name(("https://visualworkbench.app/ns/export/1.0/", "Metadata")))
        .unwrap();
    assert_eq!(node.text(), Some(metadata.as_str()));
    assert!(
        std::str::from_utf8(&bytes)
            .unwrap()
            .contains("&lt;&gt;&amp;")
    );
}
#[test]
fn metadata_does_not_change_rendered_pixels_or_retain_source_metadata() {
    let input = b"<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><metadata>private original metadata</metadata><rect width='8' height='8' fill='red'/></svg>";
    let original = SvgDocument::parse(input, Limits::default(), &|| false).unwrap();
    let bytes = original
        .export_svg_with_metadata(&[], &identity(), &|| false)
        .unwrap();
    assert!(
        !std::str::from_utf8(&bytes)
            .unwrap()
            .contains("private original metadata")
    );
    let reopened = SvgDocument::parse(&bytes, Limits::default(), &|| false).unwrap();
    let request = vw_svg::TileRequest {
        x: 0.,
        y: 0.,
        scale: 1.,
        width: 8,
        height: 8,
    };
    assert_eq!(
        original.render_tile(request, &|| false).unwrap().rgba,
        reopened.render_tile(request, &|| false).unwrap().rgba
    );
}
#[test]
fn wrong_identity_namespace_duplicate_and_comment_cannot_satisfy_readback() {
    let metadata = identity();
    let bytes = document()
        .export_svg_with_metadata(&[], &metadata, &|| false)
        .unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    for bad in [
        text.replace("r18446744073709551615-aabbccdd", "r1-deadbeef"),
        text.replace(
            "https://visualworkbench.app/ns/export/1.0/",
            "https://invalid.test/",
        ),
        text.replace(
            "</svg>",
            &format!(
                "{} </svg>",
                text.split("<metadata>")
                    .nth(1)
                    .unwrap()
                    .split("</metadata>")
                    .next()
                    .map(|v| format!("<metadata>{v}</metadata>"))
                    .unwrap()
            ),
        ),
        format!("<svg xmlns='http://www.w3.org/2000/svg'><!--{metadata}--></svg>"),
    ] {
        assert!(
            verify_svg_copy_metadata(bad.as_bytes(), &metadata, Limits::default(), &|| false)
                .is_err()
        );
    }
}
#[test]
fn malformed_metadata_controls_dtd_cancel_and_limit_are_refused() {
    let doc = document();
    for metadata in [
        String::new(),
        "bad\0value".into(),
        "bad\u{1}value".into(),
        "nonascii\u{e9}".into(),
        "x".repeat(vw_svg::MAX_EXPORT_METADATA_BYTES + 1),
    ] {
        assert!(
            doc.export_svg_with_metadata(&[], &metadata, &|| false)
                .is_err()
        );
    }
    assert!(
        doc.export_svg_with_metadata(&[], &identity(), &|| true)
            .is_err()
    );
    let limits = Limits {
        output_bytes: 1,
        ..Limits::default()
    };
    assert!(verify_svg_copy_metadata(b"<svg/>", &identity(), limits, &|| false).is_err());
    assert!(
        verify_svg_copy_metadata(
            b"<!DOCTYPE svg [<!ENTITY a 'x'>]><svg/>",
            &identity(),
            Limits::default(),
            &|| false
        )
        .is_err()
    );
}
#[test]
fn maximum_metadata_and_xml_newline_roundtrip_are_bounded() {
    for metadata in [
        "x".repeat(vw_svg::MAX_EXPORT_METADATA_BYTES),
        "{\r\n\t\"revision\":\"r1-aabbccdd\"}".into(),
    ] {
        let bytes = document()
            .export_svg_with_metadata(&[], &metadata, &|| false)
            .unwrap();
        verify_svg_copy_metadata(&bytes, &metadata, Limits::default(), &|| false).unwrap();
    }
}
