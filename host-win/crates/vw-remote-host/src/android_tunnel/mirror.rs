use super::{Error, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysicalRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PhysicalRect {
    pub fn valid(self) -> bool {
        self.left < self.right && self.top < self.bottom
    }

    pub fn intersects(self, other: Self) -> bool {
        self.valid()
            && other.valid()
            && self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }
}

/// Root supplies a selected native handle; the native probe rereads current
/// geometry and visible owned windows immediately before use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureSource {
    Window { hwnd: isize },
    Monitor { monitor: isize },
    Region { monitor: isize, rect: PhysicalRect },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MirrorWindow {
    pub hwnd: isize,
    pub rect: PhysicalRect,
    pub visible: bool,
    pub child_of_source: bool,
    pub source_child_of_mirror: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    pub selected: PhysicalRect,
    pub mirrors: Vec<MirrorWindow>,
    pub complete: bool,
}

pub(crate) fn check(snapshot: &Snapshot) -> Result<()> {
    if !snapshot.complete || !snapshot.selected.valid() || snapshot.mirrors.is_empty() {
        // An empty mirror census cannot prove current ownership after launch.
        return Err(Error::Platform);
    }
    for mirror in &snapshot.mirrors {
        if mirror.hwnd == 0 || !mirror.rect.valid() {
            return Err(Error::Platform);
        }
        if mirror.visible
            && (mirror.child_of_source
                || mirror.source_child_of_mirror
                || snapshot.selected.intersects(mirror.rect))
        {
            return Err(Error::FeedbackLoop);
        }
    }
    Ok(())
}

pub(crate) trait WindowProbe: Send {
    fn current(&mut self, source: CaptureSource, owned_scrcpy_pid: u32) -> Result<Snapshot>;
}
