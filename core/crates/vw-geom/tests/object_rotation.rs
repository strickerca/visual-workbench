#![allow(clippy::unwrap_used)]
use vw_geom::{Affine, GeometryError, Point, object_rotation};

fn p(x: f64, y: f64) -> Point {
    Point::new(x, y).unwrap()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-12, "{a} != {b}");
}

#[test]
fn clockwise_quarter_turn_keeps_pivot_and_maps_axes_exactly() {
    let rotation = object_rotation(p(10.0, 20.0), p(10.0, 10.0), p(20.0, 20.0), false).unwrap();
    assert_eq!(rotation.map(p(10.0, 20.0)).unwrap(), p(10.0, 20.0));
    assert_eq!(rotation.map(p(10.0, 10.0)).unwrap(), p(20.0, 20.0));
    assert_eq!(rotation.coefficients(), [0.0, 1.0, -1.0, 0.0, 30.0, 10.0]);
}
#[test]
fn counterclockwise_and_half_turns_have_exact_coefficients() {
    assert_eq!(
        object_rotation(p(0.0, 0.0), p(1.0, 0.0), p(0.0, -1.0), true)
            .unwrap()
            .coefficients(),
        [0.0, -1.0, 1.0, 0.0, 0.0, 0.0]
    );
    assert_eq!(
        object_rotation(p(0.0, 0.0), p(1.0, 0.0), p(-1.0, 0.0), true)
            .unwrap()
            .coefficients(),
        [-1.0, 0.0, 0.0, -1.0, 0.0, 0.0]
    );
}
#[test]
fn snapping_uses_fifteen_degree_delta_and_can_be_released_without_accumulation() {
    let angle = 13.0_f64.to_radians();
    let current = p(libm::cos(angle), libm::sin(angle));
    let snapped = object_rotation(p(0.0, 0.0), p(1.0, 0.0), current, true)
        .unwrap()
        .coefficients();
    near(snapped[0], libm::cos(core::f64::consts::PI / 12.0));
    near(snapped[1], libm::sin(core::f64::consts::PI / 12.0));
    let free = object_rotation(p(0.0, 0.0), p(1.0, 0.0), current, false)
        .unwrap()
        .coefficients();
    near(free[0], libm::cos(angle));
    near(free[1], libm::sin(angle));
    let small = 7.0_f64.to_radians();
    assert_eq!(
        object_rotation(
            p(0.0, 0.0),
            p(1.0, 0.0),
            p(libm::cos(small), libm::sin(small)),
            true
        )
        .unwrap(),
        Affine::IDENTITY
    );
}
#[test]
fn branch_cut_uses_the_short_delta() {
    let a = 170.0_f64.to_radians();
    let b = (-170.0_f64).to_radians();
    let value = object_rotation(
        p(0.0, 0.0),
        p(libm::cos(a), libm::sin(a)),
        p(libm::cos(b), libm::sin(b)),
        false,
    )
    .unwrap();
    let expected = 20.0_f64.to_radians();
    near(value.coefficients()[0], libm::cos(expected));
    near(value.coefficients()[1], libm::sin(expected));
}

#[test]
fn snapping_thresholds_are_symmetric_and_measure_delta_from_a_non_axis_start() {
    for sign in [-1.0, 1.0] {
        for (degrees, expected) in [(7.49_f64, 0.0_f64), (7.51, 15.0)] {
            let radians = sign * degrees.to_radians();
            let actual = object_rotation(
                p(0.0, 0.0),
                p(1.0, 0.0),
                p(libm::cos(radians), libm::sin(radians)),
                true,
            )
            .unwrap()
            .coefficients();
            near(actual[0], libm::cos(sign * expected.to_radians()));
            near(actual[1], libm::sin(sign * expected.to_radians()));
        }
    }
    // Radius is irrelevant and snapping applies to gesture delta, not the
    // pointer's absolute angle (45 degrees -> 58 degrees snaps by 15).
    let current = 58.0_f64.to_radians();
    let actual = object_rotation(
        p(0.0, 0.0),
        p(10.0, 10.0),
        p(2.0 * libm::cos(current), 2.0 * libm::sin(current)),
        true,
    )
    .unwrap()
    .coefficients();
    near(actual[0], libm::cos(core::f64::consts::PI / 12.0));
    near(actual[1], libm::sin(core::f64::consts::PI / 12.0));
}
#[test]
fn composition_keeps_reflection_shear_and_scale_without_refiltering_geometry() {
    let original = Affine::new(-2.0, 1.0, 0.5, 3.0, 8.0, 9.0).unwrap();
    let outer = object_rotation(p(4.0, 5.0), p(4.0, 2.0), p(7.0, 5.0), false).unwrap();
    let result = original.then(outer).unwrap();
    for point in [p(0.0, 0.0), p(-4.0, 3.0), p(8.0, 2.0)] {
        assert_eq!(
            result.map(point).unwrap(),
            outer.map(original.map(point).unwrap()).unwrap()
        );
    }
    let [a, b, c, d, _, _] = result.coefficients();
    near(a * d - b * c, -6.5);
}
#[test]
fn very_large_and_tiny_vectors_do_not_overflow_the_angle_products() {
    assert_eq!(
        object_rotation(p(0.0, 0.0), p(1e300, 0.0), p(0.0, 1e300), false)
            .unwrap()
            .coefficients(),
        [0.0, 1.0, -1.0, 0.0, 0.0, 0.0]
    );
    assert_eq!(
        object_rotation(p(0.0, 0.0), p(1e-300, 0.0), p(0.0, -1e-300), false)
            .unwrap()
            .coefficients(),
        [0.0, -1.0, 1.0, 0.0, 0.0, 0.0]
    );
}
#[test]
fn pivot_contact_and_overflowing_subtraction_refuse() {
    assert_eq!(
        object_rotation(p(1.0, 2.0), p(1.0, 2.0), p(2.0, 2.0), false),
        Err(GeometryError::EmptyExtent)
    );
    assert_eq!(
        object_rotation(p(1.0, 2.0), p(2.0, 2.0), p(1.0, 2.0), false),
        Err(GeometryError::EmptyExtent)
    );
    assert_eq!(
        object_rotation(p(-1e308, 0.0), p(1e308, 0.0), p(0.0, 1.0), false),
        Err(GeometryError::NonFinite)
    );
}
#[test]
fn identical_vectors_produce_identity_even_with_a_far_away_pivot() {
    assert_eq!(
        object_rotation(
            p(1e20, -1e20),
            p(1e20 + 1e6, -1e20 + 2e6),
            p(1e20 + 1e6, -1e20 + 2e6),
            false
        )
        .unwrap(),
        Affine::IDENTITY
    );
}
