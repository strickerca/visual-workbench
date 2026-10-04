use crate::{Error, Result, config::Capabilities, pixel_count};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CropPlan {
    pub bounds: Rect,
    pub crop: Rect,
    pub model_width: u32,
    pub model_height: u32,
    pub has_padding: bool,
    pub experimental_size: bool,
}

pub fn mask_bounds(mask: &[u8], width: u32, height: u32) -> Result<Rect> {
    if mask.len() != pixel_count(width, height)? {
        return Err(Error::Invalid("mask dimensions"));
    }
    let (mut left, mut top, mut right, mut bottom) = (width, height, 0, 0);
    for (index, alpha) in mask.iter().enumerate() {
        if *alpha != 0 {
            let x = index as u32 % width;
            let y = index as u32 / width;
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
        }
    }
    if right == 0 {
        return Err(Error::Invalid("empty change mask"));
    }
    Ok(Rect {
        x: left as i32,
        y: top as i32,
        width: right - left,
        height: bottom - top,
    })
}

fn grow_axis(start: i32, old: u32, new: u32, boundary: u32) -> i32 {
    let centered = start + (old as i32 - new as i32).div_euclid(2);
    centered.clamp(
        (boundary as i32 - new as i32).min(0),
        (boundary as i32 - new as i32).max(0),
    )
}

/// Deterministic nearest size in logarithmic dimension space. Grid rounding can
/// introduce small anisotropy; both exact scale factors are implicit in C and S.
pub fn closest_size(width: u32, height: u32, caps: &Capabilities) -> Result<(u32, u32)> {
    if width == 0 || height == 0 || caps.size_multiple == 0 || caps.max_long_edge > crate::MAX_EDGE
    {
        return Err(Error::Invalid("size search limits"));
    }
    caps.validate()?;
    let mut best: Option<(f64, u32, u32)> = None;
    for w in (caps.size_multiple..=caps.max_long_edge).step_by(caps.size_multiple as usize) {
        for h in (caps.size_multiple..=caps.max_long_edge).step_by(caps.size_multiple as usize) {
            if caps.valid_size(w, h) {
                let score = libm::pow(libm::log(f64::from(w) / f64::from(width)), 2.0)
                    + libm::pow(libm::log(f64::from(h) / f64::from(height)), 2.0);
                if best.is_none_or(|current| (score, w, h) < current) {
                    best = Some((score, w, h));
                }
            }
        }
    }
    best.map(|(_, w, h)| (w, h))
        .ok_or(Error::Invalid("provider has no valid size"))
}

pub fn plan(mask: &[u8], width: u32, height: u32, caps: &Capabilities) -> Result<CropPlan> {
    let bounds = mask_bounds(mask, width, height)?;
    if caps.max_aspect_ratio == 0 || caps.max_aspect_ratio > 16 {
        return Err(Error::Invalid("aspect limit"));
    }
    // Expansion 1.75 means 0.375 on each side, rounded outward, at least 64px.
    let mx = (bounds.width * 3).div_ceil(8).max(64) as i32;
    let my = (bounds.height * 3).div_ceil(8).max(64) as i32;
    let x = (bounds.x - mx).max(0);
    let y = (bounds.y - my).max(0);
    let right = (bounds.x + bounds.width as i32 + mx).min(width as i32);
    let bottom = (bounds.y + bounds.height as i32 + my).min(height as i32);
    let mut crop = Rect {
        x,
        y,
        width: (right - x) as u32,
        height: (bottom - y) as u32,
    };
    if crop.width > crop.height * caps.max_aspect_ratio {
        let grown = crop.width.div_ceil(caps.max_aspect_ratio);
        crop.y = grow_axis(crop.y, crop.height, grown, height);
        crop.height = grown;
    } else if crop.height > crop.width * caps.max_aspect_ratio {
        let grown = crop.height.div_ceil(caps.max_aspect_ratio);
        crop.x = grow_axis(crop.x, crop.width, grown, width);
        crop.width = grown;
    }
    pixel_count(crop.width, crop.height)?;
    let (model_width, model_height) = closest_size(crop.width, crop.height, caps)?;
    Ok(CropPlan {
        bounds,
        crop,
        model_width,
        model_height,
        has_padding: crop.x < 0
            || crop.y < 0
            || crop.x + crop.width as i32 > width as i32
            || crop.y + crop.height as i32 > height as i32,
        experimental_size: u64::from(model_width) * u64::from(model_height)
            > caps.experimental_above_pixels,
    })
}
