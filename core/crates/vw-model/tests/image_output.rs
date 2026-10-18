#![allow(clippy::unwrap_used, clippy::expect_used)]
use vw_geom::{ImageOutputFrame, QuarterTurn};
use vw_model::{AssetId, DeviceId, Document, Id, Layer, Object, Project};
use vw_proto::v1 as pb;

fn id(n: u8) -> Id {
    Id::from_parts(u64::from(n), [n; 10]).unwrap()
}
fn fixture(kind: &str) -> Project {
    let owner = DeviceId::from_bytes([4; 16]);
    let asset = AssetId::hash(b"original immutable bytes");
    let mut p = Project::new(id(1), "output".into(), owner.clone());
    p.assets.insert(
        asset.clone(),
        pb::AddAsset {
            asset_id: asset.to_string(),
            format: "png".into(),
            width: 40,
            height: 30,
            orientation: 6,
            bit_depth: 16,
            byte_size: 24,
            source: "import".into(),
            metadata_json: "{}".into(),
            ..Default::default()
        },
    );
    p.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(2).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "photo".into(),
                primary_asset_id: asset.to_string(),
                capture: None,
            },
            pages: vec![],
            created_at_ms: 1,
        },
    );
    p.layers.insert(
        id(3),
        Layer {
            definition: pb::CreateLayer {
                layer_id: Some(id(3).to_proto()),
                document_id: Some(id(2).to_proto()),
                name: "This name is deliberately not a selector".into(),
                kind: kind.into(),
                page_index: -1,
                order_key: "V".into(),
            },
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: "normal".into(),
        },
    );
    let f = ImageOutputFrame::new([30, 40], [2, 3, 20, 30], QuarterTurn::Clockwise90, [60, 40])
        .unwrap();
    let [a, b, c, d, e, f0] = f.matrix().unwrap().coefficients();
    p.objects.insert(
        id(4),
        Object {
            document_id: id(2),
            state: pb::ObjectState {
                object_id: Some(id(4).to_proto()),
                layer_id: Some(id(3).to_proto()),
                order_key: "V".into(),
                created_by: owner.to_string(),
                created_at_ms: 1,
                transform: Some(pb::Affine {
                    a,
                    b,
                    c,
                    d,
                    e,
                    f: f0,
                }),
                style: Some(pb::Style {
                    stroke: Some(pb::Color { rgba: 0 }),
                    width: 0.0,
                    ..Default::default()
                }),
                role: pb::Role::None as i32,
                shape: Some(pb::object_state::Shape::Crop(pb::RectD {
                    x: 2.0,
                    y: 3.0,
                    w: 20.0,
                    h: 30.0,
                })),
                ..Default::default()
            },
        },
    );
    p
}

#[test]
fn one_typed_frame_uses_oriented_source_and_is_independent_of_layer_title() {
    let p = fixture("output");
    p.validate().unwrap();
    let output = p.image_output(&id(2)).unwrap().unwrap();
    assert_eq!(output.frame.source_size(), [30, 40]);
    assert_eq!(output.frame.output_size(), [60, 40]);
    assert_eq!(output.frame.rotation(), QuarterTurn::Clockwise90);
    let round: Project = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
    round.validate().unwrap();
    assert_eq!(
        round.active_image_output(&id(2)).unwrap(),
        Some(output.frame)
    );
}

#[test]
fn ordinary_annotation_and_adjustment_crop_guides_remain_guides() {
    for kind in ["annotation", "adjustment"] {
        let mut p = fixture(kind);
        p.layers.get_mut(&id(3)).unwrap().definition.name = "output".into();
        p.validate().unwrap();
        assert!(p.image_output(&id(2)).unwrap().is_none());
    }
}

#[test]
fn disabled_frame_is_retained_but_ambiguous_or_malformed_disabled_frames_refuse() {
    let mut p = fixture("output");
    p.layers.get_mut(&id(3)).unwrap().visible = false;
    p.validate().unwrap();
    assert!(p.active_image_output(&id(2)).unwrap().is_none());
    assert!(p.image_output(&id(2)).unwrap().is_some());
    let mut layer = p.layers[&id(3)].clone();
    layer.definition.layer_id = Some(id(5).to_proto());
    p.layers.insert(id(5), layer);
    assert!(p.validate().is_err());
    p.layers.remove(&id(5));
    p.objects.get_mut(&id(4)).unwrap().state.hidden = true;
    assert!(p.validate().is_err());
}

