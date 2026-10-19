mod support;
use support::*;
use vw_geom::{ImageOutputFrame, QuarterTurn};
use vw_proto::v1::{self, object_state::Shape};
use vw_raster::*;

fn fixture() -> Result<(DecodedImage, vw_model::Project, vw_model::Id), Box<dyn std::error::Error>>
{
    let source = source(12, 8, 8);
    let (mut project, document, layer) = project(&source)?;
    let (id, object) = object(&document, &layer, 4, Shape::Rect(rect(2.0, 2.0, 7.0, 4.0)))?;
    project.objects.insert(id, object);
    project.validate()?;
    Ok((source, project, document))
}

fn tile() -> Region {
    Region {
        x: 4,
        y: 1,
        width: 5,
        height: 6,
    }
}

#[test]
fn region_matches_canonical_full_canvas_at_nonzero_d_origin() -> TestResult {
    let (source, project, document) = fixture()?;
    let region = tile();
    let input = source.crop(region)?;
    let info = BorrowedSource::new(&source)?.info().clone();
    let full = render_document(&source, &project, &document, &NoAssets, options())?;
    let expected = full.crop(region)?;
    let before = input.clone();
    let actual = compose_document_region(
        &input,
        &info,
        &project,
        &document,
        &NoAssets,
        options(),
        region,
        &|| false,
    )?;
    assert_pixels_equal(&actual.pixels, &expected.pixels, "D-space tile")?;
    assert_eq!(input, before);
    assert_eq!(actual.source_asset, source.source_asset);
    Ok(())
}

#[test]
fn output_crop_does_not_hide_d_space_editing_pixels() -> TestResult {
    let (source, mut project, document) = fixture()?;
    let output_layer = id(8)?;
    let mut layer = project.layers.values().next().ok_or("layer")?.clone();
    layer.definition.layer_id = Some(output_layer.to_proto());
    layer.definition.kind = vw_model::IMAGE_OUTPUT_LAYER_KIND.into();
    project.layers.insert(output_layer.clone(), layer);
    let frame = ImageOutputFrame::new([12, 8], [0, 0, 2, 2], QuarterTurn::Clockwise90, [2, 2])?;
    let (id, mut output) = object(
        &document,
        &output_layer,
        9,
        Shape::Crop(rect(0.0, 0.0, 2.0, 2.0)),
    )?;
    let [a, b, c, d, e, f] = frame.matrix()?.coefficients();
    output.state.transform = Some(v1::Affine { a, b, c, d, e, f });
    project.objects.insert(id, output);
    project.validate()?;
    assert!(project.active_image_output(&document)?.is_some());
    let region = tile();
    let input = source.crop(region)?;
    let info = BorrowedSource::new(&source)?.info().clone();
    let actual = compose_document_region(
        &input,
        &info,
        &project,
        &document,
        &NoAssets,
        options(),
        region,
        &|| false,
    )?;
    assert_ne!(actual.pixels, input.pixels);
    assert_pixels_equal(
        &actual.pixels,
        &render_document(&source, &project, &document, &NoAssets, options())?
            .crop(region)?
            .pixels,
        "output crop independent D-space edit",
    )?;
    Ok(())
}

#[test]
fn tonal_layer_and_annotation_share_full_document_order_in_a_region() -> TestResult {
    let (source, mut project, document) = fixture()?;
    let tone_id = id(10)?;
    let mut tone = project.layers.values().next().ok_or("layer")?.clone();
    tone.definition.layer_id = Some(tone_id.to_proto());
    tone.definition.kind = "adjustment".into();
    tone.definition.order_key = "W".into();
    project.layers.insert(tone_id.clone(), tone);
    let (id, object) = object(
        &document,
        &tone_id,
        11,
        Shape::Adjustment(v1::Adjustment {
            brightness: 0.15,
            contrast: 0.2,
            levels: vec![0.0, 1.0, 1.0, 0.0, 1.0],
        }),
    )?;
    project.objects.insert(id, object);
    project.validate()?;
    let region = tile();
    let info = BorrowedSource::new(&source)?.info().clone();
    let actual = compose_document_region(
        &source.crop(region)?,
        &info,
        &project,
        &document,
        &NoAssets,
        options(),
        region,
        &|| false,
    )?;
    let expected =
        render_document(&source, &project, &document, &NoAssets, options())?.crop(region)?;
    assert_pixels_equal(&actual.pixels, &expected.pixels, "tone then annotation")?;
    Ok(())
}

