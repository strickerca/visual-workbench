//! Pages of ordinary pen strokes through the calls the apps make, with the
//! budget they pass (256 MiB), on the PC that hosts the page and on the
//! phone's mirror of it. A stroke has 100 samples with pressure, tilt and
//! orientation unless said otherwise: what a stylus reports in under half a
//! second.
//!
//! The workflows the apps offer on such a page stay available, and what each
//! call allocates, measured by this binary's counting allocator, stays within
//! the charge it is admitted with: a read or a plan against what it was
//! refused with one byte short, a commit against what its commit admits at
//! that moment, the eraser against what it reports. A mirror's replay of its
//! queued work (the outbox refusing an edit, or recovered work) is run for
//! real and measured against the charge every mirror commit carries. A
//! workflow is refused only when what it really needs passes the budget.
//!
//! Measurements are process-wide: the tests of this binary take one lock, so
//! nothing else allocates while a single call is measured, and every session
//! holds its checkpoint upkeep, which otherwise runs between calls.
//!
//! A build of vw-ops with `cross-check` (the workspace's test build: vw-store
//! and vw-sim turn it on for their own tests) hashes the whole project after
//! every edit it decides. Commits are charged that there too, so there the
//! largest pages may refuse an edit, but only for what those checks add.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    ops::Range,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use support::*;
use vw_core::*;
use vw_model::{AssetId, DeviceId, Document, Id, Layer, Object, OrderKey, Project};
use vw_proto::v1 as pb;

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
fn peak_of<T>(work: impl FnOnce() -> T) -> (T, u64) {
    let base = LIVE.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let value = work();
    (
        value,
        PEAK.load(Ordering::SeqCst).saturating_sub(base) as u64,
    )
}

const MIB: u64 = 1024 * 1024;
/// What the apps pass to every workflow.
const BUDGET: u64 = 256 * MIB;
/// The core's fixed reserve and the instruction work constant, as
/// `workflow.rs` defines them, and the 1 KiB per listed object the layers
/// panel's work constant covers (`document_controls.rs`).
const RESERVE: u64 = 4 * MIB;
const INSTRUCTION_WORK: u64 = 48 * MIB;
const CONTROL_WORK: u64 = 32 * MIB;
const PANEL_ROW: u64 = 1024;
/// The device of the phone that mirrors the page.
const PHONE: [u8; 16] = [9; 16];

