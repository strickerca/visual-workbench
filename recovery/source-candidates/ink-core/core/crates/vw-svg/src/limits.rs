//! Admission shared by the worker engine and its protocol adapter.
use crate::{SvgError, SvgResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub nodes: u32,
    pub depth: u16,
    pub expanded_bytes: u64,
    pub memory_bytes: u64,
    pub document_side: u32,
    pub tile_side: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            input_bytes: 4 * 1024 * 1024,
            output_bytes: 4 * 1024 * 1024,
            nodes: 100_000,
            depth: 64,
            expanded_bytes: 16 * 1024 * 1024,
            memory_bytes: 512 * 1024 * 1024,
            document_side: 65_535,
            tile_side: 512,
        }
    }
}

impl Limits {
    pub fn validate(self) -> SvgResult<()> {
        let maximum = Self::default();
        if self.input_bytes == 0 || self.input_bytes > maximum.input_bytes
            || self.output_bytes == 0 || self.output_bytes > maximum.output_bytes
            || self.nodes == 0 || self.nodes > maximum.nodes
            || self.depth == 0 || self.depth > maximum.depth
            || self.expanded_bytes == 0 || self.expanded_bytes > maximum.expanded_bytes
            || self.memory_bytes == 0 || self.memory_bytes > 2 * 1024 * 1024 * 1024
            || self.document_side == 0 || self.document_side > maximum.document_side
            || self.tile_side == 0 || self.tile_side > maximum.tile_side
        {
            return Err(SvgError::InvalidLimits);
        }
        Ok(())
    }

    /// Includes the borrowed source, both DOM representations, resolved geometry,
    /// trusted font workspace, and serialization scratch. It is an admission
    /// reservation, not an allocator interposition: the PC worker also MUST apply
    /// its process memory and wall-clock limits around the third-party parser.
    pub(crate) fn parse_memory(self, raw: usize, expanded: u64) -> SvgResult<u64> {
        let bytes = 32_u64.checked_mul(raw as u64)
            .and_then(|v| v.checked_add(expanded.checked_mul(128)?))
            .and_then(|v| v.checked_add(32 * 1024 * 1024))
            .and_then(|v| v.checked_add(self.output_bytes as u64 * 2))
            .ok_or(SvgError::Limit)?;
        self.admit(bytes)?;
        Ok(bytes)
    }

    pub(crate) fn admit(self, bytes: u64) -> SvgResult<()> {
        if bytes > self.memory_bytes {
            return Err(SvgError::Memory { required: bytes, budget: self.memory_bytes });
        }
        Ok(())
    }
}

/// Explicit raster window at view scale; this never changes document dimensions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileRequest {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
    pub width: u32,
    pub height: u32,
}

impl TileRequest {
    pub(crate) fn validate(self, limits: Limits) -> SvgResult<()> {
        if !self.x.is_finite() || !self.y.is_finite() || !self.scale.is_finite()
            || self.x.abs() > 1e9 || self.y.abs() > 1e9
            || !(1.0 / 4096.0..=4096.0).contains(&self.scale)
            || self.width == 0 || self.height == 0
            || self.width > limits.tile_side || self.height > limits.tile_side
        {
            return Err(SvgError::InvalidTile);
        }
        Ok(())
    }
}
