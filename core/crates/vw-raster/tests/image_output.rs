#![allow(clippy::unwrap_used, clippy::expect_used)]
use vw_geom::{ImageOutputFrame, QuarterTurn};
use vw_model::AssetId;
use vw_raster::*;
const BUDGET: u64 = 256 * 1024 * 1024;
fn source(width: u32, height: u32, depth: u8) -> DecodedImage {
    let samples: Vec<u16> = (0..width * height)
        .flat_map(|i| {
            [
                i as u16 * 113,
                (i as u16 + 1) * 257,
                65535 - i as u16 * 71,
                if i % 3 == 0 { 0 } else { 32768 },
            ]
        })
        .collect();
    DecodedImage {
        width,
        height,
        pixels: if depth == 16 {
            Pixels::Rgba16(samples)
        } else {
            Pixels::Rgba8(samples.iter().map(|v| (v / 257) as u8).collect())
        },
        icc: Some(standard_srgb_profile().unwrap()),
        source_asset: AssetId::hash(b"synthetic-original"),
        original_available: true,
        orientation_applied: 6,
    }
}
fn pixel(pixels: &Pixels, at: usize) -> [u16; 4] {
    match pixels {
        Pixels::Rgba8(v) => [
            u16::from(v[at]) * 257,
            u16::from(v[at + 1]) * 257,
            u16::from(v[at + 2]) * 257,
            u16::from(v[at + 3]) * 257,
        ],
        Pixels::Rgba16(v) => [v[at], v[at + 1], v[at + 2], v[at + 3]],
    }
}
#[test]
fn exact_crop_and_every_quarter_turn_preserve_depth_alpha_hidden_rgb_profile_and_identity() {
    for depth in [8, 16] {
        for rotation in [
            QuarterTurn::Zero,
            QuarterTurn::Clockwise90,
            QuarterTurn::Half,
            QuarterTurn::Clockwise270,
        ] {
            let original = source(5, 4, depth);
            let before = original.clone();
            let size = if matches!(rotation, QuarterTurn::Zero | QuarterTurn::Half) {
                [3, 2]
            } else {
                [2, 3]
            };
            let frame = ImageOutputFrame::new([5, 4], [1, 1, 3, 2], rotation, size).unwrap();
            let result = render_image_output(&original, frame, BUDGET, &|| false).unwrap();
            assert_eq!((result.width, result.height), (size[0], size[1]));
            assert_eq!(result.pixels.bit_depth(), depth);
            for y in 0..size[1] {
                for x in 0..size[0] {
                    let [sx, sy] = frame.source_pixel(x, y).unwrap();
                    assert_eq!(
                        pixel(&result.pixels, ((y * size[0] + x) * 4) as usize),
                        pixel(&original.pixels, ((sy * 5 + sx) * 4) as usize)
                    );
                }
            }
            assert_eq!(result.icc, original.icc);
            assert_eq!(result.source_asset, original.source_asset);
            assert_eq!(result.orientation_applied, 6);
            assert_eq!(original, before);
        }
    }
}
#[test]
fn source_sample_bilinear_uses_premultiplied_alpha_and_ignores_hidden_color() {
    let mut original = source(2, 1, 8);
    original.pixels = Pixels::Rgba8(vec![255, 0, 0, 0, 0, 0, 255, 255]);
    let frame = ImageOutputFrame::new([2, 1], [0, 0, 2, 1], QuarterTurn::Zero, [3, 1]).unwrap();
    let result = render_image_output(&original, frame, BUDGET, &|| false).unwrap();
    assert_eq!(
        result.pixels,
        Pixels::Rgba8(vec![255, 0, 0, 0, 0, 0, 255, 128, 0, 0, 255, 255])
    );
    // Exact center downsampling is explicitly bilinear, not an area average.
    let one = ImageOutputFrame::new([2, 1], [0, 0, 2, 1], QuarterTurn::Zero, [1, 1]).unwrap();
    assert_eq!(
        render_image_output(&original, one, BUDGET, &|| false)
            .unwrap()
            .pixels,
        Pixels::Rgba8(vec![0, 0, 255, 128])
    );
}
#[test]
fn sixteen_bit_resizing_does_not_quantize_adjacent_source_values_to_eight_bits() {
    let mut original = source(2, 1, 16);
    original.pixels = Pixels::Rgba16(vec![1000, 2000, 3000, 65535, 1002, 2002, 3002, 65535]);
    let frame = ImageOutputFrame::new([2, 1], [0, 0, 2, 1], QuarterTurn::Zero, [1, 1]).unwrap();
    assert_eq!(
        render_image_output(&original, frame, BUDGET, &|| false)
            .unwrap()
            .pixels,
        Pixels::Rgba16(vec![1001, 2001, 3001, 65535])
    );
}
#[test]
fn every_region_uses_global_output_pixel_centers_and_stitches_exactly() {
    let original = source(7, 5, 16);
    let frame =
        ImageOutputFrame::new([7, 5], [1, 1, 5, 3], QuarterTurn::Clockwise270, [11, 13]).unwrap();
    let complete = render_image_output(&original, frame, BUDGET, &|| false).unwrap();
    for y in 0..13 {
        for x in 0..11 {
            let part = render_image_output_region(
                &original,
                frame,
                Region {
                    x,
                    y,
                    width: 1,
                    height: 1,
                },
                BUDGET,
                &|| false,
            )
            .unwrap();
            assert_eq!(
                pixel(&part.pixels, 0),
                pixel(&complete.pixels, ((y * 11 + x) * 4) as usize)
            );
        }
    }
}
#[test]
fn retained_capacity_profile_workspace_invalid_source_and_cancellation_refuse_without_output() {
    let mut original = source(2, 2, 8);
    let frame = ImageOutputFrame::identity(2, 2).unwrap();
    let region = Region {
        x: 0,
        y: 0,
        width: 2,
        height: 2,
    };
    let before = image_output_estimate(&original, frame, region).unwrap();
    if let Pixels::Rgba8(bytes) = &mut original.pixels {
        bytes.reserve_exact(4096);
    }
    let after = image_output_estimate(&original, frame, region).unwrap();
    assert!(after >= before + 4096);
    assert!(matches!(
        render_image_output(&original, frame, after - 1, &|| false),
        Err(RasterError::Memory { .. })
    ));
    assert!(matches!(
        render_image_output(&original, frame, BUDGET, &|| true),
        Err(RasterError::Cancelled)
    ));
    original.original_available = false;
    assert!(matches!(
        render_image_output(&original, frame, BUDGET, &|| false),
        Err(RasterError::OriginalRequired)
    ));
}
#[test]
fn output_metadata_roundtrips_exact_geometry_and_refuses_unknown_policy_version() {
    let frame = ImageOutputFrame::new([9, 8], [2, 3, 4, 5], QuarterTurn::Half, [7, 11]).unwrap();
    let metadata = ImageOutputMetadata::new(frame);
    assert_eq!(metadata.frame().unwrap(), frame);
    let bytes = serde_json::to_vec(&metadata).unwrap();
    let copy: ImageOutputMetadata = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(copy, metadata);
    assert!(
        ImageOutputMetadata {
            version: 2,
            ..metadata
        }
        .frame()
        .is_err()
    );
    assert!(
        ImageOutputMetadata {
            clockwise_degrees: 360,
            ..metadata
        }
        .frame()
        .is_err()
    );
}

