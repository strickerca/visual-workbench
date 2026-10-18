#![allow(clippy::unwrap_used)]
use vw_svg::{FidelityNotice, Limits, Outline, Point, SvgDocument, SvgError, TileRequest};

fn parse(text: &str) -> SvgDocument {
    SvgDocument::parse(text.as_bytes(), Limits::default(), &|| false).unwrap()
}
fn tile(document: &SvgDocument) -> Vec<u8> {
    document
        .render_tile(
            TileRequest {
                x: 0.0,
                y: 0.0,
                scale: 1.0,
                width: 32,
                height: 32,
            },
            &|| false,
        )
        .unwrap()
        .rgba
}
fn pixel(rgba: &[u8], x: usize, y: usize) -> [u8; 4] {
    rgba[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4]
        .try_into()
        .unwrap()
}

#[test]
fn static_rect_has_independent_known_pixels_and_explicit_dimensions() {
    let document = parse(
        "<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32'><rect x='4' y='5' width='8' height='9' fill='#ff0000'/></svg>",
    );
    assert_eq!(
        (document.info().width, document.info().height),
        (32.0, 32.0)
    );
    let pixels = tile(&document);
    assert_eq!(pixel(&pixels, 7, 9), [255, 0, 0, 255]);
    assert_eq!(pixel(&pixels, 0, 0), [0, 0, 0, 0]);
}

#[test]
fn scripts_events_foreign_content_and_animations_have_explicit_notices() {
    let document = parse(
        "<svg width='32' height='32' onload='throw secret'><script>fetch('https://invalid')</script><foreignObject width='2' height='2'><div>private</div></foreignObject><animate attributeName='x'/><rect width='1' height='1'/></svg>",
    );
    let report = document.info();
    assert!(
        report
            .notices
            .contains(&FidelityNotice::ScriptOrEventIgnored)
    );
    assert!(
        report
            .notices
            .contains(&FidelityNotice::ForeignObjectIgnored)
    );
    assert!(report.notices.contains(&FidelityNotice::AnimationIgnored));
    let xml = String::from_utf8(document.export_svg(&[], &|| false).unwrap()).unwrap();
    assert!(!xml.contains("fetch"));
    assert!(!xml.contains("private"));
    assert!(!xml.contains("onload"));
}

#[test]
fn all_image_resources_are_denied_including_data_urls_and_local_paths() {
    for href in [
        "https://invalid.example/private.png",
        "file:///private.png",
        "C:/private.png",
        "private.png",
        "data:image/svg+xml;base64,PHN2Zy8+",
    ] {
        let source = format!(
            "<svg width='32' height='32'><image href='{href}' width='32' height='32'/></svg>"
        );
        let document = parse(&source);
        assert!(
            document
                .info()
                .notices
                .contains(&FidelityNotice::ResourceDenied)
        );
        assert!(tile(&document).iter().all(|value| *value == 0));
        let xml = String::from_utf8(document.export_svg(&[], &|| false).unwrap()).unwrap();
        assert!(!xml.contains(href));
    }
}

#[test]
fn missing_font_and_unsupported_filter_do_not_silently_claim_fidelity() {
    let document = parse(
        "<svg width='32' height='32'><defs><filter id='f'><feUnimplemented/></filter></defs><text x='1' y='20' font-family='Unavailable Private Family'>A</text></svg>",
    );
    assert!(
        document
            .info()
            .notices
            .contains(&FidelityNotice::MissingFont)
    );
    assert!(
        document
            .info()
            .notices
            .contains(&FidelityNotice::UnsupportedFilter)
    );
}

#[test]
fn bundled_text_reimports_as_paths_without_resource_dependencies() {
    let document = parse(
        "<svg width='32' height='32'><text x='2' y='22' font-family='Inter' font-size='18'>Ab</text></svg>",
    );
    let exported = document.export_svg(&[], &|| false).unwrap();
    assert!(!String::from_utf8_lossy(&exported).contains("<text"));
    let imported = SvgDocument::parse(&exported, Limits::default(), &|| false).unwrap();
    assert_eq!(tile(&document), tile(&imported));
}

#[test]
fn compound_highlighter_is_one_fill_with_a_preserved_hole_and_exact_roundtrip() {
    let document = parse("<svg width='32' height='32'/>");
    let contour =
        |coordinates: &[(f64, f64)]| coordinates.iter().map(|&(x, y)| Point { x, y }).collect();
    let overlay = Outline {
        contours: vec![
            contour(&[(2., 2.), (28., 2.), (28., 28.), (2., 28.)]),
            contour(&[(10., 10.), (10., 20.), (20., 20.), (20., 10.)]),
        ],
        transform: [1., 0., 0., 1., 0., 0.],
        rgba: [255, 240, 0, 128],
    };
    let first = document.export_svg(&[overlay], &|| false).unwrap();
    let imported = SvgDocument::parse(&first, Limits::default(), &|| false).unwrap();
    let pixels = tile(&imported);
    assert_eq!(pixel(&pixels, 4, 4)[3], 128);
    assert_eq!(pixel(&pixels, 15, 15), [0, 0, 0, 0]);
    let second = imported.export_svg(&[], &|| false).unwrap();
    assert_eq!(
        pixels,
        tile(&SvgDocument::parse(&second, Limits::default(), &|| false).unwrap())
    );
}

