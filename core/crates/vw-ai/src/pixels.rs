//! Extracted from the frozen T0.11 spike: edge padding, premultiplied Lanczos3,
//! source-profile transforms and area masks. Mask filtering uses vw-mask v1.
mod resample;
#[cfg(test)]
mod resample_tests;
use crate::{
    Cancellation, Error, Limits, MAX_EDGE, Result, check_cancel, geometry::Rect, pixel_count,
};
use image::{Rgba, RgbaImage};
use moxcms::{ColorProfile, DataColorSpace, Layout, TransformOptions};
use std::borrow::Cow;
use vw_raster::{DecodedImage, Pixels};

#[derive(Clone)]
pub struct EditImage {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixels: Pixels,
    pub(crate) icc: Vec<u8>,
}
impl EditImage {
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn pixels(&self) -> &Pixels {
        &self.pixels
    }
    pub fn icc(&self) -> &[u8] {
        &self.icc
    }
    pub fn bit_depth(&self) -> u8 {
        self.pixels.bit_depth()
    }
    pub(crate) fn from_decoded(value: DecodedImage, assume_srgb: bool) -> Result<Self> {
        value.validate()?;
        pixel_count(value.width, value.height)?;
        if !value.original_available {
            return Err(Error::Invalid("original source required"));
        }
        if value.icc.is_none() && !assume_srgb {
            return Err(Error::Invalid(
                "untagged source requires explicit sRGB assumption",
            ));
        }
        let icc = profile(value.icc.as_deref())?;
        let parsed = ColorProfile::new_from_slice(&icc).map_err(|_| Error::Color)?;
        if parsed.color_space != DataColorSpace::Rgb {
            return Err(Error::Color);
        }
        Ok(Self {
            width: value.width,
            height: value.height,
            pixels: value.pixels,
            icc,
        })
    }
    pub(crate) fn rgba8(&self) -> RgbaImage {
        RgbaImage::from_fn(self.width, self.height, |x, y| Rgba(self.sample8(x, y)))
    }
    pub(crate) fn sample8(&self, x: u32, y: u32) -> [u8; 4] {
        let index = (y as usize * self.width as usize + x as usize) * 4;
        match &self.pixels {
            Pixels::Rgba8(values) => [
                values[index],
                values[index + 1],
                values[index + 2],
                values[index + 3],
            ],
            Pixels::Rgba16(values) => {
                std::array::from_fn(|c| ((u32::from(values[index + c]) + 128) / 257) as u8)
            }
        }
    }
    pub fn encode_png(&self, limits: Limits, cancel: &dyn Cancellation) -> Result<Vec<u8>> {
        check_cancel(cancel)?;
        let count = pixel_count(self.width, self.height)? as u64;
        limits.check(count * 32 + 32 * 1024 * 1024)?;
        let mut destination = EncodedSink {
            bytes: Vec::new(),
            limit: crate::MAX_ENCODED,
            exceeded: false,
        };
        let encoded = (|| -> Result<()> {
            let mut info = png::Info::with_size(self.width, self.height);
            info.color_type = png::ColorType::Rgba;
            info.bit_depth = if self.bit_depth() == 16 {
                png::BitDepth::Sixteen
            } else {
                png::BitDepth::Eight
            };
            info.icc_profile = Some(Cow::Borrowed(self.icc.as_slice()));
            let encoder =
                png::Encoder::with_info(&mut destination, info).map_err(|_| Error::Codec)?;
            let mut writer = encoder.write_header().map_err(|_| Error::Codec)?;
            match &self.pixels {
                Pixels::Rgba8(p) => writer.write_image_data(p).map_err(|_| Error::Codec)?,
                Pixels::Rgba16(p) => {
                    let raw = p.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<_>>();
                    writer.write_image_data(&raw).map_err(|_| Error::Codec)?;
                }
            }
            writer.finish().map_err(|_| Error::Codec)?;
            Ok(())
        })();
        if destination.exceeded {
            return Err(Error::Limit("encoded composite"));
        }
        encoded?;
        check_cancel(cancel)?;
        Ok(destination.bytes)
    }
}

