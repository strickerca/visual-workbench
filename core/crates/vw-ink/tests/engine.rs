//! All inputs below are synthetic software fixtures. Hardware acceptance is separate.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "../fixtures/synthetic.rs"]
mod synthetic;

use proptest::prelude::*;
use serde::Deserialize;
use std::hint::black_box;
use std::time::Instant;
use vw_ink::{
    ALGORITHM_VERSION, Brush, BrushFamily, InkError, MAX_BASE_WIDTH, MAX_COORDINATE, MAX_SAMPLES,
    PressureCurve, QPoint, Sample, StrokeBuilder, StrokeState, geometry_from_stroke,
    samples_from_stroke,
};
use vw_proto::v1 as pb;

fn sample(x: f64, y: f64, time: u32, pressure: f64) -> Sample {
    Sample::new(x, y, time, pressure).unwrap()
}
fn pen(width: f64) -> Brush {
    Brush::new(BrushFamily::Pen, width).unwrap()
}
fn model(brush: &Brush, samples: &[Sample]) -> pb::Stroke {
    pb::Stroke {
        x: samples.iter().map(|s| s.x).collect(),
        y: samples.iter().map(|s| s.y).collect(),
        t_ms: samples.iter().map(|s| s.t_ms).collect(),
        pressure: samples.iter().map(|s| s.pressure as f32).collect(),
        tilt: if samples.iter().all(|s| s.tilt.is_some()) {
            samples
                .iter()
                .filter_map(|s| s.tilt.map(|v| v as f32))
                .collect()
        } else {
            Vec::new()
        },
        orientation: if samples.iter().all(|s| s.orientation.is_some()) {
            samples
                .iter()
                .filter_map(|s| s.orientation.map(|v| v as f32))
                .collect()
        } else {
            Vec::new()
        },
        brush: Some(brush.to_proto()),
    }
}
#[derive(Deserialize)]
struct GoldenFile {
    algorithm_version: u32,
    provenance: String,
    fixtures: Vec<Golden>,
}
#[derive(Deserialize)]
struct Golden {
    name: String,
    samples: usize,
    polygons: usize,
    vertices: usize,
    geometry_hash: String,
}

#[test]
fn committed_synthetic_goldens_match_every_fixture() {
    let expected: GoldenFile =
        serde_json::from_str(include_str!("../fixtures/goldens.json")).unwrap();
    let fixtures = synthetic::fixtures().unwrap();
    assert_eq!(expected.algorithm_version, ALGORITHM_VERSION);
    assert!(expected.provenance.starts_with("synthetic-only"));
    assert_eq!(
        expected.fixtures.len(),
        fixtures.len(),
        "native runner must bind every synthetic golden before acceptance"
    );
    for (fixture, golden) in fixtures.into_iter().zip(expected.fixtures) {
        assert_eq!(fixture.name, golden.name);
        let mut builder = StrokeBuilder::begin(fixture.brush).unwrap();
        builder.append(&fixture.samples).unwrap();
        let geometry = builder.finish().unwrap();
        let hash = geometry.hash().unwrap();
        assert_eq!(hash.as_str(), golden.geometry_hash, "{}", fixture.name);
        assert_eq!(golden.samples, fixture.samples.len());
        assert_eq!(golden.polygons, geometry.polygons().len());
        assert_eq!(golden.vertices, geometry.vertex_count());
        println!(
            "ink_golden name={} hash={} samples={} vertices={}",
            fixture.name,
            hash.as_str(),
            fixture.samples.len(),
            geometry.vertex_count()
        );
    }
}

#[test]
fn every_incremental_batch_matches_final_and_published_prefix_exactly() {
    for fixture in synthetic::fixtures().unwrap() {
        let mut complete = StrokeBuilder::begin(fixture.brush.clone()).unwrap();
        complete.append(&fixture.samples).unwrap();
        let expected = complete.finish().unwrap();
        for chunk in [1, 2, 3, 7, 32, 256] {
            let mut builder = StrokeBuilder::begin(fixture.brush.clone()).unwrap();
            let mut published = Vec::new();
            for samples in fixture.samples.chunks(chunk) {
                let previous = builder.geometry().polygons().to_vec();
                let delta = builder.append(samples).unwrap();
                assert_eq!(delta.start, previous.len());
                assert_eq!(&builder.geometry().polygons()[..delta.start], &previous);
                assert_eq!(delta.end, builder.geometry().polygons().len());
                published.extend_from_slice(&builder.geometry().polygons()[delta.start..delta.end]);
            }
            let wet = builder.geometry().canonical_bytes().unwrap();
            let dry = builder.finish().unwrap();
            assert_eq!(published, dry.polygons());
            assert_eq!(wet, dry.canonical_bytes().unwrap());
            assert_eq!(builder.geometry(), &dry);
            assert_eq!(dry, expected, "{} chunk={chunk}", fixture.name);
        }
    }
}

