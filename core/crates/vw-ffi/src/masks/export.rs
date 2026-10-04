use super::*;
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

pub(super) fn write(
    state: &ProjectState,
    options: SelectionExportOptions,
    cancel: &Cancellation,
) -> SelectionResult<SelectionExportReceipt> {
    if options.max_encoded_bytes == 0 || options.max_encoded_bytes > MAX_ENCODED {
        return Err(SelectionError::Invalid);
    }
    let canonical = canonical_workspace(state, options.memory_budget_bytes, false)?;
    let pixel_budget = options
        .memory_budget_bytes
        .checked_sub(canonical)
        .ok_or(SelectionError::Limit)?;
    let doc = check_binding(state, &options.binding)?;
    let size = Size::new(doc.width, doc.height)?;
    check_version(state.project()?, &options.binding, &options.selection, size)?;
    let crop = options.region.map(region).unwrap_or(Region::full(size));
    crop.size(size)?;
    let (work, destination) = destination(&options.work_directory, &options.output_path)?;
    let mut estimate = mask_estimate(size, 0)?;
    let asset_id = AssetId::try_from(options.binding.source_asset_id.clone())?;
    let asset = state
        .project()?
        .assets
        .get(&asset_id)
        .ok_or(SelectionError::Corrupt)?;
    let capture = state
        .project()?
        .documents
        .get(&Id::try_from(options.binding.document_id.clone())?)
        .ok_or(SelectionError::Conflict)?
        .definition
        .capture
        .as_ref();
    let capture_session = capture
        .map(|value| Id::from_proto(value.capture_session_id.as_ref()))
        .transpose()?;
    let frame_id = capture.map(|value| value.frame_id);
    if options.kind == SelectionExportKind::Cutout {
        if !matches!(asset.format.as_str(), "png" | "jpeg" | "jpg" | "webp")
            || !matches!(asset.bit_depth, 8 | 16)
        {
            return Err(SelectionError::Unsupported);
        }
        if asset.byte_size > MAX_ENCODED {
            return Err(SelectionError::EncodedLimit);
        }
        let profile = if asset.icc_profile.is_empty() {
            0
        } else {
            72 * 1024 * 1024
        };
        let pixels = size.pixels() as u64;
        let output = u64::from(crop.width) * u64::from(crop.height);
        // Full-source decode plus retained input/mask; full straight RGBA mask
        // application; regional encoder plus retained full masked image. These
        // are separate peaks, not a claim that a small crop avoids source decode.
        let decode = pixels
            .checked_mul(33)
            .and_then(|n| n.checked_add(asset.byte_size))
            .and_then(|n| n.checked_add(profile + CODEC_RESERVE))
            .ok_or(SelectionError::Limit)?;
        let encode = pixels
            .checked_mul(9)
            .and_then(|n| n.checked_add(output * 64))
            .and_then(|n| n.checked_add(profile + 32 * 1024 * 1024))
            .ok_or(SelectionError::Limit)?;
        estimate = estimate.max(decode).max(encode);
    }
    estimate = estimate
        .checked_add(canonical)
        .ok_or(SelectionError::Limit)?;
    memory(estimate, options.memory_budget_bytes)?;
    cancel.check()?;
    let mask = load_mask(state, &options.selection, size, cancel)?;
    let metadata_json = serde_json::json!({
        "schema_version": 1, "mask_algorithm_version": vw_mask::ALGORITHM_VERSION,
        "project_id": options.binding.project_id, "document_id": options.binding.document_id,
        "source_asset_id": options.binding.source_asset_id,
        "host_seq": options.binding.host_seq, "state_hash": options.binding.state_hash,
        "selection_object_id": options.selection.object_id, "selection_asset_id": options.selection.asset_id,
        "selection_version": options.selection.version,
        "capture_session_id": capture_session.as_ref().map(Id::as_str), "frame_id": frame_id,
        "kind": if options.kind == SelectionExportKind::Mask { "mask" } else { "cutout" },
        "region": { "x": crop.x, "y": crop.y, "width": crop.width, "height": crop.height },
        "resampled": false, "color_conversion": "none", "depth_reduced": false,
    }).to_string();
    cancel.check()?;
    let (bytes, depth) = match options.kind {
        SelectionExportKind::Mask => {
            let coverage = mask.crop(crop)?;
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, crop.width, crop.height);
                encoder.set_color(png::ColorType::Grayscale);
                encoder.set_depth(png::BitDepth::Eight);
                encoder
                    .add_text_chunk("VisualWorkbenchSelection".into(), metadata_json.clone())
                    .map_err(|_| SelectionError::Storage)?;
                let mut writer = encoder
                    .write_header()
                    .map_err(|_| SelectionError::Storage)?;
                writer
                    .write_image_data(&coverage)
                    .map_err(|_| SelectionError::Storage)?;
                writer.finish().map_err(|_| SelectionError::Storage)?;
            }
            (bytes, 8)
        }
        SelectionExportKind::Cutout => {
            let bytes = read_blob(state, &asset_id, asset.byte_size, MAX_ENCODED, cancel)?;
            let retained = bytes.capacity() as u64 + size.pixels() as u64 + CODEC_RESERVE;
            let image = vw_raster::decode(
                &bytes,
                vw_raster::DecodeLimits {
                    max_encoded_bytes: MAX_ENCODED as usize,
                    max_pixels: vw_mask::MAX_PIXELS,
                    max_memory_bytes: pixel_budget
                        .checked_sub(retained)
                        .ok_or(SelectionError::Limit)?,
                },
            )?;
            if image.source_asset != asset_id
                || image.width != size.width()
                || image.height != size.height()
                || u32::from(image.orientation_applied) != asset.orientation
                || u32::from(image.pixels.bit_depth()) != asset.bit_depth
                || image.icc.as_deref().unwrap_or_default() != asset.icc_profile.as_slice()
                || image.pixels.has_alpha() != asset.has_alpha
            {
                return Err(SelectionError::Corrupt);
            }
            drop(bytes);
            cancel.check()?;
            // Keep the full document grid so the raster export's own embedded
            // source dimensions and region metadata remain truthful.
            let pixels = match &image.pixels {
                vw_raster::Pixels::Rgba8(p) => {
                    vw_raster::Pixels::Rgba8(mask.cutout_rgba8(p, Region::full(size))?.rgba)
                }
                vw_raster::Pixels::Rgba16(p) => {
                    vw_raster::Pixels::Rgba16(mask.cutout_rgba16(p, Region::full(size))?.rgba)
                }
            };
            let depth = pixels.bit_depth();
            let icc = image.icc;
            drop(image.pixels);
            let image = vw_raster::DecodedImage {
                width: size.width(),
                height: size.height(),
                pixels,
                icc,
                source_asset: asset_id,
                original_available: true,
                orientation_applied: asset.orientation as u8,
            };
            let retained = size.pixels() as u64 * (1 + if depth == 16 { 8 } else { 4 });
            cancel.check()?;
            let result = vw_raster::export(
                &image,
                &vw_raster::ExportRequest {
                    format: if depth == 16 {
                        vw_raster::ExportFormat::Png16
                    } else {
                        vw_raster::ExportFormat::Png8
                    },
                    region: Some(vw_raster::Region {
                        x: crop.x,
                        y: crop.y,
                        width: crop.width,
                        height: crop.height,
                    }),
                    revision: state.view_revision()?,
                    alpha: vw_raster::AlphaPolicy::Preserve,
                    color: vw_raster::ColorPolicy::Preserve,
                    allow_depth_reduction: false,
                    memory_budget_bytes: pixel_budget
                        .checked_sub(retained)
                        .ok_or(SelectionError::Limit)?,
                    capture_session,
                    frame_id,
                },
            )?;
            (result.bytes, depth)
        }
    };
    if bytes.len() as u64 > options.max_encoded_bytes {
        return Err(SelectionError::EncodedLimit);
    }
    cancel.check()?;
    let mut staged = tempfile::Builder::new()
        .prefix("vw-selection-")
        .suffix(".png.part")
        .tempfile_in(&work)?;
    for part in bytes.chunks(64 * 1024) {
        cancel.check()?;
        staged.write_all(part)?;
    }
    staged.as_file().sync_all()?;
    let receipt = SelectionExportReceipt {
        binding: options.binding,
        selection: options.selection,
        kind: options.kind,
        region: public_region(crop),
        output_bit_depth: depth,
        encoded_bytes: bytes.len() as u64,
        blake3: AssetId::hash(&bytes).to_string(),
        metadata_json,
        revision: doc.revision,
        estimated_peak_bytes: estimate,
    };
    cancel.check()?;
    check_components(&work)?;
    // persist_noclobber owns only this temporary and cannot overwrite the caller's
    // file. After publication, cancellation leaves the complete caller-owned PNG.
    staged
        .persist_noclobber(&destination)
        .map_err(|_| SelectionError::Storage)?;
    #[cfg(unix)]
    std::fs::File::open(&work)?.sync_all()?;
    Ok(receipt)
}

