//! How long saving one pen stroke takes as a document fills up, how much disk
//! the project takes, and how long it takes to open again.
//!
//! Each stroke goes through the same path the apps use: begin, append samples,
//! commit. The table shows the median commit time once the document already
//! holds N strokes, so growth with document size is visible directly, the
//! slowest commit since the previous row, and the size of the database with
//! its write-ahead log. After the last row the project is closed and opened
//! again three times.
//!
//! Run: cargo bench -p vw-ffi --bench commit
//! Set VW_BENCH_STROKES to change the largest document (default 2000),
//! VW_BENCH_BUDGET_MS to fail when the median at the largest size exceeds it,
//! and VW_BENCH_SLOWEST_BUDGET_MS to fail when any single save exceeds that.
//!
//! Recommended regression check (a few seconds, no other tooling needed):
//!
//!   VW_BENCH_STROKES=2000 VW_BENCH_BUDGET_MS=10 \
//!     cargo bench -p vw-ffi --bench commit --locked
//!
//! A commit costs about a millisecond at every size up to 8,000 strokes on a
//! four-core Linux container (0.9 ms at 2,000, 1.2 ms at 8,000). Ten
//! milliseconds leaves room for a slow or busy machine, and still fails on
//! any return to work that grows with the document: hashing the whole project
//! once at 2,000 strokes already takes about 60 ms.
//!
//! The slowest commit is the one in about 120 on which SQLite moves its
//! write-ahead log into the database and syncs both: 12 to 20 ms on that
//! container at every size, and now and then 30 to 100 ms when something
//! else keeps the disk busy. Nothing in a save grows with the document.
//! There, 8,000 strokes take 35 MB on disk and 1.3 s to open again.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use vw_core::*;
use vw_model::{AssetId, DeviceId, Id};
use vw_raster::{AlphaPolicy, ColorPolicy, DecodedImage, ExportFormat, ExportRequest, Pixels};

const TIME: u64 = 1_700_000_000_000;
const SAMPLES_PER_STROKE: usize = 48;
const MEASURED: usize = 15;

fn id(tag: u8, n: u32) -> String {
    let mut entropy = [tag; 10];
    entropy[6..].copy_from_slice(&n.to_be_bytes());
    Id::from_parts(TIME, entropy).unwrap().to_string()
}

fn device() -> String {
    DeviceId::from_bytes([7; 16]).to_string()
}

fn source() -> Vec<u8> {
    let image = DecodedImage {
        width: 1024,
        height: 1024,
        pixels: Pixels::Rgba8(vec![255; 1024 * 1024 * 4]),
        icc: None,
        source_asset: AssetId::hash(b"commit-benchmark"),
        original_available: true,
        orientation_applied: 1,
    };
    vw_raster::export(
        &image,
        &ExportRequest {
            format: ExportFormat::Png8,
            region: None,
            revision: vw_proto::v1::Revision {
                host_seq: 0,
                state_hash: vec![0; 32],
            },
            alpha: AlphaPolicy::Preserve,
            color: ColorPolicy::Preserve,
            allow_depth_reduction: false,
            memory_budget_bytes: 256 * 1024 * 1024,
            capture_session: None,
            frame_id: None,
            capture_provenance: None,
        },
    )
    .unwrap()
    .bytes
}

async fn stroke(project: &Arc<ProjectSession>, n: u32) -> Duration {
    let lamport = project.info().await.unwrap().next_lamport;
    let gesture = project
        .clone()
        .begin_stroke(StrokeOptions {
            gesture_id: id(4, n),
            transaction_id: id(5, n),
            object_id: id(6, n),
            document_id: id(2, 0),
            layer_id: id(3, 0),
            device_id: device(),
            lamport,
            created_at_ms: TIME as i64 + i64::from(n),
            family: "pen".into(),
            width: 4.0,
            rgba: 0x203040ff,
            stabilization: 0.0,
            pressure_curve: vec![],
        })
        .await
        .unwrap();
    // A short diagonal mark whose position varies so strokes do not coincide.
    let x0 = 20.0 + f64::from(n % 900);
    let y0 = 20.0 + f64::from((n * 7) % 900);
    let index = |i: usize| i as f64;
    gesture
        .append_samples(SampleBatch {
            sequence: 1,
            x: (0..SAMPLES_PER_STROKE).map(|i| x0 + index(i)).collect(),
            y: (0..SAMPLES_PER_STROKE)
                .map(|i| y0 + index(i) * 0.5)
                .collect(),
            t_ms: (0..SAMPLES_PER_STROKE).map(|i| (i * 4) as u32).collect(),
            pressure: vec![0.6; SAMPLES_PER_STROKE],
            tilt: vec![],
            orientation: vec![],
        })
        .await
        .unwrap();
    let started = Instant::now();
    if let Err(error) = gesture.commit(Arc::new(Cancellation::new())).await {
        let info = project.info().await.unwrap();
        panic!(
            "saving stroke {n} failed: {error:?} (document revision {})",
            info.host_seq
        );
    }
    started.elapsed()
}

