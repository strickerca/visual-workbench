//! Conservative complete FFI response accounting, including typed geometry and
//! duplicated metadata, before a response is accumulated or lowered to Kotlin.
use crate::*;
pub(crate) const MAX_RESPONSE: usize = 64 * 1024 * 1024;
/// The fixed part every display item counts, before its strings and arrays.
pub(crate) const MIN_ITEM: usize = 1024;
fn add(total: &mut usize, value: usize) -> Result<()> {
    *total = total
        .checked_add(value)
        .filter(|n| *n <= MAX_RESPONSE)
        .ok_or(CoreError::Backpressure)?;
    Ok(())
}
fn array(total: &mut usize, count: usize, stride: usize) -> Result<()> {
    add(
        total,
        count.checked_mul(stride).ok_or(CoreError::Backpressure)?,
    )?;
    add(total, 32)
}
fn string(total: &mut usize, value: &str) -> Result<()> {
    array(total, value.len(), 4)
}
pub(crate) fn info_bytes(info: &ProjectInfo) -> Result<usize> {
    let mut n = 128;
    for s in [
        &info.project_id,
        &info.title,
        &info.device_id,
        &info.state_hash,
    ] {
        string(&mut n, s)?;
    }
    for s in &info.document_ids {
        string(&mut n, s)?;
    }
    Ok(n)
}
/// What each byte of a packed outline (`PackedRenderItem::contours`) counts.
/// While the response is lowered, the item's buffer and its copy in the
/// transfer buffer exist together; that buffer grows by doubling, so it holds
/// up to twice what it carries and, at the moment it grows, its previous
/// copy as well. Kotlin reads the transfer buffer in place and the generated
/// binding copies the bytes once into a ByteArray, which the shared facade
/// keeps as it is; the transfer buffer is released once read. Four bytes per
/// byte bound each of these phases. Building the outline holds more for a
/// moment; `packed_build_bytes` counts that.
pub(crate) const PACKED_BYTE: usize = 4;
/// The fixed part, the strings and the shape geometry every item counts,
/// whatever form its outline takes.
fn item_frame(
    object_id: &str,
    layer_id: &str,
    layer_blend: &str,
    shape: &DrawShape,
) -> Result<usize> {
    let mut n = MIN_ITEM;
    for s in [object_id, layer_id, layer_blend] {
        string(&mut n, s)?;
    }
    match shape {
        DrawShape::Stroke { family } => string(&mut n, family)?,
        DrawShape::Line { points }
        | DrawShape::Arrow { points }
        | DrawShape::Polygon { points, .. } => array(&mut n, points.len(), 112)?,
        DrawShape::Text {
            outline,
            text,
            font,
            ..
        } => {
            array(&mut n, outline.len(), 128)?;
            string(&mut n, text)?;
            string(&mut n, font)?;
        }
        DrawShape::Result {
            asset_id,
            result_id,
        } => {
            string(&mut n, asset_id)?;
            string(&mut n, result_id)?;
        }
        _ => {}
    }
    Ok(n)
}
pub(crate) fn item_bytes(item: &RenderItem) -> Result<usize> {
    let mut n = item_frame(
        &item.object_id,
        &item.layer_id,
        &item.layer_blend,
        &item.shape,
    )?;
    // Generated primitive sequences are boxed Kotlin Lists before the public
    // facade converts contours to primitive arrays. Reserve both representations.
    array(&mut n, item.object_protobuf.len(), 2)?;
    array(&mut n, item.stroke_contours.x.len(), 48)?;
    array(&mut n, item.stroke_contours.y.len(), 48)?;
    array(&mut n, item.stroke_contours.ends.len(), 32)?;
    Ok(n)
}
/// What building one stroke's packed outline holds, for a moment, beside the
/// items already built: the samples vw-ink reads, its contours at their
/// allocated size, the lists that collect them (the builder's, grown by
/// doubling and briefly next to its previous block, and the geometry's), the
/// working buffers of one contour (vw-ink caps a contour at 514 points), and
/// the packed bytes, made while the geometry is still alive. All of it is
/// released before the next item is built, so a read counts it once, on top
/// of what it has counted so far: `admits` checks that sum before the bytes
/// are made. It is known only once the geometry exists.
pub(crate) fn packed_build_bytes(
    samples: usize,
    geometry: &vw_ink::Geometry,
    packed: usize,
) -> Result<usize> {
    let mut n = 64 * 1024;
    let polygons = geometry.polygons();
    add(
        &mut n,
        samples
            .checked_mul(std::mem::size_of::<vw_ink::Sample>())
            .ok_or(CoreError::Backpressure)?,
    )?;
    add(
        &mut n,
        polygons
            .len()
            .checked_mul(3 * std::mem::size_of::<vw_ink::Polygon>())
            .ok_or(CoreError::Backpressure)?,
    )?;
    for polygon in polygons {
        add(
            &mut n,
            polygon
                .points
                .capacity()
                .checked_mul(std::mem::size_of::<vw_ink::QPoint>())
                .ok_or(CoreError::Backpressure)?,
        )?;
    }
    add(&mut n, packed)?;
    Ok(n)
}
/// Whether `transient` more bytes fit beside the `total` counted so far.
pub(crate) fn admits(total: usize, transient: usize) -> Result<()> {
    let mut n = total;
    add(&mut n, transient)
}
pub(crate) fn packed_item_bytes(item: &PackedRenderItem) -> Result<usize> {
    let mut n = item_frame(
        &item.object_id,
        &item.layer_id,
        &item.layer_blend,
        &item.shape,
    )?;
    array(&mut n, item.contours.len(), PACKED_BYTE)?;
    Ok(n)
}
pub(crate) fn accumulate(total: &mut usize, addition: usize) -> Result<()> {
    add(total, addition)
}
/// What a document read counts besides its display list.
fn document_frame(
    revision: &ProjectInfo,
    document_id: &str,
    title: &str,
    layers: &[LayerInfo],
) -> Result<usize> {
    let mut n = info_bytes(revision)?;
    add(&mut n, 128)?;
    string(&mut n, document_id)?;
    string(&mut n, title)?;
    for layer in layers {
        add(&mut n, 128)?;
        for value in [&layer.id, &layer.name, &layer.blend] {
            string(&mut n, value)?;
        }
    }
    Ok(n)
}
/// A changes entry, sent or kept, counts what its item counts in a complete
/// read: the caller holds the kept ones too.
fn change_bytes(n: &mut usize, object_id: &str, layer_blend: &str, cost: u64) -> Result<()> {
    add(n, 256)?;
    string(n, object_id)?;
    string(n, layer_blend)?;
    add(
        n,
        usize::try_from(cost).map_err(|_| CoreError::Backpressure)?,
    )
}
pub(crate) fn document_bytes(document: &DocumentSnapshot) -> Result<()> {
    let mut n = document_frame(
        &document.render.revision,
        &document.document_id,
        &document.title,
        &document.layers,
    )?;
    for item in &document.render.items {
        add(&mut n, item_bytes(item)?)?;
    }
    Ok(())
}
pub(crate) fn packed_document_bytes(document: &PackedDocumentSnapshot) -> Result<()> {
    let mut n = document_frame(
        &document.render.revision,
        &document.document_id,
        &document.title,
        &document.layers,
    )?;
    for item in &document.render.items {
        add(&mut n, packed_item_bytes(item)?)?;
    }
    Ok(())
}
pub(crate) fn document_changes_bytes(document: &DocumentSnapshotChanges) -> Result<()> {
    let mut n = document_frame(
        &document.revision,
        &document.document_id,
        &document.title,
        &document.layers,
    )?;
    for change in &document.items {
        change_bytes(&mut n, &change.object_id, &change.layer_blend, change.cost)?;
    }
    Ok(())
}
pub(crate) fn packed_document_changes_bytes(
    document: &PackedDocumentSnapshotChanges,
) -> Result<()> {
    let mut n = document_frame(
        &document.revision,
        &document.document_id,
        &document.title,
        &document.layers,
    )?;
    for change in &document.items {
        change_bytes(&mut n, &change.object_id, &change.layer_blend, change.cost)?;
    }
    Ok(())
}
pub(crate) fn layout_bytes(layout: &vw_raster::TextLayout) -> Result<()> {
    let mut n = 128;
    array(&mut n, layout.glyphs.len(), 256)?;
    for glyph in &layout.glyphs {
        array(&mut n, glyph.outline.len(), 128)?;
    }
    Ok(())
}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn large_typed_outline_is_counted_even_when_protobuf_is_small() {
        let shape = DrawShape::Text {
            anchor: Point { x: 0.0, y: 0.0 },
            text: "x".into(),
            font: "Inter".into(),
            size: 12.0,
            outline: vec![
                Outline::Cubic {
                    x1: 0.0,
                    y1: 0.0,
                    x2: 1.0,
                    y2: 1.0,
                    x: 2.0,
                    y: 2.0
                };
                750000
            ],
        };
        let item = RenderItem {
            object_id: String::new(),
            layer_id: String::new(),
            layer_opacity: 1.0,
            layer_blend: "normal".into(),
            bounds: QueryRect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            object_protobuf: vec![0; 20],
            stroke_contours: Contours::default(),
            transform: Transform {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                e: 0.0,
                f: 0.0,
            },
            style: ObjectStyle {
                rgba: 0,
                width: 1.0,
                screen_constant_width: false,
                fill: None,
            },
            shape,
            locked: false,
        };
        assert!(matches!(item_bytes(&item), Err(CoreError::Backpressure)));
        let mut total = MAX_RESPONSE - 100;
        assert!(accumulate(&mut total, 101).is_err());
    }
}
