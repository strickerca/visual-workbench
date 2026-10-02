//! T0.05 probe policy. Pure guard decisions are separate from Win32 injection.
#[cfg(windows)]
pub mod platform;
pub mod script;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    pub fn valid(self) -> bool {
        (1..=32768).contains(&(i64::from(self.right) - i64::from(self.left)))
            && (1..=32768).contains(&(i64::from(self.bottom) - i64::from(self.top)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub hwnd: usize,
    pub pid: u32,
    pub thread: u32,
    pub process_created: u64,
    pub window: Rect,
    pub client: Rect,
    pub dpi: u32,
    pub integrity: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    Suspended,
    InvalidTarget,
    TargetChanged,
    ForegroundChanged,
    SessionChanged,
    HigherIntegrity,
    OutsideClient,
    Occluded,
}

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Input paused: {self:?}; explicitly start a new session to resume"
        )
    }
}
impl std::error::Error for Stop {}

pub struct Guard {
    target: Target,
    session: u64,
    caller_integrity: u32,
    suspended: bool,
}

impl Guard {
    pub fn new(target: Target, session: u64, caller_integrity: u32) -> Result<Self, Stop> {
        if target.hwnd == 0
            || target.pid == 0
            || target.thread == 0
            || target.process_created == 0
            || target.dpi == 0
            || session == 0
            || !target.window.valid()
            || !target.client.valid()
            || !target
                .window
                .contains(target.client.left, target.client.top)
            || !target
                .window
                .contains(target.client.right - 1, target.client.bottom - 1)
        {
            return Err(Stop::InvalidTarget);
        }
        if target.integrity > caller_integrity {
            return Err(Stop::HigherIntegrity);
        }
        Ok(Self {
            target,
            session,
            caller_integrity,
            suspended: false,
        })
    }

    /// Any rejection permanently suspends this session, including an OS read failure.
    pub fn suspend(&mut self) {
        self.suspended = true;
    }

    pub fn check(
        &mut self,
        current: Target,
        foreground: usize,
        hit_root: usize,
        session: u64,
        point: (i32, i32),
    ) -> Result<(), Stop> {
        let reason = if self.suspended {
            Some(Stop::Suspended)
        } else if session != self.session {
            Some(Stop::SessionChanged)
        } else if current.integrity > self.caller_integrity {
            Some(Stop::HigherIntegrity)
        } else if current != self.target {
            Some(Stop::TargetChanged)
        } else if foreground != self.target.hwnd {
            Some(Stop::ForegroundChanged)
        } else if !self.target.client.contains(point.0, point.1) {
            Some(Stop::OutsideClient)
        } else if hit_root != self.target.hwnd {
            Some(Stop::Occluded)
        } else {
            None
        };
        if let Some(reason) = reason {
            self.suspend();
            Err(reason)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> Target {
        Target {
            hwnd: 1,
            pid: 2,
            thread: 3,
            process_created: 4,
            window: Rect {
                left: -1200,
                top: 0,
                right: -200,
                bottom: 800,
            },
            client: Rect {
                left: -1190,
                top: 40,
                right: -210,
                bottom: 790,
            },
            dpi: 168,
            integrity: 8192,
        }
    }
    #[test]
    fn negative_physical_coordinates_and_exclusive_edges() -> Result<(), Stop> {
        let t = target();
        let mut g = Guard::new(t, 9, 8192)?;
        assert!(g.check(t, 1, 1, 9, (-1190, 40)).is_ok());
        assert_eq!(g.check(t, 1, 1, 9, (-210, 40)), Err(Stop::OutsideClient));
        Ok(())
    }
    #[test]
    fn guard_failure_latches_even_after_focus_returns() -> Result<(), Stop> {
        let t = target();
        let mut g = Guard::new(t, 9, 8192)?;
        assert_eq!(
            g.check(t, 5, 1, 9, (-600, 400)),
            Err(Stop::ForegroundChanged)
        );
        assert_eq!(g.check(t, 1, 1, 9, (-600, 400)), Err(Stop::Suspended));
        Ok(())
    }
    #[test]
    fn geometry_identity_dpi_and_integrity_all_fail_closed() -> Result<(), Stop> {
        let t = target();
        for field in 0..9 {
            let mut changed = t;
            match field {
                0 => changed.window.left -= 1,
                1 => changed.client.right -= 1,
                2 => changed.dpi += 1,
                3 => changed.pid += 1,
                4 => changed.thread += 1,
                5 => changed.process_created += 1,
                6 => changed.hwnd += 1,
                7 => changed.integrity += 1,
                _ => changed.client.bottom -= 1,
            }
            let mut guard = Guard::new(t, 9, 8192)?;
            assert!(guard.check(changed, 1, 1, 9, (-600, 400)).is_err());
        }
        Ok(())
    }
    #[test]
    fn session_occlusion_and_read_failure_block() -> Result<(), Stop> {
        let t = target();
        assert_eq!(
            Guard::new(t, 9, 8192)?.check(t, 1, 1, 10, (-600, 400)),
            Err(Stop::SessionChanged)
        );
        assert_eq!(
            Guard::new(t, 9, 8192)?.check(t, 1, 6, 9, (-600, 400)),
            Err(Stop::Occluded)
        );
        let mut g = Guard::new(t, 9, 8192)?;
        g.suspend();
        assert_eq!(g.check(t, 1, 1, 9, (-600, 400)), Err(Stop::Suspended));
        Ok(())
    }
    #[test]
    fn invalid_and_higher_integrity_targets_cannot_arm() {
        let mut t = target();
        assert!(matches!(Guard::new(t, 1, 4096), Err(Stop::HigherIntegrity)));
        t.client.right = t.client.left;
        assert!(matches!(Guard::new(t, 1, 8192), Err(Stop::InvalidTarget)));
    }
}
