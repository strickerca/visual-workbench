use crate::{
    ALGORITHM_VERSION, BrushFamily, InkError, MAX_VERTICES, QUANTIZATION, valid_coordinate,
};
use serde::{Deserialize, Serialize};
use vw_model::AssetId;

/// Exact document coordinate in 1/256-unit increments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QPoint {
    pub x: i64,
    pub y: i64,
}
impl QPoint {
    pub fn x_px(self) -> f64 {
        self.x as f64 / QUANTIZATION
    }
    pub fn y_px(self) -> f64 {
        self.y as f64 / QUANTIZATION
    }
    pub(crate) fn quantize(x: f64, y: f64) -> Result<Self, InkError> {
        let limit = (crate::MAX_COORDINATE + crate::MAX_BASE_WIDTH) * QUANTIZATION;
        let x = libm::round(x * QUANTIZATION);
        let y = libm::round(y * QUANTIZATION);
        if !x.is_finite() || !y.is_finite() || x < -limit || x > limit || y < -limit || y > limit {
            return Err(InkError::Invalid("computed coordinate"));
        }
        Ok(Self {
            x: x as i64,
            y: y as i64,
        })
    }
}

/// One convex, same-winding contour in the compound stroke fill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Polygon {
    pub points: Vec<QPoint>,
}
impl Polygon {
    pub(crate) fn from_points(mut points: Vec<QPoint>) -> Result<Option<Self>, InkError> {
        // Quantization can introduce tiny concavities or duplicate vertices.
        // The integer hull restores a convex contour without float predicates.
        points.sort_unstable_by_key(|point| (point.x, point.y));
        points.dedup();
        if points.len() < 3 {
            return Ok(None);
        }
        let mut hull = Vec::new();
        hull.try_reserve_exact(points.len().checked_mul(2).ok_or(InkError::ResourceLimit)?)
            .map_err(|_| InkError::Allocation)?;
        for point in &points {
            while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], *point) <= 0
            {
                hull.pop();
            }
            hull.push(*point);
        }
        let lower = hull.len();
        for point in points.iter().rev().skip(1) {
            while hull.len() > lower
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], *point) <= 0
            {
                hull.pop();
            }
            hull.push(*point);
        }
        hull.pop();
        if hull.len() < 3 {
            return Ok(None);
        }
        Ok(Some(Self { points: hull }))
    }
    fn distance(&self, x: f64, y: f64) -> f64 {
        let mut inside = true;
        let mut nearest = f64::INFINITY;
        for index in 0..self.points.len() {
            let a = self.points[index];
            let b = self.points[(index + 1) % self.points.len()];
            let (ax, ay, bx, by) = (a.x_px(), a.y_px(), b.x_px(), b.y_px());
            let (dx, dy, px, py) = (bx - ax, by - ay, x - ax, y - ay);
            if dx * py - dy * px < 0.0 {
                inside = false;
            }
            let squared = dx * dx + dy * dy;
            let t = if squared == 0.0 {
                0.0
            } else {
                ((px * dx + py * dy) / squared).clamp(0.0, 1.0)
            };
            let (ex, ey) = (px - t * dx, py - t * dy);
            let distance = libm::sqrt(ex * ex + ey * ey);
            if distance < nearest {
                nearest = distance;
            }
        }
        if inside { 0.0 } else { nearest }
    }
}

fn cross(a: QPoint, b: QPoint, c: QPoint) -> i128 {
    i128::from(b.x - a.x) * i128::from(c.y - a.y) - i128::from(b.y - a.y) * i128::from(c.x - a.x)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Bounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}
impl Bounds {
    fn include(&mut self, point: QPoint) {
        self.min_x = self.min_x.min(point.x_px());
        self.max_x = self.max_x.max(point.x_px());
        self.min_y = self.min_y.min(point.y_px());
        self.max_y = self.max_y.max(point.y_px());
    }
}

