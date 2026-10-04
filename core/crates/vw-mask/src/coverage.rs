use crate::{
    Builder, MAX_PATH_POINTS, MAX_RADIUS, MAX_WORK, Mask, MaskError, Point, Rect, Result, Size,
    filled, quantize,
};

const X_UNITS: i64 = 65_536;
const Y_SAMPLES: i64 = 256;
const PIXEL_AREA: u64 = X_UNITS as u64 * Y_SAMPLES as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Q {
    x: i64,
    y: i64,
}
impl Q {
    fn point(p: Point) -> Result<Self> {
        Ok(Self {
            x: quantize(p.x)?,
            y: quantize(p.y)?,
        })
    }
}

impl Mask {
    /// Exact box area after 1/256-edge quantization, rounded to nearest u8.
    /// Off-document geometry is clipped; an empty rectangle is an empty version.
    pub fn rectangle(size: Size, rectangle: Rect) -> Result<Self> {
        if !rectangle.width.is_finite()
            || !rectangle.height.is_finite()
            || rectangle.width < 0.0
            || rectangle.height < 0.0
        {
            return Err(MaskError::Invalid);
        }
        let left = quantize(rectangle.x)?.clamp(0, i64::from(size.width()) * 256);
        let right =
            quantize(rectangle.x + rectangle.width)?.clamp(0, i64::from(size.width()) * 256);
        let top = quantize(rectangle.y)?.clamp(0, i64::from(size.height()) * 256);
        let bottom =
            quantize(rectangle.y + rectangle.height)?.clamp(0, i64::from(size.height()) * 256);
        let mut output = Builder::new(size);
        let start = left / 256;
        let mut row = filled(((right + 255) / 256 - start).max(0) as usize, 0u8)?;
        for y in top / 256..(bottom + 255) / 256 {
            let dy = (bottom.min((y + 1) * 256) - top.max(y * 256)).max(0) as u64;
            for (index, value) in row.iter_mut().enumerate() {
                let x = start + index as i64;
                let dx = (right.min((x + 1) * 256) - left.max(x * 256)).max(0) as u64;
                *value = ((dx * dy * 255 + 32_768) / 65_536) as u8;
            }
            output.row(start as u32, y as u32, &row)?;
        }
        Ok(output.finish(1))
    }

    /// Closed even-odd freehand selection. A repeated closing point is optional;
    /// self intersections follow even-odd parity and are independent of winding.
    pub fn lasso(size: Size, points: &[Point]) -> Result<Self> {
        if points.len() < 3 || points.len() > MAX_PATH_POINTS {
            return Err(MaskError::Invalid);
        }
        let points = points
            .iter()
            .map(|&p| Q::point(p))
            .collect::<Result<Vec<_>>>()?;
        let mut primitives = Vec::with_capacity(points.len());
        add_polygon(&mut primitives, &points);
        rasterize(size, primitives, Fill::EvenOdd, 255)
    }

    /// Union of round, constant-radius capsules along the path. One point makes
    /// a round dab. Overlaps are filled once before opacity is applied, so input
    /// subdivision and repeated samples never darken a painted selection.
    pub fn paint(size: Size, points: &[Point], radius: f64, opacity: u8) -> Result<Self> {
        if points.is_empty()
            || points.len() > MAX_PATH_POINTS
            || !radius.is_finite()
            || radius <= 0.0
            || radius > f64::from(MAX_RADIUS)
        {
            return Err(MaskError::Invalid);
        }
        let radius = quantize(radius)?;
        if radius == 0 {
            return Err(MaskError::Invalid);
        }
        let mut points = points
            .iter()
            .map(|&p| Q::point(p))
            .collect::<Result<Vec<_>>>()?;
        points.dedup();
        let mut primitives = Vec::with_capacity(points.len() * 5);
        for &center in &points {
            primitives.push(Primitive::Circle { center, radius });
        }
        for pair in points.windows(2) {
            let a = pair[0];
            let b = pair[1];
            let dx = (b.x - a.x) as f64;
            let dy = (b.y - a.y) as f64;
            let length = libm::sqrt(dx * dx + dy * dy);
            let nx = libm::round(-dy * radius as f64 / length) as i64;
            let ny = libm::round(dx * radius as f64 / length) as i64;
            let mut corners = [
                Q {
                    x: a.x + nx,
                    y: a.y + ny,
                },
                Q {
                    x: b.x + nx,
                    y: b.y + ny,
                },
                Q {
                    x: b.x - nx,
                    y: b.y - ny,
                },
                Q {
                    x: a.x - nx,
                    y: a.y - ny,
                },
            ];
            // All capsule rectangles and circle intervals use the same winding.
            let area: i128 = corners
                .iter()
                .zip(corners.iter().cycle().skip(1))
                .take(4)
                .map(|(a, b)| i128::from(a.x) * i128::from(b.y) - i128::from(a.y) * i128::from(b.x))
                .sum();
            if area < 0 {
                corners.reverse();
            }
            add_polygon(&mut primitives, &corners);
        }
        rasterize(size, primitives, Fill::NonZero, opacity)
    }
}

