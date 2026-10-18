//! Synthetic, cross-platform model fixtures. No device or captured user data.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use vw_model::{
    AssetId, DeviceId, Document, Group, Id, Layer, ModelError, Object, OrderKey, PdfPage, Project,
    ResultCandidate, SemanticSnapshot, validate_asset, validate_capture, validate_object_state,
};
use vw_proto::{Message, v1 as pb};

fn id(index: u8) -> Id {
    Id::from_parts(1_700_000_000_000 + u64::from(index), [index; 10]).unwrap()
}

fn device() -> DeviceId {
    DeviceId::from_bytes([3; 16])
}

fn rect() -> pb::RectD {
    pb::RectD {
        x: -17.25,
        y: 2.5,
        w: 128.0,
        h: 96.0,
    }
}

fn points(count: usize) -> pb::Polyline {
    pb::Polyline {
        points: (0..count)
            .map(|index| pb::PointD {
                x: index as f64 * 1.25,
                y: index as f64 * -2.5,
            })
            .collect(),
        closed: count > 2,
    }
}

fn stroke() -> pb::Stroke {
    pb::Stroke {
        x: vec![0.0, 0.25, 4.5],
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
    }
}

fn shapes() -> Vec<pb::object_state::Shape> {
    use pb::object_state::Shape;
    vec![
        Shape::Stroke(stroke()),
        Shape::Line(points(2)),
        Shape::Arrow(points(3)),
        Shape::Rect(rect()),
        Shape::Ellipse(rect()),
        Shape::Polygon(points(4)),
        Shape::Text(pb::TextObject {
            text: "Synthetic annotation \u{03a9}".into(),
            font_family: "Noto Sans".into(),
            font_size: 14.0,
            anchor: Some(pb::PointD { x: 5.0, y: 7.0 }),
        }),
        Shape::Marker(pb::Marker {
            number: 1,
            point: Some(pb::PointD { x: 1.5, y: 2.5 }),
            r#box: Some(rect()),
            element_eids: vec!["synthetic-element".into()],
        }),
        Shape::SelectionVector(points(3)),
        Shape::SelectionRaster(pb::RasterMaskRef {
            mask_asset_id: AssetId::hash(b"synthetic original").to_string(),
            bounds: Some(rect()),
            feather: 1.0,
        }),
        Shape::Crop(rect()),
        Shape::ResultId(id(8).to_proto()),
        Shape::Adjustment(pb::Adjustment {
            brightness: 0.25,
            contrast: -0.25,
            levels: vec![0.0, 1.0, 1.0, 0.0, 1.0],
        }),
    ]
}

fn object(index: u8, shape: pb::object_state::Shape) -> pb::ObjectState {
    pb::ObjectState {
        object_id: Some(id(index).to_proto()),
        layer_id: Some(id(3).to_proto()),
        order_key: "V".into(),
        transform: Some(pb::Affine {
            a: 1.0,
            d: 1.0,
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
        created_at_ms: 1_700_000_000_000,
        shape: Some(shape),
        ..Default::default()
    }
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
        captured_at_ms: 1_700_000_000_000,
        metadata_json: r#"{"description":"Synthetic fixture","copyright":"Test"}"#.into(),
    }
}

fn project() -> Project {
    let mut project = Project::new(id(1), "Synthetic model fixture".into(), device());
    let original = asset();
    project.assets.insert(
        AssetId::try_from(original.asset_id.clone()).unwrap(),
        original,
    );
    project.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(2).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Fixture image".into(),
                primary_asset_id: asset().asset_id,
                capture: None,
            },
            pages: Vec::new(),
            created_at_ms: 1_700_000_000_000,
        },
    );
    project.layers.insert(
        id(3),
        Layer {
            definition: pb::CreateLayer {
                layer_id: Some(id(3).to_proto()),
                document_id: Some(id(2).to_proto()),
                page_index: -1,
                name: "Annotations".into(),
                kind: "annotation".into(),
                order_key: "V".into(),
            },
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: "normal".into(),
        },
    );
    project.results.insert(
        id(8),
        ResultCandidate {
            definition: pb::AddResult {
                result_id: Some(id(8).to_proto()),
                document_id: Some(id(2).to_proto()),
                provider: "synthetic-provider".into(),
                model: "synthetic-model".into(),
                request_json: "{}".into(),
                ..Default::default()
            },
            status: "pending".into(),
            acceptance_mask_asset_id: None,
            created_at_ms: 1_700_000_000_000,
        },
    );
    project
}

