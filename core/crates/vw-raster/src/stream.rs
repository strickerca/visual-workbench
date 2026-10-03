use crate::{
    AlphaPolicy, AssetResolver, ColorPolicy, DecodedImage, ExportFormat, ExportMetadata,
    ExportRequest, RasterError, RasterSource, Region, RenderOptions, SourceInfo,
    source::{check_cancel, check_memory, reserve},
};
use std::io::{self, Write};
use vw_model::{AssetId, Id, Project};
use vw_proto::v1::{self, object_state::Shape};

/// Encoded output is streamed to the caller's private sink; it is never retained
/// by this crate. A Vec sink's allocation is therefore the caller's responsibility.
#[derive(Debug, Clone, Copy)]
pub struct StreamLimits {
    pub max_encoded_bytes: u64,
    pub max_strip_rows: u32,
    pub max_pixels: u64,
}
impl Default for StreamLimits {
    fn default() -> Self {
        Self {
            max_encoded_bytes: 512 * 1024 * 1024,
            max_strip_rows: 128,
            max_pixels: 50_000_000,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct StreamPlan {
    pub region: Region,
    pub strip_rows: u32,
    /// Conservative accounted working set, not measured process RSS. Includes
    /// retained source bytes, input rows, encoder rows/state, color/geometry
    /// reserve and 64 bytes per strip pixel. Caller-owned project/sink excluded.
    pub estimated_peak_bytes: u64,
}
#[derive(Debug)]
pub struct StreamReport {
    pub metadata: ExportMetadata,
    pub plan: StreamPlan,
    pub encoded_bytes: u64,
    pub strips: u32,
}
pub struct MarkedDocument<'a, A: AssetResolver> {
    pub project: &'a Project,
    pub document: &'a Id,
    pub assets: &'a A,
    pub options: RenderOptions,
}

/// Preflight for clean PNG. Marked export additionally reserves shape workspace
/// and checks source/project binding before touching output. No implicit resize.
pub fn plan_png(
    source: &impl RasterSource,
    request: &ExportRequest,
    limits: StreamLimits,
) -> Result<StreamPlan, RasterError> {
    plan(source, request, limits, 16 * 1024 * 1024)
}
fn plan(
    source: &impl RasterSource,
    r: &ExportRequest,
    limits: StreamLimits,
    geometry: u64,
) -> Result<StreamPlan, RasterError> {
    let info = source.info();
    crate::checked_samples(info.width, info.height)?;
    if r.revision.state_hash.len() != 32 || r.capture_session.is_some() != r.frame_id.is_some() {
        return Err(RasterError::Invalid("revision/capture identity"));
    }
    if !matches!(r.format, ExportFormat::Png8 | ExportFormat::Png16) {
        return Err(RasterError::Unsupported(
            "bounded export supports PNG8/16; other codecs require buffered export",
        ));
    }
    if info.bit_depth == 16 && r.format == ExportFormat::Png8 && !r.allow_depth_reduction {
        return Err(RasterError::Depth);
    }
    if limits.max_strip_rows == 0 || limits.max_encoded_bytes == 0 {
        return Err(RasterError::Invalid("stream limits"));
    }
    if u64::from(info.width) * u64::from(info.height) > limits.max_pixels {
        return Err(RasterError::Unsupported(
            "image exceeds pixel limit; split or use tiled import",
        ));
    }
    let region = r.region.unwrap_or(Region {
        x: 0,
        y: 0,
        width: info.width,
        height: info.height,
    });
    region.validate(info.width, info.height)?;
    if region.width > 0x7fff_ffff || region.height > 0x7fff_ffff {
        return Err(RasterError::Dimensions {
            format: "PNG",
            limit: 0x7fff_ffff,
        });
    }
    // PNG StreamWriter allocates previous/current/filter rows. Our conversion
    // row is the fourth. The fixed reserve includes deflate/chunk/ICC machinery.
    let row = u64::from(region.width)
        * if r.format == ExportFormat::Png16 {
            8
        } else {
            4
        };
    let profile = crate::export::icc_workspace(info.icc.as_ref())?;
    let fixed = source
        .resident_bytes()
        .checked_add(source.read_workspace_bytes())
        .and_then(|v| v.checked_add(row * 4))
        .and_then(|v| v.checked_add(profile))
        .and_then(|v| v.checked_add(16 * 1024 * 1024))
        .and_then(|v| v.checked_add(geometry))
        .ok_or(RasterError::Allocation)?;
    let per_row = u64::from(region.width) * 64;
    check_memory(
        fixed.checked_add(per_row).ok_or(RasterError::Allocation)?,
        r.memory_budget_bytes,
    )?;
    let rows = ((r.memory_budget_bytes - fixed) / per_row)
        .min(u64::from(limits.max_strip_rows))
        .min(u64::from(region.height)) as u32;
    // Only after all retained bytes, aliases, strip and codec work have passed
    // the caller's budget may SourceInfo invoke the semantic ICC parser.
    info.validate()?;
    Ok(StreamPlan {
        region,
        strip_rows: rows,
        estimated_peak_bytes: fixed + per_row * u64::from(rows),
    })
}

pub fn export_png_to(
    source: &mut impl RasterSource,
    request: &ExportRequest,
    limits: StreamLimits,
    output: &mut impl Write,
    cancelled: &dyn Fn() -> bool,
) -> Result<StreamReport, RasterError> {
    check_cancel(cancelled)?;
    let plan = plan_png(source, request, limits)?;
    export_regions(
        source,
        request,
        limits,
        plan,
        output,
        cancelled,
        |image, _, _| Ok(image),
    )
}

/// Renders source-resolution strips using the same layer/ink/text compositor as
/// render_document. On any error/cancellation the caller must discard its partial
/// output. No output file is published or overwritten by this crate.
pub fn export_document_png_to<A: AssetResolver>(
    source: &mut impl RasterSource,
    document: MarkedDocument<'_, A>,
    request: &ExportRequest,
    limits: StreamLimits,
    output: &mut impl Write,
    cancelled: &dyn Fn() -> bool,
) -> Result<StreamReport, RasterError> {
    check_cancel(cancelled)?;
    validate_document(source.info(), &document)?;
    let mut request = request.clone();
    request.memory_budget_bytes = request
        .memory_budget_bytes
        .min(document.options.memory_budget_bytes);
    let resolver_bytes = resolver_reserve(
        document.project,
        document.document,
        document.options.include_guides,
    )?;
    let plan = plan(
        source,
        &request,
        limits,
        geometry_reserve(
            document.project,
            document.document,
            document.options.include_guides,
        )?
        .checked_add(resolver_bytes)
        .ok_or(RasterError::Allocation)?,
    )?;
    export_regions(
        source,
        &request,
        limits,
        plan,
        output,
        cancelled,
        |image, origin, estimated| {
            crate::render::render_buffer(
                &image,
                document.project,
                document.document,
                document.assets,
                RenderOptions {
                    memory_budget_bytes: request.memory_budget_bytes,
                    ..document.options
                },
                crate::render::RegionRender {
                    origin,
                    estimated: estimated - resolver_bytes,
                    cancelled,
                },
            )
        },
    )
}

fn validate_document<A: AssetResolver>(
    info: &SourceInfo,
    document: &MarkedDocument<'_, A>,
) -> Result<(), RasterError> {
    crate::checked_samples(info.width, info.height)?;
    document.project.validate()?;
    let definition = &document
        .project
        .documents
        .get(document.document)
        .ok_or(RasterError::Invalid("document"))?
        .definition;
    if definition.primary_asset_id != info.source_asset.as_str() {
        return Err(RasterError::Invalid("document source binding"));
    }
    if !matches!(
        v1::DocumentKind::try_from(definition.kind),
        Ok(v1::DocumentKind::Image | v1::DocumentKind::Capture)
    ) {
        return Err(RasterError::Unsupported(
            "PDF/SVG page rasterization belongs to their import modules",
        ));
    }
    let asset = document
        .project
        .assets
        .get(&info.source_asset)
        .ok_or(RasterError::MissingAsset)?;
    let dimensions = if asset.orientation >= 5 {
        (asset.height, asset.width)
    } else {
        (asset.width, asset.height)
    };
    if dimensions != (info.width, info.height)
        || asset.orientation != u32::from(info.orientation_applied)
        || asset.bit_depth.max(8) != u32::from(info.bit_depth)
        || asset.icc_profile.as_slice() != info.icc.as_deref().unwrap_or_default()
    {
        return Err(RasterError::Invalid("document source metadata"));
    }
    Ok(())
}
/// Match render_buffer's document, layer, object and guide visibility before
/// charging workspace. Both callers validate the project first, so malformed
/// layer identities cannot bypass model validation through this iterator.
fn admitted_objects<'a>(
    project: &'a Project,
    document: &'a Id,
    include_guides: bool,
) -> impl Iterator<Item = &'a vw_model::Object> + 'a {
    project.objects.values().filter(move |object| {
        if &object.document_id != document || object.state.hidden {
            return false;
        }
        if !include_guides
            && matches!(
                object.state.shape,
                Some(Shape::SelectionVector(_) | Shape::SelectionRaster(_) | Shape::Crop(_))
            )
        {
            return false;
        }
        Id::from_proto(object.state.layer_id.as_ref())
            .ok()
            .and_then(|id| project.layers.get(&id))
            .is_some_and(|layer| {
                layer.definition.document_id.as_ref() == Some(&document.to_proto())
                    && layer.visible
                    && layer.opacity > 0.0
            })
    })
}

pub(crate) fn geometry_reserve(
    project: &Project,
    document: &Id,
    include_guides: bool,
) -> Result<u64, RasterError> {
    let mut peak = 16 * 1024 * 1024;
    for object in admitted_objects(project, document, include_guides) {
        // Bound temporary contour/path/stroker expansion before allocating it.
        // Deliberately conservative: a huge persisted shape may require a larger
        // budget even though the source itself can be streamed.
        let bytes = match &object.state.shape {
            Some(Shape::Stroke(s)) => (s.x.len() as u64).checked_mul(16384),
            Some(Shape::Text(t)) => (t.text.len() as u64).checked_mul(65536),
            Some(
                Shape::Line(p) | Shape::Arrow(p) | Shape::Polygon(p) | Shape::SelectionVector(p),
            ) => (p.points.len() as u64).checked_mul(1024),
            _ => Some(0),
        }
        .ok_or(RasterError::Allocation)?;
        peak = peak.max(bytes);
    }
    peak.checked_add((project.objects.len() as u64 + project.layers.len() as u64) * 32)
        .ok_or(RasterError::Allocation)
}
fn resolver_reserve(
    project: &Project,
    document: &Id,
    include_guides: bool,
) -> Result<u64, RasterError> {
    let mut peak = 0;
    for object in admitted_objects(project, document, include_guides) {
        let Some(Shape::ResultId(id)) = &object.state.shape else {
            continue;
        };
        let result = project
            .results
            .get(&Id::from_proto(Some(id))?)
            .ok_or(RasterError::MissingAsset)?;
        let asset = if result.definition.composite_asset_id.is_empty() {
            &result.definition.output_asset_id
        } else {
            &result.definition.composite_asset_id
        };
        let binding = project
            .assets
            .get(&AssetId::try_from(asset.clone())?)
            .ok_or(RasterError::MissingAsset)?;
        let profile = crate::export::icc_workspace(
            (!binding.icc_profile.is_empty()).then_some(&binding.icc_profile),
        )?;
        let bytes = u64::from(binding.width)
            .checked_mul(u64::from(binding.height))
            .and_then(|v| v.checked_mul(32))
            .and_then(|v| v.checked_add(binding.byte_size))
            .and_then(|v| v.checked_add(32 * 1024 * 1024))
            .and_then(|v| v.checked_add(profile))
            .ok_or(RasterError::Allocation)?;
        peak = peak.max(bytes);
    }
    Ok(peak)
}

fn export_regions(
    source: &mut impl RasterSource,
    r: &ExportRequest,
    limits: StreamLimits,
    plan: StreamPlan,
    output: &mut impl Write,
    cancelled: &dyn Fn() -> bool,
    mut render: impl FnMut(DecodedImage, (u32, u32), u64) -> Result<DecodedImage, RasterError>,
) -> Result<StreamReport, RasterError> {
    let source_info = source.info().clone();
    let output_icc = match r.color {
        ColorPolicy::Preserve => source_info.icc.clone(),
        ColorPolicy::ConvertToSrgb {
            assume_untagged_srgb,
        } => {
            if source_info.icc.is_none() && !assume_untagged_srgb {
                return Err(RasterError::Unsupported(
                    "untagged color; explicitly assume sRGB",
                ));
            }
            Some(crate::pixels::standard_srgb_profile()?)
        }
    };
    let gray = output_icc
        .as_ref()
        .map(|p| {
            crate::pixels::parse_color_profile(p)
                .map(|p| p.color_space == moxcms::DataColorSpace::Gray)
        })
        .transpose()?
        .unwrap_or(false);
    if let AlphaPolicy::Matte(m) = r.alpha
        && gray
        && (m[0] != m[1] || m[0] != m[2])
    {
        return Err(RasterError::Unsupported(
            "colored matte with grayscale profile; convert to sRGB first",
        ));
    }
    let meta = export_metadata(&source_info, output_icc.as_deref(), r, plan.region);
    let json = serde_json::to_string(&meta).map_err(|_| RasterError::Metadata)?;
    let mut limited = LimitedWriter {
        output,
        max: limits.max_encoded_bytes,
        written: 0,
        failure: None,
        cancelled,
    };
    let mut strips = 0;
    let result: Result<(), RasterError> = (|| {
        let mut info = png::Info::with_size(plan.region.width, plan.region.height);
        info.color_type = if gray {
            png::ColorType::GrayscaleAlpha
        } else {
            png::ColorType::Rgba
        };
        info.bit_depth = if r.format == ExportFormat::Png16 {
            png::BitDepth::Sixteen
        } else {
            png::BitDepth::Eight
        };
        info.icc_profile = output_icc.as_deref().map(std::borrow::Cow::Borrowed);
        let mut encoder =
            png::Encoder::with_info(&mut limited, info).map_err(|_| RasterError::Codec)?;
        encoder.set_compression(png::Compression::Balanced);
        encoder.set_filter(png::Filter::Paeth);
        encoder
            .add_itxt_chunk("VisualWorkbench".into(), json)
            .map_err(|_| RasterError::Metadata)?;
        let mut writer = encoder.write_header().map_err(|_| RasterError::Codec)?;
        {
            let mut stream = writer
                .stream_writer_with_size(64 * 1024)
                .map_err(|_| RasterError::Codec)?;
            let mut row = reserve::<u8>(
                plan.region.width as usize
                    * if r.format == ExportFormat::Png16 {
                        8
                    } else {
                        4
                    },
            )?;
            let mut y = plan.region.y;
            while y < plan.region.y + plan.region.height {
                check_cancel(cancelled)?;
                let region = Region {
                    x: plan.region.x,
                    y,
                    width: plan.region.width,
                    height: plan.strip_rows.min(plan.region.y + plan.region.height - y),
                };
                let input = source.read_region(region, r.memory_budget_bytes, cancelled)?;
                crate::export::image_shape(&input)?;
                if input.width != region.width
                    || input.height != region.height
                    || input.pixels.bit_depth() != source_info.bit_depth
                    || input.source_asset != source_info.source_asset
                    || input.icc != source_info.icc
                    || !input.original_available
                    || input.orientation_applied != source_info.orientation_applied
                {
                    return Err(RasterError::Invalid("source region binding"));
                }
                let expected_profile = crate::export::icc_workspace(source_info.icc.as_ref())?;
                let actual_profile = crate::export::icc_workspace(input.icc.as_ref())?;
                check_memory(
                    plan.estimated_peak_bytes
                        .checked_add(actual_profile.saturating_sub(expected_profile))
                        .ok_or(RasterError::Allocation)?,
                    r.memory_budget_bytes,
                )?;
                input.validate()?;
                let mut image = render(input, (region.x, region.y), plan.estimated_peak_bytes)?;
                if let ColorPolicy::ConvertToSrgb {
                    assume_untagged_srgb,
                } = r.color
                {
                    image.convert_profile(None, assume_untagged_srgb)?;
                }
                apply_matte(&mut image, r.alpha);
                for samples in
                    (0..image.height).map(|line| line as usize * image.width as usize * 4)
                {
                    check_cancel(cancelled)?;
                    row.clear();
                    for at in (samples..samples + image.width as usize * 4).step_by(4) {
                        for channel in 0..4 {
                            if gray && (channel == 1 || channel == 2) {
                                continue;
                            }
                            let value = image.pixels.sample(at + channel);
                            if r.format == ExportFormat::Png16 {
                                row.extend_from_slice(&value.to_be_bytes());
                            } else {
                                row.push(((u32::from(value) + 128) / 257) as u8);
                            }
                        }
                    }
                    stream.write_all(&row).map_err(|_| RasterError::Io)?;
                }
                y += region.height;
                strips += 1;
            }
            stream.finish().map_err(|_| RasterError::Codec)?;
        }
        writer.finish().map_err(|_| RasterError::Codec)?;
        check_cancel(cancelled)?;
        limited.flush().map_err(|_| RasterError::Io)?;
        Ok(())
    })();
    if let Some(error) = limited.failure {
        return Err(error);
    }
    result?;
    Ok(StreamReport {
        metadata: meta,
        plan,
        encoded_bytes: limited.written,
        strips,
    })
}
fn apply_matte(image: &mut DecodedImage, alpha: AlphaPolicy) {
    if let AlphaPolicy::Matte(matte) = alpha {
        for i in (0..image.pixels.len()).step_by(4) {
            let a = u64::from(image.pixels.sample(i + 3));
            for (c, m) in matte.iter().enumerate() {
                let v = (u64::from(image.pixels.sample(i + c)) * a
                    + u64::from(*m) * 257 * (65535 - a)
                    + 32767)
                    / 65535;
                image.pixels.set_sample(i + c, v as u16);
            }
            image.pixels.set_sample(i + 3, 65535);
        }
    }
}
fn export_metadata(
    info: &SourceInfo,
    output_icc: Option<&[u8]>,
    r: &ExportRequest,
    region: Region,
) -> ExportMetadata {
    let hash8 = r
        .revision
        .state_hash
        .iter()
        .take(4)
        .map(|v| format!("{v:02x}"))
        .collect::<String>();
    ExportMetadata {
        schema_version: 1,
        revision: format!("r{}-{hash8}", r.revision.host_seq),
        source_asset: info.source_asset.to_string(),
        source_width: info.width,
        source_height: info.height,
        output_width: region.width,
        output_height: region.height,
        source_bit_depth: info.bit_depth,
        orientation_applied: info.orientation_applied,
        source_icc_hash: info.icc.as_ref().map(|v| AssetId::hash(v).to_string()),
        output_icc_hash: output_icc.map(|v| AssetId::hash(v).to_string()),
        color_conversion: match r.color {
            ColorPolicy::Preserve => "none",
            ColorPolicy::ConvertToSrgb { .. } if info.icc.is_some() => "ICC to sRGB",
            _ => "untagged explicitly assumed sRGB",
        }
        .into(),
        settings: r.clone(),
    }
}
struct LimitedWriter<'a, W: Write> {
    output: &'a mut W,
    max: u64,
    written: u64,
    failure: Option<RasterError>,
    cancelled: &'a dyn Fn() -> bool,
}
impl<W: Write> Write for LimitedWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.failure.is_some() {
            return Err(io::Error::other("raster output already failed"));
        }
        if (self.cancelled)() {
            self.failure = Some(RasterError::Cancelled);
            return Err(io::Error::other("raster cancelled"));
        }
        if bytes.len() as u64 > self.max - self.written {
            self.failure = Some(RasterError::EncodedLimit { limit: self.max });
            return Err(io::Error::other("raster output limit"));
        }
        match self.output.write(bytes) {
            Ok(n) => {
                self.written += n as u64;
                Ok(n)
            }
            Err(error) => {
                self.failure = Some(RasterError::Io);
                Err(error)
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.failure.is_some() {
            return Err(io::Error::other("raster output already failed"));
        }
        if (self.cancelled)() {
            self.failure = Some(RasterError::Cancelled);
            return Err(io::Error::other("raster cancelled"));
        }
        self.output.flush().inspect_err(|_| {
            self.failure = Some(RasterError::Io);
        })
    }
}
