//! Ordinary public FFI paths. These cases are written, not executed in this lane.
#![allow(clippy::unwrap_used, clippy::panic)]
mod support;
use support::*;
use std::{io::Cursor, path::Path};
use vw_core::*;
use vw_model::AssetId;
use vw_raster::{DecodedImage, Pixels};

fn fixture(depth: u8) -> (DecodedImage, Vec<u8>) {
    let image = DecodedImage { width: 2, height: 3,
        pixels: if depth == 16 { Pixels::Rgba16((0..6).flat_map(|n| [256 + n, 257 + n, 65535 - n, n * 10000]).collect()) }
        else { Pixels::Rgba8((0..6).flat_map(|n| [n, 255 - n, n * 13, n * 40]).collect()) },
        icc: Some(vw_raster::standard_srgb_profile().unwrap()), source_asset: AssetId::hash(b"original-format-test"),
        original_available: true, orientation_applied: 1 };
    let mut out = Cursor::new(Vec::new());
    vw_formats::encode_to(&image, if depth == 16 { vw_formats::ExportFormat::Tiff16 } else { vw_formats::ExportFormat::Tiff8 },
        vw_formats::Limits::default(), &mut out, &|| false).unwrap();
    (image, out.into_inner())
}
fn tiff_scalar(bytes: &mut [u8], tag: u16, value: u32) {
    let n = u16::from_le_bytes(bytes[8..10].try_into().unwrap());
    let at = (0..n).map(|i| 10 + usize::from(i) * 12)
        .find(|&at| u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()) == tag).unwrap();
    bytes[at + 8..at + 12].copy_from_slice(&value.to_le_bytes());
}
fn multipage(bytes: &mut Vec<u8>) {
    let n = u16::from_le_bytes(bytes[8..10].try_into().unwrap());
    let at = 10 + usize::from(n) * 12;
    let next = bytes.len() as u32;
    bytes[at..at + 4].copy_from_slice(&next.to_le_bytes());
    bytes.extend([0u8; 6]);
}
async fn memory_project(path: &Path, bytes: Vec<u8>) -> std::sync::Arc<ProjectSession> {
    let mut request = create(path); request.source = bytes;
    create_image_project(request, cancel()).await.unwrap()
}
fn output(root: &Path, name: &str, format: ImageFormat) -> FileExportOptions {
    let mut export = options(); export.format = format; export.marked = false;
    FileExportOptions { export, output_path: root.join(name).to_string_lossy().into(),
        work_directory: root.to_string_lossy().into(), max_encoded_bytes: 16 * 1024 * 1024,
        max_scratch_bytes: 400_000_000 }
}
fn no_scratch(root: &Path) {
    assert!(!std::fs::read_dir(root).unwrap().any(|entry|
        entry.unwrap().file_name().to_string_lossy().starts_with("vw-raster-")));
}
fn binding(info: &ProjectInfo) -> WorkflowBinding {
    WorkflowBinding { project_id: info.project_id.clone(), document_id: id(2), host_seq: info.host_seq, state_hash: info.state_hash.clone() }
}
fn temp() -> tempfile::TempDir {
    if cfg!(target_os = "android") { tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap() }
    else { tempfile::tempdir().unwrap() }
}

#[tokio::test]
async fn ordinary_memory_bmp_intake_preserves_immutable_encoded_original_and_profile() {
    let root = temp(); let (image, _) = fixture(8); let mut out = Cursor::new(Vec::new());
    vw_formats::encode_to(&image, vw_formats::ExportFormat::Bmp8, vw_formats::Limits::default(), &mut out, &|| false).unwrap();
    let bytes = out.into_inner(); let expected = AssetId::hash(&bytes);
    let path = root.path().join("project"); let project = memory_project(&path, bytes.clone()).await;
    let info = project.info().await.unwrap(); let format = project.original_format_info(binding(&info), cancel()).await.unwrap();
    assert_eq!(format.format, "bmp"); assert_eq!(format.source_asset_id, expected.to_string());
    assert!(!format.first_tiff_page_only); assert!(!format.untagged);
    let mut export = options(); export.marked = false; export.format = ImageFormat::Bmp8;
    let result = project.export_image(export, cancel()).await.unwrap();
    let decoded = vw_formats::decode(&result.bytes, vw_formats::Limits::default(), &|| false).unwrap().image;
    assert_eq!(decoded.pixels, image.pixels); assert_eq!(decoded.icc, image.icc);
    project.close().await.unwrap();
    let blobs = vw_store::BlobStore::new(&path).unwrap();
    assert_eq!(std::fs::read(blobs.path(&expected).unwrap()).unwrap(), bytes);
}

