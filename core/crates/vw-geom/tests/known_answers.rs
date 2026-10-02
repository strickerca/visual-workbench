use vw_geom::*;

type TestResult = Result<(), GeometryError>;

fn near(actual: Point, x: f64, y: f64) {
    assert!((actual.x() - x).abs() < 1e-9, "x: {} vs {x}", actual.x());
    assert!((actual.y() - y).abs() < 1e-9, "y: {} vs {y}", actual.y());
}

#[test]
fn affine_order_inverse_bounds_and_extreme_scales() -> TestResult {
    let a = Affine::translation(10.0, -20.0)?.then(Affine::scale(2.0, 3.0)?)?;
    near(a.map(Point::new(4.0, 5.0)?)?, 28.0, -45.0);
    near(a.inverse()?.map(Point::new(28.0, -45.0)?)?, 4.0, 5.0);
    let bounds = QuarterTurn::Clockwise90
        .matrix(Size::new(10.0, 20.0)?)?
        .map_rect(Rect::new(1.0, 2.0, 3.0, 4.0)?)?;
    assert_eq!(bounds, Rect::new(14.0, 1.0, 4.0, 3.0)?);
    for scale in [1e-200, 1e200] {
        let inverse = Affine::scale(scale, scale)?.inverse()?;
        near(inverse.map(Point::new(scale, scale)?)?, 1.0, 1.0);
    }
    near(
        Affine::scale(1e-200, 1e200)?
            .inverse()?
            .map(Point::new(1e-200, 1e200)?)?,
        1.0,
        1.0,
    );
    assert_eq!(
        Affine::new(1.0, 2.0, 2.0, 4.0, 0.0, 0.0)?.inverse(),
        Err(GeometryError::Singular)
    );
    assert_eq!(
        Affine::scale(0.0, 0.0)?.inverse(),
        Err(GeometryError::Singular)
    );
    Ok(())
}

#[test]
fn exif_known_pixel_centers_all_eight() -> TestResult {
    let raw = Size::new(4.0, 3.0)?;
    let expected = [
        (0.5, 1.5),
        (3.5, 1.5),
        (3.5, 1.5),
        (0.5, 1.5),
        (1.5, 0.5),
        (1.5, 0.5),
        (1.5, 3.5),
        (1.5, 3.5),
    ];
    // Also use an asymmetric off-center point: avoids reflection ambiguities.
    let edges = [
        (1.0, 0.0),
        (3.0, 0.0),
        (3.0, 3.0),
        (1.0, 3.0),
        (0.0, 1.0),
        (3.0, 1.0),
        (3.0, 3.0),
        (0.0, 3.0),
    ];
    for (index, ((x, y), (ex, ey))) in expected.into_iter().zip(edges).enumerate() {
        let matrix = ExifOrientation::from_tag(index as u8 + 1)?.raw_to_document(raw)?;
        near(matrix.map(Point::new(0.5, 1.5)?)?, x, y);
        near(matrix.map(Point::new(1.0, 0.0)?)?, ex, ey);
    }
    assert_eq!(
        ExifOrientation::from_tag(0),
        Err(GeometryError::InvalidOrientation)
    );
    assert_eq!(
        ExifOrientation::from_tag(9),
        Err(GeometryError::InvalidOrientation)
    );
    Ok(())
}

#[test]
fn camera_and_rotated_screen_with_asymmetric_insets() -> TestResult {
    let screen = ScreenTransform::new(
        Size::new(1080.0, 2400.0)?,
        QuarterTurn::Clockwise90,
        Insets::new(10.0, 20.0, 30.0, 40.0)?,
    )?;
    assert_eq!(screen.view_size(), Size::new(1020.0, 2360.0)?);
    near(
        screen
            .natural_input_to_screen()
            .map(Point::new(0.0, 0.0)?)?,
        2400.0,
        0.0,
    );
    near(
        screen
            .screen_to_natural_input()
            .map(Point::new(2400.0, 0.0)?)?,
        0.0,
        0.0,
    );
    near(
        screen.view_to_screen().map(Point::new(0.0, 0.0)?)?,
        2370.0,
        20.0,
    );
    near(
        screen.view_to_screen().map(Point::new(1020.0, 2360.0)?)?,
        10.0,
        1040.0,
    );
    for scale in [1.0 / 64.0, 64.0] {
        let camera = Camera::new(Point::new(-5.0, 10.0)?, scale, std::f64::consts::FRAC_PI_2)?;
        let matrix = camera.view_matrix(Size::new(100.0, 200.0)?)?;
        near(matrix.map(camera.center())?, 50.0, 100.0);
        near(matrix.map(Point::new(-4.0, 10.0)?)?, 50.0, 100.0 + scale);
    }
    assert_eq!(QuarterTurn::from_degrees(-90)?, QuarterTurn::Clockwise270);
    assert_eq!(QuarterTurn::from_degrees(720)?, QuarterTurn::Zero);
    assert_eq!(
        QuarterTurn::from_degrees(45),
        Err(GeometryError::InvalidOrientation)
    );
    Ok(())
}

