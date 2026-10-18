use crate::{MAX_INPUT, MAX_METADATA, MAX_OUTPUT, ProtocolError, ProtocolResult, buffer, json, read_exact, write_all};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

const MAGIC: &[u8; 8] = b"VWDOC01\0";
const HEADER: usize = 68;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobBinding { pub nonce: [u8; 16], pub source_blake3: [u8; 32] }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobLimits { pub input_bytes: u64, pub output_bytes: u64, pub memory_bytes: u64 }
impl JobLimits {
    pub fn validate(self) -> ProtocolResult<()> {
        if self.input_bytes == 0 || self.input_bytes > MAX_INPUT || self.output_bytes == 0 || self.output_bytes > MAX_OUTPUT
            || !(16 * 1024 * 1024..=2 * 1024 * 1024 * 1024).contains(&self.memory_bytes)
        { Err(ProtocolError::Limit) } else { Ok(()) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tile { pub x: f64, pub y: f64, pub scale: f64, pub width: u32, pub height: u32 }
impl Tile {
    fn validate(self) -> ProtocolResult<()> {
        if ![self.x, self.y, self.scale].into_iter().all(f64::is_finite) || self.x.abs() > 1e9 || self.y.abs() > 1e9
            || !(0.01..=64.0).contains(&self.scale) || self.width == 0 || self.height == 0 || self.width > 512 || self.height > 512
        { Err(ProtocolError::Invalid) } else { Ok(()) }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Overlay { pub page: u32, pub contours: Vec<Vec<[f64; 2]>>, pub transform: [f64; 6], pub rgba: [u8; 4] }
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageRegion { pub page: u32, pub rectangle: [f64; 4] }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CopyKind { Original, Annotated, Flattened }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    PdfInfo,
    PdfPage { page: u32 },
    PdfTile { page: u32, tile: Tile, annotations: bool },
    PdfText { page: u32, start: u32, count: u32 },
    PdfSelection { page: u32, rectangle: [f64; 4] },
    PdfCopy { copy: CopyKind, overlays: Vec<Overlay> },
    PdfRasterRedaction { scale: f64, explicitly_remove_encryption: bool, regions: Vec<PageRegion> },
    SvgInfo,
    SvgTile { tile: Tile },
    SvgCopy { overlays: Vec<Overlay> },
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub limits: JobLimits,
    pub operation: Operation,
    /// A PDF password exists only in this private bounded pipe message. It is
    /// never placed on a command line, serialized into replies, or logged.
    pub password: Option<String>,
}
impl std::fmt::Debug for Request {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.debug_struct("Request").field("limits", &self.limits).field("password_supplied", &self.password.is_some()).finish_non_exhaustive()
    }
}

impl Request {
    fn source_limit(&self) -> u64 {
        if matches!(self.operation, Operation::SvgInfo | Operation::SvgTile { .. } | Operation::SvgCopy { .. }) {
            self.limits.input_bytes.min(4 * 1024 * 1024)
        } else { self.limits.input_bytes }
    }
    pub fn validate(&self) -> ProtocolResult<()> {
        self.limits.validate()?;
        if self.password.as_ref().is_some_and(|value| value.len() > 1024 || value.as_bytes().contains(&0)) { return Err(ProtocolError::Invalid); }
        match &self.operation {
            Operation::PdfTile { tile, .. } | Operation::SvgTile { tile } => tile.validate()?,
            Operation::PdfText { count, start, .. } if *count == 0 || *count > 4096 || start.checked_add(*count).is_none() => return Err(ProtocolError::Invalid),
            Operation::PdfSelection { rectangle, .. } => rectangle_valid(*rectangle)?,
            Operation::PdfCopy { copy, overlays } => {
                if *copy == CopyKind::Original && !overlays.is_empty() { return Err(ProtocolError::Invalid); }
                overlays_valid(overlays)?;
            }
            Operation::SvgCopy { overlays } => {
                overlays_valid(overlays)?;
                if overlays.iter().any(|item| item.page != 0) { return Err(ProtocolError::Invalid); }
            }
            Operation::PdfRasterRedaction { scale, regions, .. } => {
                if !scale.is_finite() || !(0.25..=8.0).contains(scale) || regions.is_empty() || regions.len() > 4096 { return Err(ProtocolError::Invalid); }
                for region in regions { rectangle_valid(region.rectangle)?; }
            }
            _ => {}
        }
        if matches!(self.operation, Operation::SvgInfo | Operation::SvgTile { .. } | Operation::SvgCopy { .. }) && self.password.is_some() { return Err(ProtocolError::Invalid); }
        Ok(())
    }
}

fn rectangle_valid(rect: [f64; 4]) -> ProtocolResult<()> {
    if !rect.into_iter().all(|value| value.is_finite() && value.abs() <= 10_000_000.0) || rect[0] >= rect[2] || rect[1] >= rect[3] { Err(ProtocolError::Invalid) } else { Ok(()) }
}
fn overlays_valid(overlays: &[Overlay]) -> ProtocolResult<()> {
    if overlays.len() > 4096 { return Err(ProtocolError::Limit); }
    let mut contours = 0usize; let mut points = 0usize;
    for item in overlays {
        contours = contours.checked_add(item.contours.len()).ok_or(ProtocolError::Limit)?;
        if item.contours.is_empty() || contours > 16_384 { return Err(ProtocolError::Limit); }
        if !item.transform.into_iter().all(|value| value.is_finite() && value.abs() <= 1e7) { return Err(ProtocolError::Invalid); }
        let [a, b, c, d, _, _] = item.transform;
        if (a * d - b * c).abs() < 1e-12 { return Err(ProtocolError::Invalid); }
        for contour in &item.contours {
            points = points.checked_add(contour.len()).ok_or(ProtocolError::Limit)?;
            if contour.len() < 3 || points > 250_000 { return Err(ProtocolError::Limit); }
            if contour.iter().flatten().any(|value| !value.is_finite() || value.abs() > 1e7) { return Err(ProtocolError::Invalid); }
        }
    }
    Ok(())
}

pub struct Job { pub binding: JobBinding, pub request: Request, pub source: Vec<u8> }
impl std::fmt::Debug for Job {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.debug_struct("Job").field("source_bytes", &self.source.len()).finish_non_exhaustive()
    }
}

/// One input, terminated by EOF. The parent must close its writing handle after
/// sending; the process deadline covers an uncooperative or truncated producer.
pub fn read_job(reader: &mut dyn Read) -> ProtocolResult<Job> {
    let mut header = [0u8; HEADER]; read_exact(reader, &mut header)?;
    if &header[..8] != MAGIC { return Err(ProtocolError::Invalid); }
    let metadata_length = u32::from_le_bytes(header[8..12].try_into().map_err(|_| ProtocolError::Invalid)?) as usize;
    let source_length = u64::from_le_bytes(header[12..20].try_into().map_err(|_| ProtocolError::Invalid)?);
    let binding = JobBinding { source_blake3: header[20..52].try_into().map_err(|_| ProtocolError::Invalid)?, nonce: header[52..68].try_into().map_err(|_| ProtocolError::Invalid)? };
    if metadata_length == 0 || metadata_length > MAX_METADATA || source_length == 0 || source_length > MAX_INPUT || binding.nonce == [0; 16] { return Err(ProtocolError::Limit); }
    let mut metadata = buffer(metadata_length)?; read_exact(reader, &mut metadata)?;
    let request: Request = serde_json::from_slice(&metadata).map_err(|_| ProtocolError::Invalid)?;
    request.validate()?;
    if source_length > request.source_limit() { return Err(ProtocolError::Limit); }
    let admitted = source_length.checked_add(metadata_length as u64 * 32).and_then(|n| n.checked_add(8 * 1024 * 1024)).ok_or(ProtocolError::Limit)?;
    if admitted > request.limits.memory_bytes { return Err(ProtocolError::Limit); }
    // The raw JSON (including any password) is no longer needed. Rust does not
    // guarantee zeroization here; callers must keep this worker memory private.
    drop(metadata);
    let mut source = buffer(usize::try_from(source_length).map_err(|_| ProtocolError::Limit)?)?;
    read_exact(reader, &mut source)?;
    if blake3::hash(&source).as_bytes() != &binding.source_blake3 { return Err(ProtocolError::Integrity); }
    let mut extra = [0u8; 1];
    match reader.read(&mut extra) { Ok(0) => {}, Ok(_) => return Err(ProtocolError::Invalid), Err(error) => return Err(ProtocolError::Io(error.kind())) }
    Ok(Job { binding, request, source })
}

pub fn write_job(writer: &mut dyn Write, binding: JobBinding, request: &Request, source: &[u8]) -> ProtocolResult<()> {
    request.validate()?;
    if binding.nonce == [0; 16] || source.is_empty() || source.len() as u64 > request.source_limit() { return Err(ProtocolError::Limit); }
    if blake3::hash(source).as_bytes() != &binding.source_blake3 { return Err(ProtocolError::Integrity); }
    let metadata = json(request, MAX_METADATA)?;
    let mut header = [0u8; HEADER]; header[..8].copy_from_slice(MAGIC);
    header[8..12].copy_from_slice(&(metadata.len() as u32).to_le_bytes()); header[12..20].copy_from_slice(&(source.len() as u64).to_le_bytes());
    header[20..52].copy_from_slice(&binding.source_blake3); header[52..68].copy_from_slice(&binding.nonce);
    write_all(writer, &header)?; write_all(writer, &metadata)?; write_all(writer, source)?;
    writer.flush().map_err(|error| ProtocolError::Io(error.kind()))
}