#[test]
fn view_scale_tile_is_a_window_not_a_document_resize() {
    let document = parse(
        "<svg width='32' height='32'><rect x='8' y='8' width='4' height='4' fill='blue'/></svg>",
    );
    let result = document
        .render_tile(
            TileRequest {
                x: 8.,
                y: 8.,
                scale: 2.,
                width: 32,
                height: 32,
            },
            &|| false,
        )
        .unwrap();
    assert_eq!(pixel(&result.rgba, 2, 2), [0, 0, 255, 255]);
    assert_eq!(pixel(&result.rgba, 12, 12), [0, 0, 0, 0]);
    assert_eq!(document.info().width, 32.);
}

#[test]
fn cancellation_applies_before_parse_render_and_export() {
    assert!(matches!(
        SvgDocument::parse(b"not even XML", Limits::default(), &|| true),
        Err(SvgError::Cancelled)
    ));
    let document = parse("<svg width='32' height='32'/>");
    assert_eq!(
        document.render_tile(
            TileRequest {
                x: 0.,
                y: 0.,
                scale: 1.,
                width: 32,
                height: 32
            },
            &|| true
        ),
        Err(SvgError::Cancelled)
    );
    assert_eq!(document.export_svg(&[], &|| true), Err(SvgError::Cancelled));
}

#[test]
fn an_empty_or_entirely_denied_document_exports_a_well_formed_empty_root() {
    for source in [
        "<svg width='32' height='32'/>",
        "<svg width='32' height='32'><image href='file:///unavailable.png' width='32' height='32'/></svg>",
    ] {
        let document = parse(source);
        let xml = document.export_svg(&[], &|| false).unwrap();
        let reopened = SvgDocument::parse(&xml, Limits::default(), &|| false).unwrap();
        assert_eq!(
            (reopened.info().width, reopened.info().height),
            (32.0, 32.0)
        );
        assert!(tile(&reopened).iter().all(|v| *v == 0));
    }
}

#[test]
fn subnormal_coordinates_and_translations_have_bounded_scientific_output() {
    let document = parse("<svg width='32' height='32'/>");
    let tiny = f64::from_bits(1);
    let outline = Outline {
        contours: vec![vec![Point { x: tiny, y: -tiny }; 60_000]],
        transform: [1., 0., 0., 1., tiny, -tiny],
        rgba: [0, 0, 0, 128],
    };
    let bytes = document.export_svg(&[outline], &|| false).unwrap();
    let xml = String::from_utf8(bytes).unwrap();
    assert!(xml.contains("4.94065645841246544e-324"));
    assert!(xml.len() <= 60_000 * 64 + 1024);
    assert!(xml.len() <= Limits::default().output_bytes);
}

#[test]
fn subnormal_output_reservation_refuses_before_a_large_append() {
    let limits = Limits {
        output_bytes: 900,
        ..Limits::default()
    };
    let document = SvgDocument::parse(b"<svg width='32' height='32'/>", limits, &|| false).unwrap();
    let outline = Outline {
        contours: vec![vec![
            Point {
                x: f64::from_bits(1),
                y: -f64::from_bits(1)
            };
            16
        ]],
        transform: [1., 0., 0., 1., 0., 0.],
        rgba: [0, 0, 0, 255],
    };
    assert_eq!(
        document.export_svg(&[outline], &|| false),
        Err(SvgError::Limit)
    );
}

#[test]
fn denied_external_paints_and_xlink_resources_are_reported_and_removed() {
    for attribute in [
        "fill='url(https://invalid.example/paint.svg#red)'",
        "style='fill:url(file:///private.svg#paint)'",
        "xlink:href='  file:///private.svg#shape  '",
    ] {
        let source = format!(
            "<svg xmlns:xlink='http://www.w3.org/1999/xlink' width='32' height='32'><rect width='16' height='16' {attribute}/></svg>"
        );
        let document = parse(&source);
        assert!(
            document
                .info()
                .notices
                .contains(&FidelityNotice::ResourceDenied)
        );
        let output = document.export_svg(&[], &|| false).unwrap();
        let xml = String::from_utf8(output).unwrap();
        assert!(!xml.contains("invalid.example"));
        assert!(!xml.contains("private.svg"));
        assert!(!xml.contains("href="));
    }
}