mod support;
mod document_output {
    use super::support::*;
    use vw_geom::{ImageOutputFrame, QuarterTurn};
    use vw_model::{Id, Layer, Project};
    use vw_proto::v1::{self as pb, object_state::Shape};
    use vw_raster::*;

    fn output(project: &mut Project, document: &Id, frame: ImageOutputFrame) -> TestResult {
        let layer = id(70)?;
        project.layers.insert(
            layer.clone(),
            Layer {
                definition: pb::CreateLayer {
                    layer_id: Some(layer.to_proto()),
                    document_id: Some(document.to_proto()),
                    page_index: -1,
                    name: "Explicit output".into(),
                    kind: "output".into(),
                    order_key: "z".into(),
                },
                visible: true,
                locked: false,
                opacity: 1.0,
                blend: "normal".into(),
            },
        );
        let [x, y, w, h] = frame.crop();
        let [a, b, c, d, e, f] = frame.matrix()?.coefficients();
        let (key, mut value) = object(
            document,
            &layer,
            71,
            Shape::Crop(rect(x.into(), y.into(), w.into(), h.into())),
        )?;
        value.state.transform = Some(pb::Affine { a, b, c, d, e, f });
        project.objects.insert(key, value);
        project.validate()?;
        Ok(())
    }
    fn tone(project: &mut Project, document: &Id) -> TestResult {
        let layer = id(72)?;
        project.layers.insert(
            layer.clone(),
            Layer {
                definition: pb::CreateLayer {
                    layer_id: Some(layer.to_proto()),
                    document_id: Some(document.to_proto()),
                    page_index: -1,
                    name: "Levels".into(),
                    kind: "adjustment".into(),
                    order_key: "Y".into(),
                },
                visible: true,
                locked: false,
                opacity: 1.0,
                blend: "normal".into(),
            },
        );
        let (key, value) = object(
            document,
            &layer,
            73,
            Shape::Adjustment(pb::Adjustment {
                brightness: 0.1,
                contrast: 0.0,
                levels: vec![0.0, 1.0, 1.0, 0.0, 1.0],
            }),
        )?;
        project.objects.insert(key, value);
        project.validate()?;
        Ok(())
    }
    #[test]
    fn sixteen_bit_alpha_output_preview_and_encoded_export_use_exactly_one_mapping() -> TestResult {
        let original = source(8, 6, 16);
        let original_hash = original.source_asset.clone();
        let (mut project, doc, _) = project(&original)?;
        let frame = ImageOutputFrame::new([8, 6], [2, 1, 4, 3], QuarterTurn::Clockwise90, [3, 4])?;
        output(&mut project, &doc, frame)?;
        let direct = render_image_output(&original, frame, 256 * 1024 * 1024, &|| false)?;
        let mut opts = options();
        opts.include_guides = true;
        let actual =
            prepare_document_output(&original, &project, &doc, &NoAssets, opts, true, &|| false)?;
        assert_pixels_equal(
            &actual.image.pixels,
            &direct.pixels,
            "output layer is not painted as a crop guide",
        )?;
        let mut request = request(ExportFormat::Png16);
        request.memory_budget_bytes = opts.memory_budget_bytes;
        let encoded = export_document_output(
            &original,
            &project,
            &doc,
            &NoAssets,
            opts,
            true,
            &request,
            &|| false,
        )?;
        let decoded = decode(&encoded.bytes, DecodeLimits::default())?;
        assert_pixels_equal(
            &decoded.pixels,
            &direct.pixels,
            "encoded output uses the same exact quarter turn",
        )?;
        assert_eq!((decoded.width, decoded.height), (3, 4));
        assert_eq!(encoded.metadata.source_width, 8);
        assert_eq!(encoded.metadata.source_height, 6);
        assert_eq!(
            encoded.metadata.image_output,
            Some(ImageOutputMetadata::new(frame))
        );
        assert_eq!(original.source_asset, original_hash);
        assert_pixels_equal(
            &original.pixels,
            &source(8, 6, 16).pixels,
            "immutable original",
        )?;
        Ok(())
    }
    #[test]
    fn region_coordinates_are_in_output_grid_and_metadata_keeps_original_grid() -> TestResult {
        let original = source(8, 6, 8);
        let (mut project, doc, _) = project(&original)?;
        let frame = ImageOutputFrame::new([8, 6], [2, 1, 4, 3], QuarterTurn::Clockwise90, [6, 8])?;
        output(&mut project, &doc, frame)?;
        let region = Region {
            x: 2,
            y: 3,
            width: 3,
            height: 4,
        };
        let expected =
            render_image_output_region(&original, frame, region, 256 * 1024 * 1024, &|| false)?;
        let opts = options();
        let mut request = request(ExportFormat::Png8);
        request.memory_budget_bytes = opts.memory_budget_bytes;
        request.region = Some(region);
        let actual = export_document_output(
            &original,
            &project,
            &doc,
            &NoAssets,
            opts,
            false,
            &request,
            &|| false,
        )?;
        assert_pixels_equal(
            &decode(&actual.bytes, DecodeLimits::default())?.pixels,
            &expected.pixels,
            "O-grid region",
        )?;
        assert_eq!(actual.metadata.settings.region, Some(region));
        assert_eq!(
            actual.metadata.image_output,
            Some(ImageOutputMetadata::new(frame))
        );
        Ok(())
    }
    #[test]
    fn clean_tonal_output_is_adjusted_and_annotation_free() -> TestResult {
        let original = source(32, 24, 8);
        let (mut project, doc, layer) = project(&original)?;
        tone(&mut project, &doc)?;
        assert!(requires_document_output(&project, &doc, false)?);
        let clean = prepare_document_output(
            &original,
            &project,
            &doc,
            &NoAssets,
            options(),
            false,
            &|| false,
        )?;
        let expected = render_document(&original, &project, &doc, &NoAssets, options())?;
        assert_pixels_equal(
            &clean.image.pixels,
            &expected.pixels,
            "tone is included in clean output",
        )?;
        let (key, value) = object(&doc, &layer, 20, Shape::Rect(rect(1.0, 1.0, 15.0, 15.0)))?;
        project.objects.insert(key, value);
        let clean_with_marks = prepare_document_output(
            &original,
            &project,
            &doc,
            &NoAssets,
            options(),
            false,
            &|| false,
        )?;
        assert_pixels_equal(
            &clean_with_marks.image.pixels,
            &clean.image.pixels,
            "annotation excluded",
        )?;
        let marked = prepare_document_output(
            &original,
            &project,
            &doc,
            &NoAssets,
            options(),
            true,
            &|| false,
        )?;
        assert_ne!(marked.image.pixels.rgba8(), clean.image.pixels.rgba8());
        Ok(())
    }
    #[test]
    fn ordinary_crop_guides_and_disabled_frames_keep_document_dimensions() -> TestResult {
        let original = source(24, 16, 8);
        let (mut project, doc, layer) = project(&original)?;
        let (key, value) = object(&doc, &layer, 20, Shape::Crop(rect(2.0, 2.0, 12.0, 8.0)))?;
        project.objects.insert(key, value);
        assert!(!requires_document_output(&project, &doc, false)?);
        let ordinary = prepare_document_output(
            &original,
            &project,
            &doc,
            &NoAssets,
            options(),
            false,
            &|| false,
        )?;
        assert_eq!((ordinary.image.width, ordinary.image.height), (24, 16));
        assert!(ordinary.frame.is_none());
        let frame = ImageOutputFrame::new([24, 16], [2, 2, 12, 8], QuarterTurn::Half, [6, 4])?;
        output(&mut project, &doc, frame)?;
        project.layers.get_mut(&id(70)?).ok_or("layer")?.visible = false;
        let disabled = prepare_document_output(
            &original,
            &project,
            &doc,
            &NoAssets,
            options(),
            false,
            &|| false,
        )?;
        assert_eq!((disabled.image.width, disabled.image.height), (24, 16));
        assert!(disabled.frame.is_none());
        assert_pixels_equal(
            &disabled.image.pixels,
            &ordinary.image.pixels,
            "disabled output",
        )?;
        Ok(())
    }
    #[test]
    fn admission_and_cancellation_precede_full_composition_and_keep_source() -> TestResult {
        let original = source(8, 6, 16);
        let (mut project, doc, _) = project(&original)?;
        output(
            &mut project,
            &doc,
            ImageOutputFrame::new([8, 6], [0, 0, 8, 6], QuarterTurn::Zero, [1000, 1000])?,
        )?;
        assert!(matches!(
            prepare_document_output(
                &original,
                &project,
                &doc,
                &NoAssets,
                RenderOptions {
                    memory_budget_bytes: 1024,
                    ..options()
                },
                false,
                &|| false
            ),
            Err(RasterError::Memory { .. })
        ));
        assert!(matches!(
            prepare_document_output(
                &original,
                &project,
                &doc,
                &NoAssets,
                options(),
                false,
                &|| true
            ),
            Err(RasterError::Cancelled)
        ));
        let mut request = request(ExportFormat::Png8);
        request.memory_budget_bytes = options().memory_budget_bytes;
        assert!(matches!(
            export_document_output(
                &original,
                &project,
                &doc,
                &NoAssets,
                options(),
                false,
                &request,
                &|| false
            ),
            Err(RasterError::Depth)
        ));
        assert_pixels_equal(
            &original.pixels,
            &source(8, 6, 16).pixels,
            "refusal original",
        )?;
        Ok(())
    }

