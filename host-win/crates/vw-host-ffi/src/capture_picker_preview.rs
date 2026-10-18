//! Bounded display derivative of the exact helper-owned PNG. The original is
//! hashed before decoding and remains the only import/capture source.
use super::*;
use std::{
    fs::OpenOptions,
    io::{BufReader, Read, Seek, SeekFrom},
    path::Path,
};

pub(super) fn read(
    path: &Path,
    receipt: &picker::PickerFrameReceipt,
    max_side: u32,
    cancel: &capture::Cancellation,
) -> capture::Result<PickerPreview> {
    if !(16..=1024).contains(&max_side) {
        return Err(capture::Error::Limit);
    }
    let mut reader = open(path, receipt, cancel)?;
    let longer = receipt.width.max(receipt.height);
    if longer == 0 {
        return Err(capture::Error::Invalid);
    }
    let side = max_side.min(longer);
    let width = (u64::from(receipt.width) * u64::from(side) / u64::from(longer)).max(1) as u32;
    let height = (u64::from(receipt.height) * u64::from(side) / u64::from(longer)).max(1) as u32;
    let mut rgba = vec![0; width as usize * height as usize * 4];
    let mut next = 0;
    for y in 0..receipt.height {
        cancel.check()?;
        let row = reader
            .next_row()
            .map_err(|_| capture::Error::Invalid)?
            .ok_or(capture::Error::Invalid)?;
        if row.data().len() != receipt.width as usize * 4 {
            return Err(capture::Error::Invalid);
        }
        if next < height
            && y == (u64::from(next) * u64::from(receipt.height) / u64::from(height)) as u32
        {
            for x in 0..width {
                let source =
                    (u64::from(x) * u64::from(receipt.width) / u64::from(width)) as usize * 4;
                let target = (next as usize * width as usize + x as usize) * 4;
                rgba[target..target + 4].copy_from_slice(&row.data()[source..source + 4]);
            }
            next += 1;
        }
    }
    if next != height
        || reader
            .next_row()
            .map_err(|_| capture::Error::Invalid)?
            .is_some()
    {
        return Err(capture::Error::Invalid);
    }
    // Finishing pixel rows alone does not validate the terminal PNG chunks.
    reader.finish().map_err(|_| capture::Error::Invalid)?;
    cancel.check()?;
    Ok(PickerPreview {
        source_asset_id: receipt.source_asset_id.clone(),
        source_width: receipt.width,
        source_height: receipt.height,
        width,
        height,
        rgba,
    })
}

fn open(
    path: &Path,
    receipt: &picker::PickerFrameReceipt,
    cancel: &capture::Cancellation,
) -> capture::Result<png::Reader<BufReader<std::fs::File>>> {
    cancel.check()?;
    validate_snapshot(path, receipt)?;
    capture::plain(path, false)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let mut file = options.open(path).map_err(|_| capture::Error::Storage)?;
    let metadata = file.metadata().map_err(|_| capture::Error::Storage)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(capture::Error::Storage);
        }
    }
    if !metadata.is_file()
        || metadata.len() != receipt.png_bytes
        || metadata.len() > 64 * 1024 * 1024
    {
        return Err(capture::Error::Limit);
    }
    let mut hash = blake3::Hasher::new();
    let mut buffer = [0u8; 65536];
    let mut read = 0u64;
    loop {
        cancel.check()?;
        let n = file
            .read(&mut buffer)
            .map_err(|_| capture::Error::Storage)?;
        if n == 0 {
            break;
        }
        read += n as u64;
        if read > receipt.png_bytes {
            return Err(capture::Error::Limit);
        }
        hash.update(&buffer[..n]);
    }
    if read != receipt.png_bytes || hash.finalize().to_hex().as_str() != receipt.source_asset_id {
        return Err(capture::Error::Invalid);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| capture::Error::Storage)?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_limits(png::Limits {
        bytes: 4 * 1024 * 1024,
    });
    let reader = decoder.read_info().map_err(|_| capture::Error::Invalid)?;
    let info = reader.info();
    if info.width != receipt.width
        || info.height != receipt.height
        || info.color_type != png::ColorType::Rgba
        || info.bit_depth != png::BitDepth::Eight
        || info.interlaced
        || info.icc_profile.is_some()
        || info.srgb.is_none()
        || info.animation_control.is_some()
        || info.frame_control.is_some()
    {
        return Err(capture::Error::Invalid);
    }
    Ok(reader)
}

