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