#[test]
fn accepted_result_uses_existing_resolver_and_full_d_transform() -> TestResult {
    struct Assets(DecodedImage);
    impl AssetResolver for Assets {
        fn image(&self, asset: &str) -> Result<DecodedImage, RasterError> {
            if asset != self.0.source_asset.as_str() {
                return Err(RasterError::MissingAsset);
            }
            Ok(self.0.clone())
        }
    }
    let (source, mut project, document) = fixture()?;
    let layer = project.layers.keys().next().ok_or("layer")?.clone();
    let mut result_image = support::source(12, 8, 8);
    result_image.source_asset = vw_model::AssetId::hash(b"bounded result original");
    project
        .assets
        .insert(result_image.source_asset.clone(), asset(&result_image));
    let result_id = id(20)?;
    project.results.insert(
        result_id.clone(),
        vw_model::ResultCandidate {
            definition: v1::AddResult {
                result_id: Some(result_id.to_proto()),
                document_id: Some(document.to_proto()),
                provider: "synthetic".into(),
                model: "fixture".into(),
                request_json: "{}".into(),
                output_asset_id: result_image.source_asset.to_string(),
                composite_asset_id: result_image.source_asset.to_string(),
                proof_json: "{\"changed_outside\":0}".into(),
                metrics_json: "{}".into(),
                cost_estimate_usd: 0.0,
                package_id: None,
            },
            status: "ready".into(),
            acceptance_mask_asset_id: None,
            created_at_ms: 0,
        },
    );
    let (id, object) = object(&document, &layer, 21, Shape::ResultId(result_id.to_proto()))?;
    project.objects.insert(id, object);
    project.validate()?;
    let region = tile();
    let info = BorrowedSource::new(&source)?.info().clone();
    let assets = Assets(result_image);
    let mut denied = options();
    denied.assume_untagged_srgb = false;
    // Both objects have the same order key; id(4) renders first. Isolate each
    // refusal so an annotation cannot satisfy the result assertion.
    let mut annotation_only = project.clone();
    let removed_result = annotation_only
        .objects
        .remove(&id(21)?)
        .ok_or("result fixture")?;
    assert!(matches!(
        removed_result.state.shape,
        Some(Shape::ResultId(_))
    ));
    annotation_only.validate()?;
    assert!(matches!(
        compose_document_region(
            &source.crop(region)?,
            &info,
            &annotation_only,
            &document,
            &assets,
            denied,
            region,
            &|| false,
        ),
        Err(RasterError::Unsupported(
            "untagged annotation color; explicitly assume sRGB"
        ))
    ));
    let mut result_only = project.clone();
    let removed_annotation = result_only
        .objects
        .remove(&id(4)?)
        .ok_or("annotation fixture")?;
    assert!(matches!(
        removed_annotation.state.shape,
        Some(Shape::Rect(_))
    ));
    result_only.validate()?;
    assert!(matches!(
        compose_document_region(
            &source.crop(region)?,
            &info,
            &result_only,
            &document,
            &assets,
            denied,
            region,
            &|| false,
        ),
        Err(RasterError::Unsupported(
            "untagged result color; explicitly assume sRGB"
        ))
    ));
    let actual = compose_document_region(
        &source.crop(region)?,
        &info,
        &project,
        &document,
        &assets,
        options(),
        region,
        &|| false,
    )?;
    let expected =
        render_document(&source, &project, &document, &assets, options())?.crop(region)?;
    assert_pixels_equal(
        &actual.pixels,
        &expected.pixels,
        "accepted result at D origin",
    )?;
    Ok(())
}

