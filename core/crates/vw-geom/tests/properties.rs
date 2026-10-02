use proptest::{
    prelude::*,
    test_runner::{Config, RngSeed, TestCaseError},
};
use vw_geom::*;

fn checked<T>(result: Result<T, GeometryError>) -> Result<T, TestCaseError> {
    result.map_err(|error| TestCaseError::fail(error.to_string()))
}

fn quarter(value: u8) -> QuarterTurn {
    match value % 4 {
        0 => QuarterTurn::Zero,
        1 => QuarterTurn::Clockwise90,
        2 => QuarterTurn::Half,
        _ => QuarterTurn::Clockwise270,
    }
}

// Explicitly override environment case counts: acceptance requires >=10,000 per
// suite. A fixed seed repeats the same corpus on Windows and physical Android.
// Failures print minimized inputs in retained runner logs; Android has no source
// checkout in which proptest could safely persist its default regression file.
proptest! {
    #![proptest_config(Config {
        cases: 10_000,
        failure_persistence: None,
        rng_seed: RngSeed::Fixed(0x5657_0101_1000),
        ..Config::default()
    })]

    #[test]
    fn camera_screen_round_trip(
        x in -100_000.0f64..100_000.0, y in -100_000.0f64..100_000.0,
        cx in -100_000.0f64..100_000.0, cy in -100_000.0f64..100_000.0,
        scale in 0.015625f64..=64.0,
        angle in -std::f64::consts::PI..=std::f64::consts::PI,
        orientation in 0u8..4, width in 400u32..8000, height in 400u32..8000,
        edges in prop::array::uniform4(0.0f64..100.0),
    ) {
        let point = checked(Point::new(x, y))?;
        let screen = checked(ScreenTransform::new(checked(Size::new(f64::from(width), f64::from(height)))?,
            quarter(orientation), checked(Insets::new(edges[0], edges[1], edges[2], edges[3]))?))?;
        let camera = checked(Camera::new(checked(Point::new(cx, cy))?, scale, angle))?;
        let view = checked(camera.view_matrix(screen.view_size()))?;
        let p = checked(screen.view_to_screen().map(checked(view.map(point))?))?;
        let back = checked(checked(view.inverse())?.map(checked(screen.screen_to_view().map(p))?))?;
        prop_assert!((back.x()-x).abs() <= 1e-6 && (back.y()-y).abs() <= 1e-6);
    }

    #[test]
    fn exif_all_orientations_round_trip(
        w in 1u32..100_000, h in 1u32..100_000,
        u in 0.0f64..=1.0, v in 0.0f64..=1.0,
    ) {
        let size = checked(Size::new(f64::from(w), f64::from(h)))?;
        let point = checked(Point::new(f64::from(w)*u, f64::from(h)*v))?;
        for tag in 1..=8 {
            let orientation = checked(ExifOrientation::from_tag(tag))?;
            let forward = checked(orientation.raw_to_document(size))?;
            let upright = checked(forward.map(point))?;
            let back = checked(checked(orientation.document_to_raw(size))?.map(upright))?;
            prop_assert!((back.x()-point.x()).abs() <= 1e-9 && (back.y()-point.y()).abs() <= 1e-9);
            let extent = orientation.document_size(size);
            prop_assert!(upright.x() >= 0.0 && upright.x() <= extent.width());
            prop_assert!(upright.y() >= 0.0 && upright.y() <= extent.height());
        }
    }

    #[test]
    fn capture_exact_physical_pixels(
        ox in -100_000i32..0, oy in -100_000i32..0,
        width in 1u32..20_000, height in 1u32..20_000,
        xi in any::<u32>(), yi in any::<u32>(), dpi in 1.0f64..=3.0,
        revision in any::<u64>(),
    ) {
        let rect = checked(HostRect::new(PixelPoint { x: ox, y: oy }, width, height))?;
        let revision = GeometryRevision(revision);
        for rotation in 0..4 {
            let geometry = checked(CaptureGeometry::new(rect, dpi, revision, quarter(rotation)))?;
            let [w, h] = geometry.frame_extent();
            let point = PixelPoint { x: (xi % w) as i32, y: (yi % h) as i32 };
            let host = checked(geometry.frame_to_host(point, revision))?;
            prop_assert_eq!(checked(geometry.host_to_frame(host, revision))?, point);
            let unit_dpi = checked(CaptureGeometry::new(rect, 1.0, revision, quarter(rotation)))?;
            prop_assert_eq!(checked(unit_dpi.frame_to_host(point, revision))?, host);
            prop_assert!(host.x >= ox && i64::from(host.x) < i64::from(ox)+i64::from(width));
            prop_assert!(host.y >= oy && i64::from(host.y) < i64::from(oy)+i64::from(height));
        }
    }

    #[test]
    fn snapping_is_bit_idempotent(value in -1e12f64..1e12) {
        let edge = checked(snap_edge(value))?;
        let center = checked(snap_pixel_center(value))?;
        prop_assert_eq!(edge.to_bits(), checked(snap_edge(edge))?.to_bits());
        prop_assert_eq!(center.to_bits(), checked(snap_pixel_center(center))?.to_bits());
        prop_assert!(center - libm::floor(center) == 0.5);
    }

    #[test]
    fn lens_stroke_snapshot_is_immutable(
        cx in -10000.0f64..10000.0, cy in -10000.0f64..10000.0,
        scale in 0.015625f64..=64.0, angle in -std::f64::consts::PI..=std::f64::consts::PI,
        samples in prop::collection::vec((-20000.0f64..20000.0, -20000.0f64..20000.0), 2..32),
        magnification in prop::sample::select(vec![2u8, 4, 8]), close_last in any::<bool>(),
    ) {
        let focus = checked(Point::new(cx, cy))?;
        let base = checked(checked(Camera::new(focus, scale, angle))?.view_matrix(checked(Size::new(1080.0, 2400.0))?))?;
        let mut lens = checked(LensTransform::new(base))?;
        checked(lens.open_magnified(focus, checked(Point::new(350.0, 400.0))?, magnification))?;
        let latched_inverse = checked(lens.document_to_input().inverse())?;
        checked(lens.latch())?;
        for (index, (x, y)) in samples.iter().enumerate() {
            if index % 2 == 0 { lens.close(); } else { checked(lens.open(Affine::IDENTITY))?; }
            let input = checked(Point::new(*x, *y))?;
            let actual = checked(lens.input_to_document(input))?;
            let expected = checked(latched_inverse.map(input))?;
            prop_assert_eq!(actual.x().to_bits(), expected.x().to_bits());
            prop_assert_eq!(actual.y().to_bits(), expected.y().to_bits());
        }
        if close_last { lens.close(); } else { checked(lens.open(Affine::IDENTITY))?; }
        checked(lens.unlatch())?;
        let expected = if close_last { base } else { Affine::IDENTITY };
        prop_assert_eq!(lens.document_to_input().coefficients().map(f64::to_bits), expected.coefficients().map(f64::to_bits));
    }
}
