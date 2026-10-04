//! Immutable document-resolution selection coverage. See the crate README for
//! the versioned coverage, morphology, boundary and serialization contracts.
#![forbid(unsafe_code)]

mod coverage;
mod export;
mod morphology;

pub use export::{Cutout8, Cutout16};
use std::{collections::BTreeMap, sync::Arc};
use thiserror::Error;

pub const ALGORITHM_VERSION: u32 = 1;
pub const TILE_SIDE: u32 = 256;
pub const MAX_PIXELS: u64 = 50_000_000;
pub const MAX_SIDE: u32 = 1_000_000;
pub const MAX_PATH_POINTS: usize = 16_384;
pub const MAX_RADIUS: u32 = 256;
pub const MAX_WORK: u64 = 1_000_000_000;
pub const MAX_BUFFER_BYTES: usize = 256 * 1024 * 1024;
const MAX_COORDINATE: f64 = 8_388_608.0;
const MAGIC: &[u8; 8] = b"VWMASK01";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MaskError {
    #[error("invalid mask dimensions or geometry")]
    Invalid,
    #[error("mask dimensions do not match")]
    SizeMismatch,
    #[error("mask resource or work limit exceeded")]
    Limit,
    #[error("mask version counter exhausted")]
    VersionExhausted,
    #[error("invalid or non-canonical mask data")]
    Corrupt,
    #[error("mask allocation failed")]
    Allocation,
    #[error("mask PNG encoding failed")]
    Encoding,
}
pub type Result<T> = std::result::Result<T, MaskError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    width: u32,
    height: u32,
}
impl Size {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(MaskError::Invalid);
        }
        if u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err(MaskError::Limit);
        }
        Ok(Self { width, height })
    }
    pub fn width(self) -> u32 {
        self.width
    }
    pub fn height(self) -> u32 {
        self.height
    }
    pub fn pixels(self) -> usize {
        (u64::from(self.width) * u64::from(self.height)) as usize
    }
    fn tile_size(self, coord: TileCoord) -> Result<Self> {
        let x = coord.x.checked_mul(TILE_SIDE).ok_or(MaskError::Invalid)?;
        let y = coord.y.checked_mul(TILE_SIDE).ok_or(MaskError::Invalid)?;
        if x >= self.width || y >= self.height {
            return Err(MaskError::Invalid);
        }
        Ok(Self {
            width: TILE_SIDE.min(self.width - x),
            height: TILE_SIDE.min(self.height - y),
        })
    }
}

/// Continuous D-space edges, with x right, y down. Pixel (i,j) has center
/// (i+0.5,j+0.5). Coordinates are quantized to the nearest 1/256 document pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A half-open integer crop in the oriented document pixel grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl Region {
    pub fn full(size: Size) -> Self {
        Self {
            x: 0,
            y: 0,
            width: size.width,
            height: size.height,
        }
    }
    pub fn size(self, document: Size) -> Result<Size> {
        let size = Size::new(self.width, self.height)?;
        if self
            .x
            .checked_add(self.width)
            .is_none_or(|v| v > document.width)
            || self
                .y
                .checked_add(self.height)
                .is_none_or(|v| v > document.height)
        {
            return Err(MaskError::Invalid);
        }
        Ok(size)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileCoord {
    pub x: u32,
    pub y: u32,
}
impl Ord for TileCoord {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.y, self.x).cmp(&(other.y, other.x))
    }
}
impl PartialOrd for TileCoord {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileData {
    pub coord: TileCoord,
    pub pixels: Vec<u8>,
}
#[derive(Debug, Clone, Copy)]
pub struct TileView<'a> {
    pub coord: TileCoord,
    pub size: Size,
    pub pixels: &'a [u8],
}

