//! Materialized vector erasing of canonical convex ink contours.
//!
//! The original filtered stroke is replayed once by the caller. We subtract a
//! bounded convex polygon sweep from its quantized contours; surviving samples
//! are never re-filtered. The result is a single compound NONZERO fill, encoded
//! in the existing Polygon model with exactly cancelling bridge edges. Keeping
//! all pieces in one fill preserves translucent/highlighter overlap semantics.
//! A subsequent erase decodes that same explicit contour representation.
use crate::{InkError, MAX_COORDINATE, Polygon, QPoint, QUANTIZATION};
use vw_proto::v1;

pub const MAX_ERASER_POINTS: usize = 4_096;
pub const MAX_ERASER_RADIUS: f64 = 256.0;
pub const MAX_ERASER_VERTICES: usize = 262_144;
pub const MAX_ERASER_WORK: u64 = 64_000_000;
const Q_LIMIT: i64 = ((MAX_COORDINATE + crate::MAX_BASE_WIDTH) * QUANTIZATION) as i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EraseError {
    #[error("invalid vector eraser input")]
    Invalid,
    #[error("vector eraser complexity limit exceeded")]
    Limit,
    #[error("vector eraser cancelled")]
    Cancelled,
    #[error("vector eraser allocation failed")]
    Allocation,
}
impl From<InkError> for EraseError {
    fn from(value: InkError) -> Self {
        match value {
            InkError::ResourceLimit => Self::Limit,
            InkError::Allocation => Self::Allocation,
            _ => Self::Invalid,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EraseLimits {
    /// Includes untouched contours and bridge vertices, not just intersections.
    pub max_vertices: usize,
    pub max_work: u64,
}
impl Default for EraseLimits {
    fn default() -> Self {
        Self {
            max_vertices: MAX_ERASER_VERTICES,
            max_work: MAX_ERASER_WORK,
        }
    }
}
impl EraseLimits {
    fn validate(self) -> Result<(), EraseError> {
        if !(3..=MAX_ERASER_VERTICES).contains(&self.max_vertices)
            || !(1..=MAX_ERASER_WORK).contains(&self.max_work)
        {
            return Err(EraseError::Invalid);
        }
        Ok(())
    }
    /// Additional construction workspace. Caller separately charges retained
    /// raw source/canonical state and any newly constructed transaction clones.
    pub fn workspace_bytes(self) -> Result<u64, EraseError> {
        self.validate()?;
        // Source/result, two clipping halves, hull scratch, contour Vec headers,
        // bridge protobuf points, sorting scratch and allocation slack.
        Ok(self.max_vertices as u64 * 512 + 4 * 1024 * 1024)
    }
}

#[derive(Debug, Clone)]
pub struct ErasedOutline {
    pub changed: bool,
    pub contours: Vec<Polygon>,
    pub work_units: u64,
}

struct Work<'a> {
    used: u64,
    limits: EraseLimits,
    cancelled: &'a dyn Fn() -> bool,
}
impl Work<'_> {
    fn add(&mut self, count: u64) -> Result<(), EraseError> {
        if (self.cancelled)() {
            return Err(EraseError::Cancelled);
        }
        self.used = self.used.checked_add(count).ok_or(EraseError::Limit)?;
        if self.used > self.limits.max_work {
            return Err(EraseError::Limit);
        }
        Ok(())
    }
    fn copy(&mut self, points: &[QPoint]) -> Result<Vec<QPoint>, EraseError> {
        self.add(points.len() as u64)?;
        if points.len() > self.limits.max_vertices {
            return Err(EraseError::Limit);
        }
        let mut output = Vec::new();
        output
            .try_reserve_exact(points.len())
            .map_err(|_| EraseError::Allocation)?;
        output.extend_from_slice(points);
        Ok(output)
    }
}

/// `local_from_document` is the existing object's checked inverse affine,
/// [a,b,c,d,e,f]. Eraser centers/radius are D-space; transformed capsules are
/// clipped against local quantized ink. The object's transform remains exact.
/// Rotations, reflections and nonuniform scales therefore use an ellipse in
/// local space, rather than incorrectly scaling a scalar brush radius.
pub fn erase_contours(
    source: &[Polygon],
    centers: &[v1::PointD],
    radius: f64,
    local_from_document: [f64; 6],
    limits: EraseLimits,
    cancelled: &dyn Fn() -> bool,
) -> Result<ErasedOutline, EraseError> {
    limits.validate()?;
    if centers.is_empty()
        || centers.len() > MAX_ERASER_POINTS
        || !radius.is_finite()
        || !(0.5..=MAX_ERASER_RADIUS).contains(&radius)
        || local_from_document.iter().any(|v| !v.is_finite())
        || centers
            .iter()
            .any(|p| !crate::valid_coordinate(p.x) || !crate::valid_coordinate(p.y))
    {
        return Err(EraseError::Invalid);
    }
    let [a, b, c, d, _, _] = local_from_document;
    let determinant = a * d - b * c;
    if !determinant.is_finite() || determinant == 0.0 {
        return Err(EraseError::Invalid);
    }
    let mut work = Work {
        used: 0,
        limits,
        cancelled,
    };
    let count = vertex_count(source, &mut work)?;
    if count > limits.max_vertices {
        return Err(EraseError::Limit);
    }
    let mut pieces = Vec::new();
    pieces
        .try_reserve_exact(source.len())
        .map_err(|_| EraseError::Allocation)?;
    for polygon in source {
        validate_contour(polygon, &mut work)?;
        pieces.push(Polygon {
            points: work.copy(&polygon.points)?,
        });
    }
    let mut changed = false;
    let segments = centers.len().saturating_sub(1).max(1);
    for index in 0..segments {
        work.add(1)?;
        if pieces.is_empty() {
            break;
        }
        let start = centers[index];
        let end = centers[(index + 1).min(centers.len() - 1)];
        let clip = capsule(start, end, radius, local_from_document, &mut work)?;
        let clip_bounds = bounds(&clip.points)?;
        let mut next = Vec::new();
        let mut vertices = 0usize;
        for piece in pieces {
            work.add(piece.points.len() as u64 + 1)?;
            let removed = if separated(bounds(&piece.points)?, clip_bounds) {
                None
            } else {
                subtract(&piece, &clip, &mut work)?
            };
            if let Some(parts) = removed {
                changed = true;
                for part in parts {
                    append_piece(&mut next, part, &mut vertices, limits)?;
                }
            } else {
                append_piece(&mut next, piece, &mut vertices, limits)?;
            }
        }
        pieces = next;
    }
    Ok(ErasedOutline {
        changed,
        contours: pieces,
        work_units: work.used,
    })
}

fn vertex_count(polygons: &[Polygon], work: &mut Work<'_>) -> Result<usize, EraseError> {
    // A valid contour has at least three vertices. Reject header-only hostile
    // slices before scanning or reserving one allocation per supplied contour.
    if polygons.len() > work.limits.max_vertices / 3 {
        return Err(EraseError::Limit);
    }
    let mut count = 0usize;
    for polygon in polygons {
        work.add(1)?;
        if polygon.points.len() < 3 {
            return Err(EraseError::Invalid);
        }
        count = count
            .checked_add(polygon.points.len())
            .ok_or(EraseError::Limit)?;
        if count > work.limits.max_vertices {
            return Err(EraseError::Limit);
        }
    }
    Ok(count)
}

/// Allocation admission only; geometry_from_stroke still validates every raw
/// sample. This upper bound follows the existing v1 circle/segment/join limits
/// and never allocates replay samples or geometry merely to estimate them.
pub fn stroke_vertex_bound(stroke: &v1::Stroke) -> Result<usize, EraseError> {
    let count = stroke.x.len();
    if count == 0 || count > crate::MAX_SAMPLES {
        return Err(EraseError::Limit);
    }
    if [stroke.y.len(), stroke.t_ms.len(), stroke.pressure.len()]
        .iter()
        .any(|n| *n != count)
        || (!stroke.tilt.is_empty() && stroke.tilt.len() != count)
        || (!stroke.orientation.is_empty() && stroke.orientation.len() != count)
    {
        return Err(EraseError::Invalid);
    }
    let brush = crate::Brush::try_from(stroke.brush.as_ref().ok_or(EraseError::Invalid)?)?;
    if brush.family == crate::BrushFamily::VectorEraser {
        return Err(EraseError::Invalid);
    }
    let radius = brush.base_width * 0.5;
    let angle = libm::acos(1.0 - (0.125 / radius).min(1.0));
    let circle = ((libm::ceil(std::f64::consts::PI / angle) as usize)
        .max(12)
        .div_ceil(4)
        * 4)
    .min(512);
    let per_sample = if brush.family == crate::BrushFamily::Highlighter {
        (libm::ceil(std::f64::consts::PI / (2.0 * angle)) as usize).clamp(1, 512) + 6
    } else {
        circle + 4
    };
    Ok(count
        .checked_mul(per_sample)
        .ok_or(EraseError::Limit)?
        .min(crate::MAX_VERTICES))
}
fn append_piece(
    output: &mut Vec<Polygon>,
    polygon: Polygon,
    vertices: &mut usize,
    limits: EraseLimits,
) -> Result<(), EraseError> {
    *vertices = vertices
        .checked_add(polygon.points.len())
        .ok_or(EraseError::Limit)?;
    // Three bridge/end vertices per contour conservatively include the final
    // model representation in the same cap, preventing late bridge expansion.
    if vertices
        .checked_add((output.len() + 1).checked_mul(3).ok_or(EraseError::Limit)?)
        .is_none_or(|n| n > limits.max_vertices)
    {
        return Err(EraseError::Limit);
    }
    output.try_reserve(1).map_err(|_| EraseError::Allocation)?;
    output.push(polygon);
    Ok(())
}
fn cross(a: QPoint, b: QPoint, p: QPoint) -> i128 {
    i128::from(b.x - a.x) * i128::from(p.y - a.y) - i128::from(b.y - a.y) * i128::from(p.x - a.x)
}
fn valid_point(point: QPoint) -> bool {
    (-Q_LIMIT..=Q_LIMIT).contains(&point.x) && (-Q_LIMIT..=Q_LIMIT).contains(&point.y)
}
fn validate_contour(polygon: &Polygon, work: &mut Work<'_>) -> Result<(), EraseError> {
    let points = &polygon.points;
    if points.len() < 3
        || points.len() > work.limits.max_vertices
        || points.iter().any(|p| !valid_point(*p))
    {
        return Err(EraseError::Invalid);
    }
    work.add(points.len() as u64 * 64)?;
    let hull = Polygon::from_points(work.copy(points)?)?.ok_or(EraseError::Invalid)?;
    if hull.points.len() != points.len() {
        return Err(EraseError::Invalid);
    }
    let offset = hull
        .points
        .iter()
        .position(|p| *p == points[0])
        .ok_or(EraseError::Invalid)?;
    if points
        .iter()
        .enumerate()
        .any(|(i, p)| *p != hull.points[(offset + i) % points.len()])
    {
        return Err(EraseError::Invalid);
    }
    Ok(())
}
fn bounds(points: &[QPoint]) -> Result<[i64; 4], EraseError> {
    let first = points.first().ok_or(EraseError::Invalid)?;
    let mut b = [first.x, first.y, first.x, first.y];
    for p in points {
        b[0] = b[0].min(p.x);
        b[1] = b[1].min(p.y);
        b[2] = b[2].max(p.x);
        b[3] = b[3].max(p.y);
    }
    Ok(b)
}
fn separated(a: [i64; 4], b: [i64; 4]) -> bool {
    a[2] <= b[0] || b[2] <= a[0] || a[3] <= b[1] || b[3] <= a[1]
}

fn capsule(
    start: v1::PointD,
    end: v1::PointD,
    radius: f64,
    map: [f64; 6],
    work: &mut Work<'_>,
) -> Result<Polygon, EraseError> {
    // Circumscribed circular approximation: maximum radial chord excess is
    // below 1/256 D pixel before the ordinary local-grid quantization.
    let angle = libm::acos(radius / (radius + 1.0 / QUANTIZATION));
    let sides = (libm::ceil(std::f64::consts::PI / angle) as usize)
        .max(12)
        .div_ceil(4)
        * 4;
    if sides > 1024 {
        return Err(EraseError::Limit);
    }
    work.add(sides as u64 * 128)?;
    let outer = radius / libm::cos(std::f64::consts::PI / sides as f64);
    let mut points = Vec::new();
    points
        .try_reserve_exact(sides * 2)
        .map_err(|_| EraseError::Allocation)?;
    let [a, b, c, d, e, f] = map;
    for center in [start, end] {
        for index in 0..sides {
            let (sin, cos) = libm::sincos(2.0 * std::f64::consts::PI * index as f64 / sides as f64);
            let x = center.x + outer * cos;
            let y = center.y + outer * sin;
            points.push(QPoint::quantize(a * x + c * y + e, b * x + d * y + f)?);
        }
    }
    Polygon::from_points(points)?.ok_or(EraseError::Invalid)
}

/// Partition P along successive half-planes of C. Outside pieces are retained
/// disjointly; the final inside polygon is discarded. If intersection has no
/// area, preserve the original byte-for-byte rather than unnecessary splits.
fn subtract(
    polygon: &Polygon,
    clip: &Polygon,
    work: &mut Work<'_>,
) -> Result<Option<Vec<Polygon>>, EraseError> {
    let mut inside = Polygon {
        points: work.copy(&polygon.points)?,
    };
    let mut outside = Vec::new();
    let mut vertices = 0usize;
    for edge in 0..clip.points.len() {
        let a = clip.points[edge];
        let b = clip.points[(edge + 1) % clip.points.len()];
        let left = half(&inside, a, b, true, work)?;
        let Some(next) = left else {
            return Ok(None);
        };
        if let Some(part) = half(&inside, a, b, false, work)? {
            append_piece(&mut outside, part, &mut vertices, work.limits)?;
        }
        inside = next;
    }
    Ok(Some(outside))
}
fn half(
    polygon: &Polygon,
    a: QPoint,
    b: QPoint,
    left: bool,
    work: &mut Work<'_>,
) -> Result<Option<Polygon>, EraseError> {
    work.add(polygon.points.len() as u64 * 66)?;
    let mut points = Vec::new();
    points
        .try_reserve_exact(polygon.points.len() + 2)
        .map_err(|_| EraseError::Allocation)?;
    for index in 0..polygon.points.len() {
        let p = polygon.points[index];
        let q = polygon.points[(index + 1) % polygon.points.len()];
        let dp = cross(a, b, p);
        let dq = cross(a, b, q);
        let pin = if left { dp >= 0 } else { dp <= 0 };
        let qin = if left { dq >= 0 } else { dq <= 0 };
        if pin {
            points.push(p);
        }
        if pin != qin {
            points.push(intersection(p, q, dp, dq)?);
        }
    }
    Ok(Polygon::from_points(points)?)
}
fn intersection(p: QPoint, q: QPoint, dp: i128, dq: i128) -> Result<QPoint, EraseError> {
    let denominator = dp.checked_sub(dq).ok_or(EraseError::Limit)?;
    let coordinate = |p: i64, q: i64| -> Result<i64, EraseError> {
        let numerator = i128::from(q)
            .checked_mul(dp)
            .and_then(|a| i128::from(p).checked_mul(dq).and_then(|b| a.checked_sub(b)))
            .ok_or(EraseError::Limit)?;
        rounded_ratio(numerator, denominator)
    };
    let value = QPoint {
        x: coordinate(p.x, q.x)?,
        y: coordinate(p.y, q.y)?,
    };
    if !valid_point(value) {
        return Err(EraseError::Invalid);
    }
    Ok(value)
}
fn rounded_ratio(numerator: i128, denominator: i128) -> Result<i64, EraseError> {
    if denominator == 0 {
        return Err(EraseError::Invalid);
    }
    let (n, d) = if denominator < 0 {
        (
            numerator.checked_neg().ok_or(EraseError::Limit)?,
            denominator.checked_neg().ok_or(EraseError::Limit)?,
        )
    } else {
        (numerator, denominator)
    };
    let quotient = n / d;
    let remainder = n % d;
    let round = remainder
        .unsigned_abs()
        .checked_mul(2)
        .ok_or(EraseError::Limit)?
        >= d as u128;
    let result = if round {
        quotient
            .checked_add(if n < 0 { -1 } else { 1 })
            .ok_or(EraseError::Limit)?
    } else {
        quotient
    };
    i64::try_from(result).map_err(|_| EraseError::Limit)
}

/// Exact inverse-edge bridges encode multiple same-winding contours as one
/// existing closed Polygon. The caller MUST use fill only (stroke width zero).
pub fn compound_polygon(
    contours: &[Polygon],
    limits: EraseLimits,
    cancelled: &dyn Fn() -> bool,
) -> Result<v1::Polyline, EraseError> {
    limits.validate()?;
    if contours.is_empty() {
        return Err(EraseError::Invalid);
    }
    let mut work = Work {
        used: 0,
        limits,
        cancelled,
    };
    let count = vertex_count(contours, &mut work)?
        .checked_add(contours.len().checked_mul(2).ok_or(EraseError::Limit)?)
        .ok_or(EraseError::Limit)?;
    if count > limits.max_vertices {
        return Err(EraseError::Limit);
    }
    let mut points = Vec::new();
    points
        .try_reserve_exact(count)
        .map_err(|_| EraseError::Allocation)?;
    let anchor = *contours[0].points.first().ok_or(EraseError::Invalid)?;
    for (index, contour) in contours.iter().enumerate() {
        validate_contour(contour, &mut work)?;
        for point in &contour.points {
            points.push(v1::PointD {
                x: point.x_px(),
                y: point.y_px(),
            });
        }
        let first = contour.points[0];
        points.push(v1::PointD {
            x: first.x_px(),
            y: first.y_px(),
        });
        if index > 0 {
            points.push(v1::PointD {
                x: anchor.x_px(),
                y: anchor.y_px(),
            });
        }
    }
    Ok(v1::Polyline {
        points,
        closed: true,
    })
}

/// Decode only a canonical convex contour or the exact bridge representation
/// above. Arbitrary self-intersecting/unquantized user polygons are refused.
pub fn contours_from_compound(
    value: &v1::Polyline,
    limits: EraseLimits,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Polygon>, EraseError> {
    limits.validate()?;
    if !value.closed || value.points.len() < 3 || value.points.len() > limits.max_vertices {
        return Err(EraseError::Invalid);
    }
    let mut work = Work {
        used: 0,
        limits,
        cancelled,
    };
    let mut points = Vec::new();
    points
        .try_reserve_exact(value.points.len())
        .map_err(|_| EraseError::Allocation)?;
    for p in &value.points {
        work.add(1)?;
        let q = QPoint::quantize(p.x, p.y)?;
        if q.x_px() != p.x || q.y_px() != p.y {
            return Err(EraseError::Invalid);
        }
        points.push(q);
    }
    let anchor = points[0];
    let mut result = Vec::new();
    let mut start = 0usize;
    loop {
        let first = points[start];
        let close = (start + 1..points.len()).find(|index| points[*index] == first);
        let end = match close {
            Some(end) => end,
            None if start == 0 => points.len(),
            None => return Err(EraseError::Invalid),
        };
        let polygon = Polygon {
            points: work.copy(&points[start..end])?,
        };
        validate_contour(&polygon, &mut work)?;
        result.try_reserve(1).map_err(|_| EraseError::Allocation)?;
        result.push(polygon);
        if end == points.len() || end + 1 == points.len() {
            break;
        }
        if start == 0 {
            start = end + 1;
        } else {
            if points[end + 1] != anchor {
                return Err(EraseError::Invalid);
            }
            start = end + 2;
            if start == points.len() {
                break;
            }
        }
    }
    Ok(result)
}
