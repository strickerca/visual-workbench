use crate::{Error, Result, pixels, sha256};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metrics {
    pub selected_pixels: u64,
    pub delta_e2000_mean: f64,
    pub delta_e2000_max: f64,
    pub ssim_inside_mask: f64,
    pub definition: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    pub schema: u32,
    pub width: u32,
    pub height: u32,
    pub dilation_px: u32,
    pub dilation_metric: String,
    pub changed_outside: u64,
    pub outside_pixels: u64,
    pub outside_sha256_before: String,
    pub outside_sha256_after: String,
    pub source_pixels_sha256: String,
    pub result_pixels_sha256: String,
    pub source_file_sha256: String,
    pub mask_sha256: String,
    pub metrics: Metrics,
}

pub fn pixel_hash(image: &RgbaImage) -> String {
    let mut hash = Sha256::new();
    hash.update(b"vw-ai-spike/rgba8/v1\0");
    hash.update(image.width().to_le_bytes());
    hash.update(image.height().to_le_bytes());
    hash.update(image.as_raw());
    crate::hex(&hash.finalize())
}

pub fn compare(
    source: &RgbaImage,
    result: &RgbaImage,
    mask: &[u8],
    radius: u32,
    source_file_sha256: &str,
    source_icc: Option<&[u8]>,
) -> Result<Proof> {
    if source.dimensions() != result.dimensions() || radius > crate::MAX_FEATHER {
        return Err(Error::Invalid("proof dimensions or radius"));
    }
    if source_file_sha256.len() != 64 || !source_file_sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(Error::Invalid("source file binding"));
    }
    let dilation_px = radius + 1;
    let excluded = pixels::dilate(mask, source.width(), source.height(), dilation_px)?;
    let mask_sha256 = sha256(mask);
    let mut before = Sha256::new();
    before.update(b"vw-ai-spike/outside/v1\0");
    before.update(source.width().to_le_bytes());
    before.update(source.height().to_le_bytes());
    before.update(dilation_px.to_le_bytes());
    before.update(mask_sha256.as_bytes());
    let mut after = before.clone();
    let mut changed_outside = 0;
    let mut outside_pixels = 0;
    for (index, (a, b)) in source.pixels().zip(result.pixels()).enumerate() {
        if !excluded[index] {
            outside_pixels += 1;
            changed_outside += u64::from(a != b);
            before.update((index as u64).to_le_bytes());
            before.update(a.0);
            after.update((index as u64).to_le_bytes());
            after.update(b.0);
        }
    }
    let a = pixels::transform(source, source_icc, true)?;
    let b = pixels::transform(result, source_icc, true)?;
    Ok(Proof {
        schema: 1,
        width: source.width(),
        height: source.height(),
        dilation_px,
        dilation_metric: "euclidean_disk_pixel_centers".into(),
        changed_outside,
        outside_pixels,
        outside_sha256_before: crate::hex(&before.finalize()),
        outside_sha256_after: crate::hex(&after.finalize()),
        source_pixels_sha256: pixel_hash(source),
        result_pixels_sha256: pixel_hash(result),
        source_file_sha256: source_file_sha256.into(),
        mask_sha256,
        metrics: metrics(&a, &b, mask)?,
    })
}

impl Proof {
    pub fn require_unchanged_exterior(&self) -> Result<()> {
        if self.changed_outside != 0 {
            return Err(Error::Exterior(self.changed_outside));
        }
        if self.outside_sha256_before != self.outside_sha256_after {
            return Err(Error::Invalid("exterior hash mismatch"));
        }
        Ok(())
    }
}

