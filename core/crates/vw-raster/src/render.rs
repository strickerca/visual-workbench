use crate::{DecodedImage, FontFamily, Pixels, RasterError, layout_text, text};
use tiny_skia::{FillRule, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};
use vw_model::{Id, Project};
use vw_proto::v1::{self, object_state::Shape};

/// Asset reads remain outside this crate. Implementations must return originals,
/// with the requested content address; previews are rejected again here.
pub trait AssetResolver {
    fn image(&self, asset: &str) -> Result<DecodedImage, RasterError>;
}
#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    pub memory_budget_bytes: u64,
    /// Selection/mask/crop outlines are editor guides, omitted from normal marked exports.
    /// Explicit diagnostic output may include their outlines; masks themselves never
    /// become colored source pixels in a normal export.
    pub include_guides: bool,
    pub assume_untagged_srgb: bool,
}
impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            memory_budget_bytes: 1024 * 1024 * 1024,
            include_guides: false,
            assume_untagged_srgb: false,
        }
    }
}

/// Render at full D-space pixel resolution, with isolated layer opacity and stable
/// (fractional order, ID) ordering. Source alpha/16-bit samples remain exact wherever
/// no visible annotation covers them. Caller may subsequently export a clean region.
pub fn render_document(
    source: &DecodedImage,
    project: &Project,
    document: &Id,
    assets: &impl AssetResolver,
    options: RenderOptions,
) -> Result<DecodedImage, RasterError> {
    crate::export::image_shape(source)?;
    project.validate()?;
    if !source.original_available {
        return Err(RasterError::OriginalRequired);
    }
    let definition = &project
        .documents
        .get(document)
        .ok_or(RasterError::Invalid("document"))?
        .definition;
    if definition.primary_asset_id != source.source_asset.as_str() {
        return Err(RasterError::Invalid("document source binding"));
    }
    let asset = project
        .assets
        .get(&source.source_asset)
        .ok_or(RasterError::MissingAsset)?;
    validate_asset_metadata(source, asset, "document source metadata")?;
    if !matches!(
        v1::DocumentKind::try_from(definition.kind),
        Ok(v1::DocumentKind::Image | v1::DocumentKind::Capture)
    ) {
        return Err(RasterError::Unsupported(
            "PDF/SVG page rasterization belongs to their import modules",
        ));
    }
    let geometry = crate::stream::geometry_reserve(project, document, options.include_guides)?;
    let profile = crate::export::icc_workspace(source.icc.as_ref())?;
    let estimated = u64::from(source.width)
        .checked_mul(u64::from(source.height))
        .and_then(|v| v.checked_mul(64))
        .and_then(|v| v.checked_add(16 * 1024 * 1024))
        .and_then(|v| v.checked_add(geometry))
        .and_then(|v| v.checked_add(profile))
        .ok_or(RasterError::Allocation)?;
    if estimated > options.memory_budget_bytes {
        return Err(RasterError::Memory {
            estimated,
            budget: options.memory_budget_bytes,
        });
    }
    source.validate()?;
    render_buffer(
        source,
        project,
        document,
        assets,
        options,
        RegionRender {
            origin: (0, 0),
            estimated,
            cancelled: &|| false,
        },
    )
}

pub(crate) struct RegionRender<'a> {
    pub origin: (u32, u32),
    pub estimated: u64,
    pub cancelled: &'a dyn Fn() -> bool,
}

