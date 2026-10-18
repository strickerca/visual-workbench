#![allow(clippy::unwrap_used, clippy::expect_used)]
use vw_geom::{Affine, ImageOutputFrame, Point, QuarterTurn, Rect};

#[test]
fn crop_turn_and_nonuniform_resize_map_known_edges_and_centers_once() {
    let frame = ImageOutputFrame::new(
        [100, 80],
        [10, 20, 30, 40],
        QuarterTurn::Clockwise90,
        [80, 90],
    )
    .unwrap();
    assert_eq!(
        frame.point(Point::new(10.0, 20.0).unwrap()).unwrap(),
        Point::new(80.0, 0.0).unwrap()
    );
    assert_eq!(
        frame.point(Point::new(40.0, 60.0).unwrap()).unwrap(),
        Point::new(0.0, 90.0).unwrap()
    );
    assert_eq!(
        frame.point(Point::new(10.5, 20.5).unwrap()).unwrap(),
        Point::new(79.0, 1.5).unwrap()
    );
    assert_eq!(
        frame
            .bounds(Rect::new(12.0, 25.0, 3.0, 7.0).unwrap())
            .unwrap(),
        Rect::new(56.0, 6.0, 14.0, 9.0).unwrap()
    );
}
#[test]
fn every_quarter_turn_pixel_center_agrees_with_continuous_affine() {
    for rotation in [
        QuarterTurn::Zero,
        QuarterTurn::Clockwise90,
        QuarterTurn::Half,
        QuarterTurn::Clockwise270,
    ] {
        let output = if matches!(rotation, QuarterTurn::Zero | QuarterTurn::Half) {
            [3, 2]
        } else {
            [2, 3]
        };
        let frame = ImageOutputFrame::new([9, 7], [2, 3, 3, 2], rotation, output).unwrap();
        for y in 0..output[1] {
            for x in 0..output[0] {
                let [sx, sy] = frame.source_pixel(x, y).unwrap();
                assert_eq!(
                    frame
                        .point(Point::new(f64::from(sx) + 0.5, f64::from(sy) + 0.5).unwrap())
                        .unwrap(),
                    Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5).unwrap()
                );
            }
        }
    }
}

#[test]
fn asymmetric_crop_has_independent_clockwise_pixel_goldens() {
    let cases = [
        (
            QuarterTurn::Zero,
            [3, 2],
            vec![[2, 3], [3, 3], [4, 3], [2, 4], [3, 4], [4, 4]],
        ),
        (
            QuarterTurn::Clockwise90,
            [2, 3],
            vec![[2, 4], [2, 3], [3, 4], [3, 3], [4, 4], [4, 3]],
        ),
        (
            QuarterTurn::Half,
            [3, 2],
            vec![[4, 4], [3, 4], [2, 4], [4, 3], [3, 3], [2, 3]],
        ),
        (
            QuarterTurn::Clockwise270,
            [2, 3],
            vec![[4, 3], [4, 4], [3, 3], [3, 4], [2, 3], [2, 4]],
        ),
    ];
    for (rotation, size, expected) in cases {
        let frame = ImageOutputFrame::new([9, 7], [2, 3, 3, 2], rotation, size).unwrap();
        let mut actual = Vec::new();
        for y in 0..size[1] {
            for x in 0..size[0] {
                actual.push(frame.source_pixel(x, y).unwrap());
            }
        }
        assert_eq!(actual, expected);
        assert!(frame.source_pixel(size[0], 0).is_err());
        assert!(frame.source_pixel(0, size[1]).is_err());
    }
}
#[test]
fn canonical_crop_affine_roundtrips_independent_of_layer_names() {
    for rotation in [
        QuarterTurn::Zero,
        QuarterTurn::Clockwise90,
        QuarterTurn::Half,
        QuarterTurn::Clockwise270,
    ] {
        for output in [[1, 1], [17, 29], [32768, 32768]] {
            let frame =
                ImageOutputFrame::new([32768, 20000], [999, 37, 22001, 19777], rotation, output)
                    .unwrap();
            let rect = Rect::new(999.0, 37.0, 22001.0, 19777.0).unwrap();
            assert_eq!(
                ImageOutputFrame::from_crop_affine(
                    frame.source_size(),
                    rect,
                    frame.matrix().unwrap()
                )
                .unwrap(),
                frame
            );
        }
    }
}
#[test]
fn invalid_extent_fractional_crop_shear_reflection_and_translation_refuse() {
    assert!(ImageOutputFrame::new([100, 100], [99, 0, 2, 1], QuarterTurn::Zero, [1, 1]).is_err());
    assert!(ImageOutputFrame::new([100, 100], [0, 0, 1, 1], QuarterTurn::Zero, [0, 1]).is_err());
    assert!(
        ImageOutputFrame::from_crop_affine(
            [100, 100],
            Rect::new(0.5, 0.0, 10.0, 10.0).unwrap(),
            Affine::IDENTITY
        )
        .is_err()
    );
    for coefficients in [
        [1.0, 0.1, 0.0, 1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0, 1.0, 10.0, 0.0],
        [1.0, 0.0, 0.0, 1.0, 0.001, 0.0],
        [0.125, 0.0, 0.0, 1.0, 0.0, 0.0],
    ] {
        let [a, b, c, d, e, f] = coefficients;
        assert!(
            ImageOutputFrame::from_crop_affine(
                [100, 100],
                Rect::new(0.0, 0.0, 10.0, 10.0).unwrap(),
                Affine::new(a, b, c, d, e, f).unwrap()
            )
            .is_err()
        );
    }
}
#[test]
fn projected_rectangle_bounds_use_all_four_corners_and_crop_containment_is_explicit() {
    let frame = ImageOutputFrame::new(
        [20, 30],
        [2, 4, 10, 12],
        QuarterTurn::Clockwise270,
        [24, 30],
    )
    .unwrap();
    let rect = Rect::new(4.0, 5.0, 3.0, 7.0).unwrap();
    assert!(frame.contains(rect));
    assert_eq!(
        frame.bounds(rect).unwrap(),
        Rect::new(2.0, 15.0, 14.0, 9.0).unwrap()
    );
    assert!(!frame.contains(Rect::new(0.0, 5.0, 3.0, 7.0).unwrap()));
    let inverse = frame.matrix().unwrap().inverse().unwrap();
    for [x, y] in [[2.0, 4.0], [12.0, 16.0], [7.25, 10.75]] {
        let point = Point::new(x, y).unwrap();
        let back = inverse.map(frame.point(point).unwrap()).unwrap();
        assert!((back.x() - x).abs() < 1e-12 && (back.y() - y).abs() < 1e-12);
    }
}
