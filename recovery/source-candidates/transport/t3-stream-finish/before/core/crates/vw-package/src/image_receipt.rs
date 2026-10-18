//! Own-compiler PNG metadata is a bounded, uncompressed receipt. Refuse other
//! ancillary payloads before the allocating raster decoder, then compare its
//! decoded ICC bytes to the authoritative raster engine's deterministic sRGB.
use crate::{Cancellation, Error, Manifest, PackageImage, Result, bounded, check};

pub(crate) fn standard_profile(memory: u64) -> Result<Vec<u8>> {
    // Obtain the canonical profile through the existing public raster API;
    // duplicating moxcms's ICC writer would create a second color authority.
    let mut out = bounded::Buffer::new(1024);
    {
        let mut encoder = png::Encoder::new(&mut out, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder.write_header().map_err(|_| Error::Encoding)?;
        writer
            .write_image_data(&[0, 0, 0, 255])
            .map_err(|_| Error::Encoding)?;
        writer.finish().map_err(|_| Error::Encoding)?;
    }
    vw_raster::decode(
        &out.bytes,
        vw_raster::DecodeLimits {
            max_encoded_bytes: 1024,
            max_pixels: 1,
            max_memory_bytes: memory,
        },
    )?
    .icc
    .ok_or(Error::Integrity)
}

pub(crate) fn verify(
    bytes: &[u8],
    manifest: &Manifest,
    image: &PackageImage,
    cancel: &dyn Cancellation,
) -> Result<()> {
    let mapping = match image.id.as_str() {
        "clean_source" | "overview" => &manifest.extensions.overview_mapping,
        _ => {
            &manifest
                .extensions
                .crops
                .iter()
                .find(|c| c.image == image.id)
                .ok_or(Error::Integrity)?
                .mapping
        }
    };
    let expected = bounded::json(
        &serde_json::json!({
            "schema": 1, "project_id": manifest.source.project_id,
            "document_id": manifest.source.document_id, "host_seq": manifest.extensions.host_seq,
            "state_hash": manifest.extensions.state_hash, "source_asset": manifest.source.asset_hash,
            "mapping": mapping,
        }),
        8192,
    )?;
    let mut at = 8_usize;
    let mut receipt = false;
    let mut profile = false;
    let mut header = false;
    let mut data = false;
    while at < bytes.len() {
        check(cancel)?;
        let prefix = bytes
            .get(at..at.checked_add(8).ok_or(Error::Integrity)?)
            .ok_or(Error::Integrity)?;
        let size =
            u32::from_be_bytes(prefix[..4].try_into().map_err(|_| Error::Integrity)?) as usize;
        let end = at
            .checked_add(12)
            .and_then(|n| n.checked_add(size))
            .ok_or(Error::Integrity)?;
        let chunk = bytes.get(at..end).ok_or(Error::Integrity)?;
        let content = &chunk[8..8 + size];
        match &prefix[4..8] {
            b"IHDR" if !header && at == 8 && size == 13 => header = true,
            b"iCCP" if header && !profile && !data && size <= 64 * 1024 => {
                profile = true;
                crc(&chunk[4..8 + size], &chunk[8 + size..])?;
            }
            b"iTXt" if header && !receipt && !data && size <= 8192 + 32 => {
                const PREFIX: &[u8] = b"VisualWorkbenchPackage\0\0\0\0\0";
                if content.strip_prefix(PREFIX) != Some(expected.as_slice()) {
                    return Err(Error::Integrity);
                }
                crc(&chunk[4..8 + size], &chunk[8 + size..])?;
                receipt = true;
            }
            b"IDAT" if header && profile && receipt => data = true,
            b"IEND" if data && size == 0 && end == bytes.len() => return Ok(()),
            _ => return Err(Error::Integrity),
        }
        at = end;
    }
    Err(Error::Integrity)
}
fn crc(bytes: &[u8], checksum: &[u8]) -> Result<()> {
    // Only bounded metadata chunks use this small dependency-free CRC. The
    // raster decoder checks the remaining PNG stream and pixel payload.
    let mut value = u32::MAX;
    for &byte in bytes {
        value ^= u32::from(byte);
        for _ in 0..8 {
            value = (value >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(value & 1));
        }
    }
    if checksum != (!value).to_be_bytes() {
        return Err(Error::Integrity);
    }
    Ok(())
}