fn wide_id(kind: u8, number: u32) -> String {
    let mut entropy = [kind; 10];
    entropy[6..].copy_from_slice(&number.to_be_bytes());
    Id::from_parts(TIME as u64, entropy).unwrap().to_string()
}
/// Strokes drawn on the PC have IDs of kind 0x60, the phone's of 0x50.
fn stroke_id(kind: u8, number: u32) -> String {
    wide_id(kind, number)
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
/// Draw the strokes `numbers`, each saved on its own, as `project`'s device.
async fn draw(project: &Arc<ProjectSession>, kind: u8, numbers: Range<u32>, points: usize) {
    let info = project.info().await.unwrap();
    let mut lamport = info.next_lamport;
    for number in numbers {
        let mut options = stroke();
        options.gesture_id = wide_id(kind + 1, number);
        options.transaction_id = wide_id(kind + 2, number);
        options.object_id = stroke_id(kind, number);
        options.device_id = info.device_id.clone();
        options.lamport = lamport;
        options.created_at_ms = TIME + 10 + i64::from(number);
        options.width = 3.0;
        let gesture = project.clone().begin_stroke(options).await.unwrap();
        gesture.append_samples(pen(number, points)).await.unwrap();
        lamport = gesture.commit(cancel()).await.unwrap().next_lamport;
        gesture.shutdown().await.unwrap();
    }
}
/// Open a project, its checkpoint upkeep held.
async fn open(path: &Path) -> Arc<ProjectSession> {
    let project = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    project.hold_checkpoint_upkeep(true);
    project
}
/// The object the PC saves for its stroke `number` drawn after `before`,
/// built as a stroke gesture builds it.
fn pen_object(number: u32, points: usize, before: Option<&OrderKey>) -> (Object, OrderKey) {
    let options = stroke();
    let mut brush = vw_ink::Brush::new(
        vw_ink::BrushFamily::try_from(options.family.as_str()).unwrap(),
        3.0,
    )
    .unwrap();
    brush.stabilization = options.stabilization;
    let samples = pen(number, points);
    let order = OrderKey::between(before, None).unwrap();
    let state = pb::ObjectState {
        object_id: Some(Id::try_from(stroke_id(0x60, number)).unwrap().to_proto()),
        layer_id: Some(Id::try_from(id(3)).unwrap().to_proto()),
        order_key: order.as_str().into(),
        transform: Some(pb::Affine {
            a: 1.0,
            d: 1.0,
            ..Default::default()
        }),
        style: Some(pb::Style {
            stroke: Some(pb::Color { rgba: options.rgba }),
            width: 3.0,
            ..Default::default()
        }),
        role: pb::Role::None as i32,
        created_by: device(),
        created_at_ms: TIME + 10 + i64::from(number),
        shape: Some(pb::object_state::Shape::Stroke(pb::Stroke {
            x: samples.x,
            y: samples.y,
            t_ms: samples.t_ms,
            pressure: samples.pressure,
            tilt: samples.tilt,
            orientation: samples.orientation,
            brush: Some(brush.to_proto()),
        })),
        ..Default::default()
    };
    let object = Object {
        document_id: Id::try_from(id(2)).unwrap(),
        state,
    };
    (object, order)
}
/// `project` with the PC's strokes `0..strokes` added, the objects drawing
/// them one by one leaves (`ready_made_pages_hold_what_drawing_leaves`).
fn with_strokes(mut project: Project, strokes: u32, points: usize) -> Project {
    let mut before = None;
    for number in 0..strokes {
        let (object, order) = pen_object(number, points, before.as_ref());
        project
            .objects
            .insert(Id::try_from(stroke_id(0x60, number)).unwrap(), object);
        before = Some(order);
    }
    project
}
/// A new image project as the PC creates it, before anything is drawn.
async fn blank_page(root: &Path) -> Project {
    let path = root.join("blank");
    create_image_project(create(&path), cancel())
        .await
        .unwrap()
        .close()
        .await
        .unwrap();
    let project = vw_store::ProjectStore::open(&path)
        .unwrap()
        .project()
        .clone();
    std::fs::remove_dir_all(&path).unwrap();
    project
}
/// A closed host project of `strokes` pen strokes, made as the PC holds it
/// once they are drawn. All but the last are made directly, without the
/// history of drawing them; the last is drawn, so that the PC's undo
/// history has an edit on top, as after drawing, for the calls that read
/// whether one can be undone. No charge and no other measured call reads
/// the PC's history (a cross-checking build's undo copy aside, which is
/// checked against whatever history the page has). Drawing every stroke
/// in a cross-checking build would hash the whole page every stroke.
async fn draw_page(path: &Path, strokes: u32, points: usize) {
    let root = path.parent().unwrap();
    let project = with_strokes(blank_page(root).await, strokes - 1, points);
    created(path, project);
    draw_last(path, strokes, points).await;
}
/// Draw the PC's last stroke, `strokes - 1`, on the closed page at `path`.
async fn draw_last(path: &Path, strokes: u32, points: usize) {
    let project = open(path).await;
    draw(&project, 0x60, strokes - 1..strokes, points).await;
    project.close().await.unwrap();
}
/// A host store holding `project`, with the page's original.
fn created(path: &Path, project: Project) {
    drop(
        vw_store::ProjectStore::create(path, project, DeviceId::try_from(device()).unwrap(), TIME)
            .unwrap(),
    );
    vw_store::BlobStore::new(path)
        .unwrap()
        .put(&source())
        .unwrap();
}
/// The phone's mirror of the closed host page, with the page's original.
fn mirror_page(host: &Path, path: &Path, original: &[u8]) {
    let store = vw_store::ProjectStore::open(host).unwrap();
    let checkpoint = store.checkpoint_bytes().unwrap();
    let host_device = store.host_device().clone();
    drop(store);
    drop(
        vw_store::ProjectStore::create_authenticated_checkpoint(
            path,
            &checkpoint,
            &host_device,
            &DeviceId::from_bytes(PHONE),
            TIME,
        )
        .unwrap(),
    );
    vw_store::BlobStore::new(path)
        .unwrap()
        .put(original)
        .unwrap();
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
/// With `VW_LARGE_PAGES_REPORT` set, print what a call used and its charge.
fn report(what: &str, used: u64, charge: u64) {
    if std::env::var_os("VW_LARGE_PAGES_REPORT").is_some() {
        eprintln!(
            "{what}: used {:.1} MiB, charge {:.1} MiB",
            used as f64 / MIB as f64,
            charge as f64 / MIB as f64
        );
    }
}
/// What every commit is charged for the checks of a cross-checking build.
async fn verification(project: &Arc<ProjectSession>) -> u64 {
    let charge = project.cross_check_charge().await.unwrap();
    // Unknown means the page's hash is past every memory limit; it would make
    // every refusal look explained. No page here gets that large.
    assert_ne!(charge, u64::MAX, "the cross-check charge is unknown");
    charge
}
/// Whether a refusal at `estimated` is explained by a cross-checking build's
/// checks: without them the charge fits the budget. Never in the apps' build,
/// whose checks cost nothing.
fn cross_checks_explain(estimated: u64, verification: u64) -> bool {
    verification > 0
        && estimated
            .checked_sub(verification)
            .is_some_and(|rest| rest <= BUDGET)
}
/// A workflow the apps' budget admits. A cross-checking build may refuse it,
/// but only when its charge without those checks fits the budget.
fn admitted<T>(result: WorkflowResult<T>, verification: u64, what: &str) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(WorkflowError::Memory { estimated, .. })
            if cross_checks_explain(estimated, verification) =>
        {
            report(
                &format!("{what} refused for the cross-checks"),
                0,
                estimated,
            );
            None
        }
        Err(error) => panic!("{what}: {error:?}"),
    }
}
/// Commit `plan`, measured against what its commit admits at that moment.
/// Returns that charge.
async fn commit(plan: Arc<WorkflowPlan>, what: &str) -> u64 {
    let charge = plan.commit_charge().await.unwrap();
    let (receipt, used) = peak(plan.commit(cancel())).await;
    receipt.unwrap();
    plan.dispose();
    report(&format!("{what} commit"), used, charge);
    assert!(
        used <= charge,
        "{what}: commit used {used} > charge {charge}"
    );
    charge
}

