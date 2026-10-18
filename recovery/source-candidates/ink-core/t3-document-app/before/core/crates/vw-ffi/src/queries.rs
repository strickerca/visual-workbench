use crate::{
    project::{ProjectState, decode_limits},
    *,
};
use std::{io::Read, sync::Arc};
use vw_model::{AssetId, Id, Project};
use vw_proto::{
    Message,
    v1::{self as pb, object_state::Shape},
};
use vw_raster::{AssetResolver, DecodedImage};

fn rect(value: QueryRect) -> Result<QueryRect> {
    if ![
        value.x,
        value.y,
        value.width,
        value.height,
        value.x + value.width,
        value.y + value.height,
    ]
    .iter()
    .all(|v| v.is_finite())
        || value.width < 0.0
        || value.height < 0.0
    {
        return Err(CoreError::Invalid);
    }
    Ok(value)
}
fn from_bounds(points: impl IntoIterator<Item = (f64, f64)>) -> Result<QueryRect> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (x, y) in points {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    rect(QueryRect {
        x: min_x,
        y: min_y,
        width: max_x - min_x,
        height: max_y - min_y,
    })
}
fn corners(r: QueryRect) -> [(f64, f64); 4] {
    [
        (r.x, r.y),
        (r.x + r.width, r.y),
        (r.x, r.y + r.height),
        (r.x + r.width, r.y + r.height),
    ]
}
fn inflate(r: QueryRect, margin: f64) -> QueryRect {
    QueryRect {
        x: r.x - margin,
        y: r.y - margin,
        width: r.width + 2.0 * margin,
        height: r.height + 2.0 * margin,
    }
}
fn overlaps(a: QueryRect, b: QueryRect) -> bool {
    a.x <= b.x + b.width && a.x + a.width >= b.x && a.y <= b.y + b.height && a.y + a.height >= b.y
}
fn pb_rect(r: &pb::RectD) -> QueryRect {
    QueryRect {
        x: r.x,
        y: r.y,
        width: r.w,
        height: r.h,
    }
}

