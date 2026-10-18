#![allow(clippy::unwrap_used)]
use vw_svg::{Limits, Outline, Point, SvgDocument, SvgError, TileRequest};

fn refused(source: &str, expected: SvgError) {
    assert!(
        matches!(SvgDocument::parse(source.as_bytes(), Limits::default(), &|| false), Err(error) if error == expected)
    );
}

#[test]
fn dtd_internal_external_and_entity_expansion_are_refused() {
    for source in [
        "<!DOCTYPE svg SYSTEM 'file:///private'><svg width='1' height='1'/>",
        "<!DOCTYPE svg [<!ENTITY a 'abc'>]><svg width='1' height='1'>&a;</svg>",
        "<!DOCTYPE svg [<!ENTITY x SYSTEM 'https://invalid'>]><svg width='1' height='1'>&x;</svg>",
    ] {
        refused(source, SvgError::DtdDenied);
    }
}

#[test]
fn gzip_bomb_is_refused_before_any_inflate() {
    assert!(matches!(
        SvgDocument::parse(&[0x1f, 0x8b, 0, 0], Limits::default(), &|| false),
        Err(SvgError::CompressedUnsupported)
    ));
}

#[test]
fn malformed_and_non_svg_xml_are_not_successful_empty_documents() {
    for source in [
        "",
        "<svg",
        "<private/>",
        "<svg xmlns='https://invalid' width='1' height='1'/>",
        "<svg><g></svg>",
    ] {
        refused(source, SvgError::Malformed);
    }
}

#[test]
fn enormous_viewbox_is_not_downsampled_silently() {
    refused(
        "<svg width='1000000' height='1000000' viewBox='0 0 1000000 1000000'/>",
        SvgError::DocumentSize,
    );
}

#[test]
fn low_memory_refuses_before_xml_parse_or_font_allocation() {
    let limits = Limits {
        memory_bytes: 1024,
        ..Limits::default()
    };
    assert!(matches!(
        SvgDocument::parse(b"not XML", limits, &|| false),
        Err(SvgError::Memory { .. })
    ));
}

#[test]
fn lexical_reservations_and_depth_are_bounded_before_resolution() {
    let limits = Limits {
        nodes: 4,
        ..Limits::default()
    };
    assert!(matches!(
        SvgDocument::parse(
            b"<!-- <<<<<<<<<<< --><svg width='1' height='1'/>",
            limits,
            &|| false
        ),
        Err(SvgError::Limit)
    ));
    let nested = format!(
        "<svg width='1' height='1'>{}<rect width='1' height='1'/>{}</svg>",
        "<g>".repeat(70),
        "</g>".repeat(70)
    );
    refused(&nested, SvgError::Limit);
}

#[test]
fn cyclic_and_exponentially_expanding_references_are_refused() {
    refused(
        "<svg width='1' height='1'><defs><g id='a'><use href='#b'/></g><g id='b'><use href='#a'/></g></defs><use href='#a'/></svg>",
        SvgError::ReferenceCycle,
    );
    let mut source =
        String::from("<svg width='1' height='1'><defs><g id='a0'><rect width='1' height='1'/></g>");
    for index in 1..20 {
        source.push_str(&format!(
            "<g id='a{index}'><use href='#a{}'/><use href='#a{}'/></g>",
            index - 1,
            index - 1
        ));
    }
    source.push_str("</defs><use href='#a19'/></svg>");
    refused(&source, SvgError::Limit);
}

#[test]
fn escaped_or_reference_bearing_css_cannot_bypass_graph_admission() {
    for css in [
        "rect {fill:url(#a)}",
        "@import 'https://invalid';",
        "rect{f\\69ll:red}",
    ] {
        refused(
            &format!("<svg width='1' height='1'><style>{css}</style></svg>"),
            SvgError::UnsupportedCss,
        );
    }
    refused(
        "<svg width='1' height='1'><rect style='fill:url(\\23 a)'/></svg>",
        SvgError::UnsupportedCss,
    );
}

