//! Hostile-input checks for the capture-data structure check that runs before
//! any tile list is allocated. Whatever bytes arrive from the other device, it
//! must return an answer: never panic, hang or read past the input.
#![allow(clippy::unwrap_used)]
use vw_proto::capture_admission::{MAX_CAPTURE_TILES, frame_tiles, media_envelope};

fn varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 128 {
        out.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    out.push(value as u8);
}
fn scalar(number: u32, value: u64, out: &mut Vec<u8>) {
    varint(u64::from(number) << 3, out);
    varint(value, out);
}
fn bytes(number: u32, data: &[u8], out: &mut Vec<u8>) {
    varint(u64::from(number) << 3 | 2, out);
    varint(data.len() as u64, out);
    out.extend_from_slice(data);
}
fn uuid() -> Vec<u8> {
    let mut out = Vec::new();
    bytes(1, &[7; 16], &mut out);
    out
}
fn rect() -> Vec<u8> {
    let mut out = Vec::new();
    for number in 1..=4 {
        scalar(number, 100 + u64::from(number), &mut out);
    }
    out
}
fn geometry() -> Vec<u8> {
    let mut out = Vec::new();
    bytes(1, &[1; 16], &mut out);
    scalar(2, 3, &mut out);
    bytes(3, b"display", &mut out);
    bytes(4, &rect(), &mut out);
    varint(5 << 3 | 1, &mut out);
    out.extend_from_slice(&1.5f64.to_le_bytes());
    scalar(6, 1, &mut out);
    scalar(7, 2, &mut out);
    out
}
fn tile(payload: usize) -> Vec<u8> {
    let mut out = Vec::new();
    bytes(1, &rect(), &mut out);
    bytes(2, &[9; 16], &mut out);
    bytes(3, &vec![0x5a; payload], &mut out);
    out
}
fn tiles(count: usize) -> Vec<u8> {
    let mut out = Vec::new();
    bytes(1, &uuid(), &mut out);
    scalar(2, 1, &mut out);
    bytes(3, &geometry(), &mut out);
    for number in [4, 5, 6] {
        scalar(number, 9, &mut out);
    }
    for index in 0..count {
        bytes(7, &tile(8 + index % 5), &mut out);
    }
    for number in [8, 9, 10, 11, 12] {
        scalar(number, 4, &mut out);
    }
    out
}
fn envelope(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    scalar(1, 1, &mut out);
    scalar(2, 77, &mut out);
    bytes(3, &uuid(), &mut out);
    bytes(51, body, &mut out);
    out
}

#[test]
fn well_formed_capture_data_is_admitted_and_the_tile_limit_holds() {
    assert_eq!(frame_tiles(&tiles(3)), Ok(()));
    assert_eq!(frame_tiles(&tiles(MAX_CAPTURE_TILES)), Ok(()));
    assert_eq!(
        frame_tiles(&tiles(MAX_CAPTURE_TILES + 1)),
        Err("capture tile count")
    );
    assert_eq!(media_envelope(&envelope(&tiles(3))), Ok(()));
    // Two bodies (tiles and video) in one envelope are refused.
    let mut twice = Vec::new();
    scalar(1, 1, &mut twice);
    bytes(51, &tiles(1), &mut twice);
    bytes(52, &[], &mut twice);
    assert_eq!(media_envelope(&twice), Err("capture wire body"));
}

/// Deterministic so a failure reproduces.
#[test]
fn mutated_capture_data_never_panics_and_always_returns() {
    let seeds = [envelope(&tiles(3)), tiles(5), envelope(&tiles(0))];
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let (mut accepted, mut refused) = (0u32, 0u32);
    for round in 0..20_000u32 {
        let mut data = seeds[round as usize % seeds.len()].clone();
        for _ in 0..1 + next() % 4 {
            let at = (next() as usize) % data.len();
            match next() % 6 {
                0 => data[at] ^= 1 << (next() % 8),
                1 => data[at] = next() as u8,
                // Lengths and tags are varints: force the continuation bit, or a huge value.
                2 => data[at] |= 128,
                3 => data.splice(at..at, [0xff; 10]).for_each(drop),
                4 => data.truncate(at.max(1)),
                _ => data.extend(std::iter::repeat_n(next() as u8, (next() % 32) as usize)),
            }
            if data.is_empty() {
                data.push(0);
            }
        }
        for result in [frame_tiles(&data), media_envelope(&data)] {
            match result {
                Ok(()) => accepted += 1,
                Err(reason) => {
                    assert!(reason.starts_with("capture "));
                    refused += 1;
                }
            }
        }
    }
    assert!(
        refused > 30_000,
        "mutations should mostly be refused: {refused}"
    );
    assert!(
        accepted > 0,
        "some harmless mutations should still be admitted"
    );
}
