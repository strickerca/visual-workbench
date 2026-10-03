//! Allocation-bounded inspection of our uncompressed PNG export receipt.
use crate::{ExportBinding, HostError, HostResult};
use serde::Deserialize;
use std::io::{Read, Seek, SeekFrom};

#[derive(Deserialize)]
struct Metadata {
    revision: String,
    source_asset: String,
    output_width: u32,
    output_height: u32,
}
pub(crate) struct Declaration {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub icc: bool,
}

pub(crate) fn validate(binding: &ExportBinding) -> HostResult<()> {
    fn hex(s: &str, n: usize) -> bool {
        s.len() == n
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }
    let valid_revision = binding
        .revision
        .strip_prefix('r')
        .and_then(|s| s.split_once('-'))
        .is_some_and(|(seq, hash)| {
            seq.parse::<u64>().is_ok_and(|n| n.to_string() == seq) && hex(hash, 8)
        });
    if !hex(&binding.png_blake3, 64) || !hex(&binding.source_asset, 64) || !valid_revision {
        return Err(HostError::ExportBindingMismatch);
    }
    Ok(())
}

/// Seeks over data, retaining at most one 64 KiB uncompressed metadata chunk.
/// CRCs and PNG ordering are independently checked by the pixel/row decoder.
pub(crate) fn inspect(
    reader: &mut (impl Read + Seek),
    binding: &ExportBinding,
    length: u64,
) -> HostResult<Declaration> {
    validate(binding)?;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|_| HostError::HandoffStorage)?;
    let mut signature = [0u8; 8];
    reader
        .read_exact(&mut signature)
        .map_err(|_| HostError::InvalidPng)?;
    if signature != *b"\x89PNG\r\n\x1a\n" {
        return Err(HostError::InvalidPng);
    }
    let mut declaration = None;
    let mut metadata = None;
    let mut icc = false;
    let mut ended = false;
    for _ in 0..65_536 {
        let position = reader
            .stream_position()
            .map_err(|_| HostError::HandoffStorage)?;
        if position == length {
            break;
        }
        let mut head = [0u8; 8];
        reader
            .read_exact(&mut head)
            .map_err(|_| HostError::InvalidPng)?;
        let size = u64::from(u32::from_be_bytes(
            head[..4].try_into().map_err(|_| HostError::InvalidPng)?,
        ));
        let end = position
            .checked_add(12)
            .and_then(|n| n.checked_add(size))
            .filter(|n| *n <= length)
            .ok_or(HostError::InvalidPng)?;
        match &head[4..] {
            b"IHDR" => {
                if declaration.is_some() || position != 8 || size != 13 {
                    return Err(HostError::InvalidPng);
                }
                let mut ihdr = [0; 13];
                reader
                    .read_exact(&mut ihdr)
                    .map_err(|_| HostError::InvalidPng)?;
                check_crc(reader, b"IHDR", &ihdr)?;
                declaration = Some((
                    u32::from_be_bytes(ihdr[..4].try_into().map_err(|_| HostError::InvalidPng)?),
                    u32::from_be_bytes(ihdr[4..8].try_into().map_err(|_| HostError::InvalidPng)?),
                    ihdr[8],
                ));
            }
            b"iCCP" => {
                if icc {
                    return Err(HostError::InvalidPng);
                }
                icc = true;
            }
            b"iTXt" => {
                if size > 65_536 {
                    return Err(HostError::ExportBindingMismatch);
                }
                {
                    let mut data = vec![0; size as usize];
                    reader
                        .read_exact(&mut data)
                        .map_err(|_| HostError::InvalidPng)?;
                    check_crc(reader, b"iTXt", &data)?;
                    if let Some(rest) = data.strip_prefix(b"VisualWorkbench\0") {
                        if metadata.is_some() || rest.get(..2) != Some(&[0, 0]) {
                            return Err(HostError::ExportBindingMismatch);
                        }
                        let text = rest.get(2..).ok_or(HostError::ExportBindingMismatch)?;
                        let language_end = text
                            .iter()
                            .position(|b| *b == 0)
                            .ok_or(HostError::ExportBindingMismatch)?;
                        let translated = &text[language_end + 1..];
                        let translated_end = translated
                            .iter()
                            .position(|b| *b == 0)
                            .ok_or(HostError::ExportBindingMismatch)?;
                        metadata = Some(
                            serde_json::from_slice::<Metadata>(&translated[translated_end + 1..])
                                .map_err(|_| HostError::ExportBindingMismatch)?,
                        );
                    }
                }
            }
            b"IEND" => {
                if size != 0 || end != length {
                    return Err(HostError::InvalidPng);
                }
                ended = true;
            }
            b"acTL" | b"eXIf" => return Err(HostError::InvalidPng),
            _ => {}
        }
        reader
            .seek(SeekFrom::Start(end))
            .map_err(|_| HostError::HandoffStorage)?;
        if ended {
            break;
        }
    }
    let (width, height, bit_depth) = declaration.ok_or(HostError::InvalidPng)?;
    let meta = metadata.ok_or(HostError::ExportBindingMismatch)?;
    if !ended
        || width == 0
        || height == 0
        || !matches!(bit_depth, 8 | 16)
        || meta.source_asset != binding.source_asset
        || meta.revision != binding.revision
        || meta.output_width != width
        || meta.output_height != height
    {
        return Err(HostError::ExportBindingMismatch);
    }
    Ok(Declaration {
        width,
        height,
        bit_depth,
        icc,
    })
}

fn check_crc(reader: &mut impl Read, kind: &[u8; 4], data: &[u8]) -> HostResult<()> {
    let mut expected = [0u8; 4];
    reader
        .read_exact(&mut expected)
        .map_err(|_| HostError::InvalidPng)?;
    let mut crc = u32::MAX;
    for byte in kind.iter().chain(data) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb8_8320 } else { 0 };
        }
    }
    if !crc != u32::from_be_bytes(expected) {
        return Err(HostError::InvalidPng);
    }
    Ok(())
}

pub(crate) fn bytes(png: &[u8], binding: &ExportBinding) -> HostResult<Declaration> {
    validate(binding)?;
    if blake3::hash(png).to_hex().as_str() != binding.png_blake3 {
        return Err(HostError::ExportBindingMismatch);
    }
    inspect(&mut std::io::Cursor::new(png), binding, png.len() as u64)
}
