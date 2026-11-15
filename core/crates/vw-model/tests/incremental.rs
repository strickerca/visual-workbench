//! The incremental state hash and the changed-objects validation must give
//! exactly the answers of the whole-project functions they stand in for.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use vw_model::{
    AssetId, DeviceId, Document, Group, IMAGE_OUTPUT_LAYER_KIND, Id, Instruction, Layer, Object,
    Project, StateHasher,
};
use vw_proto::v1 as pb;

const TIME: u64 = 1_700_000_000_000;

fn id(kind: u8, number: u32) -> Id {
    let mut entropy = [kind; 10];
    entropy[6..].copy_from_slice(&number.to_be_bytes());
    Id::from_parts(TIME, entropy).unwrap()
}
fn device() -> DeviceId {
    DeviceId::from_bytes([3; 16])
}
fn asset() -> pb::AddAsset {
    pb::AddAsset {
        asset_id: AssetId::hash(b"synthetic original").to_string(),
        format: "png".into(),
        width: 128,
        height: 96,
        orientation: 1,
        bit_depth: 8,
        has_alpha: true,
        color_space: "sRGB".into(),
        icc_profile: vec![0, 1, 2, 255],
        byte_size: 18,
        source: "import".into(),
        captured_at_ms: TIME as i64,
        metadata_json: r#"{"description":"Synthetic fixture"}"#.into(),
    }
}
fn layer(number: u32, kind: &str) -> (Id, Layer) {
    (
        id(3, number),
        Layer {
            definition: pb::CreateLayer {
                layer_id: Some(id(3, number).to_proto()),
                document_id: Some(id(2, 0).to_proto()),
                page_index: -1,
                name: format!("Layer {number}"),
                kind: kind.into(),
                order_key: "V".into(),
            },
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: "normal".into(),
        },
    )
}
fn project() -> Project {
    let mut project = Project::new(id(1, 0), "Incremental fixture".into(), device());
    let original = asset();
    project.assets.insert(
        AssetId::try_from(original.asset_id.clone()).unwrap(),
        original,
    );
    project.documents.insert(
        id(2, 0),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(2, 0).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Fixture image".into(),
                primary_asset_id: asset().asset_id,
                capture: None,
            },
            pages: Vec::new(),
            created_at_ms: TIME as i64,
        },
    );
    project
        .layers
        .extend([layer(0, "annotation"), layer(1, "annotation")]);
    project.groups.insert(
        id(7, 0),
        Group {
            id: id(7, 0),
            document_id: id(2, 0),
            parent: None,
        },
    );
    project.instructions.insert(
        id(8, 0),
        Instruction {
            definition: pb::SetInstruction {
                instruction_id: Some(id(8, 0).to_proto()),
                document_id: Some(id(2, 0).to_proto()),
                target_object_ids: vec![id(6, 1).to_proto()],
                role: pb::Role::Change as i32,
                text: "Make it \u{03a9}".into(),
                entry_method: "pc_keyboard".into(),
                language: "en".into(),
            },
            updated_at_ms: TIME as i64,
        },
    );
    project
}
fn shape(kind: u64, seed: f64) -> pb::object_state::Shape {
    use pb::object_state::Shape;
    let rect = pb::RectD {
        x: seed,
        y: -seed,
        w: 16.5,
        h: 8.25,
    };
    let line = |count: usize| pb::Polyline {
        points: (0..count)
            .map(|n| pb::PointD {
                x: seed + n as f64 * 1.25,
                y: n as f64 * -2.5,
            })
            .collect(),
        closed: count > 2,
    };
    match kind % 8 {
        0 => Shape::Stroke(pb::Stroke {
            x: vec![seed, seed + 0.25, 4.5],
            y: vec![-2.5, 5.0, 17.0],
            t_ms: vec![0, 8, 16],
            pressure: vec![0.0, 0.5, 1.0],
            tilt: vec![0.0, 0.25, 1.0],
            orientation: vec![-1.0, 0.0, 1.0],
            brush: Some(pb::Brush {
                family: "pen".into(),
                algorithm_version: 1,
                base_width: 2.0,
                pressure_curve: vec![0.0, 0.0, 0.25, 0.25, 0.75, 0.75, 1.0, 1.0],
                stabilization: 0.5,
            }),
        }),
        1 => Shape::Line(line(2)),
        2 => Shape::Arrow(line(3)),
        3 => Shape::Rect(rect),
        4 => Shape::Ellipse(rect),
        5 => Shape::Polygon(line(4)),
        6 => Shape::Text(pb::TextObject {
            text: format!("Synthetic \"annotation\" \u{03a9} {seed}"),
            font_family: "Noto Sans".into(),
            font_size: 14.0,
            anchor: Some(pb::PointD { x: 5.0, y: seed }),
        }),
        _ => Shape::Marker(pb::Marker {
            number: 1,
            point: Some(pb::PointD { x: 1.5, y: seed }),
            r#box: Some(rect),
            element_eids: vec!["synthetic-element".into()],
        }),
    }
}
fn object(number: u32, kind: u64, seed: f64) -> (Id, Object) {
    (
        id(6, number),
        Object {
            document_id: id(2, 0),
            state: pb::ObjectState {
                object_id: Some(id(6, number).to_proto()),
                layer_id: Some(id(3, 0).to_proto()),
                order_key: "V".into(),
                transform: Some(pb::Affine {
                    a: 1.0,
                    d: 1.0,
                    // Signed zero must hash like zero.
                    e: if number.is_multiple_of(3) { -0.0 } else { seed },
                    ..Default::default()
                }),
                style: Some(pb::Style {
                    stroke: Some(pb::Color { rgba: 0x102030ff }),
                    width: 2.0,
                    fill: Some(pb::Color { rgba: 0x8090a080 }),
                    has_fill: true,
                    screen_constant_width: false,
                }),
                role: pb::Role::None as i32,
                created_by: device().to_string(),
                created_at_ms: TIME as i64,
                shape: Some(shape(kind, seed)),
                ..Default::default()
            },
        },
    )
}

