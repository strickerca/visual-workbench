use crate::{
    Cancellation, Element, Error, Result, SNAP_SCREEN_PIXELS, Snapshot, check, snapshot::rectangle,
};
use vw_geom::{Affine, Point};

/// Point markers use distance to the visible element quad (inside is zero).
/// Box markers require every corresponding corner within 12 physical pixels.
#[derive(Clone, Copy)]
pub enum SnapQuery {
    Point(Point),
    Box([f64; 4]),
}
pub struct Snap<'a> {
    pub element: &'a Element,
    pub distance_screen_pixels: f64,
    pub bounds_document: [f64; 4],
}
impl Snapshot {
    /// The caller supplies the full D-to-physical-screen affine, including camera,
    /// display rotation and insets; no document-unit approximation of 12px.
    pub fn snap(
        &self,
        query: SnapQuery,
        document_to_screen: Affine,
        cancel: &dyn Cancellation,
    ) -> Result<Option<Snap<'_>>> {
        check(cancel)?;
        document_to_screen.inverse()?;
        let point = if let SnapQuery::Point(value) = query {
            Some(screen(document_to_screen, value)?)
        } else {
            None
        };
        let box_corners = if let SnapQuery::Box(value) = query {
            Some(map_corners(document_to_screen, value)?)
        } else {
            None
        };
        let mut best: Option<(&Element, f64, f64)> = None;
        for element in self.elements() {
            check(cancel)?;
            let rect = rectangle(element.bounds_document)?;
            if !element.enabled || rect.width() == 0.0 || rect.height() == 0.0 {
                continue;
            }
            let corners = map_corners(document_to_screen, element.bounds_document)?;
            let distance = if let Some(point) = point {
                quad_distance(point, corners)
            } else if let Some(query) = box_corners {
                query
                    .into_iter()
                    .zip(corners)
                    .map(|(a, b)| squared(a, b))
                    .fold(0.0_f64, f64::max)
            } else {
                return Err(Error::Invalid("snap query"));
            };
            if distance > SNAP_SCREEN_PIXELS * SNAP_SCREEN_PIXELS {
                continue;
            }
            let area = rect.width() * rect.height();
            // Canonical EID order breaks exact ties after distance and area;
            // choosing the smallest containing element avoids the root window.
            if best.is_none_or(|(_, d, a)| distance < d || (distance == d && area < a)) {
                best = Some((element, distance, area));
            }
        }
        Ok(best.map(|(element, distance, _)| Snap {
            element,
            distance_screen_pixels: libm::sqrt(distance),
            bounds_document: element.bounds_document,
        }))
    }
}
fn screen(affine: Affine, value: Point) -> Result<Point> {
    let point = affine.map(value)?;
    if point.x().abs() > 1_000_000_000.0 || point.y().abs() > 1_000_000_000.0 {
        return Err(Error::Limit("screen coordinates"));
    }
    Ok(point)
}
fn map_corners(affine: Affine, value: [f64; 4]) -> Result<[Point; 4]> {
    let p = rectangle(value)?.corners();
    let points = [
        screen(affine, p[0])?,
        screen(affine, p[1])?,
        screen(affine, p[2])?,
        screen(affine, p[3])?,
    ];
    let area = (points[1].x() - points[0].x()) * (points[3].y() - points[0].y())
        - (points[1].y() - points[0].y()) * (points[3].x() - points[0].x());
    if area == 0.0 {
        return Err(Error::Invalid("degenerate screen geometry"));
    }
    Ok(points)
}
fn squared(a: Point, b: Point) -> f64 {
    let dx = a.x() - b.x();
    let dy = a.y() - b.y();
    dx * dx + dy * dy
}
fn quad_distance(point: Point, corners: [Point; 4]) -> f64 {
    let mut positive = false;
    let mut negative = false;
    let mut best = f64::INFINITY;
    for i in 0..4 {
        let a = corners[i];
        let b = corners[(i + 1) % 4];
        let dx = b.x() - a.x();
        let dy = b.y() - a.y();
        let cross = dx * (point.y() - a.y()) - dy * (point.x() - a.x());
        positive |= cross > 0.0;
        negative |= cross < 0.0;
        let length = dx * dx + dy * dy;
        let t = if length == 0.0 {
            0.0
        } else {
            ((point.x() - a.x()) * dx + (point.y() - a.y()) * dy) / length
        }
        .clamp(0.0, 1.0);
        let x = point.x() - (a.x() + t * dx);
        let y = point.y() - (a.y() + t * dy);
        best = best.min(x * x + y * y);
    }
    if positive && negative { best } else { 0.0 }
}
