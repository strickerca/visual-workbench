mod support;

use std::cell::Cell;
use support::*;
use vw_model::{AssetId, ResultCandidate};
use vw_proto::v1::{self, object_state::Shape};
use vw_raster::*;

// A ~16 KiB profile with eight references to the same 16 KiB text. Its parser
// estimate is ~664 KiB; the caller must reserve parsed objects, not unique bytes.
// A bad semantic signature makes an accidental early parser call report Color
// instead of the required caller-budget Memory result.
fn alias_profile() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let base = moxcms::ColorProfile::new_srgb().encode()?;
    let records = 8usize;
    let payload = 16_384usize;
    let strings = 16 + records * 12;
    let tag_size = strings + payload;
    let mut profile = vec![0; 144 + tag_size];
    profile[..128].copy_from_slice(&base[..128]);
    let length = u32::try_from(profile.len())?;
    profile[..4].copy_from_slice(&length.to_be_bytes());
    profile[36..40].copy_from_slice(b"bad!");
    profile[128..132].copy_from_slice(&1u32.to_be_bytes());
    profile[132..136].copy_from_slice(b"desc");
    profile[136..140].copy_from_slice(&144u32.to_be_bytes());
    profile[140..144].copy_from_slice(&(tag_size as u32).to_be_bytes());
    let tag = &mut profile[144..];
    tag[..4].copy_from_slice(b"mluc");
    tag[8..12].copy_from_slice(&(records as u32).to_be_bytes());
    tag[12..16].copy_from_slice(&12u32.to_be_bytes());
    for index in 0..records {
        let start = 16 + index * 12;
        tag[start..start + 4].copy_from_slice(b"enUS");
        tag[start + 4..start + 8].copy_from_slice(&(payload as u32).to_be_bytes());
        tag[start + 8..start + 12].copy_from_slice(&(strings as u32).to_be_bytes());
    }
    for pair in tag[strings..].as_chunks_mut::<2>().0.iter_mut() {
        pair.copy_from_slice(&[0, b'A']);
    }
    Ok(profile)
}
fn memory<T>(result: Result<T, RasterError>) {
    assert!(matches!(result, Err(RasterError::Memory { estimated, budget }) if estimated > budget));
}
struct UnreadSource {
    info: SourceInfo,
    reads: usize,
}
impl UnreadSource {
    fn from_image(image: &DecodedImage) -> Self {
        Self {
            info: SourceInfo {
                width: image.width,
                height: image.height,
                bit_depth: image.pixels.bit_depth(),
                icc: image.icc.clone(),
                source_asset: image.source_asset.clone(),
                original_available: true,
                orientation_applied: 1,
            },
            reads: 0,
        }
    }
}
impl RasterSource for UnreadSource {
    fn info(&self) -> &SourceInfo {
        &self.info
    }
    fn resident_bytes(&self) -> u64 {
        self.info.icc.as_ref().map_or(0, |p| p.capacity() as u64)
    }
    fn read_workspace_bytes(&self) -> u64 {
        0
    }
    fn read_region(
        &mut self,
        _: Region,
        _: u64,
        _: &dyn Fn() -> bool,
    ) -> Result<DecodedImage, RasterError> {
        self.reads += 1;
        Err(RasterError::MissingAsset)
    }
}
#[derive(Default)]
struct UnreadAssets(Cell<usize>);
impl AssetResolver for UnreadAssets {
    fn image(&self, _: &str) -> Result<DecodedImage, RasterError> {
        self.0.set(self.0.get() + 1);
        Err(RasterError::MissingAsset)
    }
}

