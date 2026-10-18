use crate::{Error, Result, Target};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageMapping {
    /// O-space rectangle when output_frame is present; otherwise the original D grid.
    pub source_rectangle: [i64; 4],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_frame: Option<vw_raster::ImageOutputMetadata>,
    pub scale_numerator: u32,
    pub scale_denominator: u32,
    pub content_width: u32,
    pub content_height: u32,
    pub width: u32,
    pub height: u32,
}
impl ImageMapping {
    pub(crate) fn new(rect: [i64; 4], edge: u32, minimum: u32) -> Result<Self> {
        if rect[2] <= 0 || rect[3] <= 0 || rect[2] > 32768 || rect[3] > 32768 {
            return Err(Error::Limit("image extent"));
        }
        let long = rect[2].max(rect[3]) as u32;
        let (n, d) = if long > edge { (edge, long) } else { (1, 1) };
        let width = (rect[2] as u64 * u64::from(n)).div_ceil(u64::from(d)) as u32;
        let height = (rect[3] as u64 * u64::from(n)).div_ceil(u64::from(d)) as u32;
        Ok(Self {
            source_rectangle: rect,
            output_frame: None,
            scale_numerator: n,
            scale_denominator: d,
            content_width: width,
            content_height: height,
            width: width.max(minimum),
            height: height.max(minimum),
        })
    }
    pub(crate) fn with_output(mut self, output: Option<vw_raster::ImageOutputMetadata>) -> Result<Self> {
        if let Some(value) = output { value.frame()?; }
        self.output_frame = output;
        Ok(self)
    }
    pub fn scale(&self) -> f64 {
        f64::from(self.scale_numerator) / f64::from(self.scale_denominator)
    }
    pub fn point(&self, point: [f64; 2], target: &Target) -> [f64; 2] {
        let point = if let Some(output) = self.output_frame {
            output.frame().ok().and_then(|frame| vw_geom::Point::new(point[0], point[1]).ok().and_then(|point| frame.point(point).ok()))
                .map_or([f64::NAN; 2], |point| [point.x(), point.y()])
        } else { point };
        let x = (point[0] - self.source_rectangle[0] as f64) * self.scale();
        let y = (point[1] - self.source_rectangle[1] as f64) * self.scale();
        if target.convention() == "normalized_1000_yxyx" {
            [
                y / f64::from(self.height) * 1000.0,
                x / f64::from(self.width) * 1000.0,
            ]
        } else {
            [x, y]
        }
    }
    pub fn bounds(&self, b: [f64; 4], target: &Target) -> [f64; 4] {
        let corners = [[b[0], b[1]], [b[0]+b[2], b[1]], [b[0], b[1]+b[3]], [b[0]+b[2], b[1]+b[3]]];
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for point in corners {
            let p = self.point(point, target);
            if p.iter().any(|v| !v.is_finite()) { return [f64::NAN; 4]; }
            for axis in 0..2 { lo[axis] = lo[axis].min(p[axis]); hi[axis] = hi[axis].max(p[axis]); }
        }
        if target.convention() == "normalized_1000_yxyx" {
            [lo[0], lo[1], hi[0], hi[1]]
        } else {
            [lo[0], lo[1], hi[0]-lo[0], hi[1]-lo[1]]
        }
    }
}
pub(crate) fn marker_bounds(
    point: [f64; 2],
    bounds: Option<[f64; 4]>,
    w: u32,
    h: u32,
) -> Result<[f64; 4]> {
    if !point.iter().all(|n| n.is_finite())
        || point[0] < 0.0
        || point[1] < 0.0
        || point[0] >= f64::from(w)
        || point[1] >= f64::from(h)
    {
        return Err(Error::Invalid("marker point outside image"));
    }
    let b = bounds.unwrap_or([
        (point[0] - 0.5).clamp(0.0, f64::from(w) - 1.0),
        (point[1] - 0.5).clamp(0.0, f64::from(h) - 1.0),
        1.0,
        1.0,
    ]);
    if !b.iter().all(|n| n.is_finite())
        || b[0] < 0.0
        || b[1] < 0.0
        || b[2] <= 0.0
        || b[3] <= 0.0
        || b[0] + b[2] > f64::from(w)
        || b[1] + b[3] > f64::from(h)
    {
        return Err(Error::Invalid("marker bounds outside image"));
    }
    Ok(b)
}
pub(crate) fn crop(bounds: [f64; 4], w: u32, h: u32, edge: u32) -> Result<ImageMapping> {
    let axis = |start: f64, len: f64, extent: u32| -> (i64, i64) {
        let size = (len * 1.75).ceil().max(256.0) as i64;
        let size = if extent >= 256 {
            size.min(i64::from(extent))
        } else {
            size
        };
        let origin = ((start + len / 2.0) - size as f64 / 2.0).floor() as i64;
        let origin = if size <= i64::from(extent) {
            origin.clamp(0, i64::from(extent) - size)
        } else {
            0
        };
        (origin, size)
    };
    let (x, width) = axis(bounds[0], bounds[2], w);
    let (y, height) = axis(bounds[1], bounds[3], h);
    ImageMapping::new([x, y, width, height], edge, 256)
}

/// All retained instructions must remain wholly inside the selected output crop.
/// Explicit refusal is preferable to silently omitting or clipping their target.
pub(crate) fn output_bounds(bounds: [f64;4], point: [f64;2], output: Option<vw_raster::ImageOutputMetadata>) -> Result<[f64;4]> {
    let Some(output) = output else { return Ok(bounds); };
    let frame = output.frame()?;
    let rect = vw_geom::Rect::new(bounds[0], bounds[1], bounds[2], bounds[3]).map_err(|_| Error::Integrity)?;
    let [x,y,w,h] = frame.crop();
    if !frame.contains(rect) || point[0] < f64::from(x) || point[1] < f64::from(y)
        || point[0] >= f64::from(x+w) || point[1] >= f64::from(y+h) {
        return Err(Error::Unsupported("instruction target lies outside the output crop"));
    }
    let rect = frame.bounds(rect).map_err(|_| Error::Integrity)?;
    Ok([rect.left(), rect.top(), rect.width(), rect.height()])
}
