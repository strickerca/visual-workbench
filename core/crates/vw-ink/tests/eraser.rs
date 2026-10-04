#![allow(clippy::unwrap_used)] // Fixture failures must fail the test immediately.
use std::cell::Cell;
use vw_ink::eraser::*;
use vw_ink::{Brush, BrushFamily, Polygon, QPoint, Sample, StrokeBuilder};
use vw_proto::v1 as pb;

const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
fn p(x: f64, y: f64) -> pb::PointD {
    pb::PointD { x, y }
}
fn q(x: i64, y: i64) -> QPoint {
    QPoint {
        x: x * 256,
        y: y * 256,
    }
}
fn square(x: i64, y: i64, width: i64, height: i64) -> Polygon {
    Polygon {
        points: vec![
            q(x, y),
            q(x + width, y),
            q(x + width, y + height),
            q(x, y + height),
        ],
    }
}
fn erased(source: &[Polygon], path: &[pb::PointD], radius: f64) -> ErasedOutline {
    erase_contours(
        source,
        path,
        radius,
        IDENTITY,
        EraseLimits::default(),
        &|| false,
    )
    .unwrap()
}
// Independent winding oracle: examines the serialized compound path, without
// using the clipping engine's convex hull, hit test or half-plane routine.
fn winding(path: &pb::Polyline, x: f64, y: f64) -> i32 {
    let mut count = 0;
    for index in 0..path.points.len() {
        let a = &path.points[index];
        let b = &path.points[(index + 1) % path.points.len()];
        let side = (b.x - a.x) * (y - a.y) - (x - a.x) * (b.y - a.y);
        if a.y <= y && b.y > y && side > 0.0 {
            count += 1;
        }
        if a.y > y && b.y <= y && side < 0.0 {
            count -= 1;
        }
    }
    count
}
fn compound(value: &ErasedOutline) -> pb::Polyline {
    compound_polygon(&value.contours, EraseLimits::default(), &|| false).unwrap()
}

#[test]
fn sweep_splits_a_strip_and_keeps_both_sides_as_vector_coverage() {
    let output = erased(
        &[square(-10, -3, 20, 6)],
        &[p(0.0, -20.0), p(0.0, 20.0)],
        2.0,
    );
    assert!(output.changed);
    assert!(output.contours.len() >= 2);
    let wire = compound(&output);
    assert_ne!(winding(&wire, -5.0, 0.1), 0);
    assert_ne!(winding(&wire, 5.0, 0.1), 0);
    assert_eq!(winding(&wire, 0.0, 0.1), 0);
    for y in [-2.5, 0.1, 2.5] {
        for x in [-9.5, -5.0, 5.0, 9.5] {
            assert_ne!(winding(&wire, x, y), 0);
        }
    }
}

#[test]
fn interior_erase_makes_a_hole_without_an_outer_fill_or_bridge_line() {
    let output = erased(&[square(-10, -10, 20, 20)], &[p(0.0, 0.0)], 3.0);
    let wire = compound(&output);
    for (x, y) in [(0.0, 0.0), (1.0, 1.0), (-1.0, -1.0)] {
        assert_eq!(winding(&wire, x, y), 0);
    }
    for (x, y) in [
        (-9.0, -9.0),
        (9.0, 9.0),
        (-9.0, 9.0),
        (9.0, -9.0),
        (5.0, 0.1),
    ] {
        assert_ne!(winding(&wire, x, y), 0);
    }
    assert!(wire.closed);
}

#[test]
fn repeated_erase_decodes_compound_contours_and_preserves_the_first_hole() {
    let first = erased(&[square(-12, -10, 24, 20)], &[p(-5.0, 0.0)], 2.0);
    let source =
        contours_from_compound(&compound(&first), EraseLimits::default(), &|| false).unwrap();
    assert_eq!(source, first.contours);
    let second = erased(&source, &[p(5.0, 0.0)], 2.0);
    let wire = compound(&second);
    assert_eq!(winding(&wire, -5.0, 0.0), 0);
    assert_eq!(winding(&wire, 5.0, 0.0), 0);
    assert_ne!(winding(&wire, 0.0, 0.1), 0);
    assert_ne!(winding(&wire, 10.0, 0.1), 0);
}

#[test]
fn overlaps_remain_one_nonzero_fill_instead_of_separate_alpha_passes() {
    let source = vec![square(-10, -3, 15, 6), square(-5, -3, 15, 6)];
    let result = erased(&source, &[p(7.0, 0.0)], 1.0);
    let wire = compound(&result);
    assert_eq!(winding(&wire, 0.1, 0.1), 2);
    // The model is one filled path. Winding magnitude never requests a second
    // alpha blend; renderers use NONZERO, while the hole still has zero winding.
    assert_eq!(winding(&wire, 7.0, 0.1), 0);
    assert_eq!(
        contours_from_compound(&wire, EraseLimits::default(), &|| false).unwrap(),
        result.contours
    );
}