#[test]
fn output_authority_refuses_blend_opacity_groups_and_noncanonical_geometry() {
    for change in 0..6 {
        let mut p = fixture("output");
        match change {
            0 => p.layers.get_mut(&id(3)).unwrap().opacity = 0.5,
            1 => p.layers.get_mut(&id(3)).unwrap().blend = "multiply".into(),
            2 => p.objects.get_mut(&id(4)).unwrap().state.role = pb::Role::Change as i32,
            3 => {
                p.objects
                    .get_mut(&id(4))
                    .unwrap()
                    .state
                    .transform
                    .as_mut()
                    .unwrap()
                    .e += 0.5
            }
            4 => {
                p.objects
                    .get_mut(&id(4))
                    .unwrap()
                    .state
                    .transform
                    .as_mut()
                    .unwrap()
                    .a = 0.1
            }
            _ => {
                let mut other = p.objects[&id(4)].clone();
                other.state.object_id = Some(id(8).to_proto());
                p.objects.insert(id(8), other);
            }
        }
        assert!(p.validate().is_err(), "case {change}");
    }
}

#[test]
fn output_plan_never_changes_originals_or_document_dimensions() {
    let p = fixture("output");
    let assets = p.assets.clone();
    let document = p.documents[&id(2)].definition.clone();
    p.validate().unwrap();
    p.image_output(&id(2)).unwrap();
    assert_eq!(p.assets, assets);
    assert_eq!(p.documents[&id(2)].definition, document);
    let mut p = p;
    p.layers.get_mut(&id(3)).unwrap().definition.kind = "unknown_output_v2".into();
    assert!(
        p.validate().is_err(),
        "unrecognized kinds fail closed rather than falling back to guides"
    );
}

#[test]
fn canonical_output_edits_restore_hash_without_rebasing_ordinary_objects() {
    let mut p = fixture("output");
    let mut layer = p.layers[&id(3)].clone();
    layer.definition.layer_id = Some(id(5).to_proto());
    layer.definition.kind = "annotation".into();
    p.layers.insert(id(5), layer);
    let mut object = p.objects[&id(4)].clone();
    object.state.object_id = Some(id(6).to_proto());
    object.state.layer_id = Some(id(5).to_proto());
    object.state.shape = Some(pb::object_state::Shape::Rect(pb::RectD {
        x: 7.0,
        y: 9.0,
        w: 3.0,
        h: 4.0,
    }));
    object.state.transform = Some(pb::Affine {
        a: 1.0,
        d: 1.0,
        ..Default::default()
    });
    p.objects.insert(id(6), object);
    let before = p.clone();
    let hash = p.state_hash().unwrap();
    // Transactions own undo; this model regression checks the exact state that
    // an inverse must restore, without introducing an ops dependency here.
    let replacement =
        ImageOutputFrame::new([30, 40], [0, 0, 30, 40], QuarterTurn::Half, [15, 20]).unwrap();
    let [a, b, c, d, e, f] = replacement.matrix().unwrap().coefficients();
    let state = &mut p.objects.get_mut(&id(4)).unwrap().state;
    state.shape = Some(pb::object_state::Shape::Crop(pb::RectD {
        x: 0.0,
        y: 0.0,
        w: 30.0,
        h: 40.0,
    }));
    state.transform = Some(pb::Affine { a, b, c, d, e, f });
    p.validate().unwrap();
    assert_ne!(p.state_hash().unwrap(), hash);
    assert_eq!(p.assets, before.assets);
    assert_eq!(p.documents, before.documents);
    assert_eq!(p.objects[&id(6)], before.objects[&id(6)]);
    p.objects.insert(id(4), before.objects[&id(4)].clone());
    assert_eq!(p.state_hash().unwrap(), hash);
    let bytes = p.canonical_bytes().unwrap();
    let restored = Project::from_canonical_bytes(&bytes).unwrap();
    assert_eq!(restored, before);
}

#[test]
fn malformed_or_unsupported_source_and_fractional_crop_refuse() {
    for change in 0..4 {
        let mut p = fixture("output");
        match change {
            0 => {
                p.documents.get_mut(&id(2)).unwrap().definition.kind = pb::DocumentKind::Svg as i32
            }
            1 => p.assets.values_mut().next().unwrap().width = 32769,
            2 => p.layers.get_mut(&id(3)).unwrap().definition.page_index = 0,
            _ => {
                if let Some(pb::object_state::Shape::Crop(crop)) =
                    &mut p.objects.get_mut(&id(4)).unwrap().state.shape
                {
                    crop.x = 2.5;
                }
            }
        }
        assert!(p.image_output(&id(2)).is_err(), "case {change}");
    }
}
