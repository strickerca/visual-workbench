//! Canonical-geometry regressions adapted from the preserved precision R2 suite.
//! These do not stand in for its FFI, app, or physical-input acceptance cases.
use vw_geom::{Affine, Camera, GeometryError, LensTransform, Point, Size};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn base(scale: f64, rotation: f64) -> Result<Affine, GeometryError> {
    Camera::new(Point::new(-73.5, 991.25)?, scale, rotation)?.view_matrix(Size::new(1280.0, 900.0)?)
}

fn bits(matrix: Affine) -> [u64; 6] {
    matrix.coefficients().map(f64::to_bits)
}

#[test]
fn exact_restore_across_supported_scales_rotations_and_magnifications() -> TestResult {
    for scale in [1.0 / 64.0, 0.75, 1.0, 13.0, 64.0] {
        for rotation in [-3.1, -0.5, 0.0, 1.7, 3.1] {
            let saved = base(scale, rotation)?;
            let mut lens = LensTransform::new(saved)?;
            for magnification in [2, 4, 8] {
                let focus = Point::new(20.0, 30.0)?;
                let center = Point::new(500.0, 250.0)?;
                lens.open_magnified(focus, center, magnification)?;
                let mapped = lens.document_to_input().map(focus)?;
                assert!((mapped.x() - center.x()).abs() <= 1e-6);
                assert!((mapped.y() - center.y()).abs() <= 1e-6);
                lens.latch()?;
                lens.close();
                assert!(lens.is_open());
                lens.unlatch()?;
                assert_eq!(bits(saved), bits(lens.document_to_input()));
            }
        }
    }
    Ok(())
}

#[test]
fn two_hundred_sixty_sample_traces_keep_the_exact_down_inverse() -> TestResult {
    for trace in 0..200 {
        let saved = base(
            0.25 + f64::from(trace % 40) / 7.0,
            -3.0 + f64::from(trace) / 37.0,
        )?;
        let mut lens = LensTransform::new(saved)?;
        lens.open_magnified(Point::new(13.0, 40.0)?, Point::new(330.0, 440.0)?, 4)?;
        let down = lens.document_to_input();
        let inverse = down.inverse()?;
        lens.latch()?;
        for index in 0..60 {
            if index == 17 {
                lens.close();
            }
            if index == 29 {
                lens.open_magnified(Point::new(300.0, 50.0)?, Point::new(70.0, 90.0)?, 8)?;
            }
            let sample = Point::new(
                10.0 + f64::from(index) * 7.0,
                950.0 - f64::from(index) * 11.0,
            )?;
            let expected = inverse.map(sample)?;
            let actual = lens.input_to_document(sample)?;
            assert_eq!(actual.x().to_bits(), expected.x().to_bits());
            assert_eq!(actual.y().to_bits(), expected.y().to_bits());
            assert_eq!(bits(down), bits(lens.document_to_input()));
        }
        lens.unlatch()?;
        assert!(!lens.is_latched() && lens.is_open());
        let focus = lens.document_to_input().map(Point::new(300.0, 50.0)?)?;
        assert!((focus.x() - 70.0).abs() <= 1e-6);
        assert!((focus.y() - 90.0).abs() <= 1e-6);
    }
    Ok(())
}

#[test]
fn opening_during_base_contact_is_deferred_until_release() -> TestResult {
    let saved = base(1.3, 0.74)?;
    let mut lens = LensTransform::new(saved)?;
    lens.latch()?;
    lens.open_magnified(Point::new(100.5, 20.25)?, Point::new(450.0, 340.0)?, 8)?;
    assert!(!lens.is_open());
    let sample = Point::new(450.0, 340.0)?;
    let expected = saved.inverse()?.map(sample)?;
    assert_eq!(lens.input_to_document(sample)?, expected);
    lens.unlatch()?;
    assert!(lens.is_open());
    assert_ne!(lens.input_to_document(sample)?, expected);
    Ok(())
}

#[test]
fn overflowing_reconfiguration_cannot_replace_a_queued_close() -> TestResult {
    let saved = Affine::scale(2.0, 2.0)?;
    let mut lens = LensTransform::new(saved)?;
    lens.open_magnified(Point::new(1.0, 2.0)?, Point::new(100.0, 200.0)?, 4)?;
    lens.latch()?;
    let down = lens.document_to_input();
    lens.close();
    assert_eq!(
        lens.open_magnified(Point::new(f64::MAX, 0.0)?, Point::new(0.0, 0.0)?, 8),
        Err(GeometryError::NonFinite)
    );
    assert_eq!(bits(down), bits(lens.document_to_input()));
    lens.unlatch()?;
    assert!(!lens.is_open());
    assert_eq!(bits(saved), bits(lens.document_to_input()));
    Ok(())
}

#[test]
fn repeated_toggles_and_cancel_restore_original_signed_zero_coefficients() -> TestResult {
    let saved = Affine::new(1.0, -0.0, 0.0, 1.0, -0.0, 17.25)?;
    let mut lens = LensTransform::new(saved)?;
    lens.latch()?;
    for magnification in [2, 4, 8] {
        lens.open_magnified(
            Point::new(1.0, 2.0)?,
            Point::new(100.0, 200.0)?,
            magnification,
        )?;
        lens.close();
    }
    // The same unlatch operation is used by UP and cancellation.
    lens.unlatch()?;
    assert_eq!(bits(saved), bits(lens.document_to_input()));
    assert_eq!(lens.unlatch(), Err(GeometryError::NotLatched));
    Ok(())
}

#[test]
fn inspection_transform_work_cannot_change_the_editable_latch_or_queue() -> TestResult {
    let saved = base(0.875, 0.42)?;
    let mut editable = LensTransform::new(saved)?;
    editable.open_magnified(Point::new(0.0, 7.0)?, Point::new(500.0, 400.0)?, 8)?;
    editable.latch()?;
    editable.close();
    let down = editable.document_to_input();
    for index in 0..100 {
        let mut inspection = LensTransform::new(saved)?;
        inspection.open_magnified(
            Point::new(f64::from(index), 7.0)?,
            Point::new(700.0, 300.0)?,
            4,
        )?;
        assert_eq!(bits(down), bits(editable.document_to_input()));
        assert!(editable.is_latched() && editable.is_open());
    }
    editable.unlatch()?;
    assert_eq!(bits(saved), bits(editable.document_to_input()));
    Ok(())
}
