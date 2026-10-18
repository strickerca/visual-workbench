use crate::{
    Cancellation, FrameIdentity, MAX_TILE_BYTES, MAX_TILES, OriginalFrame, Result, StreamError,
    StreamGeometry, rgba_bytes,
};
use image::{ExtendedColorType, ImageDecoder};
use std::io::{Cursor, Write};
use vw_model::Id;
use vw_proto::v1;

pub const TILE_SIDE: u32 = 512;
const CODEC_FIXED: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl TileRect {
    fn validate(self, width: u32, height: u32) -> Result<()> {
        if self.width == 0
            || self.height == 0
            || self.width > TILE_SIDE
            || self.height > TILE_SIDE
            || self.x >= width
            || self.y >= height
            || self.x.checked_add(self.width).is_none_or(|n| n > width)
            || self.y.checked_add(self.height).is_none_or(|n| n > height)
        {
            return Err(StreamError::Invalid);
        }
        Ok(())
    }
    fn overlaps(self, b: Self) -> bool {
        self.x < b.x + b.width
            && b.x < self.x + self.width
            && self.y < b.y + b.height
            && b.y < self.y + self.height
    }
}

fn grid(width: u32, height: u32) -> Result<Vec<TileRect>> {
    rgba_bytes(width, height)?;
    let count = width.div_ceil(TILE_SIDE) as usize * height.div_ceil(TILE_SIDE) as usize;
    if count > MAX_TILES {
        return Err(StreamError::Limit);
    }
    let mut tiles = Vec::with_capacity(count);
    for y in (0..height).step_by(TILE_SIDE as usize) {
        for x in (0..width).step_by(TILE_SIDE as usize) {
            tiles.push(TileRect {
                x,
                y,
                width: TILE_SIDE.min(width - x),
                height: TILE_SIDE.min(height - y),
            });
        }
    }
    Ok(tiles)
}

/// Codec workspace is additional to immutable ring leases/caller bytes. It
/// includes worst-case fixed tile decode/encode scratch, RGB copy, full MEDIA
/// output capacity and library temporaries. No CPU/GPU driver peak is implied.
pub fn encode_workspace(width: u32, height: u32) -> Result<u64> {
    grid(width, height)?;
    Ok(CODEC_FIXED
        + u64::from(TILE_SIDE.min(width)) * u64::from(TILE_SIDE.min(height)) * 64
        + 2 * MAX_TILE_BYTES as u64)
}