#[tokio::test]
async fn ordinary_memory_tiff16_export_keeps_samples_profile_and_revision() {
    let root = temp(); let (image, bytes) = fixture(16); let project = memory_project(&root.path().join("project"), bytes).await;
    let mut export = options(); export.marked = false; export.format = ImageFormat::Tiff16;
    let result = project.export_image(export, cancel()).await.unwrap();
    let decoded = vw_formats::decode(&result.bytes, vw_formats::Limits::default(), &|| false).unwrap().image;
    assert_eq!(decoded.pixels, image.pixels); assert_eq!(decoded.icc, image.icc);
    assert_eq!(result.blake3, AssetId::hash(&result.bytes).to_string());
    let metadata: vw_raster::ExportMetadata = serde_json::from_str(&result.metadata_json).unwrap();
    assert_eq!(metadata.settings.revision.host_seq, result.revision.host_seq);
    assert_eq!(metadata.settings.revision.state_hash, AssetId::try_from(result.revision.state_hash).unwrap().bytes().to_vec());
    assert_eq!(metadata.source_bit_depth, 16); assert!(!metadata.settings.allow_depth_reduction);
    project.close().await.unwrap();
}

#[tokio::test]
async fn multipage_tiff_memory_notice_precedes_display_consent_and_keeps_original_pages() {
    let root = temp(); let (_, mut bytes) = fixture(16); multipage(&mut bytes);
    let expected = AssetId::hash(&bytes); let path = root.path().join("project"); let project = memory_project(&path, bytes.clone()).await;
    let info = project.info().await.unwrap(); let notice = project.original_format_info(binding(&info), cancel()).await.unwrap();
    assert!(notice.first_tiff_page_only); assert_eq!(notice.source_bit_depth, 16); assert_eq!(notice.source_asset_id, expected.to_string());
    project.close().await.unwrap(); let blobs = vw_store::BlobStore::new(&path).unwrap();
    assert_eq!(std::fs::read(blobs.path(&expected).unwrap()).unwrap(), bytes);
}

#[tokio::test]
async fn file_tiff_intake_persists_first_page_notice_and_original_orientation() {
    let root = temp(); let (_, mut bytes) = fixture(8); tiff_scalar(&mut bytes, 274, 6); multipage(&mut bytes);
    let input = root.path().join("source.tiff"); std::fs::write(&input, &bytes).unwrap();
    let path = root.path().join("project"); let project = create_file_image_project(CreateFileImageProject {
        path: path.to_string_lossy().into(), project_id: id(1), document_id: id(2), layer_id: id(3), device_id: device(),
        title: "TIFF file fixture".into(), source_path: input.to_string_lossy().into(), work_directory: root.path().to_string_lossy().into(),
        now_ms: TIME, memory_budget_bytes: 256 * 1024 * 1024, max_encoded_bytes: 64 * 1024 * 1024, max_scratch_bytes: 400_000_000 }, cancel()).await.unwrap();
    let snapshot = project.document_snapshot(id(2), cancel()).await.unwrap(); assert_eq!((snapshot.width, snapshot.height), (3, 2));
    let info = project.info().await.unwrap(); assert!(project.original_format_info(binding(&info), cancel()).await.unwrap().first_tiff_page_only);
    project.close().await.unwrap(); let store = vw_store::ProjectStore::open(&path).unwrap(); let asset = store.project().assets.values().next().unwrap();
    assert_eq!(asset.format, "tiff"); assert_eq!(asset.orientation, 6); assert_eq!(asset.asset_id, AssetId::hash(&bytes).to_string());
    no_scratch(root.path());
}

#[tokio::test]
async fn ordinary_attachment_intake_retains_tiff_notice_depth_and_hash() {
    let root = temp(); let path = root.path().join("project"); let project = memory_project(&path, support::source()).await;
    let (_, mut bytes) = fixture(16); multipage(&mut bytes); let input = root.path().join("attachment.tif"); std::fs::write(&input, &bytes).unwrap();
    let info = project.info().await.unwrap(); let result = project.attach_asset_file(AttachFileOptions {
        transaction_id: id(9), device_id: device(), lamport: info.next_lamport, now_ms: TIME + 9,
        source_path: input.to_string_lossy().into(), work_directory: root.path().to_string_lossy().into(), kind: FileAssetKind::Image,
        memory_budget_bytes: 256 * 1024 * 1024, max_encoded_bytes: 64 * 1024 * 1024, max_scratch_bytes: 400_000_000 }, cancel()).await.unwrap();
    assert_eq!(result.format, "tiff"); assert_eq!(result.asset_id, AssetId::hash(&bytes).to_string());
    project.close().await.unwrap(); let store = vw_store::ProjectStore::open(&path).unwrap(); let asset = store.project().assets.get(&AssetId::hash(&bytes)).unwrap();
    assert_eq!(asset.bit_depth, 16); assert!(serde_json::from_str::<serde_json::Value>(&asset.metadata_json).unwrap()["first_tiff_page_only"].as_bool().unwrap());
    no_scratch(root.path());
}