#[test]
fn pdf_crop_y_flip_and_all_rotations() -> TestResult {
    let crop = Rect::new(-10.0, 20.0, 100.0, 200.0)?;
    let expected = [(0.0, 0.0), (200.0, 0.0), (100.0, 200.0), (0.0, 100.0)];
    for (degrees, (x, y)) in [0, 90, 180, 270].into_iter().zip(expected) {
        let page = PdfPage::new(crop, QuarterTurn::from_degrees(degrees)?)?;
        near(page.raw_to_document().map(Point::new(-10.0, 220.0)?)?, x, y);
        near(
            page.document_to_raw()?.map(Point::new(x, y)?)?,
            -10.0,
            220.0,
        );
    }
    Ok(())
}

#[test]
fn svg_meet_slice_stretch_and_flagged_fallback() -> TestResult {
    let bounds = Rect::new(10.0, 20.0, 100.0, 50.0)?;
    for (fit, x, y) in [
        (SvgFit::Meet, 0.0, 50.0),
        (SvgFit::Slice, -100.0, 0.0),
        (SvgFit::Stretch, 0.0, 0.0),
    ] {
        let svg = SvgViewport::new(Some(Size::new(200.0, 200.0)?), Some(bounds), None, fit)?;
        near(svg.user_to_document().map(Point::new(10.0, 20.0)?)?, x, y);
        near(svg.document_to_user()?.map(Point::new(x, y)?)?, 10.0, 20.0);
        assert!(!svg.used_content_bounds());
    }
    let fallback = SvgViewport::new(None, None, Some(bounds), SvgFit::Meet)?;
    assert!(fallback.used_content_bounds());
    assert_eq!(fallback.document_size(), Size::new(100.0, 50.0)?);
    near(
        fallback.user_to_document().map(Point::new(10.0, 20.0)?)?,
        0.0,
        0.0,
    );
    assert_eq!(
        SvgViewport::new(None, None, None, SvgFit::Meet),
        Err(GeometryError::MissingSvgBounds)
    );
    Ok(())
}

#[test]
fn capture_rotation_edges_stale_revisions_and_overflow() -> TestResult {
    let rect = HostRect::new(PixelPoint { x: -1920, y: -1080 }, 3, 2)?;
    let revision = GeometryRevision(7);
    let expected = [
        (-1920, -1080),
        (-1918, -1080),
        (-1918, -1079),
        (-1920, -1079),
    ];
    for (degrees, (x, y)) in [0, 90, 180, 270].into_iter().zip(expected) {
        let geometry =
            CaptureGeometry::new(rect, 2.5, revision, QuarterTurn::from_degrees(degrees)?)?;
        assert_eq!(
            geometry.frame_to_host(PixelPoint { x: 0, y: 0 }, revision)?,
            PixelPoint { x, y }
        );
        let [w, h] = geometry.frame_extent();
        for fy in 0..h {
            for fx in 0..w {
                let frame = PixelPoint {
                    x: fx as i32,
                    y: fy as i32,
                };
                assert_eq!(
                    geometry.host_to_frame(geometry.frame_to_host(frame, revision)?, revision)?,
                    frame
                );
            }
        }
        assert!(matches!(
            geometry.frame_to_host(PixelPoint { x: 0, y: 0 }, GeometryRevision(8)),
            Err(GeometryError::StaleGeometry { .. })
        ));
        assert_eq!(
            geometry.frame_to_host(PixelPoint { x: w as i32, y: 0 }, revision),
            Err(GeometryError::OutOfBounds)
        );
        assert_eq!(
            geometry.host_to_frame(PixelPoint { x: -1921, y: -1080 }, revision),
            Err(GeometryError::OutOfBounds)
        );
    }
    assert!(
        HostRect::new(
            PixelPoint {
                x: i32::MAX,
                y: i32::MIN
            },
            1,
            1
        )
        .is_ok()
    );
    assert_eq!(
        HostRect::new(PixelPoint { x: i32::MAX, y: 0 }, 2, 1),
        Err(GeometryError::IntegerOverflow)
    );
    Ok(())
}