#[test]
fn saved_model_replay_matches_live_precision_and_optional_axes() {
    let mut brush = pen(9.0);
    brush.stabilization = 0.75;
    let mut samples = vec![
        sample(0.0, 0.0, 0, 0.3),
        sample(0.0, 0.0, 0, 0.30000000001),
        sample(9.0, 3.0, 8, 0.99999999),
    ];
    for s in &mut samples {
        s.tilt = Some(0.4);
        s.orientation = Some(-0.8);
    }
    let mut live = StrokeBuilder::begin(brush.clone()).unwrap();
    live.append(&samples).unwrap();
    let stored = model(&brush, &samples);
    let replay = geometry_from_stroke(&stored).unwrap();
    assert_eq!(live.geometry(), &replay);
    let restored = samples_from_stroke(&stored).unwrap();
    assert_eq!(restored[0].tilt, Some(f64::from(0.4f32)));
    assert_eq!(restored[0].orientation, Some(f64::from(-0.8f32)));
    assert_eq!(
        Brush::try_from(stored.brush.as_ref().unwrap()).unwrap(),
        brush
    );
}

#[test]
fn invalid_append_batch_leaves_geometry_filter_and_count_unchanged() {
    let mut builder = StrokeBuilder::begin(pen(4.0)).unwrap();
    builder.append(&[sample(0.0, 0.0, 10, 0.5)]).unwrap();
    let before = builder.geometry().clone();
    let count = builder.sample_count();
    let mut invalid = sample(5.0, 5.0, 12, 1.0);
    invalid.pressure = f64::NAN;
    assert!(matches!(
        builder.append(&[sample(4.0, 2.0, 11, 1.0), invalid]),
        Err(InkError::Invalid(_))
    ));
    assert_eq!(builder.geometry(), &before);
    assert_eq!(builder.sample_count(), count);
    assert_eq!(
        builder.append(&[sample(4.0, 2.0, 9, 1.0)]),
        Err(InkError::TimeOrder)
    );
    assert_eq!(builder.geometry(), &before);
    let mut clone = builder.clone();
    let next = [sample(8.0, 4.0, 20, 0.5)];
    builder.append(&next).unwrap();
    clone.append(&next).unwrap();
    assert_eq!(builder.geometry(), clone.geometry());
}

#[test]
fn brush_version_width_curve_and_stabilization_fail_closed() {
    for width in [0.0, -1.0, f64::NAN, f64::INFINITY, MAX_BASE_WIDTH + 1.0] {
        assert!(Brush::new(BrushFamily::Pen, width).is_err());
    }
    for version in [0, 2, u32::MAX] {
        let mut b = pen(4.0);
        b.algorithm_version = version;
        assert!(matches!(
            StrokeBuilder::begin(b),
            Err(InkError::UnsupportedVersion(_))
        ));
    }
    for stabilization in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
        let mut b = pen(4.0);
        b.stabilization = stabilization;
        assert!(StrokeBuilder::begin(b).is_err());
    }
    for controls in [
        [0.1, 0.0, 0.3, 0.3, 0.7, 0.7, 1.0, 1.0],
        [0.0, 0.0, 0.8, 0.3, 0.2, 0.7, 1.0, 1.0],
        [0.0, 0.0, 0.3, 0.8, 0.7, 0.2, 1.0, 1.0],
        [0.0, 0.0, 0.3, 0.3, 0.7, 0.7, 0.9, 1.0],
        [0.0, 0.0, 0.3, f64::NAN, 0.7, 0.7, 1.0, 1.0],
    ] {
        assert!(PressureCurve::new(controls).is_err());
    }
    let mut wire = pen(4.0).to_proto();
    wire.pressure_curve.pop();
    assert!(Brush::try_from(&wire).is_err());
    wire = pen(4.0).to_proto();
    wire.family = "unknown".into();
    assert!(Brush::try_from(&wire).is_err());
}

