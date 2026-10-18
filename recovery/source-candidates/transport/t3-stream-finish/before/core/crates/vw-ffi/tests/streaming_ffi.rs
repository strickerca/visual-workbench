//! File publication and ordinary-asset tests. MP4 fixtures exercise only the
//! admitted container envelope; they are not codec/playback evidence.
#![allow(clippy::unwrap_used)]
mod support;
use std::{path::Path, sync::Arc};
use support::*;
use vw_core::*;
use vw_model::{AssetId, Id};

fn temp() -> tempfile::TempDir {
    if cfg!(target_os = "android") {
        tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()
    } else {
        tempfile::tempdir().unwrap()
    }
}

fn output(root: &Path, name: &str, format: ImageFormat) -> FileExportOptions {
    let mut export = options();
    export.format = format;
    export.marked = false;
    FileExportOptions {
        export,
        output_path: root.join(name).to_string_lossy().into(),
        work_directory: root.to_string_lossy().into(),
        max_encoded_bytes: 8 * 1024 * 1024,
        max_scratch_bytes: 400_000_000,
    }
}
fn no_scratch(root: &Path) {
    assert!(!std::fs::read_dir(root).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("vw-raster-")
    }));
}
fn attachment(
    root: &Path,
    path: &Path,
    kind: FileAssetKind,
    n: u8,
    lamport: u64,
) -> AttachFileOptions {
    AttachFileOptions {
        transaction_id: id(n),
        device_id: device(),
        lamport,
        now_ms: TIME + i64::from(n),
        source_path: path.to_string_lossy().into(),
        work_directory: root.to_string_lossy().into(),
        kind,
        memory_budget_bytes: 256 * 1024 * 1024,
        max_encoded_bytes: 64 * 1024 * 1024,
        max_scratch_bytes: 400_000_000,
    }
}
fn mp4() -> Vec<u8> {
    let mut bytes = Vec::new();
    for (tag, data) in [
        (b"ftyp", b"isom\0\0\0\0isommp42".as_slice()),
        (b"moov", b"fixture container only".as_slice()),
        (b"mdat", b"opaque synthetic payload".as_slice()),
    ] {
        bytes.extend(((data.len() + 8) as u32).to_be_bytes());
        bytes.extend(tag);
        bytes.extend(data);
    }
    bytes
}

fn embedded_export_metadata(encoded: &[u8]) -> serde_json::Value {
    let xmp = if encoded.starts_with(&[0xff, 0xd8]) {
        assert_eq!(&encoded[2..4], &[0xff, 0xe1]);
        let length = u16::from_be_bytes(encoded[4..6].try_into().unwrap()) as usize;
        let payload = &encoded[6..4 + length];
        payload
            .strip_prefix(b"http://ns.adobe.com/xap/1.0/\0")
            .unwrap()
    } else {
        assert_eq!(&encoded[..4], b"RIFF");
        assert_eq!(&encoded[8..12], b"WEBP");
        assert_eq!(
            u32::from_le_bytes(encoded[4..8].try_into().unwrap()) as usize + 8,
            encoded.len()
        );
        let mut offset = 12;
        let mut found = None;
        while offset < encoded.len() {
            let header = &encoded[offset..offset + 8];
            let length = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
            let payload = &encoded[offset + 8..offset + 8 + length];
            if &header[..4] == b"XMP " {
                assert!(found.replace(payload).is_none(), "duplicate XMP chunk");
            }
            offset += 8 + length + length % 2;
        }
        assert_eq!(offset, encoded.len());
        found.unwrap()
    };
    let xml = std::str::from_utf8(xmp).unwrap();
    let (_, body) = xml.split_once("<vwb:Metadata>").unwrap();
    let (json, _) = body.split_once("</vwb:Metadata>").unwrap();
    // Decode the exporter's XML character data, not a substring of binary data.
    // Decode ampersand last so literal entity-like text remains literal.
    let json = json
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    serde_json::from_str(&json).unwrap()
}

#[tokio::test]
async fn buffered_jpeg_and_webp_keep_revision_metadata_and_no_clobber_publication() {
    let root = temp();
    let project = create_image_project(create(&root.path().join("project")), cancel())
        .await
        .unwrap();
    let revision = project.info().await.unwrap();
    for (name, format) in [
        ("clean.jpg", ImageFormat::Jpeg { quality: 92 }),
        ("clean.webp", ImageFormat::WebpLossless),
        ("lossy.webp", ImageFormat::WebpLossy { quality: 83 }),
    ] {
        let request = output(root.path(), name, format);
        let plan = project
            .preflight_image_file(request.clone(), cancel())
            .await
            .unwrap();
        assert!(plan.buffered);
        assert_eq!((plan.width, plan.height), (64, 64));
        let receipt = project
            .export_transfer_file(request.clone(), cancel())
            .await
            .unwrap();
        let encoded = std::fs::read(root.path().join(name)).unwrap();
        assert_eq!(receipt.revision, revision);
        assert_eq!(receipt.blake3, AssetId::hash(&encoded).to_string());
        assert_eq!(receipt.encoded_bytes, encoded.len() as u64);
        let metadata: serde_json::Value = serde_json::from_str(&receipt.metadata_json).unwrap();
        assert_eq!(
            metadata["source_asset"],
            AssetId::hash(&source()).to_string()
        );
        assert_eq!(metadata["output_width"], 64);
        let embedded = embedded_export_metadata(&encoded);
        assert_eq!(embedded, metadata, "container and receipt metadata differ");
        let stored: vw_raster::ExportMetadata = serde_json::from_value(embedded).unwrap();
        assert_eq!(stored.settings.revision.host_seq, revision.host_seq);
        assert_eq!(stored.settings.revision.state_hash.len(), 32);
        let full_hash: String = stored
            .settings
            .revision
            .state_hash
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(full_hash, revision.state_hash);
        assert!(
            project
                .export_transfer_file(request, cancel())
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(root.path().join(name)).unwrap(), encoded);
        no_scratch(root.path());
    }
    project.close().await.unwrap();
}

