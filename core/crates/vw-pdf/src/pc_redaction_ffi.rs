//! Child of exclusive ffi: page-owned source objects never escape as handles.
use super::{Page, PdfDocument};
use crate::pc_redaction::{PaintProbe, PathPoint, PcRedactionError, PcRedactionResult, PlainPath};
use crate::{PageGeometry, PdfCancellation, PdfError, PdfPoint};
use pdfium_render::prelude::{FPDF_PAGEOBJECT, FS_MATRIX};

impl PdfDocument<'_> {
    pub(crate) fn pc_paint_probes(
        &mut self,
        index: u32,
        remaining_points: &mut usize,
        cancel: &PdfCancellation,
    ) -> PcRedactionResult<Vec<PaintProbe>> {
        cancel.check()?;
        let page = self.raw.page(index, self.info.pages)?;
        let info = page.info(index, self.limits)?;
        // SAFETY: immutable live page. Transparency groups have implicit state
        // that cannot be reconstructed from this admitted plain-path subset.
        if unsafe { page.bindings.FPDFPage_HasTransparency(page.handle) } != 0 {
            return Err(PcRedactionError::UnsupportedContent);
        }
        let mut objects = Vec::new();
        objects
            .try_reserve_exact(info.objects as usize)
            .map_err(|_| PdfError::Allocation)?;
        for object_index in 0..info.objects {
            cancel.check()?;
            // SAFETY: index admitted by CountObjects; object is page-owned.
            let object = unsafe {
                page.bindings
                    .FPDFPage_GetObject(page.handle, object_index as i32)
            };
            if object.is_null() {
                return Err(PdfError::Malformed.into());
            }
            let (mut left, mut bottom, mut right, mut top) = (0.0, 0.0, 0.0, 0.0);
            // SAFETY: borrowed immutable object, initialized scalar outputs.
            if unsafe {
                page.bindings.FPDFPageObj_GetBounds(
                    object,
                    &mut left,
                    &mut bottom,
                    &mut right,
                    &mut top,
                )
            } == 0
            {
                return Err(PdfError::Malformed.into());
            }
            let bounds = info.geometry.pdf_box_to_document([
                f64::from(left),
                f64::from(bottom),
                f64::from(right),
                f64::from(top),
            ])?;
            let path = page.plain_path(object, info.geometry, remaining_points, cancel)?;
            objects.push(PaintProbe {
                bounds,
                path,
                object: object_index,
            });
        }
        Ok(objects)
    }

    pub(crate) fn verify_pc_page(
        &mut self,
        index: u32,
        expected_paths: &[PlainPath],
        expected_images: u32,
        cancel: &PdfCancellation,
    ) -> PcRedactionResult<()> {
        cancel.check()?;
        let page = self.raw.page(index, self.info.pages)?;
        let info = page.info(index, self.limits)?;
        if info.annotations != 0 || info.objects != expected_paths.len() as u32 + expected_images {
            return Err(PcRedactionError::Verification);
        }
        let text = page.text()?;
        if text.count(self.limits)? != 0 {
            return Err(PcRedactionError::Verification);
        }
        drop(text);
        let mut points = crate::pc_redaction::MAX_POINTS;
        for ordinal in 0..info.objects {
            cancel.check()?;
            // SAFETY: immutable page-owned object and CountObjects-bounded index.
            let object = unsafe {
                page.bindings
                    .FPDFPage_GetObject(page.handle, ordinal as i32)
            };
            if object.is_null() {
                return Err(PcRedactionError::Verification);
            }
            if let Some(expected) = expected_paths.get(ordinal as usize) {
                let actual = page
                    .plain_path(object, info.geometry, &mut points, cancel)?
                    .ok_or(PcRedactionError::Verification)?;
                if actual.rgb != expected.rgb
                    || actual.even_odd != expected.even_odd
                    || actual.points.len() != expected.points.len()
                    || actual.points.iter().zip(&expected.points).any(|(a, b)| {
                        a.move_to != b.move_to
                            || a.close != b.close
                            || (a.point.x - b.point.x).abs() > 0.0001
                            || (a.point.y - b.point.y).abs() > 0.0001
                    })
                {
                    return Err(PcRedactionError::Verification);
                }
            } else {
                // SAFETY: same immutable page-owned object; IMAGE=3 only.
                if unsafe { page.bindings.FPDFPageObj_GetType(object) } != 3 {
                    return Err(PcRedactionError::Verification);
                }
            }
        }
        Ok(())
    }
}

