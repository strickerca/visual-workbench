//! `StrokeBuilder::preview` returns what `append` would add without copying
//! or changing the stroke. All inputs are synthetic.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use proptest::prelude::*;
use std::{hint::black_box, time::Instant};
use vw_ink::{Brush, BrushFamily, InkError, Sample, StrokeBuilder};

fn sample(x: f64, y: f64, time: u32, pressure: f64) -> Sample {
    Sample::new(x, y, time, pressure).unwrap()
}
fn brush(family: BrushFamily, stabilization: f32) -> Brush {
    let mut brush = Brush::new(family, 6.0).unwrap();
    brush.stabilization = stabilization;
    brush
}
/// A wandering stroke whose every sample moves, so every sample adds contours.
fn wander(count: usize) -> Vec<Sample> {
    (0..count)
        .map(|n| {
            let t = n as f64;
            sample(
                100.0 + t * 0.75,
                100.0 + 40.0 * libm_sin(t * 0.05),
                (n * 4) as u32,
                0.35 + 0.3 * libm_sin(t * 0.11).abs(),
            )
        })
        .collect()
}
// A portable sine is not needed for a timing fixture.
fn libm_sin(value: f64) -> f64 {
    value.sin()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 300, failure_persistence: None, ..ProptestConfig::default() })]
    #[test]
    fn a_preview_is_exactly_what_the_append_then_adds(
        values in prop::collection::vec((-1000i32..1000, -1000i32..1000, 0u16..=1024, 0u16..=16), 1..96),
        stabilization in 0u8..=8,
        chunk in 1usize..24,
        family in 0u8..3,
    ) {
        let mut time = 0u32;
        let samples: Vec<_> = values
            .into_iter()
            .map(|(x, y, p, dt)| {
                time += u32::from(dt);
                sample(f64::from(x) / 8.0, f64::from(y) / 8.0, time, f64::from(p) / 1024.0)
            })
            .collect();
        let family = [BrushFamily::Pen, BrushFamily::Marker, BrushFamily::Highlighter][usize::from(family)];
        let brush = brush(family, f32::from(stabilization) / 8.0);
        let mut previewed = StrokeBuilder::begin(brush.clone()).unwrap();
        let mut plain = StrokeBuilder::begin(brush).unwrap();
        for part in samples.chunks(chunk) {
            let before = previewed.geometry().clone();
            let predicted = previewed.preview(part).unwrap();
            // Previewing changed nothing, twice over.
            prop_assert_eq!(previewed.geometry(), &before);
            prop_assert_eq!(&previewed.preview(part).unwrap(), &predicted);
            prop_assert_eq!(previewed.sample_count(), plain.sample_count());
            let range = previewed.append(part).unwrap();
            prop_assert_eq!(range.start, before.polygons().len());
            prop_assert_eq!(&previewed.geometry().polygons()[range.start..range.end], &predicted[..]);
            // A stroke that was never previewed is identical.
            plain.append(part).unwrap();
            prop_assert_eq!(previewed.geometry(), plain.geometry());
        }
        prop_assert_eq!(
            previewed.finish().unwrap().canonical_bytes().unwrap(),
            plain.finish().unwrap().canonical_bytes().unwrap()
        );
    }
}

#[test]
fn a_preview_refuses_what_an_append_refuses_and_leaves_the_stroke_usable() {
    let mut builder = StrokeBuilder::begin(brush(BrushFamily::Pen, 0.0)).unwrap();
    assert!(builder.preview(&[]).unwrap().is_empty());
    builder.append(&wander(8)).unwrap();
    let before = builder.geometry().clone();
    // Out of time order after the last appended sample.
    assert_eq!(
        builder.preview(&[sample(0.0, 0.0, 1, 0.5)]),
        Err(InkError::TimeOrder)
    );
    assert_eq!(
        builder.append(&[sample(0.0, 0.0, 1, 0.5)]),
        Err(InkError::TimeOrder)
    );
    assert_eq!(builder.geometry(), &before);
    let mut finished = builder.clone();
    finished.finish().unwrap();
    assert_eq!(
        finished.preview(&[sample(0.0, 0.0, 100, 0.5)]),
        Err(InkError::Closed)
    );
    let mut cancelled = builder.clone();
    cancelled.cancel().unwrap();
    assert_eq!(
        cancelled.preview(&[sample(0.0, 0.0, 100, 0.5)]),
        Err(InkError::Closed)
    );
    assert!(
        !builder
            .preview(&[sample(500.0, 0.0, 100, 0.5)])
            .unwrap()
            .is_empty()
    );
}

fn median_preview_nanos(builder: &StrokeBuilder, prediction: &[Sample]) -> u128 {
    let mut timings = (0..201)
        .map(|_| {
            let start = Instant::now();
            black_box(builder.preview(black_box(prediction)).unwrap());
            start.elapsed().as_nanos()
        })
        .collect::<Vec<_>>();
    timings.sort_unstable();
    timings[timings.len() / 2]
}

#[test]
fn previewing_a_20000_sample_stroke_costs_what_a_200_sample_stroke_costs() {
    let samples = wander(20_000);
    let mut short = StrokeBuilder::begin(brush(BrushFamily::Pen, 0.25)).unwrap();
    short.append(&samples[..200]).unwrap();
    let mut long = StrokeBuilder::begin(brush(BrushFamily::Pen, 0.25)).unwrap();
    long.append(&samples).unwrap();
    assert!(long.geometry().vertex_count() > 100 * short.geometry().vertex_count() / 2);
    // The same 32 predicted samples continue each stroke from its own end.
    let predict = |after: &Sample| {
        (1..=32)
            .map(|n| {
                sample(
                    after.x + f64::from(n) * 0.75,
                    after.y + f64::from(n) * 0.5,
                    after.t_ms + n as u32 * 4,
                    0.5,
                )
            })
            .collect::<Vec<_>>()
    };
    let (short_tail, long_tail) = (predict(&samples[199]), predict(&samples[19_999]));
    // Interleave the measurements so machine load affects both alike, and
    // keep the best of several rounds.
    let mut best = (u128::MAX, u128::MAX);
    for _ in 0..5 {
        best.0 = best.0.min(median_preview_nanos(&short, &short_tail));
        best.1 = best.1.min(median_preview_nanos(&long, &long_tail));
    }
    let copy = {
        let start = Instant::now();
        black_box(long.clone());
        start.elapsed().as_nanos()
    };
    println!(
        "ink_preview_benchmark short_ns={} long_ns={} one_copy_of_long_ns={copy}",
        best.0, best.1
    );
    // Copying the long stroke once, as prediction used to, costs far more
    // than this allows; independent costs differ by noise only.
    assert!(
        best.1 <= best.0 * 3 + 20_000,
        "200 samples: {} ns, 20,000 samples: {} ns",
        best.0,
        best.1
    );
}
