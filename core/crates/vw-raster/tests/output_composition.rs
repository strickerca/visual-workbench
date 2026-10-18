mod support;

use std::cell::Cell;
use support::*;
use vw_geom::{ImageOutputFrame, QuarterTurn};
use vw_model::{Id, Project};
use vw_proto::v1::{self, object_state::Shape};
use vw_raster::*;

fn frame_project(
    original: &DecodedImage,
    frame: ImageOutputFrame,
) -> Result<(Project, Id), Box<dyn std::error::Error>> {
    let (mut project, document, layer) = project(original)?;
    project
        .layers
        .get_mut(&layer)
        .ok_or("layer")?
        .definition
        .kind = vw_model::IMAGE_OUTPUT_LAYER_KIND.into();
    let [x, y, w, h] = frame.crop();
    let (id, mut object) = object(
        &document,
        &layer,
        4,
        Shape::Crop(rect(x.into(), y.into(), w.into(), h.into())),
    )?;
    let [a, b, c, d, e, f] = frame.matrix()?.coefficients();
    object.state.transform = Some(v1::Affine { a, b, c, d, e, f });
    project.objects.insert(id, object);
    project.validate()?;
    Ok((project, document))
}

fn embedded_metadata(
    bytes: &[u8],
    format: ExportFormat,
) -> Result<ExportMetadata, Box<dyn std::error::Error>> {
    if matches!(format, ExportFormat::Png8 | ExportFormat::Png16) {
        let reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info()?;
        let text = reader.info().utf8_text.first().ok_or("PNG provenance")?;
        assert_eq!(text.keyword, "VisualWorkbench");
        return Ok(serde_json::from_str(&text.get_text()?)?);
    }
    let text = String::from_utf8_lossy(bytes);
    let (_, rest) = text.split_once("<vwb:Metadata>").ok_or("XMP provenance")?;
    let (json, _) = rest.split_once("</vwb:Metadata>").ok_or("XMP end")?;
    Ok(serde_json::from_str(json)?)
}

#[test]
fn encoded_png_jpeg_and_webp_embed_original_extent_and_complete_output_mapping() -> TestResult {
    for format in [
        ExportFormat::Png8,
        ExportFormat::Png16,
        ExportFormat::Jpeg { quality: 91 },
        ExportFormat::WebpLossless,
        ExportFormat::WebpLossy { quality: 87 },
    ] {
        let mut original = source(8, 6, 8);
        original.icc = Some(standard_srgb_profile()?);
        let before = original.clone();
        let frame = ImageOutputFrame::new([8, 6], [2, 1, 4, 3], QuarterTurn::Clockwise90, [6, 8])?;
        let (project, document) = frame_project(&original, frame)?;
        let mut request = request(format);
        request.region = Some(Region {
            x: 1,
            y: 2,
            width: 3,
            height: 4,
        });
        request.alpha = AlphaPolicy::Matte([0, 0, 0]);
        let encoded = export_document_output(
            &original,
            &project,
            &document,
            &NoAssets,
            options(),
            true,
            &request,
            &|| false,
        )?;
        let embedded = embedded_metadata(&encoded.bytes, format)?;
        assert_eq!((embedded.source_width, embedded.source_height), (8, 6));
        assert_eq!((embedded.output_width, embedded.output_height), (3, 4));
        assert_eq!(embedded.image_output, Some(ImageOutputMetadata::new(frame)));
        assert_eq!(embedded.source_asset, original.source_asset.to_string());
        assert_eq!(embedded.settings.region, request.region);
        assert_eq!(
            embedded.source_icc_hash,
            original
                .icc
                .as_ref()
                .map(|p| vw_model::AssetId::hash(p).to_string())
        );
        assert_eq!(
            serde_json::to_value(embedded)?,
            serde_json::to_value(encoded.metadata)?
        );
        let decoded = decode(&encoded.bytes, DecodeLimits::default())?;
        assert_eq!((decoded.width, decoded.height), (3, 4));
        assert_eq!(original, before);
    }
    Ok(())
}

