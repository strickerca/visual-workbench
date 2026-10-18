use crate::{
    Cancellation, CompileOptions, Error, ImageMapping, Limits, Package, Result, Target,
    UNTRUSTED_TEXT_NOTICE, bounded, check, config::timestamp, geometry, manifest::*, pixels,
    prompt, verify,
};
use std::collections::BTreeMap;
use vw_instructions::{DocumentView, InstructionExport};
use vw_model::{AssetId, Id, Project};
use vw_proto::v1;
use vw_raster::{
    AlphaPolicy, ColorPolicy, DecodeLimits, DecodedImage, ExportFormat, ExportRequest, Pixels,
    RenderOptions,
};

pub struct CompileInput<'a> {
    pub project: &'a Project,
    pub revision: &'a v1::Revision,
    pub document: &'a Id,
    /// Exact immutable encoded originals, already bounded by the caller's IO.
    /// Only supply originals required by this document; all entries are hashed.
    pub originals: &'a BTreeMap<AssetId, Vec<u8>>,
}
struct Resolver<'a> {
    originals: &'a BTreeMap<AssetId, Vec<u8>>,
    limits: DecodeLimits,
    cancel: &'a dyn Cancellation,
}
impl vw_raster::AssetResolver for Resolver<'_> {
    fn image(&self, id: &str) -> std::result::Result<DecodedImage, vw_raster::RasterError> {
        if self.cancel.is_cancelled() {
            return Err(vw_raster::RasterError::Cancelled);
        }
        let key =
            AssetId::try_from(id.to_owned()).map_err(|_| vw_raster::RasterError::MissingAsset)?;
        vw_raster::decode(
            self.originals
                .get(&key)
                .ok_or(vw_raster::RasterError::MissingAsset)?,
            self.limits,
        )
    }
}
fn resident(image: &DecodedImage) -> u64 {
    let pixels = match &image.pixels {
        Pixels::Rgba8(p) => p.capacity() as u64,
        Pixels::Rgba16(p) => p.capacity() as u64 * 2,
    };
    pixels + image.icc.as_ref().map_or(0, |p| p.capacity() as u64)
}
fn srgb(
    source: &DecodedImage,
    revision: &v1::Revision,
    options: &CompileOptions,
    retained: u64,
) -> Result<DecodedImage> {
    let budget = options.limits.working(retained)?;
    let encoded = vw_raster::export(
        source,
        &ExportRequest {
            format: ExportFormat::Png8,
            region: None,
            revision: revision.clone(),
            alpha: AlphaPolicy::Preserve,
            color: ColorPolicy::ConvertToSrgb {
                assume_untagged_srgb: options.assume_untagged_srgb,
            },
            allow_depth_reduction: options.allow_depth_reduction,
            memory_budget_bytes: budget,
            capture_session: None,
            frame_id: None,
        },
    )?;
    let remaining = options.limits.working(
        retained
            .checked_add(encoded.bytes.capacity() as u64)
            .ok_or(Error::Limit("memory"))?,
    )?;
    Ok(vw_raster::decode(
        &encoded.bytes,
        DecodeLimits {
            max_encoded_bytes: options.limits.source_bytes,
            max_pixels: options.limits.source_pixels,
            max_memory_bytes: remaining,
        },
    )?)
}
fn add(
    files: &mut BTreeMap<String, Vec<u8>>,
    path: String,
    bytes: Vec<u8>,
    limit: usize,
) -> Result<String> {
    if files.contains_key(&path) {
        return Err(Error::Invalid("duplicate output"));
    }
    let current = files.values().try_fold(0usize, |n, v| {
        n.checked_add(v.len()).ok_or(Error::Limit("package size"))
    })?;
    if current.checked_add(bytes.len()).is_none_or(|n| n > limit) {
        return Err(Error::Limit("package size"));
    }
    let hash = bounded::hash(&bytes);
    files.insert(path, bytes);
    Ok(hash)
}
struct ImageContext<'a> {
    binding: &'a vw_instructions::SourceBinding,
    asset: &'a str,
    limits: Limits,
}
fn image_file(
    files: &mut BTreeMap<String, Vec<u8>>,
    frame: &pixels::Frame,
    profile: Option<&[u8]>,
    identity: (String, &str, Option<u32>),
    map: &ImageMapping,
    context: &ImageContext<'_>,
) -> Result<PackageImage> {
    let (id, role, marker) = identity;
    let ImageContext {
        binding,
        asset,
        limits,
    } = context;
    let metadata = bounded::json(
        &serde_json::json!({"schema":1,"project_id":binding.project_id,"document_id":binding.document_id,"host_seq":binding.host_seq,"state_hash":binding.state_hash,"source_asset":asset,"mapping":map}),
        8192,
    )?;
    let bytes = pixels::encode(
        frame,
        profile,
        std::str::from_utf8(&metadata).map_err(|_| Error::Encoding)?,
        limits.image_bytes,
    )?;
    let path = format!("images/{id}.png");
    let sha256 = add(files, path.clone(), bytes, limits.package_bytes)?;
    Ok(PackageImage {
        id,
        role: role.into(),
        path,
        width: frame.width,
        height: frame.height,
        sha256,
        color_space: "sRGB".into(),
        marker,
    })
}
/// Immutable borrowed state prevents a concurrent writer from changing any
/// input during compilation. Applications obtain it under their snapshot lock.
pub fn compile(
    input: CompileInput<'_>,
    options: CompileOptions,
    cancel: &dyn Cancellation,
) -> Result<Package> {
    check(cancel)?;
    options.limits.validate()?;
    options.target.validate()?;
    let created_at = timestamp(options.created_at_ms)?;
    let mut encoded_bytes = 0u64;
    let mut source_bytes = 0u64;
    for (id, bytes) in input.originals {
        check(cancel)?;
        source_bytes = source_bytes
            .checked_add(bytes.len() as u64)
            .ok_or(Error::Limit("original bytes"))?;
        encoded_bytes = encoded_bytes
            .checked_add(bytes.capacity() as u64)
            .ok_or(Error::Limit("memory"))?;
        if source_bytes > options.limits.source_bytes as u64 {
            return Err(Error::Limit("original bytes"));
        }
        options.limits.working(encoded_bytes)?;
        let asset = input.project.assets.get(id).ok_or(Error::Original)?;
        if asset.byte_size != bytes.len() as u64 || AssetId::hash(bytes) != *id {
            return Err(Error::Original);
        }
        if u64::from(asset.width) * u64::from(asset.height) > options.limits.source_pixels {
            return Err(Error::Limit("source pixels"));
        }
    }
    // Codec/clone work is admitted before invoking the model hash projection.
    options.limits.working(encoded_bytes)?;
    bounded::admit(input.project, 2 * 1024 * 1024)?;
    let view = DocumentView::new(input.project, input.revision, input.document)?;
    let instructions = InstructionExport::from_view(&view)?;
    if instructions.markers().len() > options.limits.markers {
        return Err(Error::Limit("markers"));
    }
    let definition = &input
        .project
        .documents
        .get(input.document)
        .ok_or(Error::Invalid("document"))?
        .definition;
    let kind = match v1::DocumentKind::try_from(definition.kind) {
        Ok(v1::DocumentKind::Image) => "image",
        Ok(v1::DocumentKind::Capture) => "capture",
        _ => {
            return Err(Error::Unsupported(
                "PDF/SVG rasterization requires their importer",
            ));
        }
    };
    let asset = AssetId::try_from(definition.primary_asset_id.clone())?;
    let original = input.originals.get(&asset).ok_or(Error::Original)?;
    let source = vw_raster::decode(
        original,
        DecodeLimits {
            max_encoded_bytes: options.limits.source_bytes,
            max_pixels: options.limits.source_pixels,
            max_memory_bytes: options.limits.working(encoded_bytes)?,
        },
    )?;
    let (width, height, depth) = (source.width, source.height, source.pixels.bit_depth());
    let metadata = input.project.assets.get(&asset).ok_or(Error::Original)?;
    let expected = if metadata.orientation >= 5 {
        (metadata.height, metadata.width)
    } else {
        (metadata.width, metadata.height)
    };
    if expected != (width, height)
        || metadata.orientation != u32::from(source.orientation_applied)
        || metadata.bit_depth.max(8) != u32::from(depth)
        || metadata.icc_profile.as_slice() != source.icc.as_deref().unwrap_or_default()
    {
        return Err(Error::Original);
    }
    if depth == 16 && !options.allow_depth_reduction {
        return Err(Error::Raster(vw_raster::RasterError::Depth));
    }
    let capture = definition
        .capture
        .as_ref()
        .map(|c| -> Result<Capture> {
            let rect = c
                .geometry
                .as_ref()
                .and_then(|g| g.client_rect_host.as_ref())
                .ok_or(Error::Invalid("capture geometry"))?;
            if i64::from(rect.w) != i64::from(width) || i64::from(rect.h) != i64::from(height) {
                return Err(Error::Stale);
            }
            Ok(Capture {
                platform: c.platform.clone(),
                session_id: Id::from_proto(c.capture_session_id.as_ref())?,
                frame_id: c.frame_id,
                app_name: c.app_name.clone(),
                window_title: options.include_window_title.then(|| c.window_title.clone()),
                captured_at: timestamp(c.captured_at_ms)?,
                lossless: c.lossless,
                degraded: c.degraded,
            })
        })
        .transpose()?;
    check(cancel)?;
    let semantics = options
        .semantic_snapshot
        .as_ref()
        .map(|id| -> Result<vw_semantics::Snapshot> {
            let current =
                vw_semantics::CaptureView::new(input.project, input.revision, input.document)?;
            let b = current.binding();
            let expected = instructions.source();
            if b.project_id != expected.project_id
                || b.document_id != expected.document_id
                || b.host_seq != expected.host_seq
                || b.state_hash != expected.state_hash
                || current.context().primary_asset_id() != asset.as_str()
            {
                return Err(Error::Stale);
            }
            Ok(current.load(id, cancel)?)
        })
        .transpose()?;
    let semantic_json = if let Some(snapshot) = &semantics {
        snapshot.export_all(cancel)?.semantic_json().to_vec()
    } else {
        bounded::json(
            &serde_json::json!({"schema_version":1,"project_id":input.project.id,"document_id":input.document,"text_is_untrusted":true,"untrusted_notice":UNTRUSTED_TEXT_NOTICE,"elements":[]}),
            8192,
        )?
    };
    let overview = ImageMapping::new(
        [0, 0, i64::from(width), i64::from(height)],
        options.target.max_long_edge(),
        64,
    )?;
    let mut markers = Vec::new();
    let mut crops = Vec::new();
    let mut reference_bytes = 0usize;
    for marker in instructions.markers() {
        check(cancel)?;
        let bounds =
            geometry::marker_bounds(marker.point_document, marker.bounds_document, width, height)?;
        let map = geometry::crop(bounds, width, height, options.target.max_long_edge())?;
        let refs = if marker.element_eids.is_empty() {
            Vec::new()
        } else {
            let snapshot = semantics
                .as_ref()
                .ok_or(Error::Invalid("unresolved element references"))?;
            let export = snapshot.export_references(&marker.element_eids, cancel)?;
            reference_bytes = reference_bytes
                .checked_add(export.semantic_json().len())
                .ok_or(Error::Limit("references"))?;
            if reference_bytes > 8 * 1024 * 1024 {
                return Err(Error::Limit("references"));
            }
            export
                .references()
                .iter()
                .map(ElementReference::from)
                .collect()
        };
        let instruction = instructions
            .instructions()
            .iter()
            .find(|i| i.id == marker.instruction_id)
            .ok_or(Error::Invalid("marker instruction"))?;
        let image = format!("marker_{}", marker.number);
        crops.push(CropMapping {
            marker: marker.number,
            image: image.clone(),
            mapping: map,
            bbox_crop_pixels: map.bounds(
                bounds,
                &Target::Generic {
                    max_long_edge: 8192,
                },
            ),
        });
        markers.push(PackageMarker {
            number: marker.number,
            kind: if marker.bounds_document.is_some() {
                "box"
            } else {
                "point"
            }
            .into(),
            role: marker.role.as_str().into(),
            instruction: instruction.text.clone(),
            bbox_compiled: overview.bounds(bounds, &options.target),
            bbox_document: bounds,
            point_compiled: overview.point(marker.point_document, &options.target),
            point_document: marker.point_document,
            element_refs: refs,
            crop_image: image,
        });
    }
    let mut files = BTreeMap::new();
    let mut images = Vec::new();
    let context = ImageContext {
        binding: instructions.source(),
        asset: asset.as_str(),
        limits: options.limits,
    };
    {
        let clean = srgb(
            &source,
            input.revision,
            &options,
            encoded_bytes + resident(&source),
        )?;
        check(cancel)?;
        let frame = pixels::resample(
            &clean,
            &overview,
            options
                .limits
                .working(encoded_bytes + resident(&source) + resident(&clean))?,
            cancel,
        )?;
        images.push(image_file(
            &mut files,
            &frame,
            clean.icc.as_deref(),
            ("clean_source".into(), "clean_source", None),
            &overview,
            &context,
        )?);
    }
    check(cancel)?;
    let render_budget = options.limits.working(encoded_bytes + resident(&source))?;
    let resolver = Resolver {
        originals: input.originals,
        limits: DecodeLimits {
            max_encoded_bytes: options.limits.source_bytes,
            max_pixels: options.limits.source_pixels,
            max_memory_bytes: render_budget,
        },
        cancel,
    };
    let marked = vw_raster::render_document(
        &source,
        input.project,
        input.document,
        &resolver,
        RenderOptions {
            memory_budget_bytes: render_budget,
            include_guides: true,
            assume_untagged_srgb: options.assume_untagged_srgb,
        },
    )?;
    check(cancel)?;
    let raster = srgb(
        &marked,
        input.revision,
        &options,
        encoded_bytes + resident(&source) + resident(&marked),
    )?;
    drop(marked);
    drop(source);
    check(cancel)?;
    let working = options.limits.working(encoded_bytes + resident(&raster))?;
    {
        let mut frame = pixels::resample(&raster, &overview, working, cancel)?;
        for (marker, original) in markers.iter().zip(instructions.markers()) {
            frame.marker(
                marker.number,
                original.role,
                marker.bbox_document,
                &overview,
            );
        }
        images.push(image_file(
            &mut files,
            &frame,
            raster.icc.as_deref(),
            ("overview".into(), "overview", None),
            &overview,
            &context,
        )?);
    }
    for ((marker, crop), original) in markers.iter().zip(&crops).zip(instructions.markers()) {
        check(cancel)?;
        let mut frame = pixels::resample(&raster, &crop.mapping, working, cancel)?;
        frame.marker(
            marker.number,
            original.role,
            marker.bbox_document,
            &crop.mapping,
        );
        images.push(image_file(
            &mut files,
            &frame,
            raster.icc.as_deref(),
            (crop.image.clone(), "marker_crop", Some(marker.number)),
            &crop.mapping,
            &context,
        )?);
    }
    drop(raster);
    check(cancel)?;
    let mut manifest = Manifest {
        schema_version: "vip-1".into(),
        package_id: options.package_id,
        created_at,
        source: Source {
            project_id: input.project.id.clone(),
            document_id: input.document.clone(),
            document_kind: kind.into(),
            revision: format!(
                "r{}-{}",
                input.revision.host_seq,
                &instructions.source().state_hash.as_str()[..8]
            ),
            asset_hash: asset.to_string(),
            width,
            height,
            units: "px".into(),
            capture,
        },
        compiled_for: CompiledFor {
            target: options.target.name().into(),
            model: options.target.model().map(str::to_owned),
            max_long_edge: options.target.max_long_edge(),
            coordinate_convention: options.target.convention().into(),
            scale: overview.scale(),
        },
        images,
        global_instruction: instructions.global().map(|i| i.text.clone()),
        constraints: vec!["preserve everything outside the change regions".into()],
        markers,
        files: Vec::new(),
        semantic_file: "semantic.json".into(),
        prompt_file: "prompt.md".into(),
        untrusted_text_notice: UNTRUSTED_TEXT_NOTICE.into(),
        redactions: if definition.capture.is_some() && !options.include_window_title {
            vec![Redaction {
                field: "source.capture.window_title".into(),
                reason: "owner did not include window title".into(),
            }]
        } else {
            Vec::new()
        },
        extensions: Extensions {
            compiler: "vw-package-1-area-integer".into(),
            state_hash: instructions.source().state_hash.to_string(),
            host_seq: input.revision.host_seq,
            target_profile: options.target,
            source_bit_depth: depth,
            derivative_bit_depth: 8,
            color_conversion: "sRGB derivative; immutable original unchanged".into(),
            overview_mapping: overview,
            crops,
            semantic_snapshot: options.semantic_snapshot,
            instructions: instructions
                .global()
                .into_iter()
                .chain(instructions.instructions())
                .map(|i| InstructionBinding {
                    instruction_id: i.id.clone(),
                    target_ids: i.target_ids.clone(),
                    entry_method: i.entry_method.as_str().into(),
                    language: i.language.clone(),
                    updated_at_ms: i.updated_at_ms,
                })
                .collect(),
        },
    };
    let prompt = prompt::build(input.project, &manifest, &instructions, cancel)?;
    add(
        &mut files,
        "prompt.md".into(),
        prompt,
        options.limits.package_bytes,
    )?;
    add(
        &mut files,
        "semantic.json".into(),
        semantic_json,
        options.limits.package_bytes,
    )?;
    manifest.files = files
        .iter()
        .map(|(path, bytes)| FileEntry {
            path: path.clone(),
            sha256: bounded::hash(bytes),
        })
        .collect();
    let bytes = bounded::json(&manifest, bounded::MAX_TEXT)?;
    add(
        &mut files,
        "manifest.json".into(),
        bytes,
        options.limits.package_bytes,
    )?;
    check(cancel)?;
    verify::from_compiler(files, options.limits, encoded_bytes, cancel)
}
