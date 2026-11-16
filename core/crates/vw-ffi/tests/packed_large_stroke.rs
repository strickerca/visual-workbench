//! One very long stroke, through the packed read the apps make after every
//! save. Building its outline holds its whole geometry for a moment, far more
//! than the packed bytes it leaves behind; the read counts that build peak, so
//! a read that is admitted never holds more than the 64 MB it is admitted
//! under, and a longer stroke is refused instead. Measured by this binary's
//! counting allocator; the tests take one lock, so nothing else allocates
//! while a call is measured.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use support::*;
use vw_core::*;

/// Counts the bytes held at once; a reallocation holds both blocks, as the
/// default `realloc` does on its way from the old block to the new one.
struct Meter;
static HELD: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
/// Every byte ever allocated.
static TOTAL: AtomicUsize = AtomicUsize::new(0);
#[global_allocator]
static METER: Meter = Meter;
// SAFETY: every operation forwards the caller's unchanged layout and pointer
// to System; the counters are plain atomics.
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds GlobalAlloc's layout contract.
        let block = unsafe { System.alloc(layout) };
        if !block.is_null() {
            let held = HELD.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            PEAK.fetch_max(held, Ordering::SeqCst);
            TOTAL.fetch_add(layout.size(), Ordering::SeqCst);
        }
        block
    }
    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        HELD.fetch_sub(layout.size(), Ordering::SeqCst);
        // SAFETY: the block and layout are the pair this allocator handed out.
        unsafe { System.dealloc(block, layout) }
    }
}
async fn peak_during<T>(work: impl std::future::Future<Output = T>) -> (T, usize) {
    let before = HELD.load(Ordering::SeqCst);
    PEAK.store(before, Ordering::SeqCst);
    let value = work.await;
    (value, PEAK.load(Ordering::SeqCst).saturating_sub(before))
}
/// The peak as `peak_during`, and every byte allocated while `work` ran.
async fn allocated_during<T>(work: impl std::future::Future<Output = T>) -> (T, usize, usize) {
    let before = TOTAL.load(Ordering::SeqCst);
    let (value, peak) = peak_during(work).await;
    (value, peak, TOTAL.load(Ordering::SeqCst) - before)
}
static SERIAL: Mutex<()> = Mutex::new(());
fn alone(test: impl std::future::Future<Output = ()>) {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(test);
}

const MAX_RESPONSE: usize = 64 * 1024 * 1024;

/// A project holding one pen stroke of `samples` samples, drawn with the
/// phone's default pen along a line of handwriting.
async fn one_stroke(dir: &std::path::Path, samples: usize) -> Arc<ProjectSession> {
    let project = create_image_project(create(&dir.join("project")), cancel())
        .await
        .unwrap();
    let mut options = stroke();
    options.width = 6.0;
    options.stabilization = 0.15;
    options.pressure_curve = vec![0.0, 0.0, 0.25, 0.25, 0.75, 0.75, 1.0, 1.0];
    let gesture = project.clone().begin_stroke(options).await.unwrap();
    for (sequence, start) in (0..samples).step_by(512).enumerate() {
        let range = start..samples.min(start + 512);
        let i = |n: usize| n as f64;
        gesture
            .append_samples(SampleBatch {
                sequence: sequence as u64 + 1,
                x: range
                    .clone()
                    .map(|n| 1.4 * i(n) + 5.0 * (0.45 * i(n)).sin())
                    .collect(),
                y: range
                    .clone()
                    .map(|n| 9.0 * (0.31 * i(n)).sin() + 3.0 * (0.9 * i(n)).cos())
                    .collect(),
                t_ms: range.clone().map(|n| n as u32 * 4).collect(),
                pressure: range
                    .map(|n| (0.55 + 0.3 * (0.2 * i(n)).sin()) as f32)
                    .collect(),
                tilt: vec![],
                orientation: vec![],
            })
            .await
            .unwrap();
    }
    gesture.commit(cancel()).await.unwrap();
    // Let the stroke's live geometry go before anything is measured: the
    // project releases it on its own worker, ahead of later calls.
    drop(gesture);
    project.info().await.unwrap();
    let _ = project.render_list(id(2), None, cancel()).await;
    project
}
/// The peak of a packed read lowered into the transfer buffer the generated
/// bindings read, and that buffer's capacity and the outline's length.
async fn packed_read(project: &Arc<ProjectSession>) -> (Result<(usize, usize)>, usize) {
    peak_during(async {
        let snapshot = project.document_snapshot_packed(id(2), cancel()).await?;
        let outline = snapshot.render.items[0].contours.len();
        let buffer =
            <PackedDocumentSnapshot as uniffi::Lower<vw_core::UniFfiTag>>::lower_into_rust_buffer(
                snapshot,
            );
        let capacity = buffer.capacity();
        buffer.destroy();
        Ok((capacity, outline))
    })
    .await
}

