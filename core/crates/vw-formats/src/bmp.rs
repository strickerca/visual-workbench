use crate::admission::{alloc, le16, le32, profile, range};
use crate::{Error, Format, Header, Limits, Notice, Result, check};
use image::ImageDecoder;
use vw_model::AssetId;
use vw_raster::{DecodedImage, Pixels};

pub(crate) fn inspect(bytes: &[u8], limits: Limits) -> Result<Header> {
    if bytes.get(..2) != Some(b"BM")
        || u64::from(le32(bytes, 2)?) != bytes.len() as u64
        || bytes.get(6..10) != Some(&[0; 4])
    {
        return Err(Error::Invalid);
    }
    let offset = u64::from(le32(bytes, 10)?);
    let dib = le32(bytes, 14)?;
    if !matches!(dib, 12 | 40 | 52 | 56 | 108 | 124) {
        return Err(Error::Unsupported);
    }
    range(bytes, 14, u64::from(dib))?;
    let (width, height, bpp, compression, colors) = if dib == 12 {
        if le16(bytes, 22)? != 1 {
            return Err(Error::Invalid);
        }
        (
            u32::from(le16(bytes, 18)?),
            u32::from(le16(bytes, 20)?),
            le16(bytes, 24)?,
            0,
            0,
        )
    } else {
        let width = le32(bytes, 18)? as i32;
        let height = le32(bytes, 22)? as i32;
        if width <= 0 || height == i32::MIN || height == 0 || le16(bytes, 26)? != 1 {
            return Err(Error::Invalid);
        }
        (
            width as u32,
            height.unsigned_abs(),
            le16(bytes, 28)?,
            le32(bytes, 30)?,
            le32(bytes, 46)?,
        )
    };
    if !matches!(bpp, 1 | 4 | 8 | 16 | 24 | 32) || !matches!(compression, 0..=3) {
        return Err(Error::Unsupported);
    }
    if (compression == 1 && bpp != 8)
        || (compression == 2 && bpp != 4)
        || (compression == 3 && !matches!(bpp, 16 | 32))
    {
        return Err(Error::Invalid);
    }
    let mask_bytes = if compression == 3 && dib == 40 { 12 } else { 0 };
    let mut alpha = false;
    if compression == 3 {
        let n = if dib >= 56 { 4 } else { 3 };
        let mut seen = 0_u32;
        for i in 0..n {
            let mask = le32(bytes, 54 + i * 4)?;
            if i < 3 && mask == 0 {
                return Err(Error::Invalid);
            }
            if mask == 0 {
                continue;
            }
            let shifted = mask >> mask.trailing_zeros();
            if mask.count_ones() > 8 {
                return Err(Error::Depth);
            }
            if mask & seen != 0
                || shifted & shifted.wrapping_add(1) != 0
                || (bpp == 16 && mask >> 16 != 0)
            {
                return Err(Error::Invalid);
            }
            seen |= mask;
            if i == 3 {
                alpha = true;
            }
        }
    }
    let palette = if bpp <= 8 {
        if colors == 0 {
            1_u64 << bpp
        } else {
            if colors > 1_u32 << bpp {
                return Err(Error::Invalid);
            }
            u64::from(colors)
        }
    } else {
        if colors > 256 {
            return Err(Error::Limit);
        }
        u64::from(colors)
    };
    let minimum = 14 + u64::from(dib) + mask_bytes + palette * if dib == 12 { 3 } else { 4 };
    if offset < minimum || offset > bytes.len() as u64 {
        return Err(Error::Invalid);
    }
    let stride = (u64::from(width) * u64::from(bpp)).div_ceil(32) * 4;
    let pixel_end = if compression == 0 || compression == 3 {
        let end = offset
            .checked_add(stride.checked_mul(u64::from(height)).ok_or(Error::Limit)?)
            .ok_or(Error::Limit)?;
        range(bytes, offset, end - offset)?;
        end
    } else {
        let size = u64::from(le32(bytes, 34)?);
        if size == 0 {
            return Err(Error::Invalid);
        }
        range(bytes, offset, size)?;
        offset + size
    };
    let mut icc_range = None;
    let mut tagged_srgb = false;
    if dib >= 108 {
        match le32(bytes, 70)? {
            0x7352_4742 => tagged_srgb = true,
            0x5769_6e20 => {} // Windows default: retain untagged provenance.
            0x4d42_4544 if dib == 124 => {
                let start = 14_u64 + u64::from(le32(bytes, 126)?);
                let size = u64::from(le32(bytes, 130)?);
                if start < pixel_end || size == 0 || size > 4 * 1024 * 1024 {
                    return Err(Error::Color);
                }
                let p = range(bytes, start, size)?;
                profile(p)?;
                icc_range = Some(p);
            }
            // Linked profiles would turn metadata into local path access.
            0x4c49_4e4b => return Err(Error::Color),
            _ => return Err(Error::Color),
        }
    }
    let peak = limits.admit(
        width,
        height,
        bytes.len() as u64,
        icc_range.map_or(0, <[u8]>::len),
    )?;
    let icc = if let Some(p) = icc_range {
        Some(p.to_vec())
    } else if tagged_srgb {
        Some(vw_raster::standard_srgb_profile().map_err(|_| Error::Color)?)
    } else {
        None
    };
    Ok(Header {
        format: Format::Bmp,
        source_asset: AssetId::hash(bytes),
        width,
        height,
        bit_depth: 8,
        orientation: 1,
        alpha,
        encoded_bytes: bytes.len() as u64,
        notices: if icc.is_none() {
            vec![Notice::UntaggedColor]
        } else {
            Vec::new()
        },
        icc,
        estimated_peak_bytes: peak,
        samples: if alpha { 4 } else { 3 },
        planar: false,
    })
}