#[test]
fn pressure_curve_is_monotonic_including_degenerate_endpoint_controls() {
    for controls in [
        [0.0, 0.0, 0.0, 0.25, 0.0, 0.75, 1.0, 1.0],
        [0.0, 0.2, 1.0, 0.3, 1.0, 0.6, 1.0, 0.9],
        [0.0, 0.0, 0.2, 0.8, 0.8, 0.9, 1.0, 1.0],
    ] {
        let curve = PressureCurve::new(controls).unwrap();
        let mut previous = 0.0;
        for n in 0..=4096 {
            let y = curve.evaluate(f64::from(n) / 4096.0).unwrap();
            assert!((0.0..=1.0).contains(&y));
            assert!(y >= previous);
            previous = y;
        }
        assert_eq!(curve.evaluate(0.0).unwrap(), controls[1]);
        assert_eq!(curve.evaluate(1.0).unwrap(), controls[7]);
    }
    assert_eq!(PressureCurve::linear().evaluate(0.25).unwrap(), 0.25);
    assert_eq!(PressureCurve::constant().evaluate(0.0).unwrap(), 1.0);
    for p in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
        assert!(PressureCurve::linear().evaluate(p).is_err());
    }
}

#[test]
fn flat_highlighter_caps_and_canonical_bytes_have_known_integer_answer() {
    let mut b = StrokeBuilder::begin(Brush::new(BrushFamily::Highlighter, 2.0).unwrap()).unwrap();
    assert!(b.append(&[sample(0.0, 0.0, 0, 1.0)]).unwrap().is_empty());
    b.append(&[sample(10.0, 0.0, 1, 1.0)]).unwrap();
    let g = b.finish().unwrap();
    assert_eq!(g.polygons().len(), 1);
    let points = vec![
        QPoint { x: 0, y: -256 },
        QPoint { x: 2560, y: -256 },
        QPoint { x: 2560, y: 256 },
        QPoint { x: 0, y: 256 },
    ];
    assert_eq!(g.polygons()[0].points, points);
    let bounds = g.bounds().unwrap();
    assert_eq!(
        (bounds.min_x, bounds.min_y, bounds.max_x, bounds.max_y),
        (0.0, -1.0, 10.0, 1.0)
    );
    let mut bytes = b"VisualWorkbench.InkGeometry.v1\0".to_vec();
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.push(3);
    bytes.extend_from_slice(&1u64.to_le_bytes());
    bytes.extend_from_slice(&4u32.to_le_bytes());
    for point in points {
        bytes.extend_from_slice(&point.x.to_le_bytes());
        bytes.extend_from_slice(&point.y.to_le_bytes());
    }
    assert_eq!(g.canonical_bytes().unwrap(), bytes);
    assert!(!g.hit_test(-0.001, 0.0, 0.0).unwrap());
    assert!(g.hit_test(0.0, 0.0, 0.0).unwrap());
}

#[test]
fn dense_short_highlighter_keeps_both_flat_caps_without_interior_disks() {
    let brush = Brush::new(BrushFamily::Highlighter, 6.0).unwrap();
    let samples = [
        sample(0.0, 0.0, 0, 1.0),
        sample(0.125, 0.0, 1, 1.0),
        sample(0.25, 0.0, 2, 1.0),
    ];
    let mut builder = StrokeBuilder::begin(brush.clone()).unwrap();
    for value in &samples {
        builder.append(std::slice::from_ref(value)).unwrap();
    }
    let geometry = builder.finish().unwrap();
    let bounds = geometry.bounds().unwrap();
    assert_eq!(
        (bounds.min_x, bounds.max_x, bounds.min_y, bounds.max_y),
        (0.0, 0.25, -3.0, 3.0)
    );
    assert_eq!(geometry.polygons().len(), 2);
    assert!(!geometry.hit_test(-0.01, 0.0, 0.0).unwrap());
    assert!(!geometry.hit_test(0.26, 0.0, 0.0).unwrap());
    let mut sparse = StrokeBuilder::begin(brush).unwrap();
    sparse.append(&[samples[0], samples[2]]).unwrap();
    assert_eq!(sparse.geometry().bounds(), geometry.bounds());
}