/// Returns conservative transformed AABBs, never a claim of exact shape hits.
/// Contours are local document coordinates; apply the protobuf affine exactly once.
pub(crate) fn render_list(
    state: &ProjectState,
    document: &str,
    query: Option<QueryRect>,
) -> Result<RenderList> {
    let query = query.map(rect).transpose()?;
    let project = state.project()?;
    let document = Id::try_from(document.to_owned())?;
    let definition = &project
        .documents
        .get(&document)
        .ok_or(CoreError::Invalid)?
        .definition;
    let source = project
        .assets
        .get(&AssetId::try_from(definition.primary_asset_id.clone())?)
        .ok_or(CoreError::Invalid)?;
    let (w, h) = if source.orientation >= 5 {
        (source.height, source.width)
    } else {
        (source.width, source.height)
    };
    let page = QueryRect {
        x: 0.0,
        y: 0.0,
        width: f64::from(w),
        height: f64::from(h),
    };
    let mut layers = project
        .layers
        .iter()
        .filter(|(_, layer)| {
            layer.definition.document_id.as_ref() == Some(&document.to_proto())
                && layer.visible
                && layer.opacity > 0.0
        })
        .collect::<Vec<_>>();
    layers.sort_by(|(aid, a), (bid, b)| {
        a.definition
            .order_key
            .cmp(&b.definition.order_key)
            .then(aid.cmp(bid))
    });
    let revision = state.info()?;
    let mut items = Vec::new();
    let mut bytes = crate::payload::info_bytes(&revision)?;
    for (layer_id, layer) in layers {
        let mut objects = project
            .objects
            .iter()
            .filter(|(_, object)| {
                object.document_id == document
                    && object.state.layer_id.as_ref() == Some(&layer_id.to_proto())
                    && !object.state.hidden
            })
            .collect::<Vec<_>>();
        objects.sort_by(|(aid, a), (bid, b)| {
            a.state.order_key.cmp(&b.state.order_key).then(aid.cmp(bid))
        });
        for (id, object) in objects {
            let shape = draw_shape(&object.state, project)?;
            let (local, contours) = local_bounds(&object.state, project, page, &shape)?;
            let a = object.state.transform.as_ref().ok_or(CoreError::Invalid)?;
            let mut bounds = from_bounds(
                corners(local).map(|(x, y)| (a.a * x + a.c * y + a.e, a.b * x + a.d * y + a.f)),
            )?;
            let style = object.state.style.as_ref().ok_or(CoreError::Invalid)?;
            if style.screen_constant_width {
                bounds = rect(inflate(bounds, style.width * 2.0))?;
            }
            if query.is_some_and(|q| !overlaps(q, bounds)) {
                continue;
            }
            let protobuf = object.state.encode_to_vec();
            if items.len() >= 10_000 {
                return Err(CoreError::Backpressure);
            }
            let item = RenderItem {
                object_id: id.to_string(),
                layer_id: layer_id.to_string(),
                layer_opacity: layer.opacity,
                layer_blend: layer.blend.clone(),
                bounds,
                object_protobuf: protobuf,
                stroke_contours: contours,
                transform: Transform {
                    a: a.a,
                    b: a.b,
                    c: a.c,
                    d: a.d,
                    e: a.e,
                    f: a.f,
                },
                style: ObjectStyle {
                    rgba: style.stroke.as_ref().map_or(0, |c| c.rgba),
                    width: style.width,
                    screen_constant_width: style.screen_constant_width,
                    fill: style
                        .has_fill
                        .then(|| style.fill.as_ref().map_or(0, |c| c.rgba)),
                },
                shape,
                locked: object.state.locked || layer.locked,
            };
            crate::payload::accumulate(&mut bytes, crate::payload::item_bytes(&item)?)?;
            items.push(item);
        }
    }
    Ok(RenderList { revision, items })
}
fn local_bounds(
    object: &pb::ObjectState,
    project: &Project,
    page: QueryRect,
    draw: &DrawShape,
) -> Result<(QueryRect, Contours)> {
    let style = object.style.as_ref().ok_or(CoreError::Invalid)?;
    let mut contours = Contours::default();
    let mut margin = style.width * 2.0;
    let bounds = match object.shape.as_ref().ok_or(CoreError::Invalid)? {
        Shape::Stroke(stroke) => {
            let geometry = vw_ink::geometry_from_stroke(stroke)?;
            let b = geometry.bounds();
            contours = crate::gesture::contours(geometry.polygons())?;
            margin = 0.0;
            b.map(|b| QueryRect {
                x: b.min_x,
                y: b.min_y,
                width: b.max_x - b.min_x,
                height: b.max_y - b.min_y,
            })
            .unwrap_or(QueryRect {
                x: stroke.x[0],
                y: stroke.y[0],
                width: 0.0,
                height: 0.0,
            })
        }
        Shape::Rect(r) | Shape::Ellipse(r) | Shape::Crop(r) => pb_rect(r),
        Shape::Line(p) | Shape::Polygon(p) | Shape::SelectionVector(p) => {
            from_bounds(p.points.iter().map(|p| (p.x, p.y)))?
        }
        Shape::Arrow(p) => {
            margin = (style.width * 6.0).max(12.0);
            from_bounds(p.points.iter().map(|p| (p.x, p.y)))?
        }
        Shape::SelectionRaster(r) => pb_rect(r.bounds.as_ref().ok_or(CoreError::Invalid)?),
        Shape::Text(_) => {
            let DrawShape::Text {
                anchor, outline, ..
            } = draw
            else {
                return Err(CoreError::Invalid);
            };
            from_bounds(
                std::iter::once((anchor.x, anchor.y)).chain(
                    outline
                        .iter()
                        .flat_map(outline_points)
                        .map(|(x, y)| (anchor.x + f64::from(x), anchor.y + f64::from(y))),
                ),
            )?
        }
        Shape::Marker(m) => {
            let p = m.point.as_ref().ok_or(CoreError::Invalid)?;
            let radius = (style.width * 4.0).max(12.0);
            let text = vw_raster::layout_text(
                &m.number.to_string(),
                vw_raster::FontFamily::Inter,
                (radius * 1.15) as f32,
            )?;
            let r = radius.max(f64::from(text.width));
            let mut points = corners(QueryRect {
                x: p.x - r,
                y: p.y - r,
                width: 2.0 * r,
                height: 2.0 * r,
            })
            .to_vec();
            if let Some(b) = &m.r#box {
                points.extend(corners(pb_rect(b)));
            }
            from_bounds(points)?
        }
        Shape::ResultId(id) => {
            let asset = project
                .assets
                .get(&AssetId::try_from(result_asset(project, id)?)?)
                .ok_or(CoreError::Invalid)?;
            margin = 0.0;
            let (w, h) = if asset.orientation >= 5 {
                (asset.height, asset.width)
            } else {
                (asset.width, asset.height)
            };
            QueryRect {
                x: 0.0,
                y: 0.0,
                width: f64::from(w),
                height: f64::from(h),
            }
        }
        Shape::Adjustment(_) => {
            margin = 0.0;
            page
        }
    };
    Ok((rect(inflate(bounds, margin))?, contours))
}
pub(crate) fn preview_item(
    state: &ProjectState,
    object: &pb::ObjectState,
    document: &Id,
) -> Result<RenderItem> {
    let project = state.project()?;
    let doc = &project
        .documents
        .get(document)
        .ok_or(CoreError::Invalid)?
        .definition;
    let asset = project
        .assets
        .get(&AssetId::try_from(doc.primary_asset_id.clone())?)
        .ok_or(CoreError::Invalid)?;
    let (w, h) = if asset.orientation >= 5 {
        (asset.height, asset.width)
    } else {
        (asset.width, asset.height)
    };
    let shape = draw_shape(object, project)?;
    let (local, stroke_contours) = local_bounds(
        object,
        project,
        QueryRect {
            x: 0.0,
            y: 0.0,
            width: f64::from(w),
            height: f64::from(h),
        },
        &shape,
    )?;
    let a = object.transform.as_ref().ok_or(CoreError::Invalid)?;
    let style = object.style.as_ref().ok_or(CoreError::Invalid)?;
    let layer_id = Id::from_proto(object.layer_id.as_ref())?;
    let layer = project.layers.get(&layer_id);
    let bounds = from_bounds(
        corners(local).map(|(x, y)| (a.a * x + a.c * y + a.e, a.b * x + a.d * y + a.f)),
    )?;
    Ok(RenderItem {
        object_id: Id::from_proto(object.object_id.as_ref())?.to_string(),
        layer_id: layer_id.to_string(),
        layer_opacity: layer.map_or(1.0, |l| l.opacity),
        layer_blend: layer.map_or_else(
            || {
                if matches!(&shape,DrawShape::Stroke{family}if family=="highlighter") {
                    "multiply".into()
                } else {
                    "normal".into()
                }
            },
            |l| l.blend.clone(),
        ),
        bounds,
        object_protobuf: vec![],
        stroke_contours,
        transform: Transform {
            a: a.a,
            b: a.b,
            c: a.c,
            d: a.d,
            e: a.e,
            f: a.f,
        },
        style: ObjectStyle {
            rgba: style.stroke.as_ref().map_or(0, |c| c.rgba),
            width: style.width,
            screen_constant_width: style.screen_constant_width,
            fill: style
                .fill
                .as_ref()
                .filter(|_| style.has_fill)
                .map(|c| c.rgba),
        },
        shape,
        locked: false,
    })
}
fn outline_points(command: &Outline) -> impl Iterator<Item = (f32, f32)> {
    use Outline::*;
    match *command {
        Move { x, y } | Line { x, y } => [Some((x, y)), None, None],
        Quad { x1, y1, x, y } => [Some((x1, y1)), Some((x, y)), None],
        Cubic {
            x1,
            y1,
            x2,
            y2,
            x,
            y,
        } => [Some((x1, y1)), Some((x2, y2)), Some((x, y))],
        Close => [None, None, None],
    }
    .into_iter()
    .flatten()
}
fn outline(command: vw_raster::OutlineCommand) -> Outline {
    use vw_raster::OutlineCommand::*;
    match command {
        Move(x, y) => Outline::Move { x, y },
        Line(x, y) => Outline::Line { x, y },
        Quad(x1, y1, x, y) => Outline::Quad { x1, y1, x, y },
        Cubic(x1, y1, x2, y2, x, y) => Outline::Cubic {
            x1,
            y1,
            x2,
            y2,
            x,
            y,
        },
        Close => Outline::Close,
    }
}
fn result_asset(project: &Project, id: &pb::Uuid) -> Result<String> {
    let result = &project
        .results
        .get(&Id::from_proto(Some(id))?)
        .ok_or(CoreError::Invalid)?
        .definition;
    Ok(if result.composite_asset_id.is_empty() {
        result.output_asset_id.clone()
    } else {
        result.composite_asset_id.clone()
    })
}
fn draw_shape(object: &pb::ObjectState, project: &Project) -> Result<DrawShape> {
    let points = |p: &pb::Polyline| p.points.iter().map(|p| Point { x: p.x, y: p.y }).collect();
    Ok(match object.shape.as_ref().ok_or(CoreError::Invalid)? {
        Shape::Stroke(s) => DrawShape::Stroke {
            family: s.brush.as_ref().ok_or(CoreError::Invalid)?.family.clone(),
        },
        Shape::Line(p) => DrawShape::Line { points: points(p) },
        Shape::Arrow(p) => DrawShape::Arrow { points: points(p) },
        Shape::Rect(r) => DrawShape::Rectangle {
            rectangle: pb_rect(r),
        },
        Shape::Ellipse(r) => DrawShape::Ellipse {
            rectangle: pb_rect(r),
        },
        Shape::Polygon(p) | Shape::SelectionVector(p) => DrawShape::Polygon {
            points: points(p),
            closed: p.closed,
        },
        Shape::Text(t) => {
            let p = t.anchor.as_ref().ok_or(CoreError::Invalid)?;
            let layout = vw_raster::layout_text(
                &t.text,
                vw_raster::FontFamily::parse(&t.font_family)?,
                t.font_size as f32,
            )?;
            DrawShape::Text {
                anchor: Point { x: p.x, y: p.y },
                text: t.text.clone(),
                font: t.font_family.clone(),
                size: t.font_size,
                outline: layout
                    .glyphs
                    .into_iter()
                    .flat_map(|g| g.outline.into_iter().map(outline))
                    .collect(),
            }
        }
        Shape::Marker(m) => {
            let p = m.point.as_ref().ok_or(CoreError::Invalid)?;
            DrawShape::Marker {
                number: m.number,
                point: Point { x: p.x, y: p.y },
                rectangle: m.r#box.as_ref().map(pb_rect),
            }
        }
        Shape::SelectionRaster(r) => DrawShape::Guide {
            rectangle: pb_rect(r.bounds.as_ref().ok_or(CoreError::Invalid)?),
        },
        Shape::Crop(r) => DrawShape::Guide {
            rectangle: pb_rect(r),
        },
        Shape::ResultId(id) => DrawShape::Result {
            asset_id: result_asset(project, id)?,
            result_id: Id::from_proto(Some(id))?.to_string(),
        },
        Shape::Adjustment(_) => DrawShape::Adjustment,
    })
}

