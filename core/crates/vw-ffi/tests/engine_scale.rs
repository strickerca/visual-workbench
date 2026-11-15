//! Large documents and long strokes through the same calls the apps make:
//! a layer holds far more than 768 objects, and predicting the end of a very
//! long stroke costs what it costs on a short one.
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use support::*;
use vw_core::*;
use vw_model::Id;
use vw_proto::{Message, v1 as pb};

fn wide_id(kind: u8, number: u32) -> String {
    let mut entropy = [kind; 10];
    entropy[6..].copy_from_slice(&number.to_be_bytes());
    Id::from_parts(TIME as u64, entropy).unwrap().to_string()
}

async fn stroke_on_top(project: &Arc<ProjectSession>, number: u32) -> ProjectInfo {
    let lamport = project.info().await.unwrap().next_lamport;
    let mut options = stroke();
    options.gesture_id = wide_id(0x41, number);
    options.transaction_id = wide_id(0x42, number);
    options.object_id = wide_id(0x60, number);
    options.lamport = lamport;
    options.created_at_ms = TIME + 10 + i64::from(number);
    let gesture = project.clone().begin_stroke(options).await.unwrap();
    gesture.append_samples(batch()).await.unwrap();
    gesture.commit(cancel()).await.unwrap()
}

async fn order_keys(project: &Arc<ProjectSession>) -> Vec<(String, String)> {
    project
        .render_list(id(2), None, cancel())
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|item| {
            let state = pb::ObjectState::decode(item.object_protobuf.as_slice()).unwrap();
            (item.object_id, state.order_key)
        })
        .collect()
}

#[tokio::test]
async fn one_layer_takes_far_more_than_768_objects_in_creation_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("project");
    let project = create_image_project(create(&path), cancel()).await.unwrap();
    // 900 shapes in two edits: each new shape goes above the one before it.
    let mut created = Vec::new();
    for edit_number in 0..2_u32 {
        let lamport = project.info().await.unwrap().next_lamport;
        let commands = (0..450_u32)
            .map(|n| {
                // Descending IDs: only the order keys can produce creation order.
                let object_id = wide_id(0x50, 10_000 - (edit_number * 450 + n));
                created.push(object_id.clone());
                EditCommand::Create {
                    object_id,
                    layer_id: id(3),
                    shape: NewShape::Rectangle {
                        rectangle: QueryRect {
                            x: f64::from(n % 50),
                            y: f64::from(n / 50),
                            width: 4.0,
                            height: 3.0,
                        },
                    },
                    style: ObjectStyle {
                        rgba: 0x203040ff,
                        width: 1.0,
                        screen_constant_width: false,
                        fill: None,
                    },
                    transform: Transform {
                        a: 1.0,
                        b: 0.0,
                        c: 0.0,
                        d: 1.0,
                        e: 0.0,
                        f: 0.0,
                    },
                }
            })
            .collect();
        let options = EditOptions {
            transaction_id: wide_id(0x43, edit_number),
            document_id: id(2),
            device_id: device(),
            lamport,
            created_at_ms: TIME + 1 + i64::from(edit_number),
        };
        project
            .apply_edit(options, commands, cancel())
            .await
            .unwrap();
    }
    // Strokes take the pen-up path, which reads the layer's newest key.
    for number in 0..6 {
        stroke_on_top(&project, number).await;
        created.push(wide_id(0x60, number));
    }
    let keys = order_keys(&project).await;
    assert_eq!(keys.len(), 906);
    assert_eq!(
        keys.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>(),
        created,
        "objects are not stacked in the order they were created"
    );
    assert!(
        keys.windows(2).all(|pair| pair[0].1 < pair[1].1),
        "order keys are not strictly increasing"
    );
    let longest = keys.iter().map(|(_, key)| key.len()).max().unwrap();
    assert!(longest <= 5, "longest order key has {longest} digits");
    assert_eq!(keys[0].1, "V");

    // The project reopens with the same state and keeps taking objects.
    let before = project.info().await.unwrap();
    project.close().await.unwrap();
    let project = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let reopened = project.info().await.unwrap();
    assert_eq!(reopened.state_hash, before.state_hash);
    assert_eq!(reopened.host_seq, before.host_seq);
    let after = stroke_on_top(&project, 6).await;
    assert_eq!(after.host_seq, before.host_seq + 1);
    assert_eq!(order_keys(&project).await.len(), 907);
    project.close().await.unwrap();
}

