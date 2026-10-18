#![allow(clippy::unwrap_used, clippy::panic)]
use std::io::Cursor;
use vw_formats::{
    Error, ExportFormat, Limits, MAX_METADATA_BYTES, decode, encode_to_with_metadata,
};
use vw_raster::{DecodedImage, Pixels};

fn source(depth: u8) -> DecodedImage {
    DecodedImage {
        width: 2,
        height: 1,
        pixels: if depth == 16 {
            Pixels::Rgba16(vec![256, 257, 258, 0, 65534, 65535, 3, 32769])
        } else {
            Pixels::Rgba8(vec![1, 2, 3, 0, 254, 255, 1, 128])
        },
        icc: Some(vw_raster::standard_srgb_profile().unwrap()),
        source_asset: vw_model::AssetId::hash(b"immutable-original"),
        original_available: true,
        orientation_applied: 1,
    }
}

#[test]
fn normal_tiff_decoder_reads_embedded_metadata_without_changing_pixels_or_icc() {
    for (depth, format) in [(8, ExportFormat::Tiff8), (16, ExportFormat::Tiff16)] {
        let image = source(depth);
        let metadata = format!(
            r#"{{"revision":"r9-aabbccdd","source_asset":"{}","capture_session":"test-capture","frame_id":18446744073709551615}}"#,
            image.source_asset
        );
        let mut output = Cursor::new(Vec::new());
        let receipt = encode_to_with_metadata(
            &image,
            format,
            Limits::default(),
            &metadata,
            &mut output,
            &|| false,
        )
        .unwrap();
        let bytes = output.into_inner();
        assert_eq!(receipt.bytes, bytes.len() as u64);
        let mut reader = tiff::decoder::Decoder::new(Cursor::new(&bytes)).unwrap();
        assert_eq!(
            reader
                .get_tag_ascii_string(tiff::tags::Tag::ImageDescription)
                .unwrap(),
            metadata
        );
        assert_eq!(reader.dimensions().unwrap(), (2, 1));
        match (reader.read_image().unwrap(), &image.pixels) {
            (tiff::decoder::DecodingResult::U8(actual), Pixels::Rgba8(expected)) => {
                assert_eq!(&actual, expected)
            }
            (tiff::decoder::DecodingResult::U16(actual), Pixels::Rgba16(expected)) => {
                assert_eq!(&actual, expected)
            }
            _ => panic!("normal TIFF decoder changed the sample representation"),
        }
        let imported = decode(&bytes, Limits::default(), &|| false).unwrap();
        assert_eq!(imported.image.pixels, image.pixels);
        assert_eq!(imported.image.icc, image.icc);
    }
}

#[test]
fn maximum_metadata_keeps_multiple_strips_exact_and_counts_toward_output_limit() {
    let mut image = source(8);
    image.height = 17;
    if let Pixels::Rgba8(pixels) = &mut image.pixels {
        *pixels = pixels.repeat(17);
    }
    let metadata = "x".repeat(MAX_METADATA_BYTES);
    let mut output = Cursor::new(Vec::new());
    encode_to_with_metadata(
        &image,
        ExportFormat::Tiff8,
        Limits::default(),
        &metadata,
        &mut output,
        &|| false,
    )
    .unwrap();
    let bytes = output.into_inner();
    let mut reader = tiff::decoder::Decoder::new(Cursor::new(&bytes)).unwrap();
    assert_eq!(
        reader
            .get_tag_ascii_string(tiff::tags::Tag::ImageDescription)
            .unwrap(),
        metadata
    );
    assert_eq!(
        decode(&bytes, Limits::default(), &|| false)
            .unwrap()
            .image
            .pixels,
        image.pixels
    );
    let limits = Limits {
        output_bytes: bytes.len() as u64 - 1,
        ..Limits::default()
    };
    let mut private = Cursor::new(Vec::new());
    assert!(matches!(
        encode_to_with_metadata(
            &image,
            ExportFormat::Tiff8,
            limits,
            &metadata,
            &mut private,
            &|| false
        ),
        Err(Error::Limit)
    ));
    assert!(private.get_ref().len() as u64 <= limits.output_bytes);
}

#[test]
fn metadata_admission_is_bounded_and_refuses_non_ascii_nul_or_empty_before_writes() {
    let image = source(8);
    for metadata in [
        String::new(),
        "non-ASCII \u{00e9}".into(),
        "embedded\0NUL".into(),
    ] {
        let mut output = Cursor::new(Vec::new());
        assert!(matches!(
            encode_to_with_metadata(
                &image,
                ExportFormat::Tiff8,
                Limits::default(),
                &metadata,
                &mut output,
                &|| false
            ),
            Err(Error::Invalid)
        ));
        assert!(output.get_ref().is_empty());
    }
    let mut output = Cursor::new(Vec::new());
    assert!(matches!(
        encode_to_with_metadata(
            &image,
            ExportFormat::Tiff8,
            Limits::default(),
            &"x".repeat(MAX_METADATA_BYTES + 1),
            &mut output,
            &|| false
        ),
        Err(Error::Limit)
    ));
    assert!(output.get_ref().is_empty());
}

#[test]
fn provenance_path_cannot_publish_bmp_or_a_cancelled_tiff() {
    let image = source(8);
    let mut output = Cursor::new(Vec::new());
    assert!(matches!(
        encode_to_with_metadata(
            &image,
            ExportFormat::Bmp8,
            Limits::default(),
            "{}",
            &mut output,
            &|| false
        ),
        Err(Error::Unsupported)
    ));
    assert!(output.get_ref().is_empty());
    assert!(matches!(
        encode_to_with_metadata(
            &image,
            ExportFormat::Tiff8,
            Limits::default(),
            "{}",
            &mut output,
            &|| true
        ),
        Err(Error::Cancelled)
    ));
    assert!(output.get_ref().is_empty());
}
