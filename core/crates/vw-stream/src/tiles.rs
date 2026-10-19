use crate::{
    BudgetedBytes, Cancellation, FrameIdentity, MAX_TILE_BYTES, MAX_TILES, MemoryCharge,
    MemoryKind, OriginalFrame, Result, SessionBudget, StreamError, StreamGeometry, rgba_bytes,
};
use image::{ExtendedColorType, ImageDecoder};
use std::io::{Cursor, Write};
use vw_model::Id;
use vw_proto::{Message, v1};

pub const TILE_SIDE: u32 = 512;
const CODEC_FIXED: u64 = 8 * 1024 * 1024;
// Bounded tile Vec, rectangles, labels, geometry and ownership metadata.
const PACKET_OVERHEAD: u64 = 64 * 1024;
const DISPLAY_OVERHEAD: u64 = 4096;

/// Immutable MEDIA payload with its admission held through queueing/transport.
/// The transport DTO is available by reference; it cannot discard the charge by
/// extracting an owned packet. A separately serialized wire copy is charged too.
pub struct EncodedTiles {
    packet: v1::FrameTiles,
    charge: MemoryCharge,
}
impl EncodedTiles {
    /// Attach the producer's actual UTC capture clock before serialization.
    /// Connection generation/delivery authority remains zero for host admission.
    /// A timestamp is metadata and supplies no capture or input permission.
    pub fn with_captured_at_ms(mut self, captured_at_ms: i64) -> Result<Self> {
        if !(1..=253_402_300_799_999).contains(&captured_at_ms) {
            return Err(StreamError::Invalid);
        }
        self.packet.captured_at_ms = captured_at_ms;
        Ok(self)
    }
    pub fn packet(&self) -> &v1::FrameTiles {
        &self.packet
    }
    pub fn charge(&self) -> &MemoryCharge {
        &self.charge
    }
    pub fn into_transfer(self) -> Self {
        Self {
            packet: self.packet,
            charge: self.charge.reclassify(MemoryKind::Transfer),
        }
    }
    /// Serialize a distinct wire allocation only after a second reservation.
    /// The DTO's charge remains held while this physical copy exists.
    pub fn to_transfer_bytes(
        &self,
        budget: &SessionBudget,
        cancel: &Cancellation,
    ) -> Result<BudgetedBytes> {
        if !self.charge.belongs_to(budget) {
            return Err(StreamError::Invalid);
        }
        let len = self.packet.encoded_len();
        if len as u64 > MAX_TILE_BYTES as u64 + PACKET_OVERHEAD {
            return Err(StreamError::Limit);
        }
        let mut wire = BudgetedBytes::allocate(budget, MemoryKind::Transfer, len, cancel)?;
        self.packet
            .encode(&mut wire.bytes_mut())
            .map_err(|_| StreamError::Invalid)?;
        cancel.check()?;
        Ok(wire)
    }
    /// Reserve before the transport parser allocates an inbound DTO. Keep the
    /// reservation alive through parsing and adopt its resulting bounded packet.
    pub fn reserve(budget: &SessionBudget, cancel: &Cancellation) -> Result<TilePacketReservation> {
        cancel.check()?;
        Ok(TilePacketReservation {
            charge: budget.reserve(
                MemoryKind::EncodedTiles,
                MAX_TILE_BYTES as u64 + PACKET_OVERHEAD,
            )?,
        })
    }
}
pub struct TilePacketReservation {
    charge: MemoryCharge,
}
impl TilePacketReservation {
    pub fn adopt(self, packet: v1::FrameTiles, cancel: &Cancellation) -> Result<EncodedTiles> {
        cancel.check()?;
        if packet.tiles.len() > MAX_TILES || packet.tiles.capacity() > MAX_TILES {
            return Err(StreamError::Limit);
        }
        let mut retained =
            packet.tiles.capacity() as u64 * std::mem::size_of::<v1::TileImage>() as u64;
        retained = retained
            .checked_add(
                packet
                    .capture_session_id
                    .as_ref()
                    .map_or(0, |id| id.value.capacity()) as u64,
            )
            .ok_or(StreamError::Limit)?;
        if let Some(g) = &packet.geometry {
            retained = retained
                .checked_add(g.source_kind.capacity() as u64)
                .and_then(|n| n.checked_add(g.monitor_id.capacity() as u64))
                .ok_or(StreamError::Limit)?;
        }
        let mut encoded = 0u64;
        for tile in &packet.tiles {
            encoded = encoded
                .checked_add(tile.data.capacity() as u64)
                .ok_or(StreamError::Limit)?;
            retained = retained
                .checked_add(tile.format.capacity() as u64)
                .ok_or(StreamError::Limit)?;
        }
        if encoded > MAX_TILE_BYTES as u64
            || retained
                .checked_add(encoded)
                .is_none_or(|n| n > self.charge.bytes())
        {
            return Err(StreamError::Limit);
        }
        Ok(EncodedTiles {
            packet,
            charge: self.charge,
        })
    }
}

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