#[test]
fn highlighter_turn_joins_only_the_outside_sector_and_preserves_prefix() {
    for direction in [-1.0, 1.0] {
        let brush = Brush::new(BrushFamily::Highlighter, 6.0).unwrap();
        let samples = [
            sample(0.0, 0.0, 0, 1.0),
            sample(0.25, 0.0, 1, 1.0),
            sample(0.25, direction * 0.25, 2, 1.0),
        ];
        let mut incremental = StrokeBuilder::begin(brush.clone()).unwrap();
        incremental.append(&samples[..2]).unwrap();
        let prefix = incremental.geometry().polygons().to_vec();
        let appended = incremental.append(&samples[2..]).unwrap();
        assert_eq!(&incremental.geometry().polygons()[..appended.start], prefix);
        assert_eq!(incremental.geometry().polygons().len(), 3);
        let join = &incremental.geometry().polygons()[2];
        assert!(
            join.points
                .iter()
                .all(|point| point.x >= 64 && point.y as f64 * direction <= 0.0)
        );
        assert!(
            incremental
                .geometry()
                .hit_test(2.0, -2.0 * direction, 0.0)
                .unwrap()
        );
        let live = incremental.geometry().canonical_bytes().unwrap();
        assert_eq!(
            incremental.finish().unwrap().canonical_bytes().unwrap(),
            live
        );
        let mut batch = StrokeBuilder::begin(brush).unwrap();
        batch.append(&samples).unwrap();
        assert_eq!(batch.geometry().canonical_bytes().unwrap(), live);
    }
}

#[test]
fn highlighter_reversal_adds_a_semicircle_without_rounding_the_start_cap() {
    let mut builder =
        StrokeBuilder::begin(Brush::new(BrushFamily::Highlighter, 6.0).unwrap()).unwrap();
    builder
        .append(&[
            sample(0.0, 0.0, 0, 1.0),
            sample(0.25, 0.0, 1, 1.0),
            sample(0.0, 0.0, 2, 1.0),
        ])
        .unwrap();
    assert_eq!(builder.geometry().bounds().unwrap().min_x, 0.0);
    assert!(
        builder.geometry().polygons()[2]
            .points
            .iter()
            .all(|point| point.x >= 64)
    );
    assert!(builder.geometry().hit_test(3.0, 0.0, 0.0).unwrap());
}

#[test]
fn highlighter_stationary_pressure_change_joins_the_actual_incoming_and_outgoing_widths() {
    let mut brush = Brush::new(BrushFamily::Highlighter, 8.0).unwrap();
    brush.pressure_curve = PressureCurve::linear();
    let samples = [
        sample(0.0, 0.0, 0, 0.25),
        sample(8.0, 0.0, 1, 0.5),
        sample(8.0, 0.0, 2, 1.0),
        sample(8.0, 8.0, 3, 0.5),
    ];
    let mut batch = StrokeBuilder::begin(brush.clone()).unwrap();
    batch.append(&samples).unwrap();
    let join = &batch.geometry().polygons()[2];
    assert!(join.points.contains(&QPoint { x: 2048, y: -512 }));
    assert!(join.points.contains(&QPoint { x: 3072, y: 0 }));
    assert!(
        join.points
            .iter()
            .all(|point| point.x >= 2048 && point.y <= 0)
    );
    for chunk in 1..=3 {
        let mut incremental = StrokeBuilder::begin(brush.clone()).unwrap();
        for part in samples.chunks(chunk) {
            incremental.append(part).unwrap();
        }
        assert_eq!(incremental.finish().unwrap(), *batch.geometry());
    }
}

#[test]
fn round_caps_bounds_and_distance_use_the_actual_quantized_fill() {
    let mut b = StrokeBuilder::begin(pen(2.0)).unwrap();
    b.append(&[sample(0.0, 0.0, 0, 1.0), sample(10.0, 0.0, 10, 1.0)])
        .unwrap();
    let g = b.finish().unwrap();
    let bounds = g.bounds().unwrap();
    assert_eq!(
        (bounds.min_x, bounds.min_y, bounds.max_x, bounds.max_y),
        (-1.0, -1.0, 11.0, 1.0)
    );
    assert_eq!(g.distance_to(5.0, 0.0).unwrap(), 0.0);
    assert_eq!(g.distance_to(5.0, 2.0).unwrap(), 1.0);
    assert_eq!(g.distance_to(12.0, 0.0).unwrap(), 1.0);
    assert!(g.hit_test(-1.0, 0.0, 0.0).unwrap());
    assert!(!g.hit_test(12.0, 0.0, 0.99).unwrap());
    assert!(g.hit_test(12.0, 0.0, 1.0).unwrap());
    assert!(g.hit_test(f64::NAN, 0.0, 1.0).is_err());
    assert!(g.hit_test(0.0, 0.0, -1.0).is_err());
    assert!(g.distance_to(f64::INFINITY, 0.0).is_err());
}

