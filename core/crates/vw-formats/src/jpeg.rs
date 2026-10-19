//! TIFF's JPEG reader decodes into an owned buffer before checking TIFF extent.
//! Admit the complete single-scan baseline stream, not just its first SOF, and
//! refuse any color interpretation requiring an implicit conversion.
use crate::{Error, Result};

fn be16(bytes: &[u8]) -> Result<u16> {
    Ok(u16::from_be_bytes(
        bytes
            .get(..2)
            .ok_or(Error::Invalid)?
            .try_into()
            .map_err(|_| Error::Invalid)?,
    ))
}

pub(crate) fn tables(bytes: &[u8]) -> Result<()> {
    if bytes.len() > 65536 || !bytes.starts_with(&[0xff, 0xd8]) {
        return Err(Error::Invalid);
    }
    let mut at = 2;
    for _ in 0..1024 {
        let (marker, payload) = segment(bytes, &mut at)?;
        match marker {
            0xd9 if at == bytes.len() => return Ok(()),
            0xdb | 0xc4 => {
                if payload.is_empty() {
                    return Err(Error::Invalid);
                }
            }
            0xdd if payload.len() == 2 => {}
            _ => return Err(Error::Unsupported),
        }
    }
    Err(Error::Limit)
}

fn segment<'a>(bytes: &'a [u8], at: &mut usize) -> Result<(u8, &'a [u8])> {
    if bytes.get(*at) != Some(&0xff) {
        return Err(Error::Invalid);
    }
    while bytes.get(*at) == Some(&0xff) {
        *at += 1;
    }
    let marker = *bytes.get(*at).ok_or(Error::Invalid)?;
    *at += 1;
    if marker == 0xd9 {
        return Ok((marker, &[]));
    }
    if marker == 0 || marker == 0xd8 || (0xd0..=0xd7).contains(&marker) {
        return Err(Error::Invalid);
    }
    let len = usize::from(be16(bytes.get(*at..).ok_or(Error::Invalid)?)?);
    if len < 2 {
        return Err(Error::Invalid);
    }
    let end = at.checked_add(len).ok_or(Error::Invalid)?;
    let payload = bytes.get(*at + 2..end).ok_or(Error::Invalid)?;
    *at = end;
    Ok((marker, payload))
}

pub(crate) fn image(bytes: &[u8], width: u32, height: u32, samples: u8) -> Result<()> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return Err(Error::Invalid);
    }
    let mut at = 2;
    let mut frame = false;
    let mut ids = [0_u8; 3];
    for _ in 0..1024 {
        let (marker, p) = segment(bytes, &mut at)?;
        match marker {
            0xc0 => {
                if frame
                    || p.len() != 6 + usize::from(samples) * 3
                    || p[0] != 8
                    || p[5] != samples
                    || !matches!(samples, 1 | 3)
                {
                    return Err(Error::Unsupported);
                }
                if u32::from(be16(&p[1..])?) != height || u32::from(be16(&p[3..])?) != width {
                    return Err(Error::Invalid);
                }
                for i in 0..usize::from(samples) {
                    ids[i] = p[6 + i * 3];
                    if p[7 + i * 3] != 0x11 || p[8 + i * 3] > 3 {
                        return Err(Error::Unsupported);
                    }
                }
                // The pinned TIFF adapter disables JPEG's output color conversion.
                // Only explicit RGB identifiers can be interpreted as RGB here.
                if samples == 3 && ids != *b"RGB" {
                    return Err(Error::Color);
                }
                frame = true;
            }
            0xda => {
                if !frame || p.len() != 4 + usize::from(samples) * 2 || p[0] != samples {
                    return Err(Error::Unsupported);
                }
                for i in 0..usize::from(samples) {
                    if p[1 + i * 2] != ids[i] || p[2 + i * 2] & 0xcc != 0 {
                        return Err(Error::Unsupported);
                    }
                }
                if p[p.len() - 3..] != [0, 63, 0] {
                    return Err(Error::Unsupported);
                }
                // Entropy scanning accepts stuffing/restarts, but never a second
                // frame/scan that could revise extent, channels or allocation.
                while at < bytes.len() {
                    if bytes[at] != 0xff {
                        at += 1;
                        continue;
                    }
                    at += 1;
                    while bytes.get(at) == Some(&0xff) {
                        at += 1;
                    }
                    let next = *bytes.get(at).ok_or(Error::Invalid)?;
                    at += 1;
                    match next {
                        0 | 0xd0..=0xd7 => {}
                        0xd9 if at == bytes.len() => return Ok(()),
                        _ => return Err(Error::Unsupported),
                    }
                }
                return Err(Error::Invalid);
            }
            0xdb | 0xc4 => {
                if p.is_empty() {
                    return Err(Error::Invalid);
                }
            }
            0xdd if p.len() == 2 => {}
            0xe0 => {
                if samples == 3 && p.starts_with(b"JFIF\0") {
                    return Err(Error::Color);
                }
            }
            0xee => {
                if p.starts_with(b"Adobe") && (p.len() != 12 || p[11] != 0) {
                    return Err(Error::Color);
                }
            }
            0xe1..=0xed | 0xef | 0xfe => {}
            _ => return Err(Error::Unsupported),
        }
    }
    Err(Error::Limit)
}
