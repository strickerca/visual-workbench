mod support;
use support::*;
use vw_raster::*;

#[test]
fn png_8_and_16_roundtrip_preserve_every_channel_including_transparent_rgb() -> TestResult {
    for depth in [8, 16] {
        let source = source(17, 11, depth);
        let before = source.clone();
        let encoded = export(
            &source,
            &request(if depth == 8 {
                ExportFormat::Png8
            } else {
                ExportFormat::Png16
            }),
        )?;
        let decoded = decode(&encoded.bytes, DecodeLimits::default())?;
        assert_eq!(decoded.pixels, source.pixels);
        assert_eq!(source, before);
        assert_eq!((decoded.width, decoded.height), (17, 11));
    }
    Ok(())
}
#[test]
fn clean_region_is_exact_source_samples_without_scale() -> TestResult {
    let source = source(17, 11, 16);
    let region = Region {
        x: 3,
        y: 2,
        width: 8,
        height: 6,
    };
    let mut r = request(ExportFormat::Png16);
    r.region = Some(region);
    let result = export(&source, &r)?;
    assert_eq!(result.metadata.source_width, 17);
    assert_eq!(result.metadata.output_width, 8);
    assert_eq!(
        decode(&result.bytes, DecodeLimits::default())?.pixels,
        source.crop(region)?.pixels
    );
    Ok(())
}

