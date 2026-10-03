mod support;
use std::{
    cell::Cell,
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use support::*;
use vw_raster::*;

struct Scratch {
    file: Option<File>,
    path: PathBuf,
}
impl Scratch {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = if cfg!(target_os = "android") {
            std::env::current_dir()?
        } else {
            std::env::temp_dir()
        };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let path = root.join(format!(
            "vw-raster-stream-{}-{stamp}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self {
            file: Some(file),
            path,
        })
    }
    fn file(&mut self) -> Result<&mut File, Box<dyn std::error::Error>> {
        self.file.as_mut().ok_or_else(|| "closed scratch".into())
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        drop(self.file.take());
        // This exact create_new path is owned by this fixture. No recursive delete.
        let _ = std::fs::remove_file(&self.path);
    }
}
fn limit(rows: u32) -> StreamLimits {
    StreamLimits {
        max_strip_rows: rows,
        ..StreamLimits::default()
    }
}
fn png(image: &DecodedImage) -> Result<Vec<u8>, RasterError> {
    Ok(export(
        image,
        &request(if image.pixels.bit_depth() == 16 {
            ExportFormat::Png16
        } else {
            ExportFormat::Png8
        }),
    )?
    .bytes)
}

#[test]
fn clean_stream_matches_samples_and_bytes_for_every_strip_height() -> TestResult {
    for depth in [8, 16] {
        let original = source(37, 29, depth);
        let before = original.clone();
        let mut req = request(if depth == 16 {
            ExportFormat::Png16
        } else {
            ExportFormat::Png8
        });
        req.memory_budget_bytes = 64 * 1024 * 1024;
        let mut first = None;
        for rows in [1, 3, 17, 128] {
            let mut source = BorrowedSource::new(&original)?;
            let mut output = Vec::new();
            let report = export_png_to(&mut source, &req, limit(rows), &mut output, &|| false)?;
            assert_eq!(report.encoded_bytes, output.len() as u64);
            assert!(report.plan.estimated_peak_bytes <= req.memory_budget_bytes);
            assert_eq!(
                decode(&output, DecodeLimits::default())?.pixels,
                original.pixels
            );
            if let Some(bytes) = &first {
                assert_eq!(&output, bytes);
            } else {
                first = Some(output);
            }
        }
        assert_eq!(original, before);
    }
    Ok(())
}

#[test]
fn marked_strips_equal_whole_render_including_transforms_and_region() -> TestResult {
    let (original, mut project, doc) = all_annotations()?;
    for (i, object) in project.objects.values_mut().enumerate() {
        if i % 2 == 0 {
            object.state.transform = Some(vw_proto::v1::Affine {
                a: 0.97,
                b: 0.03,
                c: -0.03,
                d: 0.97,
                e: 2.25,
                f: 3.5,
            });
        }
    }
    let whole = render_document(&original, &project, &doc, &NoAssets, options())?;
    for region in [
        None,
        Some(Region {
            x: 11,
            y: 7,
            width: 163,
            height: 101,
        }),
    ] {
        let mut req = request(ExportFormat::Png16);
        req.region = region;
        let expected = decode(&export(&whole, &req)?.bytes, DecodeLimits::default())?;
        let mut first = None;
        for rows in [1, 7, 31, 128] {
            let mut output = Vec::new();
            let mut borrowed = BorrowedSource::new(&original)?;
            export_document_png_to(
                &mut borrowed,
                MarkedDocument {
                    project: &project,
                    document: &doc,
                    assets: &NoAssets,
                    options: options(),
                },
                &req,
                limit(rows),
                &mut output,
                &|| false,
            )?;
            let actual = decode(&output, DecodeLimits::default())?;
            assert_pixels_equal(
                &actual.pixels,
                &expected.pixels,
                &format!("strip rows={rows}, region={region:?}"),
            )?;
            if let Some(bytes) = &first {
                assert_eq!(&output, bytes);
            } else {
                first = Some(output);
            }
        }
    }
    Ok(())
}