#[test]
fn lens_queue_restore_and_rejected_changes_are_atomic() -> TestResult {
    let base = Affine::new(2.0, 0.5, -0.5, 2.0, -200.0, 50.0)?;
    let mut lens = LensTransform::new(base)?;
    assert_eq!(lens.unlatch(), Err(GeometryError::NotLatched));
    lens.open_magnified(Point::new(10.0, 20.0)?, Point::new(300.0, 400.0)?, 4)?;
    near(
        lens.document_to_input().map(Point::new(10.0, 20.0)?)?,
        300.0,
        400.0,
    );
    let before = lens.document_to_input();
    lens.latch()?;
    assert_eq!(lens.latch(), Err(GeometryError::AlreadyLatched));
    lens.close();
    assert_eq!(
        lens.open(Affine::scale(0.0, 0.0)?),
        Err(GeometryError::Singular)
    );
    assert_eq!(lens.document_to_input(), before);
    near(
        lens.input_to_document(Point::new(300.0, 400.0)?)?,
        10.0,
        20.0,
    );
    lens.unlatch()?;
    assert!(!lens.is_latched() && !lens.is_open());
    assert_eq!(
        lens.document_to_input().coefficients().map(f64::to_bits),
        base.coefficients().map(f64::to_bits)
    );
    assert_eq!(
        lens.open_magnified(Point::new(0.0, 0.0)?, Point::new(0.0, 0.0)?, 3),
        Err(GeometryError::InvalidMagnification)
    );
    Ok(())
}

#[test]
fn invalid_numbers_extents_and_snapping_boundaries() -> TestResult {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(Point::new(bad, 0.0), Err(GeometryError::NonFinite));
        assert_eq!(Affine::rotation(bad), Err(GeometryError::NonFinite));
        assert_eq!(snap_edge(bad), Err(GeometryError::NonFinite));
        assert_eq!(snap_pixel_center(bad), Err(GeometryError::NonFinite));
    }
    assert_eq!(Size::new(0.0, 1.0), Err(GeometryError::EmptyExtent));
    assert_eq!(
        Rect::new(0.0, 0.0, -1.0, 1.0),
        Err(GeometryError::NegativeExtent)
    );
    assert_eq!(
        Camera::new(Point::new(0.0, 0.0)?, 0.0, 0.0),
        Err(GeometryError::InvalidScale)
    );
    assert_eq!(
        Insets::new(-1.0, 0.0, 0.0, 0.0),
        Err(GeometryError::InvalidInsets)
    );
    assert_eq!(
        ScreenTransform::new(
            Size::new(10.0, 10.0)?,
            QuarterTurn::Zero,
            Insets::new(5.0, 0.0, 5.0, 0.0)?
        ),
        Err(GeometryError::InvalidInsets)
    );
    assert_eq!(snap_edge(-0.0)?.to_bits(), 0.0f64.to_bits());
    assert_eq!(snap_edge(-1.5)?, -2.0);
    assert_eq!(snap_pixel_center(-1.0)?, -0.5);
    assert_eq!(snap_pixel_center(0.0)?, 0.5);
    assert_eq!(
        snap_pixel_center(4_503_599_627_370_496.0),
        Err(GeometryError::IntegerOverflow)
    );
    assert_eq!(
        Affine::scale(f64::MAX, 1.0)?.map(Point::new(2.0, 0.0)?),
        Err(GeometryError::NonFinite)
    );
    Ok(())
}
