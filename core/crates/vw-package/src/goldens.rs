use crate::{ImageMapping, NeverCancel, Target, geometry, pixels};
use vw_raster::{DecodedImage, Pixels};
type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
#[test]
fn integer_area_alpha_golden_avoids_transparent_color_fringe() -> TestResult {
    let source = DecodedImage {
        width: 2,
        height: 1,
        pixels: Pixels::Rgba8(vec![255, 0, 0, 255, 0, 0, 255, 0]),
        icc: None,
        source_asset: vw_model::AssetId::hash(b"unit pixels"),
        original_available: true,
        orientation_applied: 1,
    };
    let map = ImageMapping::new([0, 0, 2, 1], 1, 1)?;
    let frame = pixels::resample(&source, &map, 64 * 1024 * 1024, &NeverCancel)?;
    // Uniform 1/2 scale creates half-row white context; RGB averages only
    // premultiplied visible color: red opaque area1/4 + white opaque area1/2.
    assert_eq!(frame.bytes, vec![255, 170, 170, 191]);
    Ok(())
}
#[test]
fn outline_and_badge_are_exact_three_and_twenty_four_pixel_geometry() -> TestResult {
    let mut frame = pixels::Frame {
        width: 128,
        height: 128,
        bytes: vec![255; 128 * 128 * 4],
    };
    let map = ImageMapping::new([0, 0, 128, 128], 256, 64)?;
    frame.marker(
        1,
        vw_instructions::Role::Change,
        [20.0, 50.0, 40.0, 30.0],
        &map,
    );
    let pixel = |x: usize, y: usize| &frame.bytes[(y * 128 + x) * 4..(y * 128 + x + 1) * 4];
    for x in 20..60 {
        for y in 50..53 {
            assert_eq!(pixel(x, y), &[240, 60, 50, 255]);
        }
    }
    assert_eq!(pixel(30, 53), &[255; 4]);
    assert_eq!(pixel(20, 26), &[255; 4]);
    assert_eq!(pixel(21, 27), &[240, 60, 50, 255]);
    assert_eq!(pixel(21, 48), &[240, 60, 50, 255]);
    assert_eq!(pixel(20, 49), &[255; 4]);
    Ok(())
}
#[test]
fn rational_mapping_crop_and_timestamp_goldens() -> TestResult {
    let m = ImageMapping::new([0, 0, 400, 300], 256, 64)?;
    assert_eq!(
        (
            m.scale_numerator,
            m.scale_denominator,
            m.content_width,
            m.content_height
        ),
        (256, 400, 256, 192)
    );
    assert_eq!(
        m.point(
            [100.0, 75.0],
            &Target::Gemini {
                model: "configured".into(),
                max_long_edge: 256
            }
        ),
        [250.0, 250.0]
    );
    assert_eq!(
        geometry::crop([100.0, 70.0, 40.0, 30.0], 400, 300, 256)?.source_rectangle,
        [0, 0, 256, 256]
    );
    assert_eq!(crate::config::timestamp(0)?, "1970-01-01T00:00:00.000Z");
    assert_eq!(
        crate::config::timestamp(951_782_400_000)?,
        "2000-02-29T00:00:00.000Z"
    );
    Ok(())
}