#[test]
fn invalid_tile_and_overlay_bounds_fail_without_rendering() {
    let document =
        SvgDocument::parse(b"<svg width='1' height='1'/>", Limits::default(), &|| false).unwrap();
    for request in [
        TileRequest {
            x: 0.,
            y: 0.,
            scale: 0.,
            width: 1,
            height: 1,
        },
        TileRequest {
            x: f64::NAN,
            y: 0.,
            scale: 1.,
            width: 1,
            height: 1,
        },
        TileRequest {
            x: 0.,
            y: 0.,
            scale: 1.,
            width: 513,
            height: 1,
        },
    ] {
        assert_eq!(
            document.render_tile(request, &|| false),
            Err(SvgError::InvalidTile)
        );
    }
    let invalid = Outline {
        contours: vec![vec![Point { x: 0., y: 0. }; 3]],
        transform: [0.; 6],
        rgba: [0; 4],
    };
    assert_eq!(
        document.export_svg(&[invalid], &|| false),
        Err(SvgError::InvalidOverlay)
    );
}

#[test]
fn diagnostics_never_echo_untrusted_paths_or_content() {
    let failure = SvgDocument::parse(
        b"<svg width='1' height='1'><private-secret>",
        Limits::default(),
        &|| false,
    )
    .err()
    .unwrap();
    assert!(!failure.to_string().contains("private-secret"));
}

#[test]
fn split_or_commented_css_cannot_hide_reference_expansion() {
    for css in [
        "rect{fill:u<!-- split -->rl(#a)}",
        "rect{fill:u<![CDATA[rl(#a)]]>}",
        "rect{fill:/* hidden */url(#a)}",
    ] {
        refused(
            &format!("<svg width='1' height='1'><style>{css}</style></svg>"),
            SvgError::UnsupportedCss,
        );
    }
    // A lowercase URL in the same style must not hide an uppercase one.
    refused(
        "<svg width='1' height='1'><rect style='fill:url(#a);stroke:URL(#b)'/></svg>",
        SvgError::UnsupportedCss,
    );
    refused(
        "<svg width='1' height='1'><rect fill='url(/* hidden */#a)'/></svg>",
        SvgError::UnsupportedCss,
    );
}

#[test]
fn markers_refuse_before_per_vertex_or_nested_definition_expansion() {
    let points = "1,1 ".repeat(20_000);
    for shape in [
        format!("<polyline points='{points}' marker-mid='url(#m)'/>"),
        "<g marker-start='url(#m)'><path d='M0 0L1 1L2 2'/></g>".to_owned(),
        "<style>path{marker:url(#m)}</style><path d='M0 0L1 1'/>".to_owned(),
        "<path style='marker-start:url(#m);marker-end:url(#m)' d='M0 0L1 1'/>".to_owned(),
    ] {
        refused(
            &format!(
                "<svg width='32' height='32'><defs><marker id='m'><path d='M0 0L1 1' marker-end='url(#nested)'/></marker><marker id='nested'><rect width='1' height='1'/></marker></defs>{shape}</svg>"
            ),
            SvgError::Malformed,
        );
    }
    // Unused definitions must not become an admission bypass through CSS/use.
    refused(
        "<svg width='1' height='1'><marker id='unused'/></svg>",
        SvgError::Malformed,
    );
}

#[test]
fn pattern_surfaces_cannot_escape_the_requested_tile_memory_budget() {
    for size in ["1", "1000000000"] {
        refused(
            &format!(
                "<svg width='32' height='32'><defs><pattern id='p' width='{size}' height='{size}' patternUnits='userSpaceOnUse' patternTransform='scale(4096)'><rect width='1' height='1'/></pattern></defs><rect width='32' height='32' fill='url(#p)'/></svg>"
            ),
            SvgError::Malformed,
        );
    }
    refused(
        "<svg width='1' height='1'><pattern id='unused'/></svg>",
        SvgError::Malformed,
    );
}

#[test]
fn quoted_url_delimiters_cannot_hide_the_actual_reference_target() {
    for target in [
        "&quot;#a)b&quot;",
        "&apos;#a)b&apos;",
        "&quot;#a&apos;",
        "#a b",
    ] {
        refused(
            &format!(
                "<svg width='1' height='1'><defs><g id='a)b'><rect width='1' height='1'/></g></defs><rect fill='url({target})'/></svg>"
            ),
            SvgError::UnsupportedCss,
        );
    }
}