#[test]
fn multiply_and_adjustment_strip_parity() -> TestResult {
    let original = source(47, 39, 16);
    let (mut project, doc, layer) = project(&original)?;
    let (oid, ellipse) = object(
        &doc,
        &layer,
        8,
        vw_proto::v1::object_state::Shape::Ellipse(rect(2.25, 3.5, 35.25, 29.0)),
    )?;
    project.objects.insert(oid, ellipse);
    let multiply = project.layers.get_mut(&layer).ok_or("layer")?;
    multiply.blend = "multiply".into();
    multiply.opacity = 0.37;
    let adjustment_id = id(9)?;
    let mut adjustment = multiply.clone();
    adjustment.definition.layer_id = Some(adjustment_id.to_proto());
    adjustment.definition.order_key = "W".into();
    adjustment.definition.kind = "adjustment".into();
    adjustment.blend = "normal".into();
    project.layers.insert(adjustment_id.clone(), adjustment);
    let (oid, adjustment) = object(
        &doc,
        &adjustment_id,
        10,
        vw_proto::v1::object_state::Shape::Adjustment(vw_proto::v1::Adjustment {
            brightness: 0.05,
            contrast: 0.12,
            levels: vec![0.0, 1.0, 1.2, 0.0, 1.0],
        }),
    )?;
    project.objects.insert(oid, adjustment);
    let whole = render_document(&original, &project, &doc, &NoAssets, options())?;
    let mut bytes = Vec::new();
    export_document_png_to(
        &mut BorrowedSource::new(&original)?,
        MarkedDocument {
            project: &project,
            document: &doc,
            assets: &NoAssets,
            options: options(),
        },
        &request(ExportFormat::Png16),
        limit(3),
        &mut bytes,
        &|| false,
    )?;
    assert_pixels_equal(
        &decode(&bytes, DecodeLimits::default())?.pixels,
        &whole.pixels,
        "multiply and adjustment",
    )?;
    Ok(())
}

#[test]
fn marked_crop_and_strips_cross_canonical_tile_boundaries_exactly() -> TestResult {
    let (_, mut project, doc) = all_annotations()?;
    let original = source(513, 387, 16);
    let asset = project
        .assets
        .get_mut(&original.source_asset)
        .ok_or("source")?;
    asset.width = original.width;
    asset.height = original.height;
    for object in project.objects.values_mut() {
        object.state.transform = Some(vw_proto::v1::Affine {
            a: 1.17,
            b: 0.13,
            c: -0.09,
            d: 1.11,
            e: 230.125,
            f: 238.375,
        });
    }
    let whole = render_document(&original, &project, &doc, &NoAssets, options())?;
    let mut req = request(ExportFormat::Png16);
    req.region = Some(Region {
        x: 237,
        y: 247,
        width: 159,
        height: 121,
    });
    let expected = decode(&export(&whole, &req)?.bytes, DecodeLimits::default())?;
    let mut first = None;
    for rows in [3, 19, 128] {
        let mut bytes = Vec::new();
        export_document_png_to(
            &mut BorrowedSource::new(&original)?,
            MarkedDocument {
                project: &project,
                document: &doc,
                assets: &NoAssets,
                options: options(),
            },
            &req,
            limit(rows),
            &mut bytes,
            &|| false,
        )?;
        assert_pixels_equal(
            &decode(&bytes, DecodeLimits::default())?.pixels,
            &expected.pixels,
            &format!("tile-crossing strip rows={rows}"),
        )?;
        if let Some(expected_bytes) = &first {
            assert_eq!(&bytes, expected_bytes, "PNG byte parity");
        } else {
            first = Some(bytes);
        }
    }
    Ok(())
}

