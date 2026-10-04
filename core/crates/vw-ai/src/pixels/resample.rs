//! Normalized Lanczos3 without intermediate channel clipping. Alpha and color
//! must keep the same overshoot until straight-color reconstruction is complete.
use crate::{Cancellation, Error, Result, check_cancel, pixel_count};
use image::{ImageBuffer, Rgba};

type FloatImage = ImageBuffer<Rgba<f32>, Vec<f32>>;

fn kernel(distance: f64) -> f64 {
    let x = distance.abs();
    if x == 0.0 {
        1.0
    } else if x >= 3.0 {
        0.0
    } else {
        let p = std::f64::consts::PI * x;
        libm::sin(p) / p * (libm::sin(p / 3.0) / (p / 3.0))
    }
}

fn axis(
    image: &FloatImage,
    length: u32,
    horizontal: bool,
    cancel: &dyn Cancellation,
) -> Result<FloatImage> {
    let (source, across) = if horizontal {
        (image.width(), image.height())
    } else {
        (image.height(), image.width())
    };
    let (width, height) = if horizontal {
        (length, across)
    } else {
        (across, length)
    };
    pixel_count(width, height)?;
    check_cancel(cancel)?;
    let mut output = FloatImage::new(width, height);
    let ratio = f64::from(source) / f64::from(length);
    let scale = ratio.max(1.0);
    let support = 3.0 * scale;
    // At most MAX_EDGE weights; reused for each output coordinate.
    let mut weights = Vec::new();
    for coordinate in 0..length {
        check_cancel(cancel)?;
        let center = (f64::from(coordinate) + 0.5) * ratio;
        let first = libm::floor(center - support).clamp(0.0, f64::from(source - 1)) as u32;
        let end =
            libm::ceil(center + support).clamp(f64::from(first + 1), f64::from(source)) as u32;
        weights.clear();
        let mut sum = 0.0;
        for index in first..end {
            let weight = kernel((f64::from(index) - center + 0.5) / scale);
            sum += weight;
            weights.push(weight);
        }
        if !sum.is_finite() || sum.abs() < 1e-12 {
            return Err(Error::Invalid("resampling weights"));
        }
        for weight in &mut weights {
            *weight /= sum;
        }
        for cross in 0..across {
            let mut channels = [0.0; 4];
            for (offset, weight) in weights.iter().enumerate() {
                let index = first + offset as u32;
                let pixel = if horizontal {
                    image.get_pixel(index, cross)
                } else {
                    image.get_pixel(cross, index)
                };
                for channel in 0..4 {
                    channels[channel] += f64::from(pixel[channel]) * weight;
                }
            }
            let pixel = Rgba(channels.map(|v| v as f32));
            if horizontal {
                output.put_pixel(coordinate, cross, pixel);
            } else {
                output.put_pixel(cross, coordinate, pixel);
            }
        }
    }
    Ok(output)
}

pub(super) fn resize(
    image: &FloatImage,
    width: u32,
    height: u32,
    cancel: &dyn Cancellation,
) -> Result<FloatImage> {
    pixel_count(image.width(), image.height())?;
    pixel_count(width, height)?;
    // Choose the smaller intermediate for mixed-axis scaling. Its pixel count
    // never exceeds the larger input/output, so existing workspace admission
    // still covers input, intermediate and final floating buffers.
    if u64::from(width) * u64::from(image.height()) <= u64::from(image.width()) * u64::from(height)
    {
        let intermediate = axis(image, width, true, cancel)?;
        axis(&intermediate, height, false, cancel)
    } else {
        let intermediate = axis(image, height, false, cancel)?;
        axis(&intermediate, width, true, cancel)
    }
}
