use crate::{Affine, GeometryError, QuarterTurn, Rect, Size};

/// EXIF tag 274: raw stored pixel grid R to upright, top-left, y-down D.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExifOrientation {
    /// Tag 1: unchanged.
    Normal = 1,
    /// Tag 2: horizontal reflection.
    MirrorHorizontal = 2,
    /// Tag 3: half turn.
    Rotate180 = 3,
    /// Tag 4: vertical reflection.
    MirrorVertical = 4,
    /// Tag 5: reflection across x=y.
    Transpose = 5,
    /// Tag 6: clockwise quarter turn.
    Rotate90 = 6,
    /// Tag 7: reflection across the opposite diagonal.
    Transverse = 7,
    /// Tag 8: counterclockwise quarter turn.
    Rotate270 = 8,
}

impl ExifOrientation {
    /// Parse a present EXIF orientation tag; callers may default absent tags to 1.
    pub fn from_tag(tag: u8) -> Result<Self, GeometryError> {
        match tag {
            1 => Ok(Self::Normal),
            2 => Ok(Self::MirrorHorizontal),
            3 => Ok(Self::Rotate180),
            4 => Ok(Self::MirrorVertical),
            5 => Ok(Self::Transpose),
            6 => Ok(Self::Rotate90),
            7 => Ok(Self::Transverse),
            8 => Ok(Self::Rotate270),
            _ => Err(GeometryError::InvalidOrientation),
        }
    }
    /// Oriented source-pixel dimensions, one D unit per oriented source pixel.
    pub const fn document_size(self, raw: Size) -> Size {
        match self {
            Self::Normal | Self::MirrorHorizontal | Self::Rotate180 | Self::MirrorVertical => raw,
            _ => raw.swapped(),
        }
    }
    /// Continuous pixel-edge R to D mapping; pixel centers include their 0.5 offset.
    pub fn raw_to_document(self, raw: Size) -> Result<Affine, GeometryError> {
        let (w, h) = (raw.width(), raw.height());
        match self {
            Self::Normal => Ok(Affine::IDENTITY),
            Self::MirrorHorizontal => Affine::new(-1.0, 0.0, 0.0, 1.0, w, 0.0),
            Self::Rotate180 => QuarterTurn::Half.matrix(raw),
            Self::MirrorVertical => Affine::new(1.0, 0.0, 0.0, -1.0, 0.0, h),
            Self::Transpose => Affine::new(0.0, 1.0, 1.0, 0.0, 0.0, 0.0),
            Self::Rotate90 => QuarterTurn::Clockwise90.matrix(raw),
            Self::Transverse => Affine::new(0.0, -1.0, -1.0, 0.0, h, w),
            Self::Rotate270 => QuarterTurn::Clockwise270.matrix(raw),
        }
    }
    /// Inverse upright D to stored R mapping.
    pub fn document_to_raw(self, raw: Size) -> Result<Affine, GeometryError> {
        self.raw_to_document(raw)?.inverse()
    }
}

/// A PDF page's crop box and /Rotate mapped to one y-down D space per page.
/// Crop coordinates are raw PDF points with x right and y up; one D unit is one
/// PDF point. The crop rectangle's `top()` is therefore its minimum PDF y.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PdfPage {
    document_size: Size,
    raw_to_document: Affine,
}

impl PdfPage {
    /// Apply crop translation, y flip, then the clockwise PDF /Rotate value.
    pub fn new(crop: Rect, rotation: QuarterTurn) -> Result<Self, GeometryError> {
        let size = crop.size()?;
        let raw_to_document = Affine::translation(-crop.left(), -crop.top())?
            .then(Affine::new(1.0, 0.0, 0.0, -1.0, 0.0, size.height())?)?
            .then(rotation.matrix(size)?)?;
        Ok(Self {
            document_size: rotation.oriented_size(size),
            raw_to_document,
        })
    }
    /// Rotated crop dimensions in PDF points.
    pub const fn document_size(self) -> Size {
        self.document_size
    }
    /// Raw PDF point to the page's top-left, y-down D coordinates.
    pub const fn raw_to_document(self) -> Affine {
        self.raw_to_document
    }
    /// Document position back to raw PDF coordinates.
    pub fn document_to_raw(self) -> Result<Affine, GeometryError> {
        self.raw_to_document.inverse()
    }
}

/// SVG viewBox fitting with centered alignment (xMidYMid).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgFit {
    /// Default SVG meet: preserve aspect and fit entirely in the viewport.
    Meet,
    /// Preserve aspect and cover the viewport, allowing overflow to be clipped.
    Slice,
    /// preserveAspectRatio="none": scale axes independently.
    Stretch,
}

/// SVG user coordinates mapped through viewBox at intrinsic dimensions into D.
/// Missing intrinsic dimensions use viewBox size; missing viewBox uses explicit
/// content bounds and is flagged. Parsing CSS sizes and clipping belong to import.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SvgViewport {
    document_size: Size,
    user_to_document: Affine,
    used_content_bounds: bool,
}

impl SvgViewport {
    /// Resolve a nonempty viewBox (or flagged content bounds) and intrinsic size.
    pub fn new(
        intrinsic: Option<Size>,
        view_box: Option<Rect>,
        content_bounds: Option<Rect>,
        fit: SvgFit,
    ) -> Result<Self, GeometryError> {
        let bounds = view_box
            .or(content_bounds)
            .ok_or(GeometryError::MissingSvgBounds)?;
        let size = bounds.size()?;
        let document_size = intrinsic.unwrap_or(size);
        let (sx, sy) = (
            document_size.width() / size.width(),
            document_size.height() / size.height(),
        );
        let (sx, sy) = match fit {
            SvgFit::Meet => (sx.min(sy), sx.min(sy)),
            SvgFit::Slice => (sx.max(sy), sx.max(sy)),
            SvgFit::Stretch => (sx, sy),
        };
        if sx <= 0.0 || sy <= 0.0 {
            return Err(GeometryError::InvalidScale);
        }
        let user_to_document = Affine::translation(-bounds.left(), -bounds.top())?
            .then(Affine::scale(sx, sy)?)?
            .then(Affine::translation(
                (document_size.width() - size.width() * sx) / 2.0,
                (document_size.height() - size.height() * sy) / 2.0,
            )?)?;
        Ok(Self {
            document_size,
            user_to_document,
            used_content_bounds: view_box.is_none(),
        })
    }
    /// Intrinsic document dimensions after applying the viewBox mapping.
    pub const fn document_size(self) -> Size {
        self.document_size
    }
    /// SVG user coordinates to D; no physical screen DPI is applied.
    pub const fn user_to_document(self) -> Affine {
        self.user_to_document
    }
    /// D to original SVG user coordinates.
    pub fn document_to_user(self) -> Result<Affine, GeometryError> {
        self.user_to_document.inverse()
    }
    /// True when an absent viewBox forced the content-bounds fallback.
    pub const fn used_content_bounds(self) -> bool {
        self.used_content_bounds
    }
}
