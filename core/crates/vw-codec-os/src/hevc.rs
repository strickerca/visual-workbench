use crate::{Error, Result};
/// Validate the actual SPS depth in addition to hvcC's advertised depth. This
/// prevents a contradictory configuration from authorizing 10->8-bit decoding.
pub(crate) fn configuration(bytes: &[u8], width: u32, height: u32) -> Result<()> {
    if bytes.len() < 23 || bytes[0] != 1 || bytes[17] & 7 != 0 || bytes[18] & 7 != 0 {
        return Err(Error::Depth);
    }
    if bytes[16] & 3 != 1 {
        return Err(Error::Unsupported);
    }
    let mut at = 23usize;
    let mut sps = 0;
    for _ in 0..bytes[22] {
        let head = bytes.get(at..at + 3).ok_or(Error::Invalid)?;
        at += 3;
        let kind = head[0] & 63;
        let count = u16::from_be_bytes([head[1], head[2]]) as usize;
        if !matches!(kind, 32..=34) {
            return Err(Error::Unsupported);
        }
        if count > 64 {
            return Err(Error::Limit);
        }
        for _ in 0..count {
            let size = bytes.get(at..at + 2).ok_or(Error::Invalid)?;
            at += 2;
            let length = u16::from_be_bytes([size[0], size[1]]) as usize;
            let end = at.checked_add(length).ok_or(Error::Invalid)?;
            let nal = bytes.get(at..end).ok_or(Error::Invalid)?;
            at = end;
            if nal.len() < 2
                || nal[0] & 0x81 != 0
                || nal[1] >> 3 != 0
                || nal[1] & 7 == 0
                || ((nal[0] >> 1) & 63) != kind
            {
                return Err(Error::Invalid);
            }
            if kind == 32
                && (nal.len() < 6 || nal[2] & 3 != 0 || nal[3] >> 4 != 0 || ((nal[3] >> 1) & 7) > 6)
            {
                return Err(Error::Unsupported);
            }
            if kind == 33 {
                sps += 1;
                if sps > 16 {
                    return Err(Error::Limit);
                }
                sequence(nal, width, height)?;
            }
        }
    }
    if at != bytes.len() || sps == 0 {
        return Err(Error::Invalid);
    }
    Ok(())
}
pub(crate) fn access_unit(bytes: &[u8], configuration: &[u8]) -> Result<()> {
    let prefix = usize::from(*configuration.get(21).ok_or(Error::Invalid)? & 3) + 1;
    let mut at = 0usize;
    let mut count = 0;
    let mut picture = false;
    while at < bytes.len() {
        count += 1;
        if count > 4096 {
            return Err(Error::Limit);
        }
        let size = bytes
            .get(at..at + prefix)
            .ok_or(Error::Invalid)?
            .iter()
            .fold(0usize, |n, b| (n << 8) | usize::from(*b));
        at += prefix;
        let end = at.checked_add(size).ok_or(Error::Invalid)?;
        let nal = bytes.get(at..end).ok_or(Error::Invalid)?;
        at = end;
        if nal.len() < 2 || nal[0] & 0x81 != 0 || nal[1] >> 3 != 0 || nal[1] & 7 == 0 {
            return Err(Error::Invalid);
        }
        let kind = (nal[0] >> 1) & 63;
        // hvc1 uses out-of-band parameter sets. Do not let an in-band SPS,
        // auxiliary layer or gain/HDR SEI override the admitted configuration.
        if kind > 31 && !matches!(kind, 35 | 38) {
            return Err(Error::Unsupported);
        }
        picture |= kind <= 31;
    }
    if !picture {
        return Err(Error::Invalid);
    }
    Ok(())
}
struct Bits {
    data: Vec<u8>,
    bit: usize,
}
impl Bits {
    fn take(&mut self, n: usize) -> Result<u32> {
        if n > 32
            || self
                .bit
                .checked_add(n)
                .is_none_or(|end| end > self.data.len() * 8)
        {
            return Err(Error::Invalid);
        }
        let mut value = 0;
        for _ in 0..n {
            value = (value << 1) | u32::from((self.data[self.bit / 8] >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        Ok(value)
    }
    fn skip(&mut self, n: usize) -> Result<()> {
        let end = self.bit.checked_add(n).ok_or(Error::Invalid)?;
        if end > self.data.len() * 8 {
            return Err(Error::Invalid);
        }
        self.bit = end;
        Ok(())
    }
    fn ue(&mut self) -> Result<u32> {
        let mut zeros = 0;
        while self.take(1)? == 0 {
            zeros += 1;
            if zeros > 24 {
                return Err(Error::Invalid);
            }
        }
        Ok(((1u32 << zeros) - 1) + self.take(zeros)?)
    }
}
fn sequence(nal: &[u8], expected_width: u32, expected_height: u32) -> Result<()> {
    if nal.len() < 3
        || nal.len() > 65535
        || ((nal[0] >> 1) & 63) != 33
        || nal[0] & 0x80 != 0
        || nal[1] & 7 == 0
    {
        return Err(Error::Invalid);
    }
    let mut data = Vec::new();
    data.try_reserve_exact(nal.len())
        .map_err(|_| Error::Limit)?;
    let mut zeros = 0;
    let payload = &nal[2..];
    let mut at = 0;
    while at < payload.len() {
        let value = payload[at];
        if zeros >= 2 && value == 3 {
            if payload.get(at + 1).is_none_or(|v| *v > 3) {
                return Err(Error::Invalid);
            }
            zeros = 0;
            at += 1;
            continue;
        }
        data.push(value);
        zeros = if value == 0 { zeros + 1 } else { 0 };
        at += 1;
    }
    let mut bits = Bits { data, bit: 0 };
    bits.skip(4)?;
    let layers = bits.take(3)? as usize;
    if layers > 6 {
        return Err(Error::Unsupported);
    }
    bits.skip(1)?;
    bits.skip(96)?;
    let mut profile = [false; 7];
    let mut level = [false; 7];
    for n in 0..layers {
        profile[n] = bits.take(1)? != 0;
        level[n] = bits.take(1)? != 0;
    }
    if layers > 0 {
        bits.skip((8 - layers) * 2)?;
    }
    for n in 0..layers {
        if profile[n] {
            bits.skip(88)?;
        }
        if level[n] {
            bits.skip(8)?;
        }
    }
    if bits.ue()? > 15 {
        return Err(Error::Invalid);
    }
    let chroma = bits.ue()?;
    if chroma != 1 {
        return Err(Error::Unsupported);
    }
    let width = bits.ue()?;
    let height = bits.ue()?;
    if width == 0
        || height == 0
        || width > expected_width.div_ceil(64) * 64
        || height > expected_height.div_ceil(64) * 64
    {
        return Err(Error::Limit);
    }
    let mut visible_width = width;
    let mut visible_height = height;
    if bits.take(1)? != 0 {
        let left = bits.ue()?;
        let right = bits.ue()?;
        let top = bits.ue()?;
        let bottom = bits.ue()?;
        visible_width = width
            .checked_sub(
                left.checked_add(right)
                    .and_then(|n| n.checked_mul(2))
                    .ok_or(Error::Invalid)?,
            )
            .ok_or(Error::Invalid)?;
        visible_height = height
            .checked_sub(
                top.checked_add(bottom)
                    .and_then(|n| n.checked_mul(2))
                    .ok_or(Error::Invalid)?,
            )
            .ok_or(Error::Invalid)?;
    }
    if visible_width != expected_width || visible_height != expected_height {
        return Err(Error::Invalid);
    }
    if bits.ue()? != 0 || bits.ue()? != 0 {
        return Err(Error::Depth);
    }
    if bits.ue()? > 12 {
        return Err(Error::Invalid);
    }
    let all = bits.take(1)? != 0;
    let first = if all { 0 } else { layers };
    for _ in first..=layers {
        let buffers = bits.ue()?;
        let reorder = bits.ue()?;
        let latency = bits.ue()?;
        if buffers > 15 || reorder > buffers || latency > 1_000_000 {
            return Err(Error::Limit);
        }
    }
    Ok(())
}
