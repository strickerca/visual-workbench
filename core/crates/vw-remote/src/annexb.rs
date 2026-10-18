//! Bounded Annex B HEVC NAL admission. Does not claim decoder conformance.
use crate::{Error, MAX_ACCESS_UNIT, MAX_CONFIG, MAX_SIDE, Result};
pub struct Unit<'a> {
    pub idr: bool,
    pub vps: Option<&'a [u8]>,
    pub sps: Option<&'a [u8]>,
    pub pps: Option<&'a [u8]>,
}
fn prefix(bytes: &[u8], at: usize) -> Option<usize> {
    if bytes.get(at..at.checked_add(4)?) == Some(&[0, 0, 0, 1]) {
        Some(4)
    } else if bytes.get(at..at.checked_add(3)?) == Some(&[0, 0, 1]) {
        Some(3)
    } else {
        None
    }
}
pub fn access_unit(bytes: &[u8]) -> Result<Unit<'_>> {
    if bytes.is_empty() || bytes.len() > MAX_ACCESS_UNIT {
        return Err(Error::Limit);
    }
    let mut out = Unit {
        idr: false,
        vps: None,
        sps: None,
        pps: None,
    };
    let mut at = 0;
    let mut count = 0;
    let mut vcl = 0;
    let mut non_idr = false;
    while at < bytes.len() {
        let start = at;
        let n = prefix(bytes, at).ok_or(Error::Invalid)?;
        at += n;
        let a = *bytes.get(at).ok_or(Error::Invalid)?;
        let b = *bytes.get(at + 1).ok_or(Error::Invalid)?;
        if a & 128 != 0 || b & 7 == 0 || (a & 1) != 0 || b & 0xf8 != 0 {
            return Err(Error::Invalid);
        } // one base layer only
        let kind = (a >> 1) & 63;
        let body = at;
        at += 2;
        while at < bytes.len() && prefix(bytes, at).is_none() {
            at += 1
        }
        if at <= body + 2 {
            return Err(Error::Invalid);
        }
        count += 1;
        if count > 256 {
            return Err(Error::Limit);
        }
        let nal = &bytes[start..at];
        match kind {
            0..=31 => {
                vcl += 1;
                if matches!(kind, 19 | 20) {
                    out.idr = true
                } else if matches!(kind,16..=18|21..=31) {
                    return Err(Error::Unavailable);
                } else {
                    non_idr = true
                }
            }
            32 => {
                if out.vps.replace(nal).is_some() {
                    return Err(Error::Invalid);
                }
            }
            33 => {
                if out.sps.replace(nal).is_some() {
                    return Err(Error::Invalid);
                }
            }
            34 => {
                if out.pps.replace(nal).is_some() {
                    return Err(Error::Invalid);
                }
            }
            35 | 39 | 40 => {}
            _ => return Err(Error::Unavailable),
        }
    }
    if vcl == 0 || out.idr && non_idr {
        return Err(Error::Invalid);
    }
    Ok(out)
}
pub fn config(bytes: &[u8], kind: u8) -> Result<()> {
    if bytes.len() > MAX_CONFIG {
        return Err(Error::Limit);
    }
    let n = prefix(bytes, 0).ok_or(Error::Invalid)?;
    if bytes.len() <= n + 2
        || bytes[n] & 128 != 0
        || (bytes[n] >> 1) & 63 != kind
        || bytes[n] & 1 != 0
        || bytes[n + 1] & 0xf8 != 0
        || bytes[n + 1] & 7 == 0
    {
        return Err(Error::Invalid);
    }
    if (n + 2..bytes.len()).any(|at| prefix(bytes, at).is_some()) {
        return Err(Error::Invalid);
    }
    Ok(())
}
/// Deliberate 8-bit Main / 4:2:0 / one-temporal-layer subset. Reject dimensions
/// and bit depth before handing peer bytes to a platform decoder.
pub fn sps(bytes: &[u8], width: u32, height: u32) -> Result<()> {
    config(bytes, 33)?;
    let start = prefix(bytes, 0).ok_or(Error::Invalid)? + 2;
    let mut rbsp = Vec::new();
    rbsp.try_reserve_exact(bytes.len() - start)
        .map_err(|_| Error::Limit)?;
    let mut zeros = 0;
    for (index, &byte) in bytes[start..].iter().enumerate() {
        if zeros >= 2 && byte == 3 {
            if bytes.get(start + index + 1).is_none_or(|next| *next > 3) {
                return Err(Error::Invalid);
            }
            zeros = 0;
            continue;
        }
        rbsp.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    let mut b = Bits {
        bytes: &rbsp,
        at: 0,
    };
    let _ = b.read(4)?;
    let layers = b.read(3)?;
    let _ = b.read(1)?;
    if layers != 0 {
        return Err(Error::Unavailable);
    }
    let profile_space = b.read(2)?;
    let _tier = b.read(1)?;
    let profile = b.read(5)?;
    if profile_space != 0 || profile != 1 {
        return Err(Error::Unavailable);
    }
    let _compatibility = b.read(32)?;
    b.skip(48)?;
    let _level = b.read(8)?;
    let _id = b.ue()?;
    if b.ue()? != 1 {
        return Err(Error::Unavailable);
    } // YUV 4:2:0
    let picture_width = b.ue()?;
    let picture_height = b.ue()?;
    if !(2..=MAX_SIDE).contains(&width)
        || !(2..=MAX_SIDE).contains(&height)
        || !width.is_multiple_of(2)
        || !height.is_multiple_of(2)
        || !(8..=MAX_SIDE).contains(&picture_width)
        || !(8..=MAX_SIDE).contains(&picture_height)
        || !picture_width.is_multiple_of(8)
        || !picture_height.is_multiple_of(8)
    {
        return Err(Error::Invalid);
    }
    let (right, bottom) = if b.read(1)? != 0 {
        let left = b.ue()?;
        let right = b.ue()?;
        let top = b.ue()?;
        let bottom = b.ue()?;
        // Preserve the selected client's origin. Only encoder padding at the
        // right/bottom is supported, bounded below one 64-pixel HEVC CTB.
        if left != 0 || top != 0 || right >= 32 || bottom >= 32 {
            return Err(Error::Unavailable);
        }
        (right, bottom)
    } else {
        (0, 0)
    };
    // H.265 7.4.3.2: Main 4:2:0 conformance offsets use two luma samples.
    // The SPS picture includes coding-block padding; transport dimensions
    // describe its conformance output, retaining the existing even NV12 size.
    let output_width = right
        .checked_mul(2)
        .and_then(|crop| picture_width.checked_sub(crop));
    let output_height = bottom
        .checked_mul(2)
        .and_then(|crop| picture_height.checked_sub(crop));
    if output_width != Some(width) || output_height != Some(height) {
        return Err(Error::Invalid);
    }
    if b.ue()? != 0 || b.ue()? != 0 {
        return Err(Error::Unavailable);
    }
    Ok(())
}
struct Bits<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Bits<'_> {
    fn read(&mut self, n: usize) -> Result<u32> {
        if n > 32
            || self
                .at
                .checked_add(n)
                .is_none_or(|end| end > self.bytes.len() * 8)
        {
            return Err(Error::Invalid);
        }
        let mut value = 0;
        for _ in 0..n {
            value = (value << 1) | u32::from((self.bytes[self.at / 8] >> (7 - self.at % 8)) & 1);
            self.at += 1
        }
        Ok(value)
    }
    fn skip(&mut self, n: usize) -> Result<()> {
        for _ in 0..n {
            let _ = self.read(1)?;
        }
        Ok(())
    }
    fn ue(&mut self) -> Result<u32> {
        let mut zeros = 0;
        while self.read(1)? == 0 {
            zeros += 1;
            if zeros > 30 {
                return Err(Error::Limit);
            }
        }
        Ok((1u32 << zeros) - 1 + self.read(zeros)?)
    }
}