/// Clones share immutable tile bytes. Every operation returns a separate mask;
/// callers retain previous versions in the project transaction/history layer.
#[derive(Debug, Clone)]
pub struct Mask {
    size: Size,
    version: u64,
    tiles: BTreeMap<TileCoord, Arc<[u8]>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combine {
    Add,
    Subtract,
    Intersect,
}

impl Mask {
    pub fn empty(size: Size) -> Self {
        Self {
            size,
            version: 0,
            tiles: BTreeMap::new(),
        }
    }
    pub fn size(&self) -> Size {
        self.size
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }
    pub fn tiles(&self) -> impl Iterator<Item = TileView<'_>> {
        self.tiles.iter().map(|(&coord, bytes)| TileView {
            coord,
            size: Size {
                width: TILE_SIDE.min(self.size.width - coord.x * TILE_SIDE),
                height: TILE_SIDE.min(self.size.height - coord.y * TILE_SIDE),
            },
            pixels: bytes,
        })
    }
    /// An out-of-document lookup is zero. This is also the filtering border rule.
    pub fn coverage(&self, x: u32, y: u32) -> u8 {
        if x >= self.size.width || y >= self.size.height {
            return 0;
        }
        let coord = TileCoord {
            x: x / TILE_SIDE,
            y: y / TILE_SIDE,
        };
        let stride = TILE_SIDE.min(self.size.width - coord.x * TILE_SIDE);
        self.tiles.get(&coord).map_or(0, |p| {
            p[((y % TILE_SIDE) * stride + x % TILE_SIDE) as usize]
        })
    }
    pub fn from_dense(size: Size, pixels: &[u8]) -> Result<Self> {
        Self::dense_version(size, pixels, 0)
    }
    pub(crate) fn dense_version(size: Size, pixels: &[u8], version: u64) -> Result<Self> {
        if pixels.len() != size.pixels() {
            return Err(MaskError::Invalid);
        }
        let mut builder = Builder::new(size);
        for (y, row) in pixels.chunks_exact(size.width as usize).enumerate() {
            builder.row(0, y as u32, row)?;
        }
        Ok(builder.finish(version))
    }
    /// Edge tiles contain only their in-document rectangle. Zero tiles, duplicate
    /// coordinates, wrong lengths and tiles outside the document are rejected.
    pub fn from_tiles(size: Size, version: u64, tiles: Vec<TileData>) -> Result<Self> {
        let maximum =
            u64::from(size.width.div_ceil(TILE_SIDE)) * u64::from(size.height.div_ceil(TILE_SIDE));
        if tiles.len() as u64 > maximum {
            return Err(MaskError::Corrupt);
        }
        let mut result = Self {
            size,
            version,
            tiles: BTreeMap::new(),
        };
        for tile in tiles {
            let expected = size.tile_size(tile.coord).map_err(|_| MaskError::Corrupt)?;
            if tile.pixels.len() != expected.pixels()
                || !tile.pixels.iter().any(|&v| v != 0)
                || result.tiles.contains_key(&tile.coord)
            {
                return Err(MaskError::Corrupt);
            }
            result.tiles.insert(tile.coord, Arc::from(tile.pixels));
        }
        Ok(result)
    }
    pub fn to_dense(&self) -> Result<Vec<u8>> {
        self.crop(Region::full(self.size))
    }
    pub fn crop(&self, region: Region) -> Result<Vec<u8>> {
        let size = region.size(self.size)?;
        let mut result = filled(size.pixels(), 0u8)?;
        for tile in self.tiles() {
            let x = tile.coord.x * TILE_SIDE;
            let y = tile.coord.y * TILE_SIDE;
            let left = x.max(region.x);
            let right = (x + tile.size.width).min(region.x + region.width);
            let top = y.max(region.y);
            let bottom = (y + tile.size.height).min(region.y + region.height);
            if left >= right || top >= bottom {
                continue;
            }
            for row in top..bottom {
                let source = ((row - y) * tile.size.width + left - x) as usize;
                let target = ((row - region.y) * region.width + left - region.x) as usize;
                let length = (right - left) as usize;
                result[target..target + length]
                    .copy_from_slice(&tile.pixels[source..source + length]);
            }
        }
        Ok(result)
    }
    pub fn bounds(&self) -> Option<Region> {
        let mut left = self.size.width;
        let mut top = self.size.height;
        let mut right = 0;
        let mut bottom = 0;
        for tile in self.tiles() {
            for (i, value) in tile.pixels.iter().enumerate() {
                if *value != 0 {
                    let x = tile.coord.x * TILE_SIDE + i as u32 % tile.size.width;
                    let y = tile.coord.y * TILE_SIDE + i as u32 / tile.size.width;
                    left = left.min(x);
                    top = top.min(y);
                    right = right.max(x + 1);
                    bottom = bottom.max(y + 1);
                }
            }
        }
        (right > left && bottom > top).then_some(Region {
            x: left,
            y: top,
            width: right.saturating_sub(left),
            height: bottom.saturating_sub(top),
        })
    }
    pub fn combine(&self, other: &Self, operation: Combine) -> Result<Self> {
        if self.size != other.size {
            return Err(MaskError::SizeMismatch);
        }
        let version = next(self.version.max(other.version))?;
        let mut result = Self {
            size: self.size,
            version,
            tiles: BTreeMap::new(),
        };
        let mut keys = self
            .tiles
            .keys()
            .chain(other.tiles.keys())
            .copied()
            .collect::<Vec<_>>();
        keys.sort_unstable();
        keys.dedup();
        for key in keys {
            let a = self.tiles.get(&key);
            let b = other.tiles.get(&key);
            match (a, b, operation) {
                (Some(a), None, Combine::Add | Combine::Subtract) => {
                    result.tiles.insert(key, a.clone());
                    continue;
                }
                (None, Some(b), Combine::Add) => {
                    result.tiles.insert(key, b.clone());
                    continue;
                }
                (None, _, _) | (_, None, _) => continue,
                _ => {}
            }
            if let (Some(a), Some(b)) = (a, b) {
                let mut values = filled(a.len(), 0u8)?;
                for ((out, a), b) in values.iter_mut().zip(a.iter()).zip(b.iter()) {
                    *out = match operation {
                        Combine::Add => (*a).max(*b),
                        Combine::Subtract => a.saturating_sub(*b),
                        Combine::Intersect => (*a).min(*b),
                    };
                }
                if values.iter().any(|&v| v != 0) {
                    result.tiles.insert(key, Arc::from(values));
                }
            }
        }
        Ok(result)
    }
    pub fn add(&self, other: &Self) -> Result<Self> {
        self.combine(other, Combine::Add)
    }
    pub fn subtract(&self, other: &Self) -> Result<Self> {
        self.combine(other, Combine::Subtract)
    }
    pub fn intersect(&self, other: &Self) -> Result<Self> {
        self.combine(other, Combine::Intersect)
    }
    pub fn invert(&self) -> Result<Self> {
        let version = next(self.version)?;
        let mut pixels = self.to_dense()?;
        for pixel in &mut pixels {
            *pixel = 255 - *pixel;
        }
        Self::dense_version(self.size, &pixels, version)
    }
    /// Pixel identity excludes the history counter; repeated identical pixels
    /// share a content address. The algorithm and document dimensions are bound.
    pub fn content_hash(&self) -> [u8; 32] {
        let mut hash = blake3::Hasher::new();
        hash.update(b"vw-mask-content-v1\0");
        hash.update(&ALGORITHM_VERSION.to_le_bytes());
        hash.update(&self.size.width.to_le_bytes());
        hash.update(&self.size.height.to_le_bytes());
        hash.update(&(self.tiles.len() as u32).to_le_bytes());
        for tile in self.tiles() {
            hash.update(&tile.coord.x.to_le_bytes());
            hash.update(&tile.coord.y.to_le_bytes());
            hash.update(&(tile.pixels.len() as u32).to_le_bytes());
            hash.update(tile.pixels);
        }
        *hash.finalize().as_bytes()
    }
    /// Canonical little-endian lossless data, bounded and uncompressed. A blob
    /// container may compress it losslessly; masks never use lossy color codecs.
    pub fn encode_lossless(&self) -> Result<Vec<u8>> {
        let length = self.tiles.values().try_fold(32usize, |n, t| {
            n.checked_add(12 + t.len()).ok_or(MaskError::Limit)
        })?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| MaskError::Allocation)?;
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&ALGORITHM_VERSION.to_le_bytes());
        bytes.extend_from_slice(&self.size.width.to_le_bytes());
        bytes.extend_from_slice(&self.size.height.to_le_bytes());
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.extend_from_slice(&(self.tiles.len() as u32).to_le_bytes());
        for tile in self.tiles() {
            bytes.extend_from_slice(&tile.coord.x.to_le_bytes());
            bytes.extend_from_slice(&tile.coord.y.to_le_bytes());
            bytes.extend_from_slice(&(tile.pixels.len() as u32).to_le_bytes());
            bytes.extend_from_slice(tile.pixels);
        }
        Ok(bytes)
    }
    pub fn decode_lossless(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_BUFFER_BYTES || bytes.get(..8) != Some(MAGIC.as_slice()) {
            return Err(MaskError::Corrupt);
        }
        let mut input = &bytes[8..];
        if take_u32(&mut input)? != ALGORITHM_VERSION {
            return Err(MaskError::Corrupt);
        }
        let size = Size::new(take_u32(&mut input)?, take_u32(&mut input)?)
            .map_err(|_| MaskError::Corrupt)?;
        let version = u64::from_le_bytes(
            take(&mut input, 8)?
                .try_into()
                .map_err(|_| MaskError::Corrupt)?,
        );
        let count = take_u32(&mut input)?;
        let maximum =
            u64::from(size.width.div_ceil(TILE_SIDE)) * u64::from(size.height.div_ceil(TILE_SIDE));
        if u64::from(count) > maximum || u64::from(count) * 13 > input.len() as u64 {
            return Err(MaskError::Corrupt);
        }
        let mut result = Self {
            size,
            version,
            tiles: BTreeMap::new(),
        };
        let mut previous = None;
        for _ in 0..count {
            let coord = TileCoord {
                x: take_u32(&mut input)?,
                y: take_u32(&mut input)?,
            };
            let length = take_u32(&mut input)? as usize;
            if previous.is_some_and(|old| coord <= old)
                || size
                    .tile_size(coord)
                    .map_err(|_| MaskError::Corrupt)?
                    .pixels()
                    != length
            {
                return Err(MaskError::Corrupt);
            }
            let pixels = take(&mut input, length)?;
            if !pixels.iter().any(|&v| v != 0) {
                return Err(MaskError::Corrupt);
            }
            result.tiles.insert(coord, Arc::from(pixels));
            previous = Some(coord);
        }
        if !input.is_empty() {
            return Err(MaskError::Corrupt);
        }
        Ok(result)
    }
}

