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
    render_list_for_page(state, document, None, query)
}
pub(crate) fn render_list_page(
    state: &ProjectState,
    document: &str,
    page: u32,
    query: Option<QueryRect>,
) -> Result<RenderList> {
    render_list_for_page(state, document, Some(page), query)
}
fn render_list_for_page(
    state: &ProjectState,
    document: &str,
    selected_page: Option<u32>,
    query: Option<QueryRect>,
) -> Result<RenderList> {
    let (revision, entries) = render_entries(
        state,
        document,
        selected_page,
        query,
        None,
        OutlineForm::Lists,
    )?;
    let mut items = Vec::new();
    items
        .try_reserve_exact(entries.len())
        .map_err(|_| CoreError::Worker)?;
    for entry in entries {
        match entry {
            RenderEntry::Item {
                item: DisplayItem::Lists(item),
                ..
            } => items.push(item),
            // Without a set of known strokes every entry is a complete item,
            // in the form asked for.
            _ => return Err(CoreError::Worker),
        }
    }
    Ok(RenderList { revision, items })
}
/// The complete display list `render_list` or `render_list_page` returns,
/// with every stroke outline packed.
pub(crate) fn packed_render_list(
    state: &ProjectState,
    document: &str,
    selected_page: Option<u32>,
) -> Result<PackedRenderList> {
    let (revision, entries) = render_entries(
        state,
        document,
        selected_page,
        None,
        None,
        OutlineForm::Packed,
    )?;
    let mut items = Vec::new();
    items
        .try_reserve_exact(entries.len())
        .map_err(|_| CoreError::Worker)?;
    for entry in entries {
        match entry {
            RenderEntry::Item {
                item: DisplayItem::Packed(item),
                ..
            } => items.push(item),
            _ => return Err(CoreError::Worker),
        }
    }
    Ok(PackedRenderList { revision, items })
}
/// How a display item carries its stroke outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutlineForm {
    /// `RenderItem`: `Contours` and the stored object bytes.
    Lists,
    /// `PackedRenderItem`.
    Packed,
}
// Either form is a complete item; boxing would add one allocation per object.
#[allow(clippy::large_enum_variant)]
pub(crate) enum DisplayItem {
    Lists(RenderItem),
    Packed(PackedRenderItem),
}
impl DisplayItem {
    /// What the item counts toward the response size limit.
    fn cost(&self) -> Result<usize> {
        match self {
            Self::Lists(item) => crate::payload::item_bytes(item),
            Self::Packed(item) => crate::payload::packed_item_bytes(item),
        }
    }
    /// The object id and the layer fields a changes entry repeats.
    fn header(&self) -> (String, f64, String, bool) {
        match self {
            Self::Lists(item) => (
                item.object_id.clone(),
                item.layer_opacity,
                item.layer_blend.clone(),
                item.locked,
            ),
            Self::Packed(item) => (
                item.object_id.clone(),
                item.layer_opacity,
                item.layer_blend.clone(),
                item.locked,
            ),
        }
    }
}
/// The strokes a caller says it holds, checked before the worker is asked.
fn held_strokes(
    known: Vec<KnownStroke>,
) -> Result<std::collections::BTreeMap<String, (StrokeDigest, usize)>> {
    if known.len() > 10_000 {
        return Err(CoreError::Invalid);
    }
    let mut held = std::collections::BTreeMap::new();
    for stroke in known {
        Id::try_from(stroke.object_id.clone())?;
        let digest: StrokeDigest = stroke
            .digest
            .as_slice()
            .try_into()
            .map_err(|_| CoreError::Invalid)?;
        // No item counts less than its fixed part or more than a whole response.
        let cost = usize::try_from(stroke.cost).map_err(|_| CoreError::Invalid)?;
        if !(crate::payload::MIN_ITEM..=crate::payload::MAX_RESPONSE).contains(&cost)
            || held.insert(stroke.object_id, (digest, cost)).is_some()
        {
            return Err(CoreError::Invalid);
        }
    }
    Ok(held)
}
/// One entry of a changes answer, before it takes its record form.
struct ChangeEntry<I> {
    object_id: String,
    digest: Option<Vec<u8>>,
    cost: u64,
    item: Option<I>,
    layer_opacity: f64,
    layer_blend: String,
    locked: bool,
}
/// The entries of a changes answer in drawing order; `take` returns a sent
/// item in the form the answer carries, or None when it is in another form.
fn change_entries<I>(
    entries: Vec<RenderEntry>,
    take: impl Fn(DisplayItem) -> Option<I>,
) -> Result<Vec<ChangeEntry<I>>> {
    let mut items = Vec::new();
    items
        .try_reserve_exact(entries.len())
        .map_err(|_| CoreError::Worker)?;
    for entry in entries {
        items.push(match entry {
            RenderEntry::Item { item, digest, cost } => {
                let (object_id, layer_opacity, layer_blend, locked) = item.header();
                ChangeEntry {
                    object_id,
                    digest: digest.map(|value| value.to_vec()),
                    cost: cost as u64,
                    item: Some(take(item).ok_or(CoreError::Worker)?),
                    layer_opacity,
                    layer_blend,
                    locked,
                }
            }
            RenderEntry::Kept {
                object_id,
                digest,
                cost,
                layer_opacity,
                layer_blend,
                locked,
            } => ChangeEntry {
                object_id,
                digest: Some(digest.to_vec()),
                cost: cost as u64,
                item: None,
                layer_opacity,
                layer_blend,
                locked,
            },
        });
    }
    Ok(items)
}
/// Digest of everything a stroke's display item is built from except its
/// layer's opacity, blend and lock, which travel with every entry.
pub(crate) type StrokeDigest = [u8; 32];
// Complete items dominate; boxing them would add one allocation per object.
#[allow(clippy::large_enum_variant)]
pub(crate) enum RenderEntry {
    Item {
        item: DisplayItem,
        /// Strokes only, and only when the caller asked for changes.
        digest: Option<StrokeDigest>,
        /// What the item counts toward the response size limit.
        cost: usize,
    },
    /// A stroke the caller still holds unchanged: its geometry is not rebuilt.
    Kept {
        object_id: String,
        digest: StrokeDigest,
        /// What the stroke counted when it was sent; it counts the same now.
        cost: usize,
        layer_opacity: f64,
        layer_blend: String,
        locked: bool,
    },
}
/// The display list in drawing order, each item in `form`. With `known`, a
/// stroke whose stored state still has the digest the caller holds is reported
/// as kept instead of being rebuilt; without it this is the complete list.
fn render_entries(
    state: &ProjectState,
    document: &str,
    selected_page: Option<u32>,
    query: Option<QueryRect>,
    known: Option<&std::collections::BTreeMap<String, (StrokeDigest, usize)>>,
    form: OutlineForm,
) -> Result<(ProjectInfo, Vec<RenderEntry>)> {
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
    let (mut w, mut h) = if source.orientation >= 5 {
        (source.height, source.width)
    } else {
        (source.width, source.height)
    };
    let page_index = if let Some(page) = selected_page {
        let descriptor = crate::documents::descriptor(state, &document.to_string())?
            .ok_or(CoreError::Unsupported)?;
        let geometry = descriptor
            .pages
            .get(page as usize)
            .filter(|item| item.page == page)
            .ok_or(CoreError::Invalid)?;
        w = geometry.width.ceil() as u32;
        h = geometry.height.ceil() as u32;
        Some(if descriptor.kind == crate::EditableDocumentKind::Pdf {
            page as i32
        } else {
            -1
        })
    } else {
        None
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
                && page_index.is_none_or(|index| layer.definition.page_index == index)
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
    let mut bytes_total = crate::payload::info_bytes(&revision)?;
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
            if crate::tone_controls::is_editing_metadata(project, &document, id)
                || crate::image_frames::is_editing_metadata(project, &document, id)
            {
                continue;
            }
            // Asked for changes only: a stroke is identified by its stored
            // bytes, which is far cheaper than rebuilding its outline.
            let mut encoded = None;
            let mut digest = None;
            if let Some(known) = known
                && matches!(object.state.shape, Some(Shape::Stroke(_)))
            {
                let bytes = object.state.encode_to_vec();
                let value: StrokeDigest = *blake3::hash(&bytes).as_bytes();
                let object_id = id.to_string();
                if let Some((held, cost)) = known.get(&object_id)
                    && held == &value
                {
                    if items.len() >= 10_000 {
                        return Err(CoreError::Backpressure);
                    }
                    // A kept stroke is not sent, but the caller still holds it:
                    // it counts toward the limit what it counted when it was
                    // sent, so a document cannot grow past what a complete
                    // read would be allowed to return.
                    crate::payload::accumulate(&mut bytes_total, *cost)?;
                    items.push(RenderEntry::Kept {
                        object_id,
                        digest: value,
                        cost: *cost,
                        layer_opacity: layer.opacity,
                        layer_blend: layer.blend.clone(),
                        locked: object.state.locked || layer.locked,
                    });
                    continue;
                }
                encoded = Some(bytes);
                digest = Some(value);
            }
            let shape = draw_shape(&object.state, project)?;
            let (local, geometry) = local_bounds(&object.state, project, page, &shape)?;
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
            if items.len() >= 10_000 {
                return Err(CoreError::Backpressure);
            }
            let outline = geometry.as_ref().map_or(&[][..], |g| g.polygons());
            let transform = Transform {
                a: a.a,
                b: a.b,
                c: a.c,
                d: a.d,
                e: a.e,
                f: a.f,
            };
            let style = ObjectStyle {
                rgba: style.stroke.as_ref().map_or(0, |c| c.rgba),
                width: style.width,
                screen_constant_width: style.screen_constant_width,
                fill: style
                    .has_fill
                    .then(|| style.fill.as_ref().map_or(0, |c| c.rgba)),
            };
            let locked = object.state.locked || layer.locked;
            let item = match form {
                OutlineForm::Lists => DisplayItem::Lists(RenderItem {
                    object_id: id.to_string(),
                    layer_id: layer_id.to_string(),
                    layer_opacity: layer.opacity,
                    layer_blend: layer.blend.clone(),
                    bounds,
                    object_protobuf: match encoded {
                        Some(bytes) => bytes,
                        None => object.state.encode_to_vec(),
                    },
                    stroke_contours: crate::gesture::contours(outline)?,
                    transform,
                    style,
                    shape,
                    locked,
                }),
                OutlineForm::Packed => DisplayItem::Packed(PackedRenderItem {
                    object_id: id.to_string(),
                    layer_id: layer_id.to_string(),
                    layer_opacity: layer.opacity,
                    layer_blend: layer.blend.clone(),
                    bounds,
                    contours: crate::packed_contours::pack(outline)?,
                    transform,
                    style,
                    shape,
                    locked,
                }),
            };
            // The working geometry is not kept once its outline is built.
            drop(geometry);
            let cost = item.cost()?;
            crate::payload::accumulate(&mut bytes_total, cost)?;
            items.push(RenderEntry::Item { item, digest, cost });
        }
    }
    Ok((revision, items))
}
/// The local bounds, and for a stroke the geometry its outline is built from.
fn local_bounds(
    object: &pb::ObjectState,
    project: &Project,
    page: QueryRect,
    draw: &DrawShape,
) -> Result<(QueryRect, Option<vw_ink::Geometry>)> {
    let style = object.style.as_ref().ok_or(CoreError::Invalid)?;
    let mut outline = None;
    let mut margin = style.width * 2.0;
    let bounds = match object.shape.as_ref().ok_or(CoreError::Invalid)? {
        Shape::Stroke(stroke) => {
            let geometry = vw_ink::geometry_from_stroke(stroke)?;
            let b = geometry.bounds();
            outline = Some(geometry);
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
    Ok((rect(inflate(bounds, margin))?, outline))
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
    let (local, geometry) = local_bounds(
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
    let stroke_contours =
        crate::gesture::contours(geometry.as_ref().map_or(&[][..], |g| g.polygons()))?;
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

/// What a document snapshot carries besides its display list.
struct DocumentFrame {
    id: Id,
    title: String,
    width: u32,
    height: u32,
    bit_depth: u32,
    layers: Vec<LayerInfo>,
    has_tone_adjustments: bool,
}
fn document_frame(state: &ProjectState, document_id: &str) -> Result<DocumentFrame> {
    let project = state.project()?;
    let id = Id::try_from(document_id.to_owned())?;
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
    let has_tone_adjustments = project
        .objects
        .keys()
        .any(|object| crate::tone_controls::is_editing_metadata(project, &id, object));
    Ok(DocumentFrame {
        title: doc.title.clone(),
        width,
        height,
        bit_depth: asset.bit_depth,
        layers,
        has_tone_adjustments,
        id,
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
                let frame = document_frame(state, &document_id)?;
                let result = DocumentSnapshot {
                    document_id,
                    title: frame.title,
                    width: frame.width,
                    height: frame.height,
                    bit_depth: frame.bit_depth,
                    has_tone_adjustments: frame.has_tone_adjustments,
                    layers: frame.layers,
                    render: render_list(state, frame.id.as_str(), None)?,
                };
                crate::payload::document_bytes(&result)?;
                cancellation.check()?;
                Ok(result)
            })
            .await
    }
    /// `document_snapshot` with every stroke outline packed: the same items in
    /// the same order, each `PackedRenderItem::contours` carrying exactly the
    /// values of `RenderItem::stroke_contours`. It is admitted under the same
    /// 64 MB response limit, counting what the packed form costs.
    pub async fn document_snapshot_packed(
        &self,
        document_id: String,
        cancellation: Arc<Cancellation>,
    ) -> Result<PackedDocumentSnapshot> {
        self.check_open()?;
        self.worker
            .call(move |state| {
                cancellation.check()?;
                let frame = document_frame(state, &document_id)?;
                let result = PackedDocumentSnapshot {
                    document_id,
                    title: frame.title,
                    width: frame.width,
                    height: frame.height,
                    bit_depth: frame.bit_depth,
                    has_tone_adjustments: frame.has_tone_adjustments,
                    layers: frame.layers,
                    render: packed_render_list(state, frame.id.as_str(), None)?,
                };
                crate::payload::packed_document_bytes(&result)?;
                cancellation.check()?;
                Ok(result)
            })
            .await
    }
    /// The same document as `document_snapshot`, without rebuilding or
    /// resending the strokes the caller still holds. `known` names those
    /// strokes by the digest this call returned for them earlier; each entry
    /// of the answer is either a complete item or a kept stroke with its
    /// current layer opacity, blend and lock. Entries are in drawing order.
    /// A stroke is kept only while its whole stored state is byte-identical,
    /// so a moved, restyled, re-ordered or erased stroke always comes back
    /// complete. Image documents only; nothing is stored between calls.
    pub async fn document_snapshot_changes(
        &self,
        document_id: String,
        known: Vec<KnownStroke>,
        cancellation: Arc<Cancellation>,
    ) -> Result<DocumentSnapshotChanges> {
        self.check_open()?;
        let held = held_strokes(known)?;
        self.worker
            .call(move |state| {
                cancellation.check()?;
                let frame = document_frame(state, &document_id)?;
                let (revision, entries) = render_entries(
                    state,
                    frame.id.as_str(),
                    None,
                    None,
                    Some(&held),
                    OutlineForm::Lists,
                )?;
                let items = change_entries(entries, |item| match item {
                    DisplayItem::Lists(item) => Some(item),
                    DisplayItem::Packed(_) => None,
                })?
                .into_iter()
                .map(|entry| RenderItemChange {
                    object_id: entry.object_id,
                    digest: entry.digest,
                    cost: entry.cost,
                    item: entry.item,
                    layer_opacity: entry.layer_opacity,
                    layer_blend: entry.layer_blend,
                    locked: entry.locked,
                })
                .collect();
                let result = DocumentSnapshotChanges {
                    document_id,
                    title: frame.title,
                    width: frame.width,
                    height: frame.height,
                    bit_depth: frame.bit_depth,
                    has_tone_adjustments: frame.has_tone_adjustments,
                    layers: frame.layers,
                    revision,
                    items,
                };
                crate::payload::document_changes_bytes(&result)?;
                cancellation.check()?;
                Ok(result)
            })
            .await
    }
    /// `document_snapshot_changes` with every sent stroke outline packed as in
    /// `document_snapshot_packed`. Each entry's `cost` is what the packed item
    /// counts; `known` must name the costs this call returned.
    pub async fn document_snapshot_changes_packed(
        &self,
        document_id: String,
        known: Vec<KnownStroke>,
        cancellation: Arc<Cancellation>,
    ) -> Result<PackedDocumentSnapshotChanges> {
        self.check_open()?;
        let held = held_strokes(known)?;
        self.worker
            .call(move |state| {
                cancellation.check()?;
                let frame = document_frame(state, &document_id)?;
                let (revision, entries) = render_entries(
                    state,
                    frame.id.as_str(),
                    None,
                    None,
                    Some(&held),
                    OutlineForm::Packed,
                )?;
                let items = change_entries(entries, |item| match item {
                    DisplayItem::Packed(item) => Some(item),
                    DisplayItem::Lists(_) => None,
                })?
                .into_iter()
                .map(|entry| PackedRenderItemChange {
                    object_id: entry.object_id,
                    digest: entry.digest,
                    cost: entry.cost,
                    item: entry.item,
                    layer_opacity: entry.layer_opacity,
                    layer_blend: entry.layer_blend,
                    locked: entry.locked,
                })
                .collect();
                let result = PackedDocumentSnapshotChanges {
                    document_id,
                    title: frame.title,
                    width: frame.width,
                    height: frame.height,
                    bit_depth: frame.bit_depth,
                    has_tone_adjustments: frame.has_tone_adjustments,
                    layers: frame.layers,
                    revision,
                    items,
                };
                crate::payload::packed_document_changes_bytes(&result)?;
                cancellation.check()?;
                Ok(result)
            })
            .await
    }
    /// Oriented display pixels, requested independently of edit snapshots.
    /// Explicit RAW imports use only their admitted embedded preview. Source
    /// bytes and depth remain on disk; display conversion is explicitly sRGB/8-bit.
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
                let raw_preview = crate::format_images::asset_is_raw_preview(asset)
                    .map_err(background_raster_error)?;
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
                let orientation = asset.orientation as u8;
                let (width, height) = if orientation >= 5 {
                    (asset.height, asset.width)
                } else {
                    (asset.width, asset.height)
                };
                let assets = Assets {
                    blobs: &state.blobs,
                    project: state.project()?,
                    budget: memory_budget_bytes,
                    cancel: &cancellation,
                };
                // This verifies the immutable source's complete BLAKE3 hash.
                // Its encoded Vec and decoder temporaries are dropped before
                // the consuming display conversion begins; there is no PNG
                // export, crop or re-decode. RAW preview availability remains
                // false throughout its explicitly admitted display route.
                let source = assets
                    .image(&source_asset_id)
                    .map_err(background_raster_error)?;
                cancellation.check()?;
                if source.width != width
                    || source.height != height
                    || source.orientation_applied != orientation
                    || source.pixels.bit_depth() != depth
                {
                    return Err(CoreError::Raster);
                }
                let display = if raw_preview {
                    vw_raster::display_preview_rgba8
                } else {
                    vw_raster::display_rgba8
                };
                let image = display(source, memory_budget_bytes, assume_untagged_srgb, &|| {
                    cancellation.is_cancelled()
                })
                .map_err(background_raster_error)?;
                cancellation.check()?;
                Ok(BackgroundImage {
                    width: image.width,
                    height: image.height,
                    rgba: image.rgba,
                    source_asset_id,
                    source_bit_depth: image.source_bit_depth,
                    icc_profile: image.icc_profile,
                })
            })
            .await
    }
}