#[test]
fn jpeg_alpha_preflight_uses_the_selected_region() -> TestResult {
    let mut source = source(2, 1, 8);
    source.pixels = Pixels::Rgba8(vec![80, 90, 100, 255, 110, 120, 130, 0]);
    let before = source.clone();
    let mut r = request(ExportFormat::Jpeg { quality: 100 });
    r.region = Some(Region {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    });
    let output = export(&source, &r)?;
    assert_eq!(
        (output.metadata.output_width, output.metadata.output_height),
        (1, 1)
    );
    r.region.as_mut().ok_or("region")?.x = 1;
    assert!(matches!(export(&source, &r), Err(RasterError::Alpha)));
    assert_eq!(source, before);
    Ok(())
}
#[test]
fn lossless_webp_roundtrip_alpha_and_rgb() -> TestResult {
    let source = source(19, 9, 8);
    let result = export(&source, &request(ExportFormat::WebpLossless))?;
    let decoded = decode(&result.bytes, DecodeLimits::default())?;
    assert_eq!(decoded.pixels, source.pixels);
    Ok(())
}
#[test]
fn jpeg_and_lossy_webp_decode_at_exact_requested_dimensions() -> TestResult {
    for format in [
        ExportFormat::Jpeg { quality: 91 },
        ExportFormat::WebpLossy { quality: 87 },
    ] {
        let source = source(19, 11, 8);
        let mut r = request(format);
        r.alpha = AlphaPolicy::Matte([255, 255, 255]);
        let result = export(&source, &r)?;
        let decoded = decode(&result.bytes, DecodeLimits::default())?;
        assert_eq!((decoded.width, decoded.height), (19, 11));
        assert!(!decoded.pixels.has_alpha());
    }
    Ok(())
}
#[test]
fn metadata_revision_hash_settings_and_capture_identity_in_every_format() -> TestResult {
    for format in [
        ExportFormat::Png8,
        ExportFormat::Png16,
        ExportFormat::Jpeg { quality: 81 },
        ExportFormat::WebpLossless,
        ExportFormat::WebpLossy { quality: 79 },
    ] {
        let source = source(4, 3, 8);
        let mut r = request(format);
        r.alpha = AlphaPolicy::Matte([0, 0, 0]);
        r.capture_session = Some(id(55)?);
        r.frame_id = Some(u64::MAX);
        let result = export(&source, &r)?;
        let text = if matches!(format, ExportFormat::Png8 | ExportFormat::Png16) {
            let reader = png::Decoder::new(std::io::Cursor::new(&result.bytes)).read_info()?;
            assert_eq!(reader.info().utf8_text.len(), 1);
            reader.info().utf8_text[0].get_text()?
        } else {
            String::from_utf8_lossy(&result.bytes).into_owned()
        };
        assert!(text.contains("r42-abababab"));
        assert!(text.contains(source.source_asset.as_str()));
        assert!(text.contains("18446744073709551615"));
        assert!(text.contains("orientation_applied"));
        assert!(text.contains("output_width"));
        assert!(text.contains("settings"));
    }
    Ok(())
}
#[test]
fn limits_preflight_before_allocations_and_never_silently_reduce() -> TestResult {
    let mut r = request(ExportFormat::WebpLossless);
    assert!(matches!(
        preflight(16384, 1, 8, false, &r),
        Err(RasterError::Dimensions { limit: 16383, .. })
    ));
    r.format = ExportFormat::Jpeg { quality: 90 };
    assert!(matches!(
        preflight(65536, 1, 8, false, &r),
        Err(RasterError::Dimensions { limit: 65535, .. })
    ));
    r.format = ExportFormat::Png16;
    r.memory_budget_bytes = 1;
    assert!(matches!(
        preflight(1, 1, 16, false, &r),
        Err(RasterError::Memory { .. })
    ));
    r = request(ExportFormat::Png8);
    assert!(matches!(
        preflight(1, 1, 16, false, &r),
        Err(RasterError::Depth)
    ));
    r.allow_depth_reduction = true;
    assert!(preflight(1, 1, 16, false, &r).is_ok());
    r.format = ExportFormat::Jpeg { quality: 0 };
    assert!(preflight(1, 1, 8, false, &r).is_err());
    r.format = ExportFormat::Jpeg { quality: 100 };
    assert!(matches!(
        preflight(1, 1, 8, true, &r),
        Err(RasterError::Alpha)
    ));
    r.region = Some(Region {
        x: u32::MAX,
        y: 0,
        width: 2,
        height: 1,
    });
    assert!(preflight(u32::MAX, 1, 8, false, &r).is_err());
    Ok(())
}
#[test]
fn preview_missing_revision_bad_buffer_and_bounded_decode_fail() -> TestResult {
    let mut source = source(8, 8, 8);
    source.original_available = false;
    assert!(matches!(
        export(&source, &request(ExportFormat::Png8)),
        Err(RasterError::OriginalRequired)
    ));
    source.original_available = true;
    let mut r = request(ExportFormat::Png8);
    r.revision.state_hash.clear();
    assert!(export(&source, &r).is_err());
    let bytes = export(&source, &request(ExportFormat::Png8))?.bytes;
    assert!(
        decode(
            &bytes,
            DecodeLimits {
                max_pixels: 1,
                ..DecodeLimits::default()
            }
        )
        .is_err()
    );
    assert!(
        decode(
            &bytes,
            DecodeLimits {
                max_encoded_bytes: 1,
                ..DecodeLimits::default()
            }
        )
        .is_err()
    );
    source.pixels = Pixels::Rgba8(vec![]);
    assert!(export(&source, &request(ExportFormat::Png8)).is_err());
    assert!(decode(b"not an image", DecodeLimits::default()).is_err());
    Ok(())
}
#[test]
fn clean_rgb_icc_is_preserved_in_png_jpeg_and_webp() -> TestResult {
    let mut source = source(13, 7, 8);
    source.icc = Some(moxcms::ColorProfile::new_display_p3().encode()?);
    for format in [
        ExportFormat::Png8,
        ExportFormat::Jpeg { quality: 90 },
        ExportFormat::WebpLossless,
        ExportFormat::WebpLossy { quality: 85 },
    ] {
        let mut r = request(format);
        r.alpha = AlphaPolicy::Matte([255, 255, 255]);
        let bytes = export(&source, &r)?.bytes;
        let decoded = decode(&bytes, DecodeLimits::default())?;
        assert_eq!(decoded.icc, source.icc);
    }
    Ok(())
}
#[test]
fn ai_color_conversion_requires_explicit_untagged_assumption_and_records_profile() -> TestResult {
    let mut source = source(8, 8, 16);
    let mut r = request(ExportFormat::Png16);
    r.color = ColorPolicy::ConvertToSrgb {
        assume_untagged_srgb: false,
    };
    assert!(export(&source, &r).is_err());
    source.icc = Some(moxcms::ColorProfile::new_display_p3().encode()?);
    let result = export(&source, &r)?;
    assert_eq!(result.metadata.color_conversion, "ICC to sRGB");
    assert_ne!(
        result.metadata.source_icc_hash,
        result.metadata.output_icc_hash
    );
    let decoded = decode(&result.bytes, DecodeLimits::default())?;
    assert_eq!(decoded.pixels.bit_depth(), 16);
    let (Pixels::Rgba16(before), Pixels::Rgba16(after)) = (&source.pixels, &decoded.pixels) else {
        return Err("unexpected depth".into());
    };
    for (a, b) in before
        .as_chunks::<4>()
        .0
        .iter()
        .zip(after.as_chunks::<4>().0.iter())
    {
        assert_eq!(a[3], b[3]);
    }
    Ok(())
}
#[test]
fn all_eight_exif_orientations_move_pixels_into_document_space_once() -> TestResult {
    let source = source(5, 3, 8);
    let mut r = request(ExportFormat::Jpeg { quality: 100 });
    r.alpha = AlphaPolicy::Matte([255, 255, 255]);
    let jpeg = export(&source, &r)?.bytes;
    let baseline = decode(&jpeg, DecodeLimits::default())?;
    let Pixels::Rgba8(pixels) = &baseline.pixels else {
        return Err("depth".into());
    };
    for orientation in 1u8..=8 {
        let mut exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
        exif.extend([orientation, 0, 0, 0, 0, 0, 0, 0]);
        let mut segment = vec![0xff, 0xe1];
        segment.extend(((exif.len() + 2) as u16).to_be_bytes());
        segment.extend(exif);
        let mut encoded = jpeg.clone();
        encoded.splice(2..2, segment);
        let decoded = decode(&encoded, DecodeLimits::default())?;
        let (w, h) = if orientation >= 5 { (3, 5) } else { (5, 3) };
        assert_eq!((decoded.width, decoded.height), (w, h));
        assert_eq!(decoded.orientation_applied, orientation);
        let mut expected = vec![0; pixels.len()];
        for y in 0..3 {
            for x in 0..5 {
                let (dx, dy) = match orientation {
                    1 => (x, y),
                    2 => (4 - x, y),
                    3 => (4 - x, 2 - y),
                    4 => (x, 2 - y),
                    5 => (y, x),
                    6 => (2 - y, x),
                    7 => (2 - y, 4 - x),
                    _ => (y, 4 - x),
                };
                let src = ((y * 5 + x) * 4) as usize;
                let dst = ((dy * w + dx) * 4) as usize;
                expected[dst..dst + 4].copy_from_slice(&pixels[src..src + 4]);
            }
        }
        assert_eq!(decoded.pixels, Pixels::Rgba8(expected));
        let reencoded = export(&decoded, &request(ExportFormat::Png8))?;
        let again = decode(&reencoded.bytes, DecodeLimits::default())?;
        assert_eq!(again.orientation_applied, 1);
        assert_eq!(again.pixels, decoded.pixels);
    }
    Ok(())
}

