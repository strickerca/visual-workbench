//! The packed document reads carry exactly the display list of the plain
//! reads, with every stroke outline in its compact form. What they count is
//! an upper bound of what a read allocates (measured here in Rust, reasoned
//! for Kotlin), and so much less than the plain read that a page of
//! handwriting the plain read refuses is read whole after every save.
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
use vw_model::Id;

/// Counts the bytes held at once; a reallocation holds both blocks, as the
/// default `realloc` does on its way from the old block to the new one.
struct Meter;
static HELD: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
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
        }
        block
    }
    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        HELD.fetch_sub(layout.size(), Ordering::SeqCst);
        // SAFETY: the block and layout are the pair this allocator handed out.
        unsafe { System.dealloc(block, layout) }
    }
}
/// The most bytes held at once while `work` ran, beyond what was held before.
async fn peak_during<T>(work: impl std::future::Future<Output = T>) -> (T, usize) {
    let before = HELD.load(Ordering::SeqCst);
    PEAK.store(before, Ordering::SeqCst);
    let value = work.await;
    (value, PEAK.load(Ordering::SeqCst).saturating_sub(before))
}
/// The allocation measurement needs the process to itself: every test here
/// runs alone on its own runtime.
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

/// An independent reader of the layout documented on `PackedRenderItem`.
fn unpack(bytes: &[u8]) -> Contours {
    let mut at = 0;
    let mut next = || {
        let mut value = 0_u64;
        for shift in (0..64).step_by(7) {
            let byte = bytes[at];
            at += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return value;
            }
        }
        panic!("number longer than ten bytes");
    };
    let signed = |value: u64| ((value >> 1) as i64) ^ -((value & 1) as i64);
    let mut contours = Contours::default();
    if bytes.is_empty() {
        return contours;
    }
    assert_eq!(next(), 1, "format version");
    let count = next();
    let vertices = next();
    let (mut x, mut y) = (0_i64, 0_i64);
    for _ in 0..count {
        for _ in 0..next() {
            x = x.wrapping_add(signed(next()));
            y = y.wrapping_add(signed(next()));
            contours.x.push(x);
            contours.y.push(y);
        }
        contours.ends.push(contours.x.len() as u32);
    }
    assert_eq!(contours.x.len() as u64, vertices);
    assert_eq!(at, bytes.len(), "trailing bytes");
    contours
}

