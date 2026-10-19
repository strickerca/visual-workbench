mod support;
use support::*;
use vw_raster::*;

const BUDGET: u64 = 256 * 1024 * 1024;

#[test]
fn preview_display_uses_canonical_color_without_granting_original_authority() -> TestResult {
    for depth in [8, 16] {
        let original = source(3, 2, depth);
        let mut preview = original.clone();
        preview.original_available = false;
        let before = preview.clone();
        let expected = display_rgba8(original, BUDGET, true, &|| false)?;
        let shown = display_preview_rgba8(preview.clone(), BUDGET, true, &|| false)?;
        assert_eq!(shown.rgba, expected.rgba);
        assert_eq!(shown.icc_profile, expected.icc_profile);
        assert_eq!(shown.source_asset, before.source_asset);
        assert_eq!(shown.source_bit_depth, depth);
        assert_eq!(
            (shown.width, shown.height, shown.orientation_applied),
            (3, 2, 1)
        );
        assert!(matches!(
            BorrowedSource::new(&preview),
            Err(RasterError::OriginalRequired)
        ));
        assert!(matches!(
            export(&preview, &request(ExportFormat::Png8)),
            Err(RasterError::OriginalRequired)
        ));
        assert_eq!(preview, before);
        assert!(!preview.original_available);
    }
    Ok(())
}

#[test]
fn preview_display_preserves_consent_memory_cancel_shape_and_kind_refusals() {
    let mut preview = source(1, 1, 8);
    preview.original_available = false;
    assert!(matches!(
        display_preview_rgba8(preview.clone(), BUDGET, false, &|| false),
        Err(RasterError::Unsupported(_))
    ));
    assert!(matches!(
        display_preview_rgba8(preview.clone(), 1, true, &|| false),
        Err(RasterError::Memory { .. })
    ));
    assert!(matches!(
        display_preview_rgba8(preview.clone(), BUDGET, true, &|| true),
        Err(RasterError::Cancelled)
    ));
    let mut malformed = preview;
    malformed.pixels = Pixels::Rgba8(vec![0; 3]);
    assert!(matches!(
        display_preview_rgba8(malformed, BUDGET, true, &|| false),
        Err(RasterError::Invalid("pixel buffer length"))
    ));
    assert!(matches!(
        display_preview_rgba8(source(1, 1, 8), BUDGET, true, &|| false),
        Err(RasterError::Invalid(_))
    ));
}
