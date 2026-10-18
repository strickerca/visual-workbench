use crate::{PdfError, PdfResult};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub const MIB: u64 = 1024 * 1024;
pub const MAX_TILE_SIDE: u32 = 512;
pub const MAX_TEXT_CHUNK: u32 = 4096;
pub const MAX_OVERLAY_PATHS: usize = 4096;
pub const MAX_OVERLAY_POINTS: usize = 1_000_000;
pub const MAX_OVERLAY_CONTOURS: usize = 16_384;

/// Bounds our owned buffers and exposed work. PDFium's internal allocation and
/// individual non-interruptible calls additionally require an OS-bounded worker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfLimits {
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub memory_bytes: u64,
    pub pages: u32,
    pub objects_per_page: u32,
    pub annotations_per_page: u32,
    pub characters_per_page: u32,
    pub page_side_points: f64,
}

impl Default for PdfLimits {
    fn default() -> Self {
        Self {
            input_bytes: 64 * MIB,
            output_bytes: 512 * MIB,
            memory_bytes: 512 * MIB,
            pages: 1000,
            objects_per_page: 100_000,
            annotations_per_page: 4096,
            characters_per_page: 100_000,
            page_side_points: 65_535.0,
        }
    }
}

impl PdfLimits {
    pub fn validate(self) -> PdfResult<Self> {
        if self.input_bytes == 0
            || self.input_bytes > 256 * MIB
            || self.output_bytes == 0
            || self.output_bytes > 2 * 1024 * MIB
            || self.memory_bytes < 16 * MIB
            || self.memory_bytes > 2 * 1024 * MIB
            || self.pages == 0
            || self.pages > 10_000
            || self.objects_per_page == 0
            || self.objects_per_page > 1_000_000
            || self.annotations_per_page == 0
            || self.annotations_per_page > 16_384
            || self.characters_per_page == 0
            || self.characters_per_page > 1_000_000
            || !self.page_side_points.is_finite()
            || !(1.0..=65_535.0).contains(&self.page_side_points)
        {
            return Err(PdfError::Invalid);
        }
        Ok(self)
    }

    pub fn admit_input(self, length: usize, capacity: usize) -> PdfResult<()> {
        self.validate()?;
        if length < 8 || length > capacity || length as u64 > self.input_bytes {
            return Err(PdfError::Limit);
        }
        self.admit_memory(capacity as u64, 0)
    }

    pub(crate) fn admit_memory(self, retained: u64, additional: u64) -> PdfResult<()> {
        // This is an explicit wrapper-buffer allowance, not a claimed bound on
        // third-party decompression or parsing. The worker has its own hard cap.
        let estimate = retained
            .checked_add(additional)
            .and_then(|n| n.checked_add(8 * MIB))
            .ok_or(PdfError::Limit)?;
        if estimate > self.memory_bytes {
            return Err(PdfError::Memory {
                estimated: estimate,
                budget: self.memory_bytes,
            });
        }
        Ok(())
    }
}

/// Cooperative between native calls; the worker deadline is the hard timeout.
#[derive(Clone, Default)]
pub struct PdfCancellation(Arc<AtomicBool>);

impl PdfCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub(crate) fn check(&self) -> PdfResult<()> {
        if self.is_cancelled() {
            Err(PdfError::Cancelled)
        } else {
            Ok(())
        }
    }
}
