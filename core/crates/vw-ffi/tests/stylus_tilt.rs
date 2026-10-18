#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::f64::consts::{FRAC_PI_2, PI};
use vw_core::{StylusTiltCalibration, StylusTiltError, StylusTiltSign, project_stylus_tilt};

fn signs() -> Option<StylusTiltCalibration> {
    Some(StylusTiltCalibration {
        x: StylusTiltSign::Direct,
        y: StylusTiltSign::Inverted,
    })
}

#[test]
fn ffi_preserves_optional_axes_and_distinguishes_calibration_refusal() {
    assert!(
        project_stylus_tilt(None, None, 0.0, None)
            .unwrap()
            .is_none()
    );
    assert!(
        project_stylus_tilt(Some(0.5), None, 0.0, signs())
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        project_stylus_tilt(Some(0.5), Some(0.0), 0.0, None),
        Err(StylusTiltError::Uncalibrated)
    ));
    assert!(matches!(
        project_stylus_tilt(Some(-0.5), Some(0.0), 0.0, signs()),
        Err(StylusTiltError::Invalid)
    ));
}

#[test]
fn ffi_applies_frame_rotation_and_returns_degrees_without_fabricated_twist() {
    let value = project_stylus_tilt(Some(PI / 6.0), Some(0.0), FRAC_PI_2, signs())
        .unwrap()
        .unwrap();
    assert!((value.x_degrees - 30.0).abs() < 1e-10);
    assert_eq!(value.y_degrees, 0.0);
    assert_eq!(value.rotation_degrees, 0);
}