struct Random(u64);
impl Random {
    fn below(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % bound
    }
}

fn assert_same(hasher: &mut StateHasher, project: &Project, what: &str) {
    let (hash, length) = hasher.hash(project).unwrap();
    assert_eq!(hash, project.state_hash().unwrap(), "hash after {what}");
    assert_eq!(
        length,
        project.canonical_bytes().unwrap().len(),
        "length after {what}"
    );
    assert_eq!(
        hasher.json_len(),
        Some(
            serde_json::to_vec(&serde_json::to_value(project).unwrap())
                .unwrap()
                .len()
        ),
        "plain JSON length after {what}"
    );
}

#[test]
fn a_fresh_hasher_matches_the_whole_project_hash() {
    let mut value = project();
    assert_same(&mut StateHasher::default(), &value, "no objects");
    for number in 0..200 {
        let (key, entry) = object(number, u64::from(number), f64::from(number) * 0.5);
        value.objects.insert(key, entry);
        if number < 10 || number % 63 < 3 {
            assert_same(&mut StateHasher::default(), &value, "a build from nothing");
        }
    }
    // A capture document, PDF pages and mask history change the other members.
    value.mask_versions.insert(
        id(6, 4),
        vec![pb::MaskOp {
            op: "invert".into(),
            input_object_ids: vec![],
            output_object_id: Some(id(6, 4).to_proto()),
            amount: -0.0,
        }],
    );
    value.layers.get_mut(&id(3, 1)).unwrap().opacity = 0.0;
    assert_same(&mut StateHasher::default(), &value, "other members");
}

#[test]
fn appends_edits_and_removals_keep_the_incremental_hash_exact() {
    for seed in 1..=3_u64 {
        let mut random = Random(seed);
        let mut value = project();
        let mut hasher = StateHasher::default();
        let mut next = 1000_u32;
        assert_same(&mut hasher, &value, "start");
        for step in 0..500_u32 {
            let existing = value.objects.keys().cloned().collect::<Vec<_>>();
            let pick = |random: &mut Random| {
                existing[random.below(existing.len() as u64) as usize].clone()
            };
            let what = match random.below(if existing.is_empty() { 1 } else { 12 }) {
                // Mostly appends, as drawing does.
                0..=5 => {
                    next += 1;
                    let (key, entry) = object(next, random.below(8), f64::from(step));
                    hasher.objects_changed_from(&key);
                    value.objects.insert(key, entry);
                    "append"
                }
                6 => {
                    // An ID before every appended one, between older objects.
                    let (key, entry) = object(random.below(1000) as u32, 3, f64::from(step));
                    hasher.objects_changed_from(&key);
                    value.objects.insert(key, entry);
                    "insert before"
                }
                7 | 8 => {
                    let key = pick(&mut random);
                    hasher.objects_changed_from(&key);
                    let state = &mut value.objects.get_mut(&key).unwrap().state;
                    state.transform.as_mut().unwrap().f = f64::from(step) * -0.25;
                    state.order_key = "k".into();
                    "edit"
                }
                9 => {
                    let key = pick(&mut random);
                    hasher.objects_changed_from(&key);
                    value.objects.remove(&key);
                    "remove"
                }
                10 => {
                    // Several objects at once: report the smallest ID.
                    let mut keys = [pick(&mut random), pick(&mut random), pick(&mut random)];
                    keys.sort();
                    hasher.objects_changed_from(&keys[0]);
                    for key in keys {
                        if let Some(entry) = value.objects.get_mut(&key) {
                            entry.state.hidden = !entry.state.hidden;
                        }
                    }
                    "edit several"
                }
                _ => {
                    hasher.reset();
                    value.layers.get_mut(&id(3, 1)).unwrap().opacity =
                        random.below(100) as f64 / 100.0;
                    value.title = format!("Renamed {step}");
                    "other member"
                }
            };
            if step % 5 == 0 || step > 470 {
                assert_same(&mut hasher, &value, what);
            } else {
                // Unchecked steps still have to leave a usable resume state.
                hasher.hash(&value).unwrap();
            }
        }
        assert!(value.objects.len() > 200);
        assert_same(&mut hasher, &value, "the end");
        // Rolling back to a shorter project invalidates what came after.
        let last = value.objects.keys().next_back().unwrap().clone();
        hasher.objects_changed_from(&last);
        value.objects.remove(&last);
        assert_same(&mut hasher, &value, "a removed tail");
    }
}

fn outcome(result: Result<(), vw_model::ModelError>) -> String {
    match result {
        Ok(()) => "valid".into(),
        Err(error) => format!("{error:?}"),
    }
}

