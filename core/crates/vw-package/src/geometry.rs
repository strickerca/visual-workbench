use crate::{Error, Result, Target};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageMapping {
    pub source_rectangle: [i64; 4],
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
            scale_numerator: n,
            scale_denominator: d,
            content_width: width,
            content_height: height,
            width: width.max(minimum),
            height: height.max(minimum),
        })
    }
    pub fn scale(&self) -> f64 {
        f64::from(self.scale_numerator) / f64::from(self.scale_denominator)
    }
    pub fn point(&self, point: [f64; 2], target: &Target) -> [f64; 2] {
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
        let a = self.point([b[0], b[1]], target);
        let z = self.point([b[0] + b[2], b[1] + b[3]], target);
        if target.convention() == "normalized_1000_yxyx" {
            [a[0], a[1], z[0], z[1]]
        } else {
            [a[0], a[1], b[2] * self.scale(), b[3] * self.scale()]
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
