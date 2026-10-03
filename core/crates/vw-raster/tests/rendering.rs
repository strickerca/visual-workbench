mod support;
use support::*;
use vw_proto::v1::{self, object_state::Shape};
use vw_raster::*;

#[test]
fn all_annotation_kinds_render_and_keep_uncovered_16bit_samples_exact() -> TestResult {
    let (source, project, doc) = all_annotations()?;
    let before = source.clone();
    let rendered = render_document(&source, &project, &doc, &NoAssets, options())?;
    assert_ne!(rendered.pixels, source.pixels);
    assert_eq!(source, before);
    let (Pixels::Rgba16(a), Pixels::Rgba16(b)) = (&source.pixels, &rendered.pixels) else {
        return Err("depth".into());
    };
    for y in 0..source.height {
        for x in 170..source.width {
            let i = ((y * source.width + x) * 4) as usize;
            assert_eq!(&a[i..i + 4], &b[i..i + 4]);
        }
    }
    let output = export(&rendered, &request(ExportFormat::Png16))?;
    assert_eq!(
        decode(&output.bytes, DecodeLimits::default())?.pixels,
        rendered.pixels
    );
    Ok(())
}
#[test]
fn invisible_layers_and_objects_do_not_modify_a_single_channel() -> TestResult {
    let (source, mut project, doc) = all_annotations()?;
    for object in project.objects.values_mut() {
        object.state.hidden = true;
    }
    assert_eq!(
        render_document(&source, &project, &doc, &NoAssets, options())?.pixels,
        source.pixels
    );
    for object in project.objects.values_mut() {
        object.state.hidden = false;
    }
    for layer in project.layers.values_mut() {
        layer.visible = false;
    }
    assert_eq!(
        render_document(&source, &project, &doc, &NoAssets, options())?.pixels,
        source.pixels
    );
    Ok(())
}
#[test]
fn selection_masks_and_crop_guides_are_explicit_and_not_silently_baked_into_marked_output()
-> TestResult {
    let source = source(32, 32, 8);
    let (mut project, doc, layer) = project(&source)?;
    let shapes = vec![
        Shape::SelectionVector(v1::Polyline {
            points: vec![point(2., 2.), point(10., 2.), point(10., 10.)],
            closed: true,
        }),
        Shape::Crop(rect(15., 3., 10., 10.)),
        Shape::SelectionRaster(v1::RasterMaskRef {
            mask_asset_id: source.source_asset.to_string(),
            bounds: Some(rect(3., 15., 10., 10.)),
            feather: 2.,
        }),
    ];
    for (n, shape) in shapes.into_iter().enumerate() {
        let (id, object) = object(&doc, &layer, 10 + n as u8, shape)?;
        project.objects.insert(id, object);
    }
    assert_eq!(
        render_document(&source, &project, &doc, &NoAssets, options())?.pixels,
        source.pixels
    );
    let mut diagnostic = options();
    diagnostic.include_guides = true;
    assert_ne!(
        render_document(&source, &project, &doc, &NoAssets, diagnostic)?.pixels,
        source.pixels
    );
    Ok(())
}
#[test]
fn isolated_layer_opacity_does_not_double_darken_overlapping_objects() -> TestResult {
    let mut source = source(32, 32, 8);
    source.pixels = Pixels::Rgba8(vec![255; 32 * 32 * 4]);
    let (mut project, doc, layer) = project(&source)?;
    project.layers.get_mut(&layer).ok_or("layer")?.opacity = 0.5;
    for (n, x) in [(10, 2.), (11, 10.)] {
        let (id, mut object) = object(&doc, &layer, n, Shape::Rect(rect(x, 2., 16., 16.)))?;
        object.state.style = Some(v1::Style {
            stroke: Some(v1::Color { rgba: 0xff0000ff }),
            fill: Some(v1::Color { rgba: 0xff0000ff }),
            has_fill: true,
            width: 0.,
            ..Default::default()
        });
        project.objects.insert(id, object);
    }
    let result = render_document(&source, &project, &doc, &NoAssets, options())?;
    let Pixels::Rgba8(p) = result.pixels else {
        return Err("depth".into());
    };
    let a = (8 * 32 + 6) * 4;
    let b = (8 * 32 + 14) * 4;
    assert_eq!(&p[a..a + 4], &p[b..b + 4]);
    assert_eq!(p[a], 255);
    assert!((127..=128).contains(&p[a + 1]));
    Ok(())
}
#[test]
fn multiply_layer_and_affine_transform_use_source_pixels() -> TestResult {
    let mut source = source(16, 16, 8);
    source.pixels = Pixels::Rgba8([128, 200, 50, 255].repeat(16 * 16));
    let (mut project, doc, layer) = project(&source)?;
    project.layers.get_mut(&layer).ok_or("layer")?.blend = "multiply".into();
    let (id, mut object) = object(&doc, &layer, 10, Shape::Rect(rect(0., 0., 4., 4.)))?;
    object.state.transform.as_mut().ok_or("transform")?.e = 5.;
    object.state.style = Some(v1::Style {
        stroke: Some(v1::Color { rgba: 0x808080ff }),
        fill: Some(v1::Color { rgba: 0x808080ff }),
        has_fill: true,
        width: 0.,
        ..Default::default()
    });
    project.objects.insert(id, object);
    let result = render_document(&source, &project, &doc, &NoAssets, options())?;
    let Pixels::Rgba8(p) = result.pixels else {
        return Err("depth".into());
    };
    assert_eq!(&p[0..4], &[128, 200, 50, 255]);
    let at = (2 * 16 + 7) * 4;
    assert_eq!(&p[at..at + 4], &[64, 100, 25, 255]);
    Ok(())
}
#[test]
fn highlighter_compound_contours_do_not_accumulate_alpha() -> TestResult {
    let mut source = source(48, 32, 8);
    source.pixels = Pixels::Rgba8(vec![0; 48 * 32 * 4]);
    let (mut project, doc, layer) = project(&source)?;
    let stroke = v1::Stroke {
        x: vec![5., 20., 35., 20., 5.],
        y: vec![16.; 5],
        t_ms: vec![0, 8, 16, 24, 32],
        pressure: vec![1.; 5],
        tilt: vec![],
        orientation: vec![],
        brush: Some(v1::Brush {
            family: "highlighter".into(),
            algorithm_version: 1,
            base_width: 8.,
            pressure_curve: vec![0., 0., 0.3, 0.3, 0.7, 0.7, 1., 1.],
            stabilization: 0.,
        }),
    };
    let (id, mut object) = object(&doc, &layer, 10, Shape::Stroke(stroke))?;
    object.state.style.as_mut().ok_or("style")?.stroke = Some(v1::Color { rgba: 0xffff0080 });
    project.objects.insert(id, object);
    let result = render_document(&source, &project, &doc, &NoAssets, options())?;
    let Pixels::Rgba8(p) = result.pixels else {
        return Err("depth".into());
    };
    assert!(p.as_chunks::<4>().0.iter().all(|p| p[3] <= 128));
    assert!(p.as_chunks::<4>().0.iter().any(|p| p[3] == 128));
    Ok(())
}
#[test]
fn nondestructive_adjustment_preserves_alpha_original_and_identity() -> TestResult {
    let source = source(16, 16, 16);
    let (mut project, doc, layer) = project(&source)?;
    project
        .layers
        .get_mut(&layer)
        .ok_or("layer")?
        .definition
        .kind = "adjustment".into();
    let (id, object) = object(
        &doc,
        &layer,
        10,
        Shape::Adjustment(v1::Adjustment {
            brightness: 0.,
            contrast: 0.,
            levels: vec![0., 1., 1., 0., 1.],
        }),
    )?;
    project.objects.insert(id.clone(), object);
    assert_eq!(
        render_document(&source, &project, &doc, &NoAssets, options())?.pixels,
        source.pixels
    );
    if let Some(Shape::Adjustment(value)) = project
        .objects
        .get_mut(&id)
        .ok_or("object")?
        .state
        .shape
        .as_mut()
    {
        value.brightness = 0.15;
        value.contrast = 0.2;
    }
    let result = render_document(&source, &project, &doc, &NoAssets, options())?;
    assert_ne!(result.pixels, source.pixels);
    let (Pixels::Rgba16(a), Pixels::Rgba16(b)) = (&source.pixels, &result.pixels) else {
        return Err("depth".into());
    };
    for (a, b) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0.iter()) {
        assert_eq!(a[3], b[3]);
        if a[3] == 0 {
            assert_eq!(a, b);
        }
    }
    Ok(())
}
#[test]
fn result_assets_keep_16bit_precision_and_reject_previews_or_wrong_hash() -> TestResult {
    struct Assets(DecodedImage);
    impl AssetResolver for Assets {
        fn image(&self, _: &str) -> Result<DecodedImage, RasterError> {
            Ok(self.0.clone())
        }
    }
    let mut source = source(8, 8, 16);
    source.pixels = Pixels::Rgba16(vec![0; 8 * 8 * 4]);
    let (mut project, doc, layer) = project(&source)?;
    let mut result_image = support::source(8, 8, 16);
    result_image.source_asset = vw_model::AssetId::hash(b"synthetic result");
    if let Pixels::Rgba16(p) = &mut result_image.pixels {
        for p in p.as_chunks_mut::<4>().0.iter_mut() {
            p[3] = 65535;
        }
    }
    project
        .assets
        .insert(result_image.source_asset.clone(), asset(&result_image));
    let result_id = id(20)?;
    project.results.insert(
        result_id.clone(),
        vw_model::ResultCandidate {
            definition: v1::AddResult {
                result_id: Some(result_id.to_proto()),
                document_id: Some(doc.to_proto()),
                provider: "synthetic".into(),
                model: "fixture".into(),
                request_json: "{}".into(),
                output_asset_id: result_image.source_asset.to_string(),
                composite_asset_id: result_image.source_asset.to_string(),
                proof_json: "{\"changed_outside\":0}".into(),
                metrics_json: "{}".into(),
                cost_estimate_usd: 0.,
                package_id: None,
            },
            status: "ready".into(),
            acceptance_mask_asset_id: None,
            created_at_ms: 0,
        },
    );
    let (id, object) = object(&doc, &layer, 10, Shape::ResultId(result_id.to_proto()))?;
    project.objects.insert(id, object);
    assert_eq!(
        render_document(
            &source,
            &project,
            &doc,
            &Assets(result_image.clone()),
            options()
        )?
        .pixels,
        result_image.pixels
    );
    for status in ["pending", "rejected", "error", "partial"] {
        project.results.get_mut(&result_id).ok_or("result")?.status = status.into();
        assert!(matches!(
            render_document(
                &source,
                &project,
                &doc,
                &Assets(result_image.clone()),
                options()
            ),
            Err(RasterError::Unsupported(_))
        ));
    }
    {
        let candidate = project.results.get_mut(&result_id).ok_or("result")?;
        candidate.status = "accepted".into();
        candidate.acceptance_mask_asset_id = Some(source.source_asset.clone());
    }
    assert!(matches!(
        render_document(
            &source,
            &project,
            &doc,
            &Assets(result_image.clone()),
            options()
        ),
        Err(RasterError::Unsupported(
            "partial result acceptance requires a materialized accepted composite"
        ))
    ));
    project
        .results
        .get_mut(&result_id)
        .ok_or("result")?
        .acceptance_mask_asset_id = None;
    assert!(
        render_document(
            &source,
            &project,
            &doc,
            &Assets(result_image.clone()),
            options()
        )
        .is_ok()
    );
    for changed in 0..4 {
        let metadata = project
            .assets
            .get_mut(&result_image.source_asset)
            .ok_or("asset")?;
        *metadata = asset(&result_image);
        match changed {
            0 => metadata.width += 1,
            1 => metadata.orientation = 3,
            2 => metadata.bit_depth = 8,
            _ => metadata.icc_profile = moxcms::ColorProfile::new_srgb().encode()?,
        }
        assert!(matches!(
            render_document(
                &source,
                &project,
                &doc,
                &Assets(result_image.clone()),
                options()
            ),
            Err(RasterError::Invalid("resolved asset metadata"))
        ));
    }
    *project
        .assets
        .get_mut(&result_image.source_asset)
        .ok_or("asset")? = asset(&result_image);
    result_image.original_available = false;
    assert!(matches!(
        render_document(
            &source,
            &project,
            &doc,
            &Assets(result_image.clone()),
            options()
        ),
        Err(RasterError::OriginalRequired)
    ));
    result_image.original_available = true;
    result_image.source_asset = source.source_asset.clone();
    assert!(render_document(&source, &project, &doc, &Assets(result_image), options()).is_err());
    Ok(())
}
#[test]
fn renderer_rejects_wrong_document_source_unsupported_fonts_and_tiny_budget() -> TestResult {
    let (source, mut project, doc) = all_annotations()?;
    let mut low = options();
    low.memory_budget_bytes = 1;
    assert!(matches!(
        render_document(&source, &project, &doc, &NoAssets, low),
        Err(RasterError::Memory { .. })
    ));
    for o in project.objects.values_mut() {
        if let Some(Shape::Text(t)) = o.state.shape.as_mut() {
            t.font_family = "OS font".into();
        }
    }
    assert!(render_document(&source, &project, &doc, &NoAssets, options()).is_err());
    let mut wrong = source.clone();
    wrong.source_asset = vw_model::AssetId::hash(b"different image");
    assert!(render_document(&wrong, &project, &doc, &NoAssets, options()).is_err());
    Ok(())
}

