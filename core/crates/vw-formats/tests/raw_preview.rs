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
    u16b(&mut b, 4, le);
    entry(&mut b, 254, 1, le);
    entry(&mut b, 513, 116, le);
    entry(&mut b, 514, jpeg.len() as u32, le);
    entry(&mut b, 50970, 0, le);
    u32b(&mut b, 0, le);
    assert_eq!(b.len(), 116);
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
                assert_eq!(preview.encoded().as_ptr(), bytes[116..].as_ptr());
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
    no_reference[76..78].copy_from_slice(&500_u16.to_le_bytes());
    no_reference[88..90].copy_from_slice(&501_u16.to_le_bytes());
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
        (84, u32::MAX),
        (96, u32::MAX),
        (30, 9),
        (108, 4),
    ] {
        let mut b = bytes.clone();
        put(&mut b, at, v);
        assert!(
            raw_preview::embedded(&b, Limits::default(), &|| false).is_err(),
            "offset {at}"
        );
    }
    let mut b = bytes.clone();
    b[88..90].copy_from_slice(&513_u16.to_le_bytes());
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

struct Embedded {
    subtype: u32,
    jpeg: Vec<u8>,
    icc: Option<Vec<u8>>,
}

// A real declared SubIFD graph with separate profile/JPEG data extents. Offsets
// are generated from complete directory sizes, never patched to fake a profile.
fn graph(previews: &[Embedded]) -> Vec<u8> {
    let array_bytes = if previews.len() > 1 {
        previews.len() * 4
    } else {
        0
    };
    let mut next = 62 + array_bytes;
    let offsets: Vec<usize> = previews
        .iter()
        .map(|p| {
            let at = next;
            next += 6 + 12 * (4 + usize::from(p.icc.is_some()));
            at
        })
        .collect();
    let mut b = b"II".to_vec();
    u16b(&mut b, 42, true);
    u32b(&mut b, 8, true);
    u16b(&mut b, 4, true);
    entry(&mut b, 262, 32803, true);
    entry(&mut b, 274, 1, true);
    u16b(&mut b, 330, true);
    u16b(&mut b, 4, true);
    u32b(&mut b, previews.len() as u32, true);
    u32b(
        &mut b,
        if previews.len() > 1 {
            62
        } else {
            offsets[0] as u32
        },
        true,
    );
    u16b(&mut b, 50706, true);
    u16b(&mut b, 1, true);
    u32b(&mut b, 4, true);
    b.extend([1, 4, 0, 0]);
    u32b(&mut b, 0, true);
    if previews.len() > 1 {
        for offset in &offsets {
            u32b(&mut b, *offset as u32, true);
        }
    }
    let mut data_at = next;
    for (p, offset) in previews.iter().zip(offsets) {
        assert_eq!(b.len(), offset);
        u16b(&mut b, (4 + usize::from(p.icc.is_some())) as u16, true);
        entry(&mut b, 254, p.subtype, true);
        let profile_len = p.icc.as_ref().map_or(0, Vec::len);
        entry(&mut b, 513, (data_at + profile_len) as u32, true);
        entry(&mut b, 514, p.jpeg.len() as u32, true);
        if let Some(icc) = &p.icc {
            u16b(&mut b, 34675, true);
            u16b(&mut b, 7, true);
            u32b(&mut b, icc.len() as u32, true);
            u32b(&mut b, data_at as u32, true);
        }
        entry(&mut b, 50970, 0, true);
        u32b(&mut b, 0, true);
        data_at += profile_len + p.jpeg.len();
    }
    assert_eq!(b.len(), next);
    for p in previews {
        if let Some(icc) = &p.icc {
            b.extend(icc);
        }
        b.extend(&p.jpeg);
    }
    b
}

fn with_icc_chunks(jpeg: &[u8], chunks: &[(u8, u8, &[u8])]) -> Vec<u8> {
    let mut b = jpeg[..2].to_vec();
    for (sequence, count, chunk) in chunks {
        b.extend([255, 226]);
        b.extend(((chunk.len() + 16) as u16).to_be_bytes());
        b.extend(b"ICC_PROFILE\0");
        b.extend([*sequence, *count]);
        b.extend(*chunk);
    }
    b.extend(&jpeg[2..]);
    b
}

#[test]
fn containing_ifd_icc_is_explicitly_refused_for_selected_preview() {
    let icc = vw_raster::standard_srgb_profile().unwrap();
    let bytes = graph(&[Embedded {
        subtype: 1,
        jpeg: jpeg(),
        icc: Some(icc.clone()),
    }]);
    assert!(matches!(
        raw_preview::embedded(&bytes, Limits::default(), &|| false),
        Err(Error::Color)
    ));
    // JPEG ICC matching the containing IFD still cannot silently override it.
    let tagged = with_icc_chunks(&jpeg(), &[(1, 1, &icc)]);
    let bytes = graph(&[Embedded {
        subtype: 1,
        jpeg: tagged,
        icc: Some(icc),
    }]);
    assert!(matches!(
        raw_preview::embedded(&bytes, Limits::default(), &|| false),
        Err(Error::Color)
    ));
}