#[test]
fn changed_object_validation_agrees_with_whole_project_validation() {
    let mut base = project();
    let (output_layer, output) = layer(2, IMAGE_OUTPUT_LAYER_KIND);
    base.layers.insert(output_layer.clone(), output);
    for number in 0..40 {
        let (key, entry) = object(number, u64::from(number), f64::from(number));
        base.objects.insert(key, entry);
    }
    // The output layer's one crop object.
    let (frame, mut crop) = object(900, 3, 0.0);
    crop.state.layer_id = Some(output_layer.to_proto());
    crop.state.shape = Some(pb::object_state::Shape::Crop(pb::RectD {
        x: 0.0,
        y: 0.0,
        w: 64.0,
        h: 48.0,
    }));
    crop.state.transform = Some(pb::Affine {
        a: 1.0,
        d: 1.0,
        ..Default::default()
    });
    base.objects.insert(frame.clone(), crop);
    base.validate().unwrap();

    let mut random = Random(77);
    let mut refused = std::collections::BTreeSet::new();
    for round in 0..6000_u32 {
        let mut after = base.clone();
        let mut changed = std::collections::BTreeMap::<Id, Option<Object>>::new();
        for _ in 0..1 + random.below(3) {
            let existing = after.objects.keys().cloned().collect::<Vec<_>>();
            let key = match random.below(6) {
                0 => id(6, 2000 + round),
                1 => id(7, 0), // collides with the group
                2 => frame.clone(),
                _ => existing[random.below(existing.len() as u64) as usize].clone(),
            };
            changed
                .entry(key.clone())
                .or_insert_with(|| base.objects.get(&key).cloned());
            if random.below(8) == 0 {
                after.objects.remove(&key);
                continue;
            }
            let entry = after.objects.entry(key.clone()).or_insert_with(|| {
                let mut entry = object(1, random.below(8), 1.0).1;
                entry.state.object_id = Some(key.to_proto());
                entry
            });
            let state = &mut entry.state;
            match random.below(14) {
                0 => state.layer_id = Some(id(3, 99).to_proto()),
                1 => state.layer_id = Some(output_layer.to_proto()),
                2 => state.group_id = Some(id(7, 0).to_proto()),
                3 => state.group_id = Some(id(7, 9).to_proto()),
                4 => state.transform.as_mut().unwrap().a = f64::NAN,
                5 => state.transform.as_mut().unwrap().a = 0.0,
                6 => state.order_key = "V0".into(),
                7 => entry.document_id = id(2, 9),
                8 => state.object_id = Some(id(6, 1).to_proto()),
                9 => {
                    state.shape = Some(pb::object_state::Shape::SelectionRaster(
                        pb::RasterMaskRef {
                            mask_asset_id: AssetId::hash(b"absent").to_string(),
                            bounds: Some(pb::RectD {
                                x: 0.0,
                                y: 0.0,
                                w: 1.0,
                                h: 1.0,
                            }),
                            feather: 0.0,
                        },
                    ))
                }
                10 => state.shape = Some(pb::object_state::Shape::ResultId(id(8, 5).to_proto())),
                11 => state.hidden = true,
                12 => state.role = 9,
                _ => state.transform.as_mut().unwrap().e += 1.0,
            }
        }
        let whole = outcome(after.validate());
        let fast =
            outcome(after.validate_changed_objects(
                changed.iter().map(|(key, before)| (key, before.as_ref())),
            ));
        assert_eq!(fast, whole, "round {round}");
        refused.insert(whole);
    }
    // The cases above reach the reference, identity, output-layer and state
    // rules, not only the valid outcome.
    assert!(refused.len() >= 10, "{refused:?}");
}

/// A long text object: a few of them fill a page of the hasher's byte cache.
fn long_text(number: u32, length: usize) -> (Id, Object) {
    let (key, mut entry) = object(number, 6, f64::from(number));
    entry.state.shape = Some(pb::object_state::Shape::Text(pb::TextObject {
        text: "long \u{03a9} ".repeat(length / 8),
        font_family: "Noto Sans".into(),
        font_size: 14.0,
        anchor: Some(pb::PointD { x: 5.0, y: 1.0 }),
    }));
    (key, entry)
}

