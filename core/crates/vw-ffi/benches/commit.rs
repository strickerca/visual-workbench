//! How long saving one pen stroke takes as a document fills up.
//!
//! Each stroke goes through the same path the apps use: begin, append samples,
//! commit. The table shows the median commit time once the document already
//! holds N strokes, so growth with document size is visible directly.
//!
//! Run: cargo bench -p vw-ffi --bench commit
//! Set VW_BENCH_STROKES to change the largest document (default 2000), and
//! VW_BENCH_BUDGET_MS to fail when the median at the largest size exceeds it.
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

fn main() {
    let largest: u32 = std::env::var("VW_BENCH_STROKES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2000);
    let budget_ms: Option<f64> = std::env::var("VW_BENCH_BUDGET_MS")
        .ok()
        .and_then(|value| value.parse().ok());
    let checkpoints: Vec<u32> = [0, 250, 500, 1000, 2000, 4000, 8000]
        .into_iter()
        .filter(|size| *size <= largest)
        .collect();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let last = runtime.block_on(async {
        let project = create_image_project(
            CreateImageProject {
                path: root.path().join("project").to_string_lossy().into(),
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
        println!("strokes in document | median commit of one more stroke");
        let mut count = 0u32;
        let mut last = Duration::ZERO;
        for checkpoint in checkpoints {
            while count < checkpoint {
                stroke(&project, count).await;
                count += 1;
            }
            let mut timings = Vec::with_capacity(MEASURED);
            for _ in 0..MEASURED {
                timings.push(stroke(&project, count).await);
                count += 1;
            }
            last = median(&mut timings);
            println!("{checkpoint:>19} | {:>8.2} ms", last.as_secs_f64() * 1000.0);
        }
        project.close().await.unwrap();
        last
    });
    if let Some(budget) = budget_ms {
        let measured = last.as_secs_f64() * 1000.0;
        if measured > budget {
            panic!("commit budget exceeded: {measured:.2} ms > {budget:.2} ms");
        }
    }
}