/// The immutable output is exposed by reference; all contours must be filled
/// together using NONZERO winding, including translucent/highlighter rendering.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Geometry {
    algorithm_version: u32,
    family: BrushFamily,
    pub(crate) polygons: Vec<Polygon>,
    pub(crate) vertex_count: usize,
    pub(crate) bounds: Option<Bounds>,
}
impl Geometry {
    pub(crate) fn empty(family: BrushFamily) -> Self {
        Self {
            algorithm_version: ALGORITHM_VERSION,
            family,
            polygons: Vec::new(),
            vertex_count: 0,
            bounds: None,
        }
    }
    pub fn algorithm_version(&self) -> u32 {
        self.algorithm_version
    }
    pub fn family(&self) -> BrushFamily {
        self.family
    }
    pub fn polygons(&self) -> &[Polygon] {
        &self.polygons
    }
    pub fn vertex_count(&self) -> usize {
        self.vertex_count
    }
    pub fn bounds(&self) -> Option<Bounds> {
        self.bounds
    }
    pub fn is_empty(&self) -> bool {
        self.polygons.is_empty()
    }
    /// Portable binary format: domain, version LE u32, family byte, contour count
    /// LE u64, then each point count LE u32 and signed LE i64 x/y pairs.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, InkError> {
        let capacity = 64usize
            .checked_add(
                self.vertex_count
                    .checked_mul(16)
                    .ok_or(InkError::ResourceLimit)?,
            )
            .and_then(|n| n.checked_add(self.polygons.len().checked_mul(4)?))
            .ok_or(InkError::ResourceLimit)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| InkError::Allocation)?;
        bytes.extend_from_slice(b"VisualWorkbench.InkGeometry.v1\0");
        bytes.extend_from_slice(&self.algorithm_version.to_le_bytes());
        bytes.push(self.family.code());
        bytes.extend_from_slice(&(self.polygons.len() as u64).to_le_bytes());
        for polygon in &self.polygons {
            bytes.extend_from_slice(&(polygon.points.len() as u32).to_le_bytes());
            for point in &polygon.points {
                bytes.extend_from_slice(&point.x.to_le_bytes());
                bytes.extend_from_slice(&point.y.to_le_bytes());
            }
        }
        Ok(bytes)
    }
    pub fn hash(&self) -> Result<AssetId, InkError> {
        Ok(AssetId::hash(&self.canonical_bytes()?))
    }
    /// Distance to the quantized filled stroke; zero inside, infinity if empty.
    pub fn distance_to(&self, x: f64, y: f64) -> Result<f64, InkError> {
        if !valid_coordinate(x) || !valid_coordinate(y) {
            return Err(InkError::Invalid("hit-test point"));
        }
        let mut nearest = f64::INFINITY;
        for polygon in &self.polygons {
            let distance = polygon.distance(x, y);
            if distance == 0.0 {
                return Ok(0.0);
            }
            if distance < nearest {
                nearest = distance;
            }
        }
        Ok(nearest)
    }
    pub fn hit_test(&self, x: f64, y: f64, tolerance: f64) -> Result<bool, InkError> {
        if !valid_coordinate(x)
            || !valid_coordinate(y)
            || !tolerance.is_finite()
            || !(0.0..=crate::MAX_COORDINATE).contains(&tolerance)
        {
            return Err(InkError::Invalid("hit-test point/tolerance"));
        }
        let Some(bounds) = self.bounds else {
            return Ok(false);
        };
        if x < bounds.min_x - tolerance
            || x > bounds.max_x + tolerance
            || y < bounds.min_y - tolerance
            || y > bounds.max_y + tolerance
        {
            return Ok(false);
        }
        Ok(self.distance_to(x, y)? <= tolerance)
    }
    pub(crate) fn extend(&mut self, polygons: Vec<Polygon>) -> Result<(), InkError> {
        let added = polygons
            .iter()
            .try_fold(0usize, |sum, polygon| sum.checked_add(polygon.points.len()))
            .ok_or(InkError::ResourceLimit)?;
        let count = self
            .vertex_count
            .checked_add(added)
            .ok_or(InkError::ResourceLimit)?;
        if count > MAX_VERTICES {
            return Err(InkError::ResourceLimit);
        }
        self.polygons
            .try_reserve(polygons.len())
            .map_err(|_| InkError::Allocation)?;
        for polygon in polygons {
            for point in &polygon.points {
                match &mut self.bounds {
                    Some(bounds) => bounds.include(*point),
                    None => {
                        self.bounds = Some(Bounds {
                            min_x: point.x_px(),
                            max_x: point.x_px(),
                            min_y: point.y_px(),
                            max_y: point.y_px(),
                        })
                    }
                }
            }
            self.polygons.push(polygon);
        }
        self.vertex_count = count;
        Ok(())
    }
}
