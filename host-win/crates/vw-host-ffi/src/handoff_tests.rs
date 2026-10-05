//! All clipboard payloads are synthetic and all native clipboard calls are
//! replaced by this injected backend. Files live only in owned temp fixtures.
use crate::{
    worker::{self, Backend, ClipboardSnapshot, RequestContext},
    *,
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Default)]
struct FakeState {
    png: Mutex<Vec<Vec<u8>>>,
    dib: Mutex<Vec<Vec<u8>>>,
    input: Mutex<Option<ClipboardSnapshot>>,
    reads: AtomicUsize,
    hold: AtomicBool,
    entered: AtomicBool,
    fail_second: AtomicBool,
}
struct Fake(Arc<FakeState>);
impl Backend for Fake {
    fn dpi_at_point(&mut self, _: i32, _: i32) -> HostResult<DpiInfo> {
        Err(HostError::DpiUnavailable)
    }
    fn window_dpi_info(&mut self, _: u64) -> HostResult<WindowDpiInfo> {
        Err(HostError::DpiUnavailable)
    }
    fn publish_png(&mut self, _: &[u8], _: &RequestContext) -> HostResult<u32> {
        Err(HostError::UnsupportedPlatform)
    }
    fn publish_image(
        &mut self,
        png: &[u8],
        dib: &[u8],
        context: &RequestContext,
    ) -> HostResult<u32> {
        self.0.entered.store(true, Ordering::Release);
        while self.0.hold.load(Ordering::Acquire) {
            context.checkpoint()?;
            std::thread::sleep(Duration::from_millis(1));
        }
        context.begin_publication()?;
        self.0
            .png
            .lock()
            .map_err(|_| HostError::WorkerUnavailable)?
            .push(png.to_vec());
        if self.0.fail_second.load(Ordering::Acquire) {
            return Err(HostError::ClipboardPublicationFailed);
        }
        self.0
            .dib
            .lock()
            .map_err(|_| HostError::WorkerUnavailable)?
            .push(dib.to_vec());
        Ok(19)
    }
    fn read_image(
        &mut self,
        _: ClipboardReadFormat,
        context: &RequestContext,
    ) -> HostResult<ClipboardSnapshot> {
        context.checkpoint()?;
        self.0.reads.fetch_add(1, Ordering::AcqRel);
        self.0
            .input
            .lock()
            .map_err(|_| HostError::WorkerUnavailable)?
            .take()
            .ok_or(HostError::ClipboardImageUnavailable)
    }
    fn pump_messages(&mut self) {}
}
fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
}
fn root() -> Result<tempfile::TempDir, std::io::Error> {
    #[cfg(target_os = "android")]
    return tempfile::tempdir_in(std::env::current_dir()?);
    #[cfg(not(target_os = "android"))]
    tempfile::tempdir()
}
async fn service(state: &Arc<FakeState>, root: &Path) -> HostResult<Arc<HostService>> {
    let state = state.clone();
    worker::start_with_root(move || Ok(Fake(state)), root.to_owned()).await
}
fn options() -> DibOptions {
    DibOptions {
        allow_depth_reduction: false,
        assume_untagged_srgb: true,
    }
}
fn png_fixture(
    depth: png::BitDepth,
    profile: Option<&[u8]>,
) -> Result<(Vec<u8>, ExportBinding), Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    let source = blake3::hash(b"synthetic original pixels")
        .to_hex()
        .to_string();
    let mut info = png::Info::with_size(2, 1);
    info.color_type = png::ColorType::Rgba;
    info.bit_depth = depth;
    info.icc_profile = profile.map(std::borrow::Cow::Borrowed);
    let mut encoder = png::Encoder::with_info(&mut bytes, info)?;
    encoder.add_itxt_chunk("VisualWorkbench".into(),serde_json::json!({"revision":"r3-abcdef12","source_asset":source,"output_width":2,"output_height":1}).to_string())?;
    let mut writer = encoder.write_header()?;
    if depth == png::BitDepth::Sixteen {
        writer.write_image_data(&[
            0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xff, 0xff, 0xff, 0, 0x80, 0, 0, 0, 0, 0,
        ])?;
    } else {
        writer.write_image_data(&[18, 52, 86, 255, 120, 154, 188, 0])?;
    }
    writer.finish()?;
    let binding = ExportBinding {
        png_blake3: blake3::hash(&bytes).to_hex().to_string(),
        source_asset: source,
        revision: "r3-abcdef12".into(),
    };
    Ok((bytes, binding))
}
fn expire(receipt: &DragFileReceipt) -> TestResult {
    let marker = Path::new(&receipt.path)
        .parent()
        .ok_or("parent")?
        .join("owner.json");
    let mut owner: serde_json::Value = serde_json::from_slice(&fs::read(&marker)?)?;
    owner["expires_at_unix_ms"] = serde_json::json!(0);
    fs::write(marker, serde_json::to_vec(&owner)?)?;
    Ok(())
}
fn assert_no_clipboard(state: &FakeState) -> TestResult {
    assert_eq!(state.reads.load(Ordering::Acquire), 0);
    assert!(state.png.lock().map_err(|_| "PNG lock")?.is_empty());
    assert!(state.dib.lock().map_err(|_| "DIB lock")?.is_empty());
    Ok(())
}
fn pixels(png: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    match crate::dib::decode_png(png)?.pixels {
        vw_raster::Pixels::Rgba8(p) => Ok(p),
        _ => Err("unexpected depth".into()),
    }
}
fn word(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn half(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn dual_copy_retains_export_binding_and_exact_png_while_dib_roundtrips_alpha() -> TestResult {
    let _serial = crate::tests::SERIAL.lock().map_err(|_| "serial")?;
    runtime()?.block_on(async {
        let root = root()?;
        let state = Arc::new(FakeState::default());
        let host = service(&state, root.path()).await?;
        let (png, binding) = png_fixture(png::BitDepth::Eight, None)?;
        let result = host
            .set_clipboard_image(
                png.clone(),
                binding.clone(),
                options(),
                HostOperation::new(),
            )
            .await?;
        assert_eq!(result.binding, binding);
        assert!(result.dib_assumed_srgb);
        assert!(!result.dib_depth_reduced);
        assert_eq!(result.image.clipboard_sequence, 19);
        assert_eq!(*state.png.lock().map_err(|_| "png")?, vec![png.clone()]);
        let dib = state.dib.lock().map_err(|_| "dib")?[0].clone();
        assert_eq!(&dib[..4], &124u32.to_le_bytes());
        assert_eq!(&dib[8..12], &(-1i32).to_le_bytes());
        let imported = crate::dib::import(&dib, false)?;
        assert_eq!(pixels(&imported.png)?, pixels(&png)?);
        host.shutdown().await?;
        root.close()?;
        Ok(())
    })
}

#[test]
fn depth_and_untagged_conversions_require_explicit_choices_before_publication() -> TestResult {
    let _serial = crate::tests::SERIAL.lock().map_err(|_| "serial")?;
    runtime()?.block_on(async {
        let root = root()?;
        let state = Arc::new(FakeState::default());
        let host = service(&state, root.path()).await?;
        let (png, binding) = png_fixture(png::BitDepth::Sixteen, None)?;
        assert!(matches!(
            host.set_clipboard_image(
                png.clone(),
                binding.clone(),
                options(),
                HostOperation::new()
            )
            .await,
            Err(HostError::DepthConversionRequired)
        ));
        assert!(matches!(
            host.set_clipboard_image(
                png.clone(),
                binding.clone(),
                DibOptions {
                    allow_depth_reduction: true,
                    assume_untagged_srgb: false
                },
                HostOperation::new()
            )
            .await,
            Err(HostError::ColorAssumptionRequired)
        ));
        assert_no_clipboard(&state)?;
        let result = host
            .set_clipboard_image(
                png.clone(),
                binding,
                DibOptions {
                    allow_depth_reduction: true,
                    ..options()
                },
                HostOperation::new(),
            )
            .await?;
        assert_eq!(result.image.bit_depth, 16);
        assert!(result.dib_depth_reduced);
        assert_eq!(state.png.lock().map_err(|_| "png")?[0], png);
        let raw = state.dib.lock().map_err(|_| "dib")?[0].clone();
        let decoded = crate::dib::import(&raw, false)?;
        assert_eq!(
            pixels(&decoded.png)?,
            vec![18, 86, 154, 255, 254, 128, 0, 0]
        );
        host.shutdown().await?;
        root.close()?;
        Ok(())
    })
}

#[test]
fn wrong_hash_source_revision_and_duplicate_metadata_cannot_reach_clipboard() -> TestResult {
    let _serial = crate::tests::SERIAL.lock().map_err(|_| "serial")?;
    runtime()?.block_on(async {
        let root = root()?;
        let state = Arc::new(FakeState::default());
        let host = service(&state, root.path()).await?;
        let (png, binding) = png_fixture(png::BitDepth::Eight, None)?;
        for invalid in [
            ExportBinding {
                png_blake3: "0".repeat(64),
                ..binding.clone()
            },
            ExportBinding {
                source_asset: "0".repeat(64),
                ..binding.clone()
            },
            ExportBinding {
                revision: "r4-abcdef12".into(),
                ..binding.clone()
            },
        ] {
            assert!(matches!(
                host.set_clipboard_image(png.clone(), invalid, options(), HostOperation::new())
                    .await,
                Err(HostError::ExportBindingMismatch)
            ));
        }
        let mut malformed = png.clone();
        let position = malformed
            .windows(4)
            .position(|b| b == b"iTXt")
            .ok_or("metadata")?;
        malformed[position + 4] = b'X';
        let bad = ExportBinding {
            png_blake3: blake3::hash(&malformed).to_hex().to_string(),
            ..binding
        };
        assert!(matches!(
            host.set_clipboard_image(malformed, bad, options(), HostOperation::new())
                .await,
            Err(HostError::InvalidPng)
        ));
        let (mut duplicate, mut duplicate_binding) = png_fixture(png::BitDepth::Eight, None)?;
        let at = duplicate
            .windows(4)
            .position(|b| b == b"iTXt")
            .ok_or("metadata")?
            - 4;
        let size = u32::from_be_bytes(duplicate[at..at + 4].try_into()?) as usize;
        let chunk = duplicate[at..at + size + 12].to_vec();
        duplicate.splice(at..at, chunk);
        duplicate_binding.png_blake3 = blake3::hash(&duplicate).to_hex().to_string();
        assert!(matches!(
            host.set_clipboard_image(
                duplicate,
                duplicate_binding,
                options(),
                HostOperation::new()
            )
            .await,
            Err(HostError::ExportBindingMismatch)
        ));
        assert_no_clipboard(&state)?;
        host.shutdown().await?;
        root.close()?;
        Ok(())
    })
}

#[test]
fn rgb_profile_is_embedded_exactly_and_gray_is_never_mislabeled_as_rgb() -> TestResult {
    let profile = moxcms::ColorProfile::new_srgb().encode()?;
    let (png, _) = png_fixture(png::BitDepth::Eight, Some(&profile))?;
    let mut image = crate::dib::decode_png(&png)?;
    let dib = crate::dib::prepare(
        &image,
        DibOptions {
            assume_untagged_srgb: false,
            ..options()
        },
    )?;
    assert!(!dib.assumed_srgb);
    assert_eq!(
        &dib.bytes[dib.bytes.len() - profile.len()..],
        profile.as_slice()
    );
    let imported = crate::dib::import(&dib.bytes, false)?;
    assert_eq!(crate::dib::decode_png(&imported.png)?.icc, Some(profile));
    image.icc.as_mut().ok_or("icc")?[16..20].copy_from_slice(b"GRAY");
    assert!(matches!(
        crate::dib::prepare(&image, options()),
        Err(HostError::ColorConversionRequired)
    ));
    Ok(())
}

#[test]
fn bottom_up_padded_rgb24_and_top_down_rgb32_have_independent_known_pixels() -> TestResult {
    let mut bitmap = vec![0; 56];
    word(&mut bitmap, 0, 40);
    word(&mut bitmap, 4, 2);
    word(&mut bitmap, 8, 2);
    half(&mut bitmap, 12, 1);
    half(&mut bitmap, 14, 24);
    word(&mut bitmap, 20, 16);
    bitmap[40..56].copy_from_slice(&[255, 0, 0, 255, 255, 255, 0, 0, 0, 0, 255, 0, 255, 0, 0, 0]);
    assert!(matches!(
        crate::dib::import(&bitmap, false),
        Err(HostError::ColorAssumptionRequired)
    ));
    let image = crate::dib::import(&bitmap, true)?;
    assert!(image.assumed_srgb);
    assert_eq!(
        pixels(&image.png)?,
        vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255
        ]
    );
    let mut bitmap = vec![0; 48];
    word(&mut bitmap, 0, 40);
    word(&mut bitmap, 4, 2);
    word(&mut bitmap, 8, (-1i32) as u32);
    half(&mut bitmap, 12, 1);
    half(&mut bitmap, 14, 32);
    bitmap[40..].copy_from_slice(&[3, 2, 1, 0, 6, 5, 4, 128]);
    // BI_RGB's fourth byte is reserved, not alpha.
    assert_eq!(
        pixels(&crate::dib::import(&bitmap, true)?.png)?,
        vec![1, 2, 3, 255, 4, 5, 6, 255]
    );
    Ok(())
}

#[test]
fn rgb565_bitfields_are_normalized_and_overlapping_masks_are_refused() -> TestResult {
    let mut bitmap = vec![0; 56];
    word(&mut bitmap, 0, 40);
    word(&mut bitmap, 4, 2);
    word(&mut bitmap, 8, 1);
    half(&mut bitmap, 12, 1);
    half(&mut bitmap, 14, 16);
    word(&mut bitmap, 16, 3);
    word(&mut bitmap, 40, 0xf800);
    word(&mut bitmap, 44, 0x07e0);
    word(&mut bitmap, 48, 0x001f);
    bitmap[52..].copy_from_slice(&[0, 0xf8, 0xe0, 0x07]);
    assert_eq!(
        pixels(&crate::dib::import(&bitmap, true)?.png)?,
        vec![255, 0, 0, 255, 0, 255, 0, 255]
    );
    word(&mut bitmap, 44, 0xf800);
    assert!(matches!(
        crate::dib::import(&bitmap, true),
        Err(HostError::UnsupportedDib)
    ));
    Ok(())
}

#[test]
fn ten_bit_dib_channels_require_explicit_conversion_without_quantization() -> TestResult {
    let mut bitmap = vec![0; 132];
    word(&mut bitmap, 0, 124);
    word(&mut bitmap, 4, 2);
    word(&mut bitmap, 8, (-1i32) as u32);
    half(&mut bitmap, 12, 1);
    half(&mut bitmap, 14, 32);
    word(&mut bitmap, 16, 3);
    word(&mut bitmap, 20, 8);
    word(&mut bitmap, 40, 0x3ff0_0000);
    word(&mut bitmap, 44, 0x000f_fc00);
    word(&mut bitmap, 48, 0x0000_03ff);
    word(&mut bitmap, 52, 0xc000_0000);
    word(&mut bitmap, 56, 0x7352_4742);
    word(&mut bitmap, 124, (256 << 20) | 0xc000_0000);
    word(&mut bitmap, 128, (257 << 20) | 0xc000_0000);
    // Adjacent 10-bit values would both round to 64 in PNG8. Refusal preserves
    // the source so the app can offer an explicit color/depth conversion flow.
    assert_ne!(&bitmap[124..128], &bitmap[128..132]);
    assert!(matches!(
        crate::dib::import(&bitmap, false),
        Err(HostError::DepthConversionRequired)
    ));
    assert!(matches!(
        crate::dib::import(&bitmap, true),
        Err(HostError::DepthConversionRequired)
    ));
    Ok(())
}

#[test]
fn malformed_dib_extents_compression_palette_and_external_profiles_fail_closed() -> TestResult {
    let (png, _) = png_fixture(png::BitDepth::Eight, None)?;
    let image = crate::dib::decode_png(&png)?;
    let valid = crate::dib::prepare(&image, options())?.bytes;
    for (offset, value) in [
        (0, 12),
        (4, u32::MAX),
        (8, i32::MIN as u32),
        (16, 1),
        (20, 1),
        (32, 1),
        (56, 0x4c49_4e4b),
        (56, 0x1234_5678),
        (40, 0x0000_ff00),
    ] {
        let mut bad = valid.clone();
        word(&mut bad, offset, value);
        assert!(crate::dib::import(&bad, true).is_err(), "offset {offset}");
    }
    for size in [0, 3, 39, 123, valid.len() - 1] {
        assert!(crate::dib::import(&valid[..size], true).is_err());
    }
    let mut bad = valid;
    word(&mut bad, 56, 0x4d42_4544);
    word(&mut bad, 112, 124);
    word(&mut bad, 116, u32::MAX);
    assert!(crate::dib::import(&bad, true).is_err());
    Ok(())
}

#[test]
fn paste_is_explicit_read_only_and_bad_preferred_png_does_not_fall_back() -> TestResult {
    let _serial = crate::tests::SERIAL.lock().map_err(|_| "serial")?;
    runtime()?.block_on(async {
        let root = root()?;
        let state = Arc::new(FakeState::default());
        let host = service(&state, root.path()).await?;
        assert_no_clipboard(&state)?;
        let (png, _) = png_fixture(png::BitDepth::Eight, None)?;
        *state.input.lock().map_err(|_| "input")? = Some(ClipboardSnapshot {
            bytes: png.clone(),
            format: ClipboardImageFormat::Png,
            sequence: 7,
        });
        let result = host
            .read_clipboard_image(
                ClipboardReadOptions {
                    format: ClipboardReadFormat::PreferPng,
                    assume_untagged_srgb: false,
                },
                HostOperation::new(),
            )
            .await?;
        assert_eq!(result.png, png);
        assert_eq!(result.clipboard_sequence, 7);
        assert_eq!(result.original_payload_blake3, result.png_blake3);
        assert!(!result.assumed_srgb);
        let diagnostics = format!("{result:?}");
        assert!(!diagnostics.contains(&format!("{:?}", result.png)));
        assert!(!diagnostics.contains(&result.original_payload_blake3));
        assert!(!diagnostics.contains(&result.png_blake3));
        *state.input.lock().map_err(|_| "input")? = Some(ClipboardSnapshot {
            bytes: vec![0; 40],
            format: ClipboardImageFormat::Png,
            sequence: 8,
        });
        let rejected = host
            .read_clipboard_image(
                ClipboardReadOptions {
                    format: ClipboardReadFormat::PreferPng,
                    assume_untagged_srgb: true,
                },
                HostOperation::new(),
            )
            .await;
        assert!(
            matches!(rejected, Err(HostError::InvalidPng)),
            "preferred PNG rejection actual error: {:?}",
            rejected.as_ref().err()
        );
        assert_eq!(state.reads.load(Ordering::Acquire), 2);
        assert!(state.png.lock().map_err(|_| "png")?.is_empty());
        assert!(state.dib.lock().map_err(|_| "dib")?.is_empty());
        host.shutdown().await?;
        root.close()?;
        Ok(())
    })
}

#[test]
fn paste_dib_returns_normalized_pixels_and_raw_payload_provenance() -> TestResult {
    let _serial = crate::tests::SERIAL.lock().map_err(|_| "serial")?;
    runtime()?.block_on(async {
        let root = root()?;
        let state = Arc::new(FakeState::default());
        let host = service(&state, root.path()).await?;
        let (png, _) = png_fixture(png::BitDepth::Eight, None)?;
        let dib = crate::dib::prepare(&crate::dib::decode_png(&png)?, options())?.bytes;
        let raw_hash = blake3::hash(&dib).to_hex().to_string();
        *state.input.lock().map_err(|_| "input")? = Some(ClipboardSnapshot {
            bytes: dib,
            format: ClipboardImageFormat::DibV5,
            sequence: 23,
        });
        let result = host
            .read_clipboard_image(
                ClipboardReadOptions {
                    format: ClipboardReadFormat::DibV5,
                    assume_untagged_srgb: false,
                },
                HostOperation::new(),
            )
            .await?;
        assert_eq!(result.original_payload_blake3, raw_hash);
        assert_eq!(pixels(&result.png)?, pixels(&png)?);
        assert_eq!(result.format, ClipboardImageFormat::DibV5);
        assert_eq!((result.width, result.height), (2, 1));
        host.shutdown().await?;
        root.close()?;
        Ok(())
    })
}

#[test]
fn dual_format_future_drop_cancels_before_commit_and_partial_publication_is_distinct() -> TestResult
{
    let _serial = crate::tests::SERIAL.lock().map_err(|_| "serial")?;
    runtime()?.block_on(async {
        let root = root()?;
        let state = Arc::new(FakeState::default());
        let host = service(&state, root.path()).await?;
        let (png, binding) = png_fixture(png::BitDepth::Eight, None)?;
        state.hold.store(true, Ordering::Release);
        let operation = HostOperation::new();
        let mut pending = Box::pin(host.set_clipboard_image(
            png.clone(),
            binding.clone(),
            options(),
            operation.clone(),
        ));
        use std::{
            future::Future,
            task::{Context, Poll, Waker},
        };
        assert!(matches!(
            pending
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        let end = Instant::now() + Duration::from_secs(3);
        while !state.entered.load(Ordering::Acquire) {
            if Instant::now() > end {
                return Err("worker did not enter".into());
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        drop(pending);
        assert_eq!(operation.status(), HostOperationStatus::Cancelled);
        state.hold.store(false, Ordering::Release);
        state.fail_second.store(true, Ordering::Release);
        let operation = HostOperation::new();
        assert!(matches!(
            host.set_clipboard_image(png, binding, options(), operation.clone())
                .await,
            Err(HostError::ClipboardPublicationFailed)
        ));
        assert!(!operation.cancel());
        assert_eq!(state.png.lock().map_err(|_| "png")?.len(), 1);
        assert!(state.dib.lock().map_err(|_| "dib")?.is_empty());
        host.shutdown().await?;
        root.close()?;
        Ok(())
    })
}

#[test]
fn drag_files_are_exact_bound_leased_and_never_use_clipboard() -> TestResult {
    let _serial = crate::tests::SERIAL.lock().map_err(|_| "serial")?;
    runtime()?.block_on(async {
        let root = root()?;
        let state = Arc::new(FakeState::default());
        let host = service(&state, root.path()).await?;
        let (png, binding) = png_fixture(png::BitDepth::Sixteen, None)?;
        let source = root.path().join("source.png");
        fs::write(&source, &png)?;
        let lease = host
            .stage_drag_png(
                source.to_string_lossy().into_owned(),
                binding.clone(),
                HostOperation::new(),
            )
            .await?;
        let receipt = lease.receipt()?;
        assert_eq!(receipt.binding, binding);
        assert_eq!(receipt.bit_depth, 16);
        assert_eq!(fs::read(&receipt.path)?, png);
        assert_eq!(fs::read(&source)?, png);
        assert!(
            Path::new(&receipt.path)
                .starts_with(root.path().join("VisualWorkbench").join("drag-v1"))
        );
        expire(&receipt)?;
        assert_eq!(host.cleanup_drag_files().await?.removed, 0);
        assert!(Path::new(&receipt.path).is_file());
        host.release_drag_file(receipt.lease_id.clone()).await?;
        assert!(Path::new(&receipt.path).is_file());
        assert_eq!(host.cleanup_drag_files().await?.removed, 1);
        assert!(!Path::new(&receipt.path).exists());
        assert!(source.is_file());
        assert_no_clipboard(&state)?;
        assert!(matches!(
            host.release_drag_file(receipt.lease_id).await,
            Err(HostError::UnknownDragLease)
        ));
        host.shutdown().await?;
        root.close()?;
        Ok(())
    })
}

#[test]
fn failed_drag_binding_and_cancel_preserve_original_and_remove_only_owned_stage() -> TestResult {
    let _serial = crate::tests::SERIAL.lock().map_err(|_| "serial")?;
    runtime()?.block_on(async {
        let root = root()?;
        let state = Arc::new(FakeState::default());
        let host = service(&state, root.path()).await?;
        let (png, binding) = png_fixture(png::BitDepth::Eight, None)?;
        let source = root.path().join("source.png");
        fs::write(&source, &png)?;
        let invalid = ExportBinding {
            revision: "r9-abcdef12".into(),
            ..binding.clone()
        };
        assert!(matches!(
            host.stage_drag_png(
                source.to_string_lossy().into_owned(),
                invalid,
                HostOperation::new()
            )
            .await,
            Err(HostError::ExportBindingMismatch)
        ));
        let cancel = HostOperation::new();
        assert!(cancel.cancel());
        assert!(matches!(
            host.stage_drag_png(source.to_string_lossy().into_owned(), binding, cancel)
                .await,
            Err(HostError::Cancelled)
        ));
        assert_eq!(fs::read(&source)?, png);
        assert_eq!(
            fs::read_dir(root.path().join("VisualWorkbench/drag-v1"))?
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name() != "admission.lock")
                .count(),
            0
        );
        assert_no_clipboard(&state)?;
        host.shutdown().await?;
        root.close()?;
        Ok(())
    })
}

#[test]
fn orphan_cleanup_honors_other_process_lease_and_preserves_unknown_files() -> TestResult {
    let _serial = crate::tests::SERIAL.lock().map_err(|_| "serial")?;
    runtime()?.block_on(async {
        let root = root()?;
        let state = Arc::new(FakeState::default());
        let host = service(&state, root.path()).await?;
        let other = service(&state, root.path()).await?;
        let (png, binding) = png_fixture(png::BitDepth::Eight, None)?;
        let source = root.path().join("source.png");
        fs::write(&source, &png)?;
        let a_lease = host
            .stage_drag_png(
                source.to_string_lossy().into_owned(),
                binding.clone(),
                HostOperation::new(),
            )
            .await?;
        let a = a_lease.receipt()?;
        expire(&a)?;
        assert_eq!(other.cleanup_drag_files().await?.removed, 0);
        assert!(Path::new(&a.path).is_file());
        host.release_drag_file(a.lease_id).await?;
        assert_eq!(other.cleanup_drag_files().await?.removed, 1);
        let b_lease = host
            .stage_drag_png(
                source.to_string_lossy().into_owned(),
                binding,
                HostOperation::new(),
            )
            .await?;
        let b = b_lease.receipt()?;
        expire(&b)?;
        let foreign = PathBuf::from(&b.path)
            .parent()
            .ok_or("parent")?
            .join("foreign.txt");
        fs::write(&foreign, b"preserve this unrelated entry")?;
        host.release_drag_file(b.lease_id).await?;
        assert_eq!(other.cleanup_drag_files().await?.removed, 0);
        assert_eq!(fs::read(&foreign)?, b"preserve this unrelated entry");
        assert!(Path::new(&b.path).is_file());
        host.shutdown().await?;
        other.shutdown().await?;
        root.close()?;
        Ok(())
    })
}

#[test]
fn concurrent_drag_stores_serialize_byte_and_entry_admission() -> TestResult {
    use std::sync::mpsc;
    for check_entries in [false, true] {
        let root = root()?;
        let (png, binding) = png_fixture(png::BitDepth::Eight, None)?;
        let source = root.path().join("source.png");
        fs::write(&source, &png)?;
        // Measure the real filesystem footprint, including its ownership marker,
        // independently of the implementation's quota arithmetic.
        let mut calibration = crate::drag::DragStore::new(root.path())?;
        let lease =
            calibration.stage(&source, binding.clone(), &worker::handoff_test_context()?)?;
        let receipt = lease.receipt()?;
        let directory = Path::new(&receipt.path).parent().ok_or("parent")?;
        let mut footprint = 0;
        for item in fs::read_dir(directory)? {
            footprint += item?.metadata()?.len();
        }
        expire(&receipt)?;
        drop(lease);
        assert_eq!(calibration.cleanup(crate::drag::unix_ms()?)?.removed, 1);
        drop(calibration);

        let byte_limit = if check_entries {
            1024 * 1024
        } else {
            footprint * 2 - 1
        };
        let entry_limit = if check_entries { 1 } else { 256 };
        let mut first = crate::drag::DragStore::with_quota(root.path(), byte_limit, entry_limit)?;
        let mut second = crate::drag::DragStore::with_quota(root.path(), byte_limit, entry_limit)?;
        let (admitted, admission) = mpsc::sync_channel(1);
        let (resume, resumed) = mpsc::sync_channel(1);
        first.after_admission(move || {
            admitted
                .send(())
                .map_err(|_| HostError::WorkerUnavailable)?;
            resumed
                .recv_timeout(Duration::from_secs(3))
                .map_err(|_| HostError::Timeout)
        });
        let (started, start) = mpsc::sync_channel(1);
        let (completed, completion) = mpsc::sync_channel(1);
        let first_source = source.clone();
        let first_binding = binding.clone();
        let (first, first_result, second, second_result) = std::thread::scope(|scope| {
            let first_thread = scope.spawn(move || {
                let result = worker::handoff_test_context()
                    .and_then(|context| first.stage(&first_source, first_binding, &context));
                (first, result)
            });
            admission.recv_timeout(Duration::from_secs(3))?;
            let second_thread = scope.spawn(move || {
                let _ = started.send(());
                let result = worker::handoff_test_context()
                    .and_then(|context| second.stage(&source, binding, &context));
                let _ = completed.send(result.is_ok());
                (second, result)
            });
            start.recv_timeout(Duration::from_secs(3))?;
            // The first stage is paused after checking its quota and before
            // publishing files. A competing admission must still wait.
            let before_release = completion.recv_timeout(Duration::from_millis(50));
            resume.send(())?;
            let (first, first_result) = first_thread.join().map_err(|_| "first stage panic")?;
            let (second, second_result) = second_thread.join().map_err(|_| "second stage panic")?;
            assert!(matches!(
                before_release,
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            Ok::<_, Box<dyn std::error::Error>>((first, first_result, second, second_result))
        })?;
        let lease = first_result?;
        if check_entries {
            assert!(matches!(second_result, Err(HostError::Busy)));
        } else {
            assert!(matches!(second_result, Err(HostError::SizeLimit)));
        }
        let staged_root = root.path().join("VisualWorkbench/drag-v1");
        let mut retained = 0;
        let mut directories = 0;
        for entry in fs::read_dir(staged_root)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                directories += 1;
                for child in fs::read_dir(entry.path())? {
                    retained += child?.metadata()?.len();
                }
            }
        }
        assert_eq!(directories, 1);
        assert!(retained <= byte_limit);
        drop(lease);
        drop(first);
        drop(second);
        root.close()?;
    }
    Ok(())
}

#[test]
fn traversal_relative_paths_and_foreign_owned_root_are_never_followed() -> TestResult {
    let _serial = crate::tests::SERIAL.lock().map_err(|_| "serial")?;
    runtime()?.block_on(async {
        let root = root()?;
        let state = Arc::new(FakeState::default());
        let host = service(&state, root.path()).await?;
        let (png, binding) = png_fixture(png::BitDepth::Eight, None)?;
        let source = root.path().join("source.png");
        fs::write(&source, &png)?;
        assert!(matches!(
            host.stage_drag_png("source.png".into(), binding.clone(), HostOperation::new())
                .await,
            Err(HostError::HandoffStorage)
        ));
        fs::File::create(root.path().join("VisualWorkbench"))?
            .write_all(b"foreign file, never replace")?;
        assert!(matches!(
            host.stage_drag_png(
                source.to_string_lossy().into_owned(),
                binding,
                HostOperation::new()
            )
            .await,
            Err(HostError::HandoffStorage)
        ));
        assert_eq!(
            fs::read(root.path().join("VisualWorkbench"))?,
            b"foreign file, never replace"
        );
        assert!(matches!(
            host.release_drag_file("../source.png".into()).await,
            Err(HostError::UnknownDragLease)
        ));
        assert_no_clipboard(&state)?;
        host.shutdown().await?;
        root.close()?;
        Ok(())
    })
}

#[test]
fn dropping_drag_result_releases_its_lease_and_shutdown_invalidates_handles() -> TestResult {
    let _serial = crate::tests::SERIAL.lock().map_err(|_| "serial")?;
    runtime()?.block_on(async {
        let root = root()?;
        let state = Arc::new(FakeState::default());
        let host = service(&state, root.path()).await?;
        let (png, binding) = png_fixture(png::BitDepth::Eight, None)?;
        let source = root.path().join("source.png");
        fs::write(&source, &png)?;
        let lease = host
            .stage_drag_png(
                source.to_string_lossy().into_owned(),
                binding.clone(),
                HostOperation::new(),
            )
            .await?;
        let receipt = lease.receipt()?;
        expire(&receipt)?;
        drop(lease);
        assert_eq!(host.cleanup_drag_files().await?.removed, 1);
        assert!(!Path::new(&receipt.path).exists());
        let lease = host
            .stage_drag_png(
                source.to_string_lossy().into_owned(),
                binding,
                HostOperation::new(),
            )
            .await?;
        host.shutdown().await?;
        assert!(matches!(lease.receipt(), Err(HostError::UnknownDragLease)));
        root.close()?;
        Ok(())
    })
}

#[test]
fn cancelled_paste_never_reads_and_drag_active_count_is_bounded() -> TestResult {
    let _serial = crate::tests::SERIAL.lock().map_err(|_| "serial")?;
    runtime()?.block_on(async {
        let root = root()?;
        let state = Arc::new(FakeState::default());
        let host = service(&state, root.path()).await?;
        let cancelled = HostOperation::new();
        assert!(cancelled.cancel());
        assert!(matches!(
            host.read_clipboard_image(
                ClipboardReadOptions {
                    format: ClipboardReadFormat::PreferPng,
                    assume_untagged_srgb: false
                },
                cancelled
            )
            .await,
            Err(HostError::Cancelled)
        ));
        assert_no_clipboard(&state)?;
        let (png, binding) = png_fixture(png::BitDepth::Eight, None)?;
        let source = root.path().join("source.png");
        fs::write(&source, &png)?;
        let path = source.to_string_lossy().into_owned();
        let mut leases = Vec::new();
        for _ in 0..crate::drag::MAX_ACTIVE {
            leases.push(
                host.stage_drag_png(path.clone(), binding.clone(), HostOperation::new())
                    .await?,
            );
        }
        assert!(matches!(
            host.stage_drag_png(path.clone(), binding.clone(), HostOperation::new())
                .await,
            Err(HostError::Busy)
        ));
        let released = leases.pop().ok_or("lease")?;
        released.release();
        released.release();
        drop(released);
        leases.push(
            host.stage_drag_png(path, binding, HostOperation::new())
                .await?,
        );
        host.shutdown().await?;
        drop(leases);
        root.close()?;
        Ok(())
    })
}