fn linear(v: u8) -> f64 {
    let s = f64::from(v) / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

pub fn srgb_lab(rgb: [u8; 3]) -> [f64; 3] {
    let [r, g, b] = rgb.map(linear);
    let f = |t: f64| {
        if t > 216.0 / 24389.0 {
            t.cbrt()
        } else {
            (24389.0 / 27.0 * t + 16.0) / 116.0
        }
    };
    let x = f((0.4124564 * r + 0.3575761 * g + 0.1804375 * b) / 0.95047);
    let y = f(0.2126729 * r + 0.7151522 * g + 0.0721750 * b);
    let z = f((0.0193339 * r + 0.1191920 * g + 0.9503041 * b) / 1.08883);
    [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
}

/// CIEDE2000 with kL=kC=kH=1; angular cases follow the published definition.
pub fn delta_e2000(lab1: [f64; 3], lab2: [f64; 3]) -> f64 {
    let [l1, a1, b1] = lab1;
    let [l2, a2, b2] = lab2;
    let cbar = (a1.hypot(b1) + a2.hypot(b2)) / 2.0;
    let pow25 = 25f64.powi(7);
    let g = 0.5 * (1.0 - (cbar.powi(7) / (cbar.powi(7) + pow25)).sqrt());
    let ap1 = (1.0 + g) * a1;
    let ap2 = (1.0 + g) * a2;
    let cp1 = ap1.hypot(b1);
    let cp2 = ap2.hypot(b2);
    let hue = |a: f64, b: f64| {
        if a == 0.0 && b == 0.0 {
            0.0
        } else {
            b.atan2(a).to_degrees().rem_euclid(360.0)
        }
    };
    let hp1 = hue(ap1, b1);
    let hp2 = hue(ap2, b2);
    let mut dh = hp2 - hp1;
    if cp1 * cp2 == 0.0 {
        dh = 0.0;
    } else if dh > 180.0 {
        dh -= 360.0;
    } else if dh < -180.0 {
        dh += 360.0;
    }
    let dl = l2 - l1;
    let dc = cp2 - cp1;
    let d_h = 2.0 * (cp1 * cp2).sqrt() * (dh / 2.0).to_radians().sin();
    let lb = (l1 + l2) / 2.0;
    let cb = (cp1 + cp2) / 2.0;
    let hb = if cp1 * cp2 == 0.0 {
        hp1 + hp2
    } else if (hp1 - hp2).abs() <= 180.0 {
        (hp1 + hp2) / 2.0
    } else if hp1 + hp2 < 360.0 {
        (hp1 + hp2 + 360.0) / 2.0
    } else {
        (hp1 + hp2 - 360.0) / 2.0
    };
    let cos = |degrees: f64| degrees.to_radians().cos();
    let t = 1.0 - 0.17 * cos(hb - 30.0) + 0.24 * cos(2.0 * hb) + 0.32 * cos(3.0 * hb + 6.0)
        - 0.20 * cos(4.0 * hb - 63.0);
    let sl = 1.0 + 0.015 * (lb - 50.0).powi(2) / (20.0 + (lb - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cb;
    let sh = 1.0 + 0.015 * cb * t;
    let rt = -2.0
        * (cb.powi(7) / (cb.powi(7) + pow25)).sqrt()
        * (60.0 * (-((hb - 275.0) / 25.0).powi(2)).exp())
            .to_radians()
            .sin();
    ((dl / sl).powi(2) + (dc / sc).powi(2) + (d_h / sh).powi(2) + rt * (dc / sc) * (d_h / sh))
        .max(0.0)
        .sqrt()
}

fn metrics(source: &RgbaImage, result: &RgbaImage, mask: &[u8]) -> Result<Metrics> {
    let mut count = 0u64;
    let mut de_sum: f64 = 0.0;
    let mut de_max: f64 = 0.0;
    let (mut sx, mut sy, mut xx, mut yy, mut xy) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for ((a, b), selected) in source.pixels().zip(result.pixels()).zip(mask) {
        if *selected == 0 {
            continue;
        }
        count += 1;
        let de = delta_e2000(srgb_lab([a[0], a[1], a[2]]), srgb_lab([b[0], b[1], b[2]]));
        de_sum += de;
        de_max = de_max.max(de);
        let luma = |p: &image::Rgba<u8>| {
            (0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])) / 255.0
        };
        let x = luma(a);
        let y = luma(b);
        sx += x;
        sy += y;
        xx += x * x;
        yy += y * y;
        xy += x * y;
    }
    if count == 0 {
        return Err(Error::Invalid("empty metric mask"));
    }
    let n = count as f64;
    let mx = sx / n;
    let my = sy / n;
    let vx = (xx / n - mx * mx).max(0.0);
    let vy = (yy / n - my * my).max(0.0);
    let covariance = xy / n - mx * my;
    let ssim = ((2.0 * mx * my + 0.0001) * (2.0 * covariance + 0.0009))
        / ((mx * mx + my * my + 0.0001) * (vx + vy + 0.0009));
    Ok(Metrics { selected_pixels: count, delta_e2000_mean: de_sum / n, delta_e2000_max: de_max,
        ssim_inside_mask: ssim.clamp(-1.0, 1.0),
        definition: "RGB ICC to 8-bit sRGB, CIELAB D65 CIEDE2000; global masked sRGB luminance SSIM, population variance, K1=.01 K2=.03 L=1; alpha excluded from perceptual metrics, included in exact proof".into() })
}