fn grid_count(width: u32, height: u32) -> Result<usize> {
    rgba_bytes(width, height)?;
    let count = width.div_ceil(TILE_SIDE) as usize * height.div_ceil(TILE_SIDE) as usize;
    if count > MAX_TILES {
        return Err(StreamError::Limit);
    }
    Ok(count)
}
fn grid(width: u32, height: u32) -> Result<Vec<TileRect>> {
    let count = grid_count(width, height)?;
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
    grid_count(width, height)?;
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
/// This unbudgeted codec utility is for isolated use; production callers use
/// encode_tiles_budgeted to retain the output's shared session admission.
pub fn encode_tiles(
    current: &OriginalFrame,
    previous: Option<&OriginalFrame>,
    keyframe: bool,
    quality: u8,
    workspace: u64,
    cancel: &Cancellation,
) -> Result<v1::FrameTiles> {
    encode_tiles_inner(current, previous, keyframe, quality, workspace, cancel)
}

/// Production encoder: all input originals must belong to this same endpoint
/// pool, and retained output plus transient codec allocations are admitted first.
pub fn encode_tiles_budgeted(
    current: &OriginalFrame,
    previous: Option<&OriginalFrame>,
    keyframe: bool,
    quality: u8,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<EncodedTiles> {
    cancel.check()?;
    if !current.belongs_to(budget) || previous.is_some_and(|old| !old.belongs_to(budget)) {
        return Err(StreamError::Invalid);
    }
    let g = &current.identity().geometry;
    let workspace = encode_workspace(g.width(), g.height())?;
    let _codec = budget.reserve(MemoryKind::CodecWorkspace, workspace)?;
    let reservation = EncodedTiles::reserve(budget, cancel)?;
    let packet = encode_tiles_inner(current, previous, keyframe, quality, workspace, cancel)?;
    reservation.adopt(packet, cancel)
}

fn encode_tiles_inner(
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
            for p in current.rgba()[start..start + rect.width as usize * 4]
                .as_chunks::<4>()
                .0
            {
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
        delivery_sequence: 0,
        source_generation: 0,
        captured_at_ms: 0,
        capture_session_id: Some(id.session.to_proto()),
        frame_id: id.frame_id,
        geometry: Some(id.geometry.wire().clone()),
        width,
        height,
        keyframe,
        tiles,
        timestamp_ns: id.timestamp_ns,
        last_input_seq_applied: 0,
    })
}

/// Immutable reconstructable frame. A UI must explicitly report actual
/// presentation before FreezeMachine will pin it; decode is not presentation.
pub struct DisplayFrame {
    identity: FrameIdentity,
    rgba: Vec<u8>,
    charge: Option<MemoryCharge>,
}
impl std::fmt::Debug for DisplayFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisplayFrame")
            .field("identity", &self.identity)
            .field("charged_bytes", &self.charged_bytes())
            .finish_non_exhaustive()
    }
}
impl DisplayFrame {
    pub fn identity(&self) -> &FrameIdentity {
        &self.identity
    }
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
    pub fn belongs_to(&self, budget: &SessionBudget) -> bool {
        self.charge
            .as_ref()
            .is_some_and(|charge| charge.belongs_to(budget))
    }
    pub fn charged_bytes(&self) -> u64 {
        self.charge.as_ref().map_or(0, MemoryCharge::bytes)
    }
    pub fn copy_into_transfer(
        &self,
        budget: &SessionBudget,
        cancel: &Cancellation,
    ) -> Result<BudgetedBytes> {
        if !self.belongs_to(budget) {
            return Err(StreamError::Invalid);
        }
        BudgetedBytes::copy_from(budget, MemoryKind::Transfer, &self.rgba, cancel)
    }
}

/// Strong failure atomicity: malformed last tiles never partially update the
/// existing frame. At most old+new RGBA plus one bounded JPEG decoder coexist.
/// Isolated utility; production callers use decode_tiles_budgeted.
pub fn decode_tiles(
    packet: &v1::FrameTiles,
    previous: Option<&DisplayFrame>,
    budget: u64,
    cancel: &Cancellation,
) -> Result<DisplayFrame> {
    decode_tiles_inner(packet, previous, budget, None, cancel)
}

pub fn decode_tiles_budgeted(
    packet: &EncodedTiles,
    previous: Option<&DisplayFrame>,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<DisplayFrame> {
    cancel.check()?;
    if !packet.charge.belongs_to(budget) || previous.is_some_and(|old| !old.belongs_to(budget)) {
        return Err(StreamError::Invalid);
    }
    // Covers bounded geometry/header/rectangle admission before any parser-side
    // allocation. The inner admission adds only the remaining codec workspace.
    let _admission = budget.reserve(MemoryKind::CodecWorkspace, CODEC_FIXED)?;
    decode_tiles_inner(
        packet.packet(),
        previous,
        budget.limit_bytes(),
        Some(budget),
        cancel,
    )
}

fn decode_tiles_inner(
    packet: &v1::FrameTiles,
    previous: Option<&DisplayFrame>,
    budget: u64,
    session: Option<&SessionBudget>,
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
    let _codec = session
        .map(|pool| {
            pool.reserve(
                MemoryKind::CodecWorkspace,
                peak - retained - bytes as u64 - CODEC_FIXED,
            )
        })
        .transpose()?;
    let charge = session
        .map(|pool| pool.reserve(MemoryKind::DisplayFrame, bytes as u64 + DISPLAY_OVERHEAD))
        .transpose()?;
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
            for (p, out) in row.as_chunks::<3>().0.iter().zip(
                rgba[start..start + rect.width as usize * 4]
                    .as_chunks_mut::<4>()
                    .0,
            ) {
                out[..3].copy_from_slice(p);
                out[3] = 255;
            }
        }
    }
    cancel.check()?;
    Ok(DisplayFrame {
        identity,
        rgba,
        charge,
    })
}

/// Nonallocating subset admission before image's JPEG header parser. Our wire
/// encoder emits baseline three-component 8-bit JFIF; progressive/ICC/Exif,
/// extra scans, giant tables, arbitrary metadata and trailing files are refused.
pub fn validate_jpeg_tile_header(bytes: &[u8], width: u32, height: u32) -> Result<()> {
    if width == 0 || height == 0 || width > TILE_SIDE || height > TILE_SIDE {
        return Err(StreamError::Limit);
    }
    jpeg_header(bytes, width, height)
}

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
                for (i, component) in payload[6..].as_chunks::<3>().0.iter().enumerate() {
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
