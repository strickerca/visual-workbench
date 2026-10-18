//! Bounded embedded opaque PSD/PSB composites; no layer interpretation or writer.
//! Layout: Adobe Photoshop File Formats Specification (November 2019),
//! https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/ .
//! Raw and PackBits RGB/grayscale 8/16-bit only. Extra/transparent channels,
//! ZIP, CMYK/Lab, HDR and absent/declared dummy composites refuse explicitly.
use crate::admission::{alloc, profile, range};
use crate::{Error, Limits, Result, check};
use vw_model::AssetId;
use vw_raster::{DecodedImage, Pixels};

pub const FLATTENED_NOTICE: &str = "Flattened embedded composite only. Original layers remain in the unchanged source; layered round trips are unsupported.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Psd,
    Psb,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub kind: Kind,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub grayscale: bool,
    pub estimated_peak_bytes: u64,
}
pub struct Composite {
    pub image: DecodedImage,
    pub info: Info,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let b = range(self.bytes, self.at as u64, n as u64)?;
        self.at = self.at.checked_add(n).ok_or(Error::Limit)?;
        Ok(b)
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(
            self.take(2)?.try_into().map_err(|_| Error::Invalid)?,
        ))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().map_err(|_| Error::Invalid)?,
        ))
    }
    fn section(&mut self, wide: bool) -> Result<&'a [u8]> {
        let len = if wide {
            u64::from_be_bytes(self.take(8)?.try_into().map_err(|_| Error::Invalid)?)
        } else {
            u64::from(self.u32()?)
        };
        self.take(usize::try_from(len).map_err(|_| Error::Limit)?)
    }
}
struct Parsed<'a> {
    info: Info,
    profile: Option<&'a [u8]>,
    compression: u16,
    data: &'a [u8],
}

fn parse<'a>(bytes: &'a [u8], limits: Limits, cancel: &dyn Fn() -> bool) -> Result<Parsed<'a>> {
    check(cancel)?;
    limits.input(bytes.len())?;
    let mut r = Reader { bytes, at: 0 };
    if r.take(4)? != b"8BPS" {
        return Err(Error::Unsupported);
    }
    let kind = match r.u16()? {
        1 => Kind::Psd,
        2 => Kind::Psb,
        _ => return Err(Error::Unsupported),
    };
    if r.take(6)? != [0; 6] {
        return Err(Error::Invalid);
    }
    let channels = r.u16()?;
    let height = r.u32()?;
    let width = r.u32()?;
    if width > if kind == Kind::Psd { 30000 } else { 300000 }
        || height > if kind == Kind::Psd { 30000 } else { 300000 }
    {
        return Err(Error::Limit);
    }
    let bit_depth = match r.u16()? {
        8 => 8,
        16 => 16,
        _ => return Err(Error::Depth),
    };
    let grayscale = match r.u16()? {
        1 => true,
        3 => false,
        _ => return Err(Error::Color),
    };
    // Additional channels may be transparency (white-matted) or saved masks.
    // Never silently drop them or mistake a saved mask for straight alpha.
    if channels != if grayscale { 1 } else { 3 } {
        return Err(Error::Unsupported);
    }
    if !r.section(false)?.is_empty() {
        return Err(Error::Unsupported);
    }
    let resources = r.section(false)?;
    // Sections stay borrowed. Bound both work count and opaque metadata bytes.
    if resources.len() > 16 * 1024 * 1024 {
        return Err(Error::Limit);
    }
    let mut rr = Reader {
        bytes: resources,
        at: 0,
    };
    let mut icc = None;
    let mut merged = false;
    for _ in 0..4096 {
        if rr.at == resources.len() {
            break;
        }
        check(cancel)?;
        if rr.take(4)? != b"8BIM" {
            return Err(Error::Invalid);
        }
        let id = rr.u16()?;
        let n = usize::from(rr.take(1)?[0]);
        rr.take(n)?;
        if (n + 1) % 2 != 0 {
            rr.take(1)?;
        }
        let data = rr.section(false)?;
        if data.len() % 2 != 0 {
            rr.take(1)?;
        }
        match id {
            1039 => {
                if icc.is_some() {
                    return Err(Error::Color);
                }
                profile(data)?;
                if data.get(16..20) != Some(if grayscale { b"GRAY" } else { b"RGB " }) {
                    return Err(Error::Color);
                }
                icc = Some(data);
            }
            1057 => {
                if merged || data.len() < 5 || data[..4] != 1_u32.to_be_bytes() || data[4] != 1 {
                    return Err(Error::Unsupported);
                }
                merged = true;
            }
            // Photoshop's composite canvas already defines coordinates. Refuse
            // EXIF orientation other than identity rather than apply it twice.
            1058 | 1059 if crate::raw_preview::exif_orientation(data)? != 1 => {
                return Err(Error::Unsupported);
            }
            _ => {}
        }
    }
    if rr.at != resources.len() {
        return Err(Error::Limit);
    }
    // Skip exactly the length-delimited layer/mask section without parsing it.
    r.section(kind == Kind::Psb)?;
    if r.at == bytes.len() {
        return Err(Error::Unsupported);
    }
    let compression = r.u16()?;
    if compression > 1 {
        return Err(Error::Unsupported);
    }
    let peak = limits.admit(
        width,
        height,
        bytes.len() as u64,
        icc.map_or(0, <[u8]>::len),
    )?;
    let data = bytes.get(r.at..).ok_or(Error::Invalid)?;
    let rows = u64::from(height) * u64::from(channels);
    let row_bytes = u64::from(width) * u64::from(bit_depth / 8);
    if compression == 0 {
        if data.len() as u64 != rows * row_bytes {
            return Err(Error::Invalid);
        }
    } else {
        let mut table = Reader { bytes: data, at: 0 };
        let mut total = 0_u64;
        for i in 0..rows {
            if i % 128 == 0 {
                check(cancel)?;
            }
            let n = if kind == Kind::Psb {
                table.u32()?
            } else {
                u32::from(table.u16()?)
            };
            if n == 0 {
                return Err(Error::Invalid);
            }
            total = total.checked_add(u64::from(n)).ok_or(Error::Limit)?;
        }
        if total.checked_add(table.at as u64) != Some(data.len() as u64) {
            return Err(Error::Invalid);
        }
    }
    Ok(Parsed {
        info: Info {
            kind,
            width,
            height,
            bit_depth,
            grayscale,
            estimated_peak_bytes: peak,
        },
        profile: icc,
        compression,
        data,
    })
}

