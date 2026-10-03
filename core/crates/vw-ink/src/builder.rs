use crate::{
    Brush, BrushFamily, Geometry, InkError, MAX_SAMPLES, MAX_VERTICES, Polygon, QPoint, Sample,
};
use serde::Serialize;
use vw_proto::v1;

/// Appended contour range in `StrokeBuilder::geometry().polygons()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct GeometryRange {
    pub start: usize,
    pub end: usize,
}
impl GeometryRange {
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrokeState {
    Active,
    Finished,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Node {
    x: f64,
    y: f64,
    radius: f64,
}
#[derive(Debug, Clone, Copy)]
struct SegmentEnd {
    ux: f64,
    uy: f64,
    radius: f64,
}
#[derive(Debug, Clone, Copy, Default)]
struct FilterState {
    raw: Option<Sample>,
    node: Option<Node>,
    pressure: f64,
    vx: f64,
    vy: f64,
    segment_end: Option<SegmentEnd>,
}

/// One bounded stroke. Clones are independent and may be used for disposable
/// motion prediction; they copy contours, so prediction callers must bound size.
#[derive(Debug, Clone)]
pub struct StrokeBuilder {
    brush: Brush,
    geometry: Geometry,
    filter: FilterState,
    sample_count: usize,
    state: StrokeState,
}
impl StrokeBuilder {
    pub fn begin(brush: Brush) -> Result<Self, InkError> {
        brush.validate()?;
        Ok(Self {
            geometry: Geometry::empty(brush.family),
            brush,
            filter: FilterState::default(),
            sample_count: 0,
            state: StrokeState::Active,
        })
    }
    pub fn geometry(&self) -> &Geometry {
        &self.geometry
    }
    pub fn state(&self) -> StrokeState {
        self.state
    }
    pub fn sample_count(&self) -> usize {
        self.sample_count
    }
    pub fn brush(&self) -> &Brush {
        &self.brush
    }
    /// Validation and geometry construction are atomic for the entire batch.
    /// Optional tilt/orientation are accepted/validated but circular v1 nibs do
    /// not use them. Pressure is rounded to the protobuf's f32 sample precision
    /// before f64 modeling, so replay from stored model arrays stays identical.
    pub fn append(&mut self, samples: &[Sample]) -> Result<GeometryRange, InkError> {
        if self.state != StrokeState::Active {
            return Err(InkError::Closed);
        }
        let count = self
            .sample_count
            .checked_add(samples.len())
            .ok_or(InkError::ResourceLimit)?;
        if count > MAX_SAMPLES {
            return Err(InkError::ResourceLimit);
        }
        let mut previous_time = self.filter.raw.map(|sample| sample.t_ms);
        for sample in samples {
            sample.validate()?;
            if previous_time.is_some_and(|time| sample.t_ms < time) {
                return Err(InkError::TimeOrder);
            }
            previous_time = Some(sample.t_ms);
        }
        let start = self.geometry.polygons().len();
        let mut filter = self.filter;
        let mut changed = Vec::new();
        let mut vertices = self.geometry.vertex_count();
        for sample in samples {
            let canonical = Sample {
                pressure: f64::from(sample.pressure as f32),
                tilt: sample.tilt.map(|value| f64::from(value as f32)),
                orientation: sample.orientation.map(|value| f64::from(value as f32)),
                ..*sample
            };
            if filter.raw.is_some_and(|old| {
                old.x == canonical.x
                    && old.y == canonical.y
                    && old.t_ms == canonical.t_ms
                    && old.pressure == canonical.pressure
            }) {
                continue;
            }
            let previous = filter.node;
            let node = filter_sample(&mut filter, canonical, &self.brush);
            let moving = previous.is_some_and(|old| old.x != node.x || old.y != node.y);
            if self.brush.family == BrushFamily::Highlighter {
                if let Some(old) = previous
                    && moving
                {
                    let before = changed.len();
                    emit_segment(old, node, false, &mut changed, &mut vertices)?;
                    if changed.len() > before {
                        if let Some(incoming) = filter.segment_end {
                            emit_highlighter_join(
                                incoming,
                                old,
                                node,
                                &mut changed,
                                &mut vertices,
                            )?;
                        }
                        let dx = node.x - old.x;
                        let dy = node.y - old.y;
                        let distance = libm::sqrt(dx * dx + dy * dy);
                        filter.segment_end = Some(SegmentEnd {
                            ux: dx / distance,
                            uy: dy / distance,
                            radius: node.radius,
                        });
                    }
                }
            } else {
                if let Some(old) = previous
                    && moving
                {
                    emit_segment(old, node, true, &mut changed, &mut vertices)?;
                }
                if previous.is_none_or(|old| moving || node.radius > old.radius) {
                    emit_circle(node, &mut changed, &mut vertices)?;
                }
            }
            filter.node = Some(node);
            filter.raw = Some(canonical);
        }
        self.geometry.extend(changed)?;
        self.filter = filter;
        self.sample_count = count;
        Ok(GeometryRange {
            start,
            end: self.geometry.polygons().len(),
        })
    }
    /// Close without adding samples, smoothing a tail or changing any vertex.
    /// The builder retains its final geometry for JNI/live readers.
    pub fn finish(&mut self) -> Result<Geometry, InkError> {
        if self.state != StrokeState::Active {
            return Err(InkError::Closed);
        }
        if self.sample_count == 0 {
            return Err(InkError::EmptyStroke);
        }
        self.state = StrokeState::Finished;
        Ok(self.geometry.clone())
    }
    /// Consume a live/finished stroke without cloning its potentially large mesh.
    pub fn into_geometry(self) -> Result<Geometry, InkError> {
        if self.state == StrokeState::Cancelled {
            return Err(InkError::Closed);
        }
        if self.sample_count == 0 {
            return Err(InkError::EmptyStroke);
        }
        Ok(self.geometry)
    }
    /// Erase all provisional contours. The owner removes this stroke's live path.
    pub fn cancel(&mut self) -> Result<(), InkError> {
        if self.state != StrokeState::Active {
            return Err(InkError::Closed);
        }
        self.geometry = Geometry::empty(self.brush.family);
        self.filter = FilterState::default();
        self.sample_count = 0;
        self.state = StrokeState::Cancelled;
        Ok(())
    }
}

fn filter_sample(state: &mut FilterState, sample: Sample, brush: &Brush) -> Node {
    let target_pressure = f64::from(sample.pressure as f32);
    let stabilization = f64::from(brush.stabilization);
    let (mut x, mut y, mut pressure) = (sample.x, sample.y, target_pressure);
    if let (Some(raw), Some(previous)) = (state.raw, state.node)
        && stabilization > 0.0
    {
        // Equal timestamps are legal historical samples: use a fixed 240-Hz
        // interval, never wall time or append-batch cadence.
        let dt = if sample.t_ms == raw.t_ms {
            1.0 / 240.0
        } else {
            f64::from(sample.t_ms - raw.t_ms) / 1_000.0
        };
        let derivative_alpha = dt / (dt + 1.0 / (2.0 * std::f64::consts::PI));
        let vx = (sample.x - raw.x) / dt;
        let vy = (sample.y - raw.y) / dt;
        state.vx += derivative_alpha * (vx - state.vx);
        state.vy += derivative_alpha * (vy - state.vy);
        let speed = libm::sqrt(state.vx * state.vx + state.vy * state.vy);
        let cutoff = 2.0 + 28.0 * (1.0 - stabilization) + 0.005 * (1.0 - stabilization) * speed;
        let alpha = (1.0 - stabilization)
            + stabilization * dt / (dt + 1.0 / (2.0 * std::f64::consts::PI * cutoff));
        x = previous.x + alpha * (sample.x - previous.x);
        y = previous.y + alpha * (sample.y - previous.y);
        pressure = state.pressure + alpha * (target_pressure - state.pressure);
    }
    state.pressure = pressure;
    Node {
        x,
        y,
        radius: 0.5 * brush.base_width * brush.pressure_curve.map(pressure),
    }
}

fn emit_circle(
    node: Node,
    polygons: &mut Vec<Polygon>,
    vertices: &mut usize,
) -> Result<(), InkError> {
    if node.radius <= 0.0 {
        return Ok(());
    }
    // Sagitta at most 1/8 D-pixel before quantization. Multiples of four retain
    // exact axis extrema; the width cap limits the count to at most 512.
    let angle = libm::acos(1.0 - (0.125 / node.radius).min(1.0));
    let estimate = libm::ceil(std::f64::consts::PI / angle) as usize;
    let count = (estimate.max(12).div_ceil(4) * 4).min(512);
    let mut points = Vec::new();
    points
        .try_reserve_exact(count)
        .map_err(|_| InkError::Allocation)?;
    for index in 0..count {
        let (sine, cosine) = if index == 0 {
            (0.0, 1.0)
        } else if index == count / 4 {
            (1.0, 0.0)
        } else if index == count / 2 {
            (0.0, -1.0)
        } else if index == count * 3 / 4 {
            (-1.0, 0.0)
        } else {
            libm::sincos(2.0 * std::f64::consts::PI * index as f64 / count as f64)
        };
        points.push(QPoint::quantize(
            node.x + node.radius * cosine,
            node.y + node.radius * sine,
        )?);
    }
    emit(points, polygons, vertices)
}

/// Fill only the outside wedge between two butt-ended segments. A full disk at
/// an interior sample extends beyond the caps of a short, densely sampled line.
/// The incoming endpoint radius is retained separately because stationary
/// pressure changes can alter the outgoing width without replacing old geometry.
fn emit_highlighter_join(
    incoming: SegmentEnd,
    center: Node,
    next: Node,
    polygons: &mut Vec<Polygon>,
    vertices: &mut usize,
) -> Result<(), InkError> {
    let dx = next.x - center.x;
    let dy = next.y - center.y;
    let distance = libm::sqrt(dx * dx + dy * dy);
    if distance == 0.0 {
        return Ok(());
    }
    let (ux, uy) = (dx / distance, dy / distance);
    let cross = incoming.ux * uy - incoming.uy * ux;
    let dot = incoming.ux * ux + incoming.uy * uy;
    let turn = if cross == 0.0 {
        if dot >= 0.0 {
            return Ok(());
        }
        std::f64::consts::PI
    } else {
        libm::atan2(cross, dot)
    };
    let radius = incoming.radius.max(center.radius);
    if radius <= 0.0 {
        return Ok(());
    }
    let side = if turn > 0.0 { 1.0 } else { -1.0 };
    let start = (side * incoming.uy, -side * incoming.ux);
    let end = (side * uy, -side * ux);
    let max_angle = 2.0 * libm::acos(1.0 - (0.125 / radius).min(1.0));
    let count = (libm::ceil(turn.abs() / max_angle) as usize).clamp(1, 512);
    let mut points = Vec::new();
    points
        .try_reserve_exact(count + 2)
        .map_err(|_| InkError::Allocation)?;
    points.push(QPoint::quantize(center.x, center.y)?);
    for index in 0..=count {
        let t = index as f64 / count as f64;
        let (nx, ny) = if index == 0 {
            start
        } else if index == count {
            end
        } else {
            let (sine, cosine) = libm::sincos(turn * t);
            (
                start.0 * cosine - start.1 * sine,
                start.0 * sine + start.1 * cosine,
            )
        };
        let width = incoming.radius + t * (center.radius - incoming.radius);
        points.push(QPoint::quantize(
            center.x + nx * width,
            center.y + ny * width,
        )?);
    }
    emit(points, polygons, vertices)
}

fn emit_segment(
    a: Node,
    b: Node,
    round: bool,
    polygons: &mut Vec<Polygon>,
    vertices: &mut usize,
) -> Result<(), InkError> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let distance = libm::sqrt(dx * dx + dy * dy);
    if distance == 0.0 || (a.radius == 0.0 && b.radius == 0.0) {
        return Ok(());
    }
    let (ux, uy) = (dx / distance, dy / distance);
    // Round-brush side walls are common external circle tangents. When one
    // circle contains the other, their disks already cover the entire sweep.
    let along = if round {
        (a.radius - b.radius) / distance
    } else {
        0.0
    };
    if round && !(-1.0..1.0).contains(&along) {
        return Ok(());
    }
    let across = libm::sqrt((1.0 - along * along).max(0.0));
    let (lx, ly) = (ux * along - uy * across, uy * along + ux * across);
    let (rx, ry) = (ux * along + uy * across, uy * along - ux * across);
    let mut points = Vec::new();
    points
        .try_reserve_exact(4)
        .map_err(|_| InkError::Allocation)?;
    points.push(QPoint::quantize(a.x + lx * a.radius, a.y + ly * a.radius)?);
    points.push(QPoint::quantize(a.x + rx * a.radius, a.y + ry * a.radius)?);
    points.push(QPoint::quantize(b.x + rx * b.radius, b.y + ry * b.radius)?);
    points.push(QPoint::quantize(b.x + lx * b.radius, b.y + ly * b.radius)?);
    emit(points, polygons, vertices)
}
fn emit(
    points: Vec<QPoint>,
    polygons: &mut Vec<Polygon>,
    vertices: &mut usize,
) -> Result<(), InkError> {
    if let Some(polygon) = Polygon::from_points(points)? {
        *vertices = vertices
            .checked_add(polygon.points.len())
            .ok_or(InkError::ResourceLimit)?;
        if *vertices > MAX_VERTICES {
            return Err(InkError::ResourceLimit);
        }
        polygons.try_reserve(1).map_err(|_| InkError::Allocation)?;
        polygons.push(polygon);
    }
    Ok(())
}

/// Map the canonical model's parallel arrays without dropping optional axes.
pub fn samples_from_stroke(stroke: &v1::Stroke) -> Result<Vec<Sample>, InkError> {
    let count = stroke.x.len();
    if count == 0 {
        return Err(InkError::EmptyStroke);
    }
    if count > MAX_SAMPLES {
        return Err(InkError::ResourceLimit);
    }
    if [stroke.y.len(), stroke.t_ms.len(), stroke.pressure.len()]
        .iter()
        .any(|n| *n != count)
        || (!stroke.tilt.is_empty() && stroke.tilt.len() != count)
        || (!stroke.orientation.is_empty() && stroke.orientation.len() != count)
    {
        return Err(InkError::Invalid("sample array lengths"));
    }
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(count)
        .map_err(|_| InkError::Allocation)?;
    for index in 0..count {
        let sample = Sample {
            x: stroke.x[index],
            y: stroke.y[index],
            t_ms: stroke.t_ms[index],
            pressure: f64::from(stroke.pressure[index]),
            tilt: stroke.tilt.get(index).copied().map(f64::from),
            orientation: stroke.orientation.get(index).copied().map(f64::from),
        };
        sample.validate()?;
        if samples
            .last()
            .is_some_and(|old: &Sample| old.t_ms > sample.t_ms)
        {
            return Err(InkError::TimeOrder);
        }
        samples.push(sample);
    }
    Ok(samples)
}
pub fn geometry_from_stroke(stroke: &v1::Stroke) -> Result<Geometry, InkError> {
    let brush = Brush::try_from(
        stroke
            .brush
            .as_ref()
            .ok_or(InkError::Invalid("missing brush"))?,
    )?;
    let samples = samples_from_stroke(stroke)?;
    let mut builder = StrokeBuilder::begin(brush)?;
    builder.append(&samples)?;
    builder.into_geometry()
}