#[derive(Clone, Copy)]
enum Fill {
    EvenOdd,
    NonZero,
}
#[derive(Clone, Copy)]
enum Primitive {
    Edge { low: Q, high: Q, winding: i32 },
    Circle { center: Q, radius: i64 },
}
impl Primitive {
    fn low(self) -> i64 {
        match self {
            Self::Edge { low, .. } => low.y,
            Self::Circle { center, radius } => center.y - radius,
        }
    }
    fn high(self) -> i64 {
        match self {
            Self::Edge { high, .. } => high.y,
            Self::Circle { center, radius } => center.y + radius,
        }
    }
    fn events(self, y2: i64, out: &mut Vec<(i64, i32)>) {
        match self {
            Self::Edge { low, high, winding } => {
                let numerator = i128::from(high.x - low.x) * 256 * i128::from(y2 - 2 * low.y);
                let denominator = i128::from(2 * (high.y - low.y));
                // Nearest 1/65536 x pixel; ties toward positive infinity. Bounds
                // are checked before this arithmetic and fit comfortably in i128.
                let offset = (numerator + denominator / 2).div_euclid(denominator);
                out.push((low.x * 256 + offset as i64, winding));
            }
            Self::Circle { center, radius } => {
                let dy = y2 - 2 * center.y;
                let square = 4 * radius * radius - dy * dy;
                if square > 0 {
                    let offset = libm::round(libm::sqrt(square as f64) * 128.0) as i64;
                    out.push((center.x * 256 - offset, -1));
                    out.push((center.x * 256 + offset, 1));
                }
            }
        }
    }
}
fn add_polygon(primitives: &mut Vec<Primitive>, points: &[Q]) {
    for (&a, &b) in points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
    {
        if a.y < b.y {
            primitives.push(Primitive::Edge {
                low: a,
                high: b,
                winding: 1,
            });
        } else if a.y > b.y {
            primitives.push(Primitive::Edge {
                low: b,
                high: a,
                winding: -1,
            });
        }
    }
}

/// Integrate horizontal intervals at 256 equally spaced scanline centers per
/// pixel. Interval endpoints have 16 fractional bits; arithmetic accumulation
/// and u8 rounding are integer. Whole spans use differences, not per-subpixel
/// loops. Work is bounded before every scanline and before allocating output.
fn rasterize(size: Size, mut primitives: Vec<Primitive>, fill: Fill, opacity: u8) -> Result<Mask> {
    if primitives.is_empty() || opacity == 0 {
        return Ok(Builder::new(size).finish(1));
    }
    primitives.sort_by_key(|p| p.low());
    let min = primitives
        .first()
        .map_or(0, |p| p.low())
        .div_euclid(256)
        .max(0)
        .min(i64::from(size.height()));
    let max = primitives.iter().map(|p| p.high()).max().unwrap_or(0);
    let max = (max + 255)
        .div_euclid(256)
        .max(0)
        .min(i64::from(size.height()));
    let mut area = filled(size.width() as usize, 0u64)?;
    let mut differences = filled(size.width() as usize + 1, 0i32)?;
    let mut row = filled(size.width() as usize, 0u8)?;
    let mut active = Vec::<usize>::new();
    let mut cursor = 0;
    let mut events = Vec::new();
    let mut output = Builder::new(size);
    let mut work = 0u64;
    for y in min..max {
        area.fill(0);
        differences.fill(0);
        charge(&mut work, u64::from(size.width()))?;
        for sample in 0..Y_SAMPLES {
            let y2 = 2 * (y * Y_SAMPLES + sample) + 1;
            while cursor < primitives.len() && 2 * primitives[cursor].low() <= y2 {
                if 2 * primitives[cursor].high() > y2 {
                    active.push(cursor);
                }
                cursor += 1;
            }
            active.retain(|&index| 2 * primitives[index].high() > y2);
            // Sorting is charged conservatively; a complex path returns Limit
            // atomically instead of running an unbounded scan conversion.
            let upper = active.len() as u64 * 2 + 1;
            charge(&mut work, upper * u64::from(upper.ilog2() + 2))?;
            events.clear();
            for &index in &active {
                primitives[index].events(y2, &mut events);
            }
            events.sort_unstable_by_key(|event| event.0);
            let mut winding = 0i32;
            let mut left = 0i64;
            let mut index = 0;
            while index < events.len() {
                let right = events[index].0;
                let inside = match fill {
                    Fill::EvenOdd => winding % 2 != 0,
                    Fill::NonZero => winding != 0,
                };
                if inside {
                    add_interval(left, right, &mut area, &mut differences, size.width());
                }
                while index < events.len() && events[index].0 == right {
                    winding += events[index].1;
                    index += 1;
                }
                left = right;
            }
            if winding != 0 {
                return Err(MaskError::Invalid);
            }
        }
        let mut full = 0i32;
        for (x, &partial) in area.iter().enumerate() {
            full += differences[x];
            if !(0..=256).contains(&full) {
                return Err(MaskError::Invalid);
            }
            let integrated = partial + full as u64 * X_UNITS as u64;
            if integrated > PIXEL_AREA {
                return Err(MaskError::Invalid);
            }
            // Apply paint opacity after union/coverage; there is only one rounding.
            row[x] = ((integrated * u64::from(opacity) + PIXEL_AREA / 2) / PIXEL_AREA) as u8;
        }
        output.row(0, y as u32, &row)?;
    }
    Ok(output.finish(1))
}
fn charge(work: &mut u64, amount: u64) -> Result<()> {
    *work = work.checked_add(amount).ok_or(MaskError::Limit)?;
    if *work > MAX_WORK {
        Err(MaskError::Limit)
    } else {
        Ok(())
    }
}
fn add_interval(left: i64, right: i64, area: &mut [u64], differences: &mut [i32], width: u32) {
    let left = left.clamp(0, i64::from(width) * X_UNITS);
    let right = right.clamp(0, i64::from(width) * X_UNITS);
    if left >= right {
        return;
    }
    let first = (left / X_UNITS) as usize;
    let last = ((right - 1) / X_UNITS) as usize;
    if first == last {
        area[first] += (right - left) as u64;
        return;
    }
    area[first] += ((first as i64 + 1) * X_UNITS - left) as u64;
    area[last] += (right - last as i64 * X_UNITS) as u64;
    if first + 1 < last {
        differences[first + 1] += 1;
        differences[last] -= 1;
    }
}
