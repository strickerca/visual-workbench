//! How long each kind of edit that is not a new stroke takes to save as a
//! document fills up, and how long a project with such edits takes to open.
//!
//! One table per document size (500, 2,000 and 8,000 strokes), one row per
//! kind of edit: the median time of saving it through the store, which is
//! where an edit is decided, hashed and written, and the median of undoing
//! it. A second table shows the same edits through the calls the apps make,
//! which add what the bindings do around the store. After the tables a
//! project of the largest size takes 200 edits that are not strokes and is
//! opened again, with what the checkpoint rule made of them.
//!
//! Run: cargo bench -p vw-ffi --bench edits
//! Set VW_BENCH_SIZES to change the sizes (default 500,2000,8000; at least 140),
//! VW_BENCH_REOPEN=0 to skip the reopening part, and VW_BENCH_EDIT_BUDGET_MS
//! to fail when the median of any store-level edit other than the 50-object
//! move exceeds it at the largest size.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use vw_core::*;
use vw_model::{AssetId, DeviceId, Id, OrderKey};
use vw_proto::v1::{self as pb, op::Kind, property_value::Value};
use vw_raster::{AlphaPolicy, ColorPolicy, DecodedImage, ExportFormat, ExportRequest, Pixels};
use vw_store::{CheckpointNeed, ProjectStore, Upkeeping};

const TIME: u64 = 1_700_000_000_000;
const SAMPLES_PER_STROKE: usize = 48;
const MEASURED: usize = 9;
const MEMORY: u64 = 256 * 1024 * 1024;

fn raw_id(tag: u8, n: u32) -> Id {
    let mut entropy = [tag; 10];
    entropy[6..].copy_from_slice(&n.to_be_bytes());
    Id::from_parts(TIME, entropy).unwrap()
}
fn id(tag: u8, n: u32) -> String {
    raw_id(tag, n).to_string()
}
fn uid(tag: u8, n: u32) -> Option<pb::Uuid> {
    Some(raw_id(tag, n).to_proto())
}
fn device_id() -> DeviceId {
    DeviceId::from_bytes([7; 16])
}
fn device() -> String {
    device_id().to_string()
}
fn cancel() -> Arc<Cancellation> {
    Arc::new(Cancellation::new())
}
fn ms(value: Duration) -> f64 {
    value.as_secs_f64() * 1000.0
}
fn median(values: &mut [Duration]) -> Duration {
    values.sort();
    values[values.len() / 2]
}

fn source() -> Vec<u8> {
    let image = DecodedImage {
        width: 1024,
        height: 1024,
        pixels: Pixels::Rgba8(vec![255; 1024 * 1024 * 4]),
        icc: None,
        source_asset: AssetId::hash(b"edit-benchmark"),
        original_available: true,
        orientation_applied: 1,
    };
    vw_raster::export(
        &image,
        &ExportRequest {
            format: ExportFormat::Png8,
            region: None,
            revision: pb::Revision {
                host_seq: 0,
                state_hash: vec![0; 32],
            },
            alpha: AlphaPolicy::Preserve,
            color: ColorPolicy::Preserve,
            allow_depth_reduction: false,
            memory_budget_bytes: MEMORY,
            capture_session: None,
            frame_id: None,
            capture_provenance: None,
        },
    )
    .unwrap()
    .bytes
}

/// One pen stroke through the calls the apps make.
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
    gesture.commit(cancel()).await.unwrap();
    started.elapsed()
}

async fn create(path: &Path, strokes: u32) -> Arc<ProjectSession> {
    let project = create_image_project(
        CreateImageProject {
            path: path.to_string_lossy().into(),
            project_id: id(1, 0),
            document_id: id(2, 0),
            layer_id: id(3, 0),
            device_id: device(),
            title: "edit benchmark".into(),
            source: source(),
            now_ms: TIME as i64,
        },
        cancel(),
    )
    .await
    .unwrap();
    for n in 0..strokes {
        stroke(&project, n).await;
    }
    project
}

