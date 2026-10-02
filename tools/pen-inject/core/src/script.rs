use crate::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Hover,
    Down,
    Move,
    Up,
    Leave,
}
impl Phase {
    pub fn name(self) -> &'static str {
        match self {
            Self::Hover => "hover",
            Self::Down => "down",
            Self::Move => "move",
            Self::Up => "up",
            Self::Leave => "leave",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub sequence: u32,
    pub stroke: u32,
    pub due_ms: u64,
    pub phase: Phase,
    pub x: i32,
    pub y: i32,
    pub pressure: u32,
    pub tilt_x: i32,
    pub tilt_y: i32,
    pub rotation: u32,
    pub pen_flags: u32,
}

/// Pure deterministic plans; creating one never opens a synthetic device.
pub fn generate(client: Rect, no_refresh: bool) -> Result<Vec<Sample>, &'static str> {
    let width = i64::from(client.right) - i64::from(client.left);
    let height = i64::from(client.bottom) - i64::from(client.top);
    if !client.valid() || width < 400 || height < 320 {
        return Err("Client must be at least 400 x 320 physical pixels");
    }
    let mut rows = Vec::new();
    let mut due_ms = 0;
    for stroke in 1..=7 {
        let count = if stroke == 6 { 61 } else { 65 };
        let y = client.top + (height * i64::from(stroke) / 8) as i32;
        let mut last_x = client.left + 24;
        for i in 0..count + 3 {
            let phase = if i == 0 || stroke == 7 && i < count + 2 {
                Phase::Hover
            } else if i == 1 {
                Phase::Down
            } else if i < count + 1 {
                Phase::Move
            } else if i == count + 1 {
                Phase::Up
            } else {
                Phase::Leave
            };
            let n = (i - 1).clamp(0, count - 1);
            let x = if matches!(phase, Phase::Up | Phase::Leave) {
                last_x
            } else {
                client.left
                    + 24
                    + if stroke == 6 {
                        0
                    } else {
                        ((width - 48) * i64::from(n) / i64::from(count - 1)) as i32
                    }
            };
            let contact = matches!(phase, Phase::Down | Phase::Move);
            let sample = Sample {
                sequence: rows.len() as u32,
                stroke,
                due_ms,
                phase,
                x,
                y,
                pressure: if !contact {
                    0
                } else if stroke == 1 {
                    (n * 1024 / (count - 1)) as u32
                } else {
                    640
                },
                tilt_x: if stroke == 2 {
                    -60 + n * 120 / (count - 1)
                } else {
                    0
                },
                tilt_y: if stroke == 2 {
                    60 - n * 120 / (count - 1)
                } else {
                    0
                },
                rotation: if stroke == 3 {
                    (n * 359 / (count - 1)) as u32
                } else {
                    0
                },
                pen_flags: if stroke == 4 && (16..48).contains(&n) {
                    1
                } else if stroke == 5 {
                    2 | 4
                } else {
                    0
                },
            };
            rows.push(sample);
            last_x = x;
            // 20 ms leaves margin for Windows scheduling jitter and the per-call guard.
            due_ms += if no_refresh && stroke == 6 && i == 1 {
                1500
            } else {
                20
            };
        }
        due_ms += 120;
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn area() -> Rect {
        Rect {
            left: -900,
            top: 100,
            right: -100,
            bottom: 700,
        }
    }
    #[test]
    fn complete_ranges_and_keepalive_inside_physical_client() -> Result<(), &'static str> {
        let rows = generate(area(), false)?;
        assert!(rows.iter().all(|s| area().contains(s.x, s.y)));
        let pressures: Vec<_> = rows
            .iter()
            .filter(|s| s.stroke == 1 && matches!(s.phase, Phase::Down | Phase::Move))
            .map(|s| s.pressure)
            .collect();
        assert_eq!(pressures.len(), 65);
        assert_eq!(pressures.first(), Some(&0));
        assert_eq!(pressures.last(), Some(&1024));
        assert_eq!(rows.iter().map(|s| s.tilt_x).min(), Some(-60));
        assert_eq!(rows.iter().map(|s| s.tilt_y).max(), Some(60));
        assert_eq!(rows.iter().map(|s| s.rotation).max(), Some(359));
        let stationary: Vec<_> = rows.iter().filter(|s| s.stroke == 6).collect();
        assert!(
            stationary
                .windows(2)
                .all(|w| w[1].due_ms - w[0].due_ms == 20 && w[0].x == w[1].x)
        );
        Ok(())
    }
    #[test]
    fn no_refresh_is_explicit_and_up_reuses_last_point() -> Result<(), &'static str> {
        for mode in [false, true] {
            let rows = generate(area(), mode)?;
            for pair in rows.windows(2) {
                if pair[1].phase == Phase::Up {
                    assert_eq!((pair[0].x, pair[0].y), (pair[1].x, pair[1].y));
                }
            }
            assert_eq!(
                rows.windows(2).any(|w| w[1].due_ms - w[0].due_ms == 1500),
                mode
            );
        }
        Ok(())
    }
    #[test]
    fn small_or_overflowing_geometry_rejected() {
        assert!(
            generate(
                Rect {
                    left: i32::MIN,
                    top: 0,
                    right: i32::MAX,
                    bottom: 500
                },
                false
            )
            .is_err()
        );
        assert!(
            generate(
                Rect {
                    left: 0,
                    top: 0,
                    right: 399,
                    bottom: 500
                },
                false
            )
            .is_err()
        );
    }
}
