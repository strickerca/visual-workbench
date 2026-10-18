//! Additional selection primitives, sharing the v1 scan converter and rounding.
use crate::{MAX_WORK, Mask, MaskError, Point, Rect, Result, Size, quantize};

impl Mask {
    /// Axis-aligned ellipse inscribed in the supplied document-space rectangle.
    /// Bounding edges are quantized to 1/256 px before rasterization. Coverage
    /// uses the existing 256 horizontal sample centers and integer area rounding;
    /// it never approximates an ellipse with a display-dependent polygon.
    pub fn ellipse(size: Size, rectangle: Rect) -> Result<Self> {
        let Some(edges) = ellipse_edges(size, rectangle)? else {
            return Ok(Self::empty(size));
        };
        crate::coverage::rasterize_ellipse(size, edges)
    }

    /// Nonallocating geometry/work admission for an ellipse. The facade uses
    /// this before loading an existing mask; no rejected work reads a blob.
    pub fn admit_ellipse(size: Size, rectangle: Rect) -> Result<()> {
        ellipse_edges(size, rectangle).map(|_| ())
    }

    /// Closed straight-edged polygon. Repeated closing vertices are optional;
    /// winding-independent even-odd coverage is the same as a closed lasso.
    pub fn polygon(size: Size, vertices: &[Point]) -> Result<Self> {
        Self::lasso(size, vertices)
    }
}

fn ellipse_edges(size: Size, rectangle: Rect) -> Result<Option<[i64; 4]>> {
    if !rectangle.width.is_finite()
        || !rectangle.height.is_finite()
        || rectangle.width < 0.0
        || rectangle.height < 0.0
    {
        return Err(MaskError::Invalid);
    }
    let left = quantize(rectangle.x)?;
    let top = quantize(rectangle.y)?;
    let right = quantize(rectangle.x + rectangle.width)?;
    let bottom = quantize(rectangle.y + rectangle.height)?;
    if right <= left
        || bottom <= top
        || right <= 0
        || bottom <= 0
        || left >= i64::from(size.width()) * 256
        || top >= i64::from(size.height()) * 256
    {
        return Ok(None);
    }
    let first = top.div_euclid(256).max(0);
    let end = (bottom + 255).div_euclid(256).min(i64::from(size.height()));
    // One primitive yields at most two events: the canonical rasterizer charges
    // upper=3 and 3*(ilog2(3)+2)=9 per subscanline, plus one full row traversal.
    let work = (end - first) as u64 * (u64::from(size.width()) + 256 * 9);
    if work > MAX_WORK {
        return Err(MaskError::Limit);
    }
    Ok(Some([left, top, right, bottom]))
}