fn capture() -> pb::CaptureInfo {
    pb::CaptureInfo {
        capture_session_id: Some(id(9).to_proto()),
        frame_id: u64::MAX - 1,
        geometry: Some(pb::CaptureGeometry {
            source_kind: "window".into(),
            window_handle: 0x1234,
            monitor_id: "synthetic-monitor".into(),
            client_rect_host: Some(pb::RectI {
                x: -3840,
                y: -2160,
                w: 1920,
                h: 1080,
            }),
            dpi_scale: 1.75,
            geometry_revision: u32::MAX,
            timestamp_ns: 17_000_000_000,
        }),
        platform: "windows".into(),
        app_name: "SyntheticApp".into(),
        window_title: "Synthetic local capture".into(),
        lossless: true,
        degraded: false,
        captured_at_ms: 1_700_000_000_001,
    }
}

#[test]
fn uuid_v7_vectors_validate_version_variant_and_canonical_encoding() {
    let value = Id::from_parts(0x010203040506, [0xff; 10]).unwrap();
    assert_eq!(value.as_str(), "01020304-0506-7fff-bfff-ffffffffffff");
    assert_eq!(Id::from_proto(Some(&value.to_proto())).unwrap(), value);
    assert_eq!(Id::try_from(value.to_string()).unwrap(), value);
    assert!(Id::from_parts(1 << 48, [0; 10]).is_err());
    for invalid in [
        "",
        "01020304-0506-4fff-bfff-ffffffffffff",
        "01020304-0506-7fff-ffff-ffffffffffff",
        "01020304-0506-7FFF-bfff-ffffffffffff",
        "0102030405067fffbfffffffffffffff",
    ] {
        assert!(Id::try_from(invalid.to_owned()).is_err());
    }
    assert!(Id::from_proto(None).is_err());
    assert!(Id::from_proto(Some(&pb::Uuid { value: vec![0; 15] })).is_err());
    assert!(Id::from_bytes([0; 16]).is_err());
    let first = Id::generate(1_700_000_000_000).unwrap();
    let second = Id::generate(1_700_000_000_000).unwrap();
    assert_ne!(first, second);
}

#[test]
fn device_and_asset_ids_reject_aliases_overflow_and_noncanonical_text() {
    assert_eq!(
        DeviceId::from_bytes([0; 16]).as_str(),
        "00000000000000000000000000"
    );
    assert_eq!(
        DeviceId::from_bytes([255; 16]).as_str(),
        "7ZZZZZZZZZZZZZZZZZZZZZZZZZ"
    );
    for invalid in [
        "80000000000000000000000000",
        "0000000000000000000000000I",
        "0000000000000000000000000l",
        "",
        "000",
    ] {
        assert!(DeviceId::try_from(invalid.to_owned()).is_err());
    }
    let random_device = DeviceId::generate().unwrap();
    assert_eq!(
        DeviceId::try_from(random_device.to_string()).unwrap(),
        random_device
    );
    // Public BLAKE3 empty-input known answer, independent of this model's encoder.
    let empty = AssetId::hash(b"");
    assert_eq!(
        empty.as_str(),
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
    );
    assert_eq!(empty.bytes()[..4], [0xaf, 0x13, 0x49, 0xb9]);
    assert!(AssetId::try_from(empty.to_string().to_uppercase()).is_err());
    assert!(AssetId::try_from("a".repeat(63)).is_err());
    assert!(serde_json::from_str::<Id>("\"bad-id\"").is_err());
    assert!(serde_json::from_str::<DeviceId>("\"bad-device\"").is_err());
    assert!(serde_json::from_str::<AssetId>("\"bad-hash\"").is_err());
}