/// Every way an edit is reported one entity at a time: a single object, a
/// member written before the objects, a member written after them, an edit
/// that is taken back, and all of them across the pages the bytes are kept
/// in, including pages that fill up, split and empty out.
#[test]
fn single_object_and_member_reports_keep_the_incremental_hash_exact() {
    for seed in 1..=4_u64 {
        let mut random = Random(seed);
        let mut value = project();
        let mut hasher = StateHasher::default();
        let mut next = 5000_u32;
        assert_same(&mut hasher, &value, "start");
        for step in 0..700_u32 {
            let existing = value.objects.keys().cloned().collect::<Vec<_>>();
            let pick = |random: &mut Random| {
                existing[random.below(existing.len() as u64) as usize].clone()
            };
            let what = match random.below(if existing.is_empty() { 3 } else { 24 }) {
                0..=4 => {
                    next += 1;
                    let (key, entry) = if random.below(3) == 0 {
                        long_text(next, 2_000 + random.below(40_000) as usize)
                    } else {
                        object(next, random.below(8), f64::from(step))
                    };
                    hasher.objects_changed([&key]);
                    value.objects.insert(key, entry);
                    "append"
                }
                5 | 6 => {
                    // Somewhere before the appended ones, now and then large
                    // enough that the page it lands in has to be cut in two.
                    let number = random.below(5000) as u32;
                    let (key, entry) = if random.below(2) == 0 {
                        long_text(number, 20_000 + random.below(60_000) as usize)
                    } else {
                        object(number, 3, f64::from(step))
                    };
                    hasher.objects_changed([&key]);
                    value.objects.insert(key, entry);
                    "insert before"
                }
                7..=9 => {
                    let key = pick(&mut random);
                    hasher.objects_changed([&key]);
                    let state = &mut value.objects.get_mut(&key).unwrap().state;
                    state.transform.as_mut().unwrap().f = f64::from(step) * -0.25;
                    state.order_key = "k".into();
                    "edit"
                }
                10 | 11 => {
                    let key = pick(&mut random);
                    hasher.objects_changed([&key]);
                    value.objects.remove(&key);
                    "remove"
                }
                12 => {
                    // A run of neighbours goes, which empties whole pages.
                    let from = pick(&mut random);
                    let run = value
                        .objects
                        .range(from..)
                        .take(1 + random.below(60) as usize)
                        .map(|(key, _)| key.clone())
                        .collect::<Vec<_>>();
                    hasher.objects_changed(&run);
                    for key in run {
                        value.objects.remove(&key);
                    }
                    "remove a run"
                }
                13 => {
                    let keys = [pick(&mut random), pick(&mut random), pick(&mut random)];
                    hasher.objects_changed(&keys);
                    for key in keys {
                        if let Some(entry) = value.objects.get_mut(&key) {
                            entry.state.hidden = !entry.state.hidden;
                        }
                    }
                    "edit several"
                }
                14 => {
                    hasher.member_changed("layers");
                    let layer = value.layers.get_mut(&id(3, 1)).unwrap();
                    layer.opacity = random.below(100) as f64 / 100.0;
                    layer.definition.name = "Layer ".repeat(random.below(40) as usize);
                    "layers"
                }
                15 => {
                    hasher.member_changed("instructions");
                    value
                        .instructions
                        .get_mut(&id(8, 0))
                        .unwrap()
                        .definition
                        .text = format!("Changed at step {step}");
                    "instructions"
                }
                16 => {
                    hasher.member_changed("mask_versions");
                    value
                        .mask_versions
                        .entry(id(6, 4))
                        .or_default()
                        .push(pb::MaskOp {
                            op: "invert".into(),
                            input_object_ids: vec![],
                            output_object_id: Some(id(6, 4).to_proto()),
                            amount: -0.0,
                        });
                    "mask history"
                }
                17 => {
                    hasher.member_changed("documents");
                    value.documents.get_mut(&id(2, 0)).unwrap().definition.title =
                        format!("Document {step}");
                    "documents"
                }
                18 => {
                    // Written behind the objects: only the end is hashed again.
                    hasher.member_changed("results");
                    value.results.insert(
                        id(9, step),
                        vw_model::ResultCandidate {
                            definition: pb::AddResult {
                                result_id: Some(id(9, step).to_proto()),
                                document_id: Some(id(2, 0).to_proto()),
                                provider: "synthetic".into(),
                                model: "synthetic".into(),
                                request_json: "{}".into(),
                                ..Default::default()
                            },
                            status: "pending".into(),
                            acceptance_mask_asset_id: None,
                            created_at_ms: TIME as i64,
                        },
                    );
                    "results"
                }
                19 => {
                    hasher.member_changed("groups");
                    value.groups.insert(
                        id(7, 1 + step),
                        Group {
                            id: id(7, 1 + step),
                            document_id: id(2, 0),
                            parent: Some(id(7, 0)),
                        },
                    );
                    "groups"
                }
                20 => {
                    // A member and objects in one edit, as a layer deletion is.
                    hasher.member_changed("layers");
                    value.layers.get_mut(&id(3, 0)).unwrap().visible = step % 2 == 0;
                    let keys = [pick(&mut random), pick(&mut random)];
                    hasher.objects_changed(&keys);
                    for key in keys {
                        value.objects.remove(&key);
                    }
                    "layer and objects"
                }
                21 => {
                    // An edit that is taken back before anything is hashed.
                    let key = pick(&mut random);
                    hasher.objects_changed([&key]);
                    hasher.member_changed("layers");
                    let kept = value.objects.remove(&key).unwrap();
                    let opacity = value.layers[&id(3, 1)].opacity;
                    value.layers.get_mut(&id(3, 1)).unwrap().opacity = 0.125;
                    hasher.objects_changed([&key]);
                    hasher.member_changed("layers");
                    value.objects.insert(key, kept);
                    value.layers.get_mut(&id(3, 1)).unwrap().opacity = opacity;
                    "taken back"
                }
                22 => {
                    // The older report, everything from one ID on, mixed in.
                    let key = pick(&mut random);
                    hasher.objects_changed_from(&key);
                    let later = value
                        .objects
                        .range(key..)
                        .take(4)
                        .map(|(key, _)| key.clone())
                        .collect::<Vec<_>>();
                    for key in later {
                        value.objects.get_mut(&key).unwrap().state.locked = step % 2 == 0;
                    }
                    "from an ID on"
                }
                _ => {
                    hasher.reset();
                    value.title = format!("Renamed {step}");
                    "another project"
                }
            };
            if step % 3 == 0 || step > 660 {
                assert_same(&mut hasher, &value, what);
            } else {
                hasher.hash(&value).unwrap();
            }
        }
        assert_same(&mut hasher, &value, "the end");
        // A copy of the hasher follows its own copy of the project.
        let mut copy = hasher.clone();
        let mut other = value.clone();
        let key = other.objects.keys().next().unwrap().clone();
        copy.objects_changed([&key]);
        other.objects.remove(&key);
        assert_same(&mut copy, &other, "a copy");
        assert_same(&mut hasher, &value, "the original");
        // Down to nothing and up again.
        let all = value.objects.keys().cloned().collect::<Vec<_>>();
        hasher.objects_changed(&all);
        value.objects.clear();
        assert_same(&mut hasher, &value, "no objects left");
        let (key, entry) = object(1, 0, 1.0);
        hasher.objects_changed([&key]);
        value.objects.insert(key, entry);
        assert_same(&mut hasher, &value, "one object again");
    }
}

/// What an edit replaced, recorded the way an edit in place records it.
macro_rules! touch {
    ($replaced:expr, $base:expr, $field:ident, $key:expr) => {{
        let key = $key.clone();
        $replaced
            .$field
            .entry(key.clone())
            .or_insert_with(|| $base.$field.get(&key).cloned());
    }};
}

fn capture() -> pb::CaptureInfo {
    pb::CaptureInfo {
        capture_session_id: Some(id(9, 9).to_proto()),
        frame_id: 7,
        geometry: Some(pb::CaptureGeometry {
            source_kind: "window".into(),
            window_handle: 0x1234,
            monitor_id: "synthetic-monitor".into(),
            client_rect_host: Some(pb::RectI {
                x: 0,
                y: 0,
                w: 128,
                h: 96,
            }),
            dpi_scale: 1.0,
            geometry_revision: 1,
            timestamp_ns: 17_000_000_000,
        }),
        platform: "windows".into(),
        app_name: "SyntheticApp".into(),
        window_title: "Synthetic local capture".into(),
        lossless: true,
        degraded: false,
        captured_at_ms: TIME as i64,
        stream_save: None,
    }
}

