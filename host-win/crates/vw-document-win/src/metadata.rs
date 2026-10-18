//! Typed result validation happens before the host writes any DATA payload.
use serde::{Deserialize, Serialize};
use vw_document_protocol::{
    CopyIdentity, CopyKind, Operation, ProtocolError, ProtocolResult, Request,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentFormat {
    Pdf,
    Svg,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Notice {
    SystemFontsDisabled,
    NonInteractiveForms,
    OriginalDocumentFeaturesRetained,
    ScriptOrEventIgnored,
    ForeignObjectIgnored,
    AnimationIgnored,
    ResourceDenied,
    StylesheetLimited,
    UnsupportedFilter,
    UnsupportedElement,
    MissingFont,
    MissingGlyph,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Glyph {
    pub index: u32,
    pub unicode: Option<char>,
    pub bounds: Option<[f64; 4]>,
    pub generated: bool,
}
/// Debug intentionally omits document text and payload metadata. The application
/// may display actual characters, but must not copy them into worker logs.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResultMetadata {
    PdfInfo {
        pages: u32,
        encrypted: bool,
        notices: Vec<Notice>,
    },
    PdfPage {
        page: u32,
        width: f64,
        height: f64,
        crop: [f64; 4],
        quarter_turns: u8,
        objects: u32,
        annotations: u32,
    },
    RgbaTile {
        format: DocumentFormat,
        page: Option<u32>,
        origin: [f64; 2],
        requested_scale: f64,
        native_scale: f64,
        width: u32,
        height: u32,
        opaque_paper: bool,
    },
    PdfText {
        page: u32,
        total_characters: u32,
        start: u32,
        next: Option<u32>,
        glyphs: Vec<Glyph>,
    },
    PdfSelection {
        page: u32,
        glyphs: Vec<Glyph>,
    },
    PdfCopy {
        export_identity: CopyIdentity,
        copy: CopyKind,
        pages: u32,
        bytes: u64,
        redacted: bool,
    },
    SvgInfo {
        width: f64,
        height: f64,
        notices: Vec<Notice>,
    },
    SvgCopy {
        export_identity: CopyIdentity,
        bytes: u64,
        notices: Vec<Notice>,
    },
    RasterRedaction {
        export_identity: CopyIdentity,
        pages: u32,
        regions: u32,
        bytes: u64,
        requested_scale: f64,
        effective_scale: f64,
        plan_blake3: String,
    },
    PcRedaction {
        export_identity: CopyIdentity,
        pages: u32,
        regions: u32,
        bytes: u64,
        requested_scale: f64,
        effective_scale: f64,
        plan_blake3: String,
        removed_objects: u32,
        preserved_paths: u32,
        raster_tiles: u32,
    },
}
impl std::fmt::Debug for ResultMetadata {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output
            .debug_struct("ResultMetadata")
            .field("data_bytes", &self.data_bytes())
            .finish_non_exhaustive()
    }
}
impl ResultMetadata {
    /// Existing image-export JSON vocabulary plus the full admitted identity.
    /// One serializer is used by both isolated exporters and final-byte checks.
    pub fn identity_metadata(identity: &CopyIdentity) -> ProtocolResult<String> {
        identity.validate()?;
        let mut value = serde_json::to_value(identity).map_err(|_| ProtocolError::Invalid)?;
        value.as_object_mut().ok_or(ProtocolError::Invalid)?.insert(
            "revision".into(),
            serde_json::Value::String(identity.revision()?),
        );
        serde_json::to_string(&value).map_err(|_| ProtocolError::Invalid)
    }
    pub fn export_identity(&self) -> Option<&CopyIdentity> {
        match self {
            Self::PdfCopy {
                export_identity, ..
            }
            | Self::SvgCopy {
                export_identity, ..
            }
            | Self::RasterRedaction {
                export_identity, ..
            }
            | Self::PcRedaction {
                export_identity, ..
            } => Some(export_identity),
            _ => None,
        }
    }
    pub fn parse_for(bytes: &[u8], request: &Request) -> ProtocolResult<Self> {
        if bytes.len() > vw_document_protocol::MAX_REPLY_METADATA {
            return Err(ProtocolError::Limit);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ProtocolError::Invalid)?;
        value.validate_for(request)?;
        Ok(value)
    }
    pub fn data_bytes(&self) -> u64 {
        match self {
            Self::RgbaTile { width, height, .. } => u64::from(*width)
                .saturating_mul(u64::from(*height))
                .saturating_mul(4),
            Self::PdfCopy { bytes, .. }
            | Self::SvgCopy { bytes, .. }
            | Self::RasterRedaction { bytes, .. }
            | Self::PcRedaction { bytes, .. } => *bytes,
            _ => 0,
        }
    }
    pub fn validate_for(&self, request: &Request) -> ProtocolResult<()> {
        request.validate()?;
        if request.is_copy() {
            if self.export_identity() != Some(request.require_copy_identity()?) {
                return Err(ProtocolError::Integrity);
            }
        } else if self.export_identity().is_some() {
            return Err(ProtocolError::Invalid);
        }
        let valid = match (&request.operation, self) {
            (Operation::PdfInfo, Self::PdfInfo { pages, notices, .. }) => {
                pages_valid(*pages) && notices_valid(notices, DocumentFormat::Pdf)
            }
            (
                Operation::PdfPage { page: wanted },
                Self::PdfPage {
                    page,
                    width,
                    height,
                    crop,
                    quarter_turns,
                    objects,
                    annotations,
                },
            ) => {
                page == wanted
                    && dimensions(*width, *height)
                    && rectangle(*crop)
                    && *quarter_turns < 4
                    && *objects <= 100_000
                    && *annotations <= 4096
            }
            (
                Operation::PdfTile {
                    page: wanted, tile, ..
                },
                Self::RgbaTile {
                    format,
                    page,
                    origin,
                    requested_scale,
                    native_scale,
                    width,
                    height,
                    opaque_paper,
                },
            ) => {
                *format == DocumentFormat::Pdf
                    && *page == Some(*wanted)
                    && *origin == [tile.x, tile.y]
                    && *requested_scale == tile.scale
                    && *native_scale == f64::from(tile.scale as f32)
                    && *width == tile.width
                    && *height == tile.height
                    && *opaque_paper
            }
            (
                Operation::SvgTile { tile },
                Self::RgbaTile {
                    format,
                    page,
                    origin,
                    requested_scale,
                    native_scale,
                    width,
                    height,
                    opaque_paper,
                },
            ) => {
                *format == DocumentFormat::Svg
                    && page.is_none()
                    && *origin == [tile.x, tile.y]
                    && *requested_scale == tile.scale
                    && *native_scale == f64::from(tile.scale as f32)
                    && *width == tile.width
                    && *height == tile.height
                    && !*opaque_paper
            }
            (
                Operation::PdfText {
                    page: wanted,
                    start: first,
                    count,
                },
                Self::PdfText {
                    page,
                    total_characters,
                    start,
                    next,
                    glyphs,
                },
            ) => {
                let end = first.saturating_add(*count).min(*total_characters);
                page == wanted
                    && start == first
                    && *start <= *total_characters
                    && *total_characters <= 100_000
                    && next == &((end < *total_characters).then_some(end))
                    && glyphs.len() == (end - *start) as usize
                    && glyphs_valid(glyphs)
                    && glyphs
                        .iter()
                        .enumerate()
                        .all(|(offset, glyph)| glyph.index == *start + offset as u32)
            }
            (
                Operation::PdfSelection {
                    page: wanted,
                    rectangle: area,
                },
                Self::PdfSelection { page, glyphs },
            ) => {
                page == wanted
                    && glyphs_valid(glyphs)
                    && glyphs.iter().all(|glyph| {
                        glyph.bounds.is_some_and(|bounds| {
                            bounds[0] < area[2]
                                && area[0] < bounds[2]
                                && bounds[1] < area[3]
                                && area[1] < bounds[3]
                        })
                    })
            }
            (
                Operation::PdfCopy { copy: wanted, .. },
                Self::PdfCopy {
                    copy,
                    pages,
                    redacted,
                    bytes,
                    ..
                },
            ) => copy == wanted && pages_valid(*pages) && !*redacted && *bytes >= 8,
            (
                Operation::SvgInfo,
                Self::SvgInfo {
                    width,
                    height,
                    notices,
                },
            ) => dimensions(*width, *height) && notices_valid(notices, DocumentFormat::Svg),
            (Operation::SvgCopy { .. }, Self::SvgCopy { bytes, notices, .. }) => {
                *bytes > 0
                    && *bytes <= 4 * 1024 * 1024
                    && notices_valid(notices, DocumentFormat::Svg)
            }
            (
                Operation::PdfRasterRedaction {
                    scale,
                    regions: wanted,
                    ..
                },
                Self::RasterRedaction {
                    pages,
                    regions,
                    bytes,
                    requested_scale,
                    effective_scale,
                    plan_blake3,
                    ..
                },
            ) => {
                pages_valid(*pages)
                    && *regions as usize == wanted.len()
                    && *requested_scale == *scale
                    && *effective_scale == f64::from(*scale as f32)
                    && *bytes >= 8
                    && *bytes <= request.limits.input_bytes
                    && plan_blake3.len() == 64
                    && plan_blake3
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            }
            (
                Operation::PdfPcRedaction {
                    scale,
                    regions: wanted,
                    ..
                },
                Self::PcRedaction {
                    pages,
                    regions,
                    bytes,
                    requested_scale,
                    effective_scale,
                    plan_blake3,
                    removed_objects,
                    preserved_paths,
                    raster_tiles,
                    ..
                },
            ) => {
                pages_valid(*pages)
                    && *regions as usize == wanted.len()
                    && *requested_scale == *scale
                    && *effective_scale == f64::from(*scale as f32)
                    && *bytes >= 8
                    && *bytes <= request.limits.input_bytes
                    && plan_blake3.len() == 64
                    && plan_blake3
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                    && *removed_objects <= 100_000_000
                    && *preserved_paths <= 100_000
                    && *raster_tiles <= 100_000
            }
            _ => false,
        };
        if !valid {
            return Err(ProtocolError::Invalid);
        }
        if self.data_bytes() > request.limits.output_bytes {
            return Err(ProtocolError::Limit);
        }
        Ok(())
    }
}
fn pages_valid(pages: u32) -> bool {
    (1..=1000).contains(&pages)
}
fn dimensions(width: f64, height: f64) -> bool {
    width.is_finite()
        && height.is_finite()
        && width > 0.0
        && height > 0.0
        && width <= 65_535.0
        && height <= 65_535.0
}
fn rectangle(value: [f64; 4]) -> bool {
    value.into_iter().all(|v| v.is_finite() && v.abs() <= 1e7)
        && value[0] <= value[2]
        && value[1] <= value[3]
}
fn glyphs_valid(glyphs: &[Glyph]) -> bool {
    glyphs.len() <= 4096
        && glyphs
            .iter()
            .all(|g| g.index < 100_000 && g.bounds.is_none_or(rectangle))
        && glyphs.windows(2).all(|pair| pair[0].index < pair[1].index)
}
fn notices_valid(notices: &[Notice], format: DocumentFormat) -> bool {
    if notices.len() > 12 {
        return false;
    }
    for (index, notice) in notices.iter().enumerate() {
        if notices[..index].contains(notice) {
            return false;
        }
        let pdf = matches!(
            notice,
            Notice::SystemFontsDisabled
                | Notice::NonInteractiveForms
                | Notice::OriginalDocumentFeaturesRetained
        );
        if pdf != (format == DocumentFormat::Pdf) {
            return false;
        }
    }
    true
}
