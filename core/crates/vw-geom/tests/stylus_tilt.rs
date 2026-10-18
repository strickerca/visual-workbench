#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};
use vw_geom::{TiltAxisSign, TiltCalibration, TiltError, WindowsTilt, windows_tilt};

// Synthetic directional convention only; this is not a device calibration.
fn signs() -> Option<TiltCalibration> {
    Some(TiltCalibration {
        x: TiltAxisSign::Direct,
        y: TiltAxisSign::Inverted,
    })
}
fn projected(theta: f64, phi: f64, rotation: f64) -> WindowsTilt {
    windows_tilt(Some(theta), Some(phi), rotation, signs())
        .unwrap()
        .unwrap()
}
fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() <= 1e-10, "{actual} != {expected}");
}

#[test]
fn four_calibrated_directions_have_independent_known_answers() {
    for (phi, x, y) in [
        (0.0, 0.0, -30.0),
        (FRAC_PI_2, 30.0, 0.0),
        (PI, 0.0, 30.0),
        (-FRAC_PI_2, -30.0, 0.0),
    ] {
        let result = projected(PI / 6.0, phi, 0.0);
        near(result.x_degrees, x);
        near(result.y_degrees, y);
        assert_eq!(result.rotation_degrees, 0);
    }
}
#[test]
fn rotation_is_applied_before_projecting_each_tilt_component() {
    let right_in_frame = projected(PI / 6.0, 0.0, FRAC_PI_2);
    near(right_in_frame.x_degrees, 30.0);
    near(right_in_frame.y_degrees, 0.0);
    let down_in_frame = projected(PI / 6.0, FRAC_PI_2, FRAC_PI_2);
    near(down_in_frame.x_degrees, 0.0);
    near(down_in_frame.y_degrees, 30.0);
    let diagonal = projected(FRAC_PI_4, FRAC_PI_4, 0.0);
    near(diagonal.x_degrees, 35.264389682754654);
    near(diagonal.y_degrees, -35.264389682754654);
}
#[test]
fn absent_axes_stay_absent_and_reported_axes_require_calibration() {
    for (tilt, orientation) in [(None, None), (Some(0.0), None), (None, Some(0.0))] {
        assert_eq!(windows_tilt(tilt, orientation, 0.0, None).unwrap(), None);
    }
    assert_eq!(
        windows_tilt(Some(0.0), Some(0.0), 0.0, None),
        Err(TiltError::Uncalibrated)
    );
}
#[test]
fn exact_horizontal_and_float_rounded_endpoints_are_finite_and_bounded() {
    for theta in [0.0, FRAC_PI_2, f64::from(std::f32::consts::FRAC_PI_2)] {
        for (phi, x, y) in [
            (0.0, 0.0, -90.0),
            (FRAC_PI_2, 90.0, 0.0),
            (PI, 0.0, 90.0),
            (-FRAC_PI_2, -90.0, 0.0),
        ] {
            let result = projected(theta, phi, 0.0);
            near(result.x_degrees, if theta == 0.0 { 0.0 } else { x });
            near(result.y_degrees, if theta == 0.0 { 0.0 } else { y });
        }
    }
}
#[test]
fn combined_float_tilt_orientation_and_camera_quarter_turns_have_zero_cross_axis() {
    let theta = f64::from(std::f32::consts::FRAC_PI_2);
    for orientation in -2i32..=2 {
        for rotation in -4i32..=4 {
            let phi = f64::from((f64::from(orientation) * FRAC_PI_2) as f32);
            let camera = f64::from((f64::from(rotation) * FRAC_PI_2) as f32);
            let expected = match (orientation + rotation).rem_euclid(4) {
                0 => (0.0, -90.0),
                1 => (90.0, 0.0),
                2 => (0.0, 90.0),
                _ => (-90.0, 0.0),
            };
            let result = projected(theta, phi, camera);
            near(result.x_degrees, expected.0);
            near(result.y_degrees, expected.1);
        }
    }
}
#[test]
fn neighboring_float_angles_and_small_nonzero_directions_are_not_cardinalized() {
    let right = std::f32::consts::FRAC_PI_2;
    let lower = f64::from(f32::from_bits(right.to_bits() - 1));
    let upper = f64::from(f32::from_bits(right.to_bits() + 1));
    let theta = f64::from(right);
    let below = projected(theta, lower, 0.0);
    let above = projected(theta, upper, 0.0);
    near(below.x_degrees, 90.0);
    near(below.y_degrees, -90.0);
    near(above.x_degrees, 90.0);
    near(above.y_degrees, 90.0);
    let tiny = projected(theta, f64::from(1e-9f32), 0.0);
    near(tiny.x_degrees, 90.0);
    near(tiny.y_degrees, -90.0);
}
#[test]
fn signs_are_explicit_and_view_turns_are_periodic() {
    let positive = projected(FRAC_PI_4, FRAC_PI_2, 0.0);
    let inverse = windows_tilt(
        Some(FRAC_PI_4),
        Some(FRAC_PI_2),
        0.0,
        Some(TiltCalibration {
            x: TiltAxisSign::Inverted,
            y: TiltAxisSign::Direct,
        }),
    )
    .unwrap()
    .unwrap();
    near(inverse.x_degrees, -positive.x_degrees);
    near(inverse.y_degrees, -positive.y_degrees);
    for turns in -4..=4 {
        let periodic = projected(0.7, 0.3 + f64::from(turns) * TAU, -0.2);
        let base = projected(0.7, 0.3, -0.2);
        near(periodic.x_degrees, base.x_degrees);
        near(periodic.y_degrees, base.y_degrees);
    }
}
#[test]
fn malformed_angles_fail_and_large_finite_rotations_do_not_overflow() {
    for theta in [f64::NAN, f64::INFINITY, -0.01, FRAC_PI_2 + 1e-5] {
        assert_eq!(
            windows_tilt(Some(theta), Some(0.0), 0.0, signs()),
            Err(TiltError::Invalid)
        );
    }
    assert_eq!(
        windows_tilt(None, Some(f64::NAN), 0.0, None),
        Err(TiltError::Invalid)
    );
    assert_eq!(
        windows_tilt(None, None, f64::INFINITY, None),
        Err(TiltError::Invalid)
    );
    let large = projected(0.5, f64::MAX, f64::MAX);
    assert!(large.x_degrees.is_finite() && (-90.0..=90.0).contains(&large.x_degrees));
    assert!(large.y_degrees.is_finite() && (-90.0..=90.0).contains(&large.y_degrees));
}