#[test]
fn rounded_taper_containment_and_zero_radius_do_not_create_gaps() {
    let mut b = StrokeBuilder::begin(pen(20.0)).unwrap();
    b.append(&[
        sample(0.0, 0.0, 0, 0.0),
        sample(1.0, 0.0, 1, 1.0),
        sample(2.0, 0.0, 2, 0.1),
    ])
    .unwrap();
    let g = b.finish().unwrap();
    assert!(g.hit_test(-8.5, 0.0, 0.0).unwrap());
    assert!(g.hit_test(10.5, 0.0, 0.0).unwrap());
    assert!(g.polygons().iter().all(|polygon| polygon.points.len() >= 3));
}

#[test]
fn finish_cancel_empty_and_clone_preview_have_explicit_lifecycles() {
    let mut b = StrokeBuilder::begin(pen(3.0)).unwrap();
    assert!(b.append(&[]).unwrap().is_empty());
    assert_eq!(b.finish(), Err(InkError::EmptyStroke));
    b.append(&[sample(0.0, 0.0, 0, 1.0)]).unwrap();
    let before = b.geometry().clone();
    let mut prediction = b.clone();
    prediction.append(&[sample(20.0, 5.0, 5, 1.0)]).unwrap();
    assert_ne!(prediction.geometry(), b.geometry());
    assert_eq!(b.geometry(), &before);
    prediction.cancel().unwrap();
    assert!(prediction.geometry().is_empty());
    assert_eq!(prediction.state(), StrokeState::Cancelled);
    assert_eq!(prediction.append(&[]), Err(InkError::Closed));
    assert_eq!(prediction.finish(), Err(InkError::Closed));
    let final_geometry = b.finish().unwrap();
    assert_eq!(final_geometry, before);
    assert_eq!(b.state(), StrokeState::Finished);
    assert_eq!(b.finish(), Err(InkError::Closed));
    assert_eq!(b.cancel(), Err(InkError::Closed));
    assert_eq!(b.append(&[]), Err(InkError::Closed));
}

#[test]
fn strict_sample_and_parallel_array_validation_precedes_geometry() {
    for value in [
        f64::NAN,
        f64::INFINITY,
        MAX_COORDINATE + 1.0,
        -MAX_COORDINATE - 1.0,
    ] {
        assert!(Sample::new(value, 0.0, 0, 1.0).is_err());
    }
    let mut s = sample(0.0, 0.0, 0, 1.0);
    s.tilt = Some(-0.01);
    assert!(s.validate().is_err());
    s.tilt = None;
    s.orientation = Some(1e100);
    assert!(s.validate().is_err());
    let mut stroke = model(
        &pen(3.0),
        &[sample(0.0, 0.0, 0, 1.0), sample(10.0, 0.0, 10, 1.0)],
    );
    stroke.tilt = vec![0.0];
    assert!(samples_from_stroke(&stroke).is_err());
    stroke.tilt.clear();
    stroke.y.pop();
    assert!(samples_from_stroke(&stroke).is_err());
    let mut empty = pb::Stroke::default();
    assert_eq!(samples_from_stroke(&empty), Err(InkError::EmptyStroke));
    empty.brush = Some(pen(3.0).to_proto());
    assert_eq!(geometry_from_stroke(&empty), Err(InkError::EmptyStroke));
}

#[test]
fn oversized_input_is_rejected_before_allocating_geometry_or_changing_state() {
    let mut b = StrokeBuilder::begin(pen(3.0)).unwrap();
    let samples = vec![sample(0.0, 0.0, 0, 0.0); MAX_SAMPLES + 1];
    assert_eq!(b.append(&samples), Err(InkError::ResourceLimit));
    assert_eq!(b.sample_count(), 0);
    assert!(b.geometry().is_empty());
}

#[test]
fn quantization_ties_and_signed_zero_are_platform_independent() {
    let mut positive = StrokeBuilder::begin(pen(1.0 / 256.0)).unwrap();
    positive.append(&[sample(0.0, 0.0, 0, 1.0)]).unwrap();
    let bounds = positive.geometry().bounds().unwrap();
    assert_eq!((bounds.min_x, bounds.max_x), (-1.0 / 256.0, 1.0 / 256.0));
    let mut negative = StrokeBuilder::begin(pen(1.0 / 256.0)).unwrap();
    negative.append(&[sample(-0.0, -0.0, 0, 1.0)]).unwrap();
    assert_eq!(
        positive.geometry().canonical_bytes().unwrap(),
        negative.geometry().canonical_bytes().unwrap()
    );
    let mut tiny = StrokeBuilder::begin(pen(1.0 / 1024.0)).unwrap();
    tiny.append(&[sample(0.0, 0.0, 0, 1.0)]).unwrap();
    assert!(tiny.geometry().is_empty());
    assert_eq!(
        tiny.geometry().distance_to(0.0, 0.0).unwrap(),
        f64::INFINITY
    );
}

