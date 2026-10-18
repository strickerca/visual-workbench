#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use vw_mask::{Combine, Mask, MaskError, Point, Rect, Size};

fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect { Rect { x, y, width, height } }

#[test]
fn ellipse_has_independent_small_area_goldens() {
    // Area of a radius-1/2 circle is pi/4 of one pixel: round(255*pi/4)=200.
    // At radius 1, each of the four quadrants has the same independent area.
    for side in [1, 2] {
        let size = Size::new(side, side).unwrap();
        let mask = Mask::ellipse(size, rect(0.0, 0.0, f64::from(side), f64::from(side))).unwrap();
        assert_eq!(mask.to_dense().unwrap(), vec![200; (side * side) as usize]);
        assert_eq!(Mask::decode_lossless(&mask.encode_lossless().unwrap()).unwrap(), mask);
    }
}

#[test]
fn ellipse_translation_clipping_and_quantization_are_exact() {
    let size = Size::new(6, 5).unwrap();
    let base = Mask::ellipse(size, rect(2.0, 1.0, 2.0, 2.0)).unwrap();
    let values = base.to_dense().unwrap();
    for y in 0..5 { for x in 0..6 {
        assert_eq!(values[y * 6 + x], if (2..4).contains(&x) && (1..3).contains(&y) { 200 } else { 0 });
    } }
    assert_eq!(base, Mask::ellipse(size, rect(2.001, 1.001, 2.0, 2.0)).unwrap());
    let clipped = Mask::ellipse(size, rect(-1.0, -1.0, 2.0, 2.0)).unwrap().to_dense().unwrap();
    assert_eq!(clipped[0], 200);
    assert!(clipped[1..].iter().all(|&v| v == 0));
}

#[test]
fn ellipse_keeps_reflection_symmetry_at_fractional_edges() {
    let size = Size::new(12, 8).unwrap();
    let pixels = Mask::ellipse(size, rect(0.25, 0.25, 11.5, 7.5)).unwrap().to_dense().unwrap();
    for y in 0..8 { for x in 0..12 {
        assert_eq!(pixels[y * 12 + x], pixels[y * 12 + 11 - x]);
        assert_eq!(pixels[y * 12 + x], pixels[(7 - y) * 12 + x]);
    } }
    assert_eq!(pixels[4 * 12 + 6], 255);
}

#[test]
fn polygon_has_exact_rectangle_and_even_odd_hole_answers() {
    let size = Size::new(8, 8).unwrap();
    let mut vertices = vec![Point { x: 1.0, y: 2.0 }, Point { x: 6.0, y: 2.0 },
        Point { x: 6.0, y: 7.0 }, Point { x: 1.0, y: 7.0 }];
    let expected = Mask::rectangle(size, rect(1.0, 2.0, 5.0, 5.0)).unwrap();
    assert_eq!(Mask::polygon(size, &vertices).unwrap(), expected);
    vertices.reverse(); vertices.push(vertices[0]);
    assert_eq!(Mask::polygon(size, &vertices).unwrap(), expected);
    // Traversing a contour twice cancels under even-odd, regardless of winding.
    let twice: Vec<_> = vertices[..4].iter().copied().cycle().take(8).collect();
    assert_eq!(Mask::polygon(size, &twice).unwrap().tile_count(), 0);
}

#[test]
fn new_shapes_combine_with_the_same_coverage_math() {
    let size = Size::new(2, 2).unwrap();
    let ellipse = Mask::ellipse(size, rect(0.0, 0.0, 2.0, 2.0)).unwrap();
    let left = Mask::rectangle(size, rect(0.0, 0.0, 1.0, 2.0)).unwrap();
    assert_eq!(ellipse.combine(&left, Combine::Intersect).unwrap().to_dense().unwrap(), [200, 0, 200, 0]);
    assert_eq!(ellipse.combine(&left, Combine::Subtract).unwrap().to_dense().unwrap(), [0, 200, 0, 200]);
    assert_eq!(ellipse.combine(&left, Combine::Add).unwrap().to_dense().unwrap(), [255, 200, 255, 200]);
}

#[test]
fn ellipse_rejects_bad_geometry_and_work_before_allocating() {
    let size = Size::new(8, 8).unwrap();
    for r in [rect(f64::NAN, 0.0, 2.0, 2.0), rect(0.0, 0.0, -1.0, 2.0),
        rect(0.0, 0.0, f64::INFINITY, 1.0), rect(8_388_608.0, 0.0, 1.0, 1.0)] {
        assert!(matches!(Mask::ellipse(size, r), Err(MaskError::Invalid)));
    }
    for r in [rect(0.0, 0.0, 0.0, 1.0), rect(0.0, 0.0, 1.0, 0.001), rect(10.0, 0.0, 1.0, 1.0)] {
        assert_eq!(Mask::ellipse(size, r).unwrap().tile_count(), 0);
    }
    let tall = Size::new(1, 1_000_000).unwrap();
    assert_eq!(Mask::admit_ellipse(tall, rect(0.0, 0.0, 1.0, 1_000_000.0)), Err(MaskError::Limit));
    assert_eq!(Mask::ellipse(tall, rect(0.0, 0.0, 1.0, 1_000_000.0)), Err(MaskError::Limit));
    assert_eq!(Mask::polygon(size, &vec![Point { x: 0.0, y: 0.0 }; 16_385]), Err(MaskError::Invalid));
}