#[test]
fn grayscale_profile_16bit_png_roundtrip_and_explicit_srgb_conversion() -> TestResult {
    let mut source = source(9, 7, 16);
    if let Pixels::Rgba16(p) = &mut source.pixels {
        for p in p.as_chunks_mut::<4>().0.iter_mut() {
            p[1] = p[0];
            p[2] = p[0];
        }
    }
    source.icc = Some(moxcms::ColorProfile::new_gray_with_gamma(2.2).encode()?);
    let output = export(&source, &request(ExportFormat::Png16))?;
    let decoded = decode(&output.bytes, DecodeLimits::default())?;
    assert_eq!(decoded.pixels, source.pixels);
    assert_eq!(decoded.icc, source.icc);
    let mut r = request(ExportFormat::WebpLossless);
    r.allow_depth_reduction = true;
    assert!(export(&source, &r).is_err());
    r.color = ColorPolicy::ConvertToSrgb {
        assume_untagged_srgb: false,
    };
    let converted = export(&source, &r)?;
    assert_eq!(converted.metadata.color_conversion, "ICC to sRGB");
    assert!(decode(&converted.bytes, DecodeLimits::default()).is_ok());
    Ok(())
}
#[test]
fn native_lossy_encoder_version_is_the_reviewed_bundled_release() -> TestResult {
    assert_eq!(webp_encoder_version(), (1, 6, 0));
    Ok(())
}