/// A project in which every kind of entity refers to others: two documents
/// (one a capture with a semantic snapshot), layers on both, nested groups,
/// a mask object with its asset, a result with the object that shows it, an
/// output frame, instructions and mask history.
fn linked_project() -> (Project, AssetId) {
    let mut value = project();
    let mask = pb::AddAsset {
        asset_id: AssetId::hash(b"synthetic mask").to_string(),
        ..asset()
    };
    let mask_id = AssetId::try_from(mask.asset_id.clone()).unwrap();
    value.assets.insert(mask_id.clone(), mask);
    value.documents.insert(
        id(2, 1),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(2, 1).to_proto()),
                kind: pb::DocumentKind::Capture as i32,
                schema_version: 1,
                title: "Fixture capture".into(),
                primary_asset_id: asset().asset_id,
                capture: Some(capture()),
            },
            pages: Vec::new(),
            created_at_ms: TIME as i64,
        },
    );
    let (other_layer, mut other) = layer(5, "annotation");
    other.definition.document_id = Some(id(2, 1).to_proto());
    value.layers.insert(other_layer, other);
    let (mask_layer, mask_definition) = layer(6, "mask");
    value.layers.insert(mask_layer.clone(), mask_definition);
    let (output_layer, output) = layer(2, IMAGE_OUTPUT_LAYER_KIND);
    value.layers.insert(output_layer.clone(), output);
    value.groups.insert(
        id(7, 1),
        Group {
            id: id(7, 1),
            document_id: id(2, 0),
            parent: Some(id(7, 0)),
        },
    );
    value.semantic_snapshots.insert(
        id(10, 0),
        vw_model::SemanticSnapshot {
            definition: pb::AddSemanticSnapshot {
                snapshot_id: Some(id(10, 0).to_proto()),
                document_id: Some(id(2, 1).to_proto()),
                platform: "uia".into(),
                frame_delta_ms: 0,
                elements_json_zstd: vec![1, 2, 3],
            },
            capture_session_id: Some(id(9, 9)),
            frame_id: Some(7),
            created_at_ms: TIME as i64,
        },
    );
    value.results.insert(
        id(11, 0),
        vw_model::ResultCandidate {
            definition: pb::AddResult {
                result_id: Some(id(11, 0).to_proto()),
                document_id: Some(id(2, 0).to_proto()),
                provider: "synthetic".into(),
                model: "synthetic".into(),
                request_json: "{}".into(),
                output_asset_id: asset().asset_id,
                ..Default::default()
            },
            status: "pending".into(),
            acceptance_mask_asset_id: Some(mask_id.clone()),
            created_at_ms: TIME as i64,
        },
    );
    for number in 0..24 {
        let (key, mut entry) = object(number, u64::from(number), f64::from(number));
        match number % 6 {
            1 => entry.state.group_id = Some(id(7, 0).to_proto()),
            2 => entry.state.group_id = Some(id(7, 1).to_proto()),
            3 => entry.state.layer_id = Some(id(3, 1).to_proto()),
            _ => {}
        }
        value.objects.insert(key, entry);
    }
    let (key, mut entry) = object(100, 3, 0.0);
    entry.state.layer_id = Some(mask_layer.to_proto());
    entry.state.shape = Some(pb::object_state::Shape::SelectionRaster(
        pb::RasterMaskRef {
            mask_asset_id: mask_id.to_string(),
            bounds: Some(pb::RectD {
                x: 0.0,
                y: 0.0,
                w: 8.0,
                h: 8.0,
            }),
            feather: 0.0,
        },
    ));
    value.objects.insert(key.clone(), entry);
    value.mask_versions.insert(
        key,
        vec![pb::MaskOp {
            op: "add".into(),
            input_object_ids: vec![],
            output_object_id: Some(id(6, 100).to_proto()),
            amount: 0.0,
        }],
    );
    let (key, mut entry) = object(101, 3, 0.0);
    entry.state.shape = Some(pb::object_state::Shape::ResultId(id(11, 0).to_proto()));
    value.objects.insert(key, entry);
    let (key, mut entry) = object(102, 3, 0.0);
    entry.document_id = id(2, 1);
    entry.state.layer_id = Some(id(3, 5).to_proto());
    value.objects.insert(key, entry);
    let (key, mut crop) = object(900, 3, 0.0);
    crop.state.layer_id = Some(output_layer.to_proto());
    crop.state.shape = Some(pb::object_state::Shape::Crop(pb::RectD {
        x: 0.0,
        y: 0.0,
        w: 64.0,
        h: 48.0,
    }));
    crop.state.transform = Some(pb::Affine {
        a: 1.0,
        d: 1.0,
        ..Default::default()
    });
    value.objects.insert(key, crop);
    value.validate().unwrap();
    (value, mask_id)
}