#[tokio::test]
async fn typed_memory_encoded_and_cancel_failures_publish_nothing() {
    let root = temp();
    let project = create_image_project(create(&root.path().join("project")), cancel())
        .await
        .unwrap();
    let mut request = output(root.path(), "bounded.webp", ImageFormat::WebpLossless);
    request.export.memory_budget_bytes = 1024;
    assert!(matches!(
        project.preflight_image_file(request, cancel()).await,
        Err(TransferError::Memory { budget: 1024, .. })
    ));
    let mut request = output(root.path(), "bounded.webp", ImageFormat::WebpLossless);
    request.max_encoded_bytes = 1;
    assert!(matches!(
        project.export_transfer_file(request, cancel()).await,
        Err(TransferError::EncodedLimit)
    ));
    assert!(!root.path().join("bounded.webp").exists());
    no_scratch(root.path());
    let token = Arc::new(Cancellation::new());
    token.cancel();
    assert!(matches!(
        project
            .export_transfer_file(
                output(root.path(), "cancel.jpg", ImageFormat::Jpeg { quality: 90 }),
                token
            )
            .await,
        Err(TransferError::Cancelled)
    ));
    assert!(!root.path().join("cancel.jpg").exists());
    no_scratch(root.path());
    project.close().await.unwrap();
}

#[tokio::test]
async fn file_attachments_are_exact_retryable_immutable_assets_without_documents() {
    let root = temp();
    let path = root.path().join("project");
    let project = create_image_project(create(&path), cancel()).await.unwrap();
    let before = project.info().await.unwrap();
    let video = root.path().join("ordinary.mp4");
    let bytes = mp4();
    std::fs::write(&video, &bytes).unwrap();
    let request = attachment(
        root.path(),
        &video,
        FileAssetKind::Mp4,
        80,
        before.next_lamport,
    );
    let attached = project
        .attach_asset_file(request.clone(), cancel())
        .await
        .unwrap();
    assert_eq!(attached.asset_id, AssetId::hash(&bytes).to_string());
    assert_eq!(attached.format, "mp4");
    assert_eq!(attached.revision.host_seq, before.host_seq + 1);
    assert_eq!(attached.revision.document_ids, before.document_ids);
    let retry = project
        .attach_asset_file(request.clone(), cancel())
        .await
        .unwrap();
    assert_eq!(retry.revision, attached.revision);
    std::fs::write(&video, mp4().into_iter().chain([0]).collect::<Vec<_>>()).unwrap();
    assert!(project.attach_asset_file(request, cancel()).await.is_err());
    assert_eq!(project.info().await.unwrap(), attached.revision);
    let image = root.path().join("ordinary.png");
    let original = source();
    std::fs::write(&image, &original).unwrap();
    let receipt = project
        .attach_asset_file(
            attachment(
                root.path(),
                &image,
                FileAssetKind::Image,
                81,
                attached.revision.next_lamport,
            ),
            cancel(),
        )
        .await
        .unwrap();
    assert_eq!(receipt.asset_id, AssetId::hash(&original).to_string());
    assert_eq!(receipt.revision.document_ids, before.document_ids);
    no_scratch(root.path());
    project.close().await.unwrap();
    let store = vw_store::ProjectStore::open(&path).unwrap();
    let video_id = AssetId::try_from(attached.asset_id).unwrap();
    assert_eq!(store.project().assets[&video_id].format, "mp4");
    assert_eq!(
        vw_store::BlobStore::new(&path)
            .unwrap()
            .read(&video_id)
            .unwrap(),
        bytes
    );
    assert_eq!(
        store
            .accepted_transactions()
            .filter(|(t, _)| t.txn_id == Some(Id::try_from(id(80)).unwrap().to_proto()))
            .count(),
        1
    );
}

