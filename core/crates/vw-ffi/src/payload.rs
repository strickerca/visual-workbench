//! Conservative complete FFI response accounting, including typed geometry and
//! duplicated metadata, before a response is accumulated or lowered to Kotlin.
use crate::*;
pub(crate) const MAX_RESPONSE: usize = 64 * 1024 * 1024;
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
pub(crate) fn item_bytes(item: &RenderItem) -> Result<usize> {
    let mut n = 1024;
    for s in [&item.object_id, &item.layer_id, &item.layer_blend] {
        string(&mut n, s)?;
    }
    // Generated primitive sequences are boxed Kotlin Lists before the public
    // facade converts contours to primitive arrays. Reserve both representations.
    array(&mut n, item.object_protobuf.len(), 2)?;
    array(&mut n, item.stroke_contours.x.len(), 48)?;
    array(&mut n, item.stroke_contours.y.len(), 48)?;
    array(&mut n, item.stroke_contours.ends.len(), 32)?;
    match &item.shape {
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
pub(crate) fn accumulate(total: &mut usize, addition: usize) -> Result<()> {
    add(total, addition)
}
pub(crate) fn document_bytes(document: &DocumentSnapshot) -> Result<()> {
    let mut n = info_bytes(&document.render.revision)?;
    add(&mut n, 128)?;
    string(&mut n, &document.document_id)?;
    string(&mut n, &document.title)?;
    for layer in &document.layers {
        add(&mut n, 128)?;
        for value in [&layer.id, &layer.name, &layer.blend] {
            string(&mut n, value)?;
        }
    }
    for item in &document.render.items {
        add(&mut n, item_bytes(item)?)?;
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