#[test]
fn jpeg_icc_sequences_must_be_complete_consistent_unique_and_nonempty() {
    let icc = vw_raster::standard_srgb_profile().unwrap();
    let mid = icc.len() / 2;
    let first = &icc[..mid];
    let second = &icc[mid..];
    let cases: Vec<Vec<(u8, u8, &[u8])>> = vec![
        vec![(1, 2, first)],
        vec![(2, 2, second)],
        vec![(0, 1, &icc)],
        vec![(1, 0, &icc)],
        vec![(2, 1, &icc)],
        vec![(1, 2, first), (1, 2, second)],
        vec![(1, 2, first), (2, 3, second)],
        vec![(1, 1, &[])],
    ];
    for chunks in cases {
        let bytes = fixture(true, true, 1, &with_icc_chunks(&jpeg(), &chunks));
        assert!(matches!(
            raw_preview::embedded(&bytes, Limits::default(), &|| false),
            Err(Error::Color)
        ));
    }
    let mut malformed = with_icc_chunks(&jpeg(), &[(1, 1, &icc)]);
    malformed[17] = b'?'; // ICC_PROFILE followed by an invalid signature terminator.
    assert!(matches!(
        raw_preview::embedded(
            &fixture(true, true, 1, &malformed),
            Limits::default(),
            &|| false
        ),
        Err(Error::Color)
    ));
}

#[test]
fn complete_jpeg_icc_survives_decoding_byte_for_byte_in_sequence_order() {
    let icc = vw_raster::standard_srgb_profile().unwrap();
    let mid = icc.len() / 2;
    // APP2 physical order is independent of the sequence numbering.
    for chunks in [
        vec![(1, 1, icc.as_slice())],
        vec![(2, 2, &icc[mid..]), (1, 2, &icc[..mid])],
    ] {
        let jpeg = with_icc_chunks(&jpeg(), &chunks);
        let bytes = fixture(true, true, 1, &jpeg);
        let preview = raw_preview::embedded(&bytes, Limits::default(), &|| false).unwrap();
        let decoded = preview.decode(Limits::default(), &|| false).unwrap();
        assert_eq!(decoded.icc.as_ref(), Some(&icc));
        assert_eq!(decoded.source_asset, vw_model::AssetId::hash(&bytes));
    }
}

#[test]
fn a_profile_declared_after_entropy_cannot_disappear_during_decode() {
    let icc = vw_raster::standard_srgb_profile().unwrap();
    let tagged = with_icc_chunks(&jpeg(), &[(1, 1, &icc)]);
    let end = 2 + 18 + icc.len();
    let app2 = &tagged[2..end];
    let mut jpeg = jpeg();
    jpeg.splice(jpeg.len() - 2..jpeg.len() - 2, app2.iter().copied());
    let bytes = fixture(true, true, 1, &jpeg);
    let preview = raw_preview::embedded(&bytes, Limits::default(), &|| false).unwrap();
    // The pinned raster adapter queries ICC while opening JPEG headers, before
    // reading entropy. This real late declaration would otherwise disappear.
    assert!(matches!(
        preview.decode(Limits::default(), &|| false),
        Err(Error::Color)
    ));
}

#[test]
fn dng_default_selects_primary_even_when_alternative_or_mask_is_larger() {
    let mut larger = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut larger)
        .encode(&[128; 8 * 6 * 3], 8, 6, image::ExtendedColorType::Rgb8)
        .unwrap();
    let primary = jpeg();
    for subtype in [0x10001, 4, 5, 8, 9, 16, 65540, 0, u32::MAX] {
        for primary_first in [false, true] {
            let mut inputs = vec![
                Embedded {
                    subtype,
                    jpeg: larger.clone(),
                    icc: Some(vw_raster::standard_srgb_profile().unwrap()),
                },
                Embedded {
                    subtype: 1,
                    jpeg: primary.clone(),
                    icc: None,
                },
            ];
            if primary_first {
                inputs.reverse();
            }
            let bytes = graph(&inputs);
            let selected = raw_preview::embedded(&bytes, Limits::default(), &|| false).unwrap();
            assert_eq!(selected.encoded(), primary);
            assert_eq!((selected.width(), selected.height()), (3, 2));
        }
        let bytes = graph(&[Embedded {
            subtype,
            jpeg: larger.clone(),
            icc: None,
        }]);
        assert!(matches!(
            raw_preview::embedded(&bytes, Limits::default(), &|| false),
            Err(Error::Unsupported)
        ));
    }
    // Missing NewSubFileType is not primary for DNG, even with a valid JPEG.
    let mut bytes = fixture(true, true, 1, &primary);
    bytes[64..66].copy_from_slice(&253_u16.to_le_bytes());
    assert!(matches!(
        raw_preview::embedded(&bytes, Limits::default(), &|| false),
        Err(Error::Unsupported)
    ));
    // Retain the documented legacy non-DNG path without admitting alternatives.
    let mut legacy = fixture(false, true, 1, &primary);
    legacy[64..66].copy_from_slice(&253_u16.to_le_bytes());
    assert_eq!(
        raw_preview::embedded(&legacy, Limits::default(), &|| false)
            .unwrap()
            .encoded(),
        primary
    );
}