pub(crate) fn decode(
    bytes: &[u8],
    h: &Header,
    limits: Limits,
    cancel: &dyn Fn() -> bool,
) -> Result<Pixels> {
    check(cancel)?;
    let input = crate::io::Input::new(bytes, cancel);
    let opened = image::codecs::bmp::BmpDecoder::new(input);
    check(cancel)?;
    let mut decoder = opened.map_err(|_| Error::Codec)?;
    let mut bounds = image::Limits::default();
    bounds.max_image_width = Some(h.width);
    bounds.max_image_height = Some(h.height);
    bounds.max_alloc = Some(u64::from(h.width) * u64::from(h.height) * 8 + 1024 * 1024);
    decoder.set_limits(bounds).map_err(|_| Error::Limit)?;
    if decoder.dimensions() != (h.width, h.height) {
        return Err(Error::Invalid);
    }
    let channels = match decoder.color_type() {
        image::ColorType::Rgb8 => 3,
        image::ColorType::Rgba8 => 4,
        _ => return Err(Error::Depth),
    };
    if h.alpha && channels != 4 {
        return Err(Error::Codec);
    }
    limits.admit(
        h.width,
        h.height,
        bytes.len() as u64,
        h.icc.as_ref().map_or(0, Vec::len),
    )?;
    let n = h.width as usize * h.height as usize;
    let mut raw = alloc(n * channels)?;
    let result = decoder.read_image(&mut raw);
    check(cancel)?;
    result.map_err(|_| Error::Codec)?;
    if channels == 4 {
        return Ok(Pixels::Rgba8(raw));
    }
    let mut rgba = alloc(n * 4)?;
    for (y, (src, dst)) in raw
        .chunks_exact(h.width as usize * 3)
        .zip(rgba.chunks_exact_mut(h.width as usize * 4))
        .enumerate()
    {
        if y % 16 == 0 {
            check(cancel)?;
        }
        for (a, b) in src
            .as_chunks::<3>()
            .0
            .iter()
            .zip(dst.as_chunks_mut::<4>().0.iter_mut())
        {
            b[..3].copy_from_slice(a);
            b[3] = 255;
        }
    }
    Ok(Pixels::Rgba8(rgba))
}

pub(crate) fn encode(image: &DecodedImage, out: &mut crate::io::Output<'_>) -> Result<()> {
    let Pixels::Rgba8(pixels) = &image.pixels else {
        return Err(Error::Depth);
    };
    if image
        .icc
        .as_ref()
        .is_some_and(|p| p.get(16..20) != Some(b"RGB "))
    {
        return Err(Error::Color);
    }
    let pixel_bytes = u64::from(image.width) * u64::from(image.height) * 4;
    let profile_bytes = image.icc.as_ref().map_or(0, Vec::len) as u64;
    let total = 138_u64
        .checked_add(pixel_bytes)
        .and_then(|n| n.checked_add(profile_bytes))
        .ok_or(Error::Limit)?;
    if total > u64::from(u32::MAX) {
        return Err(Error::Limit);
    }
    let mut h = [0_u8; 138];
    h[..2].copy_from_slice(b"BM");
    h[2..6].copy_from_slice(&(total as u32).to_le_bytes());
    h[10..14].copy_from_slice(&138_u32.to_le_bytes());
    h[14..18].copy_from_slice(&124_u32.to_le_bytes());
    h[18..22].copy_from_slice(&image.width.to_le_bytes());
    h[22..26].copy_from_slice(&(-(image.height as i32)).to_le_bytes());
    h[26..28].copy_from_slice(&1_u16.to_le_bytes());
    h[28..30].copy_from_slice(&32_u16.to_le_bytes());
    h[30..34].copy_from_slice(&3_u32.to_le_bytes());
    h[34..38].copy_from_slice(&(pixel_bytes as u32).to_le_bytes());
    for (at, mask) in [
        (54, 0x00ff0000_u32),
        (58, 0x0000ff00),
        (62, 0x000000ff),
        (66, 0xff000000),
    ] {
        h[at..at + 4].copy_from_slice(&mask.to_le_bytes());
    }
    h[70..74].copy_from_slice(
        &(if image.icc.is_some() {
            0x4d42_4544_u32
        } else {
            0x5769_6e20
        })
        .to_le_bytes(),
    );
    h[122..126].copy_from_slice(&4_u32.to_le_bytes());
    if image.icc.is_some() {
        h[126..130].copy_from_slice(&((124 + pixel_bytes) as u32).to_le_bytes());
        h[130..134].copy_from_slice(&(profile_bytes as u32).to_le_bytes());
    }
    out.bytes(&h)?;
    let mut row = alloc(image.width as usize * 4)?;
    for src in pixels.chunks_exact(image.width as usize * 4) {
        for (a, b) in src
            .as_chunks::<4>()
            .0
            .iter()
            .zip(row.as_chunks_mut::<4>().0.iter_mut())
        {
            b.copy_from_slice(&[a[2], a[1], a[0], a[3]]);
        }
        out.bytes(&row)?;
    }
    if let Some(icc) = &image.icc {
        out.bytes(icc)?;
    }
    Ok(())
}