#[test]
fn spool_preserves_8_and_16bit_and_reads_regions_without_full_buffers() -> TestResult {
    for depth in [8, 16] {
        let original = source(37, 29, depth);
        let bytes = png(&original)?;
        let mut scratch = Scratch::new()?;
        let mut spool = PngSpool::decode(
            &bytes,
            scratch.file()?,
            SpoolLimits {
                decode: DecodeLimits {
                    max_memory_bytes: 64 * 1024 * 1024,
                    ..DecodeLimits::default()
                },
                ..SpoolLimits::default()
            },
            &|| false,
        )?;
        assert_eq!(
            spool.scratch_bytes(),
            37 * 29 * if depth == 16 { 8 } else { 4 }
        );
        assert_eq!(spool.info().source_asset, vw_model::AssetId::hash(&bytes));
        for region in [
            Region {
                x: 0,
                y: 0,
                width: 37,
                height: 29,
            },
            Region {
                x: 11,
                y: 3,
                width: 13,
                height: 19,
            },
        ] {
            assert_eq!(
                spool
                    .read_region(region, 64 * 1024 * 1024, &|| false)?
                    .pixels,
                original.crop(region)?.pixels
            );
        }
        let mut output = Vec::new();
        export_png_to(
            &mut spool,
            &request(if depth == 16 {
                ExportFormat::Png16
            } else {
                ExportFormat::Png8
            }),
            limit(2),
            &mut output,
            &|| false,
        )?;
        assert_eq!(
            decode(&output, DecodeLimits::default())?.pixels,
            original.pixels
        );
    }
    Ok(())
}
fn orientation_png(orientation: u16) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let original = source(5, 3, 8);
    let mut info = png::Info::with_size(5, 3);
    info.color_type = png::ColorType::Rgba;
    info.bit_depth = png::BitDepth::Eight;
    // Little-endian TIFF, one SHORT orientation entry, no following IFD.
    let mut exif = vec![
        b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0,
    ];
    exif.extend_from_slice(&orientation.to_le_bytes());
    exif.extend_from_slice(&[0; 6]);
    info.exif_metadata = Some(std::borrow::Cow::Owned(exif));
    let mut bytes = Vec::new();
    let encoder = png::Encoder::with_info(&mut bytes, info)?;
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&original.pixels.rgba8())?;
    writer.finish()?;
    Ok(bytes)
}
#[test]
fn spool_matches_full_decoder_for_all_exif_orientations() -> TestResult {
    for orientation in 1..=8 {
        let bytes = orientation_png(orientation)?;
        let expected = decode(&bytes, DecodeLimits::default())?;
        let mut scratch = Scratch::new()?;
        let mut spool =
            PngSpool::decode(&bytes, scratch.file()?, SpoolLimits::default(), &|| false)?;
        assert_eq!(spool.info().orientation_applied, orientation as u8);
        let region = Region {
            x: 1,
            y: 1,
            width: expected.width - 2,
            height: expected.height - 2,
        };
        assert_eq!(
            spool
                .read_region(region, 64 * 1024 * 1024, &|| false)?
                .pixels,
            expected.crop(region)?.pixels,
            "orientation {orientation}"
        );
    }
    Ok(())
}