/// A page and where its last strokes are, for the eraser.
#[derive(Clone, Copy)]
struct Page {
    strokes: u32,
    points: usize,
    /// Every object of the page, for the layers panel.
    objects: u64,
}
/// Erase across the middle of one of the PC's last strokes, another one
/// every round.
fn erase(info: &ProjectInfo, page: Page, round: u32) -> VectorEraseOptions {
    let target = page.strokes - 1 - round % page.strokes;
    let stroke = pen(target, page.points);
    let (x, y) = (stroke.x[page.points / 2], stroke.y[page.points / 2]);
    VectorEraseOptions {
        page_index: None,
        binding: binding(info),
        metadata: metadata(info, 0x70, round),
        targets: vec![VectorEraseTarget {
            object_id: stroke_id(0x60, target),
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
        // What the apps pass.
        max_output_vertices: 8_192,
        max_work_units: 64_000_000,
    }
}

/// The layers panel and a layer rename. The panel lists at most 4,096
/// objects, whatever their size. Returns the rename's commit charge.
async fn layers_panel(project: &Arc<ProjectSession>, page: Page, round: u32) {
    let (snapshot, used) = peak(project.document_controls_snapshot(id(2), BUDGET, cancel())).await;
    if page.objects > 4_096 {
        assert!(matches!(snapshot, Err(WorkflowError::Limit)));
        return;
    }
    snapshot.unwrap();
    let rows = RESERVE + page.objects * PANEL_ROW;
    assert!(used <= rows, "layers panel used {used} > {rows}");
    let info = project.info().await.unwrap();
    let verification = verification(project).await;
    let (plan, used) = peak(project.clone().prepare_document_control(
        binding(&info),
        metadata(&info, 0x75, round),
        rename(&format!("Ink {round}")),
        BUDGET,
        cancel(),
    ))
    .await;
    let Some(plan) = admitted(plan, verification, "planning a rename") else {
        return;
    };
    let charge = plan.commit_charge().await.unwrap();
    // Planning reads the page in place; its rows are the panel's work.
    assert!(
        used <= charge + page.objects * PANEL_ROW,
        "planning a rename used {used} > {}",
        charge + page.objects * PANEL_ROW
    );
    commit(plan, "layer rename").await;
}

/// The workflows that read the page in place or commit in place: the layers
/// panel, selections, the eraser and a file export.
async fn borrowing_workflows(project: &Arc<ProjectSession>, page: Page, round: u32, dir: &Path) {
    layers_panel(project, page, round).await;
    let (selection, used) = peak(project.selection_document(id(2), cancel())).await;
    selection.unwrap();
    assert!(used <= RESERVE, "selection document used {used}");
    // The eraser reports the charge its commit was admitted with.
    let info = project.info().await.unwrap();
    let verification = verification(project).await;
    let (erased, used) =
        peak(project.erase_vector_strokes(erase(&info, page, round), cancel())).await;
    if let Some(receipt) = admitted(erased, verification, "eraser") {
        assert!(!receipt.changed.is_empty());
        report("eraser", used, receipt.estimated_peak_bytes);
        assert!(
            used <= receipt.estimated_peak_bytes,
            "eraser used {used} > charge {}",
            receipt.estimated_peak_bytes
        );
    }
    // Exporting the marked page to a file, within what its preflight admits.
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
    let (preflight, used) = peak(project.preflight_image_file(request.clone(), cancel())).await;
    let preflight = preflight.unwrap();
    assert!(used <= RESERVE, "export preflight used {used}");
    let (exported, used) = peak(project.export_transfer_file(request, cancel())).await;
    exported.unwrap();
    assert!(
        used <= RESERVE + preflight.estimated_peak_bytes,
        "file export used {used} > {}",
        RESERVE + preflight.estimated_peak_bytes
    );
}

/// Instructions hash the whole visible page once. A read or a plan stays
/// within its charge without the instruction work beside it: the charge of
/// the page alone covers the hash.
async fn instruction_workflows(project: &Arc<ProjectSession>, round: u32, fits: bool) {
    if !fits {
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
    report("instruction document", used, charge - INSTRUCTION_WORK);
    assert!(
        used <= charge - INSTRUCTION_WORK,
        "instruction document used {used} > charge {}",
        charge - INSTRUCTION_WORK
    );
    let info = project.info().await.unwrap();
    let prepare = |budget| {
        project.clone().prepare_instruction(
            document.binding.clone(),
            metadata(&info, 0x72, round),
            marker(round),
            budget,
            cancel(),
        )
    };
    let charge = charged(prepare(INSTRUCTION_WORK + 1).await);
    let verification = verification(project).await;
    let (plan, used) = peak(prepare(BUDGET)).await;
    if let Some(plan) = admitted(plan, verification, "planning a marker") {
        // Planning hashes the page, then decides the marker in place. In the
        // apps' build that stays within the hash charge; a cross-checking
        // build also re-hashes the whole project there, as the marker's
        // commit is charged.
        let hashed = charge - INSTRUCTION_WORK;
        let planning = if vw_ops::CROSS_CHECKED {
            hashed.max(plan.commit_charge().await.unwrap())
        } else {
            hashed
        };
        assert!(
            used <= planning,
            "planning a marker used {used} > charge {planning}"
        );
        commit(plan, "placing a marker").await;
    }
    let document = project
        .instruction_document(id(2), BUDGET, cancel())
        .await
        .unwrap();
    let (projection, used) =
        peak(project.export_instructions(document.binding, BUDGET, cancel())).await;
    projection.unwrap();
    assert!(
        used <= charge - INSTRUCTION_WORK,
        "instruction export used {used}"
    );
}

/// A tone preview decided on a private host built from a copy of the page:
/// admitted, and within what its read admission charges beside its pixels.
async fn tone_preview(project: &Arc<ProjectSession>) {
    let view = project
        .tone_controls_snapshot(id(2), BUDGET, cancel())
        .await
        .unwrap();
    let set = ToneControlCommand::Set {
        layer_id: wide_id(0x77, 1),
        object_id: wide_id(0x78, 1),
        settings: ImageToneSettings {
            brightness: -0.3,
            contrast: 0.1,
            input_black: 0.1,
            input_white: 0.9,
            gamma: 1.2,
            output_black: 0.1,
            output_white: 0.8,
        },
        enabled: true,
        opacity: 0.5,
    };
    let region = SelectionRegion {
        x: 3,
        y: 5,
        width: 17,
        height: 13,
    };
    let verification = verification(project).await;
    let (preview, used) =
        peak(project.preview_tone_control(view.binding, Some(set), region, true, BUDGET, cancel()))
            .await;
    if admitted(preview, verification, "tone preview").is_some() {
        assert!(used <= BUDGET, "tone preview used {used}");
    }
}

/// The page on its host and then on the phone's mirror of it. On the mirror
/// every commit carries the replay of its queue, which is checked against a
/// real replay. `instructions` is whether hashing the page fits the budget.
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
    let project = open(&host).await;
    instruction_workflows(&project, 1, instructions).await;
    borrowing_workflows(&project, page, 1, root.path()).await;
    if instructions {
        tone_preview(&project).await;
    }
    project.close().await.unwrap();
    let mirror = root.path().join("mirror");
    mirror_page(&host, &mirror, &source());
    let project = open(&mirror).await;
    instruction_workflows(&project, 2, instructions).await;
    borrowing_workflows(&project, page, 2, root.path()).await;
    project.close().await.unwrap();
}

/// A real replay of the mirror's queue, built as the phone builds it when it
/// opens the project: taking recovered work, then taking the last queued
/// edit back as an outbox refusal does. Each measured above the replica it
/// starts from; returns the larger.
fn replay(mirror: &Path) -> u64 {
    let store = vw_store::ProjectStore::open(mirror).unwrap();
    let host =
        vw_ops::HostSequencer::from_checkpoint_bytes(&store.checkpoint_bytes().unwrap()).unwrap();
    let mut replica =
        vw_ops::Replica::new(store.local_device().unwrap(), host.snapshot().unwrap()).unwrap();
    drop(host);
    let queued = store.pending().unwrap();
    assert!(!queued.is_empty());
    let (recovered, recovering) = peak_of(|| replica.queue_recovered_batch(queued));
    recovered.unwrap();
    let last = Id::from_proto(
        replica
            .pending()
            .last()
            .unwrap()
            .transaction
            .txn_id
            .as_ref(),
    )
    .unwrap();
    let (discarded, discarding) = peak_of(|| replica.discard_pending(&last));
    discarded.unwrap();
    recovering.max(discarding)
}
/// The phone draws `pending` strokes on its mirror while offline: they stay
/// queued. A real replay of that queue stays within the charge a commit on
/// the mirror then carries, and the mirror's workflows stay admitted.
async fn offline_phone(page: Page, pending: u32) {
    let root = tempfile::tempdir().unwrap();
    let host = root.path().join("host");
    draw_page(&host, page.strokes, page.points).await;
    let mirror = root.path().join("mirror");
    mirror_page(&host, &mirror, &source());
    let project = open(&mirror).await;
    draw(&project, 0x50, 0..pending, page.points).await;
    project.close().await.unwrap();
    let replayed = replay(&mirror);
    let project = open(&mirror).await;
    let info = project.info().await.unwrap();
    let verification = verification(&project).await;
    let planned = project
        .clone()
        .prepare_document_control(
            binding(&info),
            metadata(&info, 0x76, 1),
            rename("Offline"),
            BUDGET,
            cancel(),
        )
        .await;
    // The commit charge, or, where a cross-checking build refuses the plan,
    // the same charge read from that refusal beside the panel's work.
    let charge = match planned {
        Ok(plan) => {
            let charge = plan.commit_charge().await.unwrap();
            plan.dispose();
            charge
        }
        Err(WorkflowError::Memory { estimated, .. }) => {
            assert!(
                cross_checks_explain(estimated, verification),
                "a rename was refused at {estimated}"
            );
            estimated - CONTROL_WORK
        }
        Err(error) => panic!("planning a rename: {error:?}"),
    };
    report(
        &format!("replay of {pending} queued"),
        replayed,
        charge - RESERVE,
    );
    assert!(
        replayed <= charge - RESERVE,
        "replaying {pending} queued strokes used {replayed} > commit charge {}",
        charge - RESERVE
    );
    let page = Page {
        objects: page.objects + u64::from(pending),
        ..page
    };
    instruction_workflows(&project, 3, true).await;
    borrowing_workflows(&project, page, 3, root.path()).await;
    project.close().await.unwrap();
}

#[tokio::test]
async fn pages_of_hundreds_and_thousands_of_strokes_keep_every_workflow_within_its_charge() {
    let _serial = TESTS.lock().await;
    for strokes in [500, 2_000] {
        let objects = u64::from(strokes);
        page(
            Page {
                strokes,
                points: 100,
                objects,
            },
            true,
        )
        .await;
    }
}

#[tokio::test]
async fn a_phone_that_queued_strokes_offline_is_charged_the_replay_of_its_queue() {
    let _serial = TESTS.lock().await;
    // An almost empty page on which the phone draws two thousand strokes, a
    // page of five hundred with three hundred more from the phone, and one
    // of two thousand with twenty more.
    //
    // A cross-checking build hashes the whole visible page for every stroke
    // the phone queues, and refuses that first page's workflows for those
    // checks alone; it queues five hundred there, which its replay check
    // covers just as well, rather than spend minutes on the rest.
    let queued = if vw_ops::CROSS_CHECKED { 500 } else { 2_000 };
    offline_phone(
        Page {
            strokes: 2,
            points: 100,
            objects: 2,
        },
        queued,
    )
    .await;
    offline_phone(
        Page {
            strokes: 500,
            points: 100,
            objects: 500,
        },
        300,
    )
    .await;
    offline_phone(
        Page {
            strokes: 2_000,
            points: 100,
            objects: 2_000,
        },
        20,
    )
    .await;
}

/// An image whose asset carries a large ICC profile: a hash or a rewrite of
/// that asset turns every profile byte into a 32-byte JSON value.
fn profiled_page(profile: usize) -> Project {
    let author = DeviceId::try_from(device()).unwrap();
    let mut project = Project::new(Id::try_from(id(1)).unwrap(), "Profiled page".into(), author);
    let original = source();
    let asset = AssetId::hash(&original);
    let mut icc = vec![0u8; profile];
    for (index, byte) in icc.iter_mut().enumerate() {
        *byte = (index * 31 % 251) as u8;
    }
    project.assets.insert(
        asset.clone(),
        pb::AddAsset {
            asset_id: asset.to_string(),
            format: "png".into(),
            width: 64,
            height: 64,
            orientation: 1,
            bit_depth: 8,
            has_alpha: true,
            color_space: "ICC".into(),
            icc_profile: icc,
            byte_size: original.len() as u64,
            source: "import".into(),
            captured_at_ms: 0,
            metadata_json: "{}".into(),
        },
    );
    project.documents.insert(
        Id::try_from(id(2)).unwrap(),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(Id::try_from(id(2)).unwrap().to_proto()),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Profiled".into(),
                primary_asset_id: asset.to_string(),
                capture: None,
            },
            pages: vec![],
            created_at_ms: TIME,
        },
    );
    project.layers.insert(
        Id::try_from(id(3)).unwrap(),
        Layer {
            definition: pb::CreateLayer {
                layer_id: Some(Id::try_from(id(3)).unwrap().to_proto()),
                document_id: Some(Id::try_from(id(2)).unwrap().to_proto()),
                page_index: -1,
                name: "Ink".into(),
                kind: "annotation".into(),
                order_key: "V".into(),
            },
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: "normal".into(),
        },
    );
    project
}
/// A selection imported over that image adds a mask asset beside the large
/// one, a rename rewrites the members in front of `objects`, and reading
/// instructions hashes the profile: each within its charge, on the host and
/// on the phone's mirror.
#[tokio::test]
async fn a_large_icc_profile_is_charged_where_it_is_hashed_or_rewritten() {
    let _serial = TESTS.lock().await;
    let root = tempfile::tempdir().unwrap();
    let host = root.path().join("host");
    created(
        &host,
        with_strokes(profiled_page(2 * 1024 * 1024), 299, 100),
    );
    draw_last(&host, 300, 100).await;
    let page = Page {
        strokes: 300,
        points: 100,
        objects: 300,
    };
    let mirror = root.path().join("mirror");
    mirror_page(&host, &mirror, &source());
    for (round, path) in [(1, &host), (2, &mirror)] {
        let project = open(path).await;
        instruction_workflows(&project, round, true).await;
        layers_panel(&project, page, round).await;
        let document = project.selection_document(id(2), cancel()).await.unwrap();
        let mut coverage = vec![0u8; 64 * 64];
        coverage[..64].fill(255);
        let edit = |budget| SelectionEdit {
            binding: document.binding.clone(),
            transaction_id: wide_id(0x79, round),
            device_id: document.revision.device_id.clone(),
            lamport: document.revision.next_lamport,
            created_at_ms: TIME + 2_000_000 + i64::from(round),
            target: SelectionTarget::New {
                object_id: wide_id(0x7a, round),
                layer_id: wide_id(0x7b, round),
            },
            operation: SelectionOperation::ImportCoverage {
                width: 64,
                height: 64,
                coverage: coverage.clone(),
                combine: SelectionCombine::Add,
            },
            memory_budget_bytes: budget,
        };
        // Each stage refuses a budget short of its charge with that charge;
        // the import admitted with the largest of them stays within it.
        let verification = verification(&project).await;
        let mut budget = RESERVE + 1;
        let used = loop {
            let (result, used) = peak(project.apply_selection(edit(budget), cancel())).await;
            match result {
                Ok(_) => break Some(used),
                Err(SelectionError::Memory { estimated, .. }) if estimated > budget => {
                    if estimated > BUDGET {
                        assert!(
                            cross_checks_explain(estimated, verification),
                            "a selection import charged {estimated}"
                        );
                        break None;
                    }
                    budget = estimated;
                }
                Err(other) => panic!("expected a charge, got {other:?}"),
            }
        };
        let Some(used) = used else {
            project.close().await.unwrap();
            continue;
        };
        report("selection import", used, budget);
        assert!(
            used <= budget,
            "importing a selection used {used} > charge {budget}"
        );
        project.close().await.unwrap();
    }
}

/// A ready-made PC page holds exactly what drawing its strokes one by one
/// leaves: the same project, object for object.
#[tokio::test]
async fn ready_made_pages_hold_what_drawing_leaves() {
    let _serial = TESTS.lock().await;
    let root = tempfile::tempdir().unwrap();
    let drawn = root.path().join("drawn");
    let project = create_image_project(create(&drawn), cancel())
        .await
        .unwrap();
    draw(&project, 0x60, 0..5, 100).await;
    project.close().await.unwrap();
    let drawn = vw_store::ProjectStore::open(&drawn)
        .unwrap()
        .project()
        .clone();
    let made = with_strokes(blank_page(root.path()).await, 5, 100);
    assert_eq!(drawn.objects.len(), 5);
    assert!(drawn == made, "a ready-made page differs from a drawn one");
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
            objects: 8_000,
        },
        false,
    )
    .await;
}
