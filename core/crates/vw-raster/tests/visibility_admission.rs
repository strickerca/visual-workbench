mod support;

use std::cell::Cell;
use support::*;
use vw_model::{AssetId, Id, Project, ResultCandidate};
use vw_proto::v1::{self, object_state::Shape};
use vw_raster::*;

const BUDGET: u64 = 64 * 1024 * 1024;

#[derive(Default)]
struct CountingAssets(Cell<usize>);
impl AssetResolver for CountingAssets {
    fn image(&self, _: &str) -> Result<DecodedImage, RasterError> {
        self.0.set(self.0.get() + 1);
        Err(RasterError::MissingAsset)
    }
}

fn bounded_options(include_guides: bool) -> RenderOptions {
    RenderOptions {
        memory_budget_bytes: BUDGET,
        include_guides,
        ..options()
    }
}

fn streamed(
    source: &DecodedImage,
    project: &Project,
    document: &Id,
    assets: &CountingAssets,
    include_guides: bool,
) -> Result<Vec<u8>, RasterError> {
    let mut req = request(ExportFormat::Png16);
    req.memory_budget_bytes = BUDGET;
    let mut bytes = Vec::new();
    export_document_png_to(
        &mut BorrowedSource::new(source)?,
        MarkedDocument {
            project,
            document,
            assets,
            options: bounded_options(include_guides),
        },
        &req,
        StreamLimits {
            max_strip_rows: 1,
            ..StreamLimits::default()
        },
        &mut bytes,
        &|| false,
    )?;
    Ok(bytes)
}

fn unchanged(
    source: &DecodedImage,
    project: &Project,
    document: &Id,
    assets: &CountingAssets,
    include_guides: bool,
) -> TestResult {
    project.validate()?;
    let before = source.clone();
    assert_eq!(
        render_document(
            source,
            project,
            document,
            assets,
            bounded_options(include_guides),
        )?,
        before
    );
    let bytes = streamed(source, project, document, assets, include_guides)?;
    let decoded = decode(&bytes, DecodeLimits::default())?;
    assert_eq!(decoded.pixels, before.pixels);
    assert_eq!(decoded.icc, before.icc);
    assert_eq!(source, &before);
    assert_eq!(assets.0.get(), 0, "invisible assets must never be resolved");
    Ok(())
}

fn visible_is_rejected(
    source: &DecodedImage,
    project: &Project,
    document: &Id,
    assets: &CountingAssets,
    include_guides: bool,
) {
    assert!(matches!(
        render_document(
            source,
            project,
            document,
            assets,
            bounded_options(include_guides),
        ),
        Err(RasterError::Memory { .. })
    ));
    assert!(matches!(
        streamed(source, project, document, assets, include_guides),
        Err(RasterError::Memory { .. })
    ));
    assert_eq!(
        assets.0.get(),
        0,
        "visible over-budget assets fail before resolution"
    );
}

#[test]
fn oversized_text_on_hidden_or_transparent_layers_does_not_consume_geometry_budget() -> TestResult {
    let source = source(2, 2, 16);
    let (mut project, doc, layer) = project(&source)?;
    let (object_id, text) = object(
        &doc,
        &layer,
        10,
        Shape::Text(v1::TextObject {
            text: "i".repeat(1024),
            font_family: "Inter".into(),
            font_size: 12.0,
            anchor: Some(point(0.0, 0.0)),
        }),
    )?;
    project.objects.insert(object_id.clone(), text);
    let assets = CountingAssets::default();
    visible_is_rejected(&source, &project, &doc, &assets, false);

    project.layers.get_mut(&layer).ok_or("layer")?.visible = false;
    unchanged(&source, &project, &doc, &assets, false)?;
    project.layers.get_mut(&layer).ok_or("layer")?.visible = true;
    project.layers.get_mut(&layer).ok_or("layer")?.opacity = 0.0;
    unchanged(&source, &project, &doc, &assets, false)?;
    project.layers.get_mut(&layer).ok_or("layer")?.opacity = 1.0;
    project
        .objects
        .get_mut(&object_id)
        .ok_or("object")?
        .state
        .hidden = true;
    unchanged(&source, &project, &doc, &assets, false)?;

    // A visible, expensive shape in another document also has no workspace in
    // the active document's render, even when both share the original asset.
    let other_id = id(40)?;
    let mut other = project.documents.get(&doc).ok_or("document")?.clone();
    other.definition.document_id = Some(other_id.to_proto());
    project.documents.insert(other_id.clone(), other);
    project
        .layers
        .get_mut(&layer)
        .ok_or("layer")?
        .definition
        .document_id = Some(other_id.to_proto());
    let object = project.objects.get_mut(&object_id).ok_or("object")?;
    object.document_id = other_id;
    object.state.hidden = false;
    unchanged(&source, &project, &doc, &assets, false)?;
    Ok(())
}

#[test]
fn oversized_results_on_hidden_or_transparent_layers_never_resolve_assets() -> TestResult {
    let source = source(2, 2, 16);
    let (mut project, doc, layer) = project(&source)?;
    let result_asset = AssetId::hash(b"synthetic oversized unmaterialized result");
    let mut metadata = asset(&source);
    metadata.asset_id = result_asset.to_string();
    metadata.width = 10_000;
    metadata.height = 5_000;
    project.assets.insert(result_asset.clone(), metadata);
    let result_id = id(20)?;
    project.results.insert(
        result_id.clone(),
        ResultCandidate {
            definition: v1::AddResult {
                result_id: Some(result_id.to_proto()),
                document_id: Some(doc.to_proto()),
                provider: "synthetic".into(),
                model: "fixture".into(),
                request_json: "{}".into(),
                output_asset_id: result_asset.to_string(),
                composite_asset_id: result_asset.to_string(),
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
    let (object_id, result) = object(&doc, &layer, 10, Shape::ResultId(result_id.to_proto()))?;
    project.objects.insert(object_id.clone(), result);
    project.validate()?;
    let assets = CountingAssets::default();
    visible_is_rejected(&source, &project, &doc, &assets, false);

    project.layers.get_mut(&layer).ok_or("layer")?.visible = false;
    unchanged(&source, &project, &doc, &assets, false)?;
    project.layers.get_mut(&layer).ok_or("layer")?.visible = true;
    project.layers.get_mut(&layer).ok_or("layer")?.opacity = 0.0;
    unchanged(&source, &project, &doc, &assets, false)?;
    project.layers.get_mut(&layer).ok_or("layer")?.opacity = 1.0;
    project
        .objects
        .get_mut(&object_id)
        .ok_or("object")?
        .state
        .hidden = true;
    unchanged(&source, &project, &doc, &assets, false)?;
    Ok(())
}

#[test]
fn excluded_large_selection_guides_are_only_admitted_for_diagnostic_rendering() -> TestResult {
    let source = source(2, 2, 16);
    let (mut project, doc, layer) = project(&source)?;
    let (object_id, guide) = object(
        &doc,
        &layer,
        10,
        Shape::SelectionVector(v1::Polyline {
            points: (0..65536)
                .map(|i| match i % 3 {
                    0 => point(0.0, 0.0),
                    1 => point(1.0, 0.0),
                    _ => point(1.0, 1.0),
                })
                .collect(),
            closed: true,
        }),
    )?;
    project.objects.insert(object_id, guide);
    let assets = CountingAssets::default();
    unchanged(&source, &project, &doc, &assets, false)?;
    visible_is_rejected(&source, &project, &doc, &assets, true);
    project.layers.get_mut(&layer).ok_or("layer")?.visible = false;
    unchanged(&source, &project, &doc, &assets, true)?;
    Ok(())
}