#[uniffi::export]
impl ProjectSession {
    pub async fn document_snapshot(
        &self,
        document_id: String,
        cancellation: Arc<Cancellation>,
    ) -> Result<DocumentSnapshot> {
        self.check_open()?;
        self.worker
            .call(move |state| {
                cancellation.check()?;
                let project = state.project()?;
                let id = Id::try_from(document_id.clone())?;
                let doc = &project
                    .documents
                    .get(&id)
                    .ok_or(CoreError::Invalid)?
                    .definition;
                let asset = project
                    .assets
                    .get(&AssetId::try_from(doc.primary_asset_id.clone())?)
                    .ok_or(CoreError::Invalid)?;
                let (width, height) = if asset.orientation >= 5 {
                    (asset.height, asset.width)
                } else {
                    (asset.width, asset.height)
                };
                let mut layers = project
                    .layers
                    .iter()
                    .filter(|(_, l)| l.definition.document_id.as_ref() == Some(&id.to_proto()))
                    .collect::<Vec<_>>();
                layers.sort_by(|(a, l), (b, r)| {
                    l.definition
                        .order_key
                        .cmp(&r.definition.order_key)
                        .then(a.cmp(b))
                });
                let layers = layers
                    .into_iter()
                    .map(|(id, l)| LayerInfo {
                        id: id.to_string(),
                        name: l.definition.name.clone(),
                        visible: l.visible,
                        locked: l.locked,
                        opacity: l.opacity,
                        blend: l.blend.clone(),
                    })
                    .collect();
                let result = DocumentSnapshot {
                    document_id,
                    title: doc.title.clone(),
                    width,
                    height,
                    bit_depth: asset.bit_depth,
                    layers,
                    render: render_list(state, id.as_str(), None)?,
                };
                crate::payload::document_bytes(&result)?;
                cancellation.check()?;
                Ok(result)
            })
            .await
    }
    /// Full-resolution oriented display pixels, requested independently of edit
    /// snapshots. Original assets and bit depth are retained on disk. This display
    /// derivative is explicitly converted to sRGB/8-bit; no silent downsampling.
    pub async fn background_image(
        &self,
        document_id: String,
        memory_budget_bytes: u64,
        assume_untagged_srgb: bool,
        cancellation: Arc<Cancellation>,
    ) -> Result<BackgroundImage> {
        self.check_open()?;
        if memory_budget_bytes == 0 || memory_budget_bytes > 256 * 1024 * 1024 {
            return Err(CoreError::Invalid);
        }
        self.worker
            .call(move |state| {
                cancellation.check()?;
                let doc = &state
                    .project()?
                    .documents
                    .get(&Id::try_from(document_id.clone())?)
                    .ok_or(CoreError::Invalid)?
                    .definition;
                let asset = state
                    .project()?
                    .assets
                    .get(&AssetId::try_from(doc.primary_asset_id.clone())?)
                    .ok_or(CoreError::Invalid)?;
                let estimate = u64::from(asset.width)
                    .checked_mul(u64::from(asset.height))
                    .and_then(|n| n.checked_mul(40))
                    .and_then(|n| n.checked_add(asset.byte_size))
                    .and_then(|n| n.checked_add(32 * 1024 * 1024))
                    .ok_or(CoreError::Invalid)?;
                if estimate > memory_budget_bytes {
                    return Err(CoreError::Backpressure);
                }
                let depth = asset.bit_depth as u8;
                let source_asset_id = doc.primary_asset_id.clone();
                let output = export(
                    state,
                    ExportOptions {
                        document_id,
                        format: ImageFormat::Png8,
                        marked: false,
                        region: None,
                        matte_rgb: None,
                        convert_to_srgb: true,
                        assume_untagged_srgb,
                        allow_depth_reduction: true,
                        memory_budget_bytes,
                    },
                    &cancellation,
                )?;
                let decode_budget = memory_budget_bytes
                    .checked_sub(output.bytes.len() as u64)
                    .ok_or(CoreError::Backpressure)?;
                let image = vw_raster::decode(
                    &output.bytes,
                    vw_raster::DecodeLimits {
                        max_encoded_bytes: 64 * 1024 * 1024,
                        max_pixels: 50_000_000,
                        max_memory_bytes: decode_budget,
                    },
                )?;
                cancellation.check()?;
                let rgba = match image.pixels {
                    vw_raster::Pixels::Rgba8(bytes) => bytes,
                    _ => return Err(CoreError::Raster),
                };
                Ok(BackgroundImage {
                    width: image.width,
                    height: image.height,
                    rgba,
                    source_asset_id,
                    source_bit_depth: depth,
                    icc_profile: image.icc.unwrap_or_default(),
                })
            })
            .await
    }
}

