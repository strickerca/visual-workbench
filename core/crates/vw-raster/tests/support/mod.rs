#![allow(dead_code)]
use vw_model::{AssetId, DeviceId, Document, Id, Layer, Object, Project};
use vw_proto::v1::{self, object_state::Shape};
use vw_raster::*;
pub type TestResult = Result<(), Box<dyn std::error::Error>>;
pub fn assert_pixels_equal(actual: &Pixels, expected: &Pixels, context: &str) -> TestResult {
    assert_eq!(actual.bit_depth(), expected.bit_depth(), "{context}: depth");
    assert_eq!(actual.len(), expected.len(), "{context}: samples");
    let first = match (actual, expected) {
        (Pixels::Rgba8(a), Pixels::Rgba8(b)) => a
            .iter()
            .zip(b)
            .enumerate()
            .find(|(_, (x, y))| x != y)
            .map(|(i, (x, y))| (i, u16::from(*x), u16::from(*y))),
        (Pixels::Rgba16(a), Pixels::Rgba16(b)) => a
            .iter()
            .zip(b)
            .enumerate()
            .find(|(_, (x, y))| x != y)
            .map(|(i, (x, y))| (i, *x, *y)),
        _ => return Err("pixel depth mismatch".into()),
    };
    assert_eq!(
        first, None,
        "{context}: first differing (sample, actual, expected)"
    );
    Ok(())
}
pub fn id(value: u8) -> Result<Id, vw_model::ModelError> {
    Id::from_parts(u64::from(value), [value; 10])
}
pub fn source(w: u32, h: u32, depth: u8) -> DecodedImage {
    let samples = (0..w * h)
        .flat_map(|i| {
            [
                (i * 997 % 65536) as u16,
                (i * 499 + 12345) as u16,
                (i * 231 + 45678) as u16,
                if i % 7 == 0 { 0 } else { 65535 },
            ]
        })
        .collect::<Vec<_>>();
    let pixels = if depth == 16 {
        Pixels::Rgba16(samples)
    } else {
        Pixels::Rgba8(samples.iter().map(|v| (v >> 8) as u8).collect())
    };
    DecodedImage {
        width: w,
        height: h,
        pixels,
        icc: None,
        source_asset: AssetId::hash(b"synthetic raster original v1"),
        original_available: true,
        orientation_applied: 1,
    }
}
pub fn request(format: ExportFormat) -> ExportRequest {
    ExportRequest {
        format,
        region: None,
        revision: v1::Revision {
            host_seq: 42,
            state_hash: vec![0xab; 32],
        },
        alpha: AlphaPolicy::Preserve,
        color: ColorPolicy::Preserve,
        allow_depth_reduction: false,
        memory_budget_bytes: 1024 * 1024 * 1024,
        capture_session: None,
        frame_id: None,
    }
}
pub fn asset(image: &DecodedImage) -> v1::AddAsset {
    v1::AddAsset {
        asset_id: image.source_asset.to_string(),
        format: "png".into(),
        width: image.width,
        height: image.height,
        orientation: 1,
        bit_depth: u32::from(image.pixels.bit_depth()),
        has_alpha: image.pixels.has_alpha(),
        color_space: "sRGB".into(),
        icc_profile: image.icc.clone().unwrap_or_default(),
        byte_size: 1,
        source: "import".into(),
        captured_at_ms: 0,
        metadata_json: "{}".into(),
    }
}
pub fn project(source: &DecodedImage) -> Result<(Project, Id, Id), Box<dyn std::error::Error>> {
    let owner = DeviceId::from_bytes([1; 16]);
    let doc = id(2)?;
    let layer = id(3)?;
    let mut project = Project::new(id(1)?, "synthetic".into(), owner);
    project
        .assets
        .insert(source.source_asset.clone(), asset(source));
    project.documents.insert(
        doc.clone(),
        Document {
            definition: v1::CreateDocument {
                document_id: Some(doc.to_proto()),
                kind: v1::DocumentKind::Image as i32,
                schema_version: 1,
                title: "raster".into(),
                primary_asset_id: source.source_asset.to_string(),
                capture: None,
            },
            pages: vec![],
            created_at_ms: 0,
        },
    );
    project.layers.insert(
        layer.clone(),
        Layer {
            definition: v1::CreateLayer {
                layer_id: Some(layer.to_proto()),
                document_id: Some(doc.to_proto()),
                page_index: -1,
                name: "ink".into(),
                kind: "annotation".into(),
                order_key: "V".into(),
            },
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: "normal".into(),
        },
    );
    Ok((project, doc, layer))
}
pub fn object(
    doc: &Id,
    layer: &Id,
    number: u8,
    shape: Shape,
) -> Result<(Id, Object), vw_model::ModelError> {
    let id = id(number)?;
    Ok((
        id.clone(),
        Object {
            document_id: doc.clone(),
            state: v1::ObjectState {
                object_id: Some(id.to_proto()),
                layer_id: Some(layer.to_proto()),
                order_key: "V".into(),
                transform: Some(identity()),
                style: Some(v1::Style {
                    stroke: Some(v1::Color { rgba: 0xe03050c0 }),
                    width: 2.0,
                    screen_constant_width: false,
                    fill: Some(v1::Color { rgba: 0x20b08080 }),
                    has_fill: true,
                }),
                role: v1::Role::None as i32,
                group_id: None,
                locked: false,
                hidden: false,
                created_by: DeviceId::from_bytes([1; 16]).to_string(),
                created_at_ms: 0,
                shape: Some(shape),
            },
        },
    ))
}
pub fn identity() -> v1::Affine {
    v1::Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    }
}
pub struct NoAssets;
impl AssetResolver for NoAssets {
    fn image(&self, _: &str) -> Result<DecodedImage, RasterError> {
        Err(RasterError::MissingAsset)
    }
}
pub fn options() -> RenderOptions {
    RenderOptions {
        assume_untagged_srgb: true,
        ..RenderOptions::default()
    }
}
pub fn point(x: f64, y: f64) -> v1::PointD {
    v1::PointD { x, y }
}
pub fn rect(x: f64, y: f64, w: f64, h: f64) -> v1::RectD {
    v1::RectD { x, y, w, h }
}
pub fn all_annotations() -> Result<(DecodedImage, Project, Id), Box<dyn std::error::Error>> {
    let source = source(192, 128, 16);
    let (mut project, doc, layer) = project(&source)?;
    let shapes = vec![
        Shape::Stroke(v1::Stroke {
            x: vec![4., 15., 30.],
            y: vec![12., 6., 15.],
            t_ms: vec![0, 8, 16],
            pressure: vec![0.3, 0.7, 1.0],
            tilt: vec![],
            orientation: vec![],
            brush: Some(v1::Brush {
                family: "highlighter".into(),
                algorithm_version: 1,
                base_width: 6.,
                pressure_curve: vec![
                    0.,
                    0.,
                    0.333333333333,
                    0.333333333333,
                    0.666666666666,
                    0.666666666666,
                    1.,
                    1.,
                ],
                stabilization: 0.0,
            }),
        }),
        Shape::Line(v1::Polyline {
            points: vec![point(40., 5.), point(58., 20.)],
            closed: false,
        }),
        Shape::Arrow(v1::Polyline {
            points: vec![point(68., 8.), point(78., 14.), point(92., 5.)],
            closed: false,
        }),
        Shape::Rect(rect(5., 30., 28., 20.)),
        Shape::Ellipse(rect(40., 30., 28., 20.)),
        Shape::Polygon(v1::Polyline {
            points: vec![point(78., 28.), point(98., 48.), point(72., 45.)],
            closed: true,
        }),
        Shape::Text(v1::TextObject {
            text: "ffi AV café".into(),
            font_family: "Inter".into(),
            font_size: 15.,
            anchor: Some(point(5., 76.)),
        }),
        Shape::Marker(v1::Marker {
            number: 12,
            point: Some(point(130., 40.)),
            r#box: Some(rect(110., 20., 45., 45.)),
            element_eids: vec![],
        }),
        Shape::Text(v1::TextObject {
            text: "Noto Ω Ж".into(),
            font_family: "Noto Sans".into(),
            font_size: 12.,
            anchor: Some(point(4., 100.)),
        }),
        Shape::Text(v1::TextObject {
            text: "mono 0123".into(),
            font_family: "Noto Sans Mono".into(),
            font_size: 12.,
            anchor: Some(point(4., 119.)),
        }),
    ];
    for (n, shape) in shapes.into_iter().enumerate() {
        let (id, object) = object(&doc, &layer, 10 + n as u8, shape)?;
        project.objects.insert(id, object);
    }
    Ok((source, project, doc))
}