#[test]
fn buffered_export_and_render_charge_alias_expansion_before_semantic_validation() -> TestResult {
    let mut source = source(1, 1, 8);
    source.icc = Some(alias_profile()?);
    assert!(
        matches!(source.validate(), Err(RasterError::Color)),
        "fixture must distinguish semantic parsing from structural admission"
    );
    let before = source.clone();
    let mut req = request(ExportFormat::Png8);
    req.memory_budget_bytes = 33 * 1024 * 1024;
    memory(export(&source, &req));
    let (project, document, _) = project(&source)?;
    memory(render_document(
        &source,
        &project,
        &document,
        &UnreadAssets::default(),
        RenderOptions {
            memory_budget_bytes: req.memory_budget_bytes,
            ..options()
        },
    ));
    assert_eq!(source, before);
    Ok(())
}

#[test]
fn clean_and_marked_stream_plans_reject_before_color_parser_rows_or_output() -> TestResult {
    let mut image = source(1, 1, 8);
    image.icc = Some(alias_profile()?);
    let (project, document, _) = project(&image)?;
    let mut source = UnreadSource::from_image(&image);
    let mut req = request(ExportFormat::Png8);
    req.memory_budget_bytes = 33 * 1024 * 1024;
    let limits = StreamLimits {
        max_strip_rows: 1,
        ..StreamLimits::default()
    };
    memory(plan_png(&source, &req, limits));
    let mut output = Vec::new();
    memory(export_png_to(
        &mut source,
        &req,
        limits,
        &mut output,
        &|| false,
    ));
    assert!(output.is_empty());
    memory(export_document_png_to(
        &mut source,
        MarkedDocument {
            project: &project,
            document: &document,
            assets: &UnreadAssets::default(),
            options: RenderOptions {
                memory_budget_bytes: req.memory_budget_bytes,
                ..options()
            },
        },
        &req,
        limits,
        &mut output,
        &|| false,
    ));
    assert_eq!(source.reads, 0);
    assert!(output.is_empty());
    Ok(())
}

#[test]
fn visible_resolver_profiles_are_admitted_before_asset_resolution_in_both_paths() -> TestResult {
    let image = source(1, 1, 8);
    let (mut project, document, layer) = project(&image)?;
    let result_asset = AssetId::hash(b"synthetic aliased result profile");
    let mut binding = asset(&image);
    binding.asset_id = result_asset.to_string();
    binding.icc_profile = alias_profile()?;
    project.assets.insert(result_asset.clone(), binding);
    let result_id = id(20)?;
    project.results.insert(
        result_id.clone(),
        ResultCandidate {
            definition: v1::AddResult {
                result_id: Some(result_id.to_proto()),
                document_id: Some(document.to_proto()),
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
    let (object_id, object) = object(&document, &layer, 10, Shape::ResultId(result_id.to_proto()))?;
    project.objects.insert(object_id, object);
    project.validate()?;
    let assets = UnreadAssets::default();
    let budget = 65 * 1024 * 1024;
    memory(render_document(
        &image,
        &project,
        &document,
        &assets,
        RenderOptions {
            memory_budget_bytes: budget,
            ..options()
        },
    ));
    assert_eq!(assets.0.get(), 0);
    let mut req = request(ExportFormat::Png8);
    req.memory_budget_bytes = budget;
    let mut source = BorrowedSource::new(&image)?;
    let mut output = Vec::new();
    memory(export_document_png_to(
        &mut source,
        MarkedDocument {
            project: &project,
            document: &document,
            assets: &assets,
            options: RenderOptions {
                memory_budget_bytes: budget,
                ..options()
            },
        },
        &req,
        StreamLimits {
            max_strip_rows: 1,
            ..StreamLimits::default()
        },
        &mut output,
        &|| false,
    ));
    assert_eq!(assets.0.get(), 0);
    assert!(output.is_empty());
    Ok(())
}

#[test]
fn truncated_pixel_buffers_fail_before_region_alpha_indexing() -> TestResult {
    let mut source = source(2, 2, 8);
    source.pixels = Pixels::Rgba8(vec![0; 3]);
    source.icc = Some(alias_profile()?);
    assert!(matches!(
        export(&source, &request(ExportFormat::Png8)),
        Err(RasterError::Invalid("pixel buffer length"))
    ));
    Ok(())
}
