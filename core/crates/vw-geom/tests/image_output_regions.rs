#![allow(clippy::unwrap_used)]
use vw_geom::{ImageOutputFrame, QuarterTurn};

#[test]
fn asymmetric_region_has_independent_quarter_turn_goldens() {
    for (rotation, expected) in [
        (QuarterTurn::Zero, [4, 15, 6, 21]),
        (QuarterTurn::Clockwise90, [56, 6, 14, 9]),
        (QuarterTurn::Half, [50, 84, 6, 21]),
        (QuarterTurn::Clockwise270, [10, 75, 14, 9]),
    ] {
        let out = if matches!(rotation, QuarterTurn::Zero | QuarterTurn::Half) {
            [60, 120]
        } else {
            [80, 90]
        };
        let frame = ImageOutputFrame::new([100, 80], [10, 20, 30, 40], rotation, out).unwrap();
        assert_eq!(frame.source_region([12, 25, 3, 7]).unwrap(), expected);
    }
}

#[test]
fn crop_intersection_and_empty_or_invalid_regions_are_explicit() {
    let frame = ImageOutputFrame::new(
        [100, 80],
        [10, 20, 30, 40],
        QuarterTurn::Clockwise90,
        [80, 90],
    )
    .unwrap();
    assert_eq!(
        frame.source_region([0, 0, 100, 80]).unwrap(),
        [0, 0, 80, 90]
    );
    assert_eq!(
        frame.source_region([0, 0, 15, 25]).unwrap(),
        [70, 0, 10, 15]
    );
    for region in [
        [0, 0, 5, 5],
        [40, 20, 5, 5],
        [10, 20, 0, 2],
        [99, 0, 2, 1],
        [u32::MAX, 0, 2, 1],
    ] {
        assert!(frame.source_region(region).is_err());
    }
}

#[test]
fn exact_integer_ratios_and_fractional_edges_cover_only_required_pixels() {
    let frame =
        ImageOutputFrame::new([21, 19], [0, 0, 21, 19], QuarterTurn::Zero, [7, 11]).unwrap();
    assert_eq!(frame.source_region([3, 1, 3, 1]).unwrap(), [1, 0, 1, 2]);
    let frame = ImageOutputFrame::new(
        [32768, 32768],
        [0, 0, 32768, 32768],
        QuarterTurn::Half,
        [32768, 32768],
    )
    .unwrap();
    assert_eq!(
        frame.source_region([32767, 32767, 1, 1]).unwrap(),
        [0, 0, 1, 1]
    );
}
