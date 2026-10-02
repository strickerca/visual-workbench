use crate::{GeometryError, QuarterTurn, finite};

/// Monotonic capture-geometry identity; F to H is valid only at this revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GeometryRevision(pub u64);

/// An integer pixel index, in either capture grid F or virtual Windows screen H.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelPoint {
    /// Horizontal physical pixel index (may be negative in H).
    pub x: i32,
    /// Vertical physical pixel index (may be negative in H).
    pub y: i32,
}

/// A nonempty client rectangle in Windows virtual-screen physical pixels H.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostRect {
    origin: PixelPoint,
    width: u32,
    height: u32,
}

impl HostRect {
    /// Check that every contained pixel and every frame index fits in i32.
    pub fn new(origin: PixelPoint, width: u32, height: u32) -> Result<Self, GeometryError> {
        if width == 0 || height == 0 {
            return Err(GeometryError::EmptyExtent);
        }
        if width > i32::MAX as u32
            || height > i32::MAX as u32
            || i64::from(origin.x) + i64::from(width) - 1 > i64::from(i32::MAX)
            || i64::from(origin.y) + i64::from(height) - 1 > i64::from(i32::MAX)
        {
            return Err(GeometryError::IntegerOverflow);
        }
        Ok(Self {
            origin,
            width,
            height,
        })
    }
    /// Minimum physical pixel index, including negative monitor origins.
    pub const fn origin(self) -> PixelPoint {
        self.origin
    }
    /// Number of physical pixel columns.
    pub const fn width(self) -> u32 {
        self.width
    }
    /// Number of physical pixel rows.
    pub const fn height(self) -> u32 {
        self.height
    }
}

/// Exact F↔H mapping for an oriented lossless physical-pixel capture.
///
/// DPI is metadata: multiplying already physical pixels by it would scale twice.
/// `rotation` orients frame F into the host client rectangle. Window/source IDs
/// and capture timestamps belong to the enclosing capture record, not this math.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureGeometry {
    client_rect: HostRect,
    dpi_scale: f64,
    revision: GeometryRevision,
    rotation: QuarterTurn,
}

impl CaptureGeometry {
    /// Bind a physical rectangle and orientation to its revision and positive DPI.
    pub fn new(
        client_rect: HostRect,
        dpi_scale: f64,
        revision: GeometryRevision,
        rotation: QuarterTurn,
    ) -> Result<Self, GeometryError> {
        finite(&[dpi_scale])?;
        if dpi_scale <= 0.0 {
            return Err(GeometryError::InvalidScale);
        }
        Ok(Self {
            client_rect,
            dpi_scale,
            revision,
            rotation,
        })
    }
    /// Captured physical client rectangle in H.
    pub const fn client_rect(self) -> HostRect {
        self.client_rect
    }
    /// Display density metadata; deliberately absent from F↔H arithmetic.
    pub const fn dpi_scale(self) -> f64 {
        self.dpi_scale
    }
    /// Revision required by both mapping directions.
    pub const fn revision(self) -> GeometryRevision {
        self.revision
    }
    /// Frame-to-host clockwise orientation.
    pub const fn rotation(self) -> QuarterTurn {
        self.rotation
    }
    /// Frame dimensions in physical pixels before orientation.
    pub const fn frame_extent(self) -> [u32; 2] {
        match self.rotation {
            QuarterTurn::Zero | QuarterTurn::Half => {
                [self.client_rect.width, self.client_rect.height]
            }
            _ => [self.client_rect.height, self.client_rect.width],
        }
    }
    fn check_revision(self, actual: GeometryRevision) -> Result<(), GeometryError> {
        if self.revision != actual {
            return Err(GeometryError::StaleGeometry {
                expected: self.revision,
                actual,
            });
        }
        Ok(())
    }
    /// Map a frame pixel index to H exactly, rejecting stale or out-of-frame input.
    pub fn frame_to_host(
        self,
        frame: PixelPoint,
        current: GeometryRevision,
    ) -> Result<PixelPoint, GeometryError> {
        self.check_revision(current)?;
        let [w, h] = self.frame_extent().map(i64::from);
        let (x, y) = (i64::from(frame.x), i64::from(frame.y));
        if x < 0 || y < 0 || x >= w || y >= h {
            return Err(GeometryError::OutOfBounds);
        }
        let (x, y) = match self.rotation {
            QuarterTurn::Zero => (x, y),
            QuarterTurn::Clockwise90 => (h - 1 - y, x),
            QuarterTurn::Half => (w - 1 - x, h - 1 - y),
            QuarterTurn::Clockwise270 => (y, w - 1 - x),
        };
        pixel(
            x + i64::from(self.client_rect.origin.x),
            y + i64::from(self.client_rect.origin.y),
        )
    }
    /// Map a contained host pixel to its original F index without rounding or DPI.
    pub fn host_to_frame(
        self,
        host: PixelPoint,
        current: GeometryRevision,
    ) -> Result<PixelPoint, GeometryError> {
        self.check_revision(current)?;
        let (x, y) = (
            i64::from(host.x) - i64::from(self.client_rect.origin.x),
            i64::from(host.y) - i64::from(self.client_rect.origin.y),
        );
        if x < 0
            || y < 0
            || x >= i64::from(self.client_rect.width)
            || y >= i64::from(self.client_rect.height)
        {
            return Err(GeometryError::OutOfBounds);
        }
        let [w, h] = self.frame_extent().map(i64::from);
        let (x, y) = match self.rotation {
            QuarterTurn::Zero => (x, y),
            QuarterTurn::Clockwise90 => (y, h - 1 - x),
            QuarterTurn::Half => (w - 1 - x, h - 1 - y),
            QuarterTurn::Clockwise270 => (w - 1 - y, x),
        };
        pixel(x, y)
    }
}

fn pixel(x: i64, y: i64) -> Result<PixelPoint, GeometryError> {
    Ok(PixelPoint {
        x: i32::try_from(x).map_err(|_| GeometryError::IntegerOverflow)?,
        y: i32::try_from(y).map_err(|_| GeometryError::IntegerOverflow)?,
    })
}
