//! Pointer-free synchronous projection only; no I/O, settings, or injection.
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum StylusTiltSign {
    Direct,
    Inverted,
}
#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct StylusTiltCalibration {
    pub x: StylusTiltSign,
    pub y: StylusTiltSign,
}
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ProjectedStylusTilt {
    pub x_degrees: f64,
    pub y_degrees: f64,
    pub rotation_degrees: u32,
}
#[derive(Clone, Debug, thiserror::Error, uniffi::Error)]
pub enum StylusTiltError {
    #[error("invalid reported stylus angle")]
    Invalid,
    #[error("directional stylus calibration is required")]
    Uncalibrated,
}
impl StylusTiltSign {
    fn geometry(self) -> vw_geom::TiltAxisSign {
        match self {
            Self::Direct => vw_geom::TiltAxisSign::Direct,
            Self::Inverted => vw_geom::TiltAxisSign::Inverted,
        }
    }
}
#[uniffi::export]
pub fn project_stylus_tilt(
    tilt: Option<f64>,
    orientation: Option<f64>,
    view_to_frame_rotation: f64,
    calibration: Option<StylusTiltCalibration>,
) -> std::result::Result<Option<ProjectedStylusTilt>, StylusTiltError> {
    vw_geom::windows_tilt(
        tilt,
        orientation,
        view_to_frame_rotation,
        calibration.map(|value| vw_geom::TiltCalibration {
            x: value.x.geometry(),
            y: value.y.geometry(),
        }),
    )
    .map(|value| {
        value.map(|value| ProjectedStylusTilt {
            x_degrees: value.x_degrees,
            y_degrees: value.y_degrees,
            rotation_degrees: value.rotation_degrees,
        })
    })
    .map_err(|error| match error {
        vw_geom::TiltError::Invalid => StylusTiltError::Invalid,
        vw_geom::TiltError::Uncalibrated => StylusTiltError::Uncalibrated,
    })
}
