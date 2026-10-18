//! Exclusive PDFium FFI ownership for the isolated document worker.
use crate::limits::MAX_TEXT_CHUNK;
use crate::sink::SaveSink;
use crate::{PageGeometry, PdfCancellation, PdfCopyKind, PdfDocumentInfo, PdfError, PdfExportReceipt, PdfGlyph, PdfLimits, PdfNotice, PdfOverlay, PdfPageInfo, PdfPoint, PdfRect, PdfResult, PdfTextChunk, PdfTile, PdfTileRequest, validate_overlays};
use pdfium_render::prelude::{FPDF_ANNOTATION, FPDF_BITMAP, FPDF_DOCUMENT, FPDF_PAGE, FPDF_PAGEOBJECT, FPDF_TEXTPAGE, FS_MATRIX, FS_RECTF, PdfiumLibraryBindings};
use std::io::Write;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

static RUNTIME_OWNED: AtomicBool = AtomicBool::new(false);
struct RuntimeLease;
impl Drop for RuntimeLease {
    fn drop(&mut self) { RUNTIME_OWNED.store(false, Ordering::Release); }
}

/// Owns one PDFium initialization on one thread. Construct only inside the
/// bounded worker after verifying the exact bundled native library and notices.
pub struct PdfRuntime {
    bindings: Box<dyn PdfiumLibraryBindings>,
    limits: PdfLimits,
    initialized: bool,
    // Field drop order unloads bindings before releasing process ownership.
    _lease: RuntimeLease,
    _thread: PhantomData<Rc<()>>,
}

impl PdfRuntime {
    /// # Safety
    /// `bindings` must describe the verified PDFium 7881 ABI. This process must
    /// not initialize/use PDFium through any other API, including Pdfium::new.
    /// The OS worker must already impose memory/time/process/file-access bounds;
    /// this wrapper cannot interrupt one native parser/render call. The binary
    /// must have V8/XFA disabled and must not provide external resource callbacks.
    pub unsafe fn from_verified_bindings(bindings: Box<dyn PdfiumLibraryBindings>, limits: PdfLimits) -> PdfResult<Self> {
        let limits = limits.validate()?;
        RUNTIME_OWNED.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).map_err(|_| PdfError::Busy)?;
        let mut runtime = Self { bindings, limits, initialized: false, _lease: RuntimeLease, _thread: PhantomData };
        // SAFETY: caller guarantees a compatible, exclusively owned library;
        // RuntimeLease prevents another wrapper initialization in this process.
        unsafe { runtime.bindings.FPDF_InitLibrary(); }
        runtime.initialized = true;
        // SAFETY: global library is initialized and exclusively owned. A null
        // font provider removes host/system font lookup; no callback is retained.
        unsafe {
            runtime.bindings.FPDF_SetSystemFontInfo(std::ptr::null_mut());
            runtime.bindings.FPDF_SetSandBoxPolicy(1, 0); // machine-time access
        }
        Ok(runtime)
    }

    pub fn open<'a>(&'a mut self, source: Vec<u8>, password: Option<&str>, cancel: &PdfCancellation) -> PdfResult<PdfDocument<'a>> {
        cancel.check()?;
        self.limits.admit_input(source.len(), source.capacity())?;
        if !source.starts_with(b"%PDF-") { return Err(PdfError::Malformed); }
        if password.is_some_and(|value| value.len() > 1024 || value.as_bytes().contains(&0)) { return Err(PdfError::Invalid); }
        let raw = RawDocument::load(&*self.bindings, &source, password)?;
        cancel.check()?;
        // SAFETY: a live document owned by `raw`; queries retain no references.
        let (pages, encrypted, form_type) = unsafe {
            (raw.bindings.FPDF_GetPageCount(raw.handle), raw.bindings.FPDF_GetSecurityHandlerRevision(raw.handle) >= 0, raw.bindings.FPDF_GetFormType(raw.handle))
        };
        let pages = count(pages, self.limits.pages)?;
        if pages == 0 { return Err(PdfError::Malformed); }
        if !(0..=1).contains(&form_type) { return Err(PdfError::UnsupportedForms); }
        let mut notices = vec![PdfNotice::SystemFontsDisabled, PdfNotice::OriginalDocumentFeaturesRetained];
        if form_type == 1 { notices.push(PdfNotice::NonInteractiveForms); }
        let info = PdfDocumentInfo { pages, source_blake3: blake3::hash(&source).to_hex().to_string(), encrypted, notices };
        cancel.check()?;
        Ok(PdfDocument { raw, source, info, limits: self.limits })
    }
}