pub fn inspect(bytes: &[u8], limits: Limits, cancel: &dyn Fn() -> bool) -> Result<Info> {
    Ok(parse(bytes, limits, cancel)?.info)
}

/// Uses the existing format limits and original identity. Caller stores the
/// unchanged source and FLATTENED_NOTICE before adopting this derivative.
pub fn decode(bytes: &[u8], limits: Limits, cancel: &dyn Fn() -> bool) -> Result<Composite> {
    let parsed = parse(bytes, limits, cancel)?;
    let info = parsed.info;
    let width = info.width as usize;
    let height = info.height as usize;
    let channels = if info.grayscale { 1 } else { 3 };
    let row_bytes = width * usize::from(info.bit_depth / 8);
    let mut row = alloc::<u8>(row_bytes)?;
    let mut table = Reader {
        bytes: parsed.data,
        at: 0,
    };
    let table_bytes = if parsed.compression == 1 {
        height * channels * if info.kind == Kind::Psb { 4 } else { 2 }
    } else {
        0
    };
    let mut data = Reader {
        bytes: parsed.data,
        at: table_bytes,
    };
    let mut pixels = if info.bit_depth == 8 {
        Pixels::Rgba8(alloc(width * height * 4)?)
    } else {
        Pixels::Rgba16(alloc(width * height * 4)?)
    };
    for c in 0..channels {
        for y in 0..height {
            check(cancel)?;
            if parsed.compression == 0 {
                row.copy_from_slice(data.take(row_bytes)?);
            } else {
                let n = if info.kind == Kind::Psb {
                    table.u32()?
                } else {
                    u32::from(table.u16()?)
                };
                unpack(data.take(n as usize)?, &mut row, cancel)?;
            }
            for x in 0..width {
                let at = (y * width + x) * 4;
                match &mut pixels {
                    Pixels::Rgba8(p) => {
                        if info.grayscale {
                            p[at..at + 3].fill(row[x]);
                        } else {
                            p[at + c] = row[x];
                        }
                        p[at + 3] = 255;
                    }
                    Pixels::Rgba16(p) => {
                        let v = u16::from_be_bytes([row[x * 2], row[x * 2 + 1]]);
                        if info.grayscale {
                            p[at..at + 3].fill(v);
                        } else {
                            p[at + c] = v;
                        }
                        p[at + 3] = 65535;
                    }
                }
            }
        }
    }
    let image = DecodedImage {
        width: info.width,
        height: info.height,
        pixels,
        icc: parsed.profile.map(<[u8]>::to_vec),
        source_asset: AssetId::hash(bytes),
        original_available: true,
        orientation_applied: 1,
    };
    image.validate().map_err(|_| Error::Color)?;
    check(cancel)?;
    Ok(Composite { image, info })
}

fn unpack(input: &[u8], output: &mut [u8], cancel: &dyn Fn() -> bool) -> Result<()> {
    let mut r = Reader {
        bytes: input,
        at: 0,
    };
    let mut at = 0_usize;
    while r.at < input.len() {
        check(cancel)?;
        let control = r.take(1)?[0] as i8;
        if control == -128 {
            continue;
        }
        let n = if control >= 0 {
            control as usize + 1
        } else {
            (1 - i16::from(control)) as usize
        };
        let end = at.checked_add(n).ok_or(Error::Limit)?;
        let target = output.get_mut(at..end).ok_or(Error::Invalid)?;
        if control >= 0 {
            target.copy_from_slice(r.take(n)?);
        } else {
            target.fill(r.take(1)?[0]);
        }
        at = end;
    }
    if at != output.len() {
        return Err(Error::Invalid);
    }
    Ok(())
}
