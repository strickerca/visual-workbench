#![allow(clippy::unwrap_used, clippy::panic)]
use std::cell::Cell;
use vw_formats::{Error, Limits, psd};
use vw_raster::Pixels;

fn resource(id: u16, data: &[u8]) -> Vec<u8> {
    let mut b = b"8BIM".to_vec();
    b.extend(id.to_be_bytes());
    b.extend([0, 0]);
    b.extend((data.len() as u32).to_be_bytes());
    b.extend(data);
    if !data.len().is_multiple_of(2) {
        b.push(0);
    }
    b
}
fn fixture(psb: bool, depth: u16, gray: bool, rle: bool, resources: &[u8]) -> Vec<u8> {
    let mut b = b"8BPS".to_vec();
    b.extend((if psb { 2_u16 } else { 1 }).to_be_bytes());
    b.extend([0; 6]);
    let channels = if gray { 1_u16 } else { 3 };
    b.extend(channels.to_be_bytes());
    b.extend(2_u32.to_be_bytes());
    b.extend(3_u32.to_be_bytes());
    b.extend(depth.to_be_bytes());
    b.extend((if gray { 1_u16 } else { 3 }).to_be_bytes());
    b.extend(0_u32.to_be_bytes());
    b.extend((resources.len() as u32).to_be_bytes());
    b.extend(resources);
    // Actual layer bytes are ignored, not used to regenerate a composite.
    let layers = [0; 12];
    if psb {
        b.extend((layers.len() as u64).to_be_bytes());
    } else {
        b.extend((layers.len() as u32).to_be_bytes());
    }
    b.extend(layers);
    b.extend(u16::from(rle).to_be_bytes());
    let row_len = 3 * usize::from(depth / 8);
    if rle {
        for _ in 0..channels * 2 {
            if psb {
                b.extend(((row_len + 1) as u32).to_be_bytes());
            } else {
                b.extend(((row_len + 1) as u16).to_be_bytes());
            }
        }
    }
    for c in 0..channels {
        for y in 0..2 {
            if rle {
                b.push((row_len - 1) as u8);
            }
            for x in 0..3 {
                let v = c * 1000 + y * 256 + x + 255;
                if depth == 8 {
                    b.push(v as u8);
                } else {
                    b.extend(v.to_be_bytes());
                }
            }
        }
    }
    b
}

#[test]
fn raw_and_packbits_psd_psb_8_16_exact_planar_samples_identity_and_profile() {
    for psb in [false, true] {
        for depth in [8, 16] {
            for gray in [false, true] {
                for rle in [false, true] {
                    let profile = if gray {
                        None
                    } else {
                        Some(vw_raster::standard_srgb_profile().unwrap())
                    };
                    let resources = profile
                        .as_ref()
                        .map_or_else(Vec::new, |p| resource(1039, p));
                    let bytes = fixture(psb, depth, gray, rle, &resources);
                    let before = bytes.clone();
                    let result = psd::decode(&bytes, Limits::default(), &|| false).unwrap();
                    let values: Vec<u16> = (0..2)
                        .flat_map(|y| {
                            (0..3).flat_map(move |x| {
                                let v = y * 256 + x + 255;
                                if gray {
                                    [v, v, v, 65535]
                                } else {
                                    [v, v + 1000, v + 2000, 65535]
                                }
                            })
                        })
                        .collect();
                    let expected = if depth == 8 {
                        Pixels::Rgba8(values.iter().map(|v| *v as u8).collect())
                    } else {
                        Pixels::Rgba16(values)
                    };
                    assert_eq!(result.image.pixels, expected);
                    assert_eq!(result.image.icc, profile);
                    assert_eq!(bytes, before);
                    assert_eq!(result.image.source_asset, vw_model::AssetId::hash(&bytes));
                    assert_eq!(
                        result.info.kind,
                        if psb { psd::Kind::Psb } else { psd::Kind::Psd }
                    );
                    assert!(result.image.original_available);
                }
            }
        }
    }
}

#[test]
fn absent_dummy_transparent_zip_and_wrong_color_are_explicit_refusals() {
    let bytes = fixture(false, 8, false, false, &[]);
    let mut missing = bytes.clone();
    missing.truncate(50);
    assert!(psd::decode(&missing, Limits::default(), &|| false).is_err());
    let dummy = resource(1057, &[0, 0, 0, 1, 0]);
    assert!(matches!(
        psd::decode(
            &fixture(false, 8, false, false, &dummy),
            Limits::default(),
            &|| false
        ),
        Err(Error::Unsupported)
    ));
    for (at, v) in [(12, 4_u16), (50, 2_u16), (24, 4_u16)] {
        let mut b = bytes.clone();
        b[at..at + 2].copy_from_slice(&v.to_be_bytes());
        assert!(psd::decode(&b, Limits::default(), &|| false).is_err());
    }
}

#[test]
fn section_lengths_packbits_overrun_truncation_and_trailing_payload_refuse() {
    let bytes = fixture(true, 16, false, true, &[]);
    for n in 0..bytes.len() {
        assert!(
            psd::decode(&bytes[..n], Limits::default(), &|| false).is_err(),
            "truncation {n}"
        );
    }
    let mut b = bytes.clone();
    b.push(0);
    assert!(psd::decode(&b, Limits::default(), &|| false).is_err());
    let mut b = bytes.clone();
    b[34..42].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(psd::decode(&b, Limits::default(), &|| false).is_err());
    let mut b = bytes;
    b[80] = 127;
    assert!(psd::decode(&b, Limits::default(), &|| false).is_err());
}

#[test]
fn pixel_memory_limits_and_cancellation_are_enforced() {
    let bytes = fixture(false, 8, false, true, &[]);
    assert!(matches!(
        psd::decode(
            &bytes,
            Limits {
                pixels: 5,
                ..Limits::default()
            },
            &|| false
        ),
        Err(Error::Limit)
    ));
    assert!(matches!(
        psd::decode(
            &bytes,
            Limits {
                memory_bytes: 64 * 1024 * 1024,
                ..Limits::default()
            },
            &|| false
        ),
        Err(Error::Limit)
    ));
    let calls = Cell::new(0);
    assert!(matches!(
        psd::decode(&bytes, Limits::default(), &|| {
            let n = calls.get();
            calls.set(n + 1);
            n > 8
        }),
        Err(Error::Cancelled)
    ));
}

#[test]
fn packbits_repeat_noop_and_exact_row_boundaries() {
    for control in [254_u8, 253, 255] {
        let mut bytes = fixture(false, 8, false, true, &[]);
        bytes.truncate(64);
        for i in 0..6 {
            bytes[52 + i * 2..54 + i * 2].copy_from_slice(&3_u16.to_be_bytes());
        }
        for i in 0..6 {
            bytes.extend([128, control, i as u8]);
        }
        let decoded = psd::decode(&bytes, Limits::default(), &|| false);
        if control == 254 {
            let image = decoded.unwrap().image;
            assert_eq!(
                image.pixels,
                Pixels::Rgba8(vec![
                    0, 2, 4, 255, 0, 2, 4, 255, 0, 2, 4, 255, 1, 3, 5, 255, 1, 3, 5, 255, 1, 3, 5,
                    255
                ])
            );
        } else {
            assert!(decoded.is_err());
        }
    }
}