struct Assets<'a> {
    blobs: &'a vw_store::BlobStore,
    budget: u64,
    cancel: &'a Cancellation,
}
impl AssetResolver for Assets<'_> {
    fn image(&self, asset: &str) -> std::result::Result<DecodedImage, vw_raster::RasterError> {
        let id = AssetId::try_from(asset.to_owned())
            .map_err(|_| vw_raster::RasterError::MissingAsset)?;
        let path = self
            .blobs
            .path(&id)
            .map_err(|_| vw_raster::RasterError::MissingAsset)?;
        let mut file =
            std::fs::File::open(path).map_err(|_| vw_raster::RasterError::MissingAsset)?;
        let size = file
            .metadata()
            .map_err(|_| vw_raster::RasterError::MissingAsset)?
            .len();
        let limit = decode_limits()
            .max_encoded_bytes
            .min(usize::try_from(self.budget).unwrap_or(usize::MAX));
        if size > limit as u64 {
            return Err(vw_raster::RasterError::Memory {
                estimated: size,
                budget: self.budget,
            });
        }
        let size = usize::try_from(size).map_err(|_| vw_raster::RasterError::Allocation)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| vw_raster::RasterError::Allocation)?;
        bytes.resize(size, 0);
        file.read_exact(&mut bytes)
            .map_err(|_| vw_raster::RasterError::MissingAsset)?;
        if file
            .read(&mut [0u8; 1])
            .map_err(|_| vw_raster::RasterError::MissingAsset)?
            != 0
            || AssetId::hash(&bytes) != id
        {
            return Err(vw_raster::RasterError::MissingAsset);
        }
        let budget = self
            .budget
            .checked_sub(bytes.len() as u64)
            .ok_or(vw_raster::RasterError::Allocation)?;
        crate::os_images::decode(
            &bytes,
            vw_raster::DecodeLimits {
                max_encoded_bytes: limit,
                max_pixels: 50_000_000,
                max_memory_bytes: budget,
            },
            &|| self.cancel.is_cancelled(),
        )
    }
}
pub(crate) fn export(
    state: &ProjectState,
    options: ExportOptions,
    cancellation: &Cancellation,
) -> Result<ExportResult> {
    let document = Id::try_from(options.document_id)?;
    let definition = &state
        .project()?
        .documents
        .get(&document)
        .ok_or(CoreError::Invalid)?
        .definition;
    if options.memory_budget_bytes > 512 * 1024 * 1024 || options.memory_budget_bytes == 0 {
        return Err(CoreError::Invalid);
    }
    let assets = Assets {
        blobs: &state.blobs,
        budget: options.memory_budget_bytes,
        cancel: cancellation,
    };
    let source = assets.image(&definition.primary_asset_id)?;
    cancellation.check()?;
    let pixels = if options.marked {
        let render_working = u64::from(source.width)
            .checked_mul(u64::from(source.height))
            .and_then(|n| n.checked_mul(64))
            .and_then(|n| n.checked_add(32 * 1024 * 1024))
            .ok_or(CoreError::Invalid)?;
        let remaining = options
            .memory_budget_bytes
            .checked_sub(render_working)
            .ok_or(CoreError::Backpressure)?;
        let result_assets = Assets {
            blobs: &state.blobs,
            budget: remaining,
            cancel: cancellation,
        };
        let pixels = vw_raster::render_document(
            &source,
            state.project()?,
            &document,
            &result_assets,
            vw_raster::RenderOptions {
                memory_budget_bytes: options.memory_budget_bytes,
                include_guides: false,
                assume_untagged_srgb: options.assume_untagged_srgb,
            },
        )?;
        drop(source);
        pixels
    } else {
        source
    };
    cancellation.check()?;
    let region = options
        .region
        .map(|r| {
            rect(r)?;
            if [r.x, r.y, r.width, r.height]
                .iter()
                .any(|v| *v < 0.0 || *v > f64::from(u32::MAX) || v.fract() != 0.0)
            {
                return Err(CoreError::Invalid);
            }
            Ok(vw_raster::Region {
                x: r.x as u32,
                y: r.y as u32,
                width: r.width as u32,
                height: r.height as u32,
            })
        })
        .transpose()?;
    // Export may crop a small region while retaining the full-resolution source.
    // Reserve it separately from the codec's region-based working estimate.
    let retained = (pixels.pixels.len() as u64)
        .checked_mul(u64::from(pixels.pixels.bit_depth() / 8))
        .and_then(|n| n.checked_add(pixels.icc.as_ref().map_or(0, |p| p.len() as u64)))
        .ok_or(CoreError::Backpressure)?;
    let export_budget = options
        .memory_budget_bytes
        .checked_sub(retained)
        .ok_or(CoreError::Backpressure)?;
    let request = vw_raster::ExportRequest {
        format: match options.format {
            ImageFormat::Png8 => vw_raster::ExportFormat::Png8,
            ImageFormat::Png16 => vw_raster::ExportFormat::Png16,
            ImageFormat::Jpeg { quality } => vw_raster::ExportFormat::Jpeg { quality },
            ImageFormat::WebpLossless => vw_raster::ExportFormat::WebpLossless,
            ImageFormat::WebpLossy { quality } => vw_raster::ExportFormat::WebpLossy { quality },
        },
        region,
        revision: state.view_revision()?,
        alpha: options
            .matte_rgb
            .map_or(vw_raster::AlphaPolicy::Preserve, |rgb| {
                vw_raster::AlphaPolicy::Matte([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8])
            }),
        color: if options.convert_to_srgb {
            vw_raster::ColorPolicy::ConvertToSrgb {
                assume_untagged_srgb: options.assume_untagged_srgb,
            }
        } else {
            vw_raster::ColorPolicy::Preserve
        },
        allow_depth_reduction: options.allow_depth_reduction,
        memory_budget_bytes: export_budget,
        capture_session: definition
            .capture
            .as_ref()
            .map(|c| Id::from_proto(c.capture_session_id.as_ref()))
            .transpose()?,
        frame_id: definition.capture.as_ref().map(|c| c.frame_id),
    };
    let result = vw_raster::export(&pixels, &request)?;
    cancellation.check()?;
    if result.bytes.len() > 64 * 1024 * 1024 {
        return Err(CoreError::Backpressure);
    }
    Ok(ExportResult {
        blake3: AssetId::hash(&result.bytes).to_string(),
        bytes: result.bytes,
        metadata_json: serde_json::to_string(&result.metadata).map_err(|_| CoreError::Invalid)?,
        revision: state.info()?,
    })
}
#[uniffi::export]
pub async fn layout_text(
    text: String,
    font: String,
    size: f32,
    cancellation: Arc<Cancellation>,
) -> Result<TextLayout> {
    if text.len() > 65536 {
        return Err(CoreError::Invalid);
    }
    crate::worker::startup(move || {
        cancellation.check()?;
        let layout = vw_raster::layout_text(&text, vw_raster::FontFamily::parse(&font)?, size)?;
        crate::payload::layout_bytes(&layout)?;
        cancellation.check()?;
        Ok(TextLayout {
            algorithm_version: layout.algorithm_version,
            width: layout.width,
            height: layout.height,
            line_height: layout.line_height,
            glyphs: layout
                .glyphs
                .into_iter()
                .map(|g| GlyphOutline {
                    glyph_id: g.glyph_id,
                    cluster: g.cluster as u64,
                    font: match g.font {
                        vw_raster::FontFamily::Inter => "Inter",
                        vw_raster::FontFamily::NotoSans => "Noto Sans",
                        vw_raster::FontFamily::NotoSansMono => "Noto Sans Mono",
                    }
                    .into(),
                    x: g.x,
                    y: g.y,
                    advance: g.advance,
                    outline: g
                        .outline
                        .into_iter()
                        .map(|p| {
                            use vw_raster::OutlineCommand::*;
                            match p {
                                Move(x, y) => Outline::Move { x, y },
                                Line(x, y) => Outline::Line { x, y },
                                Quad(x1, y1, x, y) => Outline::Quad { x1, y1, x, y },
                                Cubic(x1, y1, x2, y2, x, y) => Outline::Cubic {
                                    x1,
                                    y1,
                                    x2,
                                    y2,
                                    x,
                                    y,
                                },
                                Close => Outline::Close,
                            }
                        })
                        .collect(),
                })
                .collect(),
        })
    })
    .await
}
