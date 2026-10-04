#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::io::Cursor;
use vw_mask::*;

fn size(w: u32, h: u32) -> Size {
    Size::new(w, h).unwrap()
}
fn p(x: f64, y: f64) -> Point {
    Point { x, y }
}
fn rectangle(s: Size, x: f64, y: f64, width: f64, height: f64) -> Mask {
    Mask::rectangle(
        s,
        Rect {
            x,
            y,
            width,
            height,
        },
    )
    .unwrap()
}

#[test]
fn rectangle_integrates_area_instead_of_testing_only_pixel_centers() {
    let mask = rectangle(size(3, 2), 0.25, 0.5, 1.5, 1.0);
    assert_eq!(mask.to_dense().unwrap(), [96, 96, 0, 96, 96, 0]);
    assert_eq!(
        mask.bounds(),
        Some(Region {
            x: 0,
            y: 0,
            width: 2,
            height: 2
        })
    );
    assert_eq!(
        rectangle(size(2, 2), -1.0, -1.0, 1.5, 1.5)
            .to_dense()
            .unwrap(),
        [64, 0, 0, 0]
    );
    assert_eq!(
        rectangle(size(1, 1), 0.0, 0.0, 0.5 / 256.0, 1.0)
            .to_dense()
            .unwrap(),
        [1]
    );
    assert_eq!(rectangle(size(3, 3), 1.25, 1.25, 0.0, 0.0).bounds(), None);
    assert_eq!(rectangle(size(3, 3), -8.0, -8.0, 2.0, 2.0).tile_count(), 0);
}

#[test]
fn lasso_triangle_bowtie_and_reversed_winding_have_known_area() {
    let s = size(2, 2);
    let triangle = [p(0.0, 0.0), p(2.0, 0.0), p(0.0, 2.0)];
    assert_eq!(
        Mask::lasso(s, &triangle).unwrap().to_dense().unwrap(),
        [255, 128, 128, 0]
    );
    let bowtie = [p(0.0, 0.0), p(2.0, 2.0), p(0.0, 2.0), p(2.0, 0.0)];
    let expected = Mask::lasso(s, &bowtie).unwrap();
    assert_eq!(expected.to_dense().unwrap(), [128; 4]);
    assert_eq!(
        expected.content_hash(),
        Mask::lasso(s, &bowtie.into_iter().rev().collect::<Vec<_>>())
            .unwrap()
            .content_hash()
    );
    let closed = [
        p(-1.0, -1.0),
        p(0.5, -1.0),
        p(0.5, 0.5),
        p(-1.0, 0.5),
        p(-1.0, -1.0),
    ];
    assert_eq!(
        Mask::lasso(s, &closed).unwrap().to_dense().unwrap(),
        [64, 0, 0, 0]
    );
}

#[test]
fn painted_dabs_capsules_and_opacity_are_applied_once() {
    // Circle area pi/4 and half-circle area pi/8, independently rounded to u8.
    assert_eq!(
        Mask::paint(size(1, 1), &[p(0.5, 0.5)], 0.5, 255)
            .unwrap()
            .to_dense()
            .unwrap(),
        [200]
    );
    assert_eq!(
        Mask::paint(size(1, 1), &[p(0.0, 0.5)], 0.5, 255)
            .unwrap()
            .to_dense()
            .unwrap(),
        [100]
    );
    let s = size(3, 1);
    let path = [p(0.5, 0.5), p(2.5, 0.5)];
    let mask = Mask::paint(s, &path, 0.5, 255).unwrap();
    // End pixel area = 1/2 + pi/8. The middle pixel is fully selected.
    assert_eq!(mask.to_dense().unwrap(), [228, 255, 228]);
    assert_eq!(
        Mask::paint(s, &path, 0.5, 128).unwrap().to_dense().unwrap(),
        [114, 128, 114]
    );
    let divided = [path[0], p(1.5, 0.5), p(1.5, 0.5), path[1]];
    assert_eq!(
        mask.content_hash(),
        Mask::paint(s, &divided, 0.5, 255).unwrap().content_hash()
    );
    assert_eq!(
        mask.content_hash(),
        Mask::paint(s, &[path[1], path[0]], 0.5, 255)
            .unwrap()
            .content_hash()
    );
    assert_eq!(Mask::paint(s, &path, 0.5, 0).unwrap().tile_count(), 0);
}

