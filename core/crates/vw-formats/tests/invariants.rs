#![allow(clippy::unwrap_used, clippy::panic)]

use std::{cell::Cell, io::Cursor};
use vw_formats::{Error, ExportFormat, Limits, decode, encode_to};
use vw_model::AssetId;
use vw_raster::{DecodedImage, Pixels};

fn source(pixels: Pixels) -> DecodedImage {
    DecodedImage {
        width: 1,
        height: 1,
        pixels,
        icc: None,
        source_asset: AssetId::hash(b"immutable-original"),
        original_available: true,
        orientation_applied: 1,
    }
}

#[test]
fn preview_without_original_never_writes_an_export() {
    for format in [
        ExportFormat::Bmp8,
        ExportFormat::Tiff8,
        ExportFormat::Tiff16,
    ] {
        let pixels = if format == ExportFormat::Tiff16 {
            Pixels::Rgba16(vec![256, 257, 258, 0])
        } else {
            Pixels::Rgba8(vec![1, 2, 3, 0])
        };
        let mut image = source(pixels);
        image.original_available = false;
        let mut output = Cursor::new(Vec::new());
        assert!(matches!(
            encode_to(&image, format, Limits::default(), &mut output, &|| false),
            Err(Error::Unsupported)
        ));
        assert!(output.get_ref().is_empty());
    }
}

#[test]
fn encoder_charges_retained_capacity_before_writing_or_parsing_profiles() {
    for format in [
        ExportFormat::Bmp8,
        ExportFormat::Tiff8,
        ExportFormat::Tiff16,
    ] {
        let pixels = if format == ExportFormat::Tiff16 {
            let mut values = Vec::with_capacity(2 * 1024 * 1024);
            values.extend([256, 257, 258, 0]);
            Pixels::Rgba16(values)
        } else {
            let mut values = Vec::with_capacity(4 * 1024 * 1024);
            values.extend([1, 2, 3, 0]);
            Pixels::Rgba8(values)
        };
        let image = source(pixels);
        let limits = Limits {
            memory_bytes: 65 * 1024 * 1024,
            ..Limits::default()
        };
        let mut output = Cursor::new(Vec::new());
        assert!(matches!(
            encode_to(&image, format, limits, &mut output, &|| false),
            Err(Error::Limit)
        ));
        assert!(output.get_ref().is_empty());
    }
}

// Independent classic-TIFF container and compressors; no vw-formats encoder.
// Each codec must preserve channel values and immutable encoded identity.
fn rgb_tiff(compression: u16, pixels: &[u8]) -> Vec<u8> {
    let compressed = match compression {
        1 => pixels.to_vec(),
        5 => weezl::encode::Encoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8)
            .encode(pixels)
            .unwrap(),
        8 | 32946 => {
            let size = pixels.len() as u16;
            let mut bytes = vec![0x78, 0x01, 0x01]; // zlib, final stored DEFLATE block
            bytes.extend(size.to_le_bytes());
            bytes.extend((!size).to_le_bytes());
            bytes.extend(pixels);
            let (mut a, mut b) = (1_u32, 0_u32);
            for &value in pixels {
                a = (a + u32::from(value)) % 65521;
                b = (b + a) % 65521;
            }
            bytes.extend(((b << 16) | a).to_be_bytes());
            bytes
        }
        32773 => {
            let mut bytes = vec![(pixels.len() - 1) as u8];
            bytes.extend(pixels);
            bytes
        }
        _ => panic!("fixture compression"),
    };
    let entries = [
        (256_u16, 4_u16, 1_u32, 3_u32),
        (257, 4, 1, 2),
        (258, 3, 3, 134),
        (259, 3, 1, u32::from(compression)),
        (262, 3, 1, 2),
        (273, 4, 1, 140),
        (277, 3, 1, 3),
        (278, 4, 1, 2),
        (279, 4, 1, compressed.len() as u32),
        (284, 3, 1, 1),
    ];
    let mut bytes = b"II*\0\x08\0\0\0".to_vec();
    bytes.extend((entries.len() as u16).to_le_bytes());
    for (tag, kind, count, value) in entries {
        bytes.extend(tag.to_le_bytes());
        bytes.extend(kind.to_le_bytes());
        bytes.extend(count.to_le_bytes());
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(0_u32.to_le_bytes());
    for _ in 0..3 {
        bytes.extend(8_u16.to_le_bytes());
    }
    bytes.extend(compressed);
    bytes
}

#[test]
fn independently_compressed_tiff_strips_keep_exact_samples_and_source_hash() {
    let rgb = [
        10, 20, 30, 11, 21, 31, 12, 22, 32, 40, 50, 60, 41, 51, 61, 42, 52, 62,
    ];
    let rgba: Vec<_> = rgb
        .chunks_exact(3)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect();
    for compression in [1, 5, 8, 32946, 32773] {
        let bytes = rgb_tiff(compression, &rgb);
        let imported = decode(&bytes, Limits::default(), &|| false).unwrap();
        assert_eq!(
            imported.image.pixels,
            Pixels::Rgba8(rgba.clone()),
            "compression {compression}"
        );
        assert_eq!(imported.image.source_asset, AssetId::hash(&bytes));
        assert!(imported.image.original_available);
    }
}

#[test]
fn tiff16_original_and_r164_display_derivative_keep_separate_depth_and_identity() {
    let mut image = source(Pixels::Rgba16(vec![256, 257, 258, 32768]));
    image.icc = Some(vw_raster::standard_srgb_profile().unwrap());
    let mut output = Cursor::new(Vec::new());
    encode_to(
        &image,
        ExportFormat::Tiff16,
        Limits::default(),
        &mut output,
        &|| false,
    )
    .unwrap();
    let original = output.into_inner();
    let imported = decode(&original, Limits::default(), &|| false).unwrap();
    assert_eq!(imported.image.pixels, image.pixels);
    let display =
        vw_raster::display_rgba8(imported.image, 256 * 1024 * 1024, false, &|| false).unwrap();
    assert_eq!(display.source_asset, AssetId::hash(&original));
    assert_eq!(display.source_bit_depth, 16);
    assert_eq!(display.rgba[3], 128);
    assert_eq!(
        display.icc_profile,
        vw_raster::standard_srgb_profile().unwrap()
    );
    // Display conversion consumes a decode, never the preserved original.
    assert_eq!(
        decode(&original, Limits::default(), &|| false)
            .unwrap()
            .image
            .pixels,
        image.pixels
    );
}

#[test]
fn late_export_cancellation_returns_no_receipt_and_never_changes_source() {
    let image = source(Pixels::Rgba8(vec![1, 2, 3, 0]));
    let original = image.clone();
    let calls = Cell::new(0);
    let mut output = Cursor::new(Vec::new());
    let result = encode_to(
        &image,
        ExportFormat::Bmp8,
        Limits::default(),
        &mut output,
        &|| {
            calls.set(calls.get() + 1);
            calls.get() >= 4
        },
    );
    assert!(matches!(result, Err(Error::Cancelled)));
    assert!(!output.get_ref().is_empty()); // private staging may need caller cleanup
    assert_eq!(image, original);
}
