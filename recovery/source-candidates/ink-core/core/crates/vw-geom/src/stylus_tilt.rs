//! Deterministic BUILD_SPECIFICATION §4.11 projection of reported stylus axes.
//! Raw stroke axes remain unchanged. Injection requires explicit measured signs.
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// A sign learned from the owner's directional calibration traces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TiltAxisSign {
    /// Keep the mathematical component's sign.
    Direct,
    /// Reverse the mathematical component's sign.
    Inverted,
}
impl TiltAxisSign {
    fn multiplier(self) -> f64 {
        match self { Self::Direct => 1.0, Self::Inverted => -1.0 }
    }
}

/// Explicit per-device calibration, never inferred from a vendor/device name.
/// There is deliberately no Default implementation or implicit sign choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TiltCalibration {
    /// Sign that makes a pen leaning toward frame-right produce positive tiltX.
    pub x: TiltAxisSign,
    /// Sign that makes a pen leaning toward frame-bottom produce positive tiltY.
    pub y: TiltAxisSign,
}

/// Projected Windows pen angles in degrees; orientation is not barrel twist.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowsTilt {
    /// Tilt toward the frame's positive X axis, clamped to -90..90 degrees.
    pub x_degrees: f64,
    /// Tilt toward the frame's positive Y axis, clamped to -90..90 degrees.
    pub y_degrees: f64,
    /// Always zero here. A separately reported barrel-twist axis is a different capability.
    pub rotation_degrees: u32,
}

/// Missing calibration is distinct from unsupported/absent hardware axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TiltError {
    /// A supplied angle is NaN/infinite or tilt is outside the physical range.
    #[error("invalid reported stylus angle")]
    Invalid,
    /// Reported axes exist but measured frame-right/frame-bottom signs are absent.
    #[error("stylus tilt needs explicit directional calibration")]
    Uncalibrated,
}

/// Convert reported Android θ (from perpendicular) and φ (zero up, positive
/// toward screen-right). `view_to_frame_rotation` is the signed clockwise angle
/// that maps the input screen direction into the target frame, in y-down space;
/// callers must include their actual inverse camera/display transform.
///
/// Missing either hardware axis returns None, never fabricated zero axes. With
/// both axes, calibration is mandatory. The result is atan(tan(θ)*sin(φ+r)) and
/// atan(tan(θ)*cos(φ+r)), with calibrated signs. atan2 is algebraically equivalent
/// on 0..π/2 and gives finite endpoints when the pen is exactly horizontal.
/// Android Float endpoint rounding up to 1e-7 radians beyond π/2 is admitted and
/// clamped to that exact endpoint. Before addition, the exact Float encodings
/// of quarter turns in [-2π,2π] are mapped to their f64 cardinal directions.
/// This is an exact representational rule, not a near-zero dead zone; other
/// angles keep their reported direction. Raw stored samples are never modified.
pub fn windows_tilt(
    tilt: Option<f64>, orientation: Option<f64>, view_to_frame_rotation: f64,
    calibration: Option<TiltCalibration>,
) -> Result<Option<WindowsTilt>, TiltError> {
    if !view_to_frame_rotation.is_finite()
        || orientation.is_some_and(|v| !v.is_finite())
        || tilt.is_some_and(|v| !v.is_finite() || !(0.0..=FRAC_PI_2 + 1e-7).contains(&v))
    {
        return Err(TiltError::Invalid);
    }
    let (Some(theta), Some(phi)) = (tilt, orientation) else { return Ok(None); };
    let signs = calibration.ok_or(TiltError::Uncalibrated)?;
    // Reduce each angle before addition, so finite hostile inputs cannot overflow.
    let angle = normalize(reported_direction(phi) + reported_direction(view_to_frame_rotation));
    let (direction_x, direction_y) = direction(angle);
    let theta = theta.min(FRAC_PI_2);
    let (lean, upright) = if theta == FRAC_PI_2 { (1.0, 0.0) }
        else { (libm::sin(theta), libm::cos(theta)) };
    let projected = |component: f64, sign: TiltAxisSign| {
        let degrees = libm::atan2(lean * component, upright) * (180.0 / PI) * sign.multiplier();
        // Canonicalize signed zero as well as bounding Windows' accepted range.
        if degrees == 0.0 { 0.0 } else { degrees.clamp(-90.0, 90.0) }
    };
    Ok(Some(WindowsTilt {
        x_degrees: projected(direction_x, signs.x),
        y_degrees: projected(direction_y, signs.y),
        rotation_degrees: 0,
    }))
}

fn normalize(value: f64) -> f64 {
    let reduced = libm::fmod(value, TAU);
    if reduced > PI { reduced - TAU } else if reduced < -PI { reduced + TAU } else { reduced }
}
fn reported_direction(value: f64) -> f64 {
    let reduced = normalize(value);
    // MotionEvent axes are Float, as are some platform quarter-turn values.
    // At θ=π/2 a tiny cos(f32(π/2)) residue otherwise projects to ±90°, rather
    // than the cardinal component's zero. Recognize only exact encodings:
    // nearby legitimate Float directions are NOT snapped into a dead band.
    // Normalize each angle first; this also covers their finite periodic forms.
    for quarter in -4..=4 {
        let exact = f64::from(quarter) * FRAC_PI_2;
        if reduced == normalize(f64::from(exact as f32)) {
            return normalize(exact);
        }
    }
    reduced
}
fn direction(value: f64) -> (f64, f64) {
    // Exact cardinal values avoid sin(π)'s residual becoming a large angle at
    // θ=π/2. Intermediate angles continue through deterministic libm functions.
    if value == 0.0 { (0.0, 1.0) }
    else if value == FRAC_PI_2 { (1.0, 0.0) }
    else if value == -FRAC_PI_2 { (-1.0, 0.0) }
    else if value.abs() == PI { (0.0, -1.0) }
    else { (libm::sin(value), libm::cos(value)) }
}