#[test]
fn png_explicit_color_metadata_is_preserved_or_refused() -> TestResult {
    fn fixture(
        srgb: bool,
        with_icc: bool,
        hdr: bool,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut bytes = Vec::new();
        let mut info = png::Info::with_size(1, 1);
        info.color_type = png::ColorType::Rgba;
        info.bit_depth = png::BitDepth::Eight;
        if with_icc {
            info.icc_profile = Some(moxcms::ColorProfile::new_display_p3().encode()?.into());
        }
        let mut encoder = png::Encoder::with_info(&mut bytes, info)?;
        encoder.set_source_gamma(png::ScaledFloat::new(1.0));
        if srgb {
            encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        }
        let mut writer = encoder.write_header()?;
        if hdr {
            writer.write_chunk(png::chunk::ChunkType(*b"cICP"), &[9, 16, 0, 1])?;
        }
        writer.write_image_data(&[64, 96, 128, 173])?;
        drop(writer);
        Ok(bytes)
    }
    assert!(matches!(
        decode(&fixture(false, false, false)?, DecodeLimits::default()),
        Err(RasterError::Unsupported(
            "PNG gamma/chromaticities without ICC or sRGB; convert with a color-aware decoder"
        ))
    ));
    assert!(matches!(
        decode(&fixture(false, true, true)?, DecodeLimits::default()),
        Err(RasterError::Unsupported(
            "PNG cICP/HDR metadata; convert with a supported color decoder"
        ))
    ));
    let declared_srgb = decode(&fixture(true, false, false)?, DecodeLimits::default())?;
    let mut expected_profile = moxcms::ColorProfile::new_srgb().encode()?;
    // Standard generated profiles use a fixed 2000-01-01 header. The upstream
    // constructor writes the wall clock; every other profile byte stays equal.
    expected_profile[24..36].copy_from_slice(&[7, 208, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0]);
    assert_eq!(declared_srgb.icc, Some(expected_profile));
    assert_eq!(declared_srgb.pixels, Pixels::Rgba8(vec![64, 96, 128, 173]));
    let exported = export(&declared_srgb, &request(ExportFormat::Png8))?;
    let roundtrip = decode(&exported.bytes, DecodeLimits::default())?;
    assert_eq!(roundtrip.pixels, declared_srgb.pixels);
    assert_eq!(roundtrip.icc, declared_srgb.icc);
    let encoded_icc = fixture(false, true, false)?;
    let expected_icc = png::Decoder::new(std::io::Cursor::new(&encoded_icc))
        .read_info()?
        .info()
        .icc_profile
        .clone()
        .map(|p| p.into_owned());
    let declared_icc = decode(&encoded_icc, DecodeLimits::default())?;
    assert_eq!(declared_icc.icc, expected_icc);
    Ok(())
}
#[test]
fn hostile_headers_truncation_and_mutations_respect_decode_limits() -> TestResult {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut c = 0xffffffffu32;
        for &b in bytes {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = (c >> 1) ^ if c & 1 != 0 { 0xedb88320 } else { 0 };
            }
        }
        !c
    }
    let good = export(&source(8, 8, 8), &request(ExportFormat::Png8))?.bytes;
    let mut oversized = good.clone();
    oversized[16..20].copy_from_slice(&50_000_001u32.to_be_bytes());
    oversized[20..24].copy_from_slice(&1u32.to_be_bytes());
    let crc = crc32(&oversized[12..29]);
    oversized[29..33].copy_from_slice(&crc.to_be_bytes());
    assert!(decode(&oversized, DecodeLimits::default()).is_err());
    for bytes in [
        b"RIFF\xff\xff\xff\xffWEBPVP8L\xff\xff\xff\xff".as_slice(),
        b"\xff\xd8\xff\xe1\xff\xff".as_slice(),
        &good[..33],
    ] {
        assert!(decode(bytes, DecodeLimits::default()).is_err());
    }
    let limits = DecodeLimits {
        max_encoded_bytes: 1024 * 1024,
        max_pixels: 4096,
        max_memory_bytes: 16 * 1024 * 1024,
    };
    for index in 0..64 {
        let mut corrupt = good.clone();
        let at = (index * 997 + 3) % corrupt.len();
        corrupt[at] ^= 0xa5;
        if let Ok(image) = decode(&corrupt, limits) {
            image.validate()?;
            assert!(u64::from(image.width) * u64::from(image.height) <= 4096);
        }
    }
    Ok(())
}