fn median(values: &mut [Duration]) -> Duration {
    values.sort();
    values[values.len() / 2]
}

/// Bytes the project database occupies on disk, write-ahead log included.
fn database_bytes(project: &std::path::Path) -> u64 {
    ["project.sqlite", "project.sqlite-wal"]
        .iter()
        .filter_map(|name| std::fs::metadata(project.join(name)).ok())
        .map(|file| file.len())
        .sum()
}

fn main() {
    let largest: u32 = std::env::var("VW_BENCH_STROKES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2000);
    let budget = |name: &str| -> Option<f64> {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse().ok())
    };
    let budget_ms = budget("VW_BENCH_BUDGET_MS");
    let slowest_budget_ms = budget("VW_BENCH_SLOWEST_BUDGET_MS");
    let checkpoints: Vec<u32> = [0, 250, 500, 1000, 2000, 4000, 8000]
        .into_iter()
        .filter(|size| *size <= largest)
        .collect();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("project");
    let (last, worst, total) = runtime.block_on(async {
        let project = create_image_project(
            CreateImageProject {
                path: path.to_string_lossy().into(),
                project_id: id(1, 0),
                document_id: id(2, 0),
                layer_id: id(3, 0),
                device_id: device(),
                title: "commit benchmark".into(),
                source: source(),
                now_ms: TIME as i64,
            },
            Arc::new(Cancellation::new()),
        )
        .await
        .unwrap();
        println!(
            "strokes in document | median commit of one more stroke | slowest since previous row | database on disk"
        );
        let mut count = 0u32;
        let mut last = Duration::ZERO;
        let mut worst = Duration::ZERO;
        for checkpoint in checkpoints {
            let mut slowest = Duration::ZERO;
            while count < checkpoint {
                slowest = slowest.max(stroke(&project, count).await);
                count += 1;
            }
            let mut timings = Vec::with_capacity(MEASURED);
            for _ in 0..MEASURED {
                timings.push(stroke(&project, count).await);
                count += 1;
            }
            slowest = timings.iter().copied().fold(slowest, Duration::max);
            worst = worst.max(slowest);
            last = median(&mut timings);
            println!(
                "{checkpoint:>19} | {:>30.2} ms | {:>24.2} ms | {:>12.1} MB",
                last.as_secs_f64() * 1000.0,
                slowest.as_secs_f64() * 1000.0,
                database_bytes(&path) as f64 / 1_000_000.0
            );
        }
        let started = Instant::now();
        project.close().await.unwrap();
        println!(
            "closing the project: {:.2} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
        (last, worst, count)
    });
    println!(
        "database on disk after closing, {total} strokes: {:.1} MB",
        database_bytes(&path) as f64 / 1_000_000.0
    );
    // Opening again is what a person waits for when they return to a project.
    for attempt in 1..=3 {
        let started = Instant::now();
        runtime.block_on(async {
            let project =
                open_project(path.to_string_lossy().into(), Arc::new(Cancellation::new()))
                    .await
                    .unwrap();
            let opened = started.elapsed();
            assert_eq!(project.info().await.unwrap().host_seq, u64::from(total));
            project.close().await.unwrap();
            println!(
                "opening the project again, {total} strokes (attempt {attempt}): {:.2} ms",
                opened.as_secs_f64() * 1000.0
            );
        });
    }
    println!(
        "database on disk after reopening, {total} strokes: {:.1} MB",
        database_bytes(&path) as f64 / 1_000_000.0
    );
    if let Some(budget) = budget_ms {
        let measured = last.as_secs_f64() * 1000.0;
        if measured > budget {
            panic!("commit budget exceeded: {measured:.2} ms > {budget:.2} ms");
        }
    }
    if let Some(budget) = slowest_budget_ms {
        let measured = worst.as_secs_f64() * 1000.0;
        if measured > budget {
            panic!("slowest commit budget exceeded: {measured:.2} ms > {budget:.2} ms");
        }
    }
}
