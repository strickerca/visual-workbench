// Included in streaming_ffi::tests: real file/intake/export callers, no new owner.
fn composite_fixture(psb: bool) -> Vec<u8> {
    let mut bytes = b"8BPS".to_vec();
    bytes.extend((if psb { 2_u16 } else { 1 }).to_be_bytes());
    bytes.extend([0; 6]);
    bytes.extend(3_u16.to_be_bytes());
    bytes.extend(1_u32.to_be_bytes());
    bytes.extend(2_u32.to_be_bytes());
    bytes.extend(8_u16.to_be_bytes());
    bytes.extend(3_u16.to_be_bytes());
    bytes.extend([0; 8]);
    if psb {
        bytes.extend([0; 8]);
    } else {
        bytes.extend([0; 4]);
    }
    bytes.extend(0_u16.to_be_bytes());
    bytes.extend([10, 20, 30, 40, 50, 60]);
    bytes
}
fn preview_fixture() -> Vec<u8> {
    crate::queries::output_tests::raw_preview()
}

#[tokio::test]
async fn flattened_file_intake_retains_original_and_publishes_visible_notice() {
    for psb in [false, true] {
        let root = temp();
        let bytes = composite_fixture(psb);
        let source = root
            .path()
            .join(if psb { "source.psb" } else { "source.psd" });
        std::fs::write(&source, &bytes).unwrap();
        let options = creation(root.path(), &source);
        let document = options.document_id.clone();
        let path = options.path.clone();
        let project = create_file_image_project(options, Arc::new(Cancellation::new()))
            .await
            .unwrap();
        let revision = project.info().await.unwrap();
        let info = project
            .original_format_info(
                WorkflowBinding {
                    project_id: revision.project_id.clone(),
                    document_id: document.clone(),
                    host_seq: revision.host_seq,
                    state_hash: revision.state_hash.clone(),
                },
                Arc::new(Cancellation::new()),
            )
            .await
            .unwrap();
        assert_eq!(info.format, if psb { "psb" } else { "psd" });
        assert!(info.flattened_composite);
        assert!(!info.raw_preview);
        assert_eq!(info.source_asset_id, AssetId::hash(&bytes).to_string());
        let mut output = exporting(root.path(), document);
        output.export.format = ImageFormat::Bmp8;
        output.output_path = root
            .path()
            .join("flattened.bmp")
            .to_string_lossy()
            .into_owned();
        project
            .export_image_file(output, Arc::new(Cancellation::new()))
            .await
            .unwrap();
        assert_eq!(
            vw_store::BlobStore::new(Path::new(&path))
                .unwrap()
                .read(&AssetId::hash(&bytes))
                .unwrap(),
            bytes
        );
        assert_eq!(std::fs::read(source).unwrap(), bytes);
        project.close().await.unwrap();
        no_scratch(root.path());
    }
}

#[tokio::test]
async fn raw_file_intake_is_explicit_and_reopen_remains_read_only_preview() {
    let root = temp();
    let bytes = preview_fixture();
    let source = root.path().join("source.dng");
    std::fs::write(&source, &bytes).unwrap();
    let options = creation(root.path(), &source);
    let document = options.document_id.clone();
    let path = options.path.clone();
    assert!(
        create_file_image_project(options.clone(), Arc::new(Cancellation::new()))
            .await
            .is_err()
    );
    assert!(!Path::new(&path).exists());
    let project = create_file_raw_preview_project(options, Arc::new(Cancellation::new()))
        .await
        .unwrap();
    let revision = project.info().await.unwrap();
    let info = project
        .original_format_info(
            WorkflowBinding {
                project_id: revision.project_id,
                document_id: document.clone(),
                host_seq: revision.host_seq,
                state_hash: revision.state_hash,
            },
            Arc::new(Cancellation::new()),
        )
        .await
        .unwrap();
    assert!(info.raw_preview);
    assert!(!info.flattened_composite);
    assert_eq!(info.format, "dng");
    assert_eq!(info.source_asset_id, AssetId::hash(&bytes).to_string());
    let mut output = exporting(root.path(), document.clone());
    output.export.format = ImageFormat::Png8;
    assert!(matches!(
        project
            .preflight_image_file(output.clone(), Arc::new(Cancellation::new()))
            .await,
        Err(TransferError::Unsupported)
    ));
    assert!(
        project
            .export_image_file(output, Arc::new(Cancellation::new()))
            .await
            .is_err()
    );
    assert!(!root.path().join("output.png").exists());
    project.close().await.unwrap();
    let reopened = crate::open_project(path.clone(), Arc::new(Cancellation::new()))
        .await
        .unwrap();
    let display = reopened
        .background_image(document, MEMORY_LIMIT, true, Arc::new(Cancellation::new()))
        .await
        .unwrap();
    assert_eq!((display.width, display.height), (4, 6));
    assert_eq!(display.source_asset_id, AssetId::hash(&bytes).to_string());
    assert_eq!(
        vw_store::BlobStore::new(Path::new(&path))
            .unwrap()
            .read(&AssetId::hash(&bytes))
            .unwrap(),
        bytes
    );
    assert_eq!(std::fs::read(source).unwrap(), bytes);
    reopened.close().await.unwrap();
    no_scratch(root.path());
}