#[test]
fn a_complete_erase_returns_no_outline_and_no_empty_invalid_polygon() {
    let result = erased(&[square(-1, -1, 2, 2)], &[p(0.0, 0.0)], 4.0);
    assert!(result.changed);
    assert!(result.contours.is_empty());
    assert_eq!(
        compound_polygon(&[], EraseLimits::default(), &|| false),
        Err(EraseError::Invalid)
    );
}

#[test]
fn no_hit_is_byte_identical_and_does_not_reinitialize_stabilization() {
    let mut brush = Brush::new(BrushFamily::Pen, 7.0).unwrap();
    brush.stabilization = 0.85;
    let mut builder = StrokeBuilder::begin(brush).unwrap();
    let samples = (0..80)
        .map(|n| {
            Sample::new(
                f64::from(n),
                f64::from(n % 9),
                n * 4,
                f64::from(n % 7 + 1) / 8.0,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    builder.append(&samples).unwrap();
    let original = builder.into_geometry().unwrap();
    let result = erased(original.polygons(), &[p(500.0, 500.0)], 8.0);
    assert!(!result.changed);
    assert_eq!(result.contours, original.polygons());
}

#[test]
fn untouched_canonical_stroke_contours_keep_every_quantized_vertex() {
    let brush = Brush::new(BrushFamily::Marker, 2.0).unwrap();
    let mut builder = StrokeBuilder::begin(brush).unwrap();
    builder
        .append(
            &(0..30)
                .map(|n| Sample::new(f64::from(n) * 3.0, 0.0, n * 8, 1.0).unwrap())
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let original = builder.into_geometry().unwrap();
    let output = erased(original.polygons(), &[p(40.0, -5.0), p(40.0, 5.0)], 2.0);
    assert!(output.changed);
    for polygon in original
        .polygons()
        .iter()
        .filter(|p| p.points.iter().all(|v| v.x_px() < 30.0 || v.x_px() > 50.0))
    {
        assert!(output.contours.contains(polygon));
    }
}

#[test]
fn inverse_affine_uses_elliptical_local_sweep_and_keeps_source_coordinates() {
    // Object maps local (x,y) to D (2*x+10, y/2+20).
    let result = erase_contours(
        &[square(-10, -10, 20, 20)],
        &[p(10.0, 20.0)],
        2.0,
        [0.5, 0.0, 0.0, 2.0, -5.0, -40.0],
        EraseLimits::default(),
        &|| false,
    )
    .unwrap();
    let wire = compound(&result);
    assert_eq!(winding(&wire, 0.0, 3.0), 0);
    assert_ne!(winding(&wire, 2.0, 0.1), 0);
    assert_ne!(winding(&wire, 0.1, 6.0), 0);
}

#[test]
fn reflected_rotated_and_sheared_transforms_do_not_reverse_the_cut() {
    // Local = (-D.y + D.x/2, D.x); determinant positive. Reflect x separately.
    for map in [
        [0.5, 1.0, -1.0, 0.0, 0.0, 0.0],
        [-0.5, 1.0, 1.0, 0.0, 0.0, 0.0],
    ] {
        let result = erase_contours(
            &[square(-20, -20, 40, 40)],
            &[p(4.0, 2.0)],
            1.5,
            map,
            EraseLimits::default(),
            &|| false,
        )
        .unwrap();
        let wire = compound(&result);
        assert_eq!(winding(&wire, 0.0, 4.0), 0);
        assert_ne!(winding(&wire, 12.0, 12.0), 0);
    }
}

#[test]
fn tiny_and_large_coordinates_keep_exact_quantized_output() {
    let shift = 999_999_000;
    let result = erased(
        &[square(shift, shift, 20, 20)],
        &[p((shift + 10) as f64, (shift + 10) as f64)],
        1.0,
    );
    let wire = compound(&result);
    assert!(
        wire.points
            .iter()
            .all(|p| p.x * 256.0 == (p.x * 256.0).round() && p.y * 256.0 == (p.y * 256.0).round())
    );
    assert_eq!(winding(&wire, (shift + 10) as f64, (shift + 10) as f64), 0);
}

#[test]
fn malformed_compound_and_nonconvex_input_are_refused() {
    let malformed = pb::Polyline {
        points: vec![p(0.0, 0.0), p(4.0, 4.0), p(0.0, 4.0), p(4.0, 0.0)],
        closed: true,
    };
    assert_eq!(
        contours_from_compound(&malformed, EraseLimits::default(), &|| false),
        Err(EraseError::Invalid)
    );
    let off_grid = pb::Polyline {
        points: vec![p(0.0, 0.0), p(4.0001, 0.0), p(0.0, 4.0)],
        closed: true,
    };
    assert_eq!(
        contours_from_compound(&off_grid, EraseLimits::default(), &|| false),
        Err(EraseError::Invalid)
    );
    let source = vec![Polygon {
        points: vec![q(0, 0), q(4, 0), q(1, 1), q(4, 4), q(0, 4)],
    }];
    assert!(matches!(
        erase_contours(
            &source,
            &[p(1.0, 1.0)],
            1.0,
            IDENTITY,
            EraseLimits::default(),
            &|| false
        ),
        Err(EraseError::Invalid)
    ));
}

#[test]
fn work_and_vertex_caps_refuse_without_mutating_source() {
    let source = vec![square(-10, -10, 20, 20)];
    let original = source.clone();
    let limited = EraseLimits {
        max_vertices: 4096,
        max_work: 100,
    };
    assert!(matches!(
        erase_contours(&source, &[p(0.0, 0.0)], 1.0, IDENTITY, limited, &|| false),
        Err(EraseError::Limit)
    ));
    let limited = EraseLimits {
        max_vertices: 8,
        max_work: MAX_ERASER_WORK,
    };
    assert!(matches!(
        erase_contours(&source, &[p(0.0, 0.0)], 3.0, IDENTITY, limited, &|| false),
        Err(EraseError::Limit)
    ));
    assert_eq!(source, original);
}

#[test]
fn cancellation_is_checked_during_clipping_and_leaves_input_owned() {
    let source = vec![square(-10, -10, 20, 20)];
    let calls = Cell::new(0);
    let cancelled = || {
        calls.set(calls.get() + 1);
        calls.get() > 8
    };
    assert!(matches!(
        erase_contours(
            &source,
            &[p(0.0, 0.0)],
            2.0,
            IDENTITY,
            EraseLimits::default(),
            &cancelled
        ),
        Err(EraseError::Cancelled)
    ));
    assert_eq!(source, vec![square(-10, -10, 20, 20)]);
}

#[test]
fn invalid_radius_matrix_and_nan_never_produce_geometry() {
    let source = vec![square(0, 0, 4, 4)];
    for radius in [f64::NAN, 0.0, -1.0, 257.0] {
        assert!(
            erase_contours(
                &source,
                &[p(1.0, 1.0)],
                radius,
                IDENTITY,
                EraseLimits::default(),
                &|| false
            )
            .is_err()
        );
    }
    assert!(
        erase_contours(
            &source,
            &[p(f64::NAN, 0.0)],
            1.0,
            IDENTITY,
            EraseLimits::default(),
            &|| false
        )
        .is_err()
    );
    assert!(
        erase_contours(
            &source,
            &[p(1.0, 1.0)],
            1.0,
            [0.0; 6],
            EraseLimits::default(),
            &|| false
        )
        .is_err()
    );
}

#[test]
fn allocation_bound_dominates_canonical_replay_for_all_supported_brush_families() {
    for family in [
        BrushFamily::Pen,
        BrushFamily::Marker,
        BrushFamily::Highlighter,
    ] {
        for width in [0.25, 3.0, 64.0, 4096.0] {
            let mut brush = Brush::new(family, width).unwrap();
            brush.stabilization = 0.6;
            let count = 20;
            let raw = pb::Stroke {
                brush: Some(brush.to_proto()),
                x: (0..count).map(|n| (n % 4) as f64 * 3.0).collect(),
                y: (0..count).map(|n| (n % 3) as f64 * 7.0).collect(),
                t_ms: (0..count).map(|n| n * 7).collect(),
                pressure: (0..count).map(|n| (n % 5) as f32 / 4.0).collect(),
                tilt: vec![],
                orientation: vec![],
            };
            let bound = stroke_vertex_bound(&raw).unwrap();
            let geometry = vw_ink::geometry_from_stroke(&raw).unwrap();
            assert!(geometry.vertex_count() <= bound, "{family:?}/{width}");
        }
    }
}

#[test]
fn malformed_contour_headers_cannot_bypass_a_tiny_vertex_or_work_budget() {
    let source = vec![Polygon { points: vec![] }; 1024];
    let limits = EraseLimits {
        max_vertices: 3,
        max_work: 1,
    };
    assert!(matches!(
        erase_contours(&source, &[p(0.0, 0.0)], 1.0, IDENTITY, limits, &|| false),
        Err(EraseError::Limit)
    ));
    assert_eq!(
        compound_polygon(&source, limits, &|| false),
        Err(EraseError::Limit)
    );
    assert_eq!(source.len(), 1024);
    assert!(source.iter().all(|p| p.points.is_empty()));
}
