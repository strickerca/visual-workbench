use crate::{MAX_BUFFER_BYTES, MAX_RADIUS, MAX_WORK, Mask, MaskError, Result, filled, next};
use std::collections::VecDeque;

impl Mask {
    /// Check the version, work and buffer limits shared by feather, expansion
    /// and erosion without allocating or changing the mask. Passing admission
    /// does not guarantee that a later allocator request will succeed.
    pub fn admit_morphology(&self, radius: u32) -> Result<()> {
        next(self.version())?;
        check(self, radius)
    }

    /// Grayscale dilation: maximum coverage in the exact integer Euclidean disk.
    pub fn expand(&self, radius: u32) -> Result<Self> {
        self.extreme(radius, true)
    }
    /// Grayscale erosion: minimum coverage in the exact integer Euclidean disk.
    /// Off-document coverage is zero, so the document edge erodes like any edge.
    pub fn shrink(&self, radius: u32) -> Result<Self> {
        self.extreme(radius, false)
    }

    /// A normalized, finite uniform disk kernel. Integer sums are divided once,
    /// with nearest rounding. Off-document coverage is zero and the denominator
    /// is never renormalized at the document edge. Radius zero is a new version.
    /// This kernel has no tails outside `expand(radius)`'s support.
    pub fn feather(&self, radius: u32) -> Result<Self> {
        self.admit_morphology(radius)?;
        let version = next(self.version())?;
        if radius == 0 {
            let mut result = self.clone();
            result.version = version;
            return Ok(result);
        }
        if self.tile_count() == 0 {
            let mut result = Self::empty(self.size());
            result.version = version;
            return Ok(result);
        }
        let disk = disk_rows(radius);
        let divisor: u64 = disk.iter().map(|(_, half)| u64::from(*half) * 2 + 1).sum();
        let source = self.to_dense()?;
        let mut output = filled(source.len(), 0u8)?;
        let width = self.size().width() as usize;
        let height = self.size().height() as usize;
        let mut sums = filled(width, 0u64)?;
        let mut prefix = filled(width + 1, 0u64)?;
        for (y, target) in output.chunks_exact_mut(width).enumerate() {
            sums.fill(0);
            for &(dy, half) in &disk {
                let sy = y as i64 + i64::from(dy);
                if sy < 0 || sy >= height as i64 {
                    continue;
                }
                let row = &source[sy as usize * width..(sy as usize + 1) * width];
                prefix[0] = 0;
                for (x, &pixel) in row.iter().enumerate() {
                    prefix[x + 1] = prefix[x] + u64::from(pixel);
                }
                for (x, sum) in sums.iter_mut().enumerate() {
                    let left = x.saturating_sub(half as usize);
                    let right = (x + half as usize + 1).min(width);
                    *sum += prefix[right] - prefix[left];
                }
            }
            for (pixel, sum) in target.iter_mut().zip(&sums) {
                *pixel = ((*sum + divisor / 2) / divisor) as u8;
            }
        }
        Self::dense_version(self.size(), &output, version)
    }

    fn extreme(&self, radius: u32, grow: bool) -> Result<Self> {
        self.admit_morphology(radius)?;
        let version = next(self.version())?;
        if radius == 0 {
            let mut result = self.clone();
            result.version = version;
            return Ok(result);
        }
        if self.tile_count() == 0 {
            let mut result = Self::empty(self.size());
            result.version = version;
            return Ok(result);
        }
        let source = self.to_dense()?;
        let mut output = filled(source.len(), if grow { 0u8 } else { 255u8 })?;
        let width = self.size().width() as usize;
        let height = self.size().height() as usize;
        let mut row = filled(width, 0u8)?;
        let mut deque = VecDeque::new();
        deque
            .try_reserve(width)
            .map_err(|_| MaskError::Allocation)?;
        for (dy, half) in disk_rows(radius) {
            for (y, target) in output.chunks_exact_mut(width).enumerate() {
                let sy = y as i64 + i64::from(dy);
                if sy < 0 || sy >= height as i64 {
                    if !grow {
                        target.fill(0);
                    }
                    continue;
                }
                horizontal_extreme(
                    &source[sy as usize * width..(sy as usize + 1) * width],
                    &mut row,
                    half as usize,
                    grow,
                    &mut deque,
                );
                for (a, &b) in target.iter_mut().zip(&row) {
                    *a = if grow { (*a).max(b) } else { (*a).min(b) };
                }
            }
        }
        Self::dense_version(self.size(), &output, version)
    }
}
fn check(mask: &Mask, radius: u32) -> Result<()> {
    if radius > MAX_RADIUS {
        return Err(MaskError::Limit);
    }
    if radius == 0 || mask.tile_count() == 0 {
        return Ok(());
    }
    let pixels = mask.size().pixels() as u64;
    // Three linear passes per disk row conservatively charge prefix/deque work.
    if pixels * (u64::from(radius) * 2 + 1) * 3 > MAX_WORK {
        return Err(MaskError::Limit);
    }
    // Dense source/output plus output tiles and the largest two row scratchpads.
    if pixels * 3 + u64::from(mask.size().width()) * 17 + 8 > MAX_BUFFER_BYTES as u64 {
        return Err(MaskError::Limit);
    }
    Ok(())
}
fn disk_rows(radius: u32) -> Vec<(i32, u32)> {
    let mut result = Vec::with_capacity(radius as usize * 2 + 1);
    let r = radius as i32;
    for dy in -r..=r {
        let mut half = radius;
        while half * half + (dy * dy) as u32 > radius * radius {
            half -= 1;
        }
        result.push((dy, half));
    }
    result
}
fn horizontal_extreme(
    source: &[u8],
    output: &mut [u8],
    radius: usize,
    grow: bool,
    deque: &mut VecDeque<usize>,
) {
    deque.clear();
    let mut end = 0;
    for (x, result) in output.iter_mut().enumerate() {
        let right = (x + radius + 1).min(source.len());
        while end < right {
            while deque.back().is_some_and(|&index| {
                if grow {
                    source[index] <= source[end]
                } else {
                    source[index] >= source[end]
                }
            }) {
                deque.pop_back();
            }
            deque.push_back(end);
            end += 1;
        }
        let left = x.saturating_sub(radius);
        while deque.front().is_some_and(|&index| index < left) {
            deque.pop_front();
        }
        *result = if !grow && (x < radius || x + radius >= source.len()) {
            0
        } else {
            deque.front().map_or(0, |&index| source[index])
        };
    }
}