impl Drop for PdfRuntime {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: document borrows prevent runtime destruction while a child
            // is live, and the exclusive lease persists through library unload.
            unsafe { self.bindings.FPDF_DestroyLibrary(); }
        }
    }
}

/// The raw document closes before its retained original buffer is released.
/// Methods expose DTOs only; no foreign handle escapes a lexical operation.
pub struct PdfDocument<'a> {
    raw: RawDocument<'a>,
    source: Vec<u8>,
    info: PdfDocumentInfo,
    limits: PdfLimits,
}

impl PdfDocument<'_> {
    pub fn info(&self) -> &PdfDocumentInfo { &self.info }

    pub fn page_info(&mut self, page: u32, cancel: &PdfCancellation) -> PdfResult<PdfPageInfo> {
        cancel.check()?;
        let loaded = self.raw.page(page, self.info.pages)?;
        let info = loaded.info(page, self.limits)?;
        cancel.check()?;
        Ok(info)
    }

    pub fn render_tile(&mut self, page: u32, request: PdfTileRequest, cancel: &PdfCancellation) -> PdfResult<PdfTile> {
        cancel.check()?;
        let loaded = self.raw.page(page, self.info.pages)?;
        let info = loaded.info(page, self.limits)?;
        let length = request.validate(info.geometry)?;
        self.limits.admit_memory(self.source.capacity() as u64, length as u64)?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(length).map_err(|_| PdfError::Allocation)?;
        pixels.resize(length, 255); // PDF viewer paper is explicitly opaque white.
        let stride = i32::try_from(request.width.checked_mul(4).ok_or(PdfError::Limit)?).map_err(|_| PdfError::Limit)?;
        // SAFETY: dimensions/stride fit the admitted BGRA allocation; pixels is
        // never moved/reallocated while the bitmap guard retains its pointer.
        let bitmap = unsafe { loaded.bindings.FPDFBitmap_CreateEx(request.width as i32, request.height as i32, 4, pixels.as_mut_ptr().cast(), stride) };
        if bitmap.is_null() { return Err(PdfError::Native); }
        let bitmap = Bitmap { bindings: loaded.bindings, handle: bitmap };
        // PDFium first applies its own crop/rotation/top-left display matrix.
        // This matrix is D-to-tile only; applying PDF-to-D twice would be wrong.
        let matrix = FS_MATRIX {
            a: request.pixels_per_point as f32, b: 0.0, c: 0.0, d: request.pixels_per_point as f32,
            e: (-request.x * request.pixels_per_point) as f32,
            f: (-request.y * request.pixels_per_point) as f32,
        };
        let clip = FS_RECTF { left: 0.0, top: 0.0, right: request.width as f32, bottom: request.height as f32 };
        cancel.check()?;
        // SAFETY: all handles/buffers/matrix are live; validated positive scale
        // is invertible. No JS/form execution environment has been initialized.
        unsafe { loaded.bindings.FPDF_RenderPageBitmapWithMatrix(bitmap.handle, loaded.handle, &matrix, &clip, i32::from(request.include_annotations)); }
        drop(bitmap); // External buffer is no longer referenced by PDFium.
        cancel.check()?;
        for pixel in pixels.chunks_exact_mut(4) { pixel.swap(0, 2); }
        Ok(PdfTile { page, origin: PdfPoint { x: request.x, y: request.y }, pixels_per_point: request.pixels_per_point, width: request.width, height: request.height, rgba: pixels })
    }

    pub fn text_chunk(&mut self, page: u32, start: u32, length: u32, cancel: &PdfCancellation) -> PdfResult<PdfTextChunk> {
        if length == 0 || length > MAX_TEXT_CHUNK { return Err(PdfError::Limit); }
        cancel.check()?;
        self.limits.admit_memory(self.source.capacity() as u64, u64::from(length) * std::mem::size_of::<PdfGlyph>() as u64)?;
        let loaded = self.raw.page(page, self.info.pages)?;
        let geometry = loaded.info(page, self.limits)?.geometry;
        let text = loaded.text()?;
        let total = text.count(self.limits)?;
        if start > total { return Err(PdfError::Invalid); }
        let end = start.checked_add(length).ok_or(PdfError::Limit)?.min(total);
        let mut glyphs = Vec::new();
        glyphs.try_reserve_exact((end - start) as usize).map_err(|_| PdfError::Allocation)?;
        for index in start..end { cancel.check()?; glyphs.push(text.glyph(index, geometry)?); }
        cancel.check()?;
        Ok(PdfTextChunk { page, total_characters: total, start, next: (end < total).then_some(end), glyphs })
    }

    /// Bounded geometric selection. The caller receives actual character indices
    /// and Unicode mappings; this is not OCR and does not invent missing glyphs.
    pub fn select_text(&mut self, page: u32, region: PdfRect, cancel: &PdfCancellation) -> PdfResult<Vec<PdfGlyph>> {
        region.validate()?;
        cancel.check()?;
        self.limits.admit_memory(self.source.capacity() as u64, u64::from(MAX_TEXT_CHUNK) * std::mem::size_of::<PdfGlyph>() as u64)?;
        let loaded = self.raw.page(page, self.info.pages)?;
        let geometry = loaded.info(page, self.limits)?.geometry;
        if region.left < 0.0 || region.top < 0.0 || region.right > geometry.width() || region.bottom > geometry.height() { return Err(PdfError::Invalid); }
        let text = loaded.text()?;
        let total = text.count(self.limits)?;
        let mut glyphs = Vec::new();
        glyphs.try_reserve_exact(MAX_TEXT_CHUNK.min(total) as usize).map_err(|_| PdfError::Allocation)?;
        for index in 0..total {
            cancel.check()?;
            let glyph = text.glyph(index, geometry)?;
            if glyph.bounds.is_some_and(|bounds| bounds.intersects(region)) {
                if glyphs.len() == MAX_TEXT_CHUNK as usize { return Err(PdfError::Limit); }
                glyphs.push(glyph);
            }
        }
        Ok(glyphs)
    }

    /// Writes into an already privately owned staging sink. On error partial
    /// bytes remain owned by that caller and must never be published. A receipt
    /// does not replace the caller's fsync and atomic destination handoff.
    pub fn export(&mut self, kind: PdfCopyKind, overlays: &[PdfOverlay], writer: &mut dyn Write, cancel: &PdfCancellation) -> PdfResult<PdfExportReceipt> {
        cancel.check()?;
        let overlay_bytes = validate_overlays(overlays, self.info.pages, cancel)?;
        self.limits.admit_memory(self.source.capacity() as u64, overlay_bytes)?;
        if kind == PdfCopyKind::Original {
            if !overlays.is_empty() { return Err(PdfError::Invalid); }
            if self.source.len() as u64 > self.limits.output_bytes { return Err(PdfError::Limit); }
            for block in self.source.chunks(64 * 1024) {
                cancel.check()?;
                writer.write_all(block).map_err(|error| PdfError::Io(error.kind()))?;
            }
            writer.flush().map_err(|error| PdfError::Io(error.kind()))?;
            cancel.check()?;
            return Ok(self.receipt(kind, self.source.len() as u64));
        }
        // No ambiguous security rewrite: exact Original is always available;
        // modified encrypted copies need an explicit future encryption policy.
        if self.info.encrypted { return Err(PdfError::EncryptedExport); }
        // A second parser tree is native worker-owned; no second byte Vec is
        // created. Reserve input-size work in addition to the retained original.
        self.limits.admit_memory(self.source.capacity() as u64, overlay_bytes.checked_add(self.source.len() as u64).ok_or(PdfError::Limit)?)?;
        let copy = RawDocument::load(self.raw.bindings, &self.source, None)?;
        // Validate every referenced page/combined count before native mutation.
        for page in 0..self.info.pages {
            cancel.check()?;
            let loaded = copy.page(page, self.info.pages)?;
            let info = loaded.info(page, self.limits)?;
            let added = overlays.iter().filter(|overlay| overlay.page == page).count();
            let existing = if kind == PdfCopyKind::Annotated { info.annotations } else { info.objects };
            let limit = if kind == PdfCopyKind::Annotated { self.limits.annotations_per_page } else { self.limits.objects_per_page };
            if u64::from(existing) + added as u64 > u64::from(limit) { return Err(PdfError::Limit); }
            // Form widgets need a form environment to flatten faithfully; it is
            // deliberately absent. Refuse instead of losing fields silently.
            if kind == PdfCopyKind::Flattened && self.info.notices.contains(&PdfNotice::NonInteractiveForms) { return Err(PdfError::UnsupportedForms); }
        }
        for page in 0..self.info.pages {
            cancel.check()?;
            let loaded = copy.page(page, self.info.pages)?;
            let geometry = loaded.info(page, self.limits)?.geometry;
            if kind == PdfCopyKind::Flattened {
                // SAFETY: private page, no live text handles. FLAT_NORMALDISPLAY
                // flattens existing visible annotations before our new overlay.
                match unsafe { loaded.bindings.FPDFPage_Flatten(loaded.handle, 0) } {
                    1 | 2 => {},
                    _ => return Err(PdfError::Native),
                }
            }
            for overlay in overlays.iter().filter(|overlay| overlay.page == page) {
                cancel.check()?;
                loaded.add_overlay(geometry, overlay, kind, cancel)?;
            }
            loaded.info(page, self.limits)?;
            // SAFETY: private live page; edits/flattening are complete. No
            // borrowed page object is retained after insertion.
            if unsafe { loaded.bindings.FPDFPage_GenerateContent(loaded.handle) } == 0 { return Err(PdfError::Native); }
        }
        cancel.check()?;
        let mut sink = SaveSink::new(writer, cancel, self.limits.output_bytes);
        // SAFETY: synchronous save borrows the live private document/context;
        // flags=2 requests a full non-incremental rewrite, never append-save.
        let success = unsafe { copy.bindings.FPDF_SaveAsCopy(copy.handle, sink.pointer(), 2) } != 0;
        let bytes = sink.finish(success)?;
        Ok(self.receipt(kind, bytes))
    }

    fn receipt(&self, kind: PdfCopyKind, bytes: u64) -> PdfExportReceipt {
        PdfExportReceipt { pages: self.info.pages, source_blake3: self.info.source_blake3.clone(), bytes, kind, redacted: false }
    }
}

