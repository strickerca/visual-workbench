//! The synchronous PDFium write callback. No pointer or callback outlives save.
use crate::{PdfCancellation, PdfError, PdfResult};
use pdfium_render::prelude::FPDF_FILEWRITE;
use std::ffi::{c_int, c_ulong, c_void};
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[repr(C)]
pub(crate) struct SaveSink<'a> {
    // First field: PDFium receives only its address and returns that exact value.
    header: FPDF_FILEWRITE,
    writer: &'a mut dyn Write,
    cancel: &'a PdfCancellation,
    limit: u64,
    written: u64,
    failure: Option<PdfError>,
}

impl<'a> SaveSink<'a> {
    pub(crate) fn new(writer: &'a mut dyn Write, cancel: &'a PdfCancellation, limit: u64) -> Self {
        Self { header: FPDF_FILEWRITE { version: 1, WriteBlock: Some(write_block) }, writer, cancel, limit, written: 0, failure: None }
    }
    pub(crate) fn pointer(&mut self) -> *mut FPDF_FILEWRITE { &mut self.header }
    pub(crate) fn finish(mut self, native_success: bool) -> PdfResult<u64> {
        if let Some(error) = self.failure.take() { return Err(error); }
        self.cancel.check()?;
        if !native_success || self.written == 0 { return Err(PdfError::Native); }
        self.writer.flush().map_err(|error| PdfError::Io(error.kind()))?;
        self.cancel.check()?;
        Ok(self.written)
    }
    fn write(&mut self, bytes: &[u8]) -> PdfResult<()> {
        self.cancel.check()?;
        let next = self.written.checked_add(bytes.len() as u64).ok_or(PdfError::Limit)?;
        if next > self.limit { return Err(PdfError::Limit); }
        self.writer.write_all(bytes).map_err(|error| PdfError::Io(error.kind()))?;
        self.written = next;
        self.cancel.check()
    }
}

unsafe extern "C" fn write_block(header: *mut FPDF_FILEWRITE, data: *const c_void, size: c_ulong) -> c_int {
    if header.is_null() { return 0; }
    // SAFETY: FPDF_SaveAsCopy synchronously calls the retained first-field
    // address. repr(C) fixes its offset; no other thread accesses the context.
    let sink = unsafe { &mut *header.cast::<SaveSink<'_>>() };
    if sink.failure.is_some() { return 0; }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let length = usize::try_from(size).map_err(|_| PdfError::Limit)?;
        if length == 0 { return sink.cancel.check(); }
        if data.is_null() || length > isize::MAX as usize { return Err(PdfError::Native); }
        // Reject a native oversize block before constructing or dereferencing a
        // slice. The pointer's validity for admitted bytes is PDFium's contract.
        if (length as u64) > sink.limit.saturating_sub(sink.written) { return Err(PdfError::Limit); }
        // SAFETY: admitted synchronous native buffer is valid for `length`
        // bytes until this callback returns; it is never retained.
        let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), length) };
        sink.write(bytes)
    }));
    match result {
        Ok(Ok(())) => 1,
        Ok(Err(error)) => { sink.failure = Some(error); 0 }
        Err(_) => { sink.failure = Some(PdfError::WriterPanicked); 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_block_is_refused_without_dereferencing_or_writing() {
        let mut bytes = Vec::new();
        let cancel = PdfCancellation::default();
        let mut sink = SaveSink::new(&mut bytes, &cancel, 4);
        // SAFETY: valid context; deliberately null oversize data must be rejected.
        assert_eq!(unsafe { write_block(sink.pointer(), std::ptr::null(), 5) }, 0);
        assert!(matches!(sink.finish(true), Err(PdfError::Native | PdfError::Limit)));
        assert!(bytes.is_empty());
    }
    #[test]
    fn exact_limit_and_cancellation_never_report_a_partial_success() {
        let mut bytes = Vec::new();
        let cancel = PdfCancellation::default();
        let mut sink = SaveSink::new(&mut bytes, &cancel, 4);
        // SAFETY: array and context live throughout the callback.
        assert_eq!(unsafe { write_block(sink.pointer(), b"test".as_ptr().cast(), 4) }, 1);
        cancel.cancel();
        assert!(matches!(sink.finish(true), Err(PdfError::Cancelled)));
        assert_eq!(bytes, b"test");
    }
    #[test]
    fn native_failure_and_writer_panics_are_typed_not_success() {
        struct Panics;
        impl Write for Panics {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> { panic!("synthetic writer"); }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        let cancel = PdfCancellation::default();
        let mut writer = Panics;
        let mut sink = SaveSink::new(&mut writer, &cancel, 4);
        // SAFETY: array and context live throughout the callback.
        assert_eq!(unsafe { write_block(sink.pointer(), b"test".as_ptr().cast(), 4) }, 0);
        assert!(matches!(sink.finish(true), Err(PdfError::WriterPanicked)));
        let mut bytes = Vec::new();
        assert!(matches!(SaveSink::new(&mut bytes, &cancel, 4).finish(false), Err(PdfError::Native)));
    }
}