/// A validated region with its full-document origin. Compositing math is shared
/// by whole-buffer and streaming export; only raster viewport allocation differs.
pub(crate) fn render_buffer(
    source: &DecodedImage,
    project: &Project,
    document: &Id,
    assets: &impl AssetResolver,
    options: RenderOptions,
    region: RegionRender<'_>,
) -> Result<DecodedImage, RasterError> {
    let RegionRender {
        origin,
        estimated,
        cancelled,
    } = region;
    crate::source::check_cancel(cancelled)?;
    let mut result = source.clone();
    let mut layers = project
        .layers
        .iter()
        .filter(|(_, l)| {
            l.definition.document_id.as_ref() == Some(&document.to_proto())
                && l.visible
                && l.opacity > 0.0
        })
        .collect::<Vec<_>>();
    layers.sort_by(|(a, x), (b, y)| {
        x.definition
            .order_key
            .cmp(&y.definition.order_key)
            .then(a.cmp(b))
    });
    let mut scratch = Pixmap::new(source.width, source.height).ok_or(RasterError::Allocation)?;
    for (layer_id, layer) in layers {
        crate::source::check_cancel(cancelled)?;
        let mut objects = project
            .objects
            .iter()
            .filter(|(_, o)| {
                &o.document_id == document
                    && o.state.layer_id.as_ref() == Some(&layer_id.to_proto())
                    && !o.state.hidden
            })
            .collect::<Vec<_>>();
        objects.sort_by(|(a, x), (b, y)| x.state.order_key.cmp(&y.state.order_key).then(a.cmp(b)));
        if objects.len() > 10000 {
            return Err(RasterError::Invalid("visible objects per layer limit"));
        }
        if objects
            .iter()
            .any(|(_, o)| matches!(o.state.shape, Some(Shape::Adjustment(_))))
        {
            if objects.len() != 1 || layer.blend != "normal" {
                return Err(RasterError::Unsupported(
                    "adjustment requires its own normal-blend layer",
                ));
            }
            if let Some(Shape::Adjustment(value)) = &objects[0].1.state.shape {
                apply_adjustment(&mut result, value, layer.opacity)?;
            }
            continue;
        }
        let mut layer_image = blank_like(source)?;
        for (_, object) in objects {
            crate::source::check_cancel(cancelled)?;
            let state = &object.state;
            let shape = state.shape.as_ref().ok_or(RasterError::Invalid("shape"))?;
            if matches!(
                shape,
                Shape::SelectionVector(_) | Shape::SelectionRaster(_) | Shape::Crop(_)
            ) && !options.include_guides
            {
                continue;
            }
            if let Shape::ResultId(id) = shape {
                if source.icc.is_none() && !options.assume_untagged_srgb {
                    return Err(RasterError::Unsupported(
                        "untagged result color; explicitly assume sRGB",
                    ));
                }
                let id = Id::from_proto(Some(id))?;
                let candidate = project.results.get(&id).ok_or(RasterError::MissingAsset)?;
                if candidate.acceptance_mask_asset_id.is_some() || candidate.status == "partial" {
                    return Err(RasterError::Unsupported(
                        "partial result acceptance requires a materialized accepted composite",
                    ));
                }
                if !matches!(candidate.status.as_str(), "ready" | "accepted") {
                    return Err(RasterError::Unsupported("result is not ready or accepted"));
                }
                let asset = if candidate.definition.composite_asset_id.is_empty() {
                    &candidate.definition.output_asset_id
                } else {
                    &candidate.definition.composite_asset_id
                };
                let binding = project
                    .assets
                    .get(&vw_model::AssetId::try_from(asset.clone())?)
                    .ok_or(RasterError::MissingAsset)?;
                // Check the documented full-frame resolver/decoder boundary
                // before asking it to allocate an asset, not afterward.
                let profile = crate::export::icc_workspace(
                    (!binding.icc_profile.is_empty()).then_some(&binding.icc_profile),
                )?;
                let resolver_peak = u64::from(binding.width)
                    .checked_mul(u64::from(binding.height))
                    .and_then(|v| v.checked_mul(32))
                    .and_then(|v| v.checked_add(binding.byte_size))
                    .and_then(|v| v.checked_add(32 * 1024 * 1024))
                    .and_then(|v| v.checked_add(profile))
                    .and_then(|v| v.checked_add(estimated))
                    .ok_or(RasterError::Allocation)?;
                if resolver_peak > options.memory_budget_bytes {
                    return Err(RasterError::Memory {
                        estimated: resolver_peak,
                        budget: options.memory_budget_bytes,
                    });
                }
                let mut image = assets.image(asset)?;
                crate::export::image_shape(&image)?;
                let profile = crate::export::icc_workspace(image.icc.as_ref())?;
                let estimate = estimated
                    .checked_add(
                        u64::from(image.width)
                            .checked_mul(u64::from(image.height))
                            .and_then(|v| v.checked_mul(24))
                            .ok_or(RasterError::Allocation)?,
                    )
                    .and_then(|v| v.checked_add(profile))
                    .ok_or(RasterError::Allocation)?;
                if estimate > options.memory_budget_bytes {
                    return Err(RasterError::Memory {
                        estimated: estimate,
                        budget: options.memory_budget_bytes,
                    });
                }
                if !image.original_available {
                    return Err(RasterError::OriginalRequired);
                }
                if image.source_asset.as_str() != asset {
                    return Err(RasterError::Invalid("resolved asset hash"));
                }
                let metadata = project
                    .assets
                    .get(&image.source_asset)
                    .ok_or(RasterError::MissingAsset)?;
                validate_asset_metadata(&image, metadata, "resolved asset metadata")?;
                image.validate()?;
                if image.icc != source.icc {
                    image.convert_profile(source.icc.as_deref(), options.assume_untagged_srgb)?;
                }
                composite_transformed(
                    &mut layer_image,
                    &image,
                    state
                        .transform
                        .as_ref()
                        .ok_or(RasterError::Invalid("transform"))?,
                    origin,
                )?;
                continue;
            }
            let eraser = matches!(shape,Shape::Stroke(stroke) if stroke.brush.as_ref().is_some_and(|b|b.family=="vector_eraser"));
            if eraser {
                return Err(RasterError::Unsupported(
                    "unmaterialized vector eraser; create object deletion operations before export",
                ));
            }
            scratch.fill(tiny_skia::Color::TRANSPARENT);
            render_object(&mut scratch, state, origin, cancelled)?;
            let mut overlay = DecodedImage {
                width: source.width,
                height: source.height,
                pixels: Pixels::Rgba8(unpremultiply(scratch.data())),
                icc: None,
                source_asset: source.source_asset.clone(),
                original_available: true,
                orientation_applied: 1,
            };
            if source.icc.is_some() {
                overlay.convert_profile(source.icc.as_deref(), true)?;
            } else if !options.assume_untagged_srgb {
                return Err(RasterError::Unsupported(
                    "untagged annotation color; explicitly assume sRGB",
                ));
            }
            composite(&mut layer_image, &overlay, 1.0, false);
        }
        composite(
            &mut result,
            &layer_image,
            layer.opacity,
            layer.blend == "multiply",
        );
    }
    Ok(result)
}
fn validate_asset_metadata(
    image: &DecodedImage,
    asset: &v1::AddAsset,
    error: &'static str,
) -> Result<(), RasterError> {
    let expected = if asset.orientation >= 5 {
        (asset.height, asset.width)
    } else {
        (asset.width, asset.height)
    };
    if expected != (image.width, image.height)
        || asset.orientation != u32::from(image.orientation_applied)
        || asset.bit_depth.max(8) != u32::from(image.pixels.bit_depth())
        || asset.icc_profile.as_slice() != image.icc.as_deref().unwrap_or_default()
    {
        return Err(RasterError::Invalid(error));
    }
    Ok(())
}
fn blank_like(source: &DecodedImage) -> Result<DecodedImage, RasterError> {
    let n = crate::checked_samples(source.width, source.height)?;
    let pixels = if source.pixels.bit_depth() == 16 {
        Pixels::Rgba16(vec![0; n])
    } else {
        Pixels::Rgba8(vec![0; n])
    };
    Ok(DecodedImage {
        width: source.width,
        height: source.height,
        pixels,
        icc: source.icc.clone(),
        source_asset: source.source_asset.clone(),
        original_available: true,
        orientation_applied: source.orientation_applied,
    })
}
fn unpremultiply(p: &[u8]) -> Vec<u8> {
    p.as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| {
            let a = u32::from(p[3]);
            let channel = |value| {
                (u32::from(value) * 255 + a / 2)
                    .checked_div(a)
                    .unwrap_or(0)
                    .min(255) as u8
            };
            [channel(p[0]), channel(p[1]), channel(p[2]), p[3]]
        })
        .collect()
}
fn rounded(v: f64) -> u16 {
    libm::round(v.clamp(0.0, 1.0) * 65535.0) as u16
}
fn composite(dst: &mut DecodedImage, src: &DecodedImage, opacity: f64, multiply: bool) {
    for i in (0..dst.pixels.len()).step_by(4) {
        blend_pixel(&mut dst.pixels, i, &src.pixels, i, opacity, multiply);
    }
}
fn blend_pixel(dst: &mut Pixels, i: usize, src: &Pixels, j: usize, opacity: f64, multiply: bool) {
    let sa = f64::from(src.sample(j + 3)) / 65535.0 * opacity;
    if sa == 0.0 {
        return;
    }
    let da = f64::from(dst.sample(i + 3)) / 65535.0;
    let alpha = sa + da * (1.0 - sa);
    for c in 0..3 {
        let s = f64::from(src.sample(j + c)) / 65535.0;
        let d = f64::from(dst.sample(i + c)) / 65535.0;
        let blend = if multiply { s * d } else { s };
        let premult = sa * ((1.0 - da) * s + da * blend) + (1.0 - sa) * da * d;
        dst.set_sample(i + c, rounded(premult / alpha));
    }
    dst.set_sample(i + 3, rounded(alpha));
}
fn composite_transformed(
    dst: &mut DecodedImage,
    src: &DecodedImage,
    t: &v1::Affine,
    origin: (u32, u32),
) -> Result<(), RasterError> {
    let determinant = t.a * t.d - t.b * t.c;
    if !determinant.is_finite() || determinant.abs() < 1e-12 {
        return Err(RasterError::Invalid("result transform"));
    }
    for y in 0..dst.height {
        for x in 0..dst.width {
            let dx = f64::from(x) + f64::from(origin.0) + 0.5 - t.e;
            let dy = f64::from(y) + f64::from(origin.1) + 0.5 - t.f;
            let sx = (t.d * dx - t.c * dy) / determinant;
            let sy = (-t.b * dx + t.a * dy) / determinant;
            if sx >= 0.0 && sy >= 0.0 && sx < f64::from(src.width) && sy < f64::from(src.height) {
                let i = ((u64::from(y) * u64::from(dst.width) + u64::from(x)) * 4) as usize;
                let j = ((sy as u64 * u64::from(src.width) + sx as u64) * 4) as usize;
                blend_pixel(&mut dst.pixels, i, &src.pixels, j, 1.0, false);
            }
        }
    }
    Ok(())
}
fn apply_adjustment(
    image: &mut DecodedImage,
    a: &v1::Adjustment,
    opacity: f64,
) -> Result<(), RasterError> {
    if a.levels.len() != 5 {
        return Err(RasterError::Invalid("levels"));
    }
    let l = &a.levels;
    for i in (0..image.pixels.len()).step_by(4) {
        if image.pixels.sample(i + 3) == 0 {
            continue;
        }
        for c in 0..3 {
            let original = f64::from(image.pixels.sample(i + c)) / 65535.0;
            let level = ((original - l[0]) / (l[1] - l[0])).clamp(0.0, 1.0);
            let corrected = l[3] + libm::pow(level, 1.0 / l[2]) * (l[4] - l[3]);
            let brightness = (corrected + a.brightness).clamp(0.0, 1.0);
            let contrast = if a.contrast >= 1.0 {
                if brightness >= 0.5 { 1.0 } else { 0.0 }
            } else {
                ((brightness - 0.5) * (1.0 + a.contrast) / (1.0 - a.contrast) + 0.5).clamp(0.0, 1.0)
            };
            image.pixels.set_sample(
                i + c,
                rounded(original * (1.0 - opacity) + contrast * opacity),
            );
        }
    }
    Ok(())
}
fn coordinate(x: f64) -> Result<f32, RasterError> {
    if !x.is_finite() || x.abs() > 16_000_000.0 {
        return Err(RasterError::Invalid("raster coordinate"));
    }
    Ok(x as f32)
}
fn transform(a: &v1::Affine) -> Result<Transform, RasterError> {
    Ok(Transform::from_row(
        coordinate(a.a)?,
        coordinate(a.b)?,
        coordinate(a.c)?,
        coordinate(a.d)?,
        coordinate(a.e)?,
        coordinate(a.f)?,
    ))
}
fn paint(color: Option<&v1::Color>) -> Paint<'static> {
    let mut paint = Paint::default();
    let rgba = color.map_or(0x000000ff, |v| v.rgba).to_be_bytes();
    paint.set_color_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]);
    paint
}
fn polyline(p: &v1::Polyline, close: bool) -> Result<Path, RasterError> {
    let mut b = PathBuilder::new();
    for (i, p) in p.points.iter().enumerate() {
        let (x, y) = (coordinate(p.x)?, coordinate(p.y)?);
        if i == 0 {
            b.move_to(x, y);
        } else {
            b.line_to(x, y);
        }
    }
    if close {
        b.close();
    }
    b.finish().ok_or(RasterError::Invalid("empty path"))
}
fn rect(r: &v1::RectD) -> Result<tiny_skia::Rect, RasterError> {
    tiny_skia::Rect::from_xywh(
        coordinate(r.x)?,
        coordinate(r.y)?,
        coordinate(r.w)?,
        coordinate(r.h)?,
    )
    .ok_or(RasterError::Invalid("rectangle"))
}
fn draw(pixmap: &mut Pixmap, path: &Path, style: &v1::Style, t: Transform, fill_only: bool) {
    if fill_only {
        pixmap.fill_path(
            path,
            &paint(style.stroke.as_ref()),
            FillRule::Winding,
            t,
            None,
        );
        return;
    }
    if style.has_fill {
        pixmap.fill_path(
            path,
            &paint(style.fill.as_ref()),
            FillRule::Winding,
            t,
            None,
        );
    }
    if style.width > 0.0 {
        let stroke = Stroke {
            width: style.width as f32,
            line_cap: tiny_skia::LineCap::Round,
            line_join: tiny_skia::LineJoin::Round,
            ..Stroke::default()
        };
        // screen_constant is explicitly interpreted at export's 1 D-px/pixel view.
        if style.screen_constant_width {
            if let Some(transformed) = path.clone().transform(t) {
                pixmap.stroke_path(
                    &transformed,
                    &paint(style.stroke.as_ref()),
                    &stroke,
                    Transform::identity(),
                    None,
                );
            }
        } else {
            pixmap.stroke_path(path, &paint(style.stroke.as_ref()), &stroke, t, None);
        }
    }
}
fn render_object(
    pixmap: &mut Pixmap,
    state: &v1::ObjectState,
    origin: (u32, u32),
    cancelled: &dyn Fn() -> bool,
) -> Result<(), RasterError> {
    // Skia's curve clipping and antialias coverage depend on the viewport.
    // Always rasterize against the same document-anchored 256px tile grid,
    // then copy exact samples into the requested region. A crop or encoder
    // strip boundary must never become a new rasterization boundary.
    const TILE: u32 = 256;
    let right = origin
        .0
        .checked_add(pixmap.width())
        .ok_or(RasterError::Allocation)?;
    let bottom = origin
        .1
        .checked_add(pixmap.height())
        .ok_or(RasterError::Allocation)?;
    let mut tile = Pixmap::new(TILE, TILE).ok_or(RasterError::Allocation)?;
    for y in (origin.1 / TILE * TILE..bottom).step_by(TILE as usize) {
        for x in (origin.0 / TILE * TILE..right).step_by(TILE as usize) {
            crate::source::check_cancel(cancelled)?;
            tile.fill(tiny_skia::Color::TRANSPARENT);
            render_object_tile(&mut tile, state, (x, y))?;
            let left = x.max(origin.0);
            let top = y.max(origin.1);
            let end_x = x.saturating_add(TILE).min(right);
            let end_y = y.saturating_add(TILE).min(bottom);
            let width = (end_x - left) as usize * 4;
            for row in top..end_y {
                let from = ((row - y) as usize * TILE as usize + (left - x) as usize) * 4;
                let to = ((row - origin.1) as usize * pixmap.width() as usize
                    + (left - origin.0) as usize)
                    * 4;
                pixmap.data_mut()[to..to + width].copy_from_slice(&tile.data()[from..from + width]);
            }
        }
    }
    Ok(())
}

