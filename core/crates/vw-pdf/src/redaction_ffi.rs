//! Child of the exclusive ffi module: no additional public raw handle surface.
use super::PdfDocument;
use crate::{PdfCancellation, PdfError, PdfLimits, PdfResult};

impl PdfDocument<'_> {
    pub(crate) fn redaction_limits(&self) -> PdfLimits {
        self.limits
    }
    pub(crate) fn admit_redaction_memory(&self, additional: u64) -> PdfResult<()> {
        self.limits
            .admit_memory(self.source.capacity() as u64, additional)
    }
    pub(crate) fn verify_raster_page(
        &mut self,
        index: u32,
        expected_images: u32,
        cancel: &PdfCancellation,
    ) -> PdfResult<()> {
        cancel.check()?;
        let page = self.raw.page(index, self.info.pages)?;
        let info = page.info(index, self.limits)?;
        if info.annotations != 0 || info.objects != expected_images {
            return Err(PdfError::Malformed);
        }
        let text = page.text()?;
        if text.count(self.limits)? != 0 {
            return Err(PdfError::Malformed);
        }
        drop(text);
        for index in 0..expected_images {
            cancel.check()?;
            // SAFETY: live immutable page and count-bounded index; object remains
            // page-owned and is neither mutated, retained nor independently freed.
            let object = unsafe { page.bindings.FPDFPage_GetObject(page.handle, index as i32) };
            if object.is_null() {
                return Err(PdfError::Malformed);
            }
            // SAFETY: borrowed object remains valid until page closes; IMAGE=3.
            if unsafe { page.bindings.FPDFPageObj_GetType(object) } != 3 {
                return Err(PdfError::Malformed);
            }
        }
        cancel.check()
    }
}
