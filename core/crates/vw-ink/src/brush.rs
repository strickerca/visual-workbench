use crate::{ALGORITHM_VERSION, InkError, MAX_BASE_WIDTH, valid_coordinate};
use serde::{Deserialize, Serialize};
use vw_proto::v1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrushFamily {
    Pen,
    Marker,
    Highlighter,
    VectorEraser,
}

impl BrushFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pen => "pen",
            Self::Marker => "marker",
            Self::Highlighter => "highlighter",
            Self::VectorEraser => "vector_eraser",
        }
    }
    pub(crate) fn code(self) -> u8 {
        match self {
            Self::Pen => 1,
            Self::Marker => 2,
            Self::Highlighter => 3,
            Self::VectorEraser => 4,
        }
    }
}

impl TryFrom<&str> for BrushFamily {
    type Error = InkError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pen" => Ok(Self::Pen),
            "marker" => Ok(Self::Marker),
            "highlighter" => Ok(Self::Highlighter),
            "vector_eraser" => Ok(Self::VectorEraser),
            _ => Err(InkError::Invalid("brush family")),
        }
    }
}

/// Four (x,y) cubic Bezier control points. Both axes must be nondecreasing;
/// x endpoints are exactly 0 and 1, while y endpoints may encode a width floor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PressureCurve {
    controls: [f64; 8],
}

impl PressureCurve {
    pub fn new(controls: [f64; 8]) -> Result<Self, InkError> {
        if controls
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || controls[0] != 0.0
            || controls[6] != 1.0
            || [0, 2, 4].iter().any(|i| controls[*i] > controls[*i + 2])
            || [1, 3, 5].iter().any(|i| controls[*i] > controls[*i + 2])
        {
            return Err(InkError::Invalid("monotonic pressure curve"));
        }
        Ok(Self { controls })
    }
    pub fn linear() -> Self {
        Self {
            controls: [
                0.0,
                0.0,
                1.0 / 3.0,
                1.0 / 3.0,
                2.0 / 3.0,
                2.0 / 3.0,
                1.0,
                1.0,
            ],
        }
    }
    pub fn constant() -> Self {
        Self {
            controls: [0.0, 1.0, 1.0 / 3.0, 1.0, 2.0 / 3.0, 1.0, 1.0, 1.0],
        }
    }
    pub fn controls(&self) -> &[f64; 8] {
        &self.controls
    }
    pub fn evaluate(&self, pressure: f64) -> Result<f64, InkError> {
        if !pressure.is_finite() || !(0.0..=1.0).contains(&pressure) {
            return Err(InkError::Invalid("pressure"));
        }
        Ok(self.map(pressure))
    }
    pub(crate) fn map(&self, pressure: f64) -> f64 {
        let c = self.controls;
        if pressure == 0.0 {
            return c[1];
        }
        if pressure == 1.0 {
            return c[7];
        }
        if c[0] == c[1] && c[2] == c[3] && c[4] == c[5] && c[6] == c[7] {
            return pressure;
        }
        if c[1] == c[3] && c[3] == c[5] && c[5] == c[7] {
            return c[1];
        }
        // Fixed iterations avoid epsilon-dependent/platform-dependent branches.
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..48 {
            let mid = (low + high) * 0.5;
            if cubic(c[0], c[2], c[4], c[6], mid) < pressure {
                low = mid;
            } else {
                high = mid;
            }
        }
        cubic(c[1], c[3], c[5], c[7], (low + high) * 0.5)
    }
}
fn cubic(a: f64, b: f64, c: f64, d: f64, t: f64) -> f64 {
    let q = 1.0 - t;
    q * q * q * a + 3.0 * q * q * t * b + 3.0 * q * t * t * c + t * t * t * d
}

/// Brush values validated before a builder can be created. Algorithm v1 uses
/// circular nibs; tilt/orientation are validated metadata, not nib deformation.
#[derive(Debug, Clone, PartialEq)]
pub struct Brush {
    pub family: BrushFamily,
    pub algorithm_version: u32,
    pub base_width: f64,
    pub pressure_curve: PressureCurve,
    pub stabilization: f32,
}
impl Brush {
    pub fn new(family: BrushFamily, base_width: f64) -> Result<Self, InkError> {
        let value = Self {
            family,
            algorithm_version: ALGORITHM_VERSION,
            base_width,
            pressure_curve: if matches!(
                family,
                BrushFamily::Highlighter | BrushFamily::VectorEraser
            ) {
                PressureCurve::constant()
            } else {
                PressureCurve::linear()
            },
            stabilization: 0.0,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), InkError> {
        if self.algorithm_version != ALGORITHM_VERSION {
            return Err(InkError::UnsupportedVersion(self.algorithm_version));
        }
        if !self.base_width.is_finite()
            || self.base_width <= 0.0
            || self.base_width > MAX_BASE_WIDTH
        {
            return Err(InkError::Invalid("base width"));
        }
        if !self.stabilization.is_finite() || !(0.0..=1.0).contains(&self.stabilization) {
            return Err(InkError::Invalid("stabilization"));
        }
        PressureCurve::new(*self.pressure_curve.controls())?;
        Ok(())
    }
    pub fn to_proto(&self) -> v1::Brush {
        v1::Brush {
            family: self.family.as_str().into(),
            algorithm_version: self.algorithm_version,
            base_width: self.base_width,
            pressure_curve: self.pressure_curve.controls().to_vec(),
            stabilization: self.stabilization,
        }
    }
}
impl TryFrom<&v1::Brush> for Brush {
    type Error = InkError;
    fn try_from(value: &v1::Brush) -> Result<Self, Self::Error> {
        let controls: [f64; 8] = value
            .pressure_curve
            .as_slice()
            .try_into()
            .map_err(|_| InkError::Invalid("pressure curve size"))?;
        let brush = Self {
            family: value.family.as_str().try_into()?,
            algorithm_version: value.algorithm_version,
            base_width: value.base_width,
            pressure_curve: PressureCurve::new(controls)?,
            stabilization: value.stabilization,
        };
        brush.validate()?;
        Ok(brush)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    pub x: f64,
    pub y: f64,
    pub t_ms: u32,
    pub pressure: f64,
    #[serde(default)]
    pub tilt: Option<f64>,
    #[serde(default)]
    pub orientation: Option<f64>,
}
impl Sample {
    pub fn new(x: f64, y: f64, t_ms: u32, pressure: f64) -> Result<Self, InkError> {
        let value = Self {
            x,
            y,
            t_ms,
            pressure,
            tilt: None,
            orientation: None,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), InkError> {
        if !valid_coordinate(self.x) || !valid_coordinate(self.y) {
            return Err(InkError::Invalid("sample position"));
        }
        if !self.pressure.is_finite() || !(0.0..=1.0).contains(&self.pressure) {
            return Err(InkError::Invalid("pressure"));
        }
        if self.tilt.is_some_and(|value| {
            !value.is_finite() || !(0.0..=std::f64::consts::FRAC_PI_2 + 1e-6).contains(&value)
        }) {
            return Err(InkError::Invalid("tilt"));
        }
        if self
            .orientation
            .is_some_and(|value| !value.is_finite() || !(value as f32).is_finite())
        {
            return Err(InkError::Invalid("orientation"));
        }
        Ok(())
    }
}
