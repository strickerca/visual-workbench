use crate::{Error, Header, MAX_ENCODED, MAX_ICC, Result};
use vw_model::AssetId;
#[derive(Clone, Copy)]
struct BoxRef<'a> {
    kind: [u8; 4],
    payload: &'a [u8],
    start: usize,
    end: usize,
}
fn boxes(mut bytes: &[u8], mut offset: usize) -> Result<Vec<BoxRef<'_>>> {
    let mut result = Vec::new();
    while !bytes.is_empty() {
        if result.len() == 128 {
            return Err(Error::Limit);
        }
        let prefix = bytes.get(..8).ok_or(Error::Invalid)?;
        let short = u32::from_be_bytes(prefix[..4].try_into().map_err(|_| Error::Invalid)?) as u64;
        let (length, head) = if short == 1 {
            (
                u64::from_be_bytes(
                    bytes
                        .get(8..16)
                        .ok_or(Error::Invalid)?
                        .try_into()
                        .map_err(|_| Error::Invalid)?,
                ),
                16,
            )
        } else {
            (short, 8)
        };
        let length = usize::try_from(length).map_err(|_| Error::Limit)?;
        if length < head || length > bytes.len() {
            return Err(Error::Invalid);
        }
        let end = offset.checked_add(length).ok_or(Error::Invalid)?;
        result.push(BoxRef {
            kind: prefix[4..8].try_into().map_err(|_| Error::Invalid)?,
            payload: &bytes[head..length],
            start: offset + head,
            end,
        });
        offset = end;
        bytes = &bytes[length..];
    }
    Ok(result)
}
fn one<'a>(items: &[BoxRef<'a>], kind: &[u8; 4]) -> Result<BoxRef<'a>> {
    let mut found = items.iter().filter(|x| &x.kind == kind);
    let value = *found.next().ok_or(Error::Invalid)?;
    if found.next().is_some() {
        return Err(Error::Invalid);
    }
    Ok(value)
}
fn word(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(
        bytes
            .get(at..at + 4)
            .ok_or(Error::Invalid)?
            .try_into()
            .map_err(|_| Error::Invalid)?,
    ))
}
fn short(bytes: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_be_bytes(
        bytes
            .get(at..at + 2)
            .ok_or(Error::Invalid)?
            .try_into()
            .map_err(|_| Error::Invalid)?,
    ))
}
fn full(bytes: &[u8], version: u8) -> Result<()> {
    if bytes.get(..4) != Some(&[version, 0, 0, 0]) {
        return Err(Error::Unsupported);
    }
    Ok(())
}
pub fn is_heic(bytes: &[u8]) -> bool {
    bytes.get(4..8) == Some(b"ftyp")
        && bytes
            .get(8..12)
            .is_some_and(|brand| matches!(brand, b"heic" | b"heix" | b"mif1"))
}