#[tokio::test]
async fn explicit_raw_route_refuses_an_ordinary_image_without_publication() {
    let root = temp();
    let source = root.path().join("pretend.dng");
    let bytes = image().0;
    std::fs::write(&source, &bytes).unwrap();
    let options = creation(root.path(), &source);
    let path = options.path.clone();
    assert!(
        create_file_raw_preview_project(options, Arc::new(Cancellation::new()))
            .await
            .is_err()
    );
    assert!(!Path::new(&path).exists());
    assert_eq!(std::fs::read(source).unwrap(), bytes);
    no_scratch(root.path());
}

#[tokio::test]
async fn seekable_output_routes_use_o_dimensions_embed_d_mapping_and_preserve_collision() {
    use crate::queries::output_tests as fixture;
    let root = temp();
    let bytes = fixture::original(vw_raster::ExportFormat::Png8);
    let mut model = fixture::model(&bytes, "png", "{}", 1);
    fixture::frame(&mut model, true);
    let project = fixture::session(&root.path().join("project"), model, &bytes);
    for (n, format) in [ImageFormat::Png8, ImageFormat::Tiff8, ImageFormat::Bmp8]
        .into_iter()
        .enumerate()
    {
        let mut options = exporting(root.path(), fixture::id(2).to_string());
        options.export.format = format;
        options.output_path = root
            .path()
            .join(format!("output-{n}.bin"))
            .to_string_lossy()
            .into_owned();
        let preflight = project
            .preflight_image_file(options.clone(), Arc::new(Cancellation::new()))
            .await
            .unwrap();
        assert_eq!((preflight.width, preflight.height), (2, 4));
        assert!(preflight.buffered);
        let result = project
            .export_image_file(options.clone(), Arc::new(Cancellation::new()))
            .await
            .unwrap();
        let delivered = std::fs::read(&options.output_path).unwrap();
        let metadata: vw_raster::ExportMetadata =
            serde_json::from_str(&result.metadata_json).unwrap();
        assert_eq!((metadata.source_width, metadata.source_height), (6, 4));
        assert_eq!((metadata.output_width, metadata.output_height), (2, 4));
        assert_eq!(metadata.source_asset, AssetId::hash(&bytes).to_string());
        assert_eq!(metadata.image_output.unwrap().clockwise_degrees, 90);
        if n == 2 {
            let embedded = vw_formats::bmp_provenance(&delivered, vw_formats::Limits::default())
                .unwrap()
                .unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(embedded).unwrap(),
                serde_json::from_str::<serde_json::Value>(&result.metadata_json).unwrap()
            );
        }
        let decoded =
            crate::format_images::decode(&delivered, vw_raster::DecodeLimits::default(), &|| false)
                .unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 4));
        assert!(
            project
                .export_image_file(options.clone(), Arc::new(Cancellation::new()))
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(options.output_path).unwrap(), delivered);
    }
    let mut bad = exporting(root.path(), fixture::id(2).to_string());
    bad.export.region = Some(QueryRect {
        x: 2.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    });
    assert!(
        project
            .preflight_image_file(bad, Arc::new(Cancellation::new()))
            .await
            .is_err()
    );
    project.close().await.unwrap();
    no_scratch(root.path());
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn ordinary_file_opener_refuses_fifo_without_waiting_for_a_writer() {
    let root = temp();
    let fifo = root.path().join("fifo.dng");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    assert!(open_input(&fifo, INPUT_LIMIT).is_err());
    assert!(read_bounded(&fifo, INPUT_LIMIT, &Cancellation::new()).is_err());
}

#[tokio::test]
async fn output_file_admits_budget_and_cancellation_before_reading_corrupt_original() {
    use crate::queries::output_tests as fixture;
    let root = temp();
    let bytes = fixture::original(vw_raster::ExportFormat::Png8);
    let mut model = fixture::model(&bytes, "png", "{}", 1);
    fixture::frame(&mut model, true);
    let path = root.path().join("project");
    let project = fixture::session(&path, model, &bytes);
    let revision = project.info().await.unwrap();
    let blob = vw_store::BlobStore::new(&path)
        .unwrap()
        .path(&AssetId::hash(&bytes))
        .unwrap();
    let corrupt = vec![0; bytes.len()];
    std::fs::write(&blob, &corrupt).unwrap();
    let mut limited = exporting(root.path(), fixture::id(2).to_string());
    limited.export.memory_budget_bytes = 16 * 1024 * 1024;
    assert!(matches!(
        project
            .export_transfer_file(limited, Arc::new(Cancellation::new()))
            .await,
        Err(TransferError::Memory { .. } | TransferError::Backpressure)
    ));
    let options = exporting(root.path(), fixture::id(2).to_string());
    let cancelled = Arc::new(Cancellation::new());
    cancelled.cancel();
    assert!(matches!(
        project.export_transfer_file(options, cancelled).await,
        Err(TransferError::Cancelled)
    ));
    assert!(!root.path().join("output.png").exists());
    assert_eq!(std::fs::read(blob).unwrap(), corrupt);
    assert_eq!(project.info().await.unwrap(), revision);
    project.close().await.unwrap();
    no_scratch(root.path());
}
