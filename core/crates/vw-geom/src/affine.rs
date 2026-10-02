use crate::{GeometryError, finite};

/// A finite f64 position in a caller-named R, D, V, P, L, F or H space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    x: f64,
    y: f64,
}

impl Point {
    /// Construct a finite point; units and space are determined by its API context.
    pub fn new(x: f64, y: f64) -> Result<Self, GeometryError> {
        finite(&[x, y])?;
        Ok(Self { x, y })
    }
    /// Horizontal coordinate, positive to the right in D.
    pub const fn x(self) -> f64 {
        self.x
    }
    /// Vertical coordinate, positive down in D.
    pub const fn y(self) -> f64 {
        self.y
    }
}

/// Positive, finite viewport dimensions in that viewport's coordinate units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    width: f64,
    height: f64,
}

impl Size {
    /// Reject zero, negative and non-finite dimensions.
    pub fn new(width: f64, height: f64) -> Result<Self, GeometryError> {
        finite(&[width, height])?;
        if width <= 0.0 || height <= 0.0 {
            return Err(GeometryError::EmptyExtent);
        }
        Ok(Self { width, height })
    }
    /// Horizontal extent.
    pub const fn width(self) -> f64 {
        self.width
    }
    /// Vertical extent.
    pub const fn height(self) -> f64 {
        self.height
    }
    /// Swap width and height for a 90 or 270 degree orientation.
    pub const fn swapped(self) -> Self {
        Self {
            width: self.height,
            height: self.width,
        }
    }
}

/// An axis-aligned rectangle of continuous pixel edges; empty rectangles are valid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    origin: Point,
    width: f64,
    height: f64,
}

impl Rect {
    /// Create finite nonnegative extents, also checking representable far edges.
    pub fn new(left: f64, top: f64, width: f64, height: f64) -> Result<Self, GeometryError> {
        finite(&[left, top, width, height, left + width, top + height])?;
        if width < 0.0 || height < 0.0 {
            return Err(GeometryError::NegativeExtent);
        }
        Ok(Self {
            origin: Point::new(left, top)?,
            width,
            height,
        })
    }
    /// Minimum horizontal edge.
    pub const fn left(self) -> f64 {
        self.origin.x
    }
    /// Minimum vertical edge (bottom in raw PDF coordinates).
    pub const fn top(self) -> f64 {
        self.origin.y
    }
    /// Maximum horizontal edge.
    pub fn right(self) -> f64 {
        self.origin.x + self.width
    }
    /// Maximum vertical edge (top in raw PDF coordinates).
    pub fn bottom(self) -> f64 {
        self.origin.y + self.height
    }
    /// Horizontal extent.
    pub const fn width(self) -> f64 {
        self.width
    }
    /// Vertical extent.
    pub const fn height(self) -> f64 {
        self.height
    }
    /// Convert to a nonempty viewport size, rejecting empty rectangles.
    pub fn size(self) -> Result<Size, GeometryError> {
        Size::new(self.width, self.height)
    }
    /// Corners in clockwise order starting at minimum x/y.
    pub fn corners(self) -> [Point; 4] {
        [
            self.origin,
            Point {
                x: self.right(),
                y: self.top(),
            },
            Point {
                x: self.right(),
                y: self.bottom(),
            },
            Point {
                x: self.left(),
                y: self.bottom(),
            },
        ]
    }
}

