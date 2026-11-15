//! Pages of ordinary pen strokes through the calls the apps make, with the
//! budget they pass (256 MiB), on the PC that hosts the page and on the
//! phone's mirror of it. Every stroke has 100 samples with pressure, tilt and
//! orientation: what a stylus reports in under half a second.
//!
//! The workflows the apps offer on such a page stay available: instructions,
//! the layers panel, selections, the eraser and exporting a file. What each
//! call allocates, measured by this binary's counting allocator, stays within
//! the charge it was admitted for, and a workflow is refused only when what
//! it really needs passes the budget.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use support::*;
use vw_core::*;
use vw_model::{DeviceId, Id};

static TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Counts every live heap byte of this test binary, and the most at once.
struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
fn grew(size: usize) {
    let now = LIVE.fetch_add(size, Ordering::SeqCst) + size;
    PEAK.fetch_max(now, Ordering::SeqCst);
}
// SAFETY: every call is forwarded unchanged to the system allocator; the
// counters are atomics and allocate nothing.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's valid layout is forwarded unchanged.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            grew(layout.size());
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's valid layout is forwarded unchanged.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            grew(layout.size());
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the pointer was allocated by System with this layout.
        unsafe { System.dealloc(pointer, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: the pointer was allocated by System with this layout.
        let moved = unsafe { System.realloc(pointer, layout, size) };
        if !moved.is_null() {
            // A moving reallocation holds both blocks for a moment.
            grew(size);
            LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
        }
        moved
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The most bytes allocated at once while `work` ran, above what was live
/// when it began.
async fn peak<T>(work: impl std::future::Future<Output = T>) -> (T, u64) {
    let base = LIVE.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let value = work.await;
    (
        value,
        PEAK.load(Ordering::SeqCst).saturating_sub(base) as u64,
    )
}

const MIB: u64 = 1024 * 1024;
/// What the apps pass to every workflow.
const BUDGET: u64 = 256 * MIB;
/// The core's fixed reserve and the instruction and layers-panel work
/// constants, as `workflow.rs` and `document_controls.rs` define them.
const RESERVE: u64 = 4 * MIB;
const INSTRUCTION_WORK: u64 = 48 * MIB;
const CONTROL_WORK: u64 = 32 * MIB;

fn wide_id(kind: u8, number: u32) -> String {
    let mut entropy = [kind; 10];
    entropy[6..].copy_from_slice(&number.to_be_bytes());
    Id::from_parts(TIME as u64, entropy).unwrap().to_string()
}
fn stroke_id(number: u32) -> String {
    wide_id(0x60, number)
}
/// A handwriting-like stroke of `points` samples: a wandering curve sampled
/// at 240 Hz.
fn pen(number: u32, points: usize) -> SampleBatch {
    let (x0, y0) = (
        40.0 + f64::from(number % 30) * 45.0,
        60.0 + f64::from((number / 30) % 40) * 75.0,
    );
    let phase = f64::from(number) * 0.37;
    let mut batch = SampleBatch {
        sequence: 1,
        x: vec![],
        y: vec![],
        t_ms: vec![],
        pressure: vec![],
        tilt: vec![],
        orientation: vec![],
    };
    for i in 0..points {
        let s = i as f64;
        let a = phase + s * 0.11;
        batch.x.push(x0 + s * 0.35 + 12.0 * a.sin());
        batch
            .y
            .push(y0 + 18.0 * (a * 1.7).cos() + 3.0 * (a * 5.0).sin());
        batch.t_ms.push(i as u32 * 4);
        batch
            .pressure
            .push((0.35 + 0.45 * (s * 0.05).sin().abs()) as f32);
        batch.tilt.push((0.55 + 0.1 * (s * 0.03).sin()) as f32);
        batch
            .orientation
            .push((1.2 + 0.2 * (s * 0.02).cos()) as f32);
    }
    batch
}

/// A closed host project of `strokes` pen strokes, each saved on its own.
async fn draw_page(path: &Path, strokes: u32, points: usize) {
    let project = create_image_project(create(path), cancel()).await.unwrap();
    let mut lamport = project.info().await.unwrap().next_lamport;
    for number in 0..strokes {
        let mut options = stroke();
        options.gesture_id = wide_id(0x41, number);
        options.transaction_id = wide_id(0x42, number);
        options.object_id = stroke_id(number);
        options.lamport = lamport;
        options.created_at_ms = TIME + 10 + i64::from(number);
        options.width = 3.0;
        let gesture = project.clone().begin_stroke(options).await.unwrap();
        gesture.append_samples(pen(number, points)).await.unwrap();
        lamport = gesture.commit(cancel()).await.unwrap().next_lamport;
        gesture.shutdown().await.unwrap();
    }
    project.close().await.unwrap();
}
/// The phone's mirror of the closed host page, with the page's original.
fn mirror_page(host: &Path, root: &Path) -> PathBuf {
    let store = vw_store::ProjectStore::open(host).unwrap();
    let checkpoint = store.checkpoint_bytes().unwrap();
    let host_device = store.host_device().clone();
    drop(store);
    let path = root.join("mirror");
    drop(
        vw_store::ProjectStore::create_authenticated_checkpoint(
            &path,
            &checkpoint,
            &host_device,
            &DeviceId::from_bytes([9; 16]),
            TIME,
        )
        .unwrap(),
    );
    vw_store::BlobStore::new(&path)
        .unwrap()
        .put(&source())
        .unwrap();
    path
}

fn binding(info: &ProjectInfo) -> WorkflowBinding {
    WorkflowBinding {
        project_id: info.project_id.clone(),
        document_id: id(2),
        host_seq: info.host_seq,
        state_hash: info.state_hash.clone(),
    }
}
fn metadata(info: &ProjectInfo, kind: u8, round: u32) -> WorkflowMetadata {
    WorkflowMetadata {
        transaction_id: wide_id(kind, round),
        device_id: info.device_id.clone(),
        first_lamport: info.next_lamport,
        created_at_ms: TIME + 1_000_000 + i64::from(round),
    }
}
fn marker(round: u32) -> InstructionCommand {
    InstructionCommand::PlaceMarker {
        object_id: wide_id(0x73, round),
        instruction_id: wide_id(0x74, round),
        layer_id: id(3),
        point: Point { x: 4.1, y: 7.9 },
        bounds: None,
        element_eids: vec![],
        style: ObjectStyle {
            rgba: 0xe42b37ff,
            width: 2.0,
            screen_constant_width: true,
            fill: None,
        },
        role: InstructionRole::Change,
        text: "make this red".into(),
        entry_method: InstructionEntryMethod::PhoneKeyboard,
        language: "en-US".into(),
    }
}
fn rename(name: &str) -> DocumentControlCommand {
    DocumentControlCommand::UpdateLayer {
        update: LayerControlUpdate {
            layer_id: id(3),
            name: Some(name.into()),
            visible: None,
            locked: None,
            opacity: None,
            blend: None,
        },
    }
}
/// The charge a workflow admits, read from its refusal of a budget that
/// leaves one byte beside its own work.
fn charged<T>(result: WorkflowResult<T>) -> u64 {
    match result {
        Err(WorkflowError::Memory { estimated, .. }) => estimated,
        Err(other) => panic!("expected a charge, got {other:?}"),
        Ok(_) => panic!("expected a charge, got an admission"),
    }
}
/// Erase across the middle of one of the last strokes, as one gesture does.
fn erase(info: &ProjectInfo, page: Page, round: u32) -> VectorEraseOptions {
    let target = page.strokes - round;
    let stroke = pen(target, page.points);
    let (x, y) = (stroke.x[page.points / 2], stroke.y[page.points / 2]);
    VectorEraseOptions {
        page_index: None,
        binding: binding(info),
        metadata: metadata(info, 0x70, round),
        targets: vec![VectorEraseTarget {
            object_id: stroke_id(target),
            replacement_id: wide_id(0x71, round),
        }],
        centers: vec![
            Point {
                x: x - 30.0,
                y: y - 30.0,
            },
            Point {
                x: x + 30.0,
                y: y + 30.0,
            },
        ],
        radius: 4.0,
        memory_budget_bytes: BUDGET,
        max_output_vertices: 65_536,
        max_work_units: 64_000_000,
    }
}

/// The workflows that only read the page in place, or commit in place: the
/// layers panel, selections, the eraser and a file export. Admitted on a
/// page of any size the store holds, each within its charge. The layers
/// panel lists at most 4,096 objects of a document, whatever their size.
/// Returns the eraser's charge.
async fn borrowing_workflows(
    project: &Arc<ProjectSession>,
    page: Page,
    round: u32,
    dir: &Path,
) -> u64 {
    layers_panel(project, page.strokes, round).await;
    // Selections borrow the page and charge only their pixels.
    let (selection, used) = peak(project.selection_document(id(2), cancel())).await;
    selection.unwrap();
    assert!(used <= RESERVE, "selection document used {used}");
    // The eraser reports the charge it was admitted for.
    let info = project.info().await.unwrap();
    let (erased, used) =
        peak(project.erase_vector_strokes(erase(&info, page, round), cancel())).await;
    let receipt = erased.unwrap_or_else(|error| panic!("round {round}: {error:?}"));
    assert!(!receipt.changed.is_empty());
    assert!(
        used <= receipt.estimated_peak_bytes,
        "eraser used {used} > charge {}",
        receipt.estimated_peak_bytes
    );
    // Exporting the marked page to a file.
    let mut export = options();
    export.marked = true;
    let request = FileExportOptions {
        export,
        output_path: dir
            .join(format!("page-{round}.png"))
            .to_string_lossy()
            .into(),
        work_directory: dir.to_string_lossy().into(),
        max_encoded_bytes: 8 * MIB,
        max_scratch_bytes: 400_000_000,
    };
    project
        .preflight_image_file(request.clone(), cancel())
        .await
        .unwrap();
    project
        .export_transfer_file(request, cancel())
        .await
        .unwrap();
    receipt.estimated_peak_bytes
}
/// The layers panel: its charge is the reserve and its own work.
async fn layers_panel(project: &Arc<ProjectSession>, strokes: u32, round: u32) {
    let info = project.info().await.unwrap();
    let (snapshot, used) = peak(project.document_controls_snapshot(id(2), BUDGET, cancel())).await;
    if strokes > 4_096 {
        assert!(matches!(snapshot, Err(WorkflowError::Limit)));
        return;
    }
    snapshot.unwrap();
    assert!(used <= RESERVE + CONTROL_WORK, "layers panel used {used}");
    let (renamed, used) = peak(async {
        let plan = project
            .clone()
            .prepare_document_control(
                binding(&info),
                metadata(&info, 0x75, round),
                rename(&format!("Ink {round}")),
                BUDGET,
                cancel(),
            )
            .await?;
        let receipt = plan.commit(cancel()).await;
        plan.dispose();
        receipt
    })
    .await;
    renamed.unwrap();
    assert!(used <= RESERVE + CONTROL_WORK, "layer rename used {used}");
}

/// Instructions hash the whole visible page once. Each call stays within its
/// charge without the instruction work beside it: the charge of the page
/// alone covers the hash.
async fn instruction_workflows(project: &Arc<ProjectSession>, round: u32, admitted: bool) {
    if !admitted {
        assert!(matches!(
            project.instruction_document(id(2), BUDGET, cancel()).await,
            Err(WorkflowError::Memory { .. })
        ));
        return;
    }
    let charge = charged(
        project
            .instruction_document(id(2), INSTRUCTION_WORK + 1, cancel())
            .await,
    );
    let (document, used) = peak(project.instruction_document(id(2), BUDGET, cancel())).await;
    let document = document.unwrap();
    assert!(
        used <= charge - INSTRUCTION_WORK,
        "instruction document used {used} > charge {}",
        charge - INSTRUCTION_WORK
    );
    let info = project.info().await.unwrap();
    let charge = charged(
        project
            .clone()
            .prepare_instruction(
                document.binding.clone(),
                metadata(&info, 0x72, round),
                marker(round),
                INSTRUCTION_WORK + 1,
                cancel(),
            )
            .await,
    );
    let (placed, used) = peak(async {
        let plan = project
            .clone()
            .prepare_instruction(
                document.binding.clone(),
                metadata(&info, 0x72, round),
                marker(round),
                BUDGET,
                cancel(),
            )
            .await?;
        let receipt = plan.commit(cancel()).await;
        plan.dispose();
        receipt
    })
    .await;
    placed.unwrap();
    assert!(
        used <= charge - INSTRUCTION_WORK,
        "placing a marker used {used} > charge {}",
        charge - INSTRUCTION_WORK
    );
    let document = project
        .instruction_document(id(2), BUDGET, cancel())
        .await
        .unwrap();
    assert_eq!(document.markers.len(), round as usize);
    let (projection, used) =
        peak(project.export_instructions(document.binding, BUDGET, cancel())).await;
    projection.unwrap();
    assert!(
        used <= charge - INSTRUCTION_WORK,
        "instruction export used {used}"
    );
}

/// What a mirror allocates when it takes a queued edit back: a copy of the
/// authoritative project with a fresh state hasher over it.
async fn replay_bytes(mirror: &Path) -> u64 {
    let store = vw_store::ProjectStore::open(mirror).unwrap();
    let (_, used) = peak(async {
        let copy = store.project().clone();
        let mut hasher = vw_model::StateHasher::default();
        hasher.hash(&copy).unwrap();
        (copy, hasher)
    })
    .await;
    used
}
/// Every commit on a mirror is charged that replay: read from the refusal of
/// a layer rename whose budget leaves one byte beside the panel's work. (The
/// eraser's charge, which also covers it, is checked on every page.)
async fn commit_covers_replay(project: &Arc<ProjectSession>, replay: u64) {
    let info = project.info().await.unwrap();
    let charge = charged(
        project
            .clone()
            .prepare_document_control(
                binding(&info),
                metadata(&info, 0x76, 1),
                rename("Replay"),
                RESERVE + CONTROL_WORK + 1,
                cancel(),
            )
            .await,
    );
    assert!(
        replay <= charge - RESERVE - CONTROL_WORK,
        "replay used {replay} > commit charge {}",
        charge - RESERVE - CONTROL_WORK
    );
}
/// A page of `strokes` pen strokes of `points` samples each.
#[derive(Clone, Copy)]
struct Page {
    strokes: u32,
    points: usize,
}
/// Draw the page, then run the workflows on its host and on the phone's
/// mirror of it. `instructions` is whether hashing the page fits the budget.
async fn page(page: Page, instructions: bool) {
    let root = tempfile::tempdir().unwrap();
    let host = root.path().join("host");
    draw_page(&host, page.strokes, page.points).await;
    if !instructions {
        // Refused only because the work really needs more: hashing the page
        // alone takes more than the budget leaves beside the instruction work.
        let store = vw_store::ProjectStore::open(&host).unwrap();
        let (hash, used) = peak(async { store.project().state_hash() }).await;
        hash.unwrap();
        assert!(
            used > BUDGET - INSTRUCTION_WORK,
            "hashing the page used only {used}"
        );
    }
    let project = open_project(host.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    instruction_workflows(&project, 1, instructions).await;
    borrowing_workflows(&project, page, 1, root.path()).await;
    project.close().await.unwrap();
    let mirror = mirror_page(&host, root.path());
    let replay = replay_bytes(&mirror).await;
    let project = open_project(mirror.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    if page.strokes <= 4_096 {
        commit_covers_replay(&project, replay).await;
    }
    instruction_workflows(&project, 2, instructions).await;
    let erase = borrowing_workflows(&project, page, 2, root.path()).await;
    assert!(
        replay <= erase,
        "replay used {replay} > erase charge {erase}"
    );
    project.close().await.unwrap();
}

#[tokio::test]
async fn pages_of_hundreds_and_thousands_of_strokes_keep_every_workflow_within_its_charge() {
    let _serial = TESTS.lock().await;
    page(
        Page {
            strokes: 500,
            points: 100,
        },
        true,
    )
    .await;
    page(
        Page {
            strokes: 2_000,
            points: 100,
        },
        true,
    )
    .await;
}

/// 8,000 strokes of 50 samples: the store holds this page (of 100 samples it
/// stops at about 6,400, where the checkpoint reaches the 256 MB it may take).
/// Everything that borrows the page is admitted; instructions are refused.
#[tokio::test]
#[ignore = "draws 8,000 strokes: about a minute and over a gigabyte of memory"]
async fn a_page_of_eight_thousand_strokes_refuses_only_what_cannot_fit() {
    let _serial = TESTS.lock().await;
    page(
        Page {
            strokes: 8_000,
            points: 50,
        },
        false,
    )
    .await;
}
