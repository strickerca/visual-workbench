use super::*;
fn raster(error: vw_raster::RasterError) -> PackageError {
    use vw_raster::RasterError as E;
    match error {
        E::Memory { .. } | E::Allocation | E::Dimensions { .. } | E::EncodedLimit { .. } => {
            PackageError::Limit
        }
        E::Depth => PackageError::Depth,
        E::Cancelled => PackageError::Cancelled,
        E::Unsupported(_) | E::Color => PackageError::Unsupported,
        _ => PackageError::Invalid,
    }
}
fn admission(bytes: &[u8]) -> PackageResult<()> {
    if bytes.len() < 33
        || bytes.len() > IMAGE_LIMIT
        || bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n")
        || bytes.get(8..16) != Some(b"\0\0\0\rIHDR")
    {
        return Err(PackageError::Invalid);
    }
    let u32_at = |at: usize| -> PackageResult<u32> {
        Ok(u32::from_be_bytes(
            bytes
                .get(at..at + 4)
                .ok_or(PackageError::Invalid)?
                .try_into()
                .map_err(|_| PackageError::Invalid)?,
        ))
    };
    let (w, h) = (u32_at(16)?, u32_at(20)?);
    let pixels = u64::from(w)
        .checked_mul(u64::from(h))
        .ok_or(PackageError::Limit)?;
    if w == 0
        || h == 0
        || pixels > 50_000_000
        || pixels
            .checked_mul(48)
            .and_then(|n| n.checked_add(64 * 1024 * 1024 + 2 * bytes.len() as u64))
            .is_none_or(|n| n > MAX_MEMORY)
    {
        return Err(PackageError::Limit);
    }
    Ok(())
}
fn decode(bytes: &[u8], cancel: &Cancellation) -> PackageResult<vw_raster::DecodedImage> {
    cancel.check()?;
    admission(bytes)?;
    let image = vw_raster::decode(
        bytes,
        vw_raster::DecodeLimits {
            max_encoded_bytes: IMAGE_LIMIT,
            max_pixels: 50_000_000,
            max_memory_bytes: MAX_MEMORY - 32 * 1024 * 1024,
        },
    )
    .map_err(raster)?;
    cancel.check()?;
    Ok(image)
}
fn info(image: &vw_raster::DecodedImage, bytes: &[u8]) -> McpInboxImage {
    McpInboxImage {
        width: image.width,
        height: image.height,
        bit_depth: image.pixels.bit_depth(),
        encoded_bytes: bytes.len() as u64,
        blake3: blake3::hash(bytes).to_hex().to_string(),
        icc_blake3: image
            .icc
            .as_ref()
            .map(|v| blake3::hash(v).to_hex().to_string()),
        orientation_applied: image.orientation_applied,
    }
}
pub(super) fn inspect(bytes: &[u8], cancel: &Cancellation) -> PackageResult<McpInboxImage> {
    let image = decode(bytes, cancel)?;
    Ok(info(&image, bytes))
}
pub(super) fn image_info(value: &McpInboxImage) -> PackageResult<()> {
    hash(&value.blake3)?;
    if let Some(icc) = &value.icc_blake3 {
        hash(icc)?;
    }
    if value.width == 0
        || value.height == 0
        || u64::from(value.width) * u64::from(value.height) > 50_000_000
        || !matches!(value.bit_depth, 8 | 16)
        || value.encoded_bytes == 0
        || value.encoded_bytes > IMAGE_LIMIT as u64
        || !(1..=8).contains(&value.orientation_applied)
    {
        return Err(PackageError::Integrity);
    }
    Ok(())
}
pub(super) fn region(value: McpInboxRegion) -> PackageResult<vw_raster::Region> {
    if value.width == 0 || value.height == 0 || value.width > 512 || value.height > 512 {
        return Err(PackageError::Limit);
    }
    Ok(vw_raster::Region {
        x: value.x,
        y: value.y,
        width: value.width,
        height: value.height,
    })
}
pub(super) fn display(
    record: &Record,
    bytes: &[u8],
    side: McpInboxSide,
    area: McpInboxRegion,
    assume: bool,
    reduce: bool,
    cancel: &Cancellation,
) -> PackageResult<(u32, u32, Vec<u8>)> {
    let wanted = match side {
        McpInboxSide::Before => &record.before,
        McpInboxSide::After => record.after.as_ref().ok_or(PackageError::Invalid)?,
    };
    let area = region(area)?;
    area.validate(wanted.width, wanted.height).map_err(raster)?;
    if wanted.bit_depth > 8 && !reduce {
        return Err(PackageError::Depth);
    }
    let source = decode(bytes, cancel)?;
    if &info(&source, bytes) != wanted {
        return Err(PackageError::Integrity);
    }
    let state_hash = vw_model::StateHash::try_from(record.state_hash.clone())?
        .bytes()
        .to_vec();
    let encoded = vw_raster::export(
        &source,
        &vw_raster::ExportRequest {
            format: vw_raster::ExportFormat::Png8,
            region: Some(area),
            revision: vw_proto::v1::Revision {
                host_seq: record.host_seq,
                state_hash,
            },
            alpha: vw_raster::AlphaPolicy::Preserve,
            color: vw_raster::ColorPolicy::ConvertToSrgb {
                assume_untagged_srgb: assume,
            },
            allow_depth_reduction: reduce,
            memory_budget_bytes: MAX_MEMORY - 32 * 1024 * 1024,
            capture_session: None,
            frame_id: None,
        },
    )
    .map_err(raster)?;
    drop(source);
    cancel.check()?;
    let tile = vw_raster::decode(
        &encoded.bytes,
        vw_raster::DecodeLimits {
            max_encoded_bytes: IMAGE_LIMIT,
            max_pixels: 512 * 512,
            max_memory_bytes: MAX_MEMORY - 32 * 1024 * 1024,
        },
    )
    .map_err(raster)?;
    cancel.check()?;
    if tile.width != area.width || tile.height != area.height {
        return Err(PackageError::Integrity);
    }
    let vw_raster::Pixels::Rgba8(rgba) = tile.pixels else {
        return Err(PackageError::Integrity);
    };
    Ok((wanted.width, wanted.height, rgba))
}
