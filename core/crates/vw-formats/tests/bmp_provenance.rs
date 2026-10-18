#![allow(clippy::unwrap_used, clippy::panic)]
use std::{cell::Cell, io::Cursor};
use vw_formats::{
    Error, Limits, MAX_METADATA_BYTES, bmp_provenance, decode, encode_bmp_with_provenance,
};
use vw_raster::{DecodedImage, Pixels};

fn source() -> DecodedImage {
    DecodedImage {
        width: 2,
        height: 1,
        pixels: Pixels::Rgba8(vec![254, 1, 99, 0, 13, 24, 35, 200]),
        icc: Some(vw_raster::standard_srgb_profile().unwrap()),
        source_asset: vw_model::AssetId::hash(b"unchanged-original"),
        original_available: true,
        orientation_applied: 1,
    }
}
fn encoded(metadata: &str) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    let receipt =
        encode_bmp_with_provenance(&source(), Limits::default(), metadata, &mut out, &|| false)
            .unwrap();
    assert_eq!(receipt.bytes, out.get_ref().len() as u64);
    assert_eq!(receipt.source_asset, source().source_asset);
    out.into_inner()
}
fn le(bytes: &[u8], at: usize) -> usize {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize
}

#[test]
fn gap_carrier_preserves_normal_bmp_pixels_alpha_icc_and_original_identity() {
    for metadata in [
        "abc".into(),
        format!(
            r#"{{"revision":"r9-aabbccdd","source_asset":"{}"}}"#,
            source().source_asset
        ),
        "x".repeat(MAX_METADATA_BYTES),
    ] {
        let bytes = encoded(&metadata);
        assert_eq!(
            bmp_provenance(&bytes, Limits::default()).unwrap(),
            Some(metadata.as_str())
        );
        assert_eq!(le(&bytes, 2), bytes.len());
        assert_eq!(&bytes[6..10], &[0; 4]);
        assert_eq!(&bytes[134..138], &[0; 4]);
        assert_eq!(&bytes[138..146], b"VWBMETA\0");
        assert_eq!(le(&bytes, 10), 154 + metadata.len().div_ceil(4) * 4);
        assert_eq!(le(&bytes, 126) + 14, le(&bytes, 10) + 8);
        assert_eq!(
            &bytes[le(&bytes, 126) + 14..],
            source().icc.as_ref().unwrap()
        );
        // An ordinary third-party BMP reader skips the gap using bfOffBits.
        let ordinary = image::load_from_memory_with_format(&bytes, image::ImageFormat::Bmp)
            .unwrap()
            .to_rgba8();
        let imported = decode(&bytes, Limits::default(), &|| false).unwrap();
        assert_eq!(Pixels::Rgba8(ordinary.into_raw()), source().pixels);
        assert_eq!(imported.image.pixels, source().pixels);
        assert_eq!(imported.image.icc, source().icc);
        assert_eq!(imported.image.source_asset, vw_model::AssetId::hash(&bytes));
    }
}

#[test]
fn recognized_malformed_carriers_refuse_without_confusing_icc() {
    let original = encoded("abc");
    for (at, value) in [(146, 2_u32), (150, u32::MAX), (10, 154)] {
        let mut bytes = original.clone();
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        assert!(bmp_provenance(&bytes, Limits::default()).is_err());
    }
    let mut bytes = original.clone();
    bytes[157] = 1;
    assert!(bmp_provenance(&bytes, Limits::default()).is_err());
    let mut bytes = original.clone();
    bytes[154] = 0;
    assert!(bmp_provenance(&bytes, Limits::default()).is_err());
    let mut unknown = original;
    unknown[138] = b'?';
    assert_eq!(bmp_provenance(&unknown, Limits::default()).unwrap(), None);
}

#[test]
fn invalid_payload_and_unavailable_original_refuse_before_writes() {
    for metadata in [
        String::new(),
        "bad\0payload".into(),
        "\u{e9}".into(),
        "x".repeat(MAX_METADATA_BYTES + 1),
    ] {
        let mut output = Cursor::new(Vec::new());
        assert!(
            encode_bmp_with_provenance(
                &source(),
                Limits::default(),
                &metadata,
                &mut output,
                &|| false
            )
            .is_err()
        );
        assert!(output.get_ref().is_empty());
    }
    let mut image = source();
    image.original_available = false;
    let mut output = Cursor::new(Vec::new());
    assert!(matches!(
        encode_bmp_with_provenance(&image, Limits::default(), "x", &mut output, &|| false),
        Err(Error::Unsupported)
    ));
    assert!(output.get_ref().is_empty());
}

#[test]
fn output_cap_and_mid_write_cancellation_never_produce_a_receipt() {
    let bytes = encoded("abc");
    let mut output = Cursor::new(Vec::new());
    let limits = Limits {
        output_bytes: bytes.len() as u64 - 1,
        ..Limits::default()
    };
    assert!(matches!(
        encode_bmp_with_provenance(&source(), limits, "abc", &mut output, &|| false),
        Err(Error::Limit)
    ));
    assert!(output.get_ref().len() < bytes.len());
    let calls = Cell::new(0);
    let mut output = Cursor::new(Vec::new());
    assert!(matches!(
        encode_bmp_with_provenance(&source(), Limits::default(), "abc", &mut output, &|| {
            let n = calls.get();
            calls.set(n + 1);
            n > 6
        }),
        Err(Error::Cancelled)
    ));
}

#[test]
fn magic_inside_pixels_is_not_a_metadata_gap() {
    let mut source = source();
    // BGRA byte order will produce the carrier magic as actual pixel bytes.
    source.pixels = Pixels::Rgba8(vec![b'B', b'W', b'V', b'M', b'A', b'T', b'E', 0]);
    let mut out = Cursor::new(Vec::new());
    vw_formats::encode_to(
        &source,
        vw_formats::ExportFormat::Bmp8,
        Limits::default(),
        &mut out,
        &|| false,
    )
    .unwrap();
    assert_eq!(&out.get_ref()[138..146], b"VWBMETA\0");
    assert_eq!(
        bmp_provenance(out.get_ref(), Limits::default()).unwrap(),
        None
    );
}