struct Capped<'a> {
    bytes: Vec<u8>,
    limit: usize,
    cancel: &'a Cancellation,
}
impl Write for Capped<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.cancel
            .check()
            .map_err(|_| std::io::Error::other("cancelled"))?;
        let end = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|n| *n <= self.limit)
            .ok_or_else(|| std::io::Error::other("encoded bound"))?;
        if end > self.bytes.capacity() {
            return Err(std::io::Error::other("allocation admission"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A delta is relative to the immediately prior *captured* frame. If MEDIA
/// backpressure discards it, the adapter must request keyframe=true next time.
/// Unchanged capture frames still produce an empty numbered delta, preserving
/// exact ID continuity; capture IDs are never renumbered after a network drop.
pub fn encode_tiles(
    current: &OriginalFrame,
    previous: Option<&OriginalFrame>,
    keyframe: bool,
    quality: u8,
    workspace: u64,
    cancel: &Cancellation,
) -> Result<v1::FrameTiles> {
    cancel.check()?;
    if !(40..=95).contains(&quality) {
        return Err(StreamError::Invalid);
    }
    let id = current.identity();
    let width = id.geometry.width();
    let height = id.geometry.height();
    if encode_workspace(width, height)? > workspace {
        return Err(StreamError::Limit);
    }
    if !keyframe {
        let previous = previous.ok_or(StreamError::KeyframeRequired)?;
        if previous.identity().session != id.session
            || !previous.identity().geometry.same_source(&id.geometry)
            || previous.identity().frame_id.checked_add(1) != Some(id.frame_id)
        {
            return Err(StreamError::KeyframeRequired);
        }
    }
    let mut tiles = Vec::with_capacity(MAX_TILES);
    let mut encoded = 0usize;
    for rect in grid(width, height)? {
        cancel.check()?;
        let stride = width as usize * 4;
        let changed = keyframe
            || previous.is_none_or(|before| {
                (rect.y..rect.y + rect.height).any(|y| {
                    let start = y as usize * stride + rect.x as usize * 4;
                    let end = start + rect.width as usize * 4;
                    current.rgba()[start..end] != before.rgba()[start..end]
                })
            });
        if !changed {
            continue;
        }
        let mut rgb = Vec::new();
        rgb.try_reserve_exact(rect.width as usize * rect.height as usize * 3)
            .map_err(|_| StreamError::Limit)?;
        for y in rect.y..rect.y + rect.height {
            cancel.check()?;
            let start = y as usize * stride + rect.x as usize * 4;
            for p in current.rgba()[start..start + rect.width as usize * 4].chunks_exact(4) {
                rgb.extend_from_slice(&p[..3]);
            }
        }
        let remaining = MAX_TILE_BYTES - encoded;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(remaining)
            .map_err(|_| StreamError::Limit)?;
        let mut writer = Capped {
            bytes,
            limit: remaining,
            cancel,
        };
        let result = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, quality)
            .encode(&rgb, rect.width, rect.height, ExtendedColorType::Rgb8);
        cancel.check()?;
        result.map_err(|_| StreamError::Codec)?;
        encoded = encoded
            .checked_add(writer.bytes.len())
            .ok_or(StreamError::Limit)?;
        // Do not retain a maximum-capacity allocation for every small tile.
        // Both old/new storage during compaction were admitted above.
        let data = writer.bytes.into_boxed_slice().into_vec();
        tiles.push(v1::TileImage {
            rect: Some(v1::RectI {
                x: rect.x as i32,
                y: rect.y as i32,
                w: rect.width as i32,
                h: rect.height as i32,
            }),
            format: "jpeg".into(),
            data,
        });
    }
    Ok(v1::FrameTiles {
        capture_session_id: Some(id.session.to_proto()),
        frame_id: id.frame_id,
        geometry: Some(id.geometry.wire().clone()),
        width,
        height,
        keyframe,
        tiles,
        timestamp_ns: id.timestamp_ns,
        last_input_seq_applied: 0,
        ..Default::default()
    })
}

/// Immutable reconstructable frame. A UI must explicitly report actual
/// presentation before FreezeMachine will pin it; decode is not presentation.
#[derive(Debug)]
pub struct DisplayFrame {
    identity: FrameIdentity,
    rgba: Vec<u8>,
}
impl DisplayFrame {
    pub fn identity(&self) -> &FrameIdentity {
        &self.identity
    }
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

/// Strong failure atomicity: malformed last tiles never partially update the
/// existing frame. At most old+new RGBA plus one bounded JPEG decoder coexist.
pub fn decode_tiles(
    packet: &v1::FrameTiles,
    previous: Option<&DisplayFrame>,
    budget: u64,
    cancel: &Cancellation,
) -> Result<DisplayFrame> {
    cancel.check()?;
    if packet.tiles.len() > MAX_TILES {
        return Err(StreamError::Limit);
    }
    let geometry = StreamGeometry::new(
        packet.geometry.as_ref().ok_or(StreamError::Invalid)?,
        packet.width,
        packet.height,
    )?;
    let identity = FrameIdentity {
        session: Id::from_proto(packet.capture_session_id.as_ref())
            .map_err(|_| StreamError::Invalid)?,
        frame_id: packet.frame_id,
        geometry,
        timestamp_ns: packet.timestamp_ns,
    };
    identity.validate()?;
    if !packet.keyframe {
        let old = previous.ok_or(StreamError::KeyframeRequired)?;
        if old.identity.session != identity.session
            || !old.identity.geometry.same_source(&identity.geometry)
            || old.identity.frame_id.checked_add(1) != Some(identity.frame_id)
            || old.identity.timestamp_ns >= identity.timestamp_ns
        {
            return Err(StreamError::KeyframeRequired);
        }
    } else if previous.is_some_and(|old| {
        old.identity.session == identity.session
            && (old.identity.frame_id >= identity.frame_id
                || old.identity.timestamp_ns >= identity.timestamp_ns
                || !old.identity.geometry.same_source(&identity.geometry))
    }) {
        return Err(StreamError::Stale);
    }
    let mut rectangles = Vec::with_capacity(packet.tiles.len());
    let mut encoded = 0u64;
    let mut area = 0u64;
    let mut largest = 0u64;
    // All structural/header admission precedes full-frame or decoder allocation.
    for tile in &packet.tiles {
        cancel.check()?;
        let r = tile.rect.as_ref().ok_or(StreamError::Invalid)?;
        if r.x < 0 || r.y < 0 || r.w <= 0 || r.h <= 0 || tile.format != "jpeg" {
            return Err(StreamError::Invalid);
        }
        let rect = TileRect {
            x: r.x as u32,
            y: r.y as u32,
            width: r.w as u32,
            height: r.h as u32,
        };
        rect.validate(packet.width, packet.height)?;
        if rectangles
            .iter()
            .any(|before: &TileRect| before.overlaps(rect))
        {
            return Err(StreamError::Invalid);
        }
        encoded = encoded
            .checked_add(tile.data.len() as u64)
            .ok_or(StreamError::Limit)?;
        if encoded > MAX_TILE_BYTES as u64 {
            return Err(StreamError::Limit);
        }
        jpeg_header(&tile.data, rect.width, rect.height)?;
        let pixels = u64::from(rect.width) * u64::from(rect.height);
        area += pixels;
        largest = largest.max(pixels);
        rectangles.push(rect);
    }
    if packet.keyframe && area != u64::from(packet.width) * u64::from(packet.height) {
        return Err(StreamError::Invalid);
    }
    let bytes = rgba_bytes(packet.width, packet.height)?;
    let retained = previous.map_or(0, |old| old.rgba.len() as u64);
    let peak = retained
        .checked_add(bytes as u64)
        .and_then(|n| n.checked_add(largest * 64))
        .and_then(|n| n.checked_add(encoded * 3))
        .and_then(|n| n.checked_add(CODEC_FIXED))
        .ok_or(StreamError::Limit)?;
    if peak > budget {
        return Err(StreamError::Limit);
    }
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(bytes)
        .map_err(|_| StreamError::Limit)?;
    if packet.keyframe {
        rgba.resize(bytes, 0)
    } else {
        rgba.extend_from_slice(&previous.ok_or(StreamError::KeyframeRequired)?.rgba)
    }
    for (tile, rect) in packet.tiles.iter().zip(rectangles) {
        cancel.check()?;
        let mut decoder = image::codecs::jpeg::JpegDecoder::new(Cursor::new(&tile.data))
            .map_err(|_| StreamError::Codec)?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(TILE_SIDE);
        limits.max_image_height = Some(TILE_SIDE);
        limits.max_alloc = Some(largest * 64 + CODEC_FIXED);
        decoder.set_limits(limits).map_err(|_| StreamError::Limit)?;
        if decoder.dimensions() != (rect.width, rect.height)
            || decoder.color_type() != image::ColorType::Rgb8
        {
            return Err(StreamError::Codec);
        }
        let mut rgb = vec![0u8; rect.width as usize * rect.height as usize * 3];
        decoder
            .read_image(&mut rgb)
            .map_err(|_| StreamError::Codec)?;
        cancel.check()?;
        for (row, y) in rgb
            .chunks_exact(rect.width as usize * 3)
            .zip(rect.y..rect.y + rect.height)
        {
            let start = (y as usize * packet.width as usize + rect.x as usize) * 4;
            for (p, out) in row
                .chunks_exact(3)
                .zip(rgba[start..start + rect.width as usize * 4].chunks_exact_mut(4))
            {
                out[..3].copy_from_slice(p);
                out[3] = 255;
            }
        }
    }
    cancel.check()?;
    Ok(DisplayFrame { identity, rgba })
}

/// Nonallocating subset admission before image's JPEG header parser. Our wire
/// encoder emits baseline three-component 8-bit JFIF; progressive/ICC/Exif,
/// extra scans, giant tables, arbitrary metadata and trailing files are refused.
fn jpeg_header(bytes: &[u8], width: u32, height: u32) -> Result<()> {
    if bytes.len() < 4 || bytes.len() > MAX_TILE_BYTES || bytes[..2] != [0xff, 0xd8] {
        return Err(StreamError::Codec);
    }
    let mut pos = 2usize;
    let mut header_bytes = 0usize;
    let mut frame = false;
    loop {
        if bytes.get(pos) != Some(&0xff) {
            return Err(StreamError::Codec);
        }
        pos += 1;
        while bytes.get(pos) == Some(&0xff) {
            pos += 1;
        }
        let marker = *bytes.get(pos).ok_or(StreamError::Codec)?;
        pos += 1;
        let length = u16::from_be_bytes(
            bytes
                .get(pos..pos + 2)
                .ok_or(StreamError::Codec)?
                .try_into()
                .map_err(|_| StreamError::Codec)?,
        ) as usize;
        if length < 2 {
            return Err(StreamError::Codec);
        }
        let end = pos.checked_add(length).ok_or(StreamError::Codec)?;
        let payload = bytes.get(pos + 2..end).ok_or(StreamError::Codec)?;
        header_bytes = header_bytes
            .checked_add(length + 2)
            .ok_or(StreamError::Limit)?;
        if header_bytes > 65536 {
            return Err(StreamError::Limit);
        }
        match marker {
            0xe0 => {
                if payload.len() > 32 || !payload.starts_with(b"JFIF\0") {
                    return Err(StreamError::Codec);
                }
            }
            0xdb | 0xc4 => {}
            0xdd => {
                if payload.len() != 2 {
                    return Err(StreamError::Codec);
                }
            }
            0xc0 => {
                if frame
                    || payload.len() != 15
                    || payload[0] != 8
                    || payload[5] != 3
                    || u16::from_be_bytes([payload[1], payload[2]]) as u32 != height
                    || u16::from_be_bytes([payload[3], payload[4]]) as u32 != width
                {
                    return Err(StreamError::Codec);
                }
                for (i, component) in payload[6..].chunks_exact(3).enumerate() {
                    if component[0] != i as u8 + 1
                        || !(1..=2).contains(&(component[1] >> 4))
                        || !(1..=2).contains(&(component[1] & 15))
                        || component[2] > 3
                    {
                        return Err(StreamError::Codec);
                    }
                }
                frame = true;
            }
            0xda => {
                if !frame || payload.len() != 10 || payload[0] != 3 || payload[7..] != [0, 63, 0] {
                    return Err(StreamError::Codec);
                }
                pos = end;
                while pos < bytes.len() {
                    if bytes[pos] != 0xff {
                        pos += 1;
                        continue;
                    }
                    pos += 1;
                    while bytes.get(pos) == Some(&0xff) {
                        pos += 1;
                    }
                    match bytes.get(pos) {
                        Some(0x00 | 0xd0..=0xd7) => pos += 1,
                        Some(0xd9) if pos + 1 == bytes.len() => return Ok(()),
                        _ => return Err(StreamError::Codec),
                    }
                }
                return Err(StreamError::Codec);
            }
            _ => return Err(StreamError::Codec),
        }
        pos = end;
    }
}