#[test]
fn every_math_operation_preserves_inputs_and_returns_a_new_version() {
    let s = size(4, 1);
    let a = Mask::from_dense(s, &[0, 64, 128, 255]).unwrap();
    let b = Mask::from_dense(s, &[255, 128, 64, 0]).unwrap();
    let original = a.encode_lossless().unwrap();
    assert_eq!(a.add(&b).unwrap().to_dense().unwrap(), [255, 128, 128, 255]);
    assert_eq!(a.subtract(&b).unwrap().to_dense().unwrap(), [0, 0, 64, 255]);
    assert_eq!(a.intersect(&b).unwrap().to_dense().unwrap(), [0, 64, 64, 0]);
    assert_eq!(a.invert().unwrap().to_dense().unwrap(), [255, 191, 127, 0]);
    for out in [
        a.add(&b).unwrap(),
        a.subtract(&b).unwrap(),
        a.intersect(&b).unwrap(),
        a.invert().unwrap(),
        a.expand(0).unwrap(),
        a.shrink(0).unwrap(),
        a.feather(0).unwrap(),
    ] {
        assert_eq!(out.version(), 1);
    }
    assert_eq!(a.encode_lossless().unwrap(), original);
    assert_eq!(
        a.invert().unwrap().invert().unwrap().content_hash(),
        a.content_hash()
    );
    assert_eq!(a.invert().unwrap().invert().unwrap().version(), 2);
    assert!(matches!(
        a.add(&Mask::empty(size(2, 2))),
        Err(MaskError::SizeMismatch)
    ));
}

#[test]
fn euclidean_morphology_and_feather_have_finite_known_support() {
    let mut values = vec![0; 25];
    values[12] = 255;
    let impulse = Mask::from_dense(size(5, 5), &values).unwrap();
    let plus = [
        0, 0, 0, 0, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0,
    ];
    assert_eq!(impulse.expand(1).unwrap().to_dense().unwrap(), plus);
    assert_eq!(
        impulse.feather(1).unwrap().to_dense().unwrap(),
        plus.map(|v| if v != 0 { 51 } else { 0 })
    );
    let disk = impulse.expand(2).unwrap().to_dense().unwrap();
    assert_eq!(disk.iter().filter(|&&v| v != 0).count(), 13);
    assert_eq!(
        impulse.feather(2).unwrap().to_dense().unwrap(),
        disk.iter()
            .map(|&v| if v != 0 { 20 } else { 0 })
            .collect::<Vec<_>>()
    );
    let full = Mask::from_dense(size(3, 3), &[255; 9]).unwrap();
    assert_eq!(
        full.shrink(1).unwrap().to_dense().unwrap(),
        [0, 0, 0, 0, 255, 0, 0, 0, 0]
    );
    assert_eq!(
        full.feather(1).unwrap().to_dense().unwrap(),
        [153, 204, 153, 204, 255, 204, 153, 204, 153]
    );
    assert_eq!(full.shrink(2).unwrap().tile_count(), 0);
}

#[test]
fn sparse_tiles_cross_edges_and_are_ordered_by_row() {
    let s = size(513, 258);
    let mask = rectangle(s, 255.5, 255.5, 2.0, 2.0);
    let coords = mask.tiles().map(|tile| tile.coord).collect::<Vec<_>>();
    assert_eq!(
        coords,
        [
            TileCoord { x: 0, y: 0 },
            TileCoord { x: 1, y: 0 },
            TileCoord { x: 0, y: 1 },
            TileCoord { x: 1, y: 1 }
        ]
    );
    assert_eq!(
        mask.crop(Region {
            x: 255,
            y: 255,
            width: 3,
            height: 3
        })
        .unwrap(),
        [64, 128, 64, 128, 255, 128, 64, 128, 64]
    );
    let thin = rectangle(size(513, 1), 0.0, 0.0, 513.0, 1.0);
    assert_eq!(
        thin.tiles().map(|t| t.pixels.len()).collect::<Vec<_>>(),
        [256, 256, 1]
    );
    assert_eq!(thin.coverage(513, 0), 0);
    assert_eq!(thin.coverage(0, 1), 0);
}

#[test]
fn lossless_format_is_canonical_strict_and_versioned() {
    let empty = Mask::empty(size(2, 3));
    let bytes = empty.encode_lossless().unwrap();
    assert_eq!(
        bytes,
        [
            b'V', b'W', b'M', b'A', b'S', b'K', b'0', b'1', 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ]
    );
    let original = rectangle(size(257, 257), 255.5, 255.5, 1.5, 1.5)
        .feather(1)
        .unwrap();
    let bytes = original.encode_lossless().unwrap();
    let decoded = Mask::decode_lossless(&bytes).unwrap();
    assert_eq!(decoded.encode_lossless().unwrap(), bytes);
    assert_eq!(decoded.content_hash(), original.content_hash());
    assert_eq!(decoded.version(), original.version());
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(Mask::decode_lossless(&trailing).is_err());
    for length in [0, 7, 8, 20, 31, bytes.len() - 1] {
        assert!(Mask::decode_lossless(&bytes[..length]).is_err());
    }
    let mut unknown = bytes.clone();
    unknown[8] = 2;
    assert!(Mask::decode_lossless(&unknown).is_err());
    let mut bad_tile = bytes.clone();
    bad_tile[32..36].copy_from_slice(&99u32.to_le_bytes());
    assert!(Mask::decode_lossless(&bad_tile).is_err());
    assert!(
        Mask::from_tiles(
            size(1, 1),
            0,
            vec![TileData {
                coord: TileCoord { x: 0, y: 0 },
                pixels: vec![0]
            }]
        )
        .is_err()
    );
    let tile = TileData {
        coord: TileCoord { x: 0, y: 0 },
        pixels: vec![1],
    };
    assert!(Mask::from_tiles(size(2, 2), 0, vec![tile.clone()]).is_err());
    assert!(Mask::from_tiles(size(1, 1), 0, vec![tile.clone(), tile]).is_err());
    let exhausted = Mask::from_tiles(size(1, 1), u64::MAX, vec![]).unwrap();
    assert!(matches!(
        exhausted.invert(),
        Err(MaskError::VersionExhausted)
    ));
    assert!(matches!(
        exhausted.feather(0),
        Err(MaskError::VersionExhausted)
    ));
    let mut zero = Mask::from_dense(size(1, 1), &[255])
        .unwrap()
        .encode_lossless()
        .unwrap();
    assert_eq!(zero.len(), 45);
    zero[44] = 0;
    assert!(Mask::decode_lossless(&zero).is_err());
    let thin = rectangle(size(257, 1), 0.0, 0.0, 257.0, 1.0)
        .encode_lossless()
        .unwrap();
    let mut reordered = thin[..32].to_vec();
    reordered.extend_from_slice(&thin[300..]);
    reordered.extend_from_slice(&thin[32..300]);
    assert!(Mask::decode_lossless(&reordered).is_err());
}

