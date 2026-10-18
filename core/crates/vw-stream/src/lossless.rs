use crate::{
    AcceptedRegion, BudgetedBytes, Cancellation, FrameIdentity, MemoryCharge, MemoryKind,
    OriginalFrame, Result, SessionBudget, StreamError, StreamGeometry, TileRect, rgba_bytes,
};
use std::io::{Cursor, Write};
use vw_model::AssetId;

pub const MAX_LOSSLESS_BYTES: usize = 64 * 1024 * 1024;
const PNG_WORK: u64 = 16 * 1024 * 1024;
const ORIGINAL_OVERHEAD: u64 = 4096;

/// Exact PNG plus its immutable content-address receipt. Publication to a store
/// or BLOB transport is deliberately performed by the existing storage adapter.
pub struct EncodedOriginal {
    pub identity: FrameIdentity,
    pub asset_id: AssetId,
    bytes: Vec<u8>,
}
impl EncodedOriginal {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// A charged transport PNG receipt. This remains a transport DTO, not a
/// metadata-complete user export. Moving to BLOB ownership keeps its admission.
pub struct BudgetedEncodedOriginal {
    original: EncodedOriginal,
    charge: MemoryCharge,
}
impl BudgetedEncodedOriginal {
    pub fn identity(&self) -> &FrameIdentity {
        &self.original.identity
    }
    pub fn asset_id(&self) -> &AssetId {
        &self.original.asset_id
    }
    pub fn bytes(&self) -> &[u8] {
        self.original.bytes()
    }
    pub fn charge(&self) -> &MemoryCharge {
        &self.charge
    }
    pub fn into_transfer(self) -> Result<BudgetedBytes> {
        BudgetedBytes::from_parts(
            self.original.bytes,
            self.charge.reclassify(MemoryKind::Transfer),
        )
    }
    pub fn verify(
        &self,
        budget: &SessionBudget,
        cancel: &Cancellation,
    ) -> Result<VerifiedOriginal> {
        if !self.charge.belongs_to(budget) {
            return Err(StreamError::Invalid);
        }
        verify_original_inner(
            self.identity(),
            self.asset_id(),
            self.bytes(),
            budget.limit_bytes(),
            Some(budget),
            cancel,
        )
    }
}
/// Only a bounded, hash-checked decoder can construct this value. It binds the
/// exact requested frame/geometry to admitted lossless SDR RGBA8 pixels.
pub struct VerifiedOriginal {
    identity: FrameIdentity,
    asset_id: AssetId,
    rgba: Vec<u8>,
    charge: Option<MemoryCharge>,
}
impl VerifiedOriginal {
    pub fn identity(&self) -> &FrameIdentity {
        &self.identity
    }
    pub fn asset_id(&self) -> &AssetId {
        &self.asset_id
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
impl std::fmt::Debug for VerifiedOriginal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedOriginal")
            .field("identity", &self.identity)
            .field("asset_id", &self.asset_id)
            .field("charged_bytes", &self.charged_bytes())
            .finish_non_exhaustive()
    }
}
struct Output<'a> {
    bytes: Vec<u8>,
    limit: usize,
    cancel: &'a Cancellation,
}
impl Write for Output<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.cancel
            .check()
            .map_err(|_| std::io::Error::other("cancelled"))?;
        let next = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|n| *n <= self.limit)
            .ok_or_else(|| std::io::Error::other("limit"))?;
        if next > self.bytes.capacity() {
            return Err(std::io::Error::other("allocation admission"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
/// Isolated codec utility. Production originals use encode_original_budgeted.
pub fn encode_original(
    frame: &OriginalFrame,
    encoded_limit: usize,
    workspace: u64,
    cancel: &Cancellation,
) -> Result<EncodedOriginal> {
    let g = &frame.identity().geometry;
    encode(
        frame,
        TileRect {
            x: 0,
            y: 0,
            width: g.width(),
            height: g.height(),
        },
        frame.identity().clone(),
        encoded_limit,
        workspace,
        cancel,
    )
}

pub fn encode_original_budgeted(
    frame: &OriginalFrame,
    encoded_limit: usize,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<BudgetedEncodedOriginal> {
    let g = &frame.identity().geometry;
    encode_budgeted(
        frame,
        TileRect {
            x: 0,
            y: 0,
            width: g.width(),
            height: g.height(),
        },
        frame.identity().clone(),
        encoded_limit,
        budget,
        cancel,
    )
}
/// Crop the exact locally approved original, not another capture. Host-space
/// origin shifts by F edges, never by DPI; frame/session/time remain unchanged.
/// Isolated codec utility; production callers use encode_region_budgeted.
pub fn encode_region(
    region: &AcceptedRegion,
    encoded_limit: usize,
    workspace: u64,
    cancel: &Cancellation,
) -> Result<EncodedOriginal> {
    let frame = region.frame();
    let r = region.rectangle();
    let id = region_identity(frame, r)?;
    encode(frame, r, id, encoded_limit, workspace, cancel)
}

pub fn encode_region_budgeted(
    region: &AcceptedRegion,
    encoded_limit: usize,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<BudgetedEncodedOriginal> {
    let frame = region.frame();
    let r = region.rectangle();
    encode_budgeted(
        frame,
        r,
        region_identity(frame, r)?,
        encoded_limit,
        budget,
        cancel,
    )
}

fn region_identity(frame: &OriginalFrame, r: TileRect) -> Result<FrameIdentity> {
    let mut id = frame.identity().clone();
    let mut wire = id.geometry.wire().clone();
    let h = wire.client_rect_host.as_mut().ok_or(StreamError::Invalid)?;
    h.x = h.x.checked_add(r.x as i32).ok_or(StreamError::Invalid)?;
    h.y = h.y.checked_add(r.y as i32).ok_or(StreamError::Invalid)?;
    h.w = r.width as i32;
    h.h = r.height as i32;
    // Preserve the exact window/monitor identity while distinguishing the crop.
    wire.source_kind = "region".into();
    id.geometry = StreamGeometry::new(&wire, r.width, r.height)?;
    Ok(id)
}

fn encode_budgeted(
    frame: &OriginalFrame,
    r: TileRect,
    identity: FrameIdentity,
    encoded_limit: usize,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<BudgetedEncodedOriginal> {
    cancel.check()?;
    if !frame.belongs_to(budget) {
        return Err(StreamError::Invalid);
    }
    if encoded_limit == 0 || encoded_limit > MAX_LOSSLESS_BYTES {
        return Err(StreamError::Limit);
    }
    let work = PNG_WORK + u64::from(r.width) * 16;
    let _codec = budget.reserve(MemoryKind::CodecWorkspace, work)?;
    let charge = budget.reserve(
        MemoryKind::Transfer,
        encoded_limit as u64 + ORIGINAL_OVERHEAD,
    )?;
    let original = encode(
        frame,
        r,
        identity,
        encoded_limit,
        work + encoded_limit as u64,
        cancel,
    )?;
    Ok(BudgetedEncodedOriginal { original, charge })
}
fn encode(
    frame: &OriginalFrame,
    r: TileRect,
    identity: FrameIdentity,
    encoded_limit: usize,
    workspace: u64,
    cancel: &Cancellation,
) -> Result<EncodedOriginal> {
    cancel.check()?;
    identity.validate()?;
    if encoded_limit == 0
        || encoded_limit > MAX_LOSSLESS_BYTES
        || workspace < encoded_limit as u64 + PNG_WORK + u64::from(r.width) * 16
    {
        return Err(StreamError::Limit);
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(encoded_limit)
        .map_err(|_| StreamError::Limit)?;
    let mut output = Output {
        bytes,
        limit: encoded_limit,
        cancel,
    };
    let result = (|| -> std::result::Result<(), png::EncodingError> {
        let mut encoder = png::Encoder::new(&mut output, r.width, r.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        encoder.set_compression(png::Compression::Fast);
        encoder.set_filter(png::Filter::Sub);
        let mut writer = encoder.write_header()?;
        {
            let mut rows = writer.stream_writer()?;
            for y in r.y..r.y + r.height {
                cancel
                    .check()
                    .map_err(|_| std::io::Error::other("cancelled"))?;
                let start =
                    (y as usize * frame.identity().geometry.width() as usize + r.x as usize) * 4;
                rows.write_all(&frame.rgba()[start..start + r.width as usize * 4])?;
            }
            rows.finish()?;
        }
        writer.finish()
    })();
    cancel.check()?;
    result.map_err(|_| StreamError::Codec)?;
    Ok(EncodedOriginal {
        identity,
        asset_id: AssetId::hash(&output.bytes),
        bytes: output.bytes,
    })
}

/// Isolated codec utility. Production verification requires an already charged
/// PNG via verify_original_budgeted or BudgetedEncodedOriginal::verify.
pub fn verify_original(
    identity: &FrameIdentity,
    expected: &AssetId,
    encoded: &[u8],
    budget: u64,
    cancel: &Cancellation,
) -> Result<VerifiedOriginal> {
    verify_original_inner(identity, expected, encoded, budget, None, cancel)
}

/// The inbound PNG allocation must already have a reservation from this pool.
/// Hash/header checks and decode allocations cannot escape aggregate admission.
pub fn verify_original_budgeted(
    identity: &FrameIdentity,
    expected: &AssetId,
    encoded: &BudgetedBytes,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<VerifiedOriginal> {
    if !encoded.charge().belongs_to(budget) {
        return Err(StreamError::Invalid);
    }
    verify_original_inner(
        identity,
        expected,
        encoded.bytes(),
        budget.limit_bytes(),
        Some(budget),
        cancel,
    )
}

fn verify_original_inner(
    identity: &FrameIdentity,
    expected: &AssetId,
    encoded: &[u8],
    budget: u64,
    session: Option<&SessionBudget>,
    cancel: &Cancellation,
) -> Result<VerifiedOriginal> {
    cancel.check()?;
    identity.validate()?;
    let raw = rgba_bytes(identity.geometry.width(), identity.geometry.height())?;
    let peak = (encoded.len() as u64)
        .checked_add(raw as u64)
        .and_then(|n| n.checked_add(PNG_WORK))
        .ok_or(StreamError::Limit)?;
    if encoded.is_empty() || encoded.len() > MAX_LOSSLESS_BYTES || peak > budget {
        return Err(StreamError::Limit);
    }
    if AssetId::hash(encoded) != *expected {
        return Err(StreamError::Invalid);
    }
    png_subset(
        encoded,
        identity.geometry.width(),
        identity.geometry.height(),
        cancel,
    )?;
    let _codec = session
        .map(|pool| pool.reserve(MemoryKind::CodecWorkspace, PNG_WORK))
        .transpose()?;
    let charge = session
        .map(|pool| pool.reserve(MemoryKind::VerifiedOriginal, raw as u64 + ORIGINAL_OVERHEAD))
        .transpose()?;
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(encoded),
        png::Limits {
            bytes: PNG_WORK as usize,
        },
    );
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info().map_err(|_| StreamError::Codec)?;
    if reader.info().width != identity.geometry.width()
        || reader.info().height != identity.geometry.height()
        || reader.info().color_type != png::ColorType::Rgba
        || reader.info().bit_depth != png::BitDepth::Eight
        || reader.info().interlaced
        || reader.info().srgb != Some(png::SrgbRenderingIntent::Perceptual)
    {
        return Err(StreamError::Codec);
    }
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(raw)
        .map_err(|_| StreamError::Limit)?;
    rgba.resize(raw, 0);
    let mut offset = 0usize;
    while let Some(row) = reader.next_row().map_err(|_| StreamError::Codec)? {
        cancel.check()?;
        let end = offset
            .checked_add(row.data().len())
            .filter(|n| *n <= raw)
            .ok_or(StreamError::Codec)?;
        rgba[offset..end].copy_from_slice(row.data());
        offset = end;
    }
    reader.finish().map_err(|_| StreamError::Codec)?;
    if offset != raw {
        return Err(StreamError::Codec);
    }
    crate::frame::opaque(
        &rgba,
        identity.geometry.width(),
        identity.geometry.height(),
        cancel,
    )?;
    Ok(VerifiedOriginal {
        identity: identity.clone(),
        asset_id: expected.clone(),
        rgba,
        charge,
    })
}

// Refuse metadata inflation and animation before invoking the decoder. Header
// CRCs and IDAT are subsequently checked by png's normal strict decoder.
fn png_subset(bytes: &[u8], width: u32, height: u32, cancel: &Cancellation) -> Result<()> {
    if bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n") {
        return Err(StreamError::Codec);
    }
    let mut pos = 8usize;
    let mut chunks = 0usize;
    let mut header = false;
    let mut srgb = false;
    let mut data = false;
    while pos < bytes.len() {
        cancel.check()?;
        chunks += 1;
        if chunks > 65536 {
            return Err(StreamError::Limit);
        }
        let head = bytes.get(pos..pos + 8).ok_or(StreamError::Codec)?;
        let len =
            u32::from_be_bytes(head[..4].try_into().map_err(|_| StreamError::Codec)?) as usize;
        let end = pos
            .checked_add(12)
            .and_then(|n| n.checked_add(len))
            .filter(|n| *n <= bytes.len())
            .ok_or(StreamError::Codec)?;
        let body = &bytes[pos + 8..end - 4];
        match &head[4..] {
            b"IHDR" if !header && pos == 8 && len == 13 => {
                if body[..4] != width.to_be_bytes()
                    || body[4..8] != height.to_be_bytes()
                    || body[8..] != [8, 6, 0, 0, 0]
                {
                    return Err(StreamError::Codec);
                }
                header = true;
            }
            b"sRGB" if header && !srgb && !data && body == [0] => {
                srgb = true;
            }
            b"IDAT" if header && srgb => {
                data = true;
            }
            b"IEND" if data && len == 0 && end == bytes.len() => return Ok(()),
            _ => return Err(StreamError::Codec),
        }
        pos = end;
    }
    Err(StreamError::Codec)
}