/// Affine `[a c e; b d f; 0 0 1]` applied to column-vector positions.
///
/// `first.then(second)` means apply first, then second. All six coefficients
/// are finite; singular matrices are allowed until an inverse is requested.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Affine {
    /// Identity mapping, preserving every finite input point.
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };
    /// Construct from the six SVG-style affine coefficients a,b,c,d,e,f.
    pub fn new(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> Result<Self, GeometryError> {
        finite(&[a, b, c, d, e, f])?;
        Ok(Self { a, b, c, d, e, f })
    }
    /// Coefficients in a,b,c,d,e,f order, without conversion to f32.
    pub const fn coefficients(self) -> [f64; 6] {
        [self.a, self.b, self.c, self.d, self.e, self.f]
    }
    /// Translation in the destination's coordinate units.
    pub fn translation(x: f64, y: f64) -> Result<Self, GeometryError> {
        Self::new(1.0, 0.0, 0.0, 1.0, x, y)
    }
    /// Independent finite axis scales, including reflections and zero scales.
    pub fn scale(x: f64, y: f64) -> Result<Self, GeometryError> {
        Self::new(x, 0.0, 0.0, y, 0.0, 0.0)
    }
    /// Clockwise radians in a y-down space; uses libm for platform-independent math.
    pub fn rotation(radians: f64) -> Result<Self, GeometryError> {
        finite(&[radians])?;
        let (sine, cosine) = (libm::sin(radians), libm::cos(radians));
        Self::new(cosine, sine, -sine, cosine, 0.0, 0.0)
    }
    /// Map a finite point, rejecting overflow rather than returning an infinity.
    #[inline]
    pub fn map(self, point: Point) -> Result<Point, GeometryError> {
        Point::new(
            self.a * point.x + self.c * point.y + self.e,
            self.b * point.x + self.d * point.y + self.f,
        )
    }
    /// Compose in application order: the result is `next * self`.
    pub fn then(self, next: Self) -> Result<Self, GeometryError> {
        Self::new(
            next.a * self.a + next.c * self.b,
            next.b * self.a + next.d * self.b,
            next.a * self.c + next.c * self.d,
            next.b * self.c + next.d * self.d,
            next.a * self.e + next.c * self.f + next.e,
            next.b * self.e + next.d * self.f + next.f,
        )
    }
    /// Invert with row-normalized determinant evaluation so even opposite extreme
    /// axis scales need not overflow/underflow the determinant. Unrepresentable
    /// inverses or a determinant indistinguishable from zero in f64 fail.
    pub fn inverse(self) -> Result<Self, GeometryError> {
        let row_x = libm::fabs(self.a).max(libm::fabs(self.c));
        let row_y = libm::fabs(self.b).max(libm::fabs(self.d));
        if row_x == 0.0 || row_y == 0.0 {
            return Err(GeometryError::Singular);
        }
        let (a, b, c, d) = (
            self.a / row_x,
            self.b / row_y,
            self.c / row_x,
            self.d / row_y,
        );
        let determinant = a * d - b * c;
        if determinant == 0.0 {
            return Err(GeometryError::Singular);
        }
        let (ia, ib, ic, id) = (
            (d / determinant) / row_x,
            (-b / determinant) / row_x,
            (-c / determinant) / row_y,
            (a / determinant) / row_y,
        );
        Self::new(
            ia,
            ib,
            ic,
            id,
            -(ia * self.e + ic * self.f),
            -(ib * self.e + id * self.f),
        )
    }
    /// Axis-aligned bounds enclosing all four transformed rectangle corners.
    pub fn map_rect(self, rectangle: Rect) -> Result<Rect, GeometryError> {
        let corners = rectangle.corners();
        let first = self.map(corners[0])?;
        let (mut left, mut right, mut top, mut bottom) = (first.x, first.x, first.y, first.y);
        for corner in &corners[1..] {
            let point = self.map(*corner)?;
            left = left.min(point.x);
            right = right.max(point.x);
            top = top.min(point.y);
            bottom = bottom.max(point.y);
        }
        Rect::new(left, top, right - left, bottom - top)
    }
}

/// A device-local view camera: center in D, scale, and clockwise rotation.
/// Peer edits do not mutate this value; following another view must be explicit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    center: Point,
    scale: f64,
    rotation: f64,
}

impl Camera {
    /// Construct a camera with a positive scale and finite clockwise radians.
    pub fn new(center: Point, scale: f64, rotation: f64) -> Result<Self, GeometryError> {
        finite(&[scale, rotation])?;
        if scale <= 0.0 {
            return Err(GeometryError::InvalidScale);
        }
        Ok(Self {
            center,
            scale,
            rotation,
        })
    }
    /// Camera center in document space D.
    pub const fn center(self) -> Point {
        self.center
    }
    /// Device-local magnification.
    pub const fn scale(self) -> f64 {
        self.scale
    }
    /// Clockwise rotation in radians in the y-down document space.
    pub const fn rotation(self) -> f64 {
        self.rotation
    }
    /// D to V with the camera center placed at the viewport's continuous center.
    pub fn view_matrix(self, viewport: Size) -> Result<Affine, GeometryError> {
        Affine::translation(-self.center.x, -self.center.y)?
            .then(Affine::rotation(self.rotation)?)?
            .then(Affine::scale(self.scale, self.scale)?)?
            .then(Affine::translation(
                viewport.width / 2.0,
                viewport.height / 2.0,
            )?)
    }
}

/// Snap a region edge to the nearest integer; half ties are away from zero.
/// Signed zero is normalized so repeated snapping is bit-identical.
pub fn snap_edge(value: f64) -> Result<f64, GeometryError> {
    finite(&[value])?;
    let result = libm::round(value);
    Ok(if result == 0.0 { 0.0 } else { result })
}

/// Snap a marker to the center of its containing pixel (floor(value) + 0.5).
/// Reject magnitudes where a half-pixel is not representable as an f64.
pub fn snap_pixel_center(value: f64) -> Result<f64, GeometryError> {
    finite(&[value])?;
    if libm::fabs(value) >= 4_503_599_627_370_496.0 {
        return Err(GeometryError::IntegerOverflow);
    }
    Ok(libm::floor(value) + 0.5)
}