    #[test]
    fn editing_region_keeps_canvas_annotations_and_tones_outside_output_crop() -> TestResult {
        let original = source(64, 40, 16);
        let before = original.clone();
        let (mut project, doc, layer) = project(&original)?;
        let (key, value) = object(&doc, &layer, 20, Shape::Rect(rect(1.0, 1.0, 15.0, 15.0)))?;
        project.objects.insert(key, value);
        tone(&mut project, &doc)?;
        output(
            &mut project,
            &doc,
            ImageOutputFrame::new(
                [64, 40],
                [32, 0, 32, 40],
                QuarterTurn::Clockwise90,
                [20, 16],
            )?,
        )?;
        let region = Region {
            x: 0,
            y: 0,
            width: 24,
            height: 30,
        };
        let full = render_document(&original, &project, &doc, &NoAssets, options())?;
        let expected = full.crop(region)?;
        let actual = render_document_editing_region(
            &original,
            &project,
            &doc,
            &NoAssets,
            options(),
            region,
            &|| false,
        )?;
        assert_pixels_equal(
            &actual.pixels,
            &expected.pixels,
            "D-space complete editable canvas",
        )?;
        let clean = prepare_document_output(
            &original,
            &project,
            &doc,
            &NoAssets,
            options(),
            false,
            &|| false,
        )?;
        assert_eq!((actual.width, actual.height), (24, 30));
        assert_eq!((clean.image.width, clean.image.height), (20, 16));
        assert_ne!(actual.pixels, original.crop(region)?.pixels);
        assert_eq!(original, before);
        Ok(())
    }

