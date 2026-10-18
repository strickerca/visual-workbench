// Included in format_images::tests to exercise the actual codec adapter.
#[test]
fn bmp_staging_embeds_requested_policy_and_output_mapping_before_return() {
    let source = image();
    let mut policy = request();
    policy.format = ExportFormat::Bmp8;
    let frame = vw_raster::ImageOutputMetadata::new(
        vw_geom::ImageOutputFrame::new([4, 2], [1, 0, 2, 1], vw_geom::QuarterTurn::Zero, [2, 1])
            .unwrap(),
    );
    for output in [None, Some(frame)] {
        let mut file = tempfile::tempfile().unwrap();
        let (length, metadata) = encode_private_in_budget(
            &source,
            &policy,
            output,
            8 * MIB,
            &mut file,
            &Cancellation::new(),
            192 * MIB,
        )
        .unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes.len() as u64, length);
        let embedded = vw_formats::bmp_provenance(&bytes, vw_formats::Limits::default())
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(embedded).unwrap(),
            serde_json::to_value(&metadata).unwrap()
        );
        assert_eq!(metadata.settings.memory_budget_bytes, 256 * MIB);
        assert_eq!(metadata.source_asset, source.source_asset.to_string());
        assert_eq!(metadata.image_output, output);
        assert_eq!(
            [metadata.source_width, metadata.source_height],
            if output.is_some() { [4, 2] } else { [2, 1] }
        );
        assert_eq!([metadata.output_width, metadata.output_height], [2, 1]);
        let decoded = vw_formats::decode(&bytes, vw_formats::Limits::default(), &|| false).unwrap();
        assert_eq!(decoded.image.pixels, source.pixels);
    }
}

#[test]
fn private_encoder_refuses_preview_cancel_and_cap_before_writing() {
    let mut source = image();
    let mut policy = request();
    policy.format = ExportFormat::Bmp8;
    let mut file = tempfile::tempfile().unwrap();
    source.original_available = false;
    assert!(matches!(
        encode_private(&source, &policy, 8 * MIB, &mut file, &Cancellation::new()),
        Err(RasterError::OriginalRequired)
    ));
    source.original_available = true;
    let cancelled = Cancellation::new();
    cancelled.cancel();
    assert!(matches!(
        encode_private(&source, &policy, 8 * MIB, &mut file, &cancelled),
        Err(RasterError::Cancelled)
    ));
    assert!(matches!(
        encode_private(&source, &policy, 1, &mut file, &Cancellation::new()),
        Err(RasterError::EncodedLimit { .. })
    ));
    assert_eq!(file.metadata().unwrap().len(), 0);
}

#[test]
fn persistent_raw_preview_declaration_requires_exact_flags_and_depth() {
    let mut asset = vw_proto::v1::AddAsset {
        format: "dng".into(),
        bit_depth: 8,
        metadata_json: r#"{"raw_preview":true}"#.into(),
        ..Default::default()
    };
    assert!(asset_is_raw_preview(&asset).unwrap());
    for invalid in [
        "{}",
        r#"{"raw_preview":false}"#,
        r#"{"raw_preview":"true"}"#,
        r#"{"raw_preview":true,"flattened_composite":true}"#,
        r#"{"raw_preview":true,"first_tiff_page_only":true}"#,
    ] {
        asset.metadata_json = invalid.into();
        assert!(asset_is_raw_preview(&asset).is_err());
    }
    asset.metadata_json = r#"{"raw_preview":true}"#.into();
    asset.bit_depth = 16;
    assert!(asset_is_raw_preview(&asset).is_err());
    asset.bit_depth = 8;
    asset.metadata_json = " ".repeat(4097);
    assert!(asset_is_raw_preview(&asset).is_err());
    asset.format = "png".into();
    assert!(!asset_is_raw_preview(&asset).unwrap());
}

#[test]
fn reopening_ordinary_asset_checks_identity_grid_depth_orientation_and_profile() {
    let mut policy = request();
    policy.format = ExportFormat::Png8;
    let bytes = vw_raster::export(&image(), &policy).unwrap().bytes;
    let asset = vw_proto::v1::AddAsset {
        asset_id: AssetId::hash(&bytes).to_string(),
        format: "png".into(),
        width: 2,
        height: 1,
        bit_depth: 8,
        orientation: 1,
        metadata_json: "{}".into(),
        ..Default::default()
    };
    assert!(decode_asset(&bytes, DecodeLimits::default(), &|| false, &asset).is_ok());
    let mut wrong = asset.clone();
    wrong.asset_id = AssetId::hash(b"different original").to_string();
    assert!(decode_asset(&bytes, DecodeLimits::default(), &|| false, &wrong).is_err());
    wrong = asset.clone();
    wrong.width = 1;
    assert!(decode_asset(&bytes, DecodeLimits::default(), &|| false, &wrong).is_err());
    wrong = asset.clone();
    wrong.bit_depth = 16;
    assert!(decode_asset(&bytes, DecodeLimits::default(), &|| false, &wrong).is_err());
    wrong = asset.clone();
    wrong.orientation = 2;
    assert!(decode_asset(&bytes, DecodeLimits::default(), &|| false, &wrong).is_err());
    wrong = asset;
    wrong.icc_profile = vec![1];
    assert!(decode_asset(&bytes, DecodeLimits::default(), &|| false, &wrong).is_err());
}