impl Page<'_> {
    fn plain_path(
        &self,
        object: FPDF_PAGEOBJECT,
        geometry: PageGeometry,
        remaining_points: &mut usize,
        cancel: &PdfCancellation,
    ) -> PcRedactionResult<Option<PlainPath>> {
        // SAFETY: all calls borrow one immutable page-owned object. The source
        // qpdf preflight separately denies clipping/text-clip, blend states,
        // page groups, default color-space overrides and arbitrary resources.
        if unsafe { self.bindings.FPDFPageObj_GetType(object) } != 2
            || unsafe { self.bindings.FPDFPageObj_HasTransparency(object) } != 0
            || unsafe { self.bindings.FPDFPageObj_CountMarks(object) } != 0
        {
            return Ok(None);
        }
        let (mut fill, mut stroke) = (0, 0);
        // SAFETY: valid path object and initialized scalar out parameters.
        if unsafe {
            self.bindings
                .FPDFPath_GetDrawMode(object, &mut fill, &mut stroke)
        } == 0
        {
            return Err(PdfError::Native.into());
        }
        if stroke != 0 || !(1..=2).contains(&fill) {
            return Ok(None);
        }
        let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
        // SAFETY: immutable path and initialized color out parameters.
        if unsafe {
            self.bindings
                .FPDFPageObj_GetFillColor(object, &mut r, &mut g, &mut b, &mut a)
        } == 0
        {
            return Ok(None);
        }
        if a != 255 || r > 255 || g > 255 || b > 255 {
            return Ok(None);
        }
        let mut matrix = FS_MATRIX {
            a: 0.0,
            b: 0.0,
            c: 0.0,
            d: 0.0,
            e: 0.0,
            f: 0.0,
        };
        // SAFETY: immutable path and initialized matrix output.
        if unsafe { self.bindings.FPDFPageObj_GetMatrix(object, &mut matrix) } == 0 {
            return Err(PdfError::Native.into());
        }
        if ![matrix.a, matrix.b, matrix.c, matrix.d, matrix.e, matrix.f]
            .iter()
            .all(|v| v.is_finite())
        {
            return Err(PdfError::Malformed.into());
        }
        // SAFETY: immutable admitted path.
        let count = unsafe { self.bindings.FPDFPath_CountSegments(object) };
        let count = usize::try_from(count).map_err(|_| PdfError::Malformed)?;
        if count < 3 {
            return Ok(None);
        }
        if count > *remaining_points {
            return Err(PcRedactionError::Limit);
        }
        let mut points = Vec::new();
        points
            .try_reserve_exact(count)
            .map_err(|_| PdfError::Allocation)?;
        let mut contour_open = false;
        for ordinal in 0..count {
            if ordinal % 256 == 0 {
                cancel.check()?;
            }
            // SAFETY: index range was admitted above; segment remains path-owned.
            let segment = unsafe {
                self.bindings
                    .FPDFPath_GetPathSegment(object, ordinal as i32)
            };
            if segment.is_null() {
                return Err(PdfError::Malformed.into());
            }
            // SAFETY: immutable path-owned segment.
            let kind = unsafe { self.bindings.FPDFPathSegment_GetType(segment) };
            // PDFium7881: LINETO=0, BEZIERTO=1, MOVETO=2. Curves are visibly
            // refused as unaffected content rather than approximated by lines.
            if kind != 0 && kind != 2 {
                return Ok(None);
            }
            let (mut x, mut y) = (0f32, 0f32);
            // SAFETY: live immutable segment and initialized outputs.
            if unsafe {
                self.bindings
                    .FPDFPathSegment_GetPoint(segment, &mut x, &mut y)
            } == 0
            {
                return Err(PdfError::Native.into());
            }
            // Match the source renderer's f32 transform coefficients. qpdf
            // admission already forbids additional clip/group/color state.
            let point = geometry.pdf_to_document(PdfPoint {
                x: f64::from(matrix.a * x + matrix.c * y + matrix.e),
                y: f64::from(matrix.b * x + matrix.d * y + matrix.f),
            })?;
            if point.x < 0.0
                || point.y < 0.0
                || point.x > geometry.width()
                || point.y > geometry.height()
            {
                return Ok(None);
            }
            if kind == 2 {
                contour_open = true;
            } else if !contour_open {
                return Ok(None);
            }
            // SAFETY: immutable borrowed segment; the close flag is scalar.
            let close = unsafe { self.bindings.FPDFPathSegment_GetClose(segment) } != 0;
            if close {
                contour_open = false;
            }
            points.push(PathPoint {
                point,
                move_to: kind == 2,
                close,
            });
        }
        // Canonicalize native explicit close-to-first points. Re-emitting an
        // explicit duplicate plus `h` must not grow the path on every reopen.
        let mut start = None;
        let mut canonical: Vec<PathPoint> = Vec::new();
        canonical
            .try_reserve_exact(points.len())
            .map_err(|_| PdfError::Allocation)?;
        for point in points {
            if point.move_to {
                start = Some(point.point);
            }
            if point.close && !point.move_to && start == Some(point.point) {
                if let Some(last) = canonical.last_mut() {
                    last.close = true;
                } else {
                    return Ok(None);
                }
            } else {
                canonical.push(point);
            }
        }
        *remaining_points = remaining_points
            .checked_sub(canonical.capacity())
            .ok_or(PcRedactionError::Limit)?;
        Ok(Some(PlainPath {
            points: canonical,
            rgb: [r as u8, g as u8, b as u8],
            even_odd: fill == 1,
        }))
    }
}