pub(super) fn read_blob(
    state: &ProjectState,
    id: &AssetId,
    expected_bytes: u64,
    limit: u64,
    cancel: &Cancellation,
) -> SelectionResult<Vec<u8>> {
    if expected_bytes == 0 || expected_bytes > limit {
        return Err(SelectionError::EncodedLimit);
    }
    let path = state.blobs.path(id)?;
    check_components(&path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let mut file = options.open(path)?;
    let metadata = file.metadata()?;
    reject_reparse(&metadata)?;
    if !metadata.is_file() || metadata.len() != expected_bytes {
        return Err(SelectionError::Corrupt);
    }
    let count = usize::try_from(expected_bytes).map_err(|_| SelectionError::Limit)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(count)
        .map_err(|_| SelectionError::Limit)?;
    bytes.resize(count, 0);
    let mut hash = blake3::Hasher::new();
    for part in bytes.chunks_mut(64 * 1024) {
        cancel.check()?;
        file.read_exact(part)?;
        hash.update(part);
    }
    if file.read(&mut [0u8; 1])? != 0 || *hash.finalize().as_bytes() != id.bytes() {
        return Err(SelectionError::Corrupt);
    }
    cancel.check()?;
    Ok(bytes)
}

fn reject_reparse(meta: &std::fs::Metadata) -> SelectionResult<()> {
    if meta.file_type().is_symlink() {
        return Err(SelectionError::Invalid);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(SelectionError::Invalid);
        }
    }
    Ok(())
}
fn check_components(path: &Path) -> SelectionResult<()> {
    if !path.is_absolute() {
        return Err(SelectionError::Invalid);
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err(SelectionError::Invalid);
        }
        current.push(component);
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        reject_reparse(&std::fs::symlink_metadata(&current)?)?;
    }
    Ok(())
}
fn destination(directory: &str, path: &str) -> SelectionResult<(PathBuf, PathBuf)> {
    if directory.is_empty()
        || path.is_empty()
        || directory.len() > 32767
        || path.len() > 32767
        || directory.contains('\0')
        || path.contains('\0')
    {
        return Err(SelectionError::Invalid);
    }
    let directory = Path::new(directory);
    let path = Path::new(path);
    check_components(directory)?;
    if !directory.is_dir() || !path.is_absolute() {
        return Err(SelectionError::Invalid);
    }
    let parent = path.parent().ok_or(SelectionError::Invalid)?;
    check_components(parent)?;
    let work = directory.canonicalize()?;
    if parent.canonicalize()? != work {
        return Err(SelectionError::Invalid);
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(SelectionError::Invalid)?;
    if name.len() > 240 || name.contains(':') || !name.to_ascii_lowercase().ends_with(".png") {
        return Err(SelectionError::Invalid);
    }
    let target = work.join(name);
    match std::fs::symlink_metadata(&target) {
        Ok(_) => return Err(SelectionError::Invalid),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok((work, target))
}