#[test]
fn stabilization_is_causal_and_axes_do_not_deform_version_one_circular_nibs() {
    let mut brush = pen(2.0);
    brush.stabilization = 1.0;
    let samples = [sample(0.0, 0.0, 0, 1.0), sample(100.0, 0.0, 1, 1.0)];
    let mut smooth = StrokeBuilder::begin(brush.clone()).unwrap();
    smooth.append(&samples).unwrap();
    assert!(smooth.geometry().bounds().unwrap().max_x < 50.0);
    let mut decorated = samples;
    for sample in &mut decorated {
        sample.tilt = Some(1.0);
        sample.orientation = Some(-2.0);
    }
    let mut other = StrokeBuilder::begin(brush).unwrap();
    other.append(&decorated).unwrap();
    assert_eq!(smooth.geometry(), other.geometry());
    let mut raw = StrokeBuilder::begin(pen(2.0)).unwrap();
    raw.append(&samples).unwrap();
    assert_eq!(raw.geometry().bounds().unwrap().max_x, 101.0);
}

#[test]
fn all_quantized_contours_remain_convex_with_consistent_winding() {
    for fixture in synthetic::fixtures().unwrap() {
        let mut b = StrokeBuilder::begin(fixture.brush).unwrap();
        b.append(&fixture.samples).unwrap();
        for polygon in b.geometry().polygons() {
            for i in 0..polygon.points.len() {
                let a = polygon.points[i];
                let c = polygon.points[(i + 1) % polygon.points.len()];
                let d = polygon.points[(i + 2) % polygon.points.len()];
                let cross = i128::from(c.x - a.x) * i128::from(d.y - c.y)
                    - i128::from(c.y - a.y) * i128::from(d.x - c.x);
                assert!(cross > 0, "{} nonconvex quantized contour", fixture.name);
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 500, failure_persistence: None, ..ProptestConfig::default() })]
    #[test]
    fn randomized_chunking_and_prediction_never_change_canonical_geometry(
        values in prop::collection::vec((-1000i32..1000,-1000i32..1000,0u16..=1024,0u16..=16),1..64),
        stabilization in 0u8..=8, chunk in 1usize..16,
    ) {
        let mut time=0u32;
        let samples:Vec<_>=values.into_iter().map(|(x,y,p,dt)| {time+=u32::from(dt); sample(f64::from(x)/8.0,f64::from(y)/8.0,time,f64::from(p)/1024.0)}).collect();
        let mut brush=pen(6.0); brush.stabilization=f32::from(stabilization)/8.0;
        let mut batch=StrokeBuilder::begin(brush.clone()).unwrap(); batch.append(&samples).unwrap();
        let mut incremental=StrokeBuilder::begin(brush).unwrap();
        for part in samples.chunks(chunk) {
            let mut prediction=incremental.clone(); prediction.append(part).unwrap();
            incremental.append(part).unwrap(); prop_assert_eq!(prediction.geometry(),incremental.geometry());
        }
        prop_assert_eq!(batch.geometry().canonical_bytes().unwrap(),incremental.finish().unwrap().canonical_bytes().unwrap());
    }
}

#[test]
fn append_cost_emits_a_software_only_numeric_receipt() {
    let fixture = synthetic::fixtures()
        .unwrap()
        .into_iter()
        .find(|f| f.name == "synthetic_fast_line")
        .unwrap();
    let mut timings = Vec::new();
    for repetition in 0..35 {
        let mut b = StrokeBuilder::begin(fixture.brush.clone()).unwrap();
        for sample in &fixture.samples {
            let start = Instant::now();
            black_box(b.append(std::slice::from_ref(sample)).unwrap());
            if repetition >= 10 {
                timings.push(start.elapsed().as_nanos());
            }
        }
        black_box(b.into_geometry().unwrap());
    }
    timings.sort_unstable();
    let n = timings.len();
    let p50 = timings[(n * 50).div_ceil(100) - 1];
    let p95 = timings[(n * 95).div_ceil(100) - 1];
    println!(
        "ink_append_benchmark samples={n} p50_ns={p50} p95_ns={p95} max_ns={} target_ns=20000",
        timings[n - 1]
    );
    assert_eq!(n, 3200);
}