/// This is a file/receipt fence, not a new OS observation: an old captured
/// snapshot remains usable after its source window closes or monitor moves.
fn validate_snapshot(path: &Path, receipt: &picker::PickerFrameReceipt) -> capture::Result<()> {
    let output_directory = path
        .parent()
        .and_then(Path::to_str)
        .ok_or(capture::Error::Invalid)?;
    if path.file_name().and_then(|name| name.to_str()) != Some("capture.png") {
        return Err(capture::Error::Invalid);
    }
    receipt.validate(&picker::PickerCaptureRequest {
        explicit_owner_action: true,
        owner_process_id: owner(),
        source: receipt.source.clone(),
        capture_session_id: receipt.identity.capture_session_id.clone(),
        frame_id: receipt.identity.frame_id,
        geometry_revision: receipt.identity.geometry_revision,
        output_directory: output_directory.into(),
        limits: capture::Limits::default(),
    })
}

pub(super) fn crop(
    path: &Path,
    receipt: &picker::PickerFrameReceipt,
    region: capture::Rect,
    output: &Path,
    cancel: &capture::Cancellation,
) -> capture::Result<(String, u64)> {
    let (x, first_y) = region.offset_inside(receipt.identity.client_rect)?;
    let limits = capture::Limits::default();
    let count = limits.image(region.width, region.height)?;
    let mut reader = open(path, receipt, cancel)?;
    let mut rgba = vec![0; count];
    let stride = region.width as usize * 4;
    for y in 0..receipt.height {
        cancel.check()?;
        let row = reader
            .next_row()
            .map_err(|_| capture::Error::Invalid)?
            .ok_or(capture::Error::Invalid)?;
        if row.data().len() != receipt.width as usize * 4 {
            return Err(capture::Error::Invalid);
        }
        if y >= first_y && y < first_y + region.height {
            let target = (y - first_y) as usize * stride;
            let source = x as usize * 4;
            rgba[target..target + stride].copy_from_slice(&row.data()[source..source + stride]);
        }
    }
    if reader
        .next_row()
        .map_err(|_| capture::Error::Invalid)?
        .is_some()
    {
        return Err(capture::Error::Invalid);
    }
    reader.finish().map_err(|_| capture::Error::Invalid)?;
    cancel.check()?;
    capture::publish_png(output, region.width, region.height, &rgba, limits, cancel)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    fn fixture(
        width: u32,
        height: u32,
    ) -> (tempfile::TempDir, picker::PickerFrameReceipt, Vec<u8>) {
        let dir = tempfile::tempdir().expect("owned fixture");
        let pixels = (0..width * height)
            .flat_map(|i| [(i % 251) as u8, ((i / width) % 251) as u8, 73, 255])
            .collect::<Vec<_>>();
        let (hash, bytes) = capture::publish_png(
            dir.path(),
            width,
            height,
            &pixels,
            capture::Limits::default(),
            &capture::Cancellation::default(),
        )
        .unwrap();
        let rect = capture::Rect {
            x: -100,
            y: -20,
            width,
            height,
        };
        let monitor = picker::MonitorTarget {
            monitor: 1,
            device: "fixture".into(),
            physical: rect,
            dpi: 192,
            rotation: 0,
            primary: false,
            observed_ns: 1,
        };
        let identity = capture::FrameIdentity {
            capture_session_id: "019f0000-0000-7000-8000-000000000001".into(),
            frame_id: 7,
            geometry_revision: 3,
            source_kind: "monitor".into(),
            client_rect: rect,
            dpi_scale: 2.0,
            monotonic_timestamp_ns: 2,
            captured_at_ms: 10,
            platform: "windows".into(),
            window_handle: 0,
            monitor_id: "fixture".into(),
        };
        (
            dir,
            picker::PickerFrameReceipt {
                identity,
                source: picker::PickerSource::Monitor { target: monitor },
                source_asset_id: hash,
                png_bytes: bytes,
                width,
                height,
                bit_depth: 8,
                border_visible: true,
                filename: "capture.png".into(),
                window_receipt: None,
            },
            pixels,
        )
    }
    #[test]
    fn display_derivative_retains_source_binding_and_never_rewrites_original() {
        let (dir, receipt, pixels) = fixture(32, 16);
        let path = dir.path().join("capture.png");
        let original = std::fs::read(&path).unwrap();
        let preview = read(&path, &receipt, 16, &capture::Cancellation::default()).unwrap();
        assert_eq!((preview.width, preview.height), (16, 8));
        assert_eq!(preview.source_asset_id, receipt.source_asset_id);
        for y in 0..8usize {
            for x in 0..16usize {
                assert_eq!(
                    &preview.rgba[(y * 16 + x) * 4..(y * 16 + x + 1) * 4],
                    &pixels[(y * 2 * 32 + x * 2) * 4..(y * 2 * 32 + x * 2 + 1) * 4]
                );
            }
        }
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
    #[test]
    fn exact_physical_crop_preserves_pixels_and_refuses_overwrite() {
        let (dir, receipt, pixels) = fixture(4, 3);
        let out = tempfile::tempdir().unwrap();
        let region = capture::Rect {
            x: -99,
            y: -19,
            width: 2,
            height: 2,
        };
        let cancel = capture::Cancellation::default();
        let (hash, bytes) = crop(
            &dir.path().join("capture.png"),
            &receipt,
            region,
            out.path(),
            &cancel,
        )
        .unwrap();
        let mut cropped = receipt.clone();
        let target = match &receipt.source {
            picker::PickerSource::Monitor { target } => target.clone(),
            _ => panic!("fixture monitor"),
        };
        cropped.source = picker::PickerSource::Region { target, region };
        cropped.identity.source_kind = "region".into();
        cropped.identity.client_rect = region;
        cropped.width = 2;
        cropped.height = 2;
        cropped.source_asset_id = hash;
        cropped.png_bytes = bytes;
        let preview = read(&out.path().join("capture.png"), &cropped, 16, &cancel).unwrap();
        assert_eq!(preview.rgba, [&pixels[20..28], &pixels[36..44]].concat());
        let original = std::fs::read(out.path().join("capture.png")).unwrap();
        assert!(
            crop(
                &dir.path().join("capture.png"),
                &receipt,
                region,
                out.path(),
                &cancel
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read(out.path().join("capture.png")).unwrap(),
            original
        );
    }
    #[test]
    fn changed_source_or_header_receipt_is_rejected_before_pixel_publication() {
        let (dir, receipt, _) = fixture(8, 4);
        let path = dir.path().join("capture.png");
        let cancel = capture::Cancellation::default();
        let mut wrong = receipt.clone();
        wrong.width = 7;
        assert!(read(&path, &wrong, 16, &cancel).is_err());
        wrong = receipt.clone();
        wrong.source_asset_id = "0".repeat(64);
        assert!(read(&path, &wrong, 16, &cancel).is_err());
        let mut bytes = std::fs::read(&path).unwrap();
        let end = bytes.len() - 1;
        bytes[end] ^= 1;
        std::fs::write(&path, bytes).unwrap();
        assert!(read(&path, &receipt, 16, &cancel).is_err());
    }
    #[test]
    fn cancellation_and_outside_region_create_no_output_file() {
        let (dir, receipt, _) = fixture(8, 4);
        let out = tempfile::tempdir().unwrap();
        let path = dir.path().join("capture.png");
        let cancel = capture::Cancellation::default();
        cancel.cancel();
        assert!(read(&path, &receipt, 16, &cancel).is_err());
        assert!(
            crop(
                &path,
                &receipt,
                receipt.identity.client_rect,
                out.path(),
                &cancel
            )
            .is_err()
        );
        assert!(!out.path().join("capture.png").exists());
        let invalid = capture::Rect {
            x: -101,
            ..receipt.identity.client_rect
        };
        assert!(
            crop(
                &path,
                &receipt,
                invalid,
                out.path(),
                &capture::Cancellation::default()
            )
            .is_err()
        );
        assert!(!out.path().join("capture.png").exists());
    }
    #[test]
    fn hostile_preview_bound_and_unexpected_color_format_refuse() {
        let (dir, mut receipt, _) = fixture(1, 1);
        let path = dir.path().join("capture.png");
        let cancel = capture::Cancellation::default();
        assert!(read(&path, &receipt, u32::MAX, &cancel).is_err());
        let mut data = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut data, 1, 1);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[99]).unwrap();
            writer.finish().unwrap();
        }
        receipt.source_asset_id = blake3::hash(&data).to_hex().to_string();
        receipt.png_bytes = data.len() as u64;
        std::fs::write(&path, data).unwrap();
        assert!(read(&path, &receipt, 16, &cancel).is_err());
    }

    #[test]
    fn terminal_png_damage_refuses_even_with_matching_file_hash() {
        let (dir, mut receipt, _) = fixture(8, 4);
        let path = dir.path().join("capture.png");
        let original = std::fs::read(&path).unwrap();
        let mut bad_crc = original.clone();
        *bad_crc.last_mut().unwrap() ^= 1;
        for bytes in [original[..original.len() - 12].to_vec(), bad_crc] {
            receipt.source_asset_id = blake3::hash(&bytes).to_hex().to_string();
            receipt.png_bytes = bytes.len() as u64;
            std::fs::write(&path, bytes).unwrap();
            let cancel = capture::Cancellation::default();
            assert!(read(&path, &receipt, 16, &cancel).is_err());
            let out = tempfile::tempdir().unwrap();
            assert!(
                crop(
                    &path,
                    &receipt,
                    receipt.identity.client_rect,
                    out.path(),
                    &cancel
                )
                .is_err()
            );
            assert!(!out.path().join("capture.png").exists());
        }
    }

    #[test]
    fn snapshot_binding_is_checked_without_refreshing_the_os_target() {
        let (dir, receipt, _) = fixture(8, 4);
        let path = dir.path().join("capture.png");
        let cancel = capture::Cancellation::default();
        // Synthetic monitor 1 is never looked up: preview is of the saved file.
        assert!(read(&path, &receipt, 16, &cancel).is_ok());
        for field in 0..5 {
            let mut changed = receipt.clone();
            match field {
                0 => changed.identity.source_kind = "region".into(),
                1 => changed.identity.client_rect.x += 1,
                2 => changed.identity.monitor_id.push('x'),
                3 => changed.identity.frame_id = 0,
                _ => changed.filename = "other.png".into(),
            }
            assert!(read(&path, &changed, 16, &cancel).is_err());
        }
        let renamed = dir.path().join("other.png");
        std::fs::copy(&path, &renamed).unwrap();
        assert!(read(&renamed, &receipt, 16, &cancel).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn preview_does_not_follow_a_replaced_symlink() {
        let (dir, receipt, _) = fixture(8, 4);
        let path = dir.path().join("capture.png");
        let moved = dir.path().join("preserved.png");
        std::fs::rename(&path, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &path).unwrap();
        assert!(read(&path, &receipt, 16, &capture::Cancellation::default()).is_err());
        assert!(moved.is_file());
    }
}
