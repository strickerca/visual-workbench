use crate::{Error, MAX_EDGE, MAX_FEATHER, MAX_PIXELS, Result, geometry::Rect, pixel_count};
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Rgba, RgbaImage};
use moxcms::{ColorProfile, DataColorSpace, Layout, TransformOptions};
use std::io::Cursor;

#[derive(Clone)]
pub struct SourceImage {
    pub pixels: RgbaImage,
    pub icc: Option<Vec<u8>>,
    pub orientation_applied: bool,
    pub assumed_srgb: bool,
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

pub fn decode(bytes: &[u8], allow_untagged_srgb: bool) -> Result<SourceImage> {
    if bytes.len() > 100_000_000 {
        return Err(Error::Limit("encoded image bytes"));
    }
    let format = image::guess_format(bytes)?;
    if !matches!(format, ImageFormat::Png | ImageFormat::Jpeg) {
        return Err(Error::Invalid("PNG or JPEG required"));
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(MAX_PIXELS * 8);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    pixel_count(width, height)?;
    if decoder.color_type().bits_per_pixel() / u16::from(decoder.color_type().channel_count()) > 8 {
        return Err(Error::Invalid(
            "this spike accepts 8-bit source pixels only; original was preserved",
        ));
    }
    let icc = decoder.icc_profile()?;
    if let Some(profile) = &icc {
        if profile.len() > 4_000_000 {
            return Err(Error::Limit("ICC profile bytes"));
        }
        let parsed = ColorProfile::new_from_slice(profile).map_err(|_| Error::Color)?;
        if parsed.color_space != DataColorSpace::Rgb {
            return Err(Error::Invalid("RGB ICC profile required"));
        }
    } else if !allow_untagged_srgb {
        return Err(Error::Invalid(
            "untagged input requires explicit assume-srgb",
        ));
    }
    let orientation = decoder.orientation()?;
    let orientation_applied = orientation != image::metadata::Orientation::NoTransforms;
    let mut decoded = DynamicImage::from_decoder(decoder)?;
    decoded.apply_orientation(orientation);
    Ok(SourceImage {
        pixels: decoded.to_rgba8(),
        assumed_srgb: icc.is_none(),
        icc,
        orientation_applied,
    })
}

pub fn encode_png(image: &RgbaImage, icc: Option<&[u8]>) -> Result<Vec<u8>> {
    pixel_count(image.width(), image.height())?;
    let profile = output_profile(icc)?;
    let mut output = Vec::new();
    let mut encoder = image::codecs::png::PngEncoder::new(&mut output);
    encoder.set_icc_profile(profile).map_err(|_| Error::Color)?;
    encoder.write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(output)
}

/// Exact profile written to every normalized source/composite PNG, including
/// inputs that explicitly assumed sRGB and did not originally have an ICC tag.
pub fn output_profile(icc: Option<&[u8]>) -> Result<Vec<u8>> {
    if let Some(profile) = icc {
        return Ok(profile.to_vec());
    }
    let mut profile = ColorProfile::new_srgb()
        .encode()
        .map_err(|_| Error::Color)?;
    // moxcms 0.8.1's encoder writes the current time into the ICC header even
    // when the profile's date is set. A generated standard profile needs stable
    // bytes so preparing and later confirming the same request has the same ID.
    // ICC dateTimeNumber is six big-endian u16 values; use 2000-01-01 00:00:00.
    // Its profile ID is unset, so there is no header digest to invalidate.
    profile
        .get_mut(24..36)
        .ok_or(Error::Color)?
        .copy_from_slice(&[0x07, 0xd0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0]);
    Ok(profile)
}

pub fn transform(image: &RgbaImage, profile: Option<&[u8]>, to_srgb: bool) -> Result<RgbaImage> {
    let Some(bytes) = profile else {
        return Ok(image.clone());
    };
    let source = ColorProfile::new_from_slice(bytes).map_err(|_| Error::Color)?;
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
    // Profile transforms affect color; retain original alpha exactly.
    for (src, dst) in image.pixels().zip(output.pixels_mut()) {
        dst[3] = src[3];
    }
    Ok(output)
}

pub fn crop_padded(image: &RgbaImage, rect: Rect) -> Result<RgbaImage> {
    validate_crop(rect)?;
    pixel_count(image.width(), image.height())?;
    Ok(RgbaImage::from_fn(rect.width, rect.height, |x, y| {
        let sx = (rect.x + x as i32).clamp(0, image.width() as i32 - 1) as u32;
        let sy = (rect.y + y as i32).clamp(0, image.height() as i32 - 1) as u32;
        *image.get_pixel(sx, sy)
    }))
}

/// Lanczos3 in the source color encoding, with premultiplication to avoid hidden
/// transparent RGB fringes. Unchanged exterior pixels never enter this routine.
pub fn resize(image: &RgbaImage, width: u32, height: u32) -> Result<RgbaImage> {
    pixel_count(width, height)?;
    if image.dimensions() == (width, height) {
        return Ok(image.clone());
    }
    // image::imageops filters clamp float channels to their normalized 0..1 range.
    // Keep premultiplied RGB normalized too; byte-scaled floats would clip to near black.
    let floats = image::ImageBuffer::<Rgba<f32>, Vec<f32>>::from_fn(
        image.width(),
        image.height(),
        |x, y| {
            let p = image.get_pixel(x, y);
            let alpha = f32::from(p[3]) / 255.0;
            Rgba([
                f32::from(p[0]) / 255.0 * alpha,
                f32::from(p[1]) / 255.0 * alpha,
                f32::from(p[2]) / 255.0 * alpha,
                alpha,
            ])
        },
    );
    let scaled = image::imageops::resize(
        &floats,
        width,
        height,
        image::imageops::FilterType::Lanczos3,
    );
    Ok(RgbaImage::from_fn(width, height, |x, y| {
        let p = scaled.get_pixel(x, y);
        let alpha = p[3].clamp(0.0, 1.0);
        let channel = |index| {
            if alpha > 0.0 {
                (p[index] / alpha * 255.0).round().clamp(0.0, 255.0) as u8
            } else {
                0
            }
        };
        Rgba([
            channel(0),
            channel(1),
            channel(2),
            (alpha * 255.0).round() as u8,
        ])
    }))
}

pub fn rectangle_mask(width: u32, height: u32, rect: Rect) -> Result<Vec<u8>> {
    let mut mask = vec![0; pixel_count(width, height)?];
    if rect.x < 0
        || rect.y < 0
        || rect.width == 0
        || rect.height == 0
        || i64::from(rect.x) + i64::from(rect.width) > i64::from(width)
        || i64::from(rect.y) + i64::from(rect.height) > i64::from(height)
    {
        return Err(Error::Invalid(
            "mask rectangle lies outside the oriented source",
        ));
    }
    for y in rect.y as u32..rect.y as u32 + rect.height {
        let start = (y * width + rect.x as u32) as usize;
        mask[start..start + rect.width as usize].fill(255);
    }
    Ok(mask)
}

pub fn decode_mask(bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    pixel_count(width, height)?;
    if bytes.len() > 100_000_000 {
        return Err(Error::Limit("mask bytes"));
    }
    if image::guess_format(bytes)? != ImageFormat::Png {
        return Err(Error::Invalid("mask must be PNG"));
    }
    let decoder = image::codecs::png::PngDecoder::new(Cursor::new(bytes))?;
    if decoder.dimensions() != (width, height) || !decoder.color_type().has_alpha() {
        return Err(Error::Invalid(
            "mask needs matching dimensions and an alpha channel",
        ));
    }
    let source = decode(bytes, true)?;
    if source.orientation_applied {
        return Err(Error::Invalid(
            "mask orientation must already match document coordinates",
        ));
    }
    Ok(source.pixels.pixels().map(|p| 255 - p[3]).collect())
}

pub fn union_masks(masks: &[Vec<u8>], width: u32, height: u32) -> Result<Vec<u8>> {
    let count = pixel_count(width, height)?;
    let mut result = vec![0; count];
    for mask in masks {
        if mask.len() != count {
            return Err(Error::Invalid("mask union dimensions"));
        }
        for (out, value) in result.iter_mut().zip(mask) {
            *out = (*out).max(*value);
        }
    }
    crate::geometry::mask_bounds(&result, width, height)?;
    Ok(result)
}

/// Exact source-cell overlap area; no Lanczos ringing or thresholding of masks.
pub fn area_mask(
    mask: &[u8],
    width: u32,
    height: u32,
    crop: Rect,
    sw: u32,
    sh: u32,
) -> Result<RgbaImage> {
    validate_crop(crop)?;
    if mask.len() != pixel_count(width, height)? {
        return Err(Error::Invalid("mask dimensions"));
    }
    pixel_count(sw, sh)?;
    let sx = f64::from(crop.width) / f64::from(sw);
    let sy = f64::from(crop.height) / f64::from(sh);
    Ok(RgbaImage::from_fn(sw, sh, |x, y| {
        let left = f64::from(crop.x) + f64::from(x) * sx;
        let top = f64::from(crop.y) + f64::from(y) * sy;
        let right = left + sx;
        let bottom = top + sy;
        let mut total = 0.0;
        for yy in (top.floor() as i32).max(0)..(bottom.ceil() as i32).min(height as i32) {
            for xx in (left.floor() as i32).max(0)..(right.ceil() as i32).min(width as i32) {
                let area = (right.min(f64::from(xx + 1)) - left.max(f64::from(xx)))
                    * (bottom.min(f64::from(yy + 1)) - top.max(f64::from(yy)));
                total += f64::from(mask[yy as usize * width as usize + xx as usize]) * area;
            }
        }
        let edit = (total / (sx * sy)).round().clamp(0.0, 255.0) as u8;
        Rgba([0, 0, 0, 255 - edit])
    }))
}

pub fn mask_png(mask: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    if mask.len() != pixel_count(width, height)? {
        return Err(Error::Invalid("mask dimensions"));
    }
    let image = RgbaImage::from_fn(width, height, |x, y| {
        Rgba([0, 0, 0, 255 - mask[(y * width + x) as usize]])
    });
    encode_png(&image, None)
}

/// Source-grid circular box feather, radius r, zero extension at image edges.
/// Its compact disk support is strictly inside the proof's radius r+1 disk.
pub fn feather(mask: &[u8], width: u32, height: u32, radius: u32) -> Result<Vec<f32>> {
    if mask.len() != pixel_count(width, height)? || radius > MAX_FEATHER {
        return Err(Error::Invalid("feather parameters"));
    }
    if radius == 0 {
        return Ok(mask.iter().map(|v| f32::from(*v) / 255.0).collect());
    }
    let bounds = crate::geometry::mask_bounds(mask, width, height)?;
    let prefix = row_prefix(mask, width, height);
    let spans = disk_spans(radius);
    let area: u32 = spans.iter().map(|(_, half)| (half * 2 + 1) as u32).sum();
    let mut out = vec![0.0; mask.len()];
    let radius = radius as i32;
    for y in
        (bounds.y - radius).max(0)..(bounds.y + bounds.height as i32 + radius).min(height as i32)
    {
        for x in
            (bounds.x - radius).max(0)..(bounds.x + bounds.width as i32 + radius).min(width as i32)
        {
            let sum = spans
                .iter()
                .map(|(dy, half)| row_sum(&prefix, width, height, y + dy, x - half, x + half + 1))
                .sum::<u32>();
            out[y as usize * width as usize + x as usize] = sum as f32 / (area as f32 * 255.0);
        }
    }
    Ok(out)
}

pub fn dilate(mask: &[u8], width: u32, height: u32, radius: u32) -> Result<Vec<bool>> {
    if mask.len() != pixel_count(width, height)? || radius > MAX_FEATHER + 1 {
        return Err(Error::Invalid("dilation parameters"));
    }
    let bounds = crate::geometry::mask_bounds(mask, width, height)?;
    let prefix = row_prefix(mask, width, height);
    let spans = disk_spans(radius);
    let mut out = vec![false; mask.len()];
    let radius = radius as i32;
    for y in
        (bounds.y - radius).max(0)..(bounds.y + bounds.height as i32 + radius).min(height as i32)
    {
        for x in
            (bounds.x - radius).max(0)..(bounds.x + bounds.width as i32 + radius).min(width as i32)
        {
            out[y as usize * width as usize + x as usize] = spans.iter().any(|(dy, half)| {
                row_sum(&prefix, width, height, y + dy, x - half, x + half + 1) != 0
            });
        }
    }
    Ok(out)
}

fn row_prefix(mask: &[u8], width: u32, height: u32) -> Vec<u32> {
    let stride = width as usize + 1;
    let mut prefix = vec![0; stride * height as usize];
    for y in 0..height as usize {
        for x in 0..width as usize {
            prefix[y * stride + x + 1] =
                prefix[y * stride + x] + u32::from(mask[y * width as usize + x]);
        }
    }
    prefix
}

fn row_sum(prefix: &[u32], width: u32, height: u32, y: i32, left: i32, right: i32) -> u32 {
    if y < 0 || y >= height as i32 {
        return 0;
    }
    let row = y as usize * (width as usize + 1);
    let l = left.clamp(0, width as i32) as usize;
    let r = right.clamp(0, width as i32) as usize;
    prefix[row + r] - prefix[row + l]
}

fn disk_spans(radius: u32) -> Vec<(i32, i32)> {
    let r = radius as i32;
    (-r..=r)
        .map(|dy| (dy, f64::from(r * r - dy * dy).sqrt().floor() as i32))
        .collect()
}

pub fn composite(
    source: &RgbaImage,
    generated: &RgbaImage,
    crop: Rect,
    mask: &[u8],
    radius: u32,
) -> Result<RgbaImage> {
    validate_crop(crop)?;
    if generated.dimensions() != (crop.width, crop.height) {
        return Err(Error::Invalid("returned crop dimensions"));
    }
    let weights = feather(mask, source.width(), source.height(), radius)?;
    let mut output = source.clone();
    for y in crop.y.max(0)..(crop.y + crop.height as i32).min(source.height() as i32) {
        for x in crop.x.max(0)..(crop.x + crop.width as i32).min(source.width() as i32) {
            let weight = weights[y as usize * source.width() as usize + x as usize];
            if weight == 0.0 {
                continue;
            }
            let a = source.get_pixel(x as u32, y as u32);
            let b = generated.get_pixel((x - crop.x) as u32, (y - crop.y) as u32);
            let pixel = output.get_pixel_mut(x as u32, y as u32);
            for channel in 0..4 {
                pixel[channel] = (weight * f32::from(b[channel])
                    + (1.0 - weight) * f32::from(a[channel]))
                .round()
                .clamp(0.0, 255.0) as u8;
            }
        }
    }
    Ok(output)
}