#[test]
fn budgets_cancel_and_output_failure_leave_original_unchanged() -> TestResult {
    let original = source(43, 31, 16);
    let before = original.clone();
    let mut source = BorrowedSource::new(&original)?;
    let mut req = request(ExportFormat::Png16);
    req.memory_budget_bytes = source.resident_bytes();
    let mut output = Vec::new();
    assert!(matches!(
        export_png_to(&mut source, &req, limit(2), &mut output, &|| false),
        Err(RasterError::Memory { .. })
    ));
    assert!(output.is_empty());
    req.memory_budget_bytes = 64 * 1024 * 1024;
    assert!(matches!(
        export_png_to(
            &mut source,
            &req,
            StreamLimits {
                max_encoded_bytes: 100,
                ..limit(2)
            },
            &mut output,
            &|| false
        ),
        Err(RasterError::EncodedLimit { limit: 100 })
    ));
    assert!(output.len() <= 100);
    output.clear();
    let calls = Cell::new(0);
    let cancelled = || {
        let n = calls.get();
        calls.set(n + 1);
        n > 30
    };
    assert!(matches!(
        export_png_to(&mut source, &req, limit(2), &mut output, &cancelled),
        Err(RasterError::Cancelled)
    ));
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("fixture failure"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    assert!(matches!(
        export_png_to(&mut source, &req, limit(2), &mut Broken, &|| false),
        Err(RasterError::Io)
    ));
    assert_eq!(original, before);
    Ok(())
}

#[test]
fn spool_limits_corruption_cancellation_and_nonempty_scratch_fail_closed() -> TestResult {
    let bytes = png(&source(11, 13, 16))?;
    let mut scratch = Scratch::new()?;
    assert!(matches!(
        PngSpool::decode(
            &bytes,
            scratch.file()?,
            SpoolLimits {
                max_scratch_bytes: 1,
                ..SpoolLimits::default()
            },
            &|| false
        ),
        Err(RasterError::ScratchLimit { .. })
    ));
    assert_eq!(scratch.file()?.metadata()?.len(), 0);
    assert!(matches!(
        PngSpool::decode(&bytes, scratch.file()?, SpoolLimits::default(), &|| true),
        Err(RasterError::Cancelled)
    ));
    scratch.file()?.write_all(b"preserved")?;
    assert!(matches!(
        PngSpool::decode(&bytes, scratch.file()?, SpoolLimits::default(), &|| false),
        Err(RasterError::Invalid("scratch must be fresh and empty"))
    ));
    assert_eq!(std::fs::read(&scratch.path)?, b"preserved");
    let mut corrupt = bytes.clone();
    let n = corrupt.len();
    corrupt[n - 1] ^= 1;
    let mut fresh = Scratch::new()?;
    assert!(PngSpool::decode(&corrupt, fresh.file()?, SpoolLimits::default(), &|| false).is_err());
    assert!(
        PngSpool::decode(
            &bytes[..bytes.len() - 9],
            Scratch::new()?.file()?,
            SpoolLimits::default(),
            &|| false
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn generated_srgb_header_is_fixed_and_supplied_profile_is_untouched() -> TestResult {
    let original = source(11, 9, 8);
    let mut req = request(ExportFormat::Png8);
    req.color = ColorPolicy::ConvertToSrgb {
        assume_untagged_srgb: true,
    };
    let first = export(&original, &req)?;
    let image = decode(&first.bytes, DecodeLimits::default())?;
    let icc = image.icc.as_ref().ok_or("missing ICC")?;
    assert_eq!(&icc[24..36], &[0x07, 0xd0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0]);
    assert_eq!(first.bytes, export(&original, &req)?.bytes);
    req.color = ColorPolicy::Preserve;
    let mut streamed = Vec::new();
    export_png_to(
        &mut BorrowedSource::new(&image)?,
        &req,
        limit(2),
        &mut streamed,
        &|| false,
    )?;
    assert_eq!(decode(&streamed, DecodeLimits::default())?.icc, image.icc);
    Ok(())
}

#[test]
fn baseline_jpeg_spool_matches_decoder_and_refuses_progressive_before_scratch() -> TestResult {
    let original = source(63, 49, 8);
    let mut req = request(ExportFormat::Jpeg { quality: 93 });
    req.alpha = AlphaPolicy::Matte([255, 255, 255]);
    let bytes = export(&original, &req)?.bytes;
    let expected = decode(&bytes, DecodeLimits::default())?;
    let mut scratch = Scratch::new()?;
    let mut spool = JpegSpool::decode(&bytes, scratch.file()?, SpoolLimits::default(), &|| false)?;
    let r = Region {
        x: 7,
        y: 13,
        width: 39,
        height: 31,
    };
    assert_eq!(
        spool.read_region(r, 64 * 1024 * 1024, &|| false)?.pixels,
        expected.crop(r)?.pixels
    );
    assert_eq!(spool.info().source_asset, expected.source_asset);
    drop(spool);
    let mut corrupt = bytes.clone();
    let marker = corrupt
        .windows(2)
        .position(|v| v == [0xff, 0xc0])
        .ok_or("baseline marker")?;
    corrupt[marker + 1] = 0xc2;
    let mut fresh = Scratch::new()?;
    assert!(matches!(
        JpegSpool::decode(&corrupt, fresh.file()?, SpoolLimits::default(), &|| false),
        Err(RasterError::Unsupported(_))
    ));
    assert_eq!(fresh.file()?.metadata()?.len(), 0);
    let limits = SpoolLimits {
        decode: DecodeLimits {
            max_memory_bytes: 1,
            ..DecodeLimits::default()
        },
        ..SpoolLimits::default()
    };
    assert!(matches!(
        JpegSpool::decode(&bytes, fresh.file()?, limits, &|| false),
        Err(RasterError::Memory { .. })
    ));
    assert_eq!(fresh.file()?.metadata()?.len(), 0);
    Ok(())
}

#[test]
fn streamed_matte_conversion_and_explicit_depth_reduction_equal_buffered_export() -> TestResult {
    let original = source(47, 39, 16);
    let mut req = request(ExportFormat::Png8);
    req.color = ColorPolicy::ConvertToSrgb {
        assume_untagged_srgb: true,
    };
    req.alpha = AlphaPolicy::Matte([23, 87, 196]);
    let mut output = Vec::new();
    assert!(matches!(
        export_png_to(
            &mut BorrowedSource::new(&original)?,
            &req,
            limit(3),
            &mut output,
            &|| false
        ),
        Err(RasterError::Depth)
    ));
    assert!(output.is_empty());
    req.allow_depth_reduction = true;
    let expected = decode(&export(&original, &req)?.bytes, DecodeLimits::default())?;
    export_png_to(
        &mut BorrowedSource::new(&original)?,
        &req,
        limit(3),
        &mut output,
        &|| false,
    )?;
    let actual = decode(&output, DecodeLimits::default())?;
    assert_eq!(actual.pixels, expected.pixels);
    assert_eq!(actual.icc, expected.icc);
    Ok(())
}

#[test]
fn source_project_binding_and_shape_budget_are_checked_before_output() -> TestResult {
    let original = source(13, 17, 8);
    let (mut project, doc, layer) = project(&original)?;
    project
        .assets
        .get_mut(&original.source_asset)
        .ok_or("asset")?
        .width += 1;
    let mut output = Vec::new();
    let req = request(ExportFormat::Png8);
    assert!(matches!(
        export_document_png_to(
            &mut BorrowedSource::new(&original)?,
            MarkedDocument {
                project: &project,
                document: &doc,
                assets: &NoAssets,
                options: options()
            },
            &req,
            limit(1),
            &mut output,
            &|| false
        ),
        Err(RasterError::Invalid("document source metadata"))
    ));
    assert!(output.is_empty());
    project
        .assets
        .get_mut(&original.source_asset)
        .ok_or("asset")?
        .width -= 1;
    let (oid, text) = object(
        &doc,
        &layer,
        12,
        vw_proto::v1::object_state::Shape::Text(vw_proto::v1::TextObject {
            text: "W".repeat(5000),
            font_family: "Inter".into(),
            font_size: 20.0,
            anchor: Some(point(0.0, 20.0)),
        }),
    )?;
    project.objects.insert(oid, text);
    let mut req = req;
    req.memory_budget_bytes = 64 * 1024 * 1024;
    assert!(matches!(
        export_document_png_to(
            &mut BorrowedSource::new(&original)?,
            MarkedDocument {
                project: &project,
                document: &doc,
                assets: &NoAssets,
                options: options()
            },
            &req,
            limit(1),
            &mut output,
            &|| false
        ),
        Err(RasterError::Memory { .. })
    ));
    assert!(output.is_empty());
    Ok(())
}

fn chunk(bytes: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(data);
    let mut crc = 0xffff_ffffu32;
    for b in kind.iter().chain(data) {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    bytes.extend_from_slice(&(!crc).to_be_bytes());
}
fn adam7_png() -> (Vec<u8>, Vec<u8>) {
    let (w, h) = (9u32, 11u32);
    let pixels = (0..w * h)
        .flat_map(|n| {
            [
                n as u8,
                (n * 2) as u8,
                (n * 3) as u8,
                if n % 3 == 0 { 0 } else { 255 },
            ]
        })
        .collect::<Vec<_>>();
    let mut raw = Vec::new();
    for (x0, y0, dx, dy) in [
        (0, 0, 8, 8),
        (4, 0, 8, 8),
        (0, 4, 4, 8),
        (2, 0, 4, 4),
        (0, 2, 2, 4),
        (1, 0, 2, 2),
        (0, 1, 1, 2),
    ] {
        for y in (y0..h).step_by(dy) {
            raw.push(0);
            for x in (x0..w).step_by(dx) {
                let at = (y * w + x) as usize * 4;
                raw.extend_from_slice(&pixels[at..at + 4]);
            }
        }
    }
    // One stored DEFLATE block and Adler32: an independent procedural fixture,
    // not an encoder using the decoder's interlace expansion implementation.
    let mut compressed = vec![0x78, 0x01, 1];
    let len = raw.len() as u16;
    compressed.extend_from_slice(&len.to_le_bytes());
    compressed.extend_from_slice(&(!len).to_le_bytes());
    compressed.extend_from_slice(&raw);
    let (mut a, mut b) = (1u32, 0u32);
    for v in &raw {
        a = (a + u32::from(*v)) % 65521;
        b = (b + a) % 65521;
    }
    compressed.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut png = vec![137, 80, 78, 71, 13, 10, 26, 10];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 1]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &compressed);
    chunk(&mut png, b"IEND", &[]);
    (png, pixels)
}
#[test]
fn adam7_pass_rows_are_merged_without_losing_hidden_rgb() -> TestResult {
    let (bytes, expected) = adam7_png();
    let mut scratch = Scratch::new()?;
    let mut spool = PngSpool::decode(&bytes, scratch.file()?, SpoolLimits::default(), &|| false)?;
    let actual = spool.read_region(
        Region {
            x: 0,
            y: 0,
            width: 9,
            height: 11,
        },
        64 * 1024 * 1024,
        &|| false,
    )?;
    assert_eq!(actual.pixels, Pixels::Rgba8(expected));
    assert_eq!(
        actual.pixels,
        decode(&bytes, DecodeLimits::default())?.pixels
    );
    Ok(())
}

#[test]
fn palette_transparency_and_srgb_chunk_normalize_deterministically() -> TestResult {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 4, 2);
        encoder.set_color(png::ColorType::Indexed);
        encoder.set_depth(png::BitDepth::Two);
        encoder.set_palette(vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120]);
        encoder.set_trns(vec![0, 100, 200, 255]);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&[0b00011011, 0b11100100])?;
        writer.finish()?;
    }
    let expected = decode(&bytes, DecodeLimits::default())?;
    let mut scratch = Scratch::new()?;
    let mut spool = PngSpool::decode(&bytes, scratch.file()?, SpoolLimits::default(), &|| false)?;
    let actual = spool.read_region(
        Region {
            x: 0,
            y: 0,
            width: 4,
            height: 2,
        },
        64 * 1024 * 1024,
        &|| false,
    )?;
    assert_eq!(actual.pixels, expected.pixels);
    assert_eq!(actual.icc, expected.icc);
    assert_eq!(
        &actual.icc.ok_or("ICC")?[24..36],
        &[0x07, 0xd0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0]
    );
    Ok(())
}