pub(crate) fn next(version: u64) -> Result<u64> {
    version.checked_add(1).ok_or(MaskError::VersionExhausted)
}
pub(crate) fn filled<T: Clone>(length: usize, value: T) -> Result<Vec<T>> {
    if length
        .checked_mul(std::mem::size_of::<T>())
        .is_none_or(|n| n > MAX_BUFFER_BYTES)
    {
        return Err(MaskError::Limit);
    }
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| MaskError::Allocation)?;
    values.resize(length, value);
    Ok(values)
}
pub(crate) fn quantize(value: f64) -> Result<i64> {
    if !value.is_finite() || value.abs() > MAX_COORDINATE {
        return Err(MaskError::Invalid);
    }
    Ok(libm::round(value * 256.0) as i64)
}
fn take<'a>(input: &mut &'a [u8], length: usize) -> Result<&'a [u8]> {
    let (head, tail) = input.split_at_checked(length).ok_or(MaskError::Corrupt)?;
    *input = tail;
    Ok(head)
}
fn take_u32(input: &mut &[u8]) -> Result<u32> {
    Ok(u32::from_le_bytes(
        take(input, 4)?.try_into().map_err(|_| MaskError::Corrupt)?,
    ))
}

pub(crate) struct Builder {
    size: Size,
    tiles: BTreeMap<TileCoord, Vec<u8>>,
}
impl Builder {
    pub(crate) fn new(size: Size) -> Self {
        Self {
            size,
            tiles: BTreeMap::new(),
        }
    }
    pub(crate) fn row(&mut self, mut x: u32, y: u32, mut source: &[u8]) -> Result<()> {
        if y >= self.size.height || u64::from(x) + source.len() as u64 > u64::from(self.size.width)
        {
            return Err(MaskError::Invalid);
        }
        while !source.is_empty() {
            let coord = TileCoord {
                x: x / TILE_SIDE,
                y: y / TILE_SIDE,
            };
            let size = self.size.tile_size(coord)?;
            let length = source.len().min((size.width - x % TILE_SIDE) as usize);
            let (part, rest) = source.split_at(length);
            if part.iter().any(|&v| v != 0) {
                if let std::collections::btree_map::Entry::Vacant(entry) = self.tiles.entry(coord) {
                    entry.insert(filled(size.pixels(), 0u8)?);
                }
                let pixels = self.tiles.get_mut(&coord).ok_or(MaskError::Invalid)?;
                let index = ((y % TILE_SIDE) * size.width + x % TILE_SIDE) as usize;
                pixels[index..index + length].copy_from_slice(part);
            }
            x += length as u32;
            source = rest;
        }
        Ok(())
    }
    pub(crate) fn finish(self, version: u64) -> Mask {
        Mask {
            size: self.size,
            version,
            tiles: self
                .tiles
                .into_iter()
                .map(|(c, p)| (c, Arc::from(p)))
                .collect(),
        }
    }
}
