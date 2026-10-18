#![allow(clippy::unwrap_used, clippy::panic)]
use std::{cell::Cell, io::Cursor};
use vw_formats::{Error, ExportFormat, Limits, encode_to, raw_preview};

fn jpeg() -> Vec<u8> {
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 100)
        .encode(
            &[
                255, 0, 0, 0, 255, 0, 0, 0, 255, 127, 127, 127, 255, 255, 255, 0, 0, 0,
            ],
            3,
            2,
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    bytes
}
fn u16b(b: &mut Vec<u8>, v: u16, le: bool) {
    b.extend(if le { v.to_le_bytes() } else { v.to_be_bytes() });
}
fn u32b(b: &mut Vec<u8>, v: u32, le: bool) {
    b.extend(if le { v.to_le_bytes() } else { v.to_be_bytes() });
}
fn entry(b: &mut Vec<u8>, tag: u16, value: u32, le: bool) {
    u16b(b, tag, le);
    u16b(b, 4, le);
    u32b(b, 1, le);
    u32b(b, value, le);
}
fn fixture(dng: bool, le: bool, orientation: u8, jpeg: &[u8]) -> Vec<u8> {
    let mut b = if le { b"II".to_vec() } else { b"MM".to_vec() };
    u16b(&mut b, 42, le);
    u32b(&mut b, 8, le);
    u16b(&mut b, 4, le);
    entry(&mut b, 262, 32803, le);
    entry(&mut b, 274, u32::from(orientation), le);
    entry(&mut b, 330, 62, le);
    u16b(&mut b, if dng { 50706 } else { 50707 }, le);
    u16b(&mut b, 1, le);
    u32b(&mut b, 4, le);
    b.extend([1, 4, 0, 0]);
    u32b(&mut b, 0, le);
    assert_eq!(b.len(), 62);
    u16b(&mut b, 3, le);
    entry(&mut b, 513, 104, le);
    entry(&mut b, 514, jpeg.len() as u32, le);
    entry(&mut b, 50970, 0, le);
    u32b(&mut b, 0, le);
    assert_eq!(b.len(), 104);
    b.extend(jpeg);
    b
}
fn put(b: &mut [u8], at: usize, value: u32) {
    b[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn borrowed_preview_preserves_original_hash_and_orientation_on_both_endian_containers() {
    for dng in [false, true] {
        for le in [false, true] {
            for orientation in 1..=8 {
                let jpeg = jpeg();
                let bytes = fixture(dng, le, orientation, &jpeg);
                let before = bytes.clone();
                let preview = raw_preview::embedded(&bytes, Limits::default(), &|| false).unwrap();
                assert_eq!(
                    preview.kind(),
                    if dng {
                        raw_preview::Kind::Dng
                    } else {
                        raw_preview::Kind::TiffCfa
                    }
                );
                assert_eq!(preview.encoded(), jpeg);
                assert_eq!((preview.width(), preview.height()), (3, 2));
                assert_eq!(preview.orientation(), orientation);
                assert_eq!(preview.encoded().as_ptr(), bytes[104..].as_ptr());
                assert_eq!(preview.source_asset(), &vw_model::AssetId::hash(&bytes));
                let image = preview.decode(Limits::default(), &|| false).unwrap();
                assert_eq!(image.source_asset, vw_model::AssetId::hash(&bytes));
                assert_eq!(image.orientation_applied, orientation);
                assert_eq!(
                    (image.width, image.height),
                    if orientation >= 5 { (2, 3) } else { (3, 2) }
                );
                assert!(!image.original_available);
                assert_eq!(bytes, before);
                let mut output = Cursor::new(Vec::new());
                assert!(matches!(
                    encode_to(
                        &image,
                        ExportFormat::Tiff8,
                        Limits::default(),
                        &mut output,
                        &|| false
                    ),
                    Err(Error::Unsupported)
                ));
                assert!(output.get_ref().is_empty());
            }
        }
    }
}

#[test]
fn arbitrary_jpeg_search_and_ordinary_tiff_do_not_become_raw_import() {
    let mut bytes = fixture(false, true, 1, &jpeg());
    put(&mut bytes, 18, 2); // Photometric RGB, no RAW marker.
    assert!(matches!(
        raw_preview::embedded(&bytes, Limits::default(), &|| false),
        Err(Error::Unsupported)
    ));
    let mut no_reference = fixture(true, true, 1, &jpeg());
    // Keep JPEG bytes in the file but remove the declared JPEG pointer/length.
    no_reference[64..66].copy_from_slice(&500_u16.to_le_bytes());
    no_reference[76..78].copy_from_slice(&501_u16.to_le_bytes());
    assert!(matches!(
        raw_preview::embedded(&no_reference, Limits::default(), &|| false),
        Err(Error::Unsupported)
    ));
}

#[test]
fn cycles_duplicate_tags_bad_offsets_lengths_and_orientation_refuse() {
    let bytes = fixture(true, true, 1, &jpeg());
    for (at, v) in [
        (42, 8_u32),
        (60, 8),
        (72, u32::MAX),
        (84, u32::MAX),
        (30, 9),
        (96, 4),
    ] {
        let mut b = bytes.clone();
        put(&mut b, at, v);
        assert!(
            raw_preview::embedded(&b, Limits::default(), &|| false).is_err(),
            "offset {at}"
        );
    }
    let mut b = bytes.clone();
    b[76..78].copy_from_slice(&513_u16.to_le_bytes());
    assert!(raw_preview::embedded(&b, Limits::default(), &|| false).is_err());
    for n in 0..bytes.len() {
        assert!(
            raw_preview::embedded(&bytes[..n], Limits::default(), &|| false).is_err(),
            "truncation {n}"
        );
    }
}

#[test]
fn admitted_extent_memory_encoded_and_cancellation_limits_cannot_be_enlarged() {
    let bytes = fixture(true, true, 1, &jpeg());
    for limits in [
        Limits {
            pixels: 5,
            ..Limits::default()
        },
        Limits {
            memory_bytes: 64 * 1024 * 1024,
            ..Limits::default()
        },
        Limits {
            encoded_bytes: bytes.len() as u64 - 1,
            ..Limits::default()
        },
    ] {
        assert!(matches!(
            raw_preview::embedded(&bytes, limits, &|| false),
            Err(Error::Limit)
        ));
    }
    let preview = raw_preview::embedded(&bytes, Limits::default(), &|| false).unwrap();
    assert!(matches!(
        preview.decode(
            Limits {
                pixels: 5,
                ..Limits::default()
            },
            &|| false
        ),
        Err(Error::Limit)
    ));
    let calls = Cell::new(0);
    assert!(matches!(
        raw_preview::embedded(&bytes, Limits::default(), &|| {
            let n = calls.get();
            calls.set(n + 1);
            n > 2
        }),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        preview.decode(Limits::default(), &|| true),
        Err(Error::Cancelled)
    ));
}

#[test]
fn a_later_jpeg_frame_cannot_revise_admitted_preview_extent() {
    let mut jpeg = jpeg();
    let sof = jpeg.windows(2).position(|b| b == [255, 192]).unwrap();
    let len = u16::from_be_bytes([jpeg[sof + 2], jpeg[sof + 3]]) as usize;
    let frame = jpeg[sof..sof + 2 + len].to_vec();
    jpeg.splice(jpeg.len() - 2..jpeg.len() - 2, frame);
    let bytes = fixture(true, true, 1, &jpeg);
    assert!(matches!(
        raw_preview::embedded(&bytes, Limits::default(), &|| false),
        Err(Error::Unsupported)
    ));
}