    #[test]
    fn editing_region_validates_complete_source_before_cropping() -> TestResult {
        let original = source(8, 6, 16);
        let (project, doc, _) = project(&original)?;
        let region = Region {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        for change in 0..6 {
            let mut bad = original.clone();
            match change {
                0 => bad.width = 9,
                1 => bad.source_asset = vw_model::AssetId::hash(b"different source"),
                2 => bad.orientation_applied = 2,
                3 => bad.pixels = Pixels::Rgba8(vec![0; 8 * 6 * 4]),
                4 => bad.icc = Some(standard_srgb_profile()?),
                _ => {
                    if let Pixels::Rgba16(samples) = &mut bad.pixels {
                        samples.pop();
                    }
                }
            }
            assert!(
                render_document_editing_region(
                    &bad,
                    &project,
                    &doc,
                    &NoAssets,
                    options(),
                    region,
                    &|| false
                )
                .is_err(),
                "case {change}"
            );
        }
        let outside = Region {
            x: 8,
            y: 0,
            width: 1,
            height: 1,
        };
        assert!(
            render_document_editing_region(
                &original,
                &project,
                &doc,
                &NoAssets,
                options(),
                outside,
                &|| false
            )
            .is_err()
        );
        assert!(matches!(
            render_document_editing_region(
                &original,
                &project,
                &doc,
                &NoAssets,
                options(),
                region,
                &|| true
            ),
            Err(RasterError::Cancelled)
        ));
        assert!(matches!(
            render_document_editing_region(
                &original,
                &project,
                &doc,
                &NoAssets,
                RenderOptions {
                    memory_budget_bytes: 1,
                    ..options()
                },
                region,
                &|| false
            ),
            Err(RasterError::Memory { .. })
        ));
        Ok(())
    }

    #[test]
    fn document_admission_counts_full_output_and_retained_source_capacity() -> TestResult {
        let mut original = source(8, 6, 8);
        let (mut project, doc, _) = project(&original)?;
        let before = document_output_estimate(&original, &project, &doc, options())?;
        if let Pixels::Rgba8(samples) = &mut original.pixels {
            samples.reserve_exact(4096);
        }
        let after = document_output_estimate(&original, &project, &doc, options())?;
        assert!(after >= before + 4096);
        output(
            &mut project,
            &doc,
            ImageOutputFrame::new([8, 6], [0, 0, 8, 6], QuarterTurn::Zero, [1000, 1000])?,
        )?;
        assert!(
            document_output_estimate(&original, &project, &doc, options())? > after + 7_000_000
        );
        let mut req = request(ExportFormat::Png8);
        req.memory_budget_bytes = options().memory_budget_bytes;
        req.region = Some(Region {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        });
        assert!(matches!(
            export_document_output(
                &original,
                &project,
                &doc,
                &NoAssets,
                RenderOptions {
                    memory_budget_bytes: 1024,
                    ..options()
                },
                false,
                &ExportRequest {
                    memory_budget_bytes: 1024,
                    ..req
                },
                &|| false
            ),
            Err(RasterError::Memory { .. })
        ));
        assert!(matches!(
            document_output_reservation(
                &project,
                &doc,
                RenderOptions {
                    memory_budget_bytes: 1,
                    ..options()
                }
            ),
            Err(RasterError::Memory { .. })
        ));
        Ok(())
    }

    #[test]
    fn editing_tile_predecode_reservation_is_independent_of_output_size() -> TestResult {
        let original = source(8, 6, 8);
        let (mut project, doc, _) = project(&original)?;
        let region = Region {
            x: 2,
            y: 1,
            width: 3,
            height: 2,
        };
        let before = document_editing_region_reservation(&project, &doc, options(), region)?;
        output(
            &mut project,
            &doc,
            ImageOutputFrame::new([8, 6], [0, 0, 8, 6], QuarterTurn::Half, [5000, 5000])?,
        )?;
        // Output object/layer bookkeeping adds a tiny geometry reserve; the
        // 25-million-pixel O buffer is unrelated to this D-space tile.
        let after = document_editing_region_reservation(&project, &doc, options(), region)?;
        assert!(after >= before && after - before < 1024);
        assert!(document_output_reservation(&project, &doc, options())? > after + 100_000_000);
        let actual = render_document_editing_region(
            &original,
            &project,
            &doc,
            &NoAssets,
            RenderOptions {
                memory_budget_bytes: after,
                ..options()
            },
            region,
            &|| false,
        )?;
        assert_pixels_equal(
            &actual.pixels,
            &original.crop(region)?.pixels,
            "editing tile ignores output size",
        )?;
        Ok(())
    }

    #[test]
    fn editing_region_cancellation_stops_during_copy_and_preserves_original() -> TestResult {
        let original = source(64, 40, 16);
        let before = original.clone();
        let (project, doc, _) = project(&original)?;
        let checks = std::cell::Cell::new(0);
        let cancel = || {
            checks.set(checks.get() + 1);
            checks.get() > 8
        };
        assert!(matches!(
            render_document_editing_region(
                &original,
                &project,
                &doc,
                &NoAssets,
                options(),
                Region {
                    x: 0,
                    y: 0,
                    width: 64,
                    height: 40
                },
                &cancel
            ),
            Err(RasterError::Cancelled)
        ));
        assert!(checks.get() <= 10);
        assert_eq!(original, before);
        Ok(())
    }

    #[test]
    fn editing_region_admits_result_assets_before_resolving_them() -> TestResult {
        let original = source(8, 6, 16);
        let (mut project, doc, layer) = project(&original)?;
        let result_asset = vw_model::AssetId::hash(b"oversized synthetic result");
        let mut binding = asset(&original);
        binding.asset_id = result_asset.to_string();
        binding.width = 10_000;
        binding.height = 5_000;
        project.assets.insert(result_asset.clone(), binding);
        let result = id(50)?;
        project.results.insert(
            result.clone(),
            vw_model::ResultCandidate {
                definition: pb::AddResult {
                    result_id: Some(result.to_proto()),
                    document_id: Some(doc.to_proto()),
                    provider: "synthetic".into(),
                    model: "fixture".into(),
                    request_json: "{}".into(),
                    output_asset_id: result_asset.to_string(),
                    composite_asset_id: result_asset.to_string(),
                    proof_json: "{\"changed_outside\":0}".into(),
                    metrics_json: "{}".into(),
                    ..Default::default()
                },
                status: "ready".into(),
                acceptance_mask_asset_id: None,
                created_at_ms: 0,
            },
        );
        let (key, value) = object(&doc, &layer, 20, Shape::ResultId(result.to_proto()))?;
        project.objects.insert(key, value);
        project.validate()?;
        struct CountAssets(std::cell::Cell<usize>);
        impl AssetResolver for CountAssets {
            fn image(&self, _: &str) -> Result<DecodedImage, RasterError> {
                self.0.set(self.0.get() + 1);
                Err(RasterError::MissingAsset)
            }
        }
        let assets = CountAssets(std::cell::Cell::new(0));
        let opts = RenderOptions {
            memory_budget_bytes: 128 * 1024 * 1024,
            ..options()
        };
        let region = Region {
            x: 1,
            y: 1,
            width: 1,
            height: 1,
        };
        assert!(matches!(
            render_document_editing_region(
                &original,
                &project,
                &doc,
                &assets,
                opts,
                region,
                &|| false
            ),
            Err(RasterError::Memory { .. })
        ));
        assert_eq!(assets.0.get(), 0);
        // Clean output does not resolve a result that it intentionally omits.
        prepare_document_output(&original, &project, &doc, &assets, opts, false, &|| false)?;
        assert_eq!(assets.0.get(), 0);
        Ok(())
    }
}