#[test]
fn fractional_keys_cover_prefixes_boundaries_exhaustion_and_stable_ties() {
    for (left, right) in [
        (None, None),
        (None, Some("1")),
        (Some("z"), None),
        (Some("V"), Some("V1")),
        (Some("V0z"), Some("V1")),
        (Some("Azz"), Some("B")),
    ] {
        let lower = left.map(|v| OrderKey::try_from(v.to_owned()).unwrap());
        let upper = right.map(|v| OrderKey::try_from(v.to_owned()).unwrap());
        let middle = OrderKey::between(lower.as_ref(), upper.as_ref()).unwrap();
        assert!(lower.as_ref().is_none_or(|v| v < &middle));
        assert!(upper.as_ref().is_none_or(|v| &middle < v));
        assert_eq!(
            OrderKey::try_from(middle.as_str().to_owned()).unwrap(),
            middle
        );
    }
    let key = OrderKey::try_from("V".to_owned()).unwrap();
    assert!(OrderKey::between(Some(&key), Some(&key)).is_err());
    let exhausted = OrderKey::try_from(format!("{}1", "0".repeat(127))).unwrap();
    assert!(OrderKey::between(None, Some(&exhausted)).is_err());
    for invalid in ["", "0", "V0", "a/b", "\u{00e9}"] {
        assert!(OrderKey::try_from(invalid.to_owned()).is_err());
    }
    assert!(OrderKey::try_from("V".repeat(129)).is_err());
    let mut tied = [(key.clone(), id(12)), (key.clone(), id(10)), (key, id(11))];
    tied.sort();
    assert_eq!(
        tied.iter()
            .map(|(_, value)| value.clone())
            .collect::<Vec<_>>(),
        vec![id(10), id(11), id(12)]
    );
}

#[test]
fn fractional_order_survives_51200_seeded_insertions_without_reordering() {
    // Independent deterministic PRNG; reproducible on Windows and Android.
    for seed in 0..100_u64 {
        let mut random = seed + 1;
        let mut keys = Vec::<OrderKey>::new();
        for _ in 0..512 {
            random = random
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let index = (random % (keys.len() as u64 + 1)) as usize;
            let left = index.checked_sub(1).and_then(|i| keys.get(i));
            let right = keys.get(index);
            let middle = OrderKey::between(left, right).unwrap();
            assert!(left.is_none_or(|v| v < &middle));
            assert!(right.is_none_or(|v| &middle < v));
            keys.insert(index, middle);
        }
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
    }
}

#[test]
fn every_object_variant_round_trips_through_proto_and_canonical_project() {
    let mut project = project();
    let original = project.assets.clone();
    let variants = shapes();
    assert_eq!(variants.len(), 13);
    for (index, shape) in variants.into_iter().enumerate() {
        let state = object(20 + index as u8, shape);
        validate_object_state(&state).unwrap();
        assert_eq!(
            pb::ObjectState::decode(state.encode_to_vec().as_slice()).unwrap(),
            state
        );
        project.objects.insert(
            id(20 + index as u8),
            Object {
                document_id: id(2),
                state,
            },
        );
    }
    let bytes = project.canonical_bytes().unwrap();
    let restored = Project::from_canonical_bytes(&bytes).unwrap();
    assert_eq!(restored, project);
    assert_eq!(restored.canonical_bytes().unwrap(), bytes);
    assert_eq!(
        restored.state_hash().unwrap(),
        project.state_hash().unwrap()
    );
    assert_eq!(project.assets, original);
}

#[test]
fn canonical_hash_ignores_map_insertion_order_and_signed_zero() {
    let mut first = project();
    for index in [21, 20] {
        first.objects.insert(
            id(index),
            Object {
                document_id: id(2),
                state: object(index, pb::object_state::Shape::Rect(rect())),
            },
        );
    }
    let mut second = first.clone();
    second.objects.clear();
    for index in [20, 21] {
        let mut state = first.objects[&id(index)].clone();
        state.state.transform.as_mut().unwrap().e = -0.0;
        second.objects.insert(id(index), state);
    }
    assert_eq!(
        first.canonical_bytes().unwrap(),
        second.canonical_bytes().unwrap()
    );
    assert_eq!(first.state_hash().unwrap(), second.state_hash().unwrap());
    let canonical = first.canonical_bytes().unwrap();
    let mut separated = b"VisualWorkbench.Project.v1\0".to_vec();
    separated.extend(&canonical);
    assert_eq!(first.state_hash().unwrap(), AssetId::hash(&separated));
    assert_ne!(first.state_hash().unwrap(), AssetId::hash(&canonical));
    second.title.push('!');
    assert_ne!(first.state_hash().unwrap(), second.state_hash().unwrap());
}