fn transform(n: u32) -> pb::Affine {
    pb::Affine {
        a: 1.0,
        d: 1.0,
        e: 1.5 + f64::from(n),
        f: 0.25 * f64::from(n),
        ..Default::default()
    }
}
fn property(object: u32, name: &str, value: Value) -> Kind {
    Kind::SetProperty(pb::SetProperty {
        object_id: uid(6, object),
        property: name.into(),
        value: Some(pb::PropertyValue { value: Some(value) }),
    })
}
fn layer_update(change: impl FnOnce(&mut pb::UpdateLayer)) -> Kind {
    let mut update = pb::UpdateLayer {
        layer_id: uid(3, 0),
        ..Default::default()
    };
    change(&mut update);
    Kind::UpdateLayer(update)
}
fn instruction(n: u32, text: String) -> Kind {
    Kind::SetInstruction(pb::SetInstruction {
        instruction_id: uid(8, n),
        document_id: uid(2, 0),
        target_object_ids: vec![raw_id(6, 1).to_proto()],
        role: pb::Role::Change as i32,
        text,
        entry_method: "pc_keyboard".into(),
        language: "en".into(),
    })
}
fn mask_asset(n: u32) -> pb::AddAsset {
    pb::AddAsset {
        asset_id: AssetId::hash(format!("mask {n}").as_bytes()).to_string(),
        format: "png".into(),
        width: 1024,
        height: 1024,
        orientation: 1,
        bit_depth: 8,
        color_space: "gray".into(),
        byte_size: 4096,
        source: "import".into(),
        ..Default::default()
    }
}
fn mask_shape(n: u32) -> pb::object_state::Shape {
    pb::object_state::Shape::SelectionRaster(pb::RasterMaskRef {
        mask_asset_id: mask_asset(n).asset_id,
        bounds: Some(pb::RectD {
            x: 0.0,
            y: 0.0,
            w: 1024.0,
            h: 1024.0,
        }),
        feather: 0.0,
    })
}
fn object(id: Option<pb::Uuid>, layer: Option<pb::Uuid>, order: &str) -> pb::ObjectState {
    pb::ObjectState {
        object_id: id,
        layer_id: layer,
        order_key: order.into(),
        transform: Some(pb::Affine {
            a: 1.0,
            d: 1.0,
            ..Default::default()
        }),
        style: Some(pb::Style {
            stroke: Some(pb::Color { rgba: 0x203040ff }),
            width: 1.0,
            ..Default::default()
        }),
        role: pb::Role::None as i32,
        created_by: device(),
        created_at_ms: TIME as i64,
        ..Default::default()
    }
}

/// Edits made straight on the store, each as one transaction.
struct Editor {
    store: ProjectStore,
    next: u32,
    last: Option<pb::Uuid>,
}
impl Editor {
    fn open(path: &Path) -> Self {
        Self {
            store: ProjectStore::open(path).unwrap(),
            next: 0,
            last: None,
        }
    }
    fn commit(&mut self, kinds: Vec<Kind>) -> Duration {
        self.next += 1;
        let time = TIME as i64 + 10_000_000 + i64::from(self.next);
        let transaction = pb::Transaction {
            txn_id: uid(9, self.next),
            project_id: uid(1, 0),
            device_id: device(),
            base_revision: Some(self.store.revision().unwrap()),
            created_at_wall_ms: time,
            ops: kinds
                .into_iter()
                .enumerate()
                .map(|(index, kind)| pb::Op {
                    op_id: Some(pb::OpId {
                        device_id: device(),
                        lamport: 1_000_000_000
                            + u64::from(self.next) * 4096
                            + index as u64,
                    }),
                    kind: Some(kind),
                })
                .collect(),
            gesture_id: None,
        };
        let started = Instant::now();
        self.store
            .commit(&transaction, &device_id(), time)
            .unwrap_or_else(|error| panic!("edit {} refused: {error:?}", self.next));
        let elapsed = started.elapsed();
        self.last = transaction.txn_id;
        elapsed
    }
    fn undo(&mut self) -> Duration {
        let target = self.last.clone();
        self.commit(vec![Kind::UndoTransaction(pb::UndoTransaction {
            target_txn_id: target,
        })])
    }
    /// The median of an edit made `MEASURED` times and of undoing it.
    fn row(&mut self, name: &str, mut edit: impl FnMut(u32) -> Vec<Kind>, undo: bool) -> f64 {
        let mut edits = Vec::new();
        let mut undos = Vec::new();
        for n in 0..MEASURED as u32 {
            edits.push(self.commit(edit(n)));
            if undo {
                undos.push(self.undo());
            }
        }
        let edit = ms(median(&mut edits));
        if undo {
            println!("{name:<34} | {edit:>9.2} ms | {:>9.2} ms", ms(median(&mut undos)));
        } else {
            println!("{name:<34} | {edit:>9.2} ms |            -");
        }
        edit
    }
}

