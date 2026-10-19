use crate::{Affine, GeometryError, Point, finite};

/// Rotation in document space about `pivot`, measured from the immutable
/// pen-down vector to the current real pointer vector. Positive angles are
/// clockwise in the document's y-down coordinates. Snapping rounds this gesture
/// delta to the nearest 15 degrees, with half steps away from zero.
///
/// The vectors are normalized before cross/dot products to avoid overflow.
/// A pointer at the pivot, non-finite arithmetic or an unrepresentable result
/// refuses. Transcendentals use the same pinned libm on every target. Exact
/// quarter turns and zero deltas keep exact integer coefficients.
pub fn object_rotation(
    pivot: Point,
    start: Point,
    current: Point,
    snap_15: bool,
) -> Result<Affine, GeometryError> {
    let first = direction(pivot, start)?;
    let last = direction(pivot, current)?;
    let cross = first.0 * last.1 - first.1 * last.0;
    let dot = first.0 * last.0 + first.1 * last.1;
    let angle = libm::atan2(cross, dot);
    let (sine, cosine) = if snap_15 {
        let step = core::f64::consts::PI / 12.0;
        let count = libm::round(angle / step) as i32;
        if count % 6 == 0 {
            quarter(count / 6)
        } else {
            let snapped = f64::from(count) * step;
            (libm::sin(snapped), libm::cos(snapped))
        }
    } else if cross == 0.0 {
        (0.0, if dot < 0.0 { -1.0 } else { 1.0 })
    } else if dot == 0.0 {
        (if cross < 0.0 { -1.0 } else { 1.0 }, 0.0)
    } else {
        (libm::sin(angle), libm::cos(angle))
    };
    if sine == 0.0 && cosine == 1.0 {
        return Ok(Affine::IDENTITY);
    }
    Affine::translation(-pivot.x(), -pivot.y())?
        .then(Affine::new(cosine, sine, -sine, cosine, 0.0, 0.0)?)?
        .then(Affine::translation(pivot.x(), pivot.y())?)
}

fn direction(pivot: Point, point: Point) -> Result<(f64, f64), GeometryError> {
    let x = point.x() - pivot.x();
    let y = point.y() - pivot.y();
    finite(&[x, y])?;
    let extent = libm::fabs(x).max(libm::fabs(y));
    if extent == 0.0 {
        return Err(GeometryError::EmptyExtent);
    }
    Ok((x / extent, y / extent))
}

fn quarter(count: i32) -> (f64, f64) {
    match count.rem_euclid(4) {
        0 => (0.0, 1.0),
        1 => (1.0, 0.0),
        2 => (0.0, -1.0),
        _ => (-1.0, 0.0),
    }
}