// Only the background boundary classifies budget/cancellation errors. Keep the
// existing Raster/color-consent ABI for untagged and other raster refusals; do
// not change global export/import error mapping or silently assume source color.
fn background_raster_error(error: vw_raster::RasterError) -> CoreError {
    match error {
        vw_raster::RasterError::Memory { .. } | vw_raster::RasterError::Allocation => {
            CoreError::Backpressure
        }
        vw_raster::RasterError::Cancelled => CoreError::Cancelled,
        vw_raster::RasterError::CodecUnavailable => CoreError::ImageCodecUnavailable,
        _ => CoreError::Raster,
    }
}

struct Assets<'a> {
    blobs: &'a vw_store::BlobStore,
    project: &'a Project,
    budget: u64,
    cancel: &'a Cancellation,
}
impl AssetResolver for Assets<'_> {
    fn image(&self, asset: &str) -> std::result::Result<DecodedImage, vw_raster::RasterError> {
        let id = AssetId::try_from(asset.to_owned())
            .map_err(|_| vw_raster::RasterError::MissingAsset)?;
        let asset = self
            .project
            .assets
            .get(&id)
            .ok_or(vw_raster::RasterError::MissingAsset)?;
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
        crate::format_images::decode_asset(
            &bytes,
            vw_raster::DecodeLimits {
                max_encoded_bytes: limit,
                max_pixels: 50_000_000,
                max_memory_bytes: budget,
            },
            &|| self.cancel.is_cancelled(),
            asset,
        )
    }
}
pub(crate) fn export(
    state: &ProjectState,
    options: ExportOptions,
    cancellation: &Cancellation,
) -> Result<ExportResult> {
    let document = Id::try_from(options.document_id.clone())?;
    let definition = &state
        .project()?
        .documents
        .get(&document)
        .ok_or(CoreError::Invalid)?
        .definition;
    if options.memory_budget_bytes > 512 * 1024 * 1024 || options.memory_budget_bytes == 0 {
        return Err(CoreError::Invalid);
    }
    // The admission helper meters canonical validation/hash workspace before
    // requires_document_output validates the current model or reads originals.
    crate::image_frames::admit_legacy_export(state, &options)?;
    if vw_raster::requires_document_output(state.project()?, &document, options.marked)? {
        cancellation.check()?;
        return crate::image_frames::legacy_export(state, &options, cancellation);
    }
    let assets = Assets {
        blobs: &state.blobs,
        project: state.project()?,
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
            project: state.project()?,
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
            ImageFormat::Bmp8 => vw_raster::ExportFormat::Bmp8,
            ImageFormat::Tiff8 => vw_raster::ExportFormat::Tiff8,
            ImageFormat::Tiff16 => vw_raster::ExportFormat::Tiff16,
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

        capture_provenance: definition
            .capture
            .as_ref()
            .filter(|capture| capture.stream_save.is_some())
            .cloned(),
    };
    let result = if crate::format_images::external(request.format) {
        // The external adapter reserves retained input itself.
        let mut request = request;
        request.memory_budget_bytes = options.memory_budget_bytes;
        crate::format_images::encode_buffered(&pixels, &request, cancellation)?
    } else {
        vw_raster::export(&pixels, &request)?
    };
    cancellation.check()?;
    if result.bytes.len() > 64 * 1024 * 1024 {
        return Err(CoreError::Backpressure);
    }
    let revision = state.info()?;
    let capture_provenance_json = crate::capture_provenance::export_receipt(
        &result.metadata,
        &options.document_id,
        &revision,
    )?;
    Ok(ExportResult {
        capture_provenance_json,
        blake3: AssetId::hash(&result.bytes).to_string(),
        bytes: result.bytes,
        metadata_json: serde_json::to_string(&result.metadata).map_err(|_| CoreError::Invalid)?,
        revision,
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

#[cfg(test)]
#[path = "queries_output_tests.rs"]
pub(crate) mod output_tests;
