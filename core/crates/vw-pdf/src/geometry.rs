use crate::limits::MAX_TILE_SIDE;
use crate::{PdfError, PdfLimits, PdfResult};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfRect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl PdfRect {
    pub fn validate(self) -> PdfResult<Self> {
        if ![self.left, self.top, self.right, self.bottom]
            .into_iter()
            .all(valid_coordinate)
            || self.right < self.left
            || self.bottom < self.top
        {
            return Err(PdfError::Invalid);
        }
        Ok(self)
    }
    pub fn intersects(self, other: Self) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }
}

/// D is the page's displayed, cropped, intrinsically rotated, top-left space,
/// measured in PDF points. Raw PDF coordinates remain bottom-left coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageGeometry {
    left: f64,
    bottom: f64,
    right: f64,
    top: f64,
    quarter_turns: u8,
    width: f64,
    height: f64,
}

impl PageGeometry {
    pub fn new(crop: [f64; 4], quarter_turns: u8, limits: PdfLimits) -> PdfResult<Self> {
        limits.validate()?;
        let [left, bottom, right, top] = crop;
        if !crop.into_iter().all(valid_coordinate)
            || right <= left
            || top <= bottom
            || quarter_turns > 3
        {
            return Err(PdfError::Invalid);
        }
        let (width, height) = if quarter_turns % 2 == 0 {
            (right - left, top - bottom)
        } else {
            (top - bottom, right - left)
        };
        if width > limits.page_side_points || height > limits.page_side_points {
            return Err(PdfError::Limit);
        }
        Ok(Self {
            left,
            bottom,
            right,
            top,
            quarter_turns,
            width,
            height,
        })
    }
    pub fn width(self) -> f64 {
        self.width
    }
    pub fn height(self) -> f64 {
        self.height
    }
    pub fn quarter_turns(self) -> u8 {
        self.quarter_turns
    }
    pub fn crop_pdf(self) -> [f64; 4] {
        [self.left, self.bottom, self.right, self.top]
    }

    pub fn pdf_to_document(self, point: PdfPoint) -> PdfResult<PdfPoint> {
        check_point(point)?;
        let PdfPoint { x, y } = point;
        let (x, y) = match self.quarter_turns {
            0 => (x - self.left, self.top - y),
            1 => (y - self.bottom, x - self.left),
            2 => (self.right - x, y - self.bottom),
            3 => (self.top - y, self.right - x),
            _ => return Err(PdfError::Invalid),
        };
        let result = PdfPoint { x, y };
        check_point(result)?;
        Ok(result)
    }

    pub fn document_to_pdf(self, point: PdfPoint) -> PdfResult<PdfPoint> {
        check_point(point)?;
        let PdfPoint { x, y } = point;
        let (x, y) = match self.quarter_turns {
            0 => (self.left + x, self.top - y),
            1 => (self.left + y, self.bottom + x),
            2 => (self.right - x, self.bottom + y),
            3 => (self.right - y, self.top - x),
            _ => return Err(PdfError::Invalid),
        };
        let result = PdfPoint { x, y };
        check_point(result)?;
        Ok(result)
    }

    pub fn pdf_box_to_document(self, raw: [f64; 4]) -> PdfResult<PdfRect> {
        let [left, bottom, right, top] = raw;
        if !raw.into_iter().all(valid_coordinate) || right < left || top < bottom {
            return Err(PdfError::Malformed);
        }
        let corners = [
            self.pdf_to_document(PdfPoint { x: left, y: bottom })?,
            self.pdf_to_document(PdfPoint {
                x: right,
                y: bottom,
            })?,
            self.pdf_to_document(PdfPoint { x: left, y: top })?,
            self.pdf_to_document(PdfPoint { x: right, y: top })?,
        ];
        Ok(PdfRect {
            left: corners.iter().map(|p| p.x).fold(f64::INFINITY, f64::min),
            top: corners.iter().map(|p| p.y).fold(f64::INFINITY, f64::min),
            right: corners
                .iter()
                .map(|p| p.x)
                .fold(f64::NEG_INFINITY, f64::max),
            bottom: corners
                .iter()
                .map(|p| p.y)
                .fold(f64::NEG_INFINITY, f64::max),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfTileRequest {
    pub x: f64,
    pub y: f64,
    pub pixels_per_point: f64,
    pub width: u32,
    pub height: u32,
    pub include_annotations: bool,
}

impl PdfTileRequest {
    pub fn validate(self, page: PageGeometry) -> PdfResult<usize> {
        if self.width == 0
            || self.height == 0
            || self.width > MAX_TILE_SIDE
            || self.height > MAX_TILE_SIDE
            || !self.pixels_per_point.is_finite()
            || !(0.01..=64.0).contains(&self.pixels_per_point)
            || !self.x.is_finite()
            || !self.y.is_finite()
            || self.x < 0.0
            || self.y < 0.0
            || self.x >= page.width
            || self.y >= page.height
        {
            return Err(PdfError::Invalid);
        }
        // Edge tiles may include paper outside the final page by at most one tile.
        let end_x = self.x + f64::from(self.width) / self.pixels_per_point;
        let end_y = self.y + f64::from(self.height) / self.pixels_per_point;
        if !end_x.is_finite() || !end_y.is_finite() {
            return Err(PdfError::Invalid);
        }
        (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or(PdfError::Limit)
    }
}

pub(crate) fn valid_coordinate(value: f64) -> bool {
    value.is_finite() && value.abs() <= 10_000_000.0
}
pub(crate) fn check_point(point: PdfPoint) -> PdfResult<()> {
    if valid_coordinate(point.x) && valid_coordinate(point.y) {
        Ok(())
    } else {
        Err(PdfError::Invalid)
    }
}