#[test]
fn output_preparer_binds_full_grid_before_region_and_preserves_seekable_codec_policies()
-> TestResult {
    let original = source(8, 6, 16);
    let before = original.clone();
    let frame = ImageOutputFrame::new([8, 6], [2, 1, 4, 3], QuarterTurn::Clockwise90, [6, 8])?;
    let output = render_image_output(&original, frame, options().memory_budget_bytes, &|| false)?;
    let metadata = ImageOutputMetadata::new(frame);
    let mut r = request(ExportFormat::Tiff16);
    r.region = Some(Region {
        x: 1,
        y: 6,
        width: 2,
        height: 2,
    });
    let prepared = prepare_export_pixels_with_output(&output, &r, Some(metadata), &|| false)?;
    assert_eq!(prepared.image.pixels.bit_depth(), 16);
    assert_eq!(
        prepared.image.pixels,
        output.crop(r.region.ok_or("region")?)?.pixels
    );
    assert_eq!(prepared.metadata.image_output, Some(metadata));
    assert_eq!(
        (
            prepared.metadata.source_width,
            prepared.metadata.source_height
        ),
        (8, 6)
    );
    for format in [ExportFormat::Bmp8, ExportFormat::Tiff8] {
        r.format = format;
        assert!(matches!(
            prepare_export_pixels_with_output(&output, &r, Some(metadata), &|| false),
            Err(RasterError::Depth)
        ));
        r.allow_depth_reduction = true;
        let converted = prepare_export_pixels_with_output(&output, &r, Some(metadata), &|| false)?;
        assert_eq!(converted.image.pixels.bit_depth(), 8);
        assert_eq!(converted.metadata.source_bit_depth, 16);
        assert_eq!(converted.metadata.image_output, Some(metadata));
        r.allow_depth_reduction = false;
    }
    r.format = ExportFormat::Png16;
    r.region = Some(Region {
        x: 6,
        y: 0,
        width: 1,
        height: 1,
    });
    assert!(matches!(
        prepare_export_pixels_with_output(&output, &r, Some(metadata), &|| false),
        Err(RasterError::Invalid("region outside source"))
    ));
    let wrong = ImageOutputMetadata {
        output_width: 5,
        ..metadata
    };
    assert!(matches!(
        prepare_export_pixels_with_output(&output, &r, Some(wrong), &|| false),
        Err(RasterError::Invalid("image output metadata binding"))
    ));
    let unknown = ImageOutputMetadata {
        version: 2,
        ..metadata
    };
    assert!(matches!(
        prepare_export_pixels_with_output(&output, &r, Some(unknown), &|| false),
        Err(RasterError::Unsupported("image output version"))
    ));
    assert!(matches!(
        prepare_export_pixels_with_output(&output, &r, Some(unknown), &|| true),
        Err(RasterError::Cancelled)
    ));
    assert_eq!(original, before);
    Ok(())
}

#[test]
fn absent_output_preserves_ordinary_preparation_and_old_metadata_deserializes() -> TestResult {
    let original = source(8, 6, 16);
    let request = request(ExportFormat::Png16);
    let ordinary = export(&original, &request)?;
    assert!(ordinary.metadata.image_output.is_none());
    let serialized = serde_json::to_string(&ordinary.metadata)?;
    assert!(!serialized.contains("image_output"));
    let old: ExportMetadata = serde_json::from_str(&serialized)?;
    assert!(old.image_output.is_none());
    let prepared = prepare_export_pixels_with_output(&original, &request, None, &|| false)?;
    let legacy = prepare_export_pixels(&original, &request, &|| false)?;
    assert_eq!(prepared.image, original);
    assert_eq!(prepared.image, legacy.image);
    assert_eq!(
        serde_json::to_value(&prepared.metadata)?,
        serde_json::to_value(legacy.metadata)?
    );
    assert_eq!(
        serde_json::to_value(&prepared.metadata)?,
        serde_json::to_value(embedded_metadata(&ordinary.bytes, request.format)?)?
    );
    Ok(())
}