struct RawDocument<'a> { bindings: &'a dyn PdfiumLibraryBindings, handle: FPDF_DOCUMENT }
impl<'a> RawDocument<'a> {
    fn load(bindings: &'a dyn PdfiumLibraryBindings, source: &[u8], password: Option<&str>) -> PdfResult<Self> {
        // SAFETY: callers retain source bytes until this guard drops and use
        // only the owning runtime thread. Password input is bounded/no NUL.
        let handle = unsafe { bindings.FPDF_LoadMemDocument64(source, password) };
        if handle.is_null() {
            // SAFETY: only queried immediately after documented load failure.
            return Err(match unsafe { bindings.FPDF_GetLastError() } {
                4 => PdfError::Password,
                5 => PdfError::UnsupportedSecurity,
                _ => PdfError::Malformed,
            });
        }
        Ok(Self { bindings, handle })
    }
    fn page(&self, index: u32, pages: u32) -> PdfResult<Page<'_>> {
        if index >= pages { return Err(PdfError::Invalid); }
        // SAFETY: live parent document and range checked integer index.
        let handle = unsafe { self.bindings.FPDF_LoadPage(self.handle, index as i32) };
        if handle.is_null() { return Err(PdfError::Malformed); }
        Ok(Page { bindings: self.bindings, handle })
    }
}
impl Drop for RawDocument<'_> {
    fn drop(&mut self) {
        // SAFETY: sole owner; all page/text guards are lexical and already gone.
        unsafe { self.bindings.FPDF_CloseDocument(self.handle); }
    }
}