fn render_object_tile(
    pixmap: &mut Pixmap,
    state: &v1::ObjectState,
    origin: (u32, u32),
) -> Result<(), RasterError> {
    let style = state.style.as_ref().ok_or(RasterError::Invalid("style"))?;
    if style.width > 4096.0 {
        return Err(RasterError::Invalid("stroke width"));
    }
    let t = transform(
        state
            .transform
            .as_ref()
            .ok_or(RasterError::Invalid("transform"))?,
    )?
    .post_translate(-(origin.0 as f32), -(origin.1 as f32));
    match state.shape.as_ref().ok_or(RasterError::Invalid("shape"))? {
        Shape::Stroke(stroke) => {
            let geometry = vw_ink::geometry_from_stroke(stroke)?;
            let mut builder = PathBuilder::new();
            for polygon in geometry.polygons() {
                for (i, p) in polygon.points.iter().enumerate() {
                    let (x, y) = (coordinate(p.x_px())?, coordinate(p.y_px())?);
                    if i == 0 {
                        builder.move_to(x, y);
                    } else {
                        builder.line_to(x, y);
                    }
                }
                builder.close();
            }
            if let Some(path) = builder.finish() {
                draw(pixmap, &path, style, t, true);
            }
        }
        Shape::Line(line) => draw_outline(pixmap, &polyline(line, false)?, style, t),
        Shape::Arrow(line) => {
            draw_outline(pixmap, &polyline(line, false)?, style, t);
            let end = line.points.last().ok_or(RasterError::Invalid("arrow"))?;
            let before = line
                .points
                .iter()
                .rev()
                .skip(1)
                .find(|p| p.x != end.x || p.y != end.y)
                .ok_or(RasterError::Invalid("zero length arrow"))?;
            let (dx, dy) = (end.x - before.x, end.y - before.y);
            let length = libm::sqrt(dx * dx + dy * dy);
            let size = (style.width * 4.0).max(8.0);
            let (ux, uy) = (dx / length, dy / length);
            let mut b = PathBuilder::new();
            b.move_to(coordinate(end.x)?, coordinate(end.y)?);
            b.line_to(
                coordinate(end.x - ux * size - uy * size * 0.45)?,
                coordinate(end.y - uy * size + ux * size * 0.45)?,
            );
            b.line_to(
                coordinate(end.x - ux * size + uy * size * 0.45)?,
                coordinate(end.y - uy * size - ux * size * 0.45)?,
            );
            b.close();
            let path = b.finish().ok_or(RasterError::Invalid("arrow head"))?;
            draw(pixmap, &path, style, t, true);
        }
        Shape::Rect(r) => draw(pixmap, &PathBuilder::from_rect(rect(r)?), style, t, false),
        Shape::Ellipse(r) => {
            let mut b = PathBuilder::new();
            b.push_oval(rect(r)?);
            if let Some(path) = b.finish() {
                draw(pixmap, &path, style, t, false);
            }
        }
        Shape::Polygon(p) => draw(pixmap, &polyline(p, true)?, style, t, false),
        Shape::Crop(r) => draw_outline(pixmap, &PathBuilder::from_rect(rect(r)?), style, t),
        Shape::SelectionVector(p) => draw_outline(pixmap, &polyline(p, true)?, style, t),
        Shape::SelectionRaster(mask) => {
            let r = mask
                .bounds
                .as_ref()
                .ok_or(RasterError::Invalid("mask bounds"))?;
            draw_outline(pixmap, &PathBuilder::from_rect(rect(r)?), style, t);
        }
        Shape::Text(text_object) => {
            let layout = layout_text(
                &text_object.text,
                FontFamily::parse(&text_object.font_family)?,
                coordinate(text_object.font_size)?,
            )?;
            let anchor = text_object
                .anchor
                .as_ref()
                .ok_or(RasterError::Invalid("text anchor"))?;
            let mut b = PathBuilder::new();
            text::append_outlines(&mut b, &layout);
            if let Some(path) = b.finish() {
                draw(
                    pixmap,
                    &path,
                    style,
                    t.pre_translate(coordinate(anchor.x)?, coordinate(anchor.y)?),
                    true,
                );
            }
        }
        Shape::Marker(marker) => {
            let p = marker
                .point
                .as_ref()
                .ok_or(RasterError::Invalid("marker point"))?;
            let radius = (style.width * 4.0).max(12.0);
            let mut b = PathBuilder::new();
            b.push_circle(coordinate(p.x)?, coordinate(p.y)?, coordinate(radius)?);
            let path = b.finish().ok_or(RasterError::Invalid("marker"))?;
            draw(pixmap, &path, style, t, false);
            if let Some(bounds) = &marker.r#box
                && bounds.w > 0.0
                && bounds.h > 0.0
            {
                draw(
                    pixmap,
                    &PathBuilder::from_rect(rect(bounds)?),
                    style,
                    t,
                    false,
                );
            }
            let size = (radius * 1.15) as f32;
            let layout = layout_text(&marker.number.to_string(), FontFamily::Inter, size)?;
            let mut b = PathBuilder::new();
            text::append_outlines(&mut b, &layout);
            if let Some(path) = b.finish() {
                let offset = t.pre_translate(
                    coordinate(p.x)? - layout.width / 2.0,
                    coordinate(p.y)? + size * 0.35,
                );
                draw(pixmap, &path, style, offset, true);
            }
        }
        Shape::ResultId(_) | Shape::Adjustment(_) => {
            return Err(RasterError::Invalid("composite shape routing"));
        }
    }
    Ok(())
}
fn draw_outline(pixmap: &mut Pixmap, path: &Path, style: &v1::Style, t: Transform) {
    let mut outline = *style;
    outline.has_fill = false;
    draw(pixmap, path, &outline, t, false);
}