// Declarative 50MP source: no hidden full-buffer allocation. read_region creates
// only exactly the requested rows, allowing planning and optional full-size proof.
struct FlatSource {
    info: SourceInfo,
    retained: u64,
}
impl FlatSource {
    fn new(depth: u8, retained: u64) -> Self {
        Self {
            info: SourceInfo {
                width: 10_000,
                height: 5_000,
                bit_depth: depth,
                icc: None,
                source_asset: vw_model::AssetId::hash(b"procedural flat 50MP"),
                original_available: true,
                orientation_applied: 1,
            },
            retained,
        }
    }
}
impl RasterSource for FlatSource {
    fn info(&self) -> &SourceInfo {
        &self.info
    }
    fn resident_bytes(&self) -> u64 {
        self.retained
    }
    fn read_workspace_bytes(&self) -> u64 {
        0
    }
    fn read_region(
        &mut self,
        r: Region,
        _: u64,
        _: &dyn Fn() -> bool,
    ) -> Result<DecodedImage, RasterError> {
        r.validate(self.info.width, self.info.height)?;
        let n = r.width as usize * r.height as usize * 4;
        Ok(DecodedImage {
            width: r.width,
            height: r.height,
            pixels: if self.info.bit_depth == 16 {
                Pixels::Rgba16(vec![32767; n])
            } else {
                Pixels::Rgba8(vec![127; n])
            },
            icc: None,
            source_asset: self.info.source_asset.clone(),
            original_available: true,
            orientation_applied: 1,
        })
    }
}
#[test]
fn fifty_megapixel_source_and_encoder_memory_are_accounted_before_io() -> TestResult {
    let mut req = request(ExportFormat::Png8);
    req.memory_budget_bytes = 256 * 1024 * 1024;
    let source = FlatSource::new(8, 200_000_000);
    let plan = plan_png(&source, &req, limit(128))?;
    assert_eq!((plan.region.width, plan.region.height), (10_000, 5_000));
    assert!(plan.estimated_peak_bytes > 200_000_000);
    assert!(plan.estimated_peak_bytes <= req.memory_budget_bytes);
    assert!(plan.strip_rows < 128);
    let source16 = FlatSource::new(16, 400_000_000);
    req.format = ExportFormat::Png16;
    assert!(matches!(
        plan_png(&source16, &req, limit(128)),
        Err(RasterError::Memory { .. })
    ));
    assert!(
        plan_png(&FlatSource::new(16, 0), &req, limit(128))?.estimated_peak_bytes
            <= req.memory_budget_bytes
    );
    Ok(())
}