#[tokio::test]
async fn finalized_file_hash_and_preflight_depth_match_exact_clean_pixels_for_all_formats() {
    let root = temp(); let (image, bytes) = fixture(8); let project = memory_project(&root.path().join("project"), bytes).await;
    for (name, format, depth) in [("clean.bmp", ImageFormat::Bmp8, 8), ("clean.tiff", ImageFormat::Tiff8, 8), ("clean16.tiff", ImageFormat::Tiff16, 16)] {
        let options = output(root.path(), name, format); let preflight = project.preflight_image_file(options.clone(), cancel()).await.unwrap();
        assert_eq!(preflight.output_bit_depth, depth); assert!(preflight.buffered);
        let result = project.export_transfer_file(options, cancel()).await.unwrap(); let output = std::fs::read(root.path().join(name)).unwrap();
        assert_eq!(result.encoded_bytes, output.len() as u64); assert_eq!(result.blake3, AssetId::hash(&output).to_string());
        let decoded = vw_formats::decode(&output, vw_formats::Limits::default(), &|| false).unwrap().image;
        assert_eq!(decoded.pixels.bit_depth(), depth); assert_eq!(decoded.icc, image.icc);
        fn expanded(pixels: &Pixels) -> Vec<u16> { match pixels {
            Pixels::Rgba8(values) => values.iter().map(|v| u16::from(*v) * 257).collect(),
            Pixels::Rgba16(values) => values.clone(),
        } }
        assert_eq!(expanded(&decoded.pixels), expanded(&image.pixels));
        let metadata: vw_raster::ExportMetadata = serde_json::from_str(&result.metadata_json).unwrap();
        assert_eq!(metadata.settings.revision.host_seq, result.revision.host_seq);
        assert_eq!(metadata.source_asset, image_asset(&project).await); no_scratch(root.path());
    }
    project.close().await.unwrap();
}
async fn image_asset(project: &ProjectSession) -> String {
    let info = project.info().await.unwrap(); project.original_format_info(binding(&info), cancel()).await.unwrap().source_asset_id
}

#[tokio::test]
async fn file_overwrite_and_cancellation_keep_existing_bytes_and_remove_owned_temporaries() {
    let root = temp(); let (_, bytes) = fixture(8); let project = memory_project(&root.path().join("project"), bytes).await;
    std::fs::write(root.path().join("existing.tiff"), b"unrelated existing destination").unwrap();
    assert!(project.export_transfer_file(output(root.path(), "existing.tiff", ImageFormat::Tiff8), cancel()).await.is_err());
    assert_eq!(std::fs::read(root.path().join("existing.tiff")).unwrap(), b"unrelated existing destination");
    let cancelled = cancel(); cancelled.cancel();
    assert!(project.export_transfer_file(output(root.path(), "cancelled.bmp", ImageFormat::Bmp8), cancelled).await.is_err());
    assert!(!root.path().join("cancelled.bmp").exists()); no_scratch(root.path()); project.close().await.unwrap();
}

#[tokio::test]
async fn original_format_query_refuses_stale_revision_cancellation_and_closed_project() {
    let root = temp(); let (_, bytes) = fixture(8); let project = memory_project(&root.path().join("project"), bytes).await;
    let info = project.info().await.unwrap(); let mut stale = binding(&info); stale.state_hash = "f".repeat(64);
    assert!(matches!(project.original_format_info(stale, cancel()).await, Err(WorkflowError::Stale)));
    let cancelled = cancel(); cancelled.cancel();
    assert!(matches!(project.original_format_info(binding(&info), cancelled).await, Err(WorkflowError::Cancelled)));
    project.close().await.unwrap(); assert!(matches!(project.original_format_info(binding(&info), cancel()).await, Err(WorkflowError::Closed)));
}
