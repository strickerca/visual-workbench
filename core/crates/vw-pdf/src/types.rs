use crate::{PageGeometry, PdfPoint, PdfRect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PdfNotice {
    /// No host/system font discovery is permitted. Embedded and built-in fonts
    /// may render; unavailable substitutes can change appearance.
    SystemFontsDisabled,
    /// Form widgets/popups are not executed or rendered through a form runtime.
    NonInteractiveForms,
    /// Never execute actions. Normal copies can retain original actions/data;
    /// they are not sanitization or redaction exports.
    OriginalDocumentFeaturesRetained,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdfDocumentInfo {
    pub pages: u32,
    pub source_blake3: String,
    pub encrypted: bool,
    pub notices: Vec<PdfNotice>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdfPageInfo {
    pub page: u32,
    pub geometry: PageGeometry,
    pub objects: u32,
    pub annotations: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdfTile {
    pub page: u32,
    pub origin: PdfPoint,
    pub pixels_per_point: f64,
    pub width: u32,
    pub height: u32,
    /// Straight RGBA8, sRGB display pixels over opaque white paper. Original
    /// embedded-image bit depth/profile are not represented by this viewer tile.
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdfGlyph {
    pub index: u32,
    /// None means PDFium could not map a valid Unicode scalar. Do not invent it.
    pub unicode: Option<char>,
    /// Spaces/generated line separators can have no valid geometric box.
    pub bounds: Option<PdfRect>,
    pub generated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdfTextChunk {
    pub page: u32,
    pub total_characters: u32,
    pub start: u32,
    pub next: Option<u32>,
    pub glyphs: Vec<PdfGlyph>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PdfCopyKind {
    Original,
    Annotated,
    Flattened,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdfExportReceipt {
    pub pages: u32,
    pub source_blake3: String,
    pub bytes: u64,
    pub kind: PdfCopyKind,
    /// Explicitly false for all normal-copy methods. Redaction has a separate
    /// fresh-document and independent-verification contract.
    pub redacted: bool,
}
