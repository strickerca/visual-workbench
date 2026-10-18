use crate::{AssetError, Result};

pub const TILE_SIZE: u32 = 256;
pub const MAX_PIXELS: u64 = 1_000_000_000;
pub(crate) const MAX_TILES: usize = 100_000;
const MAX_SIDE: u32 = 1_048_576;
const MAX_VISIBLE: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TileKey {
    pub level: u8,
    pub x: u32,
    pub y: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    pub index: u8,
    pub width: u32,
    pub height: u32,
    pub columns: u32,
    pub rows: u32,
}

/// Level zero is full-resolution D-space; each next level halves with ceiling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pyramid {
    levels: Vec<Level>,
    count: usize,
}
impl Pyramid {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        if width == 0
            || height == 0
            || width > MAX_SIDE
            || height > MAX_SIDE
            || u64::from(width) * u64::from(height) > MAX_PIXELS
        {
            return Err(AssetError::Limit("dimensions"));
        }
        let (mut width, mut height, mut count) = (width, height, 0usize);
        let mut levels = Vec::with_capacity(24);
        loop {
            let columns = width.div_ceil(TILE_SIZE);
            let rows = height.div_ceil(TILE_SIZE);
            count = count
                .checked_add((u64::from(columns) * u64::from(rows)) as usize)
                .ok_or(AssetError::Limit("tiles"))?;
            if count > MAX_TILES {
                return Err(AssetError::Limit("tiles"));
            }
            levels.push(Level {
                index: levels.len() as u8,
                width,
                height,
                columns,
                rows,
            });
            if width == 1 && height == 1 {
                break;
            }
            width = width.div_ceil(2);
            height = height.div_ceil(2);
        }
        Ok(Self { levels, count })
    }
    pub fn levels(&self) -> &[Level] {
        &self.levels
    }
    pub fn tile_count(&self) -> usize {
        self.count
    }
    pub fn level(&self, index: u8) -> Result<Level> {
        self.levels
            .get(index as usize)
            .copied()
            .ok_or(AssetError::Invalid("level"))
    }
    pub fn region(&self, key: TileKey) -> Result<TileRegion> {
        let level = self.level(key.level)?;
        if key.x >= level.columns || key.y >= level.rows {
            return Err(AssetError::Invalid("tile coordinates"));
        }
        let x = key.x * TILE_SIZE;
        let y = key.y * TILE_SIZE;
        Ok(TileRegion {
            x,
            y,
            width: (level.width - x).min(TILE_SIZE),
            height: (level.height - y).min(TILE_SIZE),
        })
    }
    pub fn keys(&self) -> impl Iterator<Item = TileKey> + '_ {
        self.levels.iter().flat_map(|l| {
            (0..l.rows).flat_map(move |y| {
                (0..l.columns).map(move |x| TileKey {
                    level: l.index,
                    x,
                    y,
                })
            })
        })
    }
    /// Axis-aligned D-space envelope of a (possibly rotated) physical viewport.
    /// Caller supplies maximum singular scale of its D-to-physical transform.
    /// This conservatively avoids undersampling under shear/nonuniform transforms.
    pub fn viewport(&self, view: Viewport) -> Result<ViewportTiles> {
        if ![
            view.left,
            view.top,
            view.right,
            view.bottom,
            view.physical_scale,
        ]
        .iter()
        .all(|v| v.is_finite())
            || view.right <= view.left
            || view.bottom <= view.top
            || view.physical_scale <= 0.0
        {
            return Err(AssetError::Invalid("viewport"));
        }
        let mut index = 0;
        while index + 1 < self.levels.len()
            && view.physical_scale * (1u64 << (index + 1)) as f64 <= 1.0
        {
            index += 1;
        }
        let level = self.levels[index];
        let scale = (1u64 << index) as f64;
        let left = (view.left / scale).max(0.0).min(f64::from(level.width));
        let top = (view.top / scale).max(0.0).min(f64::from(level.height));
        let right = (view.right / scale).max(0.0).min(f64::from(level.width));
        let bottom = (view.bottom / scale).max(0.0).min(f64::from(level.height));
        if left >= right || top >= bottom {
            return Ok(ViewportTiles {
                level: index as u8,
                visible: Vec::new(),
                prefetch: Vec::new(),
            });
        }
        let x0 = (left / f64::from(TILE_SIZE)).floor() as u32;
        let y0 = (top / f64::from(TILE_SIZE)).floor() as u32;
        let x1 = ((right / f64::from(TILE_SIZE)).ceil() as u32).min(level.columns);
        let y1 = ((bottom / f64::from(TILE_SIZE)).ceil() as u32).min(level.rows);
        let px0 = x0.saturating_sub(1);
        let py0 = y0.saturating_sub(1);
        let px1 = x1.saturating_add(1).min(level.columns);
        let py1 = y1.saturating_add(1).min(level.rows);
        if u64::from(px1 - px0) * u64::from(py1 - py0) > MAX_VISIBLE as u64 {
            return Err(AssetError::Limit("viewport tiles"));
        }
        let mut visible = Vec::new();
        let mut prefetch = Vec::new();
        for y in py0..py1 {
            for x in px0..px1 {
                let key = TileKey {
                    level: index as u8,
                    x,
                    y,
                };
                if x >= x0 && x < x1 && y >= y0 && y < y1 {
                    visible.push(key);
                } else {
                    prefetch.push(key);
                }
            }
        }
        Ok(ViewportTiles {
            level: index as u8,
            visible,
            prefetch,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub physical_scale: f64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewportTiles {
    pub level: u8,
    pub visible: Vec<TileKey>,
    pub prefetch: Vec<TileKey>,
}

/// Largest singular value of the linear D-to-physical transform
/// `x' = a*x + c*y`, `y' = b*x + d*y`. Translation does not affect density.
/// Use this value for `Viewport::physical_scale`; the minimum singular value
/// can undersample a stretched or sheared image. Inverse viewport mapping must
/// separately reject non-invertible transforms.
/// Scaling before the closed-form calculation avoids intermediate overflow.
pub fn maximum_physical_scale(a: f64, b: f64, c: f64, d: f64) -> Result<f64> {
    if ![a, b, c, d].iter().all(|value| value.is_finite()) {
        return Err(AssetError::Invalid("physical transform"));
    }
    let magnitude = a.abs().max(b.abs()).max(c.abs()).max(d.abs());
    if magnitude == 0.0 {
        return Err(AssetError::Invalid("physical transform"));
    }
    let (a, b, c, d) = (a / magnitude, b / magnitude, c / magnitude, d / magnitude);
    let scale = magnitude * ((a + d).hypot(b - c) * 0.5 + (a - d).hypot(b + c) * 0.5);
    if !scale.is_finite() || scale <= 0.0 {
        return Err(AssetError::Invalid("physical transform"));
    }
    Ok(scale)
}