struct EncodedSink {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}
impl std::io::Write for EncodedSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(std::io::Error::other("encoded image limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(crate) fn profile(icc: Option<&[u8]>) -> Result<Vec<u8>> {
    if let Some(icc) = icc {
        if icc.len() > 4 * 1024 * 1024 {
            return Err(Error::Color);
        }
        return Ok(icc.to_vec());
    }
    let mut profile = ColorProfile::new_srgb()
        .encode()
        .map_err(|_| Error::Color)?;
    profile
        .get_mut(24..36)
        .ok_or(Error::Color)?
        .copy_from_slice(&[0x07, 0xd0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0]);
    Ok(profile)
}
pub(crate) fn encode_png(image: &RgbaImage, icc: Option<&[u8]>) -> Result<Vec<u8>> {
    EditImage {
        width: image.width(),
        height: image.height(),
        pixels: Pixels::Rgba8(image.as_raw().clone()),
        icc: profile(icc)?,
    }
    .encode_png(
        Limits {
            memory_bytes: crate::MAX_MEMORY,
        },
        &crate::NeverCancel,
    )
}
pub(crate) fn transform(image: &RgbaImage, profile: &[u8], to_srgb: bool) -> Result<RgbaImage> {
    let source = ColorProfile::new_from_slice(profile).map_err(|_| Error::Color)?;
    if source.color_space != DataColorSpace::Rgb {
        return Err(Error::Color);
    }
    let srgb = ColorProfile::new_srgb();
    let (from, to) = if to_srgb {
        (&source, &srgb)
    } else {
        (&srgb, &source)
    };
    let converter = from
        .create_transform_8bit(Layout::Rgba, to, Layout::Rgba, TransformOptions::default())
        .map_err(|_| Error::Color)?;
    let mut output = image.clone();
    converter
        .transform(image.as_raw(), output.as_mut())
        .map_err(|_| Error::Color)?;
    for (source, target) in image.pixels().zip(output.pixels_mut()) {
        target[3] = source[3];
    }
    Ok(output)
}
fn validate_crop(rect: Rect) -> Result<()> {
    pixel_count(rect.width, rect.height)?;
    if !(-(MAX_EDGE as i32)..=MAX_EDGE as i32).contains(&rect.x)
        || !(-(MAX_EDGE as i32)..=MAX_EDGE as i32).contains(&rect.y)
    {
        return Err(Error::Invalid("crop origin"));
    }
    Ok(())
}
pub(crate) fn crop_padded(source: &EditImage, rect: Rect) -> Result<RgbaImage> {
    validate_crop(rect)?;
    Ok(RgbaImage::from_fn(rect.width, rect.height, |x, y| {
        let sx = (rect.x + x as i32).clamp(0, source.width as i32 - 1) as u32;
        let sy = (rect.y + y as i32).clamp(0, source.height as i32 - 1) as u32;
        Rgba(source.sample8(sx, sy))
    }))
}
pub(crate) fn resize(
    image: &RgbaImage,
    width: u32,
    height: u32,
    cancel: &dyn Cancellation,
) -> Result<RgbaImage> {
    check_cancel(cancel)?;
    pixel_count(width, height)?;
    if image.dimensions() == (width, height) {
        return Ok(image.clone());
    }
    let floats = image::ImageBuffer::<Rgba<f32>, Vec<f32>>::from_fn(
        image.width(),
        image.height(),
        |x, y| {
            let p = image.get_pixel(x, y);
            let a = f32::from(p[3]) / 255.0;
            Rgba([
                f32::from(p[0]) / 255.0 * a,
                f32::from(p[1]) / 255.0 * a,
                f32::from(p[2]) / 255.0 * a,
                a,
            ])
        },
    );
    let scaled = resample::resize(&floats, width, height, cancel)?;
    Ok(RgbaImage::from_fn(width, height, |x, y| {
        let p = scaled.get_pixel(x, y);
        let alpha = p[3];
        let channel = |i| {
            if alpha > 0.0 {
                libm::roundf(p[i] / alpha * 255.0).clamp(0.0, 255.0) as u8
            } else {
                0
            }
        };
        Rgba([
            channel(0),
            channel(1),
            channel(2),
            libm::roundf(alpha.clamp(0.0, 1.0) * 255.0) as u8,
        ])
    }))
}
pub(crate) fn area_mask(
    mask: &[u8],
    width: u32,
    height: u32,
    crop: Rect,
    sw: u32,
    sh: u32,
) -> Result<RgbaImage> {
    validate_crop(crop)?;
    pixel_count(sw, sh)?;
    if mask.len() != pixel_count(width, height)? {
        return Err(Error::Invalid("mask dimensions"));
    }
    // Every output cell overlaps at most ceil(scale)+1 source cells per axis.
    let work = u64::from(sw)
        * u64::from(sh)
        * (u64::from(crop.width.div_ceil(sw)) + 1)
        * (u64::from(crop.height.div_ceil(sh)) + 1);
    if work > 250_000_000 {
        return Err(Error::Limit("mask area resampling work"));
    }
    let sx = f64::from(crop.width) / f64::from(sw);
    let sy = f64::from(crop.height) / f64::from(sh);
    Ok(RgbaImage::from_fn(sw, sh, |x, y| {
        let left = f64::from(crop.x) + f64::from(x) * sx;
        let top = f64::from(crop.y) + f64::from(y) * sy;
        let right = left + sx;
        let bottom = top + sy;
        let mut total = 0.0;
        for yy in (libm::floor(top) as i32).max(0)..(libm::ceil(bottom) as i32).min(height as i32) {
            for xx in
                (libm::floor(left) as i32).max(0)..(libm::ceil(right) as i32).min(width as i32)
            {
                let area = (right.min(f64::from(xx + 1)) - left.max(f64::from(xx)))
                    * (bottom.min(f64::from(yy + 1)) - top.max(f64::from(yy)));
                total += f64::from(mask[yy as usize * width as usize + xx as usize]) * area;
            }
        }
        Rgba([
            0,
            0,
            0,
            255 - libm::round(total / (sx * sy)).clamp(0.0, 255.0) as u8,
        ])
    }))
}
pub(crate) fn composite(
    source: &EditImage,
    generated: &RgbaImage,
    crop: Rect,
    mask: &vw_mask::Mask,
    radius: u32,
    cancel: &dyn Cancellation,
) -> Result<EditImage> {
    validate_crop(crop)?;
    if generated.dimensions() != (crop.width, crop.height)
        || mask.size().width() != source.width
        || mask.size().height() != source.height
    {
        return Err(Error::Invalid("composite dimensions"));
    }
    let weights = mask.feather(radius)?.to_dense()?;
    check_cancel(cancel)?;
    let mut output = source.clone();
    for y in crop.y.max(0)..(crop.y + crop.height as i32).min(source.height as i32) {
        check_cancel(cancel)?;
        for x in crop.x.max(0)..(crop.x + crop.width as i32).min(source.width as i32) {
            let i = y as usize * source.width as usize + x as usize;
            let w = u32::from(weights[i]);
            if w == 0 {
                continue;
            }
            let b = generated.get_pixel((x - crop.x) as u32, (y - crop.y) as u32);
            match (&source.pixels, &mut output.pixels) {
                (Pixels::Rgba8(a), Pixels::Rgba8(out)) => {
                    for c in 0..4 {
                        out[i * 4 + c] =
                            ((u32::from(a[i * 4 + c]) * (255 - w) + u32::from(b[c]) * w + 127)
                                / 255) as u8;
                    }
                }
                (Pixels::Rgba16(a), Pixels::Rgba16(out)) => {
                    for c in 0..4 {
                        out[i * 4 + c] = ((u32::from(a[i * 4 + c]) * (255 - w)
                            + u32::from(b[c]) * 257 * w
                            + 127)
                            / 255) as u16;
                    }
                }
                _ => return Err(Error::Invalid("composite depth")),
            }
        }
    }
    Ok(output)
}
pub(crate) fn accept(
    source: &EditImage,
    result: &EditImage,
    selection: &vw_mask::Mask,
    cancel: &dyn Cancellation,
) -> Result<EditImage> {
    if source.width != result.width
        || source.height != result.height
        || source.bit_depth() != result.bit_depth()
        || source.icc != result.icc
        || selection.size().width() != source.width
        || selection.size().height() != source.height
    {
        return Err(Error::Invalid("acceptance dimensions or color binding"));
    }
    let values = selection.to_dense()?;
    let mut output = source.clone();
    for (i, &weight) in values.iter().enumerate() {
        if i % 4096 == 0 {
            check_cancel(cancel)?;
        }
        let w = u32::from(weight);
        if w == 0 {
            continue;
        }
        match (&source.pixels, &result.pixels, &mut output.pixels) {
            (Pixels::Rgba8(a), Pixels::Rgba8(b), Pixels::Rgba8(out)) => {
                for c in 0..4 {
                    out[i * 4 + c] =
                        ((u32::from(a[i * 4 + c]) * (255 - w) + u32::from(b[i * 4 + c]) * w + 127)
                            / 255) as u8;
                }
            }
            (Pixels::Rgba16(a), Pixels::Rgba16(b), Pixels::Rgba16(out)) => {
                for c in 0..4 {
                    out[i * 4 + c] =
                        ((u32::from(a[i * 4 + c]) * (255 - w) + u32::from(b[i * 4 + c]) * w + 127)
                            / 255) as u16;
                }
            }
            _ => return Err(Error::Invalid("acceptance depth")),
        }
    }
    Ok(output)
}
