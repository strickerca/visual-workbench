//! Nonallocating AV1 metadata admission for one reduced-still, opaque 8-bit
//! profile-0 frame. OS decoding remains responsible for tile/entropy validity.
use crate::{Error, Nclx, Result};

struct Bits<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Bits<'_> {
    fn take(&mut self, n: usize) -> Result<u32> {
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
            self.at += 1;
        }
        Ok(value)
    }
    fn trailing(&mut self) -> Result<()> {
        if self.take(1)? != 1 {
            return Err(Error::Invalid);
        }
        while self.at < self.bytes.len() * 8 {
            if self.take(1)? != 0 {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
}

fn obu<'a>(bytes: &'a [u8], at: &mut usize) -> Result<(u8, &'a [u8])> {
    let header = *bytes.get(*at).ok_or(Error::Invalid)?;
    *at += 1;
    // No multilayer extension; a size field is mandatory in this stream form.
    if header & 0x85 != 0 || header & 2 == 0 {
        return Err(Error::Unsupported);
    }
    let kind = (header >> 3) & 15;
    let mut length = 0_u64;
    let mut complete = false;
    for i in 0..8 {
        let value = *bytes.get(*at).ok_or(Error::Invalid)?;
        *at += 1;
        length |= u64::from(value & 0x7f) << (7 * i);
        if value & 0x80 == 0 {
            complete = true;
            break;
        }
    }
    if !complete {
        return Err(Error::Invalid);
    }
    let end = at
        .checked_add(usize::try_from(length).map_err(|_| Error::Limit)?)
        .ok_or(Error::Invalid)?;
    let payload = bytes.get(*at..end).ok_or(Error::Invalid)?;
    *at = end;
    Ok((kind, payload))
}

#[derive(Clone, Copy)]
struct Sequence {
    level: u8,
    chroma: u8,
    signal: Nclx,
}
fn sequence(bytes: &[u8], width: u32, height: u32) -> Result<Sequence> {
    if bytes.is_empty() || bytes.len() > 4096 {
        return Err(Error::Limit);
    }
    let mut bits = Bits { bytes, at: 0 };
    if bits.take(3)? != 0 || bits.take(1)? != 1 || bits.take(1)? != 1 {
        return Err(Error::Unsupported);
    }
    let level = bits.take(5)? as u8;
    if level > 23 {
        return Err(Error::Unsupported);
    }
    let width_bits = bits.take(4)? as usize + 1;
    let height_bits = bits.take(4)? as usize + 1;
    if bits.take(width_bits)? + 1 != width || bits.take(height_bits)? + 1 != height {
        return Err(Error::Invalid);
    }
    bits.take(3)?; // superblock size and the two intra-filter switches
    if bits.take(1)? != 0 {
        return Err(Error::Orientation);
    } // super-resolution forbidden
    bits.take(2)?; // CDEF/restoration
    if bits.take(1)? != 0 {
        return Err(Error::Depth);
    }
    if bits.take(1)? != 0 {
        return Err(Error::Unsupported);
    } // monochrome unsupported by RGB receipt
    let (primaries, transfer, matrix) = if bits.take(1)? == 1 {
        (
            bits.take(8)? as u16,
            bits.take(8)? as u16,
            bits.take(8)? as u16,
        )
    } else {
        (2, 2, 2)
    };
    // Do not hand an HDR or unspecified YUV matrix to an implicit OS transform.
    if !matches!(primaries, 1 | 2) || !matches!(transfer, 2 | 13) || !matches!(matrix, 1 | 6) {
        return Err(Error::Color);
    }
    let full_range = bits.take(1)? != 0;
    let chroma = bits.take(2)? as u8;
    if chroma > 2 {
        return Err(Error::Unsupported);
    }
    bits.take(1)?; // separate UV delta quantization
    if bits.take(1)? != 0 {
        return Err(Error::Unsupported);
    } // film grain not approximated
    bits.trailing()?;
    Ok(Sequence {
        level,
        chroma,
        signal: Nclx {
            primaries,
            transfer,
            matrix,
            full_range,
        },
    })
}

fn frame_prefix(bytes: &[u8]) -> Result<()> {
    let mut bits = Bits { bytes, at: 0 };
    bits.take(1)?; // disable_cdf_update
    if bits.take(1)? != 0 {
        bits.take(1)?;
    } // allow screen tools / integer MV
    // Reduced still fixes frame type, reference state, dimensions and order
    // hint; super-resolution is disabled by the sequence admission above.
    if bits.take(1)? != 0 {
        return Err(Error::Orientation);
    } // render extent differs
    Ok(())
}

pub(crate) fn admit(config: &[u8], stream: &[u8], width: u32, height: u32) -> Result<Nclx> {
    if config.len() < 4 || config.len() > 8192 || config[0] != 0x81 {
        return Err(Error::Invalid);
    }
    if config[1] >> 5 != 0 || config[2] & 0xf0 != 0 || config[2] & 0x0c != 0x0c || config[3] != 0 {
        return Err(Error::Unsupported);
    }
    let mut at = 0;
    let mut admitted = None;
    let mut raw_sequence = None;
    let mut frame = false;
    for _ in 0..64 {
        if at == stream.len() {
            break;
        }
        let (kind, value) = obu(stream, &mut at)?;
        match kind {
            1 if admitted.is_none() && !frame => {
                admitted = Some(sequence(value, width, height)?);
                raw_sequence = Some(value);
            }
            2 if admitted.is_none() && !frame && value.is_empty() => {}
            6 if admitted.is_some() && !frame => {
                frame_prefix(value)?;
                frame = true;
            }
            15 if value.len() <= 4096 && value.iter().all(|&v| v == 0) => {}
            _ => return Err(Error::Unsupported),
        }
    }
    if at != stream.len() || !frame {
        return Err(Error::Invalid);
    }
    let sequence = admitted.ok_or(Error::Invalid)?;
    if config[1] & 31 != sequence.level || config[2] & 3 != sequence.chroma {
        return Err(Error::Invalid);
    }
    if config.len() > 4 {
        let mut pos = 4;
        let (kind, value) = obu(config, &mut pos)?;
        if kind != 1 || pos != config.len() || Some(value) != raw_sequence {
            return Err(Error::Invalid);
        }
    }
    Ok(sequence.signal)
}