/// Every kind of edit on the store. Returns the slowest median among the
/// edits that touch a fixed handful of entities.
fn store_rows(path: &Path, strokes: u32) -> f64 {
    let mut editor = Editor::open(path);
    println!("\nsaved through the store, {strokes} strokes in the document");
    println!("{:<34} | {:>12} | {:>12}", "edit", "save", "undo");
    let mut slowest: f64 = 0.0;
    let mut order = OrderKey::try_from("zzzz".to_owned()).unwrap();
    let layer = uid(3, 0);
    let mut next_order = || {
        order = OrderKey::between(Some(&order), None).unwrap();
        order.as_str().to_owned()
    };
    // The reference: one more stroke, which lands at the end of the objects.
    let mut new = 0;
    slowest = slowest.max(editor.row(
        "new stroke",
        |_| {
            new += 1;
            let mut state = object(uid(7, new), layer.clone(), &next_order());
            state.shape = Some(pb::object_state::Shape::Stroke(pb::Stroke {
                x: (0..SAMPLES_PER_STROKE).map(|i| 20.0 + i as f64).collect(),
                y: (0..SAMPLES_PER_STROKE).map(|i| 20.0 + i as f64 * 0.5).collect(),
                t_ms: (0..SAMPLES_PER_STROKE).map(|i| (i * 4) as u32).collect(),
                pressure: vec![0.6; SAMPLES_PER_STROKE],
                tilt: vec![],
                orientation: vec![],
                brush: Some(pb::Brush {
                    family: "pen".into(),
                    algorithm_version: 1,
                    base_width: 4.0,
                    pressure_curve: vec![0.0, 0.0, 0.25, 0.25, 0.75, 0.75, 1.0, 1.0],
                    stabilization: 0.0,
                }),
            }));
            vec![Kind::CreateObject(pb::CreateObject {
                document_id: uid(2, 0),
                state: Some(state),
            })]
        },
        true,
    ));
    slowest = slowest.max(editor.row(
        "move the first stroke",
        |n| vec![property(0, "transform", Value::Transform(transform(n)))],
        true,
    ));
    slowest = slowest.max(editor.row(
        "restyle the first stroke",
        |n| {
            vec![property(
                0,
                "style",
                Value::Style(pb::Style {
                    stroke: Some(pb::Color {
                        rgba: 0x10203000 + n,
                    }),
                    width: 2.0 + f64::from(n),
                    ..Default::default()
                }),
            )]
        },
        true,
    ));
    slowest = slowest.max(editor.row(
        "delete an early stroke",
        |n| {
            vec![Kind::DeleteObject(pb::DeleteObject {
                object_id: uid(6, 10 + n),
            })]
        },
        true,
    ));
    slowest = slowest.max(editor.row(
        "erase part of an early stroke",
        |n| {
            let mut state = object(uid(7, 1_000_000 + n), layer.clone(), &next_order());
            state.style = Some(pb::Style {
                stroke: Some(pb::Color { rgba: 0x203040ff }),
                fill: Some(pb::Color { rgba: 0x203040ff }),
                has_fill: true,
                ..Default::default()
            });
            state.shape = Some(pb::object_state::Shape::Polygon(pb::Polyline {
                points: (0..64)
                    .map(|i| pb::PointD {
                        x: 20.0 + f64::from(i) * 1.25,
                        y: 30.0 + f64::from(i % 7) * 2.5,
                    })
                    .collect(),
                closed: true,
            }));
            vec![
                Kind::DeleteObject(pb::DeleteObject {
                    object_id: uid(6, 20 + n),
                }),
                Kind::CreateObject(pb::CreateObject {
                    document_id: uid(2, 0),
                    state: Some(state),
                }),
            ]
        },
        true,
    ));
    slowest = slowest.max(editor.row(
        "rename the layer",
        |n| vec![layer_update(|u| u.name = Some(format!("Layer {n}")))],
        true,
    ));
    slowest = slowest.max(editor.row(
        "reorder the layer",
        |n| vec![layer_update(|u| u.order_key = Some(if n % 2 == 0 { "W" } else { "X" }.into()))],
        true,
    ));
    slowest = slowest.max(editor.row(
        "hide or show the layer",
        |n| vec![layer_update(|u| u.visible = Some(n % 2 == 1))],
        true,
    ));
    slowest = slowest.max(editor.row(
        "layer opacity",
        |n| vec![layer_update(|u| u.opacity = Some(0.5 + n as f32 / 64.0))],
        true,
    ));
    slowest = slowest.max(editor.row(
        "document title",
        |n| {
            vec![Kind::UpdateDocument(pb::UpdateDocument {
                document_id: uid(2, 0),
                title: format!("Title {n}"),
            })]
        },
        true,
    ));
    slowest = slowest.max(editor.row(
        "add an instruction",
        |n| vec![instruction(n, format!("Make this part brighter ({n})"))],
        true,
    ));
    editor.commit(vec![instruction(1000, "Make this part darker".into())]);
    slowest = slowest.max(editor.row(
        "edit an instruction",
        |n| vec![instruction(1000, format!("Make this part darker ({n})"))],
        true,
    ));
    // A selection mask: its layer and object once, then one edit per row.
    editor.commit(vec![
        Kind::CreateLayer(pb::CreateLayer {
            layer_id: uid(3, 1),
            document_id: uid(2, 0),
            page_index: -1,
            name: "Selection".into(),
            kind: "mask".into(),
            order_key: "k".into(),
        }),
        Kind::AddAsset(mask_asset(0)),
        Kind::CreateObject(pb::CreateObject {
            document_id: uid(2, 0),
            state: Some({
                let mut state = object(uid(7, 2_000_000), uid(3, 1), "V");
                state.shape = Some(mask_shape(0));
                state
            }),
        }),
    ]);
    slowest = slowest.max(editor.row(
        "edit a selection mask",
        |n| {
            vec![
                Kind::AddAsset(mask_asset(n + 1)),
                Kind::SetProperty(pb::SetProperty {
                    object_id: uid(7, 2_000_000),
                    property: "shape".into(),
                    value: Some(pb::PropertyValue {
                        value: Some(Value::Raw(serde_json::to_vec(&mask_shape(n + 1)).unwrap())),
                    }),
                }),
                Kind::MaskOp(pb::MaskOp {
                    output_object_id: uid(7, 2_000_000),
                    op: "add".into(),
                    input_object_ids: vec![],
                    amount: 0.0,
                }),
            ]
        },
        true,
    ));
    slowest = slowest.max(editor.row(
        "add a result",
        |n| {
            vec![Kind::AddResult(pb::AddResult {
                result_id: uid(10, n),
                document_id: uid(2, 0),
                provider: "benchmark".into(),
                model: "benchmark".into(),
                request_json: "{}".into(),
                ..Default::default()
            })]
        },
        true,
    ));
    editor.commit(vec![Kind::AddResult(pb::AddResult {
        result_id: uid(10, 1000),
        document_id: uid(2, 0),
        provider: "benchmark".into(),
        model: "benchmark".into(),
        request_json: "{}".into(),
        ..Default::default()
    })]);
    slowest = slowest.max(editor.row(
        "change a result's status",
        |n| {
            vec![Kind::UpdateResult(pb::UpdateResult {
                result_id: uid(10, 1000),
                status: if n % 2 == 0 { "rejected" } else { "error" }.into(),
                acceptance_mask_asset_id: String::new(),
            })]
        },
        true,
    ));
    editor.row(
        "move 50 early strokes at once",
        |n| {
            (0..50)
                .map(|k| property(30 + k, "transform", Value::Transform(transform(n + k))))
                .collect()
        },
        true,
    );
    slowest
}