struct Page<'a> { bindings: &'a dyn PdfiumLibraryBindings, handle: FPDF_PAGE }
impl Page<'_> {
    fn info(&self, index: u32, limits: PdfLimits) -> PdfResult<PdfPageInfo> {
        let mut rect = FS_RECTF { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 };
        // SAFETY: live page; out parameter points to an initialized local value.
        let (ok, rotation, width, height, objects, annotations) = unsafe {
            (self.bindings.FPDF_GetPageBoundingBox(self.handle, &mut rect), self.bindings.FPDFPage_GetRotation(self.handle),
             self.bindings.FPDF_GetPageWidthF(self.handle), self.bindings.FPDF_GetPageHeightF(self.handle),
             self.bindings.FPDFPage_CountObjects(self.handle), self.bindings.FPDFPage_GetAnnotCount(self.handle))
        };
        if ok == 0 { return Err(PdfError::Malformed); }
        let rotation = u8::try_from(rotation).map_err(|_| PdfError::Malformed)?;
        let geometry = PageGeometry::new([f64::from(rect.left), f64::from(rect.bottom), f64::from(rect.right), f64::from(rect.top)], rotation, limits)?;
        // Refuse any unrepresented geometry convention (including a changed
        // native UserUnit policy), rather than misplacing text or overlays.
        let tolerance = 0.01_f64;
        if !width.is_finite() || !height.is_finite()
            || (f64::from(width) - geometry.width()).abs() > tolerance
            || (f64::from(height) - geometry.height()).abs() > tolerance
        { return Err(PdfError::UnsupportedGeometry); }
        Ok(PdfPageInfo { page: index, geometry, objects: count(objects, limits.objects_per_page)?, annotations: count(annotations, limits.annotations_per_page)? })
    }
    fn text(&self) -> PdfResult<TextPage<'_>> {
        // SAFETY: live page remains borrowed until TextPage closes; no mutation.
        let handle = unsafe { self.bindings.FPDFText_LoadPage(self.handle) };
        if handle.is_null() { return Err(PdfError::Native); }
        Ok(TextPage { bindings: self.bindings, handle })
    }

    fn add_overlay(&self, geometry: PageGeometry, overlay: &PdfOverlay, kind: PdfCopyKind, cancel: &PdfCancellation) -> PdfResult<()> {
        let first = overlay.contours.first().and_then(|contour| contour.first()).copied().ok_or(PdfError::Invalid)?;
        let first = geometry.document_to_pdf(overlay.document_point(first)?)?;
        // SAFETY: finite validated coordinates; returned object is initially
        // unowned by any page/annotation and must be destroyed on every failure.
        let handle = unsafe { self.bindings.FPDFPageObj_CreateNewPath(first.x as f32, first.y as f32) };
        if handle.is_null() { return Err(PdfError::Native); }
        let mut path = PathObject { bindings: self.bindings, handle, transferred: false };
        let mut bounds = FS_RECTF { left: first.x as f32, right: first.x as f32, bottom: first.y as f32, top: first.y as f32 };
        for contour in &overlay.contours {
            cancel.check()?;
            for (index, point) in contour.iter().copied().enumerate() {
                if index % 256 == 0 { cancel.check()?; }
                let point = geometry.document_to_pdf(overlay.document_point(point)?)?;
                let (x, y) = (point.x as f32, point.y as f32);
                bounds.left = bounds.left.min(x); bounds.right = bounds.right.max(x);
                bounds.bottom = bounds.bottom.min(y); bounds.top = bounds.top.max(y);
                // SAFETY: exclusively owned path; all coordinates were checked.
                let ok = unsafe { if index == 0 { self.bindings.FPDFPath_MoveTo(path.handle, x, y) } else { self.bindings.FPDFPath_LineTo(path.handle, x, y) } };
                if ok == 0 { return Err(PdfError::Native); }
            }
            // SAFETY: a complete nonempty subpath is exclusively owned.
            if unsafe { self.bindings.FPDFPath_Close(path.handle) } == 0 { return Err(PdfError::Native); }
        }
        if bounds.left >= bounds.right || bounds.bottom >= bounds.top { return Err(PdfError::Invalid); }
        let [r, g, b, a] = overlay.rgba.map(u32::from);
        // SAFETY: live exclusively owned path; NONZERO fill, no implicit stroke.
        if unsafe { self.bindings.FPDFPath_SetDrawMode(path.handle, 2, 0) } == 0
            // SAFETY: same owned path; channels are exactly 0..=255.
            || unsafe { self.bindings.FPDFPageObj_SetFillColor(path.handle, r, g, b, a) } == 0
        { return Err(PdfError::Native); }
        if kind == PdfCopyKind::Annotated {
            // SAFETY: live private page; STAMP=13 supports compound path objects.
            let handle = unsafe { self.bindings.FPDFPage_CreateAnnot(self.handle, 13) };
            if handle.is_null() { return Err(PdfError::Native); }
            let annotation = Annotation { bindings: self.bindings, handle };
            // SAFETY: live annotation and local rectangle. PRINT flag only; no
            // action, JavaScript, file, author name or arbitrary string is added.
            if unsafe { self.bindings.FPDFAnnot_SetRect(annotation.handle, &bounds) } == 0
                // SAFETY: same exclusively owned annotation, bounded flag value.
                || unsafe { self.bindings.FPDFAnnot_SetFlags(annotation.handle, 4) } == 0
            { return Err(PdfError::Native); }
            // SAFETY: successful AppendObject transfers path ownership. On
            // failure PathObject still owns and destroys it.
            if unsafe { self.bindings.FPDFAnnot_AppendObject(annotation.handle, path.handle) } == 0 { return Err(PdfError::Native); }
            path.transferred = true;
        } else {
            // SAFETY: live page takes ownership of the valid path; insertion is
            // void in this ABI. GenerateContent later determines export success.
            unsafe { self.bindings.FPDFPage_InsertObject(self.handle, path.handle); }
            path.transferred = true;
        }
        Ok(())
    }
}
impl Drop for Page<'_> {
    fn drop(&mut self) {
        // SAFETY: all child text/annotation handles have already closed.
        unsafe { self.bindings.FPDF_ClosePage(self.handle); }
    }
}

