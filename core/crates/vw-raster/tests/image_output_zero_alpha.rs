#![allow(clippy::unwrap_used)]
use vw_geom::{ImageOutputFrame, QuarterTurn};
use vw_model::AssetId;
use vw_raster::{DecodedImage, Pixels, render_image_output};

#[test]
fn interpolating_fully_transparent_samples_cannot_divide_by_zero_or_reveal_hidden_rgb() {
    for pixels in [
        Pixels::Rgba8(vec![17, 31, 47, 0, 90, 100, 110, 0]),
        Pixels::Rgba16(vec![1700, 3100, 4700, 0, 9000, 10000, 11000, 0]),
    ] {
        let source = DecodedImage {
            width: 2,
            height: 1,
            pixels,
            icc: None,
            source_asset: AssetId::hash(b"synthetic-transparent-original"),
            original_available: true,
            orientation_applied: 1,
        };
        let before = source.clone();
        let frame = ImageOutputFrame::new([2, 1], [0, 0, 2, 1], QuarterTurn::Zero, [1, 1]).unwrap();
        let result = render_image_output(&source, frame, 256 * 1024 * 1024, &|| false).unwrap();
        assert_eq!(
            result.pixels,
            if source.pixels.bit_depth() == 8 {
                Pixels::Rgba8(vec![0; 4])
            } else {
                Pixels::Rgba16(vec![0; 4])
            }
        );
        assert_eq!(source, before);
        assert_eq!(result.source_asset, source.source_asset);
    }
}