#[test]
fn canonical_roundtrip_preserves_full_f64_geometry_precision() {
    let mut project = project();
    project.objects.insert(
        id(20),
        Object {
            document_id: id(2),
            state: object(20, pb::object_state::Shape::Rect(rect())),
        },
    );
    let mut random = 0x4d4f44454c_u64;
    for index in 0..512_u64 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        // Finite, nonzero values across 241 binary exponents; avoid fixture
        // arithmetic that would mask a lossy JSON parser behind rounded inputs.
        let bits = ((903 + index % 241) << 52) | (random & ((1 << 52) - 1));
        let coordinate = f64::from_bits(bits);
        project
            .objects
            .get_mut(&id(20))
            .unwrap()
            .state
            .transform
            .as_mut()
            .unwrap()
            .e = coordinate;
        let canonical = project.canonical_bytes().unwrap();
        let restored = Project::from_canonical_bytes(&canonical).unwrap();
        assert_eq!(
            restored.objects[&id(20)]
                .state
                .transform
                .as_ref()
                .unwrap()
                .e
                .to_bits(),
            bits
        );
        assert_eq!(restored.canonical_bytes().unwrap(), canonical);
        assert_eq!(
            restored.state_hash().unwrap(),
            project.state_hash().unwrap()
        );
    }
}

#[test]
fn malformed_json_unknown_fields_and_nonfinite_numbers_never_hash() {
    let mut project = project();
    assert!(Project::from_canonical_bytes(b"{broken}").is_err());
    let mut unknown = serde_json::to_value(&project).unwrap();
    unknown["unexpected"] = true.into();
    assert!(Project::from_canonical_bytes(&serde_json::to_vec(&unknown).unwrap()).is_err());
    let mut unknown_nested = serde_json::to_value(&project).unwrap();
    unknown_nested["documents"][id(2).as_str()]["definition"]["unexpected"] = true.into();
    assert!(Project::from_canonical_bytes(&serde_json::to_vec(&unknown_nested).unwrap()).is_err());
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        project.layers.get_mut(&id(3)).unwrap().opacity = invalid;
        assert!(project.state_hash().is_err());
    }
    let invalid: Result<Id, _> = serde_json::from_str("\"synthetic-private-text\"");
    assert!(
        !invalid
            .unwrap_err()
            .to_string()
            .contains("synthetic-private-text")
    );
}

#[test]
fn reserved_timeline_unknown_kind_and_schema_return_typed_errors() {
    for kind in [
        pb::DocumentKind::Timeline as i32,
        pb::DocumentKind::Unspecified as i32,
        700,
    ] {
        let mut project = project();
        project.documents.get_mut(&id(2)).unwrap().definition.kind = kind;
        let encoded = serde_json::to_vec(&project).unwrap();
        assert!(
            matches!(Project::from_canonical_bytes(&encoded), Err(ModelError::UnsupportedKind(value)) if value == kind)
        );
    }
    let mut project = project();
    project.schema_version = 2;
    assert!(matches!(
        project.validate(),
        Err(ModelError::UnsupportedSchema(2))
    ));
    project.schema_version = 1;
    project
        .documents
        .get_mut(&id(2))
        .unwrap()
        .definition
        .schema_version = 12;
    assert!(matches!(
        project.validate(),
        Err(ModelError::UnsupportedSchema(12))
    ));
}

#[test]
fn capture_info_round_trips_all_fields_without_integer_precision_loss() {
    let mut project = project();
    let capture = capture();
    validate_capture(&capture).unwrap();
    assert_eq!(
        pb::CaptureInfo::decode(capture.encode_to_vec().as_slice()).unwrap(),
        capture
    );
    let document = project.documents.get_mut(&id(2)).unwrap();
    document.definition.kind = pb::DocumentKind::Capture as i32;
    document.definition.capture = Some(capture.clone());
    let roundtrip = Project::from_canonical_bytes(&project.canonical_bytes().unwrap()).unwrap();
    assert_eq!(
        roundtrip.documents[&id(2)].definition.capture.as_ref(),
        Some(&capture)
    );
    assert_eq!(
        roundtrip.state_hash().unwrap(),
        project.state_hash().unwrap()
    );
    for change in 0..5 {
        let mut invalid = capture.clone();
        match change {
            0 => invalid.degraded = true,
            1 => invalid.geometry.as_mut().unwrap().dpi_scale = f64::NAN,
            2 => {
                invalid
                    .geometry
                    .as_mut()
                    .unwrap()
                    .client_rect_host
                    .as_mut()
                    .unwrap()
                    .w = 0
            }
            3 => {
                invalid
                    .geometry
                    .as_mut()
                    .unwrap()
                    .client_rect_host
                    .as_mut()
                    .unwrap()
                    .h = -1
            }
            _ => invalid.capture_session_id = None,
        }
        assert!(validate_capture(&invalid).is_err());
    }
}

