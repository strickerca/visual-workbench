//! Actual query callers for canonical output and explicit RAW preview assets.
//! Native execution awaits the coordinator's registered output/display hooks.
#![allow(clippy::unwrap_used)]

use super::*;
use std::{io::Cursor, path::Path};
use vw_geom::{ImageOutputFrame, QuarterTurn};
use vw_model::{DeviceId, Document, Layer, Object};
use vw_raster::{ExportFormat, ExportMetadata, ExportRequest, Pixels};

const BUDGET: u64 = 256 * 1024 * 1024;
const TIME: i64 = 1_700_000_000_000;
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) fn id(n: u8) -> Id {
    Id::from_parts(TIME as u64, [n; 10]).unwrap()
}
fn cancel() -> Arc<Cancellation> {
    Arc::new(Cancellation::new())
}
fn sample(x: u8, y: u8) -> [u8; 4] {
    [x * 30, y * 50, (x + y) * 20, 255]
}
fn request(format: ExportFormat) -> ExportRequest {
    ExportRequest {
        format,
        region: None,
        revision: pb::Revision {
            host_seq: 0,
            state_hash: vec![0; 32],
        },
        alpha: vw_raster::AlphaPolicy::Preserve,
        color: vw_raster::ColorPolicy::Preserve,
        allow_depth_reduction: false,
        memory_budget_bytes: BUDGET,
        capture_session: None,
        frame_id: None,

        capture_provenance: None,
    }
}
pub(crate) fn original(format: ExportFormat) -> Vec<u8> {
    vw_raster::export(
        &DecodedImage {
            width: 6,
            height: 4,
            pixels: Pixels::Rgba8(
                (0..4)
                    .flat_map(|y| (0..6).flat_map(move |x| sample(x, y)))
                    .collect(),
            ),
            icc: None,
            source_asset: AssetId::hash(b"synthetic-query-output"),
            original_available: true,
            orientation_applied: 1,
        },
        &request(format),
    )
    .unwrap()
    .bytes
}
pub(crate) fn model(bytes: &[u8], format: &str, metadata: &str, orientation: u32) -> Project {
    let owner = DeviceId::from_bytes([7; 16]);
    let asset = AssetId::hash(bytes);
    let mut project = Project::new(id(1), "query output fixture".into(), owner.clone());
    project.assets.insert(
        asset.clone(),
        pb::AddAsset {
            asset_id: asset.to_string(),
            format: format.into(),
            width: 6,
            height: 4,
            orientation,
            bit_depth: 8,
            byte_size: bytes.len() as u64,
            color_space: "untagged".into(),
            source: "import".into(),
            metadata_json: metadata.into(),
            ..Default::default()
        },
    );
    project.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(2).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "source stays in D".into(),
                primary_asset_id: asset.to_string(),
                ..Default::default()
            },
            pages: vec![],
            created_at_ms: TIME,
        },
    );
    layer(&mut project, 3, "annotation", true);
    project
}
fn layer(project: &mut Project, n: u8, kind: &str, visible: bool) {
    project.layers.insert(
        id(n),
        Layer {
            definition: pb::CreateLayer {
                layer_id: Some(id(n).to_proto()),
                document_id: Some(id(2).to_proto()),
                page_index: -1,
                kind: kind.into(),
                name: "names do not select metadata".into(),
                order_key: "V".into(),
            },
            visible,
            locked: false,
            opacity: 1.0,
            blend: "normal".into(),
        },
    );
}
fn object(project: &mut Project, n: u8, layer: u8, shape: Shape, transform: pb::Affine) {
    project.objects.insert(
        id(n),
        Object {
            document_id: id(2),
            state: pb::ObjectState {
                object_id: Some(id(n).to_proto()),
                layer_id: Some(id(layer).to_proto()),
                order_key: "V".into(),
                transform: Some(transform),
                style: Some(pb::Style {
                    stroke: Some(pb::Color { rgba: 0 }),
                    width: 0.0,
                    ..Default::default()
                }),
                role: pb::Role::None as i32,
                created_by: project.created_by.to_string(),
                created_at_ms: TIME,
                shape: Some(shape),
                ..Default::default()
            },
        },
    );
}
fn identity() -> pb::Affine {
    pb::Affine {
        a: 1.0,
        d: 1.0,
        ..Default::default()
    }
}
pub(crate) fn frame(project: &mut Project, enabled: bool) {
    layer(project, 30, vw_model::IMAGE_OUTPUT_LAYER_KIND, enabled);
    let frame =
        ImageOutputFrame::new([6, 4], [1, 1, 4, 2], QuarterTurn::Clockwise90, [2, 4]).unwrap();
    let [a, b, c, d, e, f] = frame.matrix().unwrap().coefficients();
    object(
        project,
        31,
        30,
        Shape::Crop(pb::RectD {
            x: 1.0,
            y: 1.0,
            w: 4.0,
            h: 2.0,
        }),
        pb::Affine { a, b, c, d, e, f },
    );
}
fn tone(project: &mut Project) {
    layer(project, 40, "adjustment", true);
    object(
        project,
        41,
        40,
        Shape::Adjustment(pb::Adjustment {
            brightness: -0.25,
            contrast: 0.0,
            levels: vec![0.0, 1.0, 1.0, 0.0, 1.0],
        }),
        identity(),
    );
}
pub(crate) fn session(path: &Path, project: Project, bytes: &[u8]) -> Arc<ProjectSession> {
    let owner = project.created_by.clone();
    let store = vw_store::ProjectStore::create(path, project, owner, TIME).unwrap();
    let blobs = vw_store::BlobStore::new(path).unwrap();
    blobs.put(bytes).unwrap();
    ProjectSession::from_store(
        store,
        blobs,
        crate::project::SessionPermit::acquire().unwrap(),
    )
    .unwrap()
}
fn options() -> ExportOptions {
    ExportOptions {
        document_id: id(2).to_string(),
        format: ImageFormat::Png8,
        marked: false,
        region: None,
        matte_rgb: None,
        convert_to_srgb: false,
        assume_untagged_srgb: true,
        allow_depth_reduction: false,
        memory_budget_bytes: BUDGET,
    }
}
fn png_metadata(bytes: &[u8]) -> ExportMetadata {
    let reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    let text = reader
        .info()
        .utf8_text
        .iter()
        .find(|value| value.keyword == "VisualWorkbench")
        .unwrap()
        .get_text()
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

#[tokio::test]
async fn render_lists_and_snapshots_omit_frame_metadata_but_keep_ordinary_crop_guides() {
    let _serial = SERIAL.lock().await;
    let root = tempfile::tempdir().unwrap();
    let bytes = original(ExportFormat::Png8);
    let mut project = model(&bytes, "png", "{}", 1);
    frame(&mut project, true);
    tone(&mut project);
    object(
        &mut project,
        4,
        3,
        Shape::Crop(pb::RectD {
            x: 0.0,
            y: 0.0,
            w: 2.0,
            h: 2.0,
        }),
        identity(),
    );
    let session = session(&root.path().join("p"), project, &bytes);
    let render = session
        .render_list(id(2).to_string(), None, cancel())
        .await
        .unwrap();
    assert_eq!(render.items.len(), 1);
    assert_eq!(render.items[0].object_id, id(4).to_string());
    assert!(matches!(render.items[0].shape, DrawShape::Guide { .. }));
    let snapshot = session
        .document_snapshot(id(2).to_string(), cancel())
        .await
        .unwrap();
    assert_eq!((snapshot.width, snapshot.height), (6, 4));
    assert!(snapshot.has_tone_adjustments);
    assert_eq!(snapshot.render.items.len(), 1);
    assert_eq!(snapshot.render.items[0].object_id, id(4).to_string());
    session.close().await.unwrap();
}

#[tokio::test]
async fn query_png_and_external_tiff_use_output_pixels_and_embed_source_identity() {
    let _serial = SERIAL.lock().await;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("p");
    let bytes = original(ExportFormat::Png8);
    let mut project = model(&bytes, "png", "{}", 1);
    frame(&mut project, true);
    let session = session(&path, project, &bytes);
    let before = session.info().await.unwrap();
    let expected: Vec<u8> = (1..5)
        .flat_map(|x| [sample(x, 2), sample(x, 1)].into_iter().flatten())
        .collect();
    for format in [ImageFormat::Png8, ImageFormat::Tiff8] {
        for marked in [false, true] {
            let mut options = options();
            options.format = format.clone();
            options.marked = marked;
            let result = session.export_image(options, cancel()).await.unwrap();
            let metadata: ExportMetadata = serde_json::from_str(&result.metadata_json).unwrap();
            assert_eq!((metadata.source_width, metadata.source_height), (6, 4));
            assert_eq!((metadata.output_width, metadata.output_height), (2, 4));
            assert_eq!(metadata.source_asset, AssetId::hash(&bytes).to_string());
            assert_eq!(metadata.image_output.unwrap().clockwise_degrees, 90);
            assert_eq!(result.revision, before);
            let decoded = crate::format_images::decode(
                &result.bytes,
                vw_raster::DecodeLimits::default(),
                &|| false,
            )
            .unwrap();
            assert_eq!((decoded.width, decoded.height), (2, 4));
            assert_eq!(decoded.pixels.rgba8(), expected);
            if matches!(format, ImageFormat::Png8) {
                let embedded = png_metadata(&result.bytes);
                assert_eq!(embedded.image_output, metadata.image_output);
                assert_eq!(embedded.source_asset, metadata.source_asset);
                assert_eq!((embedded.source_width, embedded.source_height), (6, 4));
            }
        }
    }
    assert_eq!(session.info().await.unwrap(), before);
    session.close().await.unwrap();
    let store = vw_store::ProjectStore::open(&path).unwrap();
    assert_eq!(store.project().assets.len(), 1);
    assert_eq!(
        vw_store::BlobStore::new(&path)
            .unwrap()
            .read(&AssetId::hash(&bytes))
            .unwrap(),
        bytes
    );
}

#[tokio::test]
async fn query_export_regions_are_output_coordinates_and_refuse_outside_output() {
    let _serial = SERIAL.lock().await;
    let root = tempfile::tempdir().unwrap();
    let bytes = original(ExportFormat::Png8);
    let mut project = model(&bytes, "png", "{}", 1);
    frame(&mut project, true);
    let session = session(&root.path().join("p"), project, &bytes);
    let mut options = options();
    options.region = Some(QueryRect {
        x: 1.0,
        y: 3.0,
        width: 1.0,
        height: 1.0,
    });
    let result = session
        .export_image(options.clone(), cancel())
        .await
        .unwrap();
    let decoded = vw_raster::decode(&result.bytes, vw_raster::DecodeLimits::default()).unwrap();
    assert_eq!((decoded.width, decoded.height), (1, 1));
    assert_eq!(decoded.pixels.rgba8(), sample(4, 1));
    let embedded = png_metadata(&result.bytes);
    assert_eq!((embedded.source_width, embedded.source_height), (6, 4));
    assert_eq!(embedded.image_output.unwrap().output_width, 2);
    // This fits D but does not fit the narrow rotated O grid.
    options.region.as_mut().unwrap().x = 2.0;
    assert!(session.export_image(options, cancel()).await.is_err());
    session.close().await.unwrap();
}

#[tokio::test]
async fn disabled_frames_keep_ordinary_export_and_clean_tones_still_compose() {
    let _serial = SERIAL.lock().await;
    let root = tempfile::tempdir().unwrap();
    let bytes = original(ExportFormat::Png8);
    let mut project = model(&bytes, "png", "{}", 1);
    frame(&mut project, false);
    let ordinary = session(&root.path().join("ordinary"), project.clone(), &bytes);
    let result = ordinary.export_image(options(), cancel()).await.unwrap();
    let metadata = png_metadata(&result.bytes);
    assert_eq!((metadata.output_width, metadata.output_height), (6, 4));
    assert!(metadata.image_output.is_none());
    let untoned = vw_raster::decode(&result.bytes, vw_raster::DecodeLimits::default()).unwrap();
    ordinary.close().await.unwrap();
    tone(&mut project);
    let toned = session(&root.path().join("toned"), project, &bytes);
    let result = toned.export_image(options(), cancel()).await.unwrap();
    let metadata = png_metadata(&result.bytes);
    assert_eq!((metadata.output_width, metadata.output_height), (6, 4));
    assert!(metadata.image_output.is_none());
    let decoded = vw_raster::decode(&result.bytes, vw_raster::DecodeLimits::default()).unwrap();
    assert_ne!(decoded.pixels.rgba8(), untoned.pixels.rgba8());
    toned.close().await.unwrap();
}

#[tokio::test]
async fn output_query_admits_budget_before_reading_a_corrupt_source_and_preserves_state() {
    let _serial = SERIAL.lock().await;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("p");
    let bytes = original(ExportFormat::Png8);
    let mut project = model(&bytes, "png", "{}", 1);
    frame(&mut project, true);
    let session = session(&path, project, &bytes);
    let before = session.info().await.unwrap();
    let blob = vw_store::BlobStore::new(&path)
        .unwrap()
        .path(&AssetId::hash(&bytes))
        .unwrap();
    std::fs::write(blob, vec![0; bytes.len()]).unwrap();
    for budget in [0, 512 * 1024 * 1024 + 1] {
        let mut options = options();
        options.memory_budget_bytes = budget;
        assert!(matches!(
            session.export_image(options, cancel()).await,
            Err(CoreError::Invalid)
        ));
    }
    let mut limited = options();
    limited.memory_budget_bytes = 16 * 1024 * 1024;
    assert!(matches!(
        session.export_image(limited, cancel()).await,
        Err(CoreError::Backpressure)
    ));
    let cancelled = cancel();
    cancelled.cancel();
    assert!(matches!(
        session.export_image(options(), cancelled).await,
        Err(CoreError::Cancelled)
    ));
    assert!(session.export_image(options(), cancel()).await.is_err());
    assert_eq!(session.info().await.unwrap(), before);
    session.close().await.unwrap();
}

pub(crate) fn raw_preview() -> Vec<u8> {
    fn entry(bytes: &mut Vec<u8>, tag: u16, value: u32) {
        bytes.extend(tag.to_le_bytes());
        bytes.extend(4_u16.to_le_bytes());
        bytes.extend(1_u32.to_le_bytes());
        bytes.extend(value.to_le_bytes());
    }
    let jpeg = original(ExportFormat::Jpeg { quality: 100 });
    let mut bytes = b"II*\0\x08\0\0\0".to_vec();
    bytes.extend(4_u16.to_le_bytes());
    entry(&mut bytes, 262, 32803);
    entry(&mut bytes, 274, 6);
    entry(&mut bytes, 330, 62);
    bytes.extend(50706_u16.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(4_u32.to_le_bytes());
    bytes.extend([1, 4, 0, 0]);
    bytes.extend(0_u32.to_le_bytes());
    assert_eq!(bytes.len(), 62);
    bytes.extend(4_u16.to_le_bytes());
    entry(&mut bytes, 254, 1);
    entry(&mut bytes, 513, 116);
    entry(&mut bytes, 514, jpeg.len() as u32);
    entry(&mut bytes, 50970, 0);
    bytes.extend(0_u32.to_le_bytes());
    assert_eq!(bytes.len(), 116);
    bytes.extend(jpeg);
    bytes
}

#[test]
fn asset_resolver_requires_canonical_raw_admission_and_never_promotes_preview_availability() {
    let root = tempfile::tempdir().unwrap();
    let bytes = raw_preview();
    let asset_id = AssetId::hash(&bytes);
    let blobs = vw_store::BlobStore::new(root.path()).unwrap();
    blobs.put(&bytes).unwrap();
    let cancellation = Cancellation::new();
    let mut project = model(&bytes, "dng", r#"{"raw_preview":true}"#, 6);
    let image = Assets {
        blobs: &blobs,
        project: &project,
        budget: BUDGET,
        cancel: &cancellation,
    }
    .image(asset_id.as_str())
    .unwrap();
    assert_eq!(image.source_asset, asset_id);
    assert_eq!(
        (image.width, image.height, image.orientation_applied),
        (4, 6, 6)
    );
    assert!(!image.original_available);
    assert!(vw_raster::BorrowedSource::new(&image).is_err());
    assert!(vw_raster::export(&image, &request(ExportFormat::Png8)).is_err());
    project.assets.get_mut(&asset_id).unwrap().metadata_json = "{}".into();
    assert!(
        Assets {
            blobs: &blobs,
            project: &project,
            budget: BUDGET,
            cancel: &cancellation
        }
        .image(asset_id.as_str())
        .is_err()
    );
    project.assets.get_mut(&asset_id).unwrap().metadata_json = r#"{"raw_preview":true}"#.into();
    project.assets.get_mut(&asset_id).unwrap().format = "tiff".into();
    assert!(
        Assets {
            blobs: &blobs,
            project: &project,
            budget: BUDGET,
            cancel: &cancellation
        }
        .image(asset_id.as_str())
        .is_err()
    );
    project.assets.clear();
    assert!(matches!(
        Assets {
            blobs: &blobs,
            project: &project,
            budget: BUDGET,
            cancel: &cancellation
        }
        .image(asset_id.as_str()),
        Err(vw_raster::RasterError::MissingAsset)
    ));
    assert_eq!(blobs.read(&asset_id).unwrap(), bytes);
}

#[tokio::test]
async fn reopened_explicit_raw_preview_can_display_but_cannot_export_original_pixels() {
    let _serial = SERIAL.lock().await;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("p");
    let bytes = raw_preview();
    let project = model(&bytes, "dng", r#"{"raw_preview":true}"#, 6);
    let created = session(&path, project, &bytes);
    created.close().await.unwrap();
    let reopened = open_project(path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    let before = reopened.info().await.unwrap();
    let background = reopened
        .background_image(id(2).to_string(), BUDGET, true, cancel())
        .await
        .unwrap();
    assert_eq!((background.width, background.height), (4, 6));
    assert_eq!(
        background.source_asset_id,
        AssetId::hash(&bytes).to_string()
    );
    assert_eq!(background.rgba.len(), 4 * 6 * 4);
    assert!(!background.icc_profile.is_empty());
    for marked in [false, true] {
        let mut options = options();
        options.marked = marked;
        assert!(reopened.export_image(options, cancel()).await.is_err());
    }
    assert_eq!(reopened.info().await.unwrap(), before);
    reopened.close().await.unwrap();
    assert_eq!(
        vw_store::BlobStore::new(&path)
            .unwrap()
            .read(&AssetId::hash(&bytes))
            .unwrap(),
        bytes
    );
}