struct CountingSource<'a> {
    source: BorrowedSource<'a>,
    reads: Cell<usize>,
}
impl RasterSource for CountingSource<'_> {
    fn info(&self) -> &SourceInfo {
        self.source.info()
    }
    fn resident_bytes(&self) -> u64 {
        self.source.resident_bytes()
    }
    fn read_workspace_bytes(&self) -> u64 {
        self.source.read_workspace_bytes()
    }
    fn read_region(
        &mut self,
        region: Region,
        budget: u64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<DecodedImage, RasterError> {
        self.reads.set(self.reads.get() + 1);
        self.source.read_region(region, budget, cancelled)
    }
}

#[test]
fn legacy_document_stream_refuses_active_identity_and_transformed_frames_before_io() -> TestResult {
    let original = source(8, 6, 16);
    for frame in [
        ImageOutputFrame::identity(8, 6)?,
        ImageOutputFrame::new([8, 6], [2, 1, 4, 3], QuarterTurn::Clockwise270, [9, 7])?,
    ] {
        let (project, document) = frame_project(&original, frame)?;
        let mut source = CountingSource {
            source: BorrowedSource::new(&original)?,
            reads: Cell::new(0),
        };
        let mut bytes = vec![9, 8, 7];
        assert!(matches!(
            export_document_png_to(
                &mut source,
                MarkedDocument {
                    project: &project,
                    document: &document,
                    assets: &NoAssets,
                    options: options()
                },
                &request(ExportFormat::Png16),
                StreamLimits::default(),
                &mut bytes,
                &|| false,
            ),
            Err(RasterError::Unsupported(
                "image output requires the bounded output compositor"
            ))
        ));
        assert_eq!(source.reads.get(), 0);
        assert_eq!(bytes, [9, 8, 7]);
    }
    Ok(())
}

#[test]
fn ordinary_renderer_skips_frame_metadata_even_with_guides_and_stream_keeps_disabled_behavior()
-> TestResult {
    let original = source(8, 6, 16);
    let frame = ImageOutputFrame::new([8, 6], [2, 1, 4, 3], QuarterTurn::Half, [7, 9])?;
    let (mut project, document) = frame_project(&original, frame)?;
    let opts = RenderOptions {
        include_guides: true,
        ..options()
    };
    assert_eq!(
        render_document(&original, &project, &document, &NoAssets, opts)?,
        original
    );
    for layer in project.layers.values_mut() {
        layer.visible = false;
    }
    let mut bytes = Vec::new();
    let report = export_document_png_to(
        &mut BorrowedSource::new(&original)?,
        MarkedDocument {
            project: &project,
            document: &document,
            assets: &NoAssets,
            options: opts,
        },
        &request(ExportFormat::Png16),
        StreamLimits::default(),
        &mut bytes,
        &|| false,
    )?;
    assert!(report.metadata.image_output.is_none());
    assert_eq!(
        decode(&bytes, DecodeLimits::default())?.pixels,
        original.pixels
    );
    project
        .objects
        .values_mut()
        .next()
        .ok_or("frame object")?
        .state
        .hidden = true;
    let mut unchanged = Vec::new();
    assert!(matches!(
        export_document_png_to(
            &mut BorrowedSource::new(&original)?,
            MarkedDocument {
                project: &project,
                document: &document,
                assets: &NoAssets,
                options: opts
            },
            &request(ExportFormat::Png16),
            StreamLimits::default(),
            &mut unchanged,
            &|| false,
        ),
        Err(RasterError::Model(_))
    ));
    assert!(unchanged.is_empty());
    Ok(())
}
