//! Production per-call guard derived from the T0.05 source; no harness mutations.

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
    pub fn suspended(&self) -> bool {
        self.suspended
    }
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
