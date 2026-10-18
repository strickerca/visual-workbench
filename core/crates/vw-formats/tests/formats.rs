#![allow(clippy::unwrap_used, clippy::panic)]

use std::cell::Cell;
use std::io::Cursor;
use vw_formats::{Error, ExportFormat, Limits, Notice, decode, encode_to, inspect};
use vw_model::AssetId;
use vw_raster::{DecodedImage, Pixels};

fn source(pixels: Pixels, width: u32, height: u32) -> DecodedImage {
    DecodedImage {
        width,
        height,
        pixels,
        icc: None,
        source_asset: AssetId::hash(b"immutable original"),
        original_available: true,
        orientation_applied: 1,
    }
}
fn encoded(image: &DecodedImage, format: ExportFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    let receipt = encode_to(image, format, Limits::default(), &mut out, &|| false).unwrap();
    assert_eq!(receipt.bytes, out.get_ref().len() as u64);
    assert_eq!(receipt.source_asset, image.source_asset);
    out.into_inner()
}
fn short_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}
fn tag(bytes: &[u8], key: u16) -> usize {
    (0..usize::from(short_at(bytes, 8)))
        .map(|i| 10 + i * 12)
        .find(|&at| short_at(bytes, at) == key)
        .unwrap()
}
fn scalar(bytes: &mut [u8], key: u16, value: u32) {
    let at = tag(bytes, key);
    bytes[at + 8..at + 12].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn bmp_v5_retains_hidden_rgb_and_straight_alpha_and_icc() {
    let mut image = source(Pixels::Rgba8(vec![254, 1, 67, 0, 18, 233, 99, 7]), 2, 1);
    image.icc = Some(vw_raster::standard_srgb_profile().unwrap());
    let bytes = encoded(&image, ExportFormat::Bmp8);
    let result = decode(&bytes, Limits::default(), &|| false).unwrap();
    assert_eq!(result.image.pixels, image.pixels);
    assert_eq!(result.image.icc, image.icc);
    assert_eq!(result.image.source_asset, AssetId::hash(&bytes));
    assert_eq!(result.header.encoded_bytes, bytes.len() as u64);
}

#[test]
fn tiff_16_keeps_adjacent_samples_and_zero_alpha_color_exact() {
    let mut image = source(
        Pixels::Rgba16(vec![256, 257, 258, 0, 65534, 65535, 3, 32769]),
        2,
        1,
    );
    image.icc = Some(vw_raster::standard_srgb_profile().unwrap());
    let bytes = encoded(&image, ExportFormat::Tiff16);
    let imported = decode(&bytes, Limits::default(), &|| false).unwrap();
    assert_eq!(imported.image.pixels, image.pixels);
    assert_eq!(imported.image.icc, image.icc);
    assert_eq!(imported.header.bit_depth, 16);
    assert_eq!(imported.header.orientation, 1);
}

#[test]
fn tiff_multiple_strips_have_exact_rows_including_last_short_strip() {
    let pixels = (0..3 * 35)
        .flat_map(|i| [i as u8, (i * 13) as u8, 255, (i * 7) as u8])
        .collect();
    let image = source(Pixels::Rgba8(pixels), 3, 35);
    let bytes = encoded(&image, ExportFormat::Tiff8);
    assert_eq!(
        decode(&bytes, Limits::default(), &|| false)
            .unwrap()
            .image
            .pixels,
        image.pixels
    );
}

#[test]
fn tiff_orientation_is_exact_permutation_and_retained_provenance() {
    let image = source(
        Pixels::Rgba8((1..=6).flat_map(|v| [v, v, v, 255]).collect()),
        3,
        2,
    );
    let expected = [
        vec![1, 2, 3, 4, 5, 6],
        vec![3, 2, 1, 6, 5, 4],
        vec![6, 5, 4, 3, 2, 1],
        vec![4, 5, 6, 1, 2, 3],
        vec![1, 4, 2, 5, 3, 6],
        vec![4, 1, 5, 2, 6, 3],
        vec![6, 3, 5, 2, 4, 1],
        vec![3, 6, 2, 5, 1, 4],
    ];
    for orientation in 1..=8 {
        let mut bytes = encoded(&image, ExportFormat::Tiff8);
        scalar(&mut bytes, 274, orientation);
        let imported = decode(&bytes, Limits::default(), &|| false).unwrap();
        assert_eq!(imported.image.orientation_applied, orientation as u8);
        assert_eq!(
            (imported.image.width, imported.image.height),
            if orientation < 5 { (3, 2) } else { (2, 3) }
        );
        let Pixels::Rgba8(p) = imported.image.pixels else {
            panic!("depth changed")
        };
        assert_eq!(
            p.as_chunks::<4>()
                .0
                .iter()
                .map(|p| p[0])
                .collect::<Vec<_>>(),
            expected[orientation as usize - 1]
        );
    }
}

#[test]
fn multipage_tiff_warns_and_does_not_visit_an_unrequested_ifd() {
    let image = source(Pixels::Rgba8(vec![9, 8, 7, 255]), 1, 1);
    let mut bytes = encoded(&image, ExportFormat::Tiff8);
    let next_at = 10 + usize::from(short_at(&bytes, 8)) * 12;
    let second = bytes.len() as u32;
    bytes[next_at..next_at + 4].copy_from_slice(&second.to_le_bytes());
    // A later page's incomplete IFD is deliberately not decoded or allocated.
    bytes.extend_from_slice(&65535_u16.to_le_bytes());
    let result = decode(&bytes, Limits::default(), &|| false).unwrap();
    assert!(result.header.notices.contains(&Notice::FirstTiffPageOnly));
    assert_eq!(result.image.pixels, image.pixels);
}

// Independent minimal big-endian, planar RGB fixture: three two-byte strips.
// This catches using the pinned decoder's deprecated first-plane-only reader.
fn planar_rgb() -> Vec<u8> {
    let entries: [(u16, u16, u32, u32); 10] = [
        (256, 4, 1, 2),
        (257, 4, 1, 1),
        (258, 3, 3, 134),
        (259, 3, 1, 1 << 16),
        (262, 3, 1, 2 << 16),
        (273, 4, 3, 140),
        (277, 3, 1, 3 << 16),
        (278, 4, 1, 1),
        (279, 4, 3, 152),
        (284, 3, 1, 2 << 16),
    ];
    let mut bytes = b"MM\0*\0\0\0\x08".to_vec();
    bytes.extend_from_slice(&10_u16.to_be_bytes());
    for (key, kind, count, value) in entries {
        bytes.extend_from_slice(&key.to_be_bytes());
        bytes.extend_from_slice(&kind.to_be_bytes());
        bytes.extend_from_slice(&count.to_be_bytes());
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.extend_from_slice(&0_u32.to_be_bytes());
    for _ in 0..3 {
        bytes.extend_from_slice(&8_u16.to_be_bytes());
    }
    for offset in [164_u32, 166, 168] {
        bytes.extend_from_slice(&offset.to_be_bytes());
    }
    for _ in 0..3 {
        bytes.extend_from_slice(&2_u32.to_be_bytes());
    }
    bytes.extend_from_slice(&[10, 11, 20, 21, 30, 31]);
    bytes
}

#[test]
fn big_endian_planar_rgb_reads_all_planes() {
    let imported = decode(&planar_rgb(), Limits::default(), &|| false).unwrap();
    assert_eq!(
        imported.image.pixels,
        Pixels::Rgba8(vec![10, 20, 30, 255, 11, 21, 31, 255])
    );
}

#[test]
fn duplicate_tiff_tags_and_alias_amplification_fail_in_header_admission() {
    let image = source(Pixels::Rgba8(vec![1, 2, 3, 255]), 1, 1);
    let mut bytes = encoded(&image, ExportFormat::Tiff8);
    let at = tag(&bytes, 257);
    bytes[at..at + 2].copy_from_slice(&256_u16.to_le_bytes());
    assert!(matches!(
        inspect(&bytes, Limits::default()),
        Err(Error::Invalid)
    ));
    let mut bytes = encoded(&image, ExportFormat::Tiff8);
    let at = tag(&bytes, 258);
    bytes[at + 4..at + 8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        inspect(&bytes, Limits::default()),
        Err(Error::Limit)
    ));
}

#[test]
fn tiff_budget_and_pixel_limit_refuse_before_codec_allocation() {
    let image = source(Pixels::Rgba8(vec![1, 2, 3, 255]), 1, 1);
    let mut bytes = encoded(&image, ExportFormat::Tiff8);
    scalar(&mut bytes, 256, 4096);
    scalar(&mut bytes, 257, 4096);
    let low = Limits {
        memory_bytes: 128 * 1024 * 1024,
        ..Limits::default()
    };
    assert!(matches!(inspect(&bytes, low), Err(Error::Limit)));
    assert!(matches!(decode(&bytes, low, &|| false), Err(Error::Limit)));
    scalar(&mut bytes, 256, 65535);
    scalar(&mut bytes, 257, 65535);
    assert!(matches!(
        inspect(&bytes, Limits::default()),
        Err(Error::Limit)
    ));
}

#[test]
fn associated_alpha_and_float_tiff_refuse_implicit_conversion() {
    let image = source(Pixels::Rgba8(vec![1, 2, 3, 4]), 1, 1);
    let mut bytes = encoded(&image, ExportFormat::Tiff8);
    scalar(&mut bytes, 338, 1);
    assert!(matches!(
        inspect(&bytes, Limits::default()),
        Err(Error::Unsupported)
    ));
    let mut bytes = encoded(&image, ExportFormat::Tiff8);
    let at = tag(&bytes, 339);
    let values = u32::from_le_bytes(bytes[at + 8..at + 12].try_into().unwrap()) as usize;
    for component in 0..4 {
        bytes[values + component * 2..values + component * 2 + 2]
            .copy_from_slice(&3_u16.to_le_bytes());
    }
    assert!(matches!(
        inspect(&bytes, Limits::default()),
        Err(Error::Depth)
    ));
    let sixteen = source(Pixels::Rgba16(vec![1, 2, 3, 4]), 1, 1);
    let mut untouched = Cursor::new(Vec::new());
    assert!(matches!(
        encode_to(
            &sixteen,
            ExportFormat::Bmp8,
            Limits::default(),
            &mut untouched,
            &|| false
        ),
        Err(Error::Depth)
    ));
    assert!(untouched.get_ref().is_empty());
}

// Independent BigTIFF fixture, not generated by our classic-TIFF encoder.
// Gray + straight alpha is represented by the pinned decoder as Multiband(2).
fn big_tiff_gray_alpha() -> Vec<u8> {
    let entries: [(u16, u16, u64, u64); 12] = [
        (256, 4, 1, 2),
        (257, 4, 1, 1),
        (258, 3, 2, 16 | (16 << 16)),
        (259, 3, 1, 1),
        (262, 3, 1, 1),
        (273, 16, 1, 272),
        (277, 3, 1, 2),
        (278, 4, 1, 1),
        (279, 16, 1, 8),
        (284, 3, 1, 1),
        (338, 3, 1, 2),
        (339, 3, 2, 1 | (1 << 16)),
    ];
    let mut bytes = b"II+\0\x08\0\0\0\x10\0\0\0\0\0\0\0".to_vec();
    bytes.extend_from_slice(&12_u64.to_le_bytes());
    for (key, kind, count, value) in entries {
        bytes.extend_from_slice(&key.to_le_bytes());
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    for value in [256_u16, 0, 257, 32769] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[test]
fn big_tiff_gray_alpha_preserves_sixteen_bit_values() {
    let imported = decode(&big_tiff_gray_alpha(), Limits::default(), &|| false).unwrap();
    assert_eq!(
        imported.image.pixels,
        Pixels::Rgba16(vec![256, 256, 256, 0, 257, 257, 257, 32769])
    );
    assert_eq!(imported.header.bit_depth, 16);
    assert!(imported.header.alpha);
    assert!(imported.header.notices.contains(&Notice::UntaggedColor));
}

#[test]
fn white_is_zero_alpha_is_refused_before_pinned_decoder_can_invert_alpha() {
    let mut bytes = big_tiff_gray_alpha();
    let photo_value = 24 + 4 * 20 + 12;
    bytes[photo_value..photo_value + 8].copy_from_slice(&0_u64.to_le_bytes());
    assert!(matches!(
        inspect(&bytes, Limits::default()),
        Err(Error::Unsupported)
    ));
}

fn jpeg_gray_tiff(jpeg_width: u16, jpeg_height: u16) -> Vec<u8> {
    fn segment(out: &mut Vec<u8>, marker: u8, payload: &[u8]) {
        out.extend_from_slice(&[0xff, marker]);
        out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(payload);
    }
    // Baseline 1x1 gray with a zero DC difference and EOB: decoded value 128.
    let mut jpeg = vec![0xff, 0xd8];
    let mut table = vec![0];
    table.extend_from_slice(&[1; 64]);
    segment(&mut jpeg, 0xdb, &table);
    let mut sof = vec![8];
    sof.extend_from_slice(&jpeg_height.to_be_bytes());
    sof.extend_from_slice(&jpeg_width.to_be_bytes());
    sof.extend_from_slice(&[1, 1, 0x11, 0]);
    segment(&mut jpeg, 0xc0, &sof);
    for class in [0, 0x10] {
        let mut huffman = vec![class, 1];
        huffman.extend_from_slice(&[0; 15]);
        huffman.push(0);
        segment(&mut jpeg, 0xc4, &huffman);
    }
    segment(&mut jpeg, 0xda, &[1, 1, 0, 0, 63, 0]);
    jpeg.extend_from_slice(&[0x3f, 0xff, 0xd9]);
    let entries = [
        (256_u16, 4_u16, 1_u32),
        (257, 4, 1),
        (258, 3, 8),
        (259, 3, 7),
        (262, 3, 1),
        (273, 4, 122),
        (277, 3, 1),
        (278, 4, 1),
        (279, 4, jpeg.len() as u32),
    ];
    let mut bytes = b"II*\0\x08\0\0\0".to_vec();
    bytes.extend_from_slice(&9_u16.to_le_bytes());
    for (tag, kind, value) in entries {
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&jpeg);
    bytes
}

#[test]
fn embedded_jpeg_extent_is_checked_before_unbounded_nested_decoder() {
    let bytes = jpeg_gray_tiff(8192, 8192);
    assert!(matches!(
        inspect(&bytes, Limits::default()),
        Err(Error::Invalid)
    ));
    assert!(matches!(
        decode(&bytes, Limits::default(), &|| false),
        Err(Error::Invalid)
    ));
}

#[test]
fn admitted_baseline_gray_jpeg_strip_decodes_without_color_conversion() {
    let bytes = jpeg_gray_tiff(1, 1);
    let imported = decode(&bytes, Limits::default(), &|| false).unwrap();
    assert_eq!(
        imported.image.pixels,
        Pixels::Rgba8(vec![128, 128, 128, 255])
    );
}

#[test]
fn bmp_linked_profile_and_more_than_eight_bit_channels_are_refused() {
    let image = source(Pixels::Rgba8(vec![1, 2, 3, 4]), 1, 1);
    let mut bytes = encoded(&image, ExportFormat::Bmp8);
    bytes[70..74].copy_from_slice(&0x4c49_4e4b_u32.to_le_bytes());
    assert!(matches!(
        inspect(&bytes, Limits::default()),
        Err(Error::Color)
    ));
    let mut bytes = encoded(&image, ExportFormat::Bmp8);
    for (at, mask) in [
        (54, 0x3ff0_0000_u32),
        (58, 0x000f_fc00),
        (62, 0x0000_03ff),
        (66, 0xc000_0000),
    ] {
        bytes[at..at + 4].copy_from_slice(&mask.to_le_bytes());
    }
    assert!(matches!(
        inspect(&bytes, Limits::default()),
        Err(Error::Depth)
    ));
}

#[test]
fn output_cap_empty_destination_and_cancellation_have_no_success_receipt() {
    let image = source(Pixels::Rgba8(vec![1, 2, 3, 4]), 1, 1);
    let mut out = Cursor::new(Vec::new());
    let limited = Limits {
        output_bytes: 10,
        ..Limits::default()
    };
    assert!(matches!(
        encode_to(&image, ExportFormat::Tiff8, limited, &mut out, &|| false),
        Err(Error::Limit)
    ));
    assert!(out.get_ref().len() <= 10);
    let mut occupied = Cursor::new(vec![0xaa]);
    assert!(matches!(
        encode_to(
            &image,
            ExportFormat::Tiff8,
            Limits::default(),
            &mut occupied,
            &|| false
        ),
        Err(Error::Invalid)
    ));
    assert_eq!(occupied.get_ref(), &[0xaa]);
    let mut empty = Cursor::new(Vec::new());
    assert!(matches!(
        encode_to(
            &image,
            ExportFormat::Tiff8,
            Limits::default(),
            &mut empty,
            &|| true
        ),
        Err(Error::Cancelled)
    ));
    assert!(empty.get_ref().is_empty());
    let encoded = encoded(&image, ExportFormat::Tiff8);
    let calls = Cell::new(0);
    assert!(matches!(
        decode(&encoded, Limits::default(), &|| {
            calls.set(calls.get() + 1);
            calls.get() > 2
        }),
        Err(Error::Cancelled)
    ));
}
