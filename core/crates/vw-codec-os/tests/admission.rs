#![allow(clippy::expect_used)] // Fixture construction fails the test on malformed setup.
use vw_codec_os::{Decoded, Error, inspect};
const BUDGET: u64 = 256 * 1024 * 1024;
fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
    v.extend(kind);
    v.extend(payload);
    v
}
#[derive(Default)]
struct Bits(Vec<bool>);
impl Bits {
    fn put(&mut self, value: u32, count: usize) {
        for bit in (0..count).rev() {
            self.0.push(value & (1 << bit) != 0);
        }
    }
    fn ue(&mut self, value: u32) {
        let n = value + 1;
        let bits = 32 - n.leading_zeros();
        self.put(0, bits as usize - 1);
        self.put(n, bits as usize);
    }
    fn bytes(mut self) -> Vec<u8> {
        self.0.push(true);
        while !self.0.len().is_multiple_of(8) {
            self.0.push(false);
        }
        self.0
            .chunks(8)
            .map(|p| p.iter().fold(0, |n, b| (n << 1) | u8::from(*b)))
            .collect()
    }
}
fn config(depth_minus8: u32, sps_width: u32) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.put(0, 4);
    bits.put(0, 3);
    bits.put(1, 1);
    for _ in 0..12 {
        bits.put(0, 8);
    }
    bits.ue(0);
    bits.ue(1);
    bits.ue(sps_width);
    bits.ue(8);
    bits.put(0, 1);
    bits.ue(depth_minus8);
    bits.ue(depth_minus8);
    bits.ue(0);
    bits.put(0, 1);
    bits.ue(0);
    bits.ue(0);
    bits.ue(0);
    let mut nal = vec![0x42, 1];
    let mut zeros = 0;
    for value in bits.bytes() {
        if zeros >= 2 && value <= 3 {
            nal.push(3);
            zeros = 0;
        }
        nal.push(value);
        zeros = if value == 0 { zeros + 1 } else { 0 };
    }
    let mut config = vec![0; 23];
    config[0] = 1;
    config[16] = 0xfd;
    config[17] = 0xf8;
    config[18] = 0xf8;
    config[21] = 0xff;
    config[22] = 1;
    config.extend([0xa1, 0, 1]);
    config.extend((nal.len() as u16).to_be_bytes());
    config.extend(nal);
    config
}
/// Container-only synthetic fixture; its tiny IDR body is deliberately not a
/// decodable image. These tests prove admission, never OS pixel acceptance.
fn fixture(depth: u32, sps_width: u32, rotate: bool, media: &[u8]) -> Vec<u8> {
    let ftyp = b"heic\0\0\0\0mif1heic".to_vec();
    let ftyp = bx(b"ftyp", &ftyp);
    let offset = (ftyp.len() + 8) as u32;
    let mut handler = vec![0; 8];
    handler.extend(b"pict");
    handler.extend([0; 13]);
    let mut entry = vec![2, 0, 0, 0, 0, 1, 0, 0];
    entry.extend(b"hvc1");
    entry.push(0);
    let mut iinf = vec![0, 0, 0, 0, 0, 1];
    iinf.extend(bx(b"infe", &entry));
    let mut iloc = vec![0, 0, 0, 0, 0x44, 0, 0, 1, 0, 1, 0, 0, 0, 1];
    iloc.extend(offset.to_be_bytes());
    iloc.extend((media.len() as u32).to_be_bytes());
    let mut ispe = vec![0; 4];
    ispe.extend(16u32.to_be_bytes());
    ispe.extend(8u32.to_be_bytes());
    let mut properties = bx(b"ispe", &ispe);
    properties.extend(bx(b"hvcC", &config(depth, sps_width)));
    properties.extend(bx(b"pixi", &[0, 0, 0, 0, 3, 8, 8, 8]));
    properties.extend(bx(b"colr", b"nclx\0\x01\0\x0d\0\x06\x80"));
    let mut association = vec![
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        1,
        0,
        1,
        if rotate { 5 } else { 4 },
        0x81,
        0x82,
        0x83,
        0x84,
    ];
    if rotate {
        properties.extend(bx(b"irot", &[1]));
        association.push(0x85);
    }
    let mut iprp = bx(b"ipco", &properties);
    iprp.extend(bx(b"ipma", &association));
    let mut meta = vec![0; 4];
    meta.extend(bx(b"hdlr", &handler));
    meta.extend(bx(b"pitm", &[0, 0, 0, 0, 0, 1]));
    meta.extend(bx(b"iinf", &iinf));
    meta.extend(bx(b"iloc", &iloc));
    meta.extend(bx(b"iprp", &iprp));
    let mut value = ftyp;
    value.extend(bx(b"mdat", media));
    value.extend(bx(b"meta", &meta));
    value
}
fn ordinary() -> Vec<u8> {
    fixture(0, 16, false, &[0, 0, 0, 3, 0x26, 1, 0x80])
}
#[test]
fn primary_identity_and_original_hash_are_preserved() {
    let bytes = ordinary();
    let header = inspect(&bytes, BUDGET).expect("header");
    assert_eq!(
        (
            header.width,
            header.height,
            header.bit_depth,
            header.orientation
        ),
        (16, 8, 8, 1)
    );
    assert_eq!(header.source_asset, vw_model::AssetId::hash(&bytes));
    assert!(header.icc.is_none());
}
#[test]
fn actual_sps_depth_overrides_a_false_eight_bit_hvcc_claim() {
    assert_eq!(
        inspect(&fixture(2, 16, false, &[0, 0, 0, 3, 0x26, 1, 0x80]), BUDGET),
        Err(Error::Depth)
    );
}
#[test]
fn concealed_coded_dimensions_are_refused_before_os_decode() {
    assert_eq!(
        inspect(
            &fixture(0, 65536, false, &[0, 0, 0, 3, 0x26, 1, 0x80]),
            BUDGET
        ),
        Err(Error::Limit)
    );
}
#[test]
fn rotated_original_is_not_silently_reinterpreted() {
    assert_eq!(
        inspect(&fixture(0, 16, true, &[0, 0, 0, 3, 0x26, 1, 0x80]), BUDGET),
        Err(Error::Orientation)
    );
}
#[test]
fn in_band_parameter_sets_cannot_bypass_admitted_depth() {
    assert_eq!(
        inspect(&fixture(0, 16, false, &[0, 0, 0, 3, 0x42, 1, 0x80]), BUDGET),
        Err(Error::Unsupported)
    );
}
#[test]
fn malformed_lengths_and_truncation_refuse() {
    let bytes = ordinary();
    for end in [0, 7, 31, bytes.len() - 1] {
        assert!(inspect(&bytes[..end], BUDGET).is_err());
    }
    let mut over = bytes;
    over[..4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(inspect(&over, BUDGET).is_err());
}
#[test]
fn duplicate_primary_boxes_are_not_last_value_wins() {
    let mut bytes = ordinary();
    let meta = bytes.windows(4).position(|v| v == b"meta").expect("meta") - 4;
    let extra = bx(b"pitm", &[0, 0, 0, 0, 0, 1]);
    let length =
        u32::from_be_bytes(bytes[meta..meta + 4].try_into().expect("size")) + extra.len() as u32;
    bytes[meta..meta + 4].copy_from_slice(&length.to_be_bytes());
    bytes.extend(extra);
    assert_eq!(inspect(&bytes, BUDGET), Err(Error::Invalid));
}
#[test]
fn insufficient_memory_refuses_even_a_tiny_source_before_os_work() {
    assert_eq!(inspect(&ordinary(), 1024), Err(Error::Limit));
}
#[test]
fn asset_hash_cannot_be_replaced_by_a_png_proxy() {
    let header = inspect(&ordinary(), BUDGET).expect("header");
    let mut other = header.clone();
    other.source_asset = vw_model::AssetId::hash(b"PNG proxy");
    let decoded = Decoded {
        header: other,
        rgba: vec![255; 16 * 8 * 4],
    };
    assert_eq!(decoded.validate(&header, BUDGET), Err(Error::Decode));
}
#[test]
fn alpha_or_incomplete_os_output_is_refused() {
    let header = inspect(&ordinary(), BUDGET).expect("header");
    let mut decoded = Decoded {
        header: header.clone(),
        rgba: vec![255; 16 * 8 * 4],
    };
    decoded.validate(&header, BUDGET).expect("opaque");
    decoded.rgba[3] = 254;
    assert_eq!(decoded.validate(&header, BUDGET), Err(Error::Decode));
    decoded.rgba.pop();
    assert_eq!(decoded.validate(&header, BUDGET), Err(Error::Decode));
}
