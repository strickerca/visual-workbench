use crate::{Affine, GeometryError, Size, finite};

/// Exact clockwise display orientation in the y-down screen space P.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuarterTurn {
    /// Natural orientation.
    Zero,
    /// Ninety degrees clockwise.
    Clockwise90,
    /// One hundred eighty degrees.
    Half,
    /// Two hundred seventy degrees clockwise.
    Clockwise270,
}

impl QuarterTurn {
    /// Parse integral degrees; negative and full-turn equivalents are accepted.
    pub fn from_degrees(degrees: i32) -> Result<Self, GeometryError> {
        match degrees.rem_euclid(360) {
            0 => Ok(Self::Zero),
            90 => Ok(Self::Clockwise90),
            180 => Ok(Self::Half),
            270 => Ok(Self::Clockwise270),
            _ => Err(GeometryError::InvalidOrientation),
        }
    }
    /// Dimensions after this orientation.
    pub const fn oriented_size(self, size: Size) -> Size {
        match self {
            Self::Zero | Self::Half => size,
            Self::Clockwise90 | Self::Clockwise270 => size.swapped(),
        }
    }
    /// Map continuous edges of a y-down rectangle into positive oriented space.
    pub fn matrix(self, size: Size) -> Result<Affine, GeometryError> {
        let (w, h) = (size.width(), size.height());
        match self {
            Self::Zero => Ok(Affine::IDENTITY),
            Self::Clockwise90 => Affine::new(0.0, 1.0, -1.0, 0.0, h, 0.0),
            Self::Half => Affine::new(-1.0, 0.0, 0.0, -1.0, w, h),
            Self::Clockwise270 => Affine::new(0.0, -1.0, 1.0, 0.0, 0.0, w),
        }
    }
}

/// Nonnegative safe-area insets measured in final, rotated physical screen P.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Insets {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl Insets {
    /// A screen with no excluded edges.
    pub const ZERO: Self = Self {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };
    /// Construct in left, top, right, bottom order.
    pub fn new(left: f64, top: f64, right: f64, bottom: f64) -> Result<Self, GeometryError> {
        finite(&[left, top, right, bottom])?;
        if [left, top, right, bottom].iter().any(|v| *v < 0.0) {
            return Err(GeometryError::InvalidInsets);
        }
        Ok(Self {
            left,
            top,
            right,
            bottom,
        })
    }
    /// Physical edge exclusions in left, top, right, bottom order.
    pub const fn edges(self) -> [f64; 4] {
        [self.left, self.top, self.right, self.bottom]
    }
}

/// V to physical P: orient a natural-orientation content viewport, then inset it.
///
/// `natural_panel` includes system edges. Insets are expressed after rotation.
/// Give the camera `view_size()`, not the full panel dimensions. If platform
/// coordinates already include rotation, use Zero to avoid applying it twice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenTransform {
    view_size: Size,
    view_to_screen: Affine,
    screen_to_view: Affine,
    natural_to_screen: Affine,
    screen_to_natural: Affine,
}

impl ScreenTransform {
    /// Compute the content viewport and both directions, rejecting consumed space.
    pub fn new(
        natural_panel: Size,
        rotation: QuarterTurn,
        insets: Insets,
    ) -> Result<Self, GeometryError> {
        let panel = rotation.oriented_size(natural_panel);
        let content = Size::new(
            panel.width() - insets.left - insets.right,
            panel.height() - insets.top - insets.bottom,
        )
        .map_err(|_| GeometryError::InvalidInsets)?;
        let view_size = rotation.oriented_size(content);
        let view_to_screen = rotation
            .matrix(view_size)?
            .then(Affine::translation(insets.left, insets.top)?)?;
        let natural_to_screen = rotation.matrix(natural_panel)?;
        Ok(Self {
            view_size,
            screen_to_view: view_to_screen.inverse()?,
            view_to_screen,
            natural_to_screen,
            screen_to_natural: natural_to_screen.inverse()?,
        })
    }
    /// Natural-orientation content dimensions used to center the D to V camera.
    pub const fn view_size(self) -> Size {
        self.view_size
    }
    /// V to P in physical pixels, without an additional density/DPI multiplier.
    pub const fn view_to_screen(self) -> Affine {
        self.view_to_screen
    }
    /// P to V, including removal of the rotated safe-area offset.
    pub const fn screen_to_view(self) -> Affine {
        self.screen_to_view
    }
    /// Android A (full physical panel in natural orientation) to rotated P.
    /// Full-panel input already includes system edges, so no inset is added.
    pub const fn natural_input_to_screen(self) -> Affine {
        self.natural_to_screen
    }
    /// Rotated full-screen P back to natural Android display space A.
    pub const fn screen_to_natural_input(self) -> Affine {
        self.screen_to_natural
    }
}