fn binding(info: &ProjectInfo) -> WorkflowBinding {
    WorkflowBinding {
        project_id: info.project_id.clone(),
        document_id: id(2, 0),
        host_seq: info.host_seq,
        state_hash: info.state_hash.clone(),
    }
}

/// The same edits through the calls the apps make: what a person waits for.
async fn app_rows(project: &Arc<ProjectSession>, strokes: u32, source_asset: &str) {
    println!("\nsaved through the calls the apps make, {strokes} strokes in the document");
    println!("{:<34} | {:>12} | {:>12}", "edit", "save", "undo");
    let mut next = 0u32;
    let mut options = |info: &ProjectInfo| {
        next += 1;
        EditOptions {
            transaction_id: id(11, next),
            document_id: id(2, 0),
            device_id: device(),
            lamport: info.next_lamport,
            created_at_ms: TIME as i64 + 20_000_000 + i64::from(next),
        }
    };
    let print = |name: &str, edits: &mut [Duration], undos: &mut [Duration]| {
        if undos.is_empty() {
            println!("{name:<34} | {:>9.2} ms |            -", ms(median(edits)));
        } else {
            println!(
                "{name:<34} | {:>9.2} ms | {:>9.2} ms",
                ms(median(edits)),
                ms(median(undos))
            );
        }
    };
    let (mut edits, mut undos) = (Vec::new(), Vec::new());
    for n in 0..MEASURED as u32 {
        edits.push(stroke(project, 3_000_000 + n).await);
    }
    print("new stroke", &mut edits, &mut undos);
    for (name, objects) in [("move the first stroke", 1u32), ("move 50 early strokes at once", 50)]
    {
        let (mut edits, mut undos) = (Vec::new(), Vec::new());
        for n in 0..MEASURED as u32 {
            let info = project.info().await.unwrap();
            let commands = (0..objects)
                .map(|k| EditCommand::Transform {
                    object_id: id(6, 80 + k),
                    transform: Transform {
                        a: 1.0,
                        b: 0.0,
                        c: 0.0,
                        d: 1.0,
                        e: 2.5 + f64::from(n + k),
                        f: 0.5,
                    },
                })
                .collect();
            let started = Instant::now();
            let info = project
                .apply_edit(options(&info), commands, cancel())
                .await
                .unwrap();
            edits.push(started.elapsed());
            let started = Instant::now();
            project
                .undo_redo(options(&info), false, cancel())
                .await
                .unwrap();
            undos.push(started.elapsed());
        }
        print(name, &mut edits, &mut undos);
    }
    let mut undos = Vec::new();
    let mut refused: Option<WorkflowError> = None;
    let report = |name: &str, edits: &mut [Duration], refused: Option<WorkflowError>| match refused {
        Some(error) => println!("{name:<34} | refused: {error}"),
        None => println!("{name:<34} | {:>9.2} ms |            -", ms(median(edits))),
    };
    let mut edits = Vec::new();
    for n in 0..MEASURED as u32 {
        let info = project.info().await.unwrap();
        next += 1;
        let started = Instant::now();
        let plan = project
            .clone()
            .prepare_instruction(
                binding(&info),
                WorkflowMetadata {
                    transaction_id: id(11, next),
                    device_id: device(),
                    first_lamport: info.next_lamport,
                    created_at_ms: TIME as i64 + 20_000_000 + i64::from(next),
                },
                InstructionCommand::SetInstruction {
                    instruction_id: id(12, 0),
                    target_ids: vec![id(6, 1)],
                    role: InstructionRole::Change,
                    text: format!("Make this part brighter ({n})"),
                    entry_method: InstructionEntryMethod::PcKeyboard,
                    language: "en".into(),
                },
                MEMORY,
                cancel(),
            )
            .await;
        let plan = match plan {
            Ok(plan) => plan,
            Err(error) => {
                refused = Some(error);
                break;
            }
        };
        plan.commit(cancel()).await.unwrap();
        edits.push(started.elapsed());
        plan.dispose();
    }
    report("edit an instruction", &mut edits, refused.take());
    // The layer panel refuses documents of more than 4,096 objects.
    let mut edits = Vec::new();
    for n in 0..MEASURED as u32 {
        let info = project.info().await.unwrap();
        next += 1;
        let started = Instant::now();
        let plan = project
            .clone()
            .prepare_document_control(
                binding(&info),
                WorkflowMetadata {
                    transaction_id: id(11, next),
                    device_id: device(),
                    first_lamport: info.next_lamport,
                    created_at_ms: TIME as i64 + 20_000_000 + i64::from(next),
                },
                DocumentControlCommand::UpdateLayer {
                    update: LayerControlUpdate {
                        layer_id: id(3, 0),
                        name: Some(format!("Layer {n}")),
                        visible: None,
                        locked: None,
                        opacity: None,
                        blend: None,
                    },
                },
                MEMORY,
                cancel(),
            )
            .await;
        let plan = match plan {
            Ok(plan) => plan,
            Err(error) => {
                refused = Some(error);
                break;
            }
        };
        plan.commit(cancel()).await.unwrap();
        edits.push(started.elapsed());
        plan.dispose();
    }
    report("rename the layer", &mut edits, refused.take());
    let mut edits = Vec::new();
    for n in 0..MEASURED as u32 {
        let info = project.info().await.unwrap();
        next += 1;
        let target = 130 + n;
        let started = Instant::now();
        let erased = project
            .erase_vector_strokes(
                VectorEraseOptions {
                    binding: binding(&info),
                    metadata: WorkflowMetadata {
                        transaction_id: id(11, next),
                        device_id: device(),
                        first_lamport: info.next_lamport,
                        created_at_ms: TIME as i64 + 20_000_000 + i64::from(next),
                    },
                    targets: vec![VectorEraseTarget {
                        object_id: id(6, target),
                        replacement_id: id(13, n),
                    }],
                    centers: vec![Point {
                        x: 20.0 + f64::from(target % 900) + 24.0,
                        y: 20.0 + f64::from((target * 7) % 900) + 12.0,
                    }],
                    radius: 3.0,
                    memory_budget_bytes: MEMORY,
                    max_output_vertices: 16_384,
                    max_work_units: 50_000_000,
                    page_index: None,
                },
                cancel(),
            )
            .await;
        match erased {
            Ok(erased) => assert_eq!(erased.changed.len(), 1),
            Err(error) => {
                refused = Some(error);
                break;
            }
        }
        edits.push(started.elapsed());
    }
    report("erase part of an early stroke", &mut edits, refused.take());
    let mut edits = Vec::new();
    let mut selection = None;
    for n in 0..=MEASURED as u32 {
        let info = project.info().await.unwrap();
        next += 1;
        let started = Instant::now();
        let receipt = project
            .apply_selection(
                SelectionEdit {
                    binding: SelectionBinding {
                        project_id: info.project_id.clone(),
                        document_id: id(2, 0),
                        source_asset_id: source_asset.into(),
                        host_seq: info.host_seq,
                        state_hash: info.state_hash.clone(),
                    },
                    transaction_id: id(11, next),
                    device_id: device(),
                    lamport: info.next_lamport,
                    created_at_ms: TIME as i64 + 20_000_000 + i64::from(next),
                    target: match selection.take() {
                        Some(selection) => SelectionTarget::Existing { selection },
                        None => SelectionTarget::New {
                            object_id: id(14, 0),
                            layer_id: id(3, 2),
                        },
                    },
                    operation: SelectionOperation::Rectangle {
                        rectangle: QueryRect {
                            x: 8.0 + f64::from(n),
                            y: 8.0,
                            width: 16.0,
                            height: 16.0,
                        },
                        combine: SelectionCombine::Add,
                    },
                    memory_budget_bytes: MEMORY,
                },
                cancel(),
            )
            .await;
        let receipt = match receipt {
            Ok(receipt) => receipt,
            Err(error) => {
                println!("{:<34} | refused: {error}", "edit a selection mask");
                edits.clear();
                break;
            }
        };
        // The first one creates the mask; the rest edit it.
        if n > 0 {
            edits.push(started.elapsed());
        }
        selection = Some(receipt.snapshot.selection);
    }
    if !edits.is_empty() {
        print("edit a selection mask", &mut edits, &mut undos);
    }
    // What the editor asks for after every change, holding every stroke. A
    // complete read is refused once the document counts more than 64 MB.
    let all = match project
        .document_snapshot_changes(id(2, 0), vec![], cancel())
        .await
    {
        Ok(all) => all,
        Err(error) => {
            println!(
                "{:<34} | refused: {error}",
                "refresh of the drawing afterwards"
            );
            return;
        }
    };
    let known = || {
        all.items
            .iter()
            .filter_map(|item| {
                Some(KnownStroke {
                    object_id: item.object_id.clone(),
                    digest: item.digest.clone()?,
                    cost: item.cost,
                })
            })
            .collect::<Vec<_>>()
    };
    let mut edits = Vec::new();
    for _ in 0..MEASURED {
        let held = known();
        let started = Instant::now();
        project
            .document_snapshot_changes(id(2, 0), held, cancel())
            .await
            .unwrap();
        edits.push(started.elapsed());
    }
    print("refresh of the drawing afterwards", &mut edits, &mut undos);
}