#[test]
fn mask_png_decodes_to_exact_coverage_without_color_transform() {
    let mask = Mask::from_dense(size(3, 2), &[0, 64, 128, 192, 254, 255]).unwrap();
    let region = Region {
        x: 1,
        y: 0,
        width: 2,
        height: 2,
    };
    let bytes = mask.encode_mask_png(region).unwrap();
    let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    assert!(reader.info().icc_profile.is_none());
    assert!(reader.info().srgb.is_none());
    let mut output = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut output).unwrap();
    assert_eq!(info.color_type, png::ColorType::Grayscale);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    assert_eq!(&output[..info.buffer_size()], &[64, 128, 254, 255]);
}

#[test]
fn cutouts_preserve_original_rgb_depth_and_partial_alpha() {
    let mask = Mask::from_dense(size(3, 1), &[0, 128, 255]).unwrap();
    let source = [10, 20, 30, 255, 40, 50, 60, 128, 70, 80, 90, 77];
    let saved = source;
    assert_eq!(
        mask.cutout_rgba8(&source, Region::full(mask.size()))
            .unwrap()
            .rgba,
        [10, 20, 30, 0, 40, 50, 60, 64, 70, 80, 90, 77]
    );
    assert_eq!(source, saved);
    let region = Region {
        x: 1,
        y: 0,
        width: 2,
        height: 1,
    };
    assert_eq!(mask.cutout_rgba8(&source, region).unwrap().region, region);
    let source16 = [
        1, 257, 65534, 65535, 222, 333, 444, 32768, 777, 888, 999, 12345,
    ];
    assert_eq!(
        mask.cutout_rgba16(&source16, Region::full(mask.size()))
            .unwrap()
            .rgba,
        [1, 257, 65534, 0, 222, 333, 444, 16448, 777, 888, 999, 12345]
    );
    assert!(mask.cutout_rgba8(&source[..8], region).is_err());
    assert!(
        mask.crop(Region {
            x: u32::MAX,
            y: 0,
            width: 2,
            height: 1
        })
        .is_err()
    );
}

#[test]
fn invalid_geometry_and_expensive_work_fail_without_changing_inputs() {
    assert!(Size::new(0, 1).is_err());
    assert!(Size::new(MAX_SIDE + 1, 1).is_err());
    assert!(Size::new(10_000, 10_000).is_err());
    assert!(Mask::from_dense(size(2, 2), &[0; 3]).is_err());
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 9_000_000.0] {
        assert!(
            Mask::rectangle(
                size(1, 1),
                Rect {
                    x: bad,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0
                }
            )
            .is_err()
        );
        assert!(Mask::lasso(size(1, 1), &[p(bad, 0.0), p(1.0, 0.0), p(0.0, 1.0)]).is_err());
    }
    assert!(Mask::lasso(size(1, 1), &[p(0.0, 0.0); 2]).is_err());
    assert!(Mask::paint(size(1, 1), &[], 1.0, 255).is_err());
    for radius in [0.0, -1.0, f64::NAN, f64::from(MAX_RADIUS) + 1.0] {
        assert!(Mask::paint(size(1, 1), &[p(0.0, 0.0)], radius, 255).is_err());
    }
    let large = rectangle(size(1_000_000, 50), 0.0, 0.0, 1.0, 1.0);
    let before = large.encode_lossless().unwrap();
    assert!(matches!(large.expand(MAX_RADIUS), Err(MaskError::Limit)));
    assert!(matches!(large.shrink(MAX_RADIUS), Err(MaskError::Limit)));
    assert!(matches!(large.feather(MAX_RADIUS), Err(MaskError::Limit)));
    assert!(matches!(
        large.expand(MAX_RADIUS + 1),
        Err(MaskError::Limit)
    ));
    assert_eq!(large.encode_lossless().unwrap(), before);
}
