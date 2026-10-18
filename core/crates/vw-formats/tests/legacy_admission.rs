#![allow(clippy::unwrap_used)]

use vw_formats::{Error, Limits, Notice, decode, inspect};
use vw_raster::Pixels;

struct Entry {
    tag: u16,
    kind: u16,
    count: u32,
    data: Vec<u8>,
}
fn short(tag: u16, value: u16) -> Entry {
    Entry {
        tag,
        kind: 3,
        count: 1,
        data: value.to_le_bytes().to_vec(),
    }
}
fn long(tag: u16, value: u32) -> Entry {
    Entry {
        tag,
        kind: 4,
        count: 1,
        data: value.to_le_bytes().to_vec(),
    }
}
fn rationals(tag: u16, values: &[(u32, u32)]) -> Entry {
    Entry {
        tag,
        kind: 5,
        count: values.len() as u32,
        data: values
            .iter()
            .flat_map(|(n, d)| n.to_le_bytes().into_iter().chain(d.to_le_bytes()))
            .collect(),
    }
}

// Independent 1x1 8-bit BlackIsZero TIFF. FillOrder=2 stores bit-reversed
// sample 1 as 0x80; the pinned decoder otherwise interprets it as sample 128.
fn gray_tiff(fill: Option<u16>, legacy: Option<Entry>, icc: Option<&[u8]>) -> Vec<u8> {
    let mut entries = vec![
        long(256, 1),
        long(257, 1),
        short(258, 8),
        short(259, 1),
        short(262, 1),
        long(273, 0),
        short(277, 1),
        long(278, 1),
        long(279, 1),
        short(284, 1),
    ];
    if let Some(fill) = fill {
        entries.push(short(266, fill));
    }
    if let Some(entry) = legacy {
        entries.push(entry);
    }
    if let Some(icc) = icc {
        entries.push(Entry {
            tag: 34675,
            kind: 7,
            count: icc.len() as u32,
            data: icc.to_vec(),
        });
    }
    entries.sort_by_key(|e| e.tag);
    let fixed = 8 + 2 + entries.len() * 12 + 4;
    let end = entries
        .iter()
        .filter(|e| e.data.len() > 4)
        .fold(fixed, |n, e| {
            let end = n + e.data.len();
            end + end % 2
        });
    for entry in &mut entries {
        if entry.tag == 273 {
            entry.data = (end as u32).to_le_bytes().to_vec();
        }
    }
    let mut bytes = vec![0; end];
    bytes[..8].copy_from_slice(b"II*\0\x08\0\0\0");
    bytes[8..10].copy_from_slice(&(entries.len() as u16).to_le_bytes());
    let mut extra = fixed;
    for (i, entry) in entries.iter().enumerate() {
        let at = 10 + i * 12;
        bytes[at..at + 2].copy_from_slice(&entry.tag.to_le_bytes());
        bytes[at + 2..at + 4].copy_from_slice(&entry.kind.to_le_bytes());
        bytes[at + 4..at + 8].copy_from_slice(&entry.count.to_le_bytes());
        if entry.data.len() <= 4 {
            bytes[at + 8..at + 8 + entry.data.len()].copy_from_slice(&entry.data);
        } else {
            bytes[at + 8..at + 12].copy_from_slice(&(extra as u32).to_le_bytes());
            bytes[extra..extra + entry.data.len()].copy_from_slice(&entry.data);
            extra += entry.data.len();
            extra += extra % 2;
        }
    }
    bytes.push(0x80);
    bytes
}

#[test]
fn reversed_fill_order_refuses_before_ignoring_its_pixel_semantics() {
    for fill in [None, Some(1)] {
        let bytes = gray_tiff(fill, None, None);
        assert_eq!(
            decode(&bytes, Limits::default(), &|| false)
                .unwrap()
                .image
                .pixels,
            Pixels::Rgba8(vec![128, 128, 128, 255])
        );
    }
    let reversed = gray_tiff(Some(2), None, None);
    assert!(matches!(
        inspect(&reversed, Limits::default()),
        Err(Error::Unsupported)
    ));
    assert!(matches!(
        decode(&reversed, Limits::default(), &|| false),
        Err(Error::Unsupported)
    ));
}

#[test]
fn unsupported_explicit_color_never_becomes_untagged_even_with_an_icc() {
    let icc = vw_raster::standard_srgb_profile().unwrap();
    for profile in [None, Some(icc.as_slice())] {
        let curves = || (0..256_u16).flat_map(|v| (v * 257).to_le_bytes()).collect();
        let declarations = [
            short(290, 2),
            Entry {
                tag: 291,
                kind: 3,
                count: 256,
                data: curves(),
            },
            short(300, 2),
            Entry {
                tag: 301,
                kind: 3,
                count: 256,
                data: curves(),
            },
            rationals(318, &[(3127, 10000), (3290, 10000)]),
            rationals(
                319,
                &[
                    (64, 100),
                    (33, 100),
                    (30, 100),
                    (60, 100),
                    (15, 100),
                    (6, 100),
                ],
            ),
            Entry {
                tag: 320,
                kind: 3,
                count: 768,
                data: vec![0; 1536],
            },
            Entry {
                tag: 342,
                kind: 3,
                count: 6,
                data: vec![0; 12],
            },
            rationals(529, &[(299, 1000), (587, 1000), (114, 1000)]),
            rationals(532, &[(0, 1), (255, 1), (0, 1), (255, 1), (0, 1), (255, 1)]),
        ];
        for declaration in declarations {
            let tag = declaration.tag;
            let bytes = gray_tiff(None, Some(declaration), profile);
            assert!(
                matches!(inspect(&bytes, Limits::default()), Err(Error::Color)),
                "tag {tag}"
            );
            assert!(
                matches!(
                    decode(&bytes, Limits::default(), &|| false),
                    Err(Error::Color)
                ),
                "tag {tag}"
            );
        }
    }
}

#[test]
fn ordinary_untagged_and_icc_color_still_preserve_their_exact_declarations() {
    let icc = vw_raster::standard_srgb_profile().unwrap();
    for profile in [None, Some(icc.as_slice())] {
        let bytes = gray_tiff(Some(1), None, profile);
        let imported = decode(&bytes, Limits::default(), &|| false).unwrap();
        assert_eq!(imported.header.icc.as_deref(), profile);
        assert_eq!(imported.image.icc.as_deref(), profile);
        assert_eq!(
            imported.header.notices.contains(&Notice::UntaggedColor),
            profile.is_none()
        );
        assert_eq!(imported.image.source_asset, vw_model::AssetId::hash(&bytes));
    }
}
