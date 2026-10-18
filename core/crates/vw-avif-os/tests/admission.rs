#![allow(clippy::unwrap_used, clippy::panic)]
use vw_avif_os::{Decoded, Error, inspect, is_avif};
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
    fn bytes(mut self) -> Vec<u8> {
        self.0.push(true);
        while self.0.len() % 8 != 0 {
            self.0.push(false);
        }
        self.0
            .chunks(8)
            .map(|p| p.iter().fold(0, |n, b| (n << 1) | u8::from(*b)))
            .collect()
    }
}
fn sequence(high_depth: bool, width: u32) -> Vec<u8> {
    let mut b = Bits::default();
    b.put(0, 3);
    b.put(1, 1);
    b.put(1, 1);
    b.put(0, 5);
    b.put(15, 4);
    b.put(2, 4);
    b.put(width - 1, 16);
    b.put(7, 3);
    b.put(0, 3);
    b.put(0, 1);
    b.put(3, 2);
    b.put(u32::from(high_depth), 1);
    b.put(0, 1);
    b.put(1, 1);
    b.put(1, 8);
    b.put(13, 8);
    b.put(6, 8);
    b.put(1, 1);
    b.put(0, 2);
    b.put(0, 1);
    b.put(0, 1);
    b.bytes()
}
fn stream(high: bool, width: u32, resized: bool) -> Vec<u8> {
    let seq = sequence(high, width);
    let mut v = vec![0x0a, seq.len() as u8];
    v.extend(seq);
    v.extend([0x32, 2, if resized { 0x20 } else { 0 }, 0x80]);
    v
}
/// Structural fixture only. The frame payload has no real AV1 tile entropy;
/// these cases establish refusal/admission, never installed-OS decoder fidelity.
fn fixture(media: &[u8], rotate: bool) -> Vec<u8> {
    let ftyp = bx(b"ftyp", b"avif\0\0\0\0mif1avif");
    let offset = (ftyp.len() + 8) as u32;
    let mut handler = vec![0; 8];
    handler.extend(b"pict");
    handler.extend([0; 13]);
    let mut entry = vec![2, 0, 0, 0, 0, 1, 0, 0];
    entry.extend(b"av01");
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
    properties.extend(bx(b"av1C", &[0x81, 0, 0x0c, 0]));
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
    let mut out = ftyp;
    out.extend(bx(b"mdat", media));
    out.extend(bx(b"meta", &meta));
    out
}
fn ordinary() -> Vec<u8> {
    fixture(&stream(false, 16, false), false)
}
#[test]
fn source_and_exact_color_declaration_survive() {
    let bytes = ordinary();
    let h = inspect(&bytes, BUDGET).unwrap();
    assert_eq!(
        (h.width, h.height, h.bit_depth, h.orientation),
        (16, 8, 8, 1)
    );
    assert_eq!(h.source_asset, vw_model::AssetId::hash(&bytes));
    assert_eq!(h.nclx.unwrap().matrix, 6);
}
#[test]
fn actual_bitstream_depth_cannot_hide_behind_av1c() {
    assert_eq!(
        inspect(&fixture(&stream(true, 16, false), false), BUDGET),
        Err(Error::Depth)
    );
}
#[test]
fn sequence_dimensions_cannot_hide_behind_ispe() {
    assert_eq!(
        inspect(&fixture(&stream(false, 8192, false), false), BUDGET),
        Err(Error::Invalid)
    );
}
#[test]
fn render_resize_and_container_rotation_refuse() {
    assert_eq!(
        inspect(&fixture(&stream(false, 16, true), false), BUDGET),
        Err(Error::Orientation)
    );
    assert_eq!(
        inspect(&fixture(&stream(false, 16, false), true), BUDGET),
        Err(Error::Orientation)
    );
}
#[test]
fn later_sequence_or_frame_cannot_reallocate_decoder() {
    let mut media = stream(false, 16, false);
    media.extend(stream(false, 8192, false));
    assert_eq!(
        inspect(&fixture(&media, false), BUDGET),
        Err(Error::Unsupported)
    );
}
#[test]
fn nclx_must_match_coded_matrix_and_range() {
    let mut bytes = ordinary();
    let at = bytes.windows(4).position(|v| v == b"nclx").unwrap();
    bytes[at + 9] = 1;
    assert_eq!(inspect(&bytes, BUDGET), Err(Error::Color));
}
#[test]
fn malformed_lengths_and_insufficient_memory_refuse_before_os_work() {
    let bytes = ordinary();
    for end in [0, 7, 31, bytes.len() - 1] {
        assert!(inspect(&bytes[..end], BUDGET).is_err());
    }
    assert_eq!(inspect(&bytes, 1024), Err(Error::Limit));
}
#[test]
fn sniffing_does_not_steal_heic_mif1_sources() {
    assert!(is_avif(&ordinary()));
    assert!(!is_avif(&bx(b"ftyp", b"mif1\0\0\0\0heic")));
    assert!(is_avif(&bx(b"ftyp", b"mif1\0\0\0\0avif")));
}
#[test]
fn callback_cannot_substitute_proxy_or_alpha() {
    let h = inspect(&ordinary(), BUDGET).unwrap();
    let mut result = Decoded {
        header: h.clone(),
        rgba: vec![255; 16 * 8 * 4],
    };
    result.validate(&h, BUDGET).unwrap();
    result.rgba[3] = 0;
    assert_eq!(result.validate(&h, BUDGET), Err(Error::Decode));
    result.rgba[3] = 255;
    result.header.source_asset = vw_model::AssetId::hash(b"proxy");
    assert_eq!(result.validate(&h, BUDGET), Err(Error::Decode));
}

#[test]
fn public_receipt_cannot_expand_the_restricted_adapter_capability() {
    let admitted = inspect(&ordinary(), BUDGET).unwrap();
    let mut header = admitted.clone();
    header.bit_depth = 16;
    assert_eq!(header.admit(BUDGET), Err(Error::Depth));
    header = admitted.clone();
    header.orientation = 6;
    assert_eq!(header.admit(BUDGET), Err(Error::Orientation));
    header = admitted;
    header.encoded_bytes = vw_avif_os::MAX_ENCODED as u64 + 1;
    assert_eq!(header.admit(BUDGET), Err(Error::Limit));
}

#[test]
fn callback_spare_allocation_is_part_of_the_decode_budget() {
    let header = inspect(&ordinary(), BUDGET).unwrap();
    let minimum = header.admit(BUDGET).unwrap();
    let mut rgba = Vec::with_capacity(4 * 1024 * 1024);
    rgba.resize(16 * 8 * 4, 255);
    let decoded = Decoded {
        header: header.clone(),
        rgba,
    };
    assert_eq!(
        decoded.validate(&header, minimum + 1024 * 1024),
        Err(Error::Limit)
    );
    decoded.validate(&header, BUDGET).unwrap();
}

#[test]
fn animated_brand_and_hdr_container_color_are_explicit_refusals() {
    let mut bytes = ordinary();
    bytes[8..12].copy_from_slice(b"avis");
    assert_eq!(inspect(&bytes, BUDGET), Err(Error::Unsupported));
    let mut bytes = ordinary();
    let at = bytes.windows(4).position(|v| v == b"nclx").unwrap();
    bytes[at + 6..at + 8].copy_from_slice(&16_u16.to_be_bytes());
    assert_eq!(inspect(&bytes, BUDGET), Err(Error::Color));
}