fn need(store: &ProjectStore) -> &'static str {
    match store.checkpoint_need() {
        CheckpointNeed::None => "none",
        CheckpointNeed::Due => "due",
        CheckpointNeed::Overdue => "overdue",
        CheckpointNeed::Urgent => "urgent",
    }
}
fn upkeep(step: &Upkeeping) -> String {
    match step {
        Upkeeping::NotDue => "not due".into(),
        Upkeeping::Done(elapsed) => format!("written in {:.0} ms", ms(*elapsed)),
        Upkeeping::Failed(elapsed, error) => format!("failed after {:.0} ms: {error}", ms(*elapsed)),
        Upkeeping::Skipped => "skipped".into(),
    }
}

/// 200 edits that are not strokes on a project of `strokes` strokes, then
/// opening it again the way it opens after the application was killed.
fn reopen(path: &Path, strokes: u32) {
    println!("\nopening again after 200 edits that are not strokes, {strokes} strokes");
    let started = Instant::now();
    let mut editor = Editor::open(path);
    println!(
        "opening with strokes only: {:.0} ms ({} transactions decided again)",
        ms(started.elapsed()),
        editor.store.replayed_transactions()
    );
    let mut total = Duration::ZERO;
    let mut said = "none";
    for n in 0..200u32 {
        total += editor.commit(match n % 4 {
            0 => vec![property(0, "transform", Value::Transform(transform(n)))],
            1 => vec![layer_update(|u| u.name = Some(format!("Layer {n}")))],
            2 => vec![Kind::UpdateDocument(pb::UpdateDocument {
                document_id: uid(2, 0),
                title: format!("Title {n}"),
            })],
            _ => vec![instruction(0, format!("Make this part darker ({n})"))],
        });
        let now = need(&editor.store);
        if now != said {
            println!("after {} such edits a checkpoint is {now}", n + 1);
            said = now;
        }
    }
    println!(
        "saving the 200 edits: {:.0} ms in all, {:.2} ms each; a checkpoint is {said}",
        ms(total),
        ms(total) / 200.0
    );
    // Dropped without a checkpoint, as when the application is killed.
    drop(editor);
    for attempt in 1..=2 {
        let started = Instant::now();
        let store = ProjectStore::open(path).unwrap();
        let elapsed = started.elapsed();
        let report = store.open_report();
        let checkpoint = match &report.checkpoint {
            Upkeeping::Done(elapsed) | Upkeeping::Failed(elapsed, _) => *elapsed,
            _ => Duration::ZERO,
        };
        println!(
            "opening again (attempt {attempt}): {:.0} ms, of which checkpoint {:.0} ms; {} transactions decided again; checkpoint {}; afterwards a checkpoint is {}",
            ms(elapsed),
            ms(checkpoint),
            store.replayed_transactions(),
            upkeep(&report.checkpoint),
            need(&store)
        );
    }
}