#[test]
fn persisted_vector_eraser_requires_deletion_operations() -> TestResult {
    let (source, mut project, doc) = all_annotations()?;
    for object in project.objects.values_mut() {
        if let Some(Shape::Stroke(stroke)) = object.state.shape.as_mut() {
            stroke.brush.as_mut().ok_or("brush")?.family = "vector_eraser".into();
        }
    }
    assert!(matches!(
        render_document(&source, &project, &doc, &NoAssets, options()),
        Err(RasterError::Unsupported(
            "unmaterialized vector eraser; create object deletion operations before export"
        ))
    ));
    Ok(())
}

#[test]
fn buffered_renderer_admits_geometry_before_layout_even_on_a_tiny_image() -> TestResult {
    let original = source(1, 1, 16);
    let (mut project, doc, layer) = project(&original)?;
    let (id, text) = object(
        &doc,
        &layer,
        9,
        Shape::Text(v1::TextObject {
            text: "i".repeat(1024),
            font_family: "Inter".into(),
            font_size: 12.0,
            anchor: Some(point(0.0, 0.0)),
        }),
    )?;
    project.objects.insert(id, text);
    assert!(matches!(
        render_document(
            &original,
            &project,
            &doc,
            &NoAssets,
            RenderOptions {
                memory_budget_bytes: 64 * 1024 * 1024,
                ..options()
            }
        ),
        Err(RasterError::Memory { .. })
    ));
    Ok(())
}