#[test]
fn semantic_capture_binding_is_exact_and_text_is_untrusted() {
    let mut project = project();
    let capture = capture();
    let document = project.documents.get_mut(&id(2)).unwrap();
    document.definition.kind = pb::DocumentKind::Capture as i32;
    document.definition.capture = Some(capture.clone());
    let snapshot = SemanticSnapshot {
        definition: pb::AddSemanticSnapshot {
            snapshot_id: Some(id(50).to_proto()),
            document_id: Some(id(2).to_proto()),
            platform: "uia".into(),
            frame_delta_ms: -3,
            elements_json_zstd: vec![1, 2, 3],
        },
        capture_session_id: Some(id(9)),
        frame_id: Some(capture.frame_id),
        created_at_ms: capture.captured_at_ms,
    };
    assert!(snapshot.text_is_untrusted());
    project.semantic_snapshots.insert(id(50), snapshot.clone());
    project.validate().unwrap();
    project
        .semantic_snapshots
        .get_mut(&id(50))
        .unwrap()
        .frame_id = Some(7);
    assert!(project.validate().is_err());
    project.semantic_snapshots.insert(id(50), snapshot.clone());
    project
        .semantic_snapshots
        .get_mut(&id(50))
        .unwrap()
        .capture_session_id = Some(id(10));
    assert!(project.validate().is_err());
    project.semantic_snapshots.insert(id(50), snapshot);
    project
        .semantic_snapshots
        .get_mut(&id(50))
        .unwrap()
        .frame_id = None;
    assert!(project.validate().is_err());
}

#[test]
fn invalid_shapes_and_common_fields_are_rejected_before_serialization() {
    use pb::object_state::Shape;
    let mut invalid_shapes = vec![
        Shape::Line(points(3)),
        Shape::Arrow(points(1)),
        Shape::Polygon(points(2)),
        Shape::SelectionVector(points(2)),
        Shape::Rect(pb::RectD { w: -1.0, ..rect() }),
        Shape::Ellipse(pb::RectD {
            h: f64::NAN,
            ..rect()
        }),
        Shape::Crop(pb::RectD {
            x: f64::INFINITY,
            ..rect()
        }),
        Shape::Text(pb::TextObject {
            text: "fixture".into(),
            font_family: "Noto Sans".into(),
            font_size: 0.0,
            anchor: Some(pb::PointD::default()),
        }),
        Shape::Marker(pb::Marker {
            number: 0,
            point: Some(pb::PointD::default()),
            ..Default::default()
        }),
        Shape::SelectionRaster(pb::RasterMaskRef {
            mask_asset_id: asset().asset_id,
            bounds: Some(rect()),
            feather: -1.0,
        }),
        Shape::ResultId(pb::Uuid { value: vec![0; 16] }),
        Shape::Adjustment(pb::Adjustment {
            brightness: 0.0,
            contrast: 0.0,
            levels: vec![0.0, 1.0, 0.0, 0.0, 1.0],
        }),
    ];
    for issue in 0..7 {
        let mut value = stroke();
        match issue {
            0 => {
                value.y.pop();
            }
            1 => value.t_ms = vec![0, 16, 8],
            2 => value.pressure[1] = 1.1,
            3 => value.tilt[1] = -0.1,
            4 => value.orientation[1] = f32::NAN,
            5 => value.brush.as_mut().unwrap().algorithm_version = 0,
            _ => {
                value.brush.as_mut().unwrap().pressure_curve.pop();
            }
        }
        invalid_shapes.push(Shape::Stroke(value));
    }
    for shape in invalid_shapes {
        assert!(validate_object_state(&object(20, shape)).is_err());
    }
    for issue in 0..8 {
        let mut value = object(20, Shape::Rect(rect()));
        match issue {
            0 => value.transform.as_mut().unwrap().a = 0.0,
            1 => value.style.as_mut().unwrap().width = f64::NAN,
            2 => value.style.as_mut().unwrap().fill = None,
            3 => value.role = 600,
            4 => value.created_by = "hardware-identity".into(),
            5 => value.shape = None,
            6 => value.order_key = "V0".into(),
            _ => value.object_id = None,
        }
        assert!(validate_object_state(&value).is_err());
    }
}

