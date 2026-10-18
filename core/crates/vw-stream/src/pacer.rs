use crate::{Result, StreamError};

/// One capture/encode in flight; no frame FIFO. Feedback adapts JPEG quality
/// before the capture interval. Requested fps and observed output are separate.
/// A transport drop always requires a fresh keyframe, never a broken delta.
pub struct StreamPacer {
    target_fps: u32,
    interval_ms: u64,
    last_start: Option<u64>,
    quality: u8,
    good: u32,
    in_flight: bool,
    requested: u64,
    sent: u64,
    started_request: u64,
}
impl StreamPacer {
    pub fn new(target_fps: u32) -> Result<Self> {
        if !(15..=60).contains(&target_fps) {
            return Err(StreamError::Invalid);
        }
        Ok(Self {
            target_fps,
            interval_ms: 1000u64.div_ceil(u64::from(target_fps)),
            last_start: None,
            quality: 85,
            good: 0,
            in_flight: false,
            requested: 1,
            sent: 0,
            started_request: 0,
        })
    }
    pub fn requested_fps(&self) -> u32 {
        self.target_fps
    }
    pub fn interval_ms(&self) -> u64 {
        self.interval_ms
    }
    pub fn quality(&self) -> u8 {
        self.quality
    }
    pub fn can_begin(&self, now_ms: u64) -> bool {
        !self.in_flight
            && self
                .last_start
                .is_none_or(|last| now_ms.saturating_sub(last) >= self.interval_ms)
    }
    pub fn keyframe_pending(&self) -> bool {
        self.requested != self.sent
    }
    pub fn begin(&mut self, now_ms: u64) -> Option<bool> {
        if self.in_flight
            || self
                .last_start
                .is_some_and(|last| now_ms.saturating_sub(last) < self.interval_ms)
        {
            return None;
        }
        self.in_flight = true;
        self.last_start = Some(now_ms);
        self.started_request = self.requested;
        Some(self.requested != self.sent)
    }
    pub fn complete(&mut self, elapsed_ms: u64, admitted_to_media: bool) -> Result<()> {
        if !self.in_flight || elapsed_ms > 60_000 {
            return Err(StreamError::Invalid);
        }
        self.in_flight = false;
        if admitted_to_media && self.requested != u64::MAX {
            self.sent = self.started_request;
        } else {
            self.keyframe_required();
        }
        let minimum = 1000u64.div_ceil(u64::from(self.target_fps));
        if !admitted_to_media || elapsed_ms > self.interval_ms {
            self.good = 0;
            if self.quality > 45 {
                self.quality -= 5;
            } else {
                self.interval_ms = self
                    .interval_ms
                    .max(elapsed_ms)
                    .saturating_add(5)
                    .min(1_000);
            }
        } else {
            self.good = self.good.saturating_add(1);
            if self.good >= 60 {
                self.good = 0;
                if self.interval_ms > minimum {
                    self.interval_ms = (self.interval_ms - 5).max(minimum);
                } else {
                    self.quality = (self.quality + 5).min(85);
                }
            }
        }
        Ok(())
    }
    pub fn keyframe_required(&mut self) {
        // Exhaustion keeps the requirement asserted forever; no wrap to stale.
        if let Some(next) = self.requested.checked_add(1) {
            self.requested = next;
        } else {
            self.sent = 0;
            self.started_request = 0;
        }
    }
    pub fn cancelled(&mut self) {
        self.in_flight = false;
        self.keyframe_required();
    }
}