#[test]
fn an_admitted_read_of_one_long_stroke_holds_less_than_its_limit() {
    alone(async {
        let dir = tempfile::tempdir().unwrap();
        // The longest stroke admitted is about 85,700 samples.
        let samples = 82_000;
        let project = one_stroke(dir.path(), samples).await;
        let (read, peak) = packed_read(&project).await;
        let (transfer, outline) = read.unwrap();
        let cost = project
            .document_snapshot_changes_packed(id(2), vec![], cancel())
            .await
            .unwrap()
            .items[0]
            .cost as usize;
        println!(
            "one stroke of {samples} samples: Rust peak {peak} B, packed outline {outline} B, \
             transfer buffer {transfer} B, retained charge {cost} B"
        );
        // The geometry dominates: the read is close to its limit, and stays under it.
        assert!(peak > MAX_RESPONSE / 2, "{peak}");
        assert!(peak <= MAX_RESPONSE, "held {peak} bytes");
        assert!(peak > cost, "the build peak exceeds what the item keeps");
        // Kotlin holds the transfer buffer, the outline's ByteArray and the
        // item's objects; the retained charge bounds them.
        assert!(transfer + outline + 1024 <= cost);
        // The changes read builds the same outline under the same rule.
        let (changes, peak) = peak_during(async {
            project
                .document_snapshot_changes_packed(id(2), vec![], cancel())
                .await
                .unwrap()
        })
        .await;
        assert_eq!(changes.items.len(), 1);
        assert!(peak <= MAX_RESPONSE, "held {peak} bytes");
        project.close().await.unwrap();
    });
}

#[test]
fn a_longer_stroke_is_refused_where_its_build_would_pass_the_limit() {
    alone(async {
        let dir = tempfile::tempdir().unwrap();
        // About 100,000 samples: about 1.6 million vertices. Its packed bytes
        // alone would count about 25 MB, but building them holds about 69 MB:
        // the read is refused as soon as that is known, before they are made.
        // What the refused build held is bounded by the ink limits on one
        // stroke, as it is for every read.
        let project = one_stroke(dir.path(), 100_000).await;
        let (read, peak) = packed_read(&project).await;
        println!("one stroke of 100000 samples: refused after holding {peak} B");
        assert!(matches!(read, Err(CoreError::Backpressure)));
        assert!(matches!(
            project
                .document_snapshot_changes_packed(id(2), vec![], cancel())
                .await,
            Err(CoreError::Backpressure)
        ));
        // As the plain read refuses it.
        assert!(matches!(
            project.document_snapshot(id(2), cancel()).await,
            Err(CoreError::Backpressure)
        ));
        project.close().await.unwrap();
    });
}

/// A display list asked for a region builds outlines only for the items in
/// it: a stroke outside the region costs its bounds, never its outline lists,
/// and nothing about that outline can fail the call.
#[test]
fn a_region_read_builds_no_outline_for_a_stroke_outside_it() {
    alone(async {
        let dir = tempfile::tempdir().unwrap();
        let samples = 20_000;
        let project = one_stroke(dir.path(), samples).await;
        let region = |x: f64| QueryRect {
            x,
            y: -50.0,
            width: 100.0,
            height: 100.0,
        };
        let (inside, inside_peak, inside_total) = allocated_during(async {
            project
                .render_list(id(2), Some(region(0.0)), cancel())
                .await
                .unwrap()
        })
        .await;
        let vertices = inside.items[0].stroke_contours.x.len();
        drop(inside);
        let (outside, outside_peak, outside_total) = allocated_during(async {
            project
                .render_list(id(2), Some(region(-10_000.0)), cancel())
                .await
                .unwrap()
        })
        .await;
        println!(
            "{vertices} vertices: region with the stroke allocates {inside_total} B (peak \
             {inside_peak}), without it {outside_total} B (peak {outside_peak})"
        );
        assert!(outside.items.is_empty());
        // Its bounds still need its geometry, but the two i64 lists of its
        // outline (16 bytes a vertex) are never built.
        assert!(outside_total + 16 * vertices <= inside_total);
        assert!(outside_peak <= inside_peak);
        project.close().await.unwrap();
    });
}
