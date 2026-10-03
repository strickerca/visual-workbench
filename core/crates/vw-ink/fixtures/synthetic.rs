//! Generated numeric fixtures, not recordings of a device or an owner.
use vw_ink::{Brush, BrushFamily, InkError, PressureCurve, Sample};

pub struct Fixture {
    pub name: &'static str,
    pub brush: Brush,
    pub samples: Vec<Sample>,
}
fn sample(x: f64, y: f64, t_ms: u32, pressure: f64) -> Sample {
    Sample {
        x,
        y,
        t_ms,
        pressure,
        tilt: None,
        orientation: None,
    }
}
fn brush(family: BrushFamily, width: f64, stabilization: f32) -> Result<Brush, InkError> {
    let mut brush = Brush::new(family, width)?;
    brush.stabilization = stabilization;
    brush.validate()?;
    Ok(brush)
}
pub fn fixtures() -> Result<Vec<Fixture>, InkError> {
    let mut output = vec![
        Fixture {
            name: "synthetic_dot",
            brush: brush(BrushFamily::Pen, 8.0, 0.0)?,
            samples: vec![sample(20.25, 11.5, 0, 1.0)],
        },
        Fixture {
            name: "synthetic_straight",
            brush: brush(BrushFamily::Pen, 4.0, 0.0)?,
            samples: (0..8)
                .map(|i| sample(f64::from(i) * 4.0, 8.0, i * 8, 0.5))
                .collect(),
        },
        Fixture {
            name: "synthetic_pressure",
            brush: brush(BrushFamily::Pen, 12.0, 0.0)?,
            samples: [0.0, 0.125, 0.25, 0.5, 0.75, 1.0, 0.75, 0.5, 0.25, 0.0]
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    sample(
                        i as f64 * 3.5,
                        ((i % 3) as f64 - 1.0) * 2.0,
                        i as u32 * 4,
                        *p,
                    )
                })
                .collect(),
        },
        Fixture {
            name: "synthetic_circle",
            brush: brush(BrushFamily::Pen, 5.0, 0.5)?,
            samples: (0..=64)
                .map(|i| {
                    let (s, c) = libm::sincos(2.0 * std::f64::consts::PI * f64::from(i) / 64.0);
                    sample(40.0 + 25.0 * c, 40.0 + 25.0 * s, i * 4, 0.75)
                })
                .collect(),
        },
        Fixture {
            name: "synthetic_handwriting",
            brush: brush(BrushFamily::Pen, 4.0, 0.25)?,
            samples: [
                (0, 20),
                (0, 8),
                (1, 6),
                (2, 20),
                (7, 20),
                (7, 8),
                (8, 6),
                (10, 8),
                (10, 20),
                (10, 10),
                (13, 6),
                (16, 10),
                (16, 20),
                (21, 20),
                (21, 4),
                (21, 12),
                (27, 6),
                (21, 12),
                (28, 20),
            ]
            .iter()
            .enumerate()
            .map(|(i, (x, y))| {
                sample(
                    f64::from(*x),
                    f64::from(*y),
                    i as u32 * 6,
                    0.5 + (i % 3) as f64 / 8.0,
                )
            })
            .collect(),
        },
        Fixture {
            name: "synthetic_fast_line",
            brush: brush(BrushFamily::Pen, 3.0, 0.25)?,
            samples: (0..128)
                .map(|i| sample(f64::from(i) * 6.0, f64::from(i % 7) * 0.25, i, 0.75))
                .collect(),
        },
        Fixture {
            name: "synthetic_stationary_pressure",
            brush: brush(BrushFamily::Pen, 16.0, 0.0)?,
            samples: [0.0, 0.25, 0.5, 1.0, 0.5, 0.25, 0.0]
                .iter()
                .enumerate()
                .map(|(i, p)| sample(12.0, 12.0, i as u32 * 10, *p))
                .collect(),
        },
        Fixture {
            name: "synthetic_equal_timestamps",
            brush: brush(BrushFamily::Pen, 6.0, 0.75)?,
            samples: vec![
                sample(0.0, 0.0, 0, 0.2),
                sample(4.0, 0.0, 0, 0.4),
                sample(8.0, 1.0, 0, 0.8),
                sample(12.0, 1.0, 1, 1.0),
            ],
        },
        Fixture {
            name: "synthetic_negative_fractional",
            brush: brush(BrushFamily::Pen, 1.25, 0.0)?,
            samples: (0..12)
                .map(|i| {
                    sample(
                        -10.125 + f64::from(i) / 7.0,
                        -4.0625 + f64::from(i % 2) / 128.0,
                        i * 3,
                        1.0 / f64::from(i + 2),
                    )
                })
                .collect(),
        },
        Fixture {
            name: "synthetic_highlighter_flat",
            brush: brush(BrushFamily::Highlighter, 12.0, 0.0)?,
            samples: [-10.0, 0.0, 15.0, 30.0]
                .iter()
                .enumerate()
                .map(|(i, x)| sample(*x, 0.0, i as u32 * 12, 0.5))
                .collect(),
        },
        Fixture {
            name: "synthetic_highlighter_corner",
            brush: brush(BrushFamily::Highlighter, 6.0, 0.0)?,
            samples: vec![
                sample(0.0, 0.0, 0, 1.0),
                sample(10.0, 0.0, 10, 1.0),
                sample(10.0, 10.0, 20, 1.0),
            ],
        },
        Fixture {
            name: "synthetic_vector_eraser",
            brush: brush(BrushFamily::VectorEraser, 10.0, 0.0)?,
            samples: vec![
                sample(0.0, 0.0, 0, 0.0),
                sample(25.0, 10.0, 10, 0.5),
                sample(50.0, 0.0, 20, 1.0),
            ],
        },
        Fixture {
            name: "synthetic_large_coordinates",
            brush: brush(BrushFamily::Pen, 8.0, 0.0)?,
            samples: vec![
                sample(999_999_980.125, -999_999_980.25, 0, 1.0),
                sample(999_999_988.0, -999_999_975.0, 5, 0.75),
            ],
        },
        Fixture {
            name: "synthetic_zero_pressure",
            brush: brush(BrushFamily::Pen, 8.0, 0.0)?,
            samples: vec![sample(0.0, 0.0, 0, 0.0), sample(10.0, 10.0, 4, 0.0)],
        },
        Fixture {
            name: "synthetic_quantization_edge",
            brush: brush(BrushFamily::Pen, 0.03125, 0.0)?,
            samples: vec![
                sample(-0.001953125, 0.001953125, 0, 1.0),
                sample(0.009765625, 0.005859375, 1, 1.0),
                sample(0.017578125, 0.0, 2, 0.5),
            ],
        },
    ];
    let mut marker = brush(BrushFamily::Marker, 9.0, 0.5)?;
    marker.pressure_curve = PressureCurve::new([0.0, 0.2, 0.15, 0.35, 0.8, 0.75, 1.0, 1.0])?;
    output.push(Fixture {
        name: "synthetic_marker_curve",
        brush: marker,
        samples: (0..20)
            .map(|i| {
                sample(
                    f64::from(i) * 3.0,
                    f64::from(i % 2) * 10.0,
                    i * 4,
                    f64::from(i) / 19.0,
                )
            })
            .collect(),
    });
    Ok(output)
}