/// Bounded structural admission precedes ICC copies and OS decode. Only the
/// primary item's properties are authoritative; unused properties cannot alter
/// the receipt. Sequence/grid/auxiliary/Exif transforms require a later adapter.
pub fn inspect(bytes: &[u8], budget: u64) -> Result<Header> {
    if bytes.len() > MAX_ENCODED || bytes.len() < 32 || bytes.len() as u64 > budget {
        return Err(Error::Limit);
    }
    let top = boxes(bytes, 0)?;
    if top
        .iter()
        .any(|x| x.kind != *b"mdat" && x.payload.len() > 4 * 1024 * 1024)
    {
        return Err(Error::Limit);
    }
    if top
        .iter()
        .any(|x| !matches!(&x.kind, b"ftyp" | b"meta" | b"mdat" | b"free" | b"skip"))
    {
        return Err(Error::Unsupported);
    }
    let ftyp = one(&top, b"ftyp")?;
    if ftyp.payload.len() < 8 || ftyp.payload.len() % 4 != 0 {
        return Err(Error::Invalid);
    }
    let mut hevc = false;
    for brand in std::iter::once(&ftyp.payload[..4]).chain(
        ftyp.payload[8..]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|brand| brand.as_slice()),
    ) {
        if matches!(brand, b"heic" | b"heix") {
            hevc = true;
        }
        if matches!(brand, b"avis" | b"avif" | b"msf1" | b"hevc" | b"hevx") {
            return Err(Error::Unsupported);
        }
    }
    if !hevc {
        return Err(Error::Unsupported);
    }
    let meta = one(&top, b"meta")?;
    full(meta.payload, 0)?;
    let meta = boxes(&meta.payload[4..], meta.start + 4)?;
    if meta.iter().any(|x| {
        !matches!(
            &x.kind,
            b"hdlr" | b"pitm" | b"iinf" | b"iloc" | b"iprp" | b"free"
        )
    }) {
        return Err(Error::Unsupported);
    }
    let handler = one(&meta, b"hdlr")?;
    full(handler.payload, 0)?;
    if handler.payload.get(8..12) != Some(b"pict") {
        return Err(Error::Unsupported);
    }
    let primary = one(&meta, b"pitm")?;
    let item = match primary.payload.first() {
        Some(0) => {
            full(primary.payload, 0)?;
            if primary.payload.len() != 6 {
                return Err(Error::Invalid);
            }
            u32::from(short(primary.payload, 4)?)
        }
        Some(1) => {
            full(primary.payload, 1)?;
            if primary.payload.len() != 8 {
                return Err(Error::Invalid);
            }
            word(primary.payload, 4)?
        }
        _ => return Err(Error::Unsupported),
    };
    if item == 0 {
        return Err(Error::Invalid);
    }
    item_info(one(&meta, b"iinf")?, item)?;
    let extent = locations(one(&meta, b"iloc")?.payload, item, &top)?;
    let iprp = one(&meta, b"iprp")?;
    let iprp = boxes(iprp.payload, iprp.start)?;
    if iprp.len() != 2 {
        return Err(Error::Unsupported);
    }
    let ipco = one(&iprp, b"ipco")?;
    let properties = boxes(ipco.payload, ipco.start)?;
    if properties.len() > 64 {
        return Err(Error::Limit);
    }
    let selected = associations(one(&iprp, b"ipma")?.payload, item, properties.len())?;
    let mut spatial = None;
    let mut config = None;
    let mut pixels = None;
    let mut color = None;
    for index in selected {
        let property = properties[index];
        match &property.kind {
            b"ispe" => {
                if spatial.replace(property.payload).is_some() {
                    return Err(Error::Invalid);
                }
            }
            b"hvcC" => {
                if config.replace(property.payload).is_some() {
                    return Err(Error::Invalid);
                }
            }
            b"pixi" => {
                if pixels.replace(property.payload).is_some() {
                    return Err(Error::Invalid);
                }
            }
            b"colr" => {
                if color.replace(property.payload).is_some() {
                    return Err(Error::Invalid);
                }
            }
            b"irot" => {
                if property.payload != [0] {
                    return Err(Error::Orientation);
                }
            }
            b"imir" | b"clap" => return Err(Error::Orientation),
            _ => return Err(Error::Unsupported),
        }
    }
    let spatial = spatial.ok_or(Error::Invalid)?;
    full(spatial, 0)?;
    if spatial.len() != 12 {
        return Err(Error::Invalid);
    }
    let width = word(spatial, 4)?;
    let height = word(spatial, 8)?;
    let mut header = Header {
        source_asset: AssetId::hash(bytes),
        width,
        height,
        bit_depth: 8,
        orientation: 1,
        icc: None,
        encoded_bytes: bytes.len() as u64,
    };
    header.admit(budget)?;
    if let Some(pixels) = pixels {
        full(pixels, 0)?;
        if pixels != [0, 0, 0, 0, 3, 8, 8, 8] {
            return Err(Error::Depth);
        }
    }
    let config = config.ok_or(Error::Invalid)?;
    crate::hevc::configuration(config, width, height)?;
    crate::hevc::access_unit(bytes.get(extent.0..extent.1).ok_or(Error::Invalid)?, config)?;
    let color = color.ok_or(Error::Color)?;
    match color.get(..4) {
        Some(b"nclx") => {
            if color.len() != 11
                || short(color, 4)? != 1
                || short(color, 6)? != 13
                || !matches!(short(color, 8)?, 0 | 1 | 6)
                || color[10] & 0x7f != 0
            {
                return Err(Error::Color);
            }
        }
        Some(b"prof" | b"rICC") => {
            let icc = &color[4..];
            if icc.len() < 132
                || icc.len() > MAX_ICC
                || word(icc, 0)? as usize != icc.len()
                || icc.get(36..40) != Some(b"acsp")
                || icc.get(16..20) != Some(b"RGB ")
            {
                return Err(Error::Color);
            }
            header.icc = Some(icc.to_vec());
        }
        _ => return Err(Error::Color),
    }
    if let Some(icc) = &header.icc {
        // Reuse the reviewed nonallocating ICC admission + bounded semantic
        // parser before an OS decoder is allowed to interpret profile metadata.
        let probe = vw_raster::DecodedImage {
            width: 1,
            height: 1,
            pixels: vw_raster::Pixels::Rgba8(vec![0, 0, 0, 255]),
            icc: Some(icc.clone()),
            source_asset: header.source_asset.clone(),
            original_available: true,
            orientation_applied: 1,
        };
        probe.validate().map_err(|_| Error::Color)?;
    }
    Ok(header)
}
fn item_info(value: BoxRef<'_>, item: u32) -> Result<()> {
    let bytes = value.payload;
    let head = match bytes.first() {
        Some(0) => {
            full(bytes, 0)?;
            if short(bytes, 4)? != 1 {
                return Err(Error::Unsupported);
            }
            6
        }
        Some(1) => {
            full(bytes, 1)?;
            if word(bytes, 4)? != 1 {
                return Err(Error::Unsupported);
            }
            8
        }
        _ => return Err(Error::Unsupported),
    };
    let entries = boxes(&bytes[head..], value.start + head)?;
    if entries.len() != 1 {
        return Err(Error::Unsupported);
    }
    let entry = one(&entries, b"infe")?.payload;
    let (id, at) = match entry.first() {
        Some(2) => {
            full(entry, 2)?;
            (u32::from(short(entry, 4)?), 6)
        }
        Some(3) => {
            full(entry, 3)?;
            (word(entry, 4)?, 8)
        }
        _ => return Err(Error::Unsupported),
    };
    if id != item || short(entry, at)? != 0 || entry.get(at + 2..at + 6) != Some(b"hvc1") {
        return Err(Error::Unsupported);
    }
    let name = entry.get(at + 6..).ok_or(Error::Invalid)?;
    if name.is_empty()
        || name.len() > 4096
        || name.last() != Some(&0)
        || name[..name.len() - 1].contains(&0)
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn associations(bytes: &[u8], item: u32, count: usize) -> Result<Vec<usize>> {
    let prefix = bytes.get(..8).ok_or(Error::Invalid)?;
    if prefix[0] > 1 || prefix[1..3] != [0, 0] || prefix[3] > 1 || word(bytes, 4)? != 1 {
        return Err(Error::Unsupported);
    }
    let wide = prefix[3] == 1;
    let version = prefix[0];
    let mut at = 8;
    let id = if version == 0 {
        let v = u32::from(short(bytes, at)?);
        at += 2;
        v
    } else {
        let v = word(bytes, at)?;
        at += 4;
        v
    };
    if id != item {
        return Err(Error::Invalid);
    }
    let n = usize::from(*bytes.get(at).ok_or(Error::Invalid)?);
    at += 1;
    if n > 64 {
        return Err(Error::Limit);
    }
    let mut output = Vec::new();
    for _ in 0..n {
        let raw = if wide {
            let v = short(bytes, at)?;
            at += 2;
            v & 0x7fff
        } else {
            let v = u16::from(*bytes.get(at).ok_or(Error::Invalid)?);
            at += 1;
            v & 0x7f
        };
        if raw == 0 {
            continue;
        }
        let index = usize::from(raw) - 1;
        if index >= count || output.contains(&index) {
            return Err(Error::Invalid);
        }
        output.push(index);
    }
    if at != bytes.len() {
        return Err(Error::Invalid);
    }
    Ok(output)
}
fn locations(bytes: &[u8], item: u32, top: &[BoxRef<'_>]) -> Result<(usize, usize)> {
    let p = bytes.get(..8).ok_or(Error::Invalid)?;
    if p[0] > 1 || p[1..4] != [0, 0, 0] || p[5] & 15 != 0 {
        return Err(Error::Unsupported);
    }
    let offset = usize::from(p[4] >> 4);
    let length = usize::from(p[4] & 15);
    let base = usize::from(p[5] >> 4);
    if ![0, 4, 8].contains(&offset)
        || ![4, 8].contains(&length)
        || ![0, 4, 8].contains(&base)
        || short(bytes, 6)? != 1
    {
        return Err(Error::Unsupported);
    }
    let mut at = 8;
    if u32::from(short(bytes, at)?) != item {
        return Err(Error::Invalid);
    }
    at += 2;
    if p[0] == 1 {
        if short(bytes, at)? != 0 {
            return Err(Error::Unsupported);
        }
        at += 2;
    }
    if short(bytes, at)? != 0 {
        return Err(Error::Unsupported);
    }
    at += 2;
    let read = |at: &mut usize, size: usize| -> Result<u64> {
        let part = bytes.get(*at..*at + size).ok_or(Error::Invalid)?;
        *at += size;
        Ok(part.iter().fold(0u64, |n, b| (n << 8) | u64::from(*b)))
    };
    let base = read(&mut at, base)?;
    let n = short(bytes, at)?;
    at += 2;
    if n != 1 {
        return Err(Error::Unsupported);
    }
    let start = base
        .checked_add(read(&mut at, offset)?)
        .ok_or(Error::Invalid)?;
    let size = read(&mut at, length)?;
    let end = start.checked_add(size).ok_or(Error::Invalid)?;
    if size == 0
        || !top
            .iter()
            .any(|b| b.kind == *b"mdat" && start >= b.start as u64 && end <= b.end as u64)
    {
        return Err(Error::Invalid);
    }
    if at != bytes.len() {
        return Err(Error::Invalid);
    }
    Ok((
        usize::try_from(start).map_err(|_| Error::Limit)?,
        usize::try_from(end).map_err(|_| Error::Limit)?,
    ))
}
