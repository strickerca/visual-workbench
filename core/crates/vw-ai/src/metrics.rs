use crate::{Error, Result, proof::Metrics};
use image::RgbaImage;
fn linear(v: u8) -> f64 {
    let s = f64::from(v) / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        libm::pow((s + 0.055) / 1.055, 2.4)
    }
}

pub fn srgb_lab(rgb: [u8; 3]) -> [f64; 3] {
    let [r, g, b] = rgb.map(linear);
    let f = |t: f64| {
        if t > 216.0 / 24389.0 {
            libm::cbrt(t)
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
    let cbar = (libm::hypot(a1, b1) + libm::hypot(a2, b2)) / 2.0;
    let pow25 = 6_103_515_625.0;
    let power7 = |v: f64| {
        let square = v * v;
        square * square * square * v
    };
    let g = 0.5 * (1.0 - libm::sqrt(power7(cbar) / (power7(cbar) + pow25)));
    let ap1 = (1.0 + g) * a1;
    let ap2 = (1.0 + g) * a2;
    let cp1 = libm::hypot(ap1, b1);
    let cp2 = libm::hypot(ap2, b2);
    let hue = |a: f64, b: f64| {
        if a == 0.0 && b == 0.0 {
            0.0
        } else {
            libm::atan2(b, a).to_degrees().rem_euclid(360.0)
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
    let d_h = 2.0 * libm::sqrt(cp1 * cp2) * libm::sin((dh / 2.0).to_radians());
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
    let cos = |degrees: f64| libm::cos(degrees.to_radians());
    let t = 1.0 - 0.17 * cos(hb - 30.0) + 0.24 * cos(2.0 * hb) + 0.32 * cos(3.0 * hb + 6.0)
        - 0.20 * cos(4.0 * hb - 63.0);
    let square = |v: f64| v * v;
    let sl = 1.0 + 0.015 * square(lb - 50.0) / libm::sqrt(20.0 + square(lb - 50.0));
    let sc = 1.0 + 0.045 * cb;
    let sh = 1.0 + 0.015 * cb * t;
    let rt = -2.0
        * libm::sqrt(power7(cb) / (power7(cb) + pow25))
        * libm::sin((60.0 * libm::exp(-square((hb - 275.0) / 25.0))).to_radians());
    libm::sqrt(
        (square(dl / sl) + square(dc / sc) + square(d_h / sh) + rt * (dc / sc) * (d_h / sh))
            .max(0.0),
    )
}

pub(crate) fn metrics(
    source: &RgbaImage,
    result: &RgbaImage,
    mask: &[u8],
    cancel: &dyn crate::Cancellation,
) -> Result<Metrics> {
    if source.dimensions() != result.dimensions()
        || mask.len() != crate::pixel_count(source.width(), source.height())?
    {
        return Err(Error::Invalid("metric dimensions"));
    }
    let mut count = 0u64;
    let mut de_sum: f64 = 0.0;
    let mut de_max: f64 = 0.0;
    let (mut sx, mut sy, mut xx, mut yy, mut xy) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (index, ((a, b), selected)) in source.pixels().zip(result.pixels()).zip(mask).enumerate() {
        if index % 1024 == 0 {
            crate::check_cancel(cancel)?;
        }
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