struct TextPage<'a> { bindings: &'a dyn PdfiumLibraryBindings, handle: FPDF_TEXTPAGE }
impl TextPage<'_> {
    fn count(&self, limits: PdfLimits) -> PdfResult<u32> {
        // SAFETY: live immutable text page.
        count(unsafe { self.bindings.FPDFText_CountChars(self.handle) }, limits.characters_per_page)
    }
    fn glyph(&self, index: u32, geometry: PageGeometry) -> PdfResult<PdfGlyph> {
        let (mut left, mut right, mut bottom, mut top) = (0.0, 0.0, 0.0, 0.0);
        // SAFETY: caller bounded index to CountChars; local out parameters live.
        let (unicode, generated, has_box) = unsafe {
            (self.bindings.FPDFText_GetUnicode(self.handle, index as i32), self.bindings.FPDFText_IsGenerated(self.handle, index as i32),
             self.bindings.FPDFText_GetCharBox(self.handle, index as i32, &mut left, &mut right, &mut bottom, &mut top))
        };
        if generated < 0 { return Err(PdfError::Native); }
        let bounds = if has_box == 0 { None } else { Some(geometry.pdf_box_to_document([left, bottom, right, top])?) };
        Ok(PdfGlyph { index, unicode: (unicode != 0).then(|| char::from_u32(unicode)).flatten(), bounds, generated: generated != 0 })
    }
}
impl Drop for TextPage<'_> {
    fn drop(&mut self) {
        // SAFETY: sole owner, parent page is still alive and unmodified.
        unsafe { self.bindings.FPDFText_ClosePage(self.handle); }
    }
}
struct Bitmap<'a> { bindings: &'a dyn PdfiumLibraryBindings, handle: FPDF_BITMAP }
impl Drop for Bitmap<'_> {
    fn drop(&mut self) {
        // SAFETY: sole native wrapper; external pixel memory is still alive.
        unsafe { self.bindings.FPDFBitmap_Destroy(self.handle); }
    }
}
struct Annotation<'a> { bindings: &'a dyn PdfiumLibraryBindings, handle: FPDF_ANNOTATION }
impl Drop for Annotation<'_> {
    fn drop(&mut self) {
        // SAFETY: sole annotation handle; page owns its inserted objects.
        unsafe { self.bindings.FPDFPage_CloseAnnot(self.handle); }
    }
}
struct PathObject<'a> { bindings: &'a dyn PdfiumLibraryBindings, handle: FPDF_PAGEOBJECT, transferred: bool }
impl Drop for PathObject<'_> {
    fn drop(&mut self) {
        if !self.transferred {
            // SAFETY: only untransferred objects remain owned by this guard.
            unsafe { self.bindings.FPDFPageObj_Destroy(self.handle); }
        }
    }
}
fn count(value: i32, limit: u32) -> PdfResult<u32> {
    let value = u32::try_from(value).map_err(|_| PdfError::Malformed)?;
    if value > limit { Err(PdfError::Limit) } else { Ok(value) }
}
