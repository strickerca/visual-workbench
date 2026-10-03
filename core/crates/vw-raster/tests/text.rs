mod support;
use support::*;
use vw_raster::*;
#[test]
fn shared_fonts_shape_ligatures_and_publish_quantized_outlines() -> TestResult {
    for font in [
        FontFamily::Inter,
        FontFamily::NotoSans,
        FontFamily::NotoSansMono,
    ] {
        let a = layout_text("office AV café Ω Ж", font, 17.)?;
        let b = layout_text("office AV café Ω Ж", font, 17.)?;
        assert_eq!(a, b);
        assert!(!a.glyphs.is_empty());
        assert!(a.width > 50.);
        assert!(a.glyphs.iter().any(|g| !g.outline.is_empty()));
        for glyph in a.glyphs {
            assert_eq!(glyph.x * 256., (glyph.x * 256.).round());
            assert_eq!(glyph.y * 256., (glyph.y * 256.).round());
        }
    }
    // The pinned Inter has no standard `liga` feature. Noto Sans contains the
    // f_f_i substitution, so this checks shaping against the actual font data.
    let ligatures = layout_text("ffi", FontFamily::NotoSans, 20.)?;
    assert_eq!(ligatures.glyphs.len(), 1);
    assert_eq!(ligatures.glyphs[0].cluster, 0);
    assert_eq!(ligatures.glyphs[0].font, FontFamily::NotoSans);
    Ok(())
}
#[test]
fn mixed_script_direction_and_newlines_keep_logical_clusters() -> TestResult {
    let text = "abc \u{202e}123\u{202c} Ω Ж\nsecond";
    let result = layout_text(text, FontFamily::NotoSans, 18.)?;
    assert!(
        result
            .glyphs
            .iter()
            .all(|g| text.is_char_boundary(g.cluster))
    );
    let digits = result
        .glyphs
        .iter()
        .filter(|g| matches!(text[g.cluster..].chars().next(), Some('1'..='3')))
        .map(|g| g.cluster)
        .collect::<Vec<_>>();
    assert_eq!(digits.len(), 3);
    assert!(digits[0] > digits[1] && digits[1] > digits[2]);
    assert!(result.glyphs.iter().any(|g| g.y >= result.line_height));
    assert_eq!(result.height, result.line_height * 2.);
    Ok(())
}
#[test]
fn invalid_fonts_glyphs_controls_and_resource_limits_are_explicit() -> TestResult {
    assert!(FontFamily::parse("Arial").is_err());
    assert!(matches!(
        layout_text("\u{10ffff}", FontFamily::Inter, 12.),
        Err(RasterError::MissingGlyph(0x10ffff))
    ));
    assert!(layout_text("x", FontFamily::Inter, f32::NAN).is_err());
    assert!(layout_text("a\tb", FontFamily::Inter, 12.).is_err());
    assert!(layout_text(&"a".repeat(65537), FontFamily::Inter, 12.).is_err());
    assert!(layout_text("x", FontFamily::Inter, 5000.).is_err());
    Ok(())
}

#[test]
fn fallback_keeps_combining_marks_and_joining_controls_with_their_base() -> TestResult {
    // The bundled Inter lacks U+0255 while Noto Sans covers it. Both fonts have
    // U+0301, so selecting fonts per character incorrectly splits this cluster.
    let inter = rustybuzz::Face::from_slice(FontFamily::Inter.bytes(), 0).ok_or("Inter")?;
    let noto = rustybuzz::Face::from_slice(FontFamily::NotoSans.bytes(), 0).ok_or("Noto Sans")?;
    assert!(inter.glyph_index('\u{0255}').is_none());
    assert!(inter.glyph_index('\u{0301}').is_some());
    assert!(noto.glyph_index('\u{0255}').is_some());
    for text in [
        "\u{0255}\u{0301}",
        "\u{0255}\u{200d}\u{0255}",
        "a\u{0255}\u{0301}",
    ] {
        let fallback = layout_text(text, FontFamily::Inter, 24.)?;
        let explicit = layout_text(text, FontFamily::NotoSans, 24.)?;
        assert_eq!(fallback, explicit, "fallback changed the shaping context");
        assert!(
            fallback
                .glyphs
                .iter()
                .all(|glyph| glyph.font == FontFamily::NotoSans)
        );
    }
    Ok(())
}