fn wide_id(kind: u8, number: u32) -> String {
    let mut entropy = [kind; 10];
    entropy[6..].copy_from_slice(&number.to_be_bytes());
    Id::from_parts(TIME as u64, entropy).unwrap().to_string()
}
/// A stroke as the phone draws it with its default brushes: 240 samples a
/// second along a looping line of handwriting, pressure varying, sent in
/// batches of 64 samples.
async fn handwriting(
    project: &Arc<ProjectSession>,
    number: u32,
    samples: usize,
    family: &str,
    width: f64,
    layer_id: String,
) -> String {
    let info = project.info().await.unwrap();
    let options = StrokeOptions {
        gesture_id: wide_id(0x41, number),
        transaction_id: wide_id(0x42, number),
        object_id: wide_id(0x60, number),
        document_id: id(2),
        layer_id,
        device_id: device(),
        lamport: info.next_lamport,
        created_at_ms: TIME + 10 + i64::from(number),
        family: family.into(),
        width,
        rgba: 0x51c7c2ff,
        stabilization: 0.15,
        pressure_curve: vec![0.0, 0.0, 0.25, 0.25, 0.75, 0.75, 1.0, 1.0],
    };
    let gesture = project.clone().begin_stroke(options.clone()).await.unwrap();
    let (x0, y0) = (
        f64::from(number % 12) * 80.0,
        f64::from(number / 12 % 40) * 30.0,
    );
    for (sequence, start) in (0..samples).step_by(64).enumerate() {
        let range = start..samples.min(start + 64);
        let i = |n: usize| n as f64;
        gesture
            .append_samples(SampleBatch {
                sequence: sequence as u64 + 1,
                x: range
                    .clone()
                    .map(|n| x0 + 1.4 * i(n) + 5.0 * (0.45 * i(n)).sin())
                    .collect(),
                y: range
                    .clone()
                    .map(|n| y0 + 9.0 * (0.31 * i(n)).sin() + 3.0 * (0.9 * i(n)).cos())
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
    options.object_id
}
async fn edit(project: &Arc<ProjectSession>, number: u32, commands: Vec<EditCommand>) {
    let options = EditOptions {
        transaction_id: wide_id(0x43, number),
        document_id: id(2),
        device_id: device(),
        lamport: project.info().await.unwrap().next_lamport,
        created_at_ms: TIME + 100_000 + i64::from(number),
    };
    project
        .apply_edit(options, commands, cancel())
        .await
        .unwrap();
}
fn style() -> ObjectStyle {
    ObjectStyle {
        rgba: 0x203040ff,
        width: 2.0,
        screen_constant_width: false,
        fill: Some(0x10203040),
    }
}
fn identity() -> Transform {
    Transform {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    }
}
/// Every field of the plain item except its stored bytes, which the packed
/// item leaves out, and its outline, compared value for value.
fn assert_same_item(plain: &RenderItem, packed: &PackedRenderItem) {
    assert_eq!(plain.object_id, packed.object_id);
    assert_eq!(plain.layer_id, packed.layer_id);
    assert_eq!(
        plain.layer_opacity.to_bits(),
        packed.layer_opacity.to_bits()
    );
    assert_eq!(plain.layer_blend, packed.layer_blend);
    assert_eq!(
        format!("{:?}", plain.bounds),
        format!("{:?}", packed.bounds)
    );
    assert_eq!(
        format!("{:?}", plain.transform),
        format!("{:?}", packed.transform)
    );
    assert_eq!(format!("{:?}", plain.style), format!("{:?}", packed.style));
    assert_eq!(format!("{:?}", plain.shape), format!("{:?}", packed.shape));
    assert_eq!(plain.locked, packed.locked);
    assert_eq!(unpack(&packed.contours), plain.stroke_contours);
    assert_eq!(
        packed.contours.is_empty(),
        plain.stroke_contours.ends.is_empty()
    );
}
fn assert_same_document(plain: &DocumentSnapshot, packed: &PackedDocumentSnapshot) {
    assert_eq!(plain.document_id, packed.document_id);
    assert_eq!(plain.title, packed.title);
    assert_eq!(
        (plain.width, plain.height, plain.bit_depth),
        (packed.width, packed.height, packed.bit_depth)
    );
    assert_eq!(
        format!("{:?}", plain.layers),
        format!("{:?}", packed.layers)
    );
    assert_eq!(plain.render.revision, packed.render.revision);
    assert_eq!(plain.has_tone_adjustments, packed.has_tone_adjustments);
    assert_eq!(plain.render.items.len(), packed.render.items.len());
    for (plain, packed) in plain.render.items.iter().zip(&packed.render.items) {
        assert_same_item(plain, packed);
    }
}

#[test]
fn packed_reads_carry_the_plain_reads_value_for_value() {
    alone(async {
        let dir = tempfile::tempdir().unwrap();
        let project = create_image_project(create(&dir.path().join("project")), cancel())
            .await
            .unwrap();
        let mut strokes = Vec::new();
        for (number, (samples, family, width)) in [
            (3, "pen", 6.0),
            (50, "pen", 6.0),
            (100, "pen", 6.0),
            (300, "pen", 6.0),
            (100, "highlighter", 24.0),
            (100, "marker", 12.0),
            (40, "pen", 0.25),
        ]
        .into_iter()
        .enumerate()
        {
            strokes.push(handwriting(&project, number as u32, samples, family, width, id(3)).await);
        }
        edit(
            &project,
            1,
            vec![
                EditCommand::Create {
                    object_id: wide_id(0x50, 1),
                    layer_id: id(3),
                    shape: NewShape::Rectangle {
                        rectangle: QueryRect {
                            x: 4.0,
                            y: 5.0,
                            width: 20.0,
                            height: 10.0,
                        },
                    },
                    style: style(),
                    transform: identity(),
                },
                EditCommand::Create {
                    object_id: wide_id(0x50, 2),
                    layer_id: id(3),
                    shape: NewShape::Text {
                        anchor: Point { x: 8.0, y: 30.0 },
                        text: "ink".into(),
                        font: "Inter".into(),
                        size: 12.0,
                    },
                    style: style(),
                    transform: identity(),
                },
                EditCommand::Create {
                    object_id: wide_id(0x50, 3),
                    layer_id: id(3),
                    shape: NewShape::Arrow {
                        points: vec![Point { x: 1.0, y: 2.0 }, Point { x: 40.0, y: 9.5 }],
                    },
                    style: style(),
                    transform: identity(),
                },
                // A moved and turned stroke keeps its local outline.
                EditCommand::Transform {
                    object_id: strokes[2].clone(),
                    transform: Transform {
                        a: 0.6,
                        b: 0.8,
                        c: -0.8,
                        d: 0.6,
                        e: 1e6,
                        f: -3.25,
                    },
                },
            ],
        )
        .await;

        let plain = project.document_snapshot(id(2), cancel()).await.unwrap();
        let packed = project
            .document_snapshot_packed(id(2), cancel())
            .await
            .unwrap();
        assert_eq!(plain.render.items.len(), strokes.len() + 3);
        assert_same_document(&plain, &packed);

        // The changes reads: the same digests, each item exactly as the plain
        // read, and a packed cost far below the plain one.
        let lists = project
            .document_snapshot_changes(id(2), vec![], cancel())
            .await
            .unwrap();
        let changes = project
            .document_snapshot_changes_packed(id(2), vec![], cancel())
            .await
            .unwrap();
        assert_eq!(changes.revision, plain.render.revision);
        assert_eq!(changes.items.len(), plain.render.items.len());
        let mut known = Vec::new();
        for ((change, list), item) in changes
            .items
            .iter()
            .zip(&lists.items)
            .zip(&plain.render.items)
        {
            assert_eq!(change.object_id, item.object_id);
            assert_eq!(change.digest, list.digest);
            assert_same_item(item, change.item.as_ref().unwrap());
            assert!(change.cost >= 1024);
            if let Some(digest) = &change.digest {
                assert!(change.cost < list.cost);
                if item.stroke_contours.x.len() > 500 {
                    assert!(
                        change.cost * 3 < list.cost,
                        "{} against {}",
                        change.cost,
                        list.cost
                    );
                }
                known.push(KnownStroke {
                    object_id: change.object_id.clone(),
                    digest: digest.clone(),
                    cost: change.cost,
                });
            }
        }
        assert_eq!(known.len(), strokes.len());
        // Held strokes come back kept, with the cost they were sent with.
        let again = project
            .document_snapshot_changes_packed(id(2), known.clone(), cancel())
            .await
            .unwrap();
        for (change, sent) in again.items.iter().zip(&changes.items) {
            assert_eq!(change.item.is_none(), sent.digest.is_some());
            assert_eq!((change.cost, &change.digest), (sent.cost, &sent.digest));
            assert_eq!(change.layer_blend, sent.layer_blend);
            assert_eq!(change.locked, sent.locked);
        }

        // What one stroke counts, by its number of pen samples.
        println!("samples | vertices | plain cost | packed cost | packed bytes");
        for (n, item) in plain.render.items.iter().enumerate().take(4) {
            println!(
                "{:7} | {:8} | {:10} | {:11} | {:12}",
                [3, 50, 100, 300][n],
                item.stroke_contours.x.len(),
                lists.items[n].cost,
                changes.items[n].cost,
                packed.render.items[n].contours.len(),
            );
        }
        project.close().await.unwrap();
    });
}

#[test]
fn a_packed_page_read_carries_the_plain_page_read() {
    alone(async {
        let dir = tempfile::tempdir().unwrap();
        let source = b"%PDF-synthetic-two-page-packed-read".to_vec();
        let project = create_editable_document(
            CreateEditableDocument {
                path: dir.path().join("project").to_string_lossy().into(),
                project_id: id(1),
                document_id: id(2),
                device_id: device(),
                title: "two pages of notes".into(),
                source_blake3: vw_model::AssetId::hash(&source).to_string(),
                source,
                kind: EditableDocumentKind::Pdf,
                now_ms: TIME,
                pages: (0..2)
                    .map(|page| DocumentPageGeometry {
                        page,
                        width: 612.0,
                        height: 792.0,
                        crop: vec![0.0, 0.0, 612.0, 792.0],
                        quarter_turns: 0,
                        layer_id: id(3 + page as u8),
                    })
                    .collect(),
            },
            cancel(),
        )
        .await
        .unwrap();
        for number in 0..6 {
            handwriting(&project, number, 80, "pen", 6.0, id(3 + (number % 2) as u8)).await;
        }
        for page in 0..2 {
            let plain = project
                .editable_document_page(id(2), page, cancel())
                .await
                .unwrap();
            let packed = project
                .editable_document_page_packed(id(2), page, cancel())
                .await
                .unwrap();
            assert_eq!(plain.render.items.len(), 3);
            assert_same_document(&plain, &packed);
        }
        assert!(matches!(
            project
                .editable_document_page_packed(id(2), 2, cancel())
                .await,
            Err(CoreError::Invalid)
        ));
        project.close().await.unwrap();
    });
}

/// A stroke of 100 samples: the plain read counted about 168 KB for it and
/// refused a page from about 400 of them. The packed read counts what its
/// form allocates, admits that page and more than 2,000 such strokes.
#[test]
fn the_packed_read_admits_a_page_the_plain_read_refuses() {
    alone(async {
        let dir = tempfile::tempdir().unwrap();
        let project = create_image_project(create(&dir.path().join("project")), cancel())
            .await
            .unwrap();
        let mut count = 0;
        while count < 40 {
            handwriting(&project, count, 100, "pen", 6.0, id(3)).await;
            count += 1;
        }
        let costs = |items: Vec<(Option<Vec<u8>>, u64)>| -> (usize, usize) {
            let strokes = items.iter().filter(|(digest, _)| digest.is_some()).count();
            (strokes, items.iter().map(|(_, cost)| *cost as usize).sum())
        };
        let (strokes, plain_charge) = costs(
            project
                .document_snapshot_changes(id(2), vec![], cancel())
                .await
                .unwrap()
                .items
                .into_iter()
                .map(|c| (c.digest, c.cost))
                .collect(),
        );
        let (_, packed_charge) = costs(
            project
                .document_snapshot_changes_packed(id(2), vec![], cancel())
                .await
                .unwrap()
                .items
                .into_iter()
                .map(|c| (c.digest, c.cost))
                .collect(),
        );
        assert_eq!(strokes, 40);

        // Rust: building the answer and lowering it into the transfer buffer the
        // generated bindings read, exactly as the exported call does.
        let (plain_transfer, plain_peak) = peak_during(async {
            let snapshot = project.document_snapshot(id(2), cancel()).await.unwrap();
            let buffer =
                <DocumentSnapshot as uniffi::Lower<vw_core::UniFfiTag>>::lower_into_rust_buffer(
                    snapshot,
                );
            let size = (buffer.len(), buffer.capacity());
            buffer.destroy();
            size
        })
        .await;
        let mut contours = 0;
        let (packed_transfer, packed_peak) = peak_during(async {
            let snapshot = project
                .document_snapshot_packed(id(2), cancel())
                .await
                .unwrap();
            contours = snapshot
                .render
                .items
                .iter()
                .map(|item| item.contours.len())
                .sum::<usize>();
            let buffer =
            <PackedDocumentSnapshot as uniffi::Lower<vw_core::UniFfiTag>>::lower_into_rust_buffer(
                snapshot,
            );
            let size = (buffer.len(), buffer.capacity());
            buffer.destroy();
            size
        })
        .await;
        assert!(
            packed_peak <= packed_charge,
            "Rust held {packed_peak} bytes at once against a charge of {packed_charge}"
        );
        // Kotlin, per item: the generated record, its strings, rectangle,
        // transform, style and shape, then the shared facade's item, rectangle,
        // transform, style, shape and contours wrapper: under 1 KB together with
        // 16-byte HotSpot headers and compressed references, less on ART. The
        // outline is one ByteArray (16-byte header, 8-byte aligned) copied once
        // from the transfer buffer and kept by the facade. While the bindings read,
        // the transfer buffer is still held; it is released before the facade
        // converts the records.
        let arrays = contours + strokes * 24;
        let objects = strokes * 1024;
        let reading = packed_transfer.1 + arrays + objects;
        let converting = arrays + objects;
        assert!(
            reading.max(converting) <= packed_charge,
            "Kotlin may hold {reading} bytes against a charge of {packed_charge}"
        );
        println!(
            "40 strokes of 100 samples | per stroke: plain charge {}, packed charge {}; \
         Rust peak plain {} packed {}; transfer plain {:?} packed {:?}; outlines {} bytes",
            plain_charge / strokes,
            packed_charge / strokes,
            plain_peak / strokes,
            packed_peak / strokes,
            plain_transfer,
            packed_transfer,
            contours / strokes,
        );
        assert!(
            packed_charge / strokes * 2_000 < MAX_RESPONSE,
            "a page of 2,000 such strokes must be read whole"
        );

        // Fill the page past what the plain read admits.
        let refused = MAX_RESPONSE / (plain_charge / strokes) + 5;
        while (count as usize) < refused {
            handwriting(&project, count, 100, "pen", 6.0, id(3)).await;
            count += 1;
        }
        assert!(matches!(
            project.document_snapshot(id(2), cancel()).await,
            Err(CoreError::Backpressure)
        ));
        let packed = project
            .document_snapshot_packed(id(2), cancel())
            .await
            .unwrap();
        assert_eq!(packed.render.items.len(), count as usize);
        let changes = project
            .document_snapshot_changes_packed(id(2), vec![], cancel())
            .await
            .unwrap();
        assert_eq!(changes.items.len(), count as usize);
        project.close().await.unwrap();
    });
}
