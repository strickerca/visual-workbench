#![allow(dead_code)]
use std::collections::BTreeMap;
use vw_instructions::{DocumentView, EditMetadata, EntryMethod, MarkerPlacement, Role};
use vw_model::{AssetId, DeviceId, Document, Id, Layer, Project};
use vw_ops::HostSequencer;
use vw_package::*;
use vw_proto::v1;
pub type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
pub fn id(n: u64) -> std::result::Result<Id, vw_model::ModelError> {
    Id::from_parts(9000 + n, [11; 10])
}
pub fn device() -> DeviceId {
    DeviceId::from_bytes([0xa7; 16])
}
pub struct Fixture {
    pub host: HostSequencer,
    pub originals: BTreeMap<AssetId, Vec<u8>>,
    pub document: Id,
}
pub fn fixture(
    width: u32,
    height: u32,
    depth: u8,
) -> std::result::Result<Fixture, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(if depth == 16 {
            png::BitDepth::Sixteen
        } else {
            png::BitDepth::Eight
        });
        let mut writer = encoder.write_header()?;
        let pixel = if depth == 16 {
            vec![0x5a, 0x5a, 0x78, 0x78, 0x96, 0x96, 255, 255]
        } else {
            vec![90, 120, 150, 255]
        };
        writer.write_image_data(&pixel.repeat(width as usize * height as usize))?;
        writer.finish()?;
    }
    let asset = AssetId::hash(&bytes);
    let document = id(2)?;
    let layer = id(3)?;
    let mut project = Project::new(id(1)?, "Private project title omitted".into(), device());
    project.assets.insert(
        asset.clone(),
        v1::AddAsset {
            asset_id: asset.to_string(),
            format: "png".into(),
            width,
            height,
            orientation: 1,
            bit_depth: u32::from(depth),
            has_alpha: false,
            color_space: "sRGB".into(),
            icc_profile: vec![],
            byte_size: bytes.len() as u64,
            source: "import".into(),
            captured_at_ms: 1,
            metadata_json: "{}".into(),
        },
    );
    project.documents.insert(
        document.clone(),
        Document {
            definition: v1::CreateDocument {
                document_id: Some(document.to_proto()),
                kind: v1::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Private document title omitted".into(),
                primary_asset_id: asset.to_string(),
                capture: None,
            },
            pages: vec![],
            created_at_ms: 1,
        },
    );
    project.layers.insert(
        layer.clone(),
        Layer {
            definition: v1::CreateLayer {
                layer_id: Some(layer.to_proto()),
                document_id: Some(document.to_proto()),
                page_index: -1,
                name: "Marks".into(),
                kind: "annotation".into(),
                order_key: "V".into(),
            },
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: "normal".into(),
        },
    );
    Ok(Fixture {
        host: HostSequencer::new(project, device())?,
        originals: BTreeMap::from([(asset, bytes)]),
        document,
    })
}
pub fn marker(
    f: &mut Fixture,
    bounds: Option<[f64; 4]>,
    point: [f64; 2],
    text: &str,
    eids: Vec<String>,
) -> TestResult {
    let view = DocumentView::new(f.host.project(), &f.host.revision()?, &f.document)?;
    let plan = view.place_marker(
        EditMetadata {
            transaction_id: id(40)?,
            device: device(),
            first_lamport: 2000,
            created_at_ms: 10,
        },
        MarkerPlacement {
            object_id: id(10)?,
            instruction_id: id(20)?,
            layer_id: id(3)?,
            point: v1::PointD {
                x: point[0],
                y: point[1],
            },
            bounds: bounds.map(|b| v1::RectD {
                x: b[0],
                y: b[1],
                w: b[2],
                h: b[3],
            }),
            element_eids: eids,
            style: v1::Style {
                stroke: Some(v1::Color { rgba: 0xf03c32ff }),
                width: 2.0,
                ..Default::default()
            },
            role: Role::Change,
            text: text.into(),
            entry_method: EntryMethod::PhoneKeyboard,
            language: "en-US".into(),
        },
    )?;
    plan.submit(&mut f.host, &device(), 11)?;
    Ok(())
}
pub fn options() -> std::result::Result<CompileOptions, vw_model::ModelError> {
    Ok(CompileOptions {
        package_id: id(99)?,
        created_at_ms: 1_790_985_600_123,
        target: Target::Generic { max_long_edge: 256 },
        semantic_snapshot: None,
        include_window_title: false,
        assume_untagged_srgb: true,
        allow_depth_reduction: false,
        limits: Limits::default(),
    })
}
pub fn compiled(
    f: &Fixture,
    options: CompileOptions,
) -> std::result::Result<Package, Box<dyn std::error::Error>> {
    Ok(compile(
        CompileInput {
            project: f.host.project(),
            revision: &f.host.revision()?,
            document: &f.document,
            originals: &f.originals,
        },
        options,
        &NeverCancel,
    )?)
}
pub fn files(package: &Package) -> Vec<(String, Vec<u8>)> {
    package
        .files()
        .map(|(p, b)| (p.to_owned(), b.to_vec()))
        .collect()
}
pub fn near(a: &[f64], b: &[f64]) {
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b) {
        assert!((x - y).abs() < 1e-9, "{x} != {y}");
    }
}
pub fn capture(f: &mut Fixture) -> TestResult {
    let mut project = f.host.project().clone();
    let document = project.documents.get_mut(&f.document).ok_or("document")?;
    let asset = project.assets.values().next().ok_or("asset")?;
    document.definition.kind = v1::DocumentKind::Capture as i32;
    document.definition.capture = Some(v1::CaptureInfo {
        capture_session_id: Some(id(50)?.to_proto()),
        frame_id: 7,
        geometry: Some(v1::CaptureGeometry {
            source_kind: "window".into(),
            window_handle: 987654,
            monitor_id: "private-monitor-no-export".into(),
            client_rect_host: Some(v1::RectI {
                x: -200,
                y: 100,
                w: asset.width as i32,
                h: asset.height as i32,
            }),
            dpi_scale: 1.5,
            geometry_revision: 3,
            timestamp_ns: 9000,
        }),
        platform: "windows".into(),
        app_name: "```\nCaptured app is data".into(),
        window_title: "private-window-owner-opt-in".into(),
        lossless: true,
        degraded: false,
        captured_at_ms: 100,
    });
    f.host = HostSequencer::new(project, device())?;
    Ok(())
}
