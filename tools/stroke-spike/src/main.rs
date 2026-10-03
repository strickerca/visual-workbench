//! Software-only stroke fixture runner and append microbenchmark. No hardware
//! provenance, wet-ink latency, device preference or D6 decision is inferred.
#[path = "../../../core/crates/vw-ink/fixtures/synthetic.rs"]
mod synthetic;

use serde::Serialize;
use std::error::Error;
use std::hint::black_box;
use std::io::Read;
use std::time::Instant;
use vw_ink::{ALGORITHM_VERSION, StrokeBuilder, geometry_from_stroke};

#[derive(Serialize)]
struct FixtureHash {
    name: String,
    samples: usize,
    polygons: usize,
    vertices: usize,
    geometry_hash: String,
}
#[derive(Serialize)]
struct Goldens {
    algorithm_version: u32,
    provenance: &'static str,
    fixtures: Vec<FixtureHash>,
}
#[derive(Serialize)]
struct Benchmark {
    algorithm_version: u32,
    provenance: &'static str,
    samples_measured: usize,
    repetitions: usize,
    p50_ns: u128,
    p95_ns: u128,
    max_ns: u128,
    target_p95_ns: u128,
    meets_target: bool,
}
fn error(message: &'static str) -> Box<dyn Error> {
    Box::new(std::io::Error::other(message))
}
fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("goldens") => {
            if args.next().is_some() {
                return Err(error("goldens takes no arguments"));
            }
            let mut records = Vec::new();
            for fixture in synthetic::fixtures()? {
                let mut builder = StrokeBuilder::begin(fixture.brush)?;
                builder.append(&fixture.samples)?;
                let geometry = builder.into_geometry()?;
                records.push(FixtureHash {
                    name: fixture.name.into(),
                    samples: fixture.samples.len(),
                    polygons: geometry.polygons().len(),
                    vertices: geometry.vertex_count(),
                    geometry_hash: geometry.hash()?.to_string(),
                });
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&Goldens {
                    algorithm_version: ALGORITHM_VERSION,
                    provenance: "synthetic-only; no owner/device recording",
                    fixtures: records
                })?
            );
        }
        Some("benchmark") => {
            let repetitions = args
                .next()
                .map(|value| value.parse::<usize>())
                .transpose()?
                .unwrap_or(100);
            if !(10..=500).contains(&repetitions) || args.next().is_some() {
                return Err(error("benchmark repetitions must be 10..500"));
            }
            let mut fixtures = synthetic::fixtures()?;
            let selected = fixtures
                .iter()
                .position(|f| f.name == "synthetic_fast_line")
                .ok_or_else(|| error("missing benchmark fixture"))?;
            let fixture = fixtures.swap_remove(selected);
            let mut timings = Vec::with_capacity(repetitions * fixture.samples.len());
            for iteration in 0..repetitions + 10 {
                let mut builder = StrokeBuilder::begin(fixture.brush.clone())?;
                for sample in &fixture.samples {
                    let started = Instant::now();
                    black_box(builder.append(std::slice::from_ref(sample))?);
                    if iteration >= 10 {
                        timings.push(started.elapsed().as_nanos());
                    }
                }
                black_box(builder.into_geometry()?);
                if iteration % 25 == 0 {
                    eprintln!("stroke benchmark progress {iteration}/{}", repetitions + 10);
                }
            }
            timings.sort_unstable();
            let n = timings.len();
            let p50 = timings[(n * 50).div_ceil(100) - 1];
            let p95 = timings[(n * 95).div_ceil(100) - 1];
            println!(
                "{}",
                serde_json::to_string_pretty(&Benchmark {
                    algorithm_version: ALGORITHM_VERSION,
                    provenance: "synthetic CPU append cost; excludes renderer/JNI/display latency",
                    samples_measured: n,
                    repetitions,
                    p50_ns: p50,
                    p95_ns: p95,
                    max_ns: timings[n - 1],
                    target_p95_ns: 20_000,
                    meets_target: p95 < 20_000
                })?
            );
        }
        Some("geometry") => {
            let path = args
                .next()
                .ok_or_else(|| error("geometry needs a model Stroke JSON file"))?;
            if args.next().is_some() {
                return Err(error("geometry accepts one file"));
            }
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(16 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 16 * 1024 * 1024 {
                return Err(error("stroke input exceeds 16 MiB"));
            }
            let stroke: vw_proto::v1::Stroke = serde_json::from_slice(&bytes)?;
            let geometry = geometry_from_stroke(&stroke)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({ "algorithm_version": ALGORITHM_VERSION,
                "geometry_hash": geometry.hash()?.as_str(), "geometry": geometry })
                )?
            );
        }
        _ => {
            return Err(error(
                "usage: vw-stroke-spike goldens | benchmark [10..500] | geometry <stroke.json>",
            ));
        }
    }
    Ok(())
}
