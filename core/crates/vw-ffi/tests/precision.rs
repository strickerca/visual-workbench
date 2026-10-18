use vw_core::*;
type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
fn camera(scale: f64, rotation: f64) -> Camera {
    Camera {
        center: Point {
            x: -73.5,
            y: 991.25,
        },
        scale,
        rotation,
        viewport_width: 1280.0,
        viewport_height: 900.0,
    }
}
fn bits(t: &Transform) -> [u64; 6] {
    [
        t.a.to_bits(),
        t.b.to_bits(),
        t.c.to_bits(),
        t.d.to_bits(),
        t.e.to_bits(),
        t.f.to_bits(),
    ]
}
fn point_close(a: &Point, b: &Point) {
    assert!((a.x - b.x).abs() <= 1e-6, "x");
    assert!((a.y - b.y).abs() <= 1e-6, "y");
}
fn apply(t: &Transform, p: &Point) -> Point {
    Point {
        x: t.a * p.x + t.c * p.y + t.e,
        y: t.b * p.x + t.d * p.y + t.f,
    }
}
#[test]
fn closing_restores_the_exact_saved_matrix_at_every_scale_and_rotation() -> TestResult {
    for scale in [1.0 / 64.0, 0.75, 1.0, 13.0, 64.0] {
        for rotation in [-3.1, -0.5, 0.0, 1.7, 3.1] {
            let c = camera(scale, rotation);
            let lens = create_precision_lens(c)?;
            let before = camera_matrix(c, false)?;
            for mag in [2, 4, 8] {
                lens.configure(
                    Point { x: 20.0, y: 30.0 },
                    Point { x: 500.0, y: 250.0 },
                    mag,
                )?;
                lens.close_lens()?;
                assert_eq!(bits(&before), bits(&lens.mapping()?.lens));
            }
        }
    }

    Ok(())
}
#[test]
fn the_focus_maps_to_inset_center_and_native_samples_return_to_document() -> TestResult {
    let c = camera(1.3, 0.74);
    let lens = create_precision_lens(c)?;
    let focus = Point { x: 100.5, y: 20.25 };
    for mag in [2, 4, 8] {
        let center = Point { x: 450.0, y: 340.0 };
        lens.configure(focus, center, mag)?;
        point_close(&apply(&lens.mapping()?.lens, &focus), &center);
        lens.down(true)?;
        let mapped = lens.map_to_base(vec![center], false)?;
        let doc = camera_map_points(c, true, mapped)?;
        point_close(&doc[0], &focus);
        lens.up()?;
    }

    Ok(())
}
#[test]
fn two_hundred_midstroke_toggle_traces_have_one_unbroken_mapping() -> TestResult {
    for trace in 0..200 {
        let c = camera(
            0.25 + f64::from(trace % 40) / 7.0,
            -3.0 + f64::from(trace) / 37.0,
        );
        let lens = create_precision_lens(c)?;
        lens.configure(Point { x: 13.0, y: 40.0 }, Point { x: 330.0, y: 440.0 }, 4)?;
        lens.down(true)?;
        let points: Vec<_> = (0..60)
            .map(|i| Point {
                x: 10.0 + f64::from(i) * 7.0,
                y: 950.0 - f64::from(i) * 11.0,
            })
            .collect();
        let expected = lens.map_to_base(points.clone(), true)?;
        let mut actual = Vec::new();
        for (i, p) in points.into_iter().enumerate() {
            if i == 17 {
                lens.close_lens()?;
            }
            if i == 29 {
                lens.configure(Point { x: 300.0, y: 50.0 }, Point { x: 70.0, y: 90.0 }, 8)?;
            }
            actual.extend(lens.map_to_base(vec![p], i % 2 == 0)?);
        }
        for (a, b) in actual.iter().zip(&expected) {
            point_close(a, b);
        }
        assert!(lens.mapping()?.latched);
        lens.up()?;
        assert!(!lens.mapping()?.latched);
        assert!(lens.mapping()?.open);
    }

    Ok(())
}
#[test]
fn outside_down_stays_bit_identical_after_crossing_inset_and_queued_open() -> TestResult {
    let lens = create_precision_lens(camera(1.0, 0.0))?;
    lens.down(false)?;
    lens.configure(Point { x: 5.0, y: 6.0 }, Point { x: 500.0, y: 600.0 }, 8)?;
    let raw = vec![
        Point { x: -0.0, y: 0.0 },
        Point {
            x: 1e-12,
            y: -999.125,
        },
    ];
    let got = lens.map_to_base(raw.clone(), true)?;
    for (a, b) in raw.iter().zip(got) {
        assert_eq!(a.x.to_bits(), b.x.to_bits());
        assert_eq!(a.y.to_bits(), b.y.to_bits());
    }
    lens.up()?;
    assert!(lens.mapping()?.open);

    Ok(())
}
#[test]
fn invalid_reconfiguration_is_atomic_and_does_not_replace_queued_close() -> TestResult {
    let lens = create_precision_lens(camera(1.0, 0.3))?;
    lens.configure(Point { x: 1.0, y: 2.0 }, Point { x: 100.0, y: 200.0 }, 4)?;
    lens.down(true)?;
    lens.close_lens()?;
    assert!(
        lens.configure(
            Point {
                x: f64::NAN,
                y: 0.0
            },
            Point { x: 0.0, y: 0.0 },
            4
        )
        .is_err()
    );
    assert!(
        lens.configure(Point { x: 1.0, y: 2.0 }, Point { x: 0.0, y: 0.0 }, 3)
            .is_err()
    );
    lens.up()?;
    assert!(!lens.mapping()?.open);

    Ok(())
}
#[test]
fn render_footprint_ignores_outside_contact_latch_without_changing_input() -> TestResult {
    let lens = create_precision_lens(camera(1.0, 0.0))?;
    lens.configure(Point { x: 0.0, y: 0.0 }, Point { x: 500.0, y: 400.0 }, 4)?;
    lens.down(false)?;
    let input = Point { x: 500.0, y: 400.0 };
    let raw = lens.map_to_base(vec![input], true)?;
    assert_eq!(raw[0].x, input.x);
    let view = lens.view_to_base(vec![input])?;
    assert_ne!(view[0].x, raw[0].x);
    assert!(!lens.mapping()?.lens_input);

    Ok(())
}
#[test]
fn disposal_and_batch_bounds_refuse_without_changing_geometry() -> TestResult {
    let lens = create_precision_lens(camera(1.0, 0.0))?;
    let old = bits(&lens.mapping()?.lens);
    assert!(
        lens.map_to_base(vec![Point { x: 0.0, y: 0.0 }; 513], false)
            .is_err()
    );
    assert!(
        lens.map_to_base(
            vec![Point {
                x: f64::INFINITY,
                y: 0.0
            }],
            false
        )
        .is_err()
    );
    assert_eq!(old, bits(&lens.mapping()?.lens));
    assert!(lens.up().is_err());
    lens.down(false)?;
    assert!(lens.down(true).is_err());
    lens.dispose()?;
    assert!(matches!(lens.mapping(), Err(CoreError::Closed)));
    lens.dispose()?;

    Ok(())
}
#[test]
fn inspection_matrix_never_changes_the_editable_handle() -> TestResult {
    let c = camera(0.875, 0.42);
    let lens = create_precision_lens(c)?;
    lens.down(false)?;
    let before = bits(&lens.mapping()?.lens);
    for i in 0..100 {
        precision_view_transform(
            c,
            Point {
                x: f64::from(i),
                y: 7.0,
            },
            Point { x: 700.0, y: 300.0 },
            4,
        )?;
    }
    assert_eq!(before, bits(&lens.mapping()?.lens));
    assert!(!lens.mapping()?.open);

    Ok(())
}
