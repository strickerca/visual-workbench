#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("invalid PDF request")]
    Invalid,
    #[error("PDF size or work limit exceeded")]
    Limit,
    #[error("PDF wrapper memory estimate {estimated} exceeds budget {budget}")]
    Memory { estimated: u64, budget: u64 },
    #[error("PDF allocation refused")]
    Allocation,
    #[error("PDF operation cancelled")]
    Cancelled,
    #[error("PDF runtime already owned")]
    Busy,
    #[error("malformed or unsupported PDF")]
    Malformed,
    #[error("PDF requires a valid password")]
    Password,
    #[error("unsupported PDF security handler")]
    UnsupportedSecurity,
    #[error("modified encrypted export requires an explicit encryption policy")]
    EncryptedExport,
    #[error("PDF form content cannot be represented by the non-interactive renderer")]
    UnsupportedForms,
    #[error("unsupported PDF coordinate convention")]
    UnsupportedGeometry,
    #[error("PDFium operation failed")]
    Native,
    #[error("private PDF output failed: {0:?}")]
    Io(std::io::ErrorKind),
    #[error("private PDF output writer panicked")]
    WriterPanicked,
}

pub type PdfResult<T> = Result<T, PdfError>;