#[test]
#[ignore = "explicit large-image software run: writes and removes 400MB caller-owned scratch; parent serializes"]
fn fifty_megapixel_png16_spool_marked_export_with_256mib_budget() -> TestResult {
    let mut req = request(ExportFormat::Png16);
    req.memory_budget_bytes = 256 * 1024 * 1024;
    let mut original = Vec::new();
    export_png_to(
        &mut FlatSource::new(16, 0),
        &req,
        limit(64),
        &mut original,
        &|| false,
    )?;
    let mut scratch = Scratch::new()?;
    let mut spool = PngSpool::decode(
        &original,
        scratch.file()?,
        SpoolLimits {
            decode: DecodeLimits {
                max_memory_bytes: req.memory_budget_bytes,
                ..DecodeLimits::default()
            },
            ..SpoolLimits::default()
        },
        &|| false,
    )?;
    let sample = spool.read_region(
        Region {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        },
        req.memory_budget_bytes,
        &|| false,
    )?;
    let (mut project, doc, layer) = project(&sample)?;
    let binding = project
        .assets
        .get_mut(&sample.source_asset)
        .ok_or("asset")?;
    binding.width = 10_000;
    binding.height = 5_000;
    binding.byte_size = original.len() as u64;
    let (oid, object) = object(
        &doc,
        &layer,
        8,
        vw_proto::v1::object_state::Shape::Rect(rect(125.0, 125.0, 9500.0, 4500.0)),
    )?;
    project.objects.insert(oid, object);
    let mut output = Scratch::new()?;
    let report = export_document_png_to(
        &mut spool,
        MarkedDocument {
            project: &project,
            document: &doc,
            assets: &NoAssets,
            options: RenderOptions {
                memory_budget_bytes: req.memory_budget_bytes,
                ..options()
            },
        },
        &req,
        limit(64),
        output.file()?,
        &|| false,
    )?;
    assert!(report.plan.estimated_peak_bytes <= req.memory_budget_bytes);
    assert_eq!(
        (report.metadata.output_width, report.metadata.output_height),
        (10_000, 5_000)
    );
    drop(spool);
    drop(scratch);
    drop(original);
    // The encoded output is a file sink. Bound its verified small fixture size
    // before reading it for a separate verification phase.
    assert!(output.file()?.metadata()?.len() < 16 * 1024 * 1024);
    output.file()?.seek(SeekFrom::Start(0))?;
    let mut encoded = Vec::new();
    output.file()?.read_to_end(&mut encoded)?;
    let mut verify_file = Scratch::new()?;
    let mut verify = PngSpool::decode(
        &encoded,
        verify_file.file()?,
        SpoolLimits {
            decode: DecodeLimits {
                max_memory_bytes: req.memory_budget_bytes,
                ..DecodeLimits::default()
            },
            ..SpoolLimits::default()
        },
        &|| false,
    )?;
    assert_eq!(
        verify
            .read_region(
                Region {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 2
                },
                req.memory_budget_bytes,
                &|| false
            )?
            .pixels,
        sample.pixels
    );
    assert_ne!(
        verify
            .read_region(
                Region {
                    x: 500,
                    y: 500,
                    width: 2,
                    height: 2
                },
                req.memory_budget_bytes,
                &|| false
            )?
            .pixels,
        sample.pixels
    );
    Ok(())
}