fn samples(sequence: u64, first: u32, count: u32) -> SampleBatch {
    // Every sample moves, so every sample adds contours to the stroke.
    let at = |n: u32| f64::from(first + n);
    SampleBatch {
        sequence,
        x: (0..count).map(|n| 10.0 + at(n) * 0.75).collect(),
        y: (0..count)
            .map(|n| 500.0 + 40.0 * (at(n) * 0.05).sin())
            .collect(),
        t_ms: (0..count).map(|n| (first + n) * 4).collect(),
        pressure: vec![0.5; count as usize],
        tilt: vec![],
        orientation: vec![],
    }
}

async fn gesture_with(
    project: &Arc<ProjectSession>,
    number: u32,
    total: u32,
) -> (Arc<StrokeGesture>, u64) {
    let mut options = stroke();
    options.gesture_id = wide_id(0x44, number);
    options.transaction_id = wide_id(0x45, number);
    options.object_id = wide_id(0x61, number);
    let gesture = project.clone().begin_stroke(options).await.unwrap();
    let mut sequence = 1;
    let mut sent = 0;
    while sent < total {
        let count = 500.min(total - sent);
        gesture
            .append_samples(samples(sequence, sent, count))
            .await
            .unwrap();
        sent += count;
        sequence += 1;
    }
    (gesture, sequence)
}

async fn median_preview(gesture: &StrokeGesture, prediction: &SampleBatch) -> Duration {
    let mut timings = Vec::with_capacity(51);
    for _ in 0..51 {
        let start = Instant::now();
        gesture.preview_samples(prediction.clone()).await.unwrap();
        timings.push(start.elapsed());
    }
    timings.sort();
    timings[timings.len() / 2]
}

#[tokio::test]
async fn predicting_after_20000_samples_costs_what_it_costs_after_200() {
    let dir = tempfile::tempdir().unwrap();
    let project = create_image_project(create(&dir.path().join("project")), cancel())
        .await
        .unwrap();
    let (short, short_next) = gesture_with(&project, 0, 200).await;
    let (long, long_next) = gesture_with(&project, 1, 20_000).await;
    let short_tail = samples(short_next, 200, 32);
    let long_tail = samples(long_next, 20_000, 32);

    // Prediction used to copy the stroke and refused strokes this long.
    let mut best = (Duration::MAX, Duration::MAX);
    for _ in 0..5 {
        best.0 = best.0.min(median_preview(&short, &short_tail).await);
        best.1 = best.1.min(median_preview(&long, &long_tail).await);
    }
    println!(
        "ffi_preview_benchmark after_200_samples_us={} after_20000_samples_us={}",
        best.0.as_micros(),
        best.1.as_micros()
    );
    assert!(
        best.1 <= best.0 * 3 + Duration::from_millis(1),
        "after 200 samples: {:?}, after 20,000 samples: {:?}",
        best.0,
        best.1
    );

    // A prediction shows exactly what appending the same samples then adds,
    // and leaves no trace in the stroke.
    for (gesture, tail) in [(&short, short_tail), (&long, long_tail)] {
        let predicted = gesture.preview_samples(tail.clone()).await.unwrap();
        assert_eq!(
            predicted,
            gesture.preview_samples(tail.clone()).await.unwrap()
        );
        let appended = gesture.append_samples(tail).await.unwrap();
        assert_eq!(predicted.first_polygon, appended.first_polygon);
        assert_eq!(predicted.contours, appended.contours);
        assert_eq!(predicted.sample_count + 32, appended.sample_count);
        assert!(!predicted.contours.ends.is_empty());
    }
    short.shutdown().await.unwrap();
    long.shutdown().await.unwrap();
    project.close().await.unwrap();
}