fn main() {
    let sizes: Vec<u32> = std::env::var("VW_BENCH_SIZES")
        .ok()
        .map(|sizes| {
            sizes
                .split(',')
                .filter_map(|size| size.trim().parse().ok())
                .collect()
        })
        .unwrap_or_else(|| vec![500, 2000, 8000]);
    let reopening = std::env::var("VW_BENCH_REOPEN").map_or(true, |value| value != "0");
    let budget: Option<f64> = std::env::var("VW_BENCH_EDIT_BUDGET_MS")
        .ok()
        .and_then(|value| value.parse().ok());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let source_asset = AssetId::hash(&source()).to_string();
    let mut slowest = 0.0;
    for (index, strokes) in sizes.iter().copied().enumerate() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("project");
        runtime.block_on(async {
            let project = create(&path, strokes).await;
            project.close().await.unwrap();
        });
        if reopening && index + 1 == sizes.len() {
            let copy = root.path().join("reopened");
            std::fs::create_dir(&copy).unwrap();
            copy_directory(&path, &copy);
            reopen(&copy, strokes);
            std::fs::remove_dir_all(&copy).unwrap();
        }
        slowest = store_rows(&path, strokes);
        runtime.block_on(async {
            let project = open_project(path.to_string_lossy().into(), cancel())
                .await
                .unwrap();
            app_rows(&project, strokes, &source_asset).await;
            let started = Instant::now();
            project.close().await.unwrap();
            println!("closing the project: {:.0} ms", ms(started.elapsed()));
        });
    }
    if let Some(budget) = budget
        && slowest > budget
    {
        panic!("edit budget exceeded: {slowest:.2} ms > {budget:.2} ms");
    }
}

fn copy_directory(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir(&target).unwrap();
            copy_directory(&entry.path(), &target);
        } else if entry.file_name() != ".store.lock" {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}