#[tokio::test]
async fn malformed_video_and_cancelled_attachment_leave_history_unchanged() {
    let root = temp();
    let project = create_image_project(create(&root.path().join("project")), cancel())
        .await
        .unwrap();
    let before = project.info().await.unwrap();
    let path = root.path().join("bad.mp4");
    let mut bytes = mp4();
    bytes[..4].copy_from_slice(&u32::MAX.to_be_bytes());
    std::fs::write(&path, bytes).unwrap();
    assert!(matches!(
        project
            .attach_asset_file(
                attachment(
                    root.path(),
                    &path,
                    FileAssetKind::Mp4,
                    90,
                    before.next_lamport
                ),
                cancel()
            )
            .await,
        Err(TransferError::Metadata)
    ));
    assert_eq!(project.info().await.unwrap(), before);
    no_scratch(root.path());
    std::fs::write(&path, mp4()).unwrap();
    let token = Arc::new(Cancellation::new());
    token.cancel();
    assert!(matches!(
        project
            .attach_asset_file(
                attachment(
                    root.path(),
                    &path,
                    FileAssetKind::Mp4,
                    90,
                    before.next_lamport
                ),
                token
            )
            .await,
        Err(TransferError::Cancelled)
    ));
    assert_eq!(project.info().await.unwrap(), before);
    no_scratch(root.path());
    project.close().await.unwrap();
}

#[tokio::test]
async fn typed_format_and_source_binding_failures_precede_publication() {
    let root = temp();
    let first = root.path().join("first");
    let project = create_image_project(create(&first), cancel())
        .await
        .unwrap();
    project.close().await.unwrap();
    drop(project);
    let original = vw_store::ProjectStore::open(&first).unwrap();
    let mut wide = original.project().clone();
    let asset = AssetId::hash(&source());
    wide.assets.get_mut(&asset).unwrap().width = 20000;
    let device = original.host_device().clone();
    drop(original);
    let wide_path = root.path().join("wide");
    let store = vw_store::ProjectStore::create(&wide_path, wide, device, TIME).unwrap();
    vw_store::BlobStore::new(&wide_path)
        .unwrap()
        .put(&source())
        .unwrap();
    drop(store);
    let project = open_project(wide_path.to_string_lossy().into(), cancel())
        .await
        .unwrap();
    assert!(matches!(
        project
            .preflight_image_file(
                output(root.path(), "wide.webp", ImageFormat::WebpLossless),
                cancel()
            )
            .await,
        Err(TransferError::Dimensions {
            width: 20000,
            limit: 16383,
            ..
        })
    ));
    // PNG admits the dimensions, but verified preflight refuses metadata that
    // does not describe the immutable original's real oriented pixel grid.
    assert!(matches!(
        project
            .preflight_image_file(output(root.path(), "wide.png", ImageFormat::Png8), cancel())
            .await,
        Err(TransferError::Metadata)
    ));
    assert!(!root.path().join("wide.webp").exists());
    assert!(!root.path().join("wide.png").exists());
    no_scratch(root.path());
    project.close().await.unwrap();
}

#[tokio::test]
async fn depth_and_alpha_reductions_require_explicit_export_choices() {
    use vw_raster::{AlphaPolicy, ColorPolicy, DecodedImage, ExportFormat, ExportRequest, Pixels};
    let root = temp();
    let image = DecodedImage {
        width: 2,
        height: 2,
        pixels: Pixels::Rgba16(vec![
            12345, 23456, 34567, 0, 12345, 23456, 34567, 65535, 12345, 23456, 34567, 65535, 12345,
            23456, 34567, 65535,
        ]),
        icc: None,
        source_asset: AssetId::hash(b"16-bit typed transfer"),
        original_available: true,
        orientation_applied: 1,
    };
    let source = vw_raster::export(
        &image,
        &ExportRequest {
            format: ExportFormat::Png16,
            region: None,
            revision: vw_proto::v1::Revision {
                host_seq: 0,
                state_hash: vec![0; 32],
            },
            alpha: AlphaPolicy::Preserve,
            color: ColorPolicy::Preserve,
            allow_depth_reduction: false,
            memory_budget_bytes: 256 * 1024 * 1024,
            capture_session: None,
            frame_id: None,
        },
    )
    .unwrap()
    .bytes;
    let mut creation = create(&root.path().join("project"));
    creation.source = source;
    let project = create_image_project(creation, cancel()).await.unwrap();
    let mut request = output(
        root.path(),
        "explicit.jpg",
        ImageFormat::Jpeg { quality: 90 },
    );
    assert!(matches!(
        project
            .preflight_image_file(request.clone(), cancel())
            .await,
        Err(TransferError::Depth)
    ));
    request.export.allow_depth_reduction = true;
    assert!(matches!(
        project
            .preflight_image_file(request.clone(), cancel())
            .await,
        Err(TransferError::Alpha)
    ));
    request.export.matte_rgb = Some(0xffffff);
    request.export.convert_to_srgb = true;
    request.export.assume_untagged_srgb = false;
    assert!(matches!(
        project
            .preflight_image_file(request.clone(), cancel())
            .await,
        Err(TransferError::Metadata)
    ));
    request.export.assume_untagged_srgb = true;
    let exported = project
        .export_transfer_file(request, cancel())
        .await
        .unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&exported.metadata_json).unwrap();
    assert_eq!(metadata["source_bit_depth"], 16);
    assert_eq!(metadata["settings"]["allow_depth_reduction"], true);
    assert_ne!(metadata["color_conversion"], "none");
    no_scratch(root.path());
    project.close().await.unwrap();
}