#[test]
fn groups_reject_cycles_missing_parents_and_cross_document_membership() {
    let mut project = project();
    project.groups.insert(
        id(60),
        Group {
            id: id(60),
            document_id: id(2),
            parent: None,
        },
    );
    project.groups.insert(
        id(61),
        Group {
            id: id(61),
            document_id: id(2),
            parent: Some(id(60)),
        },
    );
    let mut state = object(20, pb::object_state::Shape::Rect(rect()));
    state.group_id = Some(id(61).to_proto());
    project.objects.insert(
        id(20),
        Object {
            document_id: id(2),
            state,
        },
    );
    project.validate().unwrap();
    project.groups.get_mut(&id(60)).unwrap().parent = Some(id(61));
    assert!(matches!(
        project.validate(),
        Err(ModelError::Invalid("group cycle"))
    ));
    project.groups.get_mut(&id(60)).unwrap().parent = Some(id(62));
    assert!(matches!(
        project.validate(),
        Err(ModelError::Missing("parent group"))
    ));
    project.groups.get_mut(&id(60)).unwrap().parent = None;
    let mut other = project.documents[&id(2)].clone();
    other.definition.document_id = Some(id(4).to_proto());
    project.documents.insert(id(4), other);
    project.groups.get_mut(&id(60)).unwrap().document_id = id(4);
    assert!(project.validate().is_err());
    project.groups.get_mut(&id(60)).unwrap().document_id = id(2);
    project.objects.get_mut(&id(20)).unwrap().document_id = id(4);
    assert!(matches!(
        project.validate(),
        Err(ModelError::Invalid("cross-document object layer"))
    ));
}

#[test]
fn pdf_pages_and_entity_references_are_checked() {
    let mut project = project();
    let document = project.documents.get_mut(&id(2)).unwrap();
    document.definition.kind = pb::DocumentKind::Pdf as i32;
    document.pages.push(PdfPage {
        page_index: 0,
        crop_box: rect(),
        rotation_degrees: 90,
    });
    project
        .layers
        .get_mut(&id(3))
        .unwrap()
        .definition
        .page_index = 0;
    project.validate().unwrap();
    project
        .layers
        .get_mut(&id(3))
        .unwrap()
        .definition
        .page_index = 1;
    assert!(matches!(
        project.validate(),
        Err(ModelError::Missing("layer PDF page"))
    ));
    project
        .layers
        .get_mut(&id(3))
        .unwrap()
        .definition
        .page_index = 0;
    let page = project.documents[&id(2)].pages[0].clone();
    project.documents.get_mut(&id(2)).unwrap().pages.push(page);
    assert!(matches!(
        project.validate(),
        Err(ModelError::Invalid("duplicate PDF page"))
    ));
    project.documents.get_mut(&id(2)).unwrap().pages.pop();
    project.documents.get_mut(&id(2)).unwrap().pages[0].rotation_degrees = 13;
    assert!(project.validate().is_err());
    project.documents.get_mut(&id(2)).unwrap().pages[0].rotation_degrees = 0;
    project.assets.clear();
    assert!(matches!(
        project.validate(),
        Err(ModelError::Missing("asset reference"))
    ));
}

#[test]
fn asset_metadata_is_whitelisted_and_result_candidates_require_outside_proof() {
    let good = asset();
    validate_asset(&good).unwrap();
    for forbidden in ["gps", "serial_number", "account", "location"] {
        let mut value = good.clone();
        value.metadata_json = format!("{{\"{forbidden}\":\"synthetic\"}}");
        assert!(validate_asset(&value).is_err());
    }
    let mut nested = good.clone();
    nested.metadata_json = r#"{"description":{"location":"synthetic"}}"#.into();
    assert!(validate_asset(&nested).is_err());
    for orientation in [0, 9] {
        let mut value = good.clone();
        value.orientation = orientation;
        assert!(validate_asset(&value).is_err());
    }
    let mut video = good;
    video.format = "mp4".into();
    video.width = 0;
    video.height = 0;
    validate_asset(&video).unwrap();
    let mut project = project();
    project.results.get_mut(&id(8)).unwrap().status = "ready".into();
    assert!(project.validate().is_err());
    let result = project.results.get_mut(&id(8)).unwrap();
    result.definition.output_asset_id = asset().asset_id;
    result.definition.composite_asset_id = asset().asset_id;
    result.definition.proof_json = r#"{"changed_outside":1}"#.into();
    assert!(project.validate().is_err());
    project
        .results
        .get_mut(&id(8))
        .unwrap()
        .definition
        .proof_json = r#"{"changed_outside":0}"#.into();
    project.validate().unwrap();
    project
        .results
        .get_mut(&id(8))
        .unwrap()
        .definition
        .cost_estimate_usd = f64::INFINITY;
    assert!(project.state_hash().is_err());
}