#[test]
fn tagged_profile_validation_waits_for_region_memory_admission() -> TestResult {
    let (source, mut project, document) = fixture()?;
    let mut info = BorrowedSource::new(&source)?.info().clone();
    let profile = standard_srgb_profile()?;
    info.icc = Some(profile.clone());
    project
        .assets
        .get_mut(&source.source_asset)
        .ok_or("asset")?
        .icc_profile = profile;
    let mut tight = options();
    tight.memory_budget_bytes = 1;
    assert!(matches!(
        region_composition_estimate(&info, &project, &document, tight, tile()),
        Err(RasterError::Memory { .. })
    ));
    let malformed = vec![0; 132];
    info.icc = Some(malformed.clone());
    project
        .assets
        .get_mut(&source.source_asset)
        .ok_or("asset")?
        .icc_profile = malformed;
    assert!(matches!(
        region_composition_estimate(&info, &project, &document, tight, tile()),
        Err(RasterError::Memory { .. })
    ));
    assert!(matches!(
        region_composition_estimate(&info, &project, &document, options(), tile()),
        Err(RasterError::Color)
    ));
    Ok(())
}

#[test]
fn wrong_original_region_metadata_budget_and_cancellation_refuse() -> TestResult {
    let (source, project, document) = fixture()?;
    let region = tile();
    let input = source.crop(region)?;
    let info = BorrowedSource::new(&source)?.info().clone();
    let mut foreign = info.clone();
    foreign.source_asset = vw_model::AssetId::hash(b"other original");
    assert!(matches!(
        compose_document_region(
            &input,
            &foreign,
            &project,
            &document,
            &NoAssets,
            options(),
            region,
            &|| false
        ),
        Err(RasterError::Invalid("document source binding"))
    ));
    let wrong = Region { x: 11, ..region };
    assert!(region_composition_estimate(&info, &project, &document, options(), wrong).is_err());
    let mut tight = options();
    tight.memory_budget_bytes = 1;
    assert!(matches!(
        region_composition_estimate(&info, &project, &document, tight, region),
        Err(RasterError::Memory { .. })
    ));
    assert!(matches!(
        compose_document_region(
            &input,
            &info,
            &project,
            &document,
            &NoAssets,
            options(),
            region,
            &|| true
        ),
        Err(RasterError::Cancelled)
    ));
    Ok(())
}

#[test]
fn composed_box_reduction_uses_original_samples_and_partial_edge_blocks() -> TestResult {
    let mut source = source(3, 3, 8);
    source.pixels = Pixels::Rgba8(vec![
        0, 0, 0, 255, 100, 0, 0, 255, 10, 0, 0, 255, 0, 100, 0, 255, 100, 100, 0, 255, 10, 100, 0,
        255, 0, 10, 0, 255, 100, 10, 0, 255, 10, 10, 0, 255,
    ]);
    assert_eq!(
        downsample_composed_box_rgba8(&source, 2, 2, 2, &|| false)?,
        vec![
            50, 50, 0, 255, 10, 50, 0, 255, 50, 10, 0, 255, 10, 10, 0, 255
        ]
    );
    assert!(matches!(
        downsample_composed_box_rgba8(&source, 8, 1, 1, &|| false),
        Err(RasterError::Unsupported("bounded composed box level"))
    ));
    assert!(matches!(
        downsample_composed_box_rgba8(&source, 2, 2, 2, &|| true),
        Err(RasterError::Cancelled)
    ));
    source.width = 2;
    source.height = 2;
    source.pixels = Pixels::Rgba8(vec![
        255, 0, 0, 0, 0, 0, 255, 255, 255, 0, 0, 0, 0, 0, 255, 255,
    ]);
    assert_eq!(
        downsample_composed_box_rgba8(&source, 2, 1, 1, &|| false)?,
        vec![0, 0, 255, 128]
    );
    Ok(())
}

#[test]
fn coarse_editing_tile_reduces_the_composed_d_region() -> TestResult {
    let (source, project, document) = fixture()?;
    let region = Region {
        x: 2,
        y: 2,
        width: 8,
        height: 4,
    };
    let info = BorrowedSource::new(&source)?.info().clone();
    let composed = compose_document_region(
        &source.crop(region)?,
        &info,
        &project,
        &document,
        &NoAssets,
        options(),
        region,
        &|| false,
    )?;
    let actual = downsample_composed_box_rgba8(&composed, 2, 4, 2, &|| false)?;
    let expected = downsample_composed_box_rgba8(
        &render_document(&source, &project, &document, &NoAssets, options())?.crop(region)?,
        2,
        4,
        2,
        &|| false,
    )?;
    assert_eq!(actual, expected);
    assert_ne!(
        actual,
        downsample_composed_box_rgba8(&source.crop(region)?, 2, 4, 2, &|| false)?
    );
    Ok(())
}