/// An edit to anything in the project, valid or not: what the check of the
/// replaced entities says is what the check of the whole project says, the
/// same refusal included.
#[test]
fn validation_of_what_an_edit_replaced_agrees_with_whole_project_validation() {
    let (base, mask) = linked_project();
    let original = AssetId::try_from(asset().asset_id).unwrap();
    let mut random = Random(4242);
    let mut outcomes = std::collections::BTreeMap::<String, u32>::new();
    for round in 0..40_000_u32 {
        let mut after = base.clone();
        let mut replaced = vw_model::Replaced::default();
        for _ in 0..1 + random.below(3) {
            let objects = after.objects.keys().cloned().collect::<Vec<_>>();
            let any_object =
                |random: &mut Random| objects[random.below(objects.len() as u64) as usize].clone();
            let layers = after.layers.keys().cloned().collect::<Vec<_>>();
            let groups = after.groups.keys().cloned().collect::<Vec<_>>();
            let documents = after.documents.keys().cloned().collect::<Vec<_>>();
            match random.below(46) {
                // Objects: what the earlier check covered, through this one.
                0..=5 => {
                    if objects.is_empty() {
                        continue;
                    }
                    let key = any_object(&mut random);
                    touch!(replaced, base, objects, key);
                    let entry = after.objects.get_mut(&key).unwrap();
                    let state = &mut entry.state;
                    match random.below(12) {
                        0 => state.layer_id = Some(id(3, 99).to_proto()),
                        1 => state.layer_id = Some(id(3, 2).to_proto()),
                        2 => state.group_id = Some(id(7, 1).to_proto()),
                        3 => state.group_id = Some(id(7, 9).to_proto()),
                        4 => state.transform.as_mut().unwrap().a = f64::NAN,
                        5 => entry.document_id = id(2, 1),
                        6 => state.layer_id = Some(id(3, 5).to_proto()),
                        7 => {
                            state.shape =
                                Some(pb::object_state::Shape::ResultId(id(11, 5).to_proto()))
                        }
                        8 => state.hidden = true,
                        9 => state.role = 9,
                        10 => state.layer_id = Some(id(3, 6).to_proto()),
                        _ => state.transform.as_mut().unwrap().e += 1.0,
                    }
                }
                6 | 7 => {
                    if objects.is_empty() {
                        continue;
                    }
                    let key = any_object(&mut random);
                    touch!(replaced, base, objects, key);
                    after.objects.remove(&key);
                }
                8 => {
                    // A new object, now and then under a group's ID.
                    let key = if random.below(3) == 0 {
                        id(7, random.below(2) as u32)
                    } else {
                        id(6, 3000 + round)
                    };
                    touch!(replaced, base, objects, key);
                    let mut entry = object(1, random.below(8), 1.0).1;
                    entry.state.object_id = Some(key.to_proto());
                    after.objects.insert(key, entry);
                }
                // Layers.
                9..=13 => {
                    if layers.is_empty() {
                        continue;
                    }
                    let key = layers[random.below(layers.len() as u64) as usize].clone();
                    touch!(replaced, base, layers, key);
                    let layer = after.layers.get_mut(&key).unwrap();
                    match random.below(10) {
                        0 => layer.definition.name = "Renamed".into(),
                        1 => layer.opacity = 0.5,
                        2 => layer.opacity = 1.5,
                        3 => layer.visible = !layer.visible,
                        4 => layer.definition.order_key = "k".into(),
                        5 => layer.definition.order_key = "k0".into(),
                        6 => layer.definition.document_id = Some(id(2, 1).to_proto()),
                        7 => layer.definition.kind = IMAGE_OUTPUT_LAYER_KIND.into(),
                        8 => layer.definition.page_index = 0,
                        _ => layer.blend = "multiply".into(),
                    }
                }
                14 | 15 => {
                    if layers.is_empty() {
                        continue;
                    }
                    let key = layers[random.below(layers.len() as u64) as usize].clone();
                    touch!(replaced, base, layers, key);
                    after.layers.remove(&key);
                    // A layer deletion takes its objects along, most times.
                    if random.below(3) != 0 {
                        let wire = Some(key.to_proto());
                        let members = after
                            .objects
                            .iter()
                            .filter(|(_, object)| object.state.layer_id == wire)
                            .map(|(key, _)| key.clone())
                            .collect::<Vec<_>>();
                        for member in members {
                            touch!(replaced, base, objects, member);
                            after.objects.remove(&member);
                        }
                    }
                }
                16 => {
                    let key = id(3, 40 + random.below(3) as u32);
                    touch!(replaced, base, layers, key);
                    let (_, mut new) = layer(
                        40,
                        if random.below(4) == 0 {
                            "output"
                        } else {
                            "mask"
                        },
                    );
                    new.definition.layer_id = Some(key.to_proto());
                    if random.below(4) == 0 {
                        new.definition.document_id = Some(id(2, 7).to_proto());
                    }
                    after.layers.insert(key, new);
                }
                // Groups.
                17..=19 => {
                    if groups.is_empty() {
                        continue;
                    }
                    let key = groups[random.below(groups.len() as u64) as usize].clone();
                    touch!(replaced, base, groups, key);
                    match random.below(5) {
                        0 => {
                            after.groups.remove(&key);
                        }
                        1 => after.groups.get_mut(&key).unwrap().parent = Some(id(7, 1)),
                        2 => after.groups.get_mut(&key).unwrap().document_id = id(2, 1),
                        3 => after.groups.get_mut(&key).unwrap().parent = Some(id(7, 9)),
                        _ => after.groups.get_mut(&key).unwrap().parent = None,
                    }
                }
                20 => {
                    // A new group, now and then under an object's ID.
                    let key = if random.below(3) == 0 {
                        id(6, random.below(24) as u32)
                    } else {
                        id(7, 50 + random.below(3) as u32)
                    };
                    touch!(replaced, base, groups, key);
                    after.groups.insert(
                        key.clone(),
                        Group {
                            id: key,
                            document_id: id(2, random.below(2) as u32),
                            parent: (random.below(2) == 0).then(|| id(7, 0)),
                        },
                    );
                }
                // Documents.
                21..=24 => {
                    if documents.is_empty() {
                        continue;
                    }
                    let key = documents[random.below(documents.len() as u64) as usize].clone();
                    touch!(replaced, base, documents, key);
                    let document = after.documents.get_mut(&key).unwrap();
                    match random.below(7) {
                        0 => document.definition.title = "Retitled".into(),
                        1 => document.definition.kind = pb::DocumentKind::Pdf as i32,
                        2 => document.definition.capture = None,
                        3 => document.definition.capture = Some(capture()),
                        4 => document.definition.primary_asset_id = mask.to_string(),
                        5 => {
                            document.definition.primary_asset_id =
                                AssetId::hash(b"absent").to_string()
                        }
                        _ => {
                            if let Some(capture) = &mut document.definition.capture {
                                capture.frame_id += 1;
                            }
                        }
                    }
                }
                25 => {
                    if documents.is_empty() {
                        continue;
                    }
                    let key = documents[random.below(documents.len() as u64) as usize].clone();
                    touch!(replaced, base, documents, key);
                    after.documents.remove(&key);
                }
                // Assets.
                26 | 27 => {
                    let key = if random.below(2) == 0 {
                        mask.clone()
                    } else {
                        original.clone()
                    };
                    touch!(replaced, base, assets, key);
                    after.assets.remove(&key);
                }
                28 => {
                    let new = pb::AddAsset {
                        asset_id: AssetId::hash(format!("new {round}").as_bytes()).to_string(),
                        width: random.below(2) as u32 * 16,
                        ..asset()
                    };
                    let key = AssetId::try_from(new.asset_id.clone()).unwrap();
                    touch!(replaced, base, assets, key);
                    after.assets.insert(key, new);
                }
                29 => {
                    // The source of the output frame turned on its side.
                    touch!(replaced, base, assets, original);
                    if let Some(asset) = after.assets.get_mut(&original) {
                        asset.orientation = 6;
                    }
                }
                // Instructions.
                30 | 31 => {
                    touch!(replaced, base, instructions, id(8, 0));
                    match random.below(4) {
                        0 => {
                            after.instructions.remove(&id(8, 0));
                        }
                        1 => {
                            if let Some(instruction) = after.instructions.get_mut(&id(8, 0)) {
                                instruction.definition.document_id = Some(id(2, 7).to_proto());
                            }
                        }
                        2 => {
                            if let Some(instruction) = after.instructions.get_mut(&id(8, 0)) {
                                instruction.definition.role = 12;
                            }
                        }
                        _ => {
                            if let Some(instruction) = after.instructions.get_mut(&id(8, 0)) {
                                instruction.definition.text = "Changed".into();
                            }
                        }
                    }
                }
                // Semantic snapshots.
                32 | 33 => {
                    touch!(replaced, base, semantic_snapshots, id(10, 0));
                    if let Some(snapshot) = after.semantic_snapshots.get_mut(&id(10, 0)) {
                        match random.below(4) {
                            0 => snapshot.frame_id = Some(8),
                            1 => snapshot.definition.document_id = Some(id(2, 0).to_proto()),
                            2 => snapshot.definition.platform = "other".into(),
                            _ => snapshot.definition.frame_delta_ms = 5,
                        }
                    }
                }
                // Results.
                34..=37 => {
                    touch!(replaced, base, results, id(11, 0));
                    match random.below(7) {
                        0 => {
                            after.results.remove(&id(11, 0));
                        }
                        1 => {
                            if let Some(result) = after.results.get_mut(&id(11, 0)) {
                                result.definition.document_id = Some(id(2, 1).to_proto());
                            }
                        }
                        2 => {
                            if let Some(result) = after.results.get_mut(&id(11, 0)) {
                                result.status = "ready".into();
                            }
                        }
                        3 => {
                            if let Some(result) = after.results.get_mut(&id(11, 0)) {
                                result.status = "rejected".into();
                            }
                        }
                        4 => {
                            if let Some(result) = after.results.get_mut(&id(11, 0)) {
                                result.acceptance_mask_asset_id = None;
                            }
                        }
                        5 => {
                            if let Some(result) = after.results.get_mut(&id(11, 0)) {
                                result.definition.composite_asset_id =
                                    AssetId::hash(b"absent").to_string();
                            }
                        }
                        _ => {
                            if let Some(result) = after.results.get_mut(&id(11, 0)) {
                                result.definition.provider.clear();
                            }
                        }
                    }
                }
                // Mask history.
                38 | 39 => {
                    touch!(replaced, base, mask_versions, id(6, 100));
                    match random.below(4) {
                        0 => {
                            after.mask_versions.remove(&id(6, 100));
                        }
                        1 => {
                            if let Some(versions) = after.mask_versions.get_mut(&id(6, 100)) {
                                versions[0].op = "blur".into();
                            }
                        }
                        2 => {
                            if let Some(versions) = after.mask_versions.get_mut(&id(6, 100)) {
                                versions[0].output_object_id = Some(id(6, 1).to_proto());
                            }
                        }
                        _ => {
                            if let Some(versions) = after.mask_versions.get_mut(&id(6, 100)) {
                                versions[0].amount = 2.0;
                            }
                        }
                    }
                }
                // The output frame and its layer.
                40 | 41 => {
                    touch!(replaced, base, objects, id(6, 900));
                    if let Some(frame) = after.objects.get_mut(&id(6, 900)) {
                        match random.below(4) {
                            0 => frame.state.hidden = true,
                            1 => frame.state.transform.as_mut().unwrap().e = 0.5,
                            2 => frame.state.layer_id = Some(id(3, 0).to_proto()),
                            _ => {
                                if let Some(pb::object_state::Shape::Crop(crop)) =
                                    &mut frame.state.shape
                                {
                                    crop.w = 32.0;
                                }
                            }
                        }
                    }
                }
                42 | 43 => {
                    touch!(replaced, base, layers, id(3, 2));
                    if let Some(layer) = after.layers.get_mut(&id(3, 2)) {
                        match random.below(4) {
                            0 => layer.opacity = 0.5,
                            1 => layer.blend = "multiply".into(),
                            2 => layer.visible = false,
                            _ => layer.definition.kind = "annotation".into(),
                        }
                    }
                }
                // A second object lands on the output layer.
                44 => {
                    if objects.is_empty() {
                        continue;
                    }
                    let key = any_object(&mut random);
                    touch!(replaced, base, objects, key);
                    after.objects.get_mut(&key).unwrap().state.layer_id = Some(id(3, 2).to_proto());
                }
                // Ungroup: the group goes and its members move up.
                _ => {
                    touch!(replaced, base, groups, id(7, 1));
                    after.groups.remove(&id(7, 1));
                    let wire = Some(id(7, 1).to_proto());
                    let members = after
                        .objects
                        .iter()
                        .filter(|(_, object)| object.state.group_id == wire)
                        .map(|(key, _)| key.clone())
                        .collect::<Vec<_>>();
                    for member in members {
                        touch!(replaced, base, objects, member);
                        after.objects.get_mut(&member).unwrap().state.group_id =
                            Some(id(7, 0).to_proto());
                    }
                }
            }
        }
        let whole = outcome(after.validate());
        let fast = outcome(after.validate_replaced(&replaced));
        assert_eq!(fast, whole, "round {round}: {replaced:?}");
        *outcomes.entry(whole).or_default() += 1;
    }
    // The edits above reach every reference rule, not only the valid outcome.
    assert!(outcomes.len() >= 30, "{outcomes:?}");
    assert!(outcomes["valid"] > 4_000, "{outcomes:?}");
}

