//! A nondestructive document-output grid, independent of the local camera.
use crate::{Affine, GeometryError, Point, QuarterTurn, Rect, Size};

/// Maximum side admitted by the first explicit image-output convention.
pub const IMAGE_OUTPUT_MAX_SIDE: u32 = 32_768;

/// Integer D-space crop followed by an exact clockwise quarter-turn and
/// independent positive output-axis resize. Pixel edges use the same affine
/// for pixels, package coordinates and output-preview overlays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageOutputFrame {
    source: [u32; 2],
    crop: [u32; 4],
    rotation: QuarterTurn,
    output: [u32; 2],
}
impl ImageOutputFrame {
    /// Construct a positive, in-bounds integer crop. Nothing changes the source
    /// grid or any annotation's D-space coordinates.
    pub fn new(
        source: [u32; 2],
        crop: [u32; 4],
        rotation: QuarterTurn,
        output: [u32; 2],
    ) -> Result<Self, GeometryError> {
        if source
            .into_iter()
            .chain(output)
            .any(|n| n == 0 || n > IMAGE_OUTPUT_MAX_SIDE)
            || crop[2] == 0
            || crop[3] == 0
        {
            return Err(GeometryError::EmptyExtent);
        }
        if crop[0].checked_add(crop[2]).is_none_or(|v| v > source[0])
            || crop[1].checked_add(crop[3]).is_none_or(|v| v > source[1])
        {
            return Err(GeometryError::OutOfBounds);
        }
        Ok(Self {
            source,
            crop,
            rotation,
            output,
        })
    }
    /// Full original with no rotation or resizing.
    pub fn identity(width: u32, height: u32) -> Result<Self, GeometryError> {
        Self::new(
            [width, height],
            [0, 0, width, height],
            QuarterTurn::Zero,
            [width, height],
        )
    }
    /// Oriented original extent in D pixels.
    pub const fn source_size(self) -> [u32; 2] {
        self.source
    }
    /// Integer D-space `[left, top, width, height]`.
    pub const fn crop(self) -> [u32; 4] {
        self.crop
    }
    /// Exact clockwise output rotation.
    pub const fn rotation(self) -> QuarterTurn {
        self.rotation
    }
    /// Final physical output pixel dimensions; no implicit aspect correction.
    pub const fn output_size(self) -> [u32; 2] {
        self.output
    }
    /// Crop dimensions after the quarter-turn, before resizing.
    pub const fn rotated_size(self) -> [u32; 2] {
        match self.rotation {
            QuarterTurn::Zero | QuarterTurn::Half => [self.crop[2], self.crop[3]],
            QuarterTurn::Clockwise90 | QuarterTurn::Clockwise270 => [self.crop[3], self.crop[2]],
        }
    }
    /// True only when the output preserves the complete D grid.
    pub fn is_identity(self) -> bool {
        self.crop == [0, 0, self.source[0], self.source[1]]
            && self.rotation == QuarterTurn::Zero
            && self.output == self.source
    }
    /// Map continuous D edges/centers into output O. Applying this to the
    /// camera would be incorrect: editing remains in the original D grid.
    pub fn matrix(self) -> Result<Affine, GeometryError> {
        let [x, y, w, h] = self.crop;
        let [rw, rh] = self.rotated_size();
        Affine::new(1.0, 0.0, 0.0, 1.0, -f64::from(x), -f64::from(y))?
            .then(
                self.rotation
                    .matrix(Size::new(f64::from(w), f64::from(h))?)?,
            )?
            .then(Affine::new(
                f64::from(self.output[0]) / f64::from(rw),
                0.0,
                0.0,
                f64::from(self.output[1]) / f64::from(rh),
                0.0,
                0.0,
            )?)
    }
    /// Project one finite D point, including points outside the crop. Callers
    /// decide whether out-of-output points are clipped or explicitly refused.
    pub fn point(self, point: Point) -> Result<Point, GeometryError> {
        self.matrix()?.map(point)
    }
    /// Project all four rectangle corners. Quarter-turns preserve an exact
    /// axis-aligned bounding box even when output axes have different scales.
    pub fn bounds(self, rect: Rect) -> Result<Rect, GeometryError> {
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for [x, y] in [
            [rect.left(), rect.top()],
            [rect.right(), rect.top()],
            [rect.left(), rect.bottom()],
            [rect.right(), rect.bottom()],
        ] {
            let p = self.point(Point::new(x, y)?)?;
            lo[0] = lo[0].min(p.x());
            lo[1] = lo[1].min(p.y());
            hi[0] = hi[0].max(p.x());
            hi[1] = hi[1].max(p.y());
        }
        Rect::new(lo[0], lo[1], hi[0] - lo[0], hi[1] - lo[1])
    }
    /// Intersect an integral D region with the crop and cover its projected O
    /// edges with whole output pixels. Integer ratios avoid adding a spurious
    /// pixel when a mathematically integral edge has floating-point error.
    pub fn source_region(self, region: [u32; 4]) -> Result<[u32; 4], GeometryError> {
        let [x, y, w, h] = region;
        let right = x.checked_add(w).ok_or(GeometryError::OutOfBounds)?;
        let bottom = y.checked_add(h).ok_or(GeometryError::OutOfBounds)?;
        if w == 0 || h == 0 || right > self.source[0] || bottom > self.source[1] {
            return Err(GeometryError::OutOfBounds);
        }
        let [cx, cy, cw, ch] = self.crop;
        let left = x.max(cx);
        let top = y.max(cy);
        let right = right.min(cx + cw);
        let bottom = bottom.min(cy + ch);
        if left >= right || top >= bottom {
            return Err(GeometryError::EmptyExtent);
        }
        let [u0, v0, u1, v1] = [left - cx, top - cy, right - cx, bottom - cy];
        let [x0, y0, x1, y1] = match self.rotation {
            QuarterTurn::Zero => [u0, v0, u1, v1],
            QuarterTurn::Clockwise90 => [ch - v1, u0, ch - v0, u1],
            QuarterTurn::Half => [cw - u1, ch - v1, cw - u0, ch - v0],
            QuarterTurn::Clockwise270 => [v0, cw - u1, v1, cw - u0],
        };
        let [rw, rh] = self.rotated_size();
        // Every operand is at most IMAGE_OUTPUT_MAX_SIDE, so products and
        // the ceiling adjustment fit in u64 and quotients fit in u32.
        let floor =
            |n: u32, out: u32, size: u32| (u64::from(n) * u64::from(out) / u64::from(size)) as u32;
        let ceil = |n: u32, out: u32, size: u32| {
            (u64::from(n) * u64::from(out)).div_ceil(u64::from(size)) as u32
        };
        let ox = floor(x0, self.output[0], rw);
        let oy = floor(y0, self.output[1], rh);
        let ow = ceil(x1, self.output[0], rw) - ox;
        let oh = ceil(y1, self.output[1], rh) - oy;
        Ok([ox, oy, ow, oh])
    }
    /// True when the complete supplied D rectangle lies inside the crop.
    /// Package compilation uses this to refuse silently omitted instructions.
    pub fn contains(self, rect: Rect) -> bool {
        rect.left() >= f64::from(self.crop[0])
            && rect.top() >= f64::from(self.crop[1])
            && rect.right() <= f64::from(self.crop[0] + self.crop[2])
            && rect.bottom() <= f64::from(self.crop[1] + self.crop[3])
    }
    /// Convert an index in the rotated crop back to the exact original pixel.
    /// No DPI, floating-point rounding or sample interpolation is involved.
    pub fn source_pixel(self, x: u32, y: u32) -> Result<[u32; 2], GeometryError> {
        let [rw, rh] = self.rotated_size();
        if x >= rw || y >= rh {
            return Err(GeometryError::OutOfBounds);
        }
        let [cx, cy, w, h] = self.crop;
        let [sx, sy] = match self.rotation {
            QuarterTurn::Zero => [x, y],
            QuarterTurn::Clockwise90 => [y, h - 1 - x],
            QuarterTurn::Half => [w - 1 - x, h - 1 - y],
            QuarterTurn::Clockwise270 => [w - 1 - y, x],
        };
        Ok([cx + sx, cy + sy])
    }
    /// Decode the canonical Crop+Affine representation, rejecting shear,
    /// reflection, fractional crop edges, nonintegral output extents, or an
    /// arbitrary translation. Tiny serialization error is accepted only after
    /// the typed integer frame has been recovered and all six coefficients
    /// match its canonical matrix within a 1e-9 output-pixel envelope.
    pub fn from_crop_affine(
        source: [u32; 2],
        crop: Rect,
        affine: Affine,
    ) -> Result<Self, GeometryError> {
        let integer = |v: f64| -> Result<u32, GeometryError> {
            if !v.is_finite()
                || !(0.0..=f64::from(IMAGE_OUTPUT_MAX_SIDE)).contains(&v)
                || v != libm::floor(v)
            {
                return Err(GeometryError::OutOfBounds);
            }
            Ok(v as u32)
        };
        let crop = [
            integer(crop.left())?,
            integer(crop.top())?,
            integer(crop.width())?,
            integer(crop.height())?,
        ];
        let [a, b, c, d, _, _] = affine.coefficients();
        let (rotation, sx, sy) = if b == 0.0 && c == 0.0 && a > 0.0 && d > 0.0 {
            (QuarterTurn::Zero, a, d)
        } else if a == 0.0 && d == 0.0 && b > 0.0 && c < 0.0 {
            (QuarterTurn::Clockwise90, -c, b)
        } else if b == 0.0 && c == 0.0 && a < 0.0 && d < 0.0 {
            (QuarterTurn::Half, -a, -d)
        } else if a == 0.0 && d == 0.0 && b < 0.0 && c > 0.0 {
            (QuarterTurn::Clockwise270, c, -b)
        } else {
            return Err(GeometryError::InvalidOrientation);
        };
        let rotated = match rotation {
            QuarterTurn::Zero | QuarterTurn::Half => [crop[2], crop[3]],
            _ => [crop[3], crop[2]],
        };
        let extent = |scale: f64, size: u32| -> Result<u32, GeometryError> {
            let v = scale * f64::from(size);
            let n = libm::round(v);
            if !v.is_finite()
                || !(1.0..=f64::from(IMAGE_OUTPUT_MAX_SIDE)).contains(&n)
                || (v - n).abs() > 1e-9
            {
                return Err(GeometryError::InvalidScale);
            }
            Ok(n as u32)
        };
        let frame = Self::new(
            source,
            crop,
            rotation,
            [extent(sx, rotated[0])?, extent(sy, rotated[1])?],
        )?;
        // Compare coefficient error in output-pixel units, not a generous
        // relative tolerance that could admit visible drift on a large grid.
        let expected = frame.matrix()?.coefficients();
        let actual = affine.coefficients();
        for (index, (expected, actual)) in expected.into_iter().zip(actual).enumerate() {
            let span = if index < 4 {
                f64::from(source[0].max(source[1]))
            } else {
                1.0
            };
            if (expected - actual).abs() * span > 1e-9 {
                return Err(GeometryError::OutOfBounds);
            }
        }
        Ok(frame)
    }
}
