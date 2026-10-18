//! Bounded PDFium engine for an exclusively owned isolated worker.
//! Runtime isolation and verified native artifacts are required by the unsafe
//! constructor; this crate does not claim that input bounds sandbox PDFium.

mod error;
mod ffi;
mod geometry;
mod limits;
mod overlay;
mod raster_redaction;
#[cfg(windows)]
mod pc_content;
#[cfg(windows)]
mod pc_redaction;
#[cfg(windows)]
mod qpdf_cleanup;
mod sink;
mod types;

pub use error::{PdfError, PdfResult};
pub use ffi::{PdfDocument, PdfRuntime};
pub use geometry::{PageGeometry, PdfPoint, PdfRect, PdfTileRequest};
pub use limits::{PdfCancellation, PdfLimits};
pub use overlay::{PdfOverlay, validate_overlays};
pub use types::{PdfCopyKind, PdfDocumentInfo, PdfExportReceipt, PdfGlyph, PdfNotice, PdfPageInfo, PdfTextChunk, PdfTile};

pub use raster_redaction::{PageRedaction, RasterProgress, RasterRedactionError, RasterRedactionOptions, RasterRedactionPlan, RasterRedactionResult, RasterSecurityPolicy, RasterWork, UnverifiedRasterCopy, VerifiedRasterRedaction, admit_raster_redaction};

#[cfg(windows)]
pub use pc_redaction::{CleanedPcCopy, PcRedactionError, PcRedactionOptions, PcRedactionPlan, PcRedactionProgress, PcRedactionResult, PcRedactionWork, PcSourceAdmission, UnverifiedPcCopy, VerifiedPcRedaction};
#[cfg(windows)]
pub use qpdf_cleanup::QpdfRuntime;
