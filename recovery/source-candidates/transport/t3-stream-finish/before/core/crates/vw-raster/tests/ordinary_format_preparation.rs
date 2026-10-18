#![allow(clippy::unwrap_used, clippy::panic)]
use std::cell::Cell;
use vw_model::AssetId;
use vw_raster::*;

fn source(depth: u8) -> DecodedImage {
    DecodedImage { width: 2, height: 1, pixels: if depth == 16 {
        Pixels::Rgba16(vec![256, 257, 258, 0, 65534, 65535, 3, 32769])
    } else { Pixels::Rgba8(vec![1, 2, 3, 0, 254, 255, 1, 128]) },
        icc: Some(standard_srgb_profile().unwrap()), source_asset: AssetId::hash(b"unchanged encoded source"),
        original_available: true, orientation_applied: 6 }
}
fn request(format: ExportFormat) -> ExportRequest {
    ExportRequest { format, region: None, revision: vw_proto::v1::Revision { host_seq: 17, state_hash: vec![9; 32] },
        alpha: AlphaPolicy::Preserve, color: ColorPolicy::Preserve, allow_depth_reduction: false,
        memory_budget_bytes: 256 * 1024 * 1024, capture_session: None, frame_id: None }
}

#[test]
fn tiff16_preserves_adjacent_samples_and_original_provenance_without_permission() {
    let image = source(16);
    let prepared = prepare_export_pixels(&image, &request(ExportFormat::Tiff16), &|| false).unwrap();
    assert_eq!(prepared.image.pixels, image.pixels);
    assert_eq!(prepared.image.icc, image.icc);
    assert_eq!(prepared.metadata.source_bit_depth, 16);
    assert_eq!(prepared.metadata.orientation_applied, 6);
    assert_eq!(prepared.metadata.revision, "r17-09090909");
    assert_eq!(prepared.metadata.color_conversion, "none");
}

#[test]
fn eight_to_sixteen_promotion_is_exact_and_needs_no_depth_reduction_permission() {
    let image = source(8);
    let prepared = prepare_export_pixels(&image, &request(ExportFormat::Tiff16), &|| false).unwrap();
    let Pixels::Rgba16(samples) = prepared.image.pixels else { unreachable!() };
    let Pixels::Rgba8(original) = image.pixels else { unreachable!() };
    assert_eq!(samples, original.iter().map(|v| u16::from(*v) * 257).collect::<Vec<_>>());
    assert_eq!(prepared.metadata.source_bit_depth, 8);
}

#[test]
fn bmp_and_tiff8_refuse_silent_depth_reduction_and_record_explicit_permission() {
    let image = source(16);
    for format in [ExportFormat::Bmp8, ExportFormat::Tiff8] {
        let mut r = request(format);
        assert!(matches!(prepare_export_pixels(&image, &r, &|| false), Err(RasterError::Depth)));
        r.allow_depth_reduction = true;
        let prepared = prepare_export_pixels(&image, &r, &|| false).unwrap();
        assert_eq!(prepared.image.pixels, Pixels::Rgba8(vec![1, 1, 1, 0, 255, 255, 0, 128]));
        assert!(prepared.metadata.settings.allow_depth_reduction);
        assert_eq!(image.pixels.bit_depth(), 16);
    }
}

#[test]
fn gray_icc_can_label_tiff_but_bmp_requires_explicit_conversion() {
    let mut image = source(16);
    image.pixels = Pixels::Rgba16(vec![257, 257, 257, 65535, 40000, 40000, 40000, 0]);
    image.icc = Some(moxcms::ColorProfile::new_gray_with_gamma(2.2).encode().unwrap());
    assert!(prepare_export_pixels(&image, &request(ExportFormat::Tiff16), &|| false).is_ok());
    let mut r = request(ExportFormat::Bmp8); r.allow_depth_reduction = true;
    assert!(matches!(prepare_export_pixels(&image, &r, &|| false), Err(RasterError::Unsupported(_))));
    r.color = ColorPolicy::ConvertToSrgb { assume_untagged_srgb: false };
    let converted = prepare_export_pixels(&image, &r, &|| false).unwrap();
    assert_eq!(converted.metadata.color_conversion, "ICC to sRGB");
    assert_ne!(converted.image.icc, image.icc);
    assert_eq!(converted.metadata.source_icc_hash, Some(AssetId::hash(image.icc.as_ref().unwrap()).to_string()));
}

#[test]
fn format_limits_refuse_oversized_sides_and_preserve_exact_crop_without_scaling() {
    let mut r = request(ExportFormat::Tiff16);
    assert!(matches!(preflight(65536, 1, 16, false, &r), Err(RasterError::Dimensions { limit: 65535, .. })));
    r.region = Some(Region { x: 1, y: 0, width: 1, height: 1 });
    let prepared = prepare_export_pixels(&source(16), &r, &|| false).unwrap();
    assert_eq!((prepared.image.width, prepared.image.height), (1, 1));
    assert_eq!(prepared.image.pixels, Pixels::Rgba16(vec![65534, 65535, 3, 32769]));
    assert_eq!((prepared.metadata.source_width, prepared.metadata.output_width), (2, 1));
}

#[test]
fn cancellation_during_depth_promotion_does_not_modify_source() {
    let image = source(8);
    let original = image.pixels.clone();
    let calls = Cell::new(0);
    let result = prepare_export_pixels(&image, &request(ExportFormat::Tiff16), &|| {
        calls.set(calls.get() + 1); calls.get() >= 2
    });
    assert!(matches!(result, Err(RasterError::Cancelled)));
    assert_eq!(image.pixels, original);
}

#[test]
fn missing_original_or_insufficient_workspace_refuses_before_preparation() {
    let mut image = source(8); image.original_available = false;
    assert!(matches!(prepare_export_pixels(&image, &request(ExportFormat::Bmp8), &|| false), Err(RasterError::OriginalRequired)));
    image.original_available = true;
    let mut r = request(ExportFormat::Bmp8); r.memory_budget_bytes = 64 * 1024 * 1024;
    assert!(matches!(prepare_export_pixels(&image, &r, &|| false), Err(RasterError::Memory { .. })));
}

#[test]
fn raster_memory_encoder_refuses_external_formats_and_existing_png_still_roundtrips() {
    let image = source(16);
    assert!(matches!(export(&image, &request(ExportFormat::Tiff16)), Err(RasterError::Unsupported(_))));
    let png = export(&image, &request(ExportFormat::Png16)).unwrap();
    let decoded = decode(&png.bytes, DecodeLimits::default()).unwrap();
    assert_eq!(decoded.pixels, image.pixels);
    assert_eq!(decoded.icc, image.icc);
}
