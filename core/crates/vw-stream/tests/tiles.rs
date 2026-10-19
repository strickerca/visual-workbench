#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;
use common::*;
use vw_stream::*;
const WORK: u64 = 128 * 1024 * 1024;

#[test]
fn tiled_keyframe_and_dirty_delta_preserve_every_unchanged_pixel() {
    let mut r = ring(600, 520);
    let first = frame(&mut r, 1, 600, 520, [16, 32, 64, 255]);
    let old = display(&first, None, true, None);
    let mut writer = r
        .begin_frame(identity(2, 600, 520), &Cancellation::default())
        .unwrap();
    writer.rgba_mut().copy_from_slice(first.rgba());
    let offset = (519 * 600 + 599) * 4;
    writer.rgba_mut()[offset..offset + 4].copy_from_slice(&[240, 120, 5, 255]);
    let second = writer.finish(&Cancellation::default()).unwrap();
    r.publish(second.clone()).unwrap();
    let packet = encode_tiles(
        &second,
        Some(&first),
        false,
        85,
        WORK,
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(packet.tiles.len(), 1);
    let rect = packet.tiles[0].rect.as_ref().unwrap();
    assert_eq!((rect.x, rect.y), (512, 512));
    let next = decode_tiles(&packet, Some(&old), WORK, &Cancellation::default()).unwrap();
    for y in 0..520 {
        for x in 0..600 {
            if x < 512 || y < 512 {
                let i = (y * 600 + x) * 4;
                assert_eq!(&old.rgba()[i..i + 4], &next.rgba()[i..i + 4]);
            }
        }
    }
    assert_eq!(next.identity().frame_id, 2);
}
#[test]
fn unchanged_numbered_delta_keeps_id_continuity_without_reencoding() {
    let mut r = ring(16, 16);
    let first = frame(&mut r, 1, 16, 16, [50, 80, 120, 255]);
    let old = display(&first, None, true, None);
    let second = frame(&mut r, 2, 16, 16, [50, 80, 120, 255]);
    let packet = encode_tiles(
        &second,
        Some(&first),
        false,
        85,
        WORK,
        &Cancellation::default(),
    )
    .unwrap();
    assert!(packet.tiles.is_empty());
    let next = decode_tiles(&packet, Some(&old), WORK, &Cancellation::default()).unwrap();
    assert_eq!(old.rgba(), next.rgba());
    assert_eq!(next.identity().frame_id, 2);
}
#[test]
fn dropped_delta_requires_keyframe_and_keyframe_can_jump() {
    let mut r = ring(16, 16);
    let first = frame(&mut r, 1, 16, 16, [1, 2, 3, 255]);
    let old = display(&first, None, true, None);
    let third = frame(&mut r, 3, 16, 16, [50, 80, 120, 255]);
    assert!(matches!(
        encode_tiles(
            &third,
            Some(&first),
            false,
            85,
            WORK,
            &Cancellation::default()
        ),
        Err(StreamError::KeyframeRequired)
    ));
    let key = encode_tiles(&third, None, true, 85, WORK, &Cancellation::default()).unwrap();
    assert_eq!(
        decode_tiles(&key, Some(&old), WORK, &Cancellation::default())
            .unwrap()
            .identity()
            .frame_id,
        3
    );
}
#[test]
fn missing_overlapping_outside_or_excess_keyframe_tiles_refuse_atomically() {
    let mut r = ring(600, 520);
    let f = frame(&mut r, 1, 600, 520, [10, 20, 30, 255]);
    let p = encode_tiles(&f, None, true, 85, WORK, &Cancellation::default()).unwrap();
    let mut missing = p.clone();
    missing.tiles.pop();
    assert!(decode_tiles(&missing, None, WORK, &Cancellation::default()).is_err());
    let mut overlap = p.clone();
    overlap.tiles.push(overlap.tiles[0].clone());
    assert!(decode_tiles(&overlap, None, WORK, &Cancellation::default()).is_err());
    let mut outside = p.clone();
    outside.tiles[0].rect.as_mut().unwrap().x = -1;
    assert!(decode_tiles(&outside, None, WORK, &Cancellation::default()).is_err());
    let mut many = p.clone();
    many.tiles = vec![p.tiles[0].clone(); MAX_TILES + 1];
    assert!(matches!(
        decode_tiles(&many, None, WORK, &Cancellation::default()),
        Err(StreamError::Limit)
    ));
}
#[test]
fn hostile_jpeg_header_metadata_dimensions_and_trailing_files_refuse() {
    let mut r = ring(16, 16);
    let f = frame(&mut r, 1, 16, 16, [10, 20, 30, 255]);
    let p = encode_tiles(&f, None, true, 85, WORK, &Cancellation::default()).unwrap();
    let mut wrong = p.clone();
    wrong.tiles[0].rect.as_mut().unwrap().w = 15;
    assert!(decode_tiles(&wrong, None, WORK, &Cancellation::default()).is_err());
    let mut tail = p.clone();
    tail.tiles[0].data.extend_from_slice(b"extra");
    assert!(decode_tiles(&tail, None, WORK, &Cancellation::default()).is_err());
    let mut metadata = p.clone();
    metadata.tiles[0]
        .data
        .splice(2..2, [0xff, 0xe2, 0, 6, b'I', b'C', b'C', 0]);
    assert!(decode_tiles(&metadata, None, WORK, &Cancellation::default()).is_err());
    let mut progressive = p.clone();
    let bytes = &mut progressive.tiles[0].data;
    let pos = bytes.windows(2).position(|p| p == [0xff, 0xc0]).unwrap();
    bytes[pos + 1] = 0xc2;
    assert!(decode_tiles(&progressive, None, WORK, &Cancellation::default()).is_err());
}
#[test]
fn memory_and_cancellation_refuse_before_decoder_staging() {
    let mut r = ring(16, 16);
    let f = frame(&mut r, 1, 16, 16, [10, 20, 30, 255]);
    assert!(matches!(
        encode_tiles(&f, None, true, 85, 1, &Cancellation::default()),
        Err(StreamError::Limit)
    ));
    let p = encode_tiles(&f, None, true, 85, WORK, &Cancellation::default()).unwrap();
    assert!(matches!(
        decode_tiles(&p, None, 1, &Cancellation::default()),
        Err(StreamError::Limit)
    ));
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(matches!(
        decode_tiles(&p, None, WORK, &cancel),
        Err(StreamError::Cancelled)
    ));
}
#[test]
fn negative_host_origin_and_dpi_never_scale_frame_pixels() {
    use vw_geom::{GeometryRevision, PixelPoint};
    let g = geometry(600, 520, 1);
    assert_eq!(
        g.mapping()
            .unwrap()
            .frame_to_host(PixelPoint { x: 599, y: 519 }, GeometryRevision(1))
            .unwrap(),
        PixelPoint { x: -1321, y: 263 }
    );
}
#[test]
fn adaptive_backpressure_demands_a_keyframe_without_unbounded_in_flight_work() {
    let mut p = StreamPacer::new(30).unwrap();
    assert_eq!(p.begin(100), Some(true));
    assert_eq!(p.begin(200), None);
    p.complete(50, false).unwrap();
    assert_eq!(p.quality(), 80);
    assert_eq!(p.begin(200), Some(true));
    p.complete(1, true).unwrap();
    assert_eq!(p.begin(201), None);
    assert_eq!(p.begin(240), Some(false));
    p.cancelled();
    assert_eq!(p.begin(300), Some(true));
    assert_eq!(p.requested_fps(), 30);
}
#[test]
fn repair_request_during_an_in_flight_encode_is_not_lost_on_completion() {
    let mut p = StreamPacer::new(30).unwrap();
    assert_eq!(p.begin(0), Some(true));
    p.complete(1, true).unwrap();
    assert_eq!(p.begin(40), Some(false));
    p.keyframe_required();
    p.complete(1, true).unwrap();
    assert_eq!(p.begin(80), Some(true));
}