/// Edits to single entities of every member other than the objects, reported
/// as an edit in place reports them: the hasher serializes only those
/// entities again, and the hash and lengths stay exact.
#[test]
fn entity_reports_in_every_member_keep_the_incremental_hash_exact() {
    let (mut value, mask) = linked_project();
    let mut hasher = StateHasher::default();
    assert_same(&mut hasher, &value, "start");
    let mut random = Random(99);
    for step in 0..600_u32 {
        let base = value.clone();
        let mut replaced = vw_model::Replaced::default();
        for _ in 0..1 + random.below(3) {
            match random.below(10) {
                0 => {
                    let key = id(8, random.below(4) as u32);
                    touch!(replaced, base, instructions, key);
                    if random.below(4) == 0 {
                        value.instructions.remove(&key);
                    } else {
                        value.instructions.insert(
                            key.clone(),
                            Instruction {
                                definition: pb::SetInstruction {
                                    instruction_id: Some(key.to_proto()),
                                    document_id: Some(id(2, 0).to_proto()),
                                    target_object_ids: vec![],
                                    role: pb::Role::Change as i32,
                                    text: "Long \u{03a9} ".repeat(random.below(3000) as usize),
                                    entry_method: "voice".into(),
                                    language: "en".into(),
                                },
                                updated_at_ms: i64::from(step),
                            },
                        );
                    }
                }
                1 => {
                    // Not the result an object shows.
                    let key = id(11, 1 + random.below(3) as u32);
                    touch!(replaced, base, results, key);
                    if random.below(4) == 0 {
                        value.results.remove(&key);
                    } else {
                        value.results.insert(
                            key.clone(),
                            vw_model::ResultCandidate {
                                definition: pb::AddResult {
                                    result_id: Some(key.to_proto()),
                                    document_id: Some(id(2, 0).to_proto()),
                                    provider: "synthetic".into(),
                                    model: "synthetic".into(),
                                    request_json: format!("{{\"n\":{step}}}"),
                                    cost_estimate_usd: if step % 2 == 0 { -0.0 } else { 0.25 },
                                    ..Default::default()
                                },
                                status: "pending".into(),
                                acceptance_mask_asset_id: None,
                                created_at_ms: 0,
                            },
                        );
                    }
                }
                2 => {
                    // The annotation layers; the output layer has rules.
                    let key = id(3, random.below(2) as u32);
                    touch!(replaced, base, layers, key);
                    if let Some(layer) = value.layers.get_mut(&key) {
                        layer.opacity = if step % 3 == 0 { 0.0 } else { 0.5 };
                        layer.definition.name = format!("Layer {step}");
                    }
                }
                3 => {
                    let key = id(7, 10 + random.below(3) as u32);
                    touch!(replaced, base, groups, key);
                    if value.groups.remove(&key).is_none() {
                        value.groups.insert(
                            key.clone(),
                            Group {
                                id: key,
                                document_id: id(2, 0),
                                parent: None,
                            },
                        );
                    }
                }
                4 => {
                    let key = id(6, 100);
                    touch!(replaced, base, mask_versions, key);
                    value
                        .mask_versions
                        .entry(key)
                        .or_default()
                        .push(pb::MaskOp {
                            op: "feather".into(),
                            input_object_ids: vec![],
                            output_object_id: Some(id(6, 100).to_proto()),
                            amount: if step % 2 == 0 { -0.0 } else { 1.5 },
                        });
                }
                5 => {
                    let key = id(2, random.below(2) as u32);
                    touch!(replaced, base, documents, key);
                    if let Some(document) = value.documents.get_mut(&key) {
                        document.definition.title = format!("Title {step}");
                    }
                }
                6 => {
                    let new = pb::AddAsset {
                        asset_id: AssetId::hash(format!("asset {}", random.below(5)).as_bytes())
                            .to_string(),
                        ..asset()
                    };
                    let key = AssetId::try_from(new.asset_id.clone()).unwrap();
                    touch!(replaced, base, assets, key);
                    if value.assets.remove(&key).is_none() {
                        value.assets.insert(key, new);
                    }
                }
                7 => {
                    touch!(replaced, base, semantic_snapshots, id(10, 0));
                    if let Some(snapshot) = value.semantic_snapshots.get_mut(&id(10, 0)) {
                        snapshot.definition.elements_json_zstd =
                            vec![step as u8; random.below(5000) as usize];
                    }
                }
                8 => {
                    // An object together with members, as most edits are.
                    let key = id(6, random.below(24) as u32);
                    touch!(replaced, base, objects, key);
                    if let Some(object) = value.objects.get_mut(&key) {
                        object.state.transform.as_mut().unwrap().e = f64::from(step);
                    }
                }
                _ => {
                    touch!(replaced, base, assets, mask);
                }
            }
        }
        hasher.replaced(&replaced);
        if step % 2 == 0 || step > 580 {
            assert_same(&mut hasher, &value, "an entity edit");
            // Only what changed was serialized again.
            let work = hasher.work();
            assert!(
                work.serialized < value.canonical_bytes().unwrap().len(),
                "step {step}: {work:?}"
            );
        } else {
            hasher.hash(&value).unwrap();
        }
    }
}
