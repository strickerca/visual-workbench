use crate::geometry::check_point;
use crate::limits::{MAX_OVERLAY_CONTOURS, MAX_OVERLAY_PATHS, MAX_OVERLAY_POINTS};
use crate::{PdfCancellation, PdfError, PdfPoint, PdfResult};

/// One NONZERO compound fill. Contours within a highlighter are painted once,
/// not independently alpha-composited; opposite winding retains holes.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfOverlay {
    pub page: u32,
    pub contours: Vec<Vec<PdfPoint>>,
    pub rgba: [u8; 4],
    /// Local points to page D: x'=a*x+c*y+e; y'=b*x+d*y+f.
    pub transform: [f64; 6],
}

impl PdfOverlay {
    pub fn document_point(&self, point: PdfPoint) -> PdfResult<PdfPoint> {
        let [a, b, c, d, e, f] = self.transform;
        let result = PdfPoint {
            x: a * point.x + c * point.y + e,
            y: b * point.x + d * point.y + f,
        };
        check_point(result)?;
        Ok(result)
    }
}

/// Validate borrowed input and count capacity before the first native mutation.
/// The caller retains these vectors during export, so their capacities count.
pub fn validate_overlays(
    paths: &[PdfOverlay],
    pages: u32,
    cancel: &PdfCancellation,
) -> PdfResult<u64> {
    if paths.len() > MAX_OVERLAY_PATHS {
        return Err(PdfError::Limit);
    }
    let mut bytes = (paths.len() as u64)
        .checked_mul(std::mem::size_of::<PdfOverlay>() as u64)
        .ok_or(PdfError::Limit)?;
    let mut point_count = 0_usize;
    let mut contour_count = 0_usize;
    for path in paths {
        cancel.check()?;
        if path.page >= pages || path.contours.is_empty() {
            return Err(PdfError::Invalid);
        }
        contour_count = contour_count
            .checked_add(path.contours.len())
            .ok_or(PdfError::Limit)?;
        if contour_count > MAX_OVERLAY_CONTOURS {
            return Err(PdfError::Limit);
        }
        bytes = bytes
            .checked_add(
                (path.contours.capacity() as u64)
                    .checked_mul(std::mem::size_of::<Vec<PdfPoint>>() as u64)
                    .ok_or(PdfError::Limit)?,
            )
            .ok_or(PdfError::Limit)?;
        let [a, b, c, d, e, f] = path.transform;
        let determinant = a * d - b * c;
        if ![a, b, c, d, e, f]
            .into_iter()
            .all(crate::geometry::valid_coordinate)
            || !determinant.is_finite()
            || determinant.abs() < 1e-12
        {
            return Err(PdfError::Invalid);
        }
        for contour in &path.contours {
            cancel.check()?;
            if contour.len() < 3 {
                return Err(PdfError::Invalid);
            }
            point_count = point_count
                .checked_add(contour.len())
                .ok_or(PdfError::Limit)?;
            if point_count > MAX_OVERLAY_POINTS {
                return Err(PdfError::Limit);
            }
            bytes = bytes
                .checked_add(
                    (contour.capacity() as u64)
                        .checked_mul(std::mem::size_of::<PdfPoint>() as u64)
                        .ok_or(PdfError::Limit)?,
                )
                .ok_or(PdfError::Limit)?;
            for (index, point) in contour.iter().copied().enumerate() {
                if index % 256 == 0 {
                    cancel.check()?;
                }
                check_point(point)?;
                path.document_point(point)?;
            }
        }
    }
    // Native path points/segments, including serializer work, are a separate
    // conservative reserve. Worker hard limits cover the opaque implementation.
    bytes
        .checked_add(
            (point_count as u64)
                .checked_mul(128)
                .ok_or(PdfError::Limit)?,
        )
        .ok_or(PdfError::Limit)
}
