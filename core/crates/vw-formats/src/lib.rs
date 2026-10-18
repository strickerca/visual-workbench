//! Bounded BMP/TIFF conversion. The encoded original remains authoritative.
#![forbid(unsafe_code)]
mod admission;
mod bmp;
mod io;
mod jpeg;
mod orientation;
mod tiff;
mod tiff_header;
pub use admission::{Format, Header, Limits, Notice};
pub use io::WriteSeek;
use vw_raster::DecodedImage;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid image container or metadata")]
    Invalid,
    #[error("image representation is unsupported")]
    Unsupported,
    #[error("image depth conversion must be explicit")]
    Depth,
    #[error("image color declaration cannot be preserved")]
    Color,
    #[error("image resource limit exceeded")]
    Limit,
    #[error("image allocation refused")]
    Allocation,
    #[error("image operation cancelled")]
    Cancelled,
    #[error("image codec rejected the admitted source")]
    Codec,
    #[error("image I/O failed: {0:?}")]
    Io(std::io::ErrorKind),
}
pub type Result<T> = std::result::Result<T, Error>;
pub(crate) fn check(cancel: &dyn Fn() -> bool) -> Result<()> {
    if cancel() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

pub struct Imported {
    pub image: DecodedImage,
    pub header: Header,
}

/// Header admission happens before third-party parsing or profile copies.
pub fn inspect(bytes: &[u8], limits: Limits) -> Result<Header> {
    limits.input(bytes.len())?;
    if bytes.starts_with(b"BM") {
        bmp::inspect(bytes, limits)
    } else if bytes.starts_with(b"II") || bytes.starts_with(b"MM") {
        tiff_header::inspect(bytes, limits)
    } else {
        Err(Error::Unsupported)
    }
}

pub fn decode(bytes: &[u8], limits: Limits, cancel: &dyn Fn() -> bool) -> Result<Imported> {
    check(cancel)?;
    let header = inspect(bytes, limits)?;
    let pixels = match header.format {
        Format::Bmp => bmp::decode(bytes, &header, limits, cancel)?,
        Format::Tiff => tiff::decode(bytes, &header, limits, cancel)?,
    };
    check(cancel)?;
    let (width, height, pixels) = orientation::apply(
        header.width,
        header.height,
        header.orientation,
        pixels,
        cancel,
    )?;
    let image = DecodedImage {
        width,
        height,
        pixels,
        icc: header.icc.clone(),
        source_asset: header.source_asset.clone(),
        original_available: true,
        orientation_applied: header.orientation,
    };
    image.validate().map_err(|_| Error::Color)?;
    check(cancel)?;
    Ok(Imported { image, header })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Bmp8,
    Tiff8,
    Tiff16,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportReceipt {
    pub format: ExportFormat,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub source_asset: vw_model::AssetId,
}

/// Writes only to caller-owned private staging. Caller must fsync, hash and
/// atomically publish after success and its exact revision check. No automatic
/// depth, profile, alpha or size conversion occurs here. A preview whose original
/// is unavailable cannot be exported through this boundary. This raw codec
/// primitive carries no application provenance; standalone application exports
/// must use `encode_to_with_metadata`.
pub fn encode_to(
    image: &DecodedImage,
    format: ExportFormat,
    limits: Limits,
    output: &mut dyn WriteSeek,
    cancel: &dyn Fn() -> bool,
) -> Result<ExportReceipt> {
    encode(image, format, limits, None, output, cancel)
}

/// Maximum application provenance payload, excluding the TIFF NUL terminator.
pub const MAX_METADATA_BYTES: usize = 16 * 1024;

/// Store application provenance in TIFF's standard ASCII ImageDescription tag.
/// The caller supplies serialized metadata for this exact source and revision.
/// BMP has no admitted provenance carrier and is refused before writing bytes.
pub fn encode_to_with_metadata(
    image: &DecodedImage,
    format: ExportFormat,
    limits: Limits,
    metadata: &str,
    output: &mut dyn WriteSeek,
    cancel: &dyn Fn() -> bool,
) -> Result<ExportReceipt> {
    check(cancel)?;
    if format == ExportFormat::Bmp8 {
        return Err(Error::Unsupported);
    }
    if metadata.len() > MAX_METADATA_BYTES {
        return Err(Error::Limit);
    }
    if metadata.is_empty() || !metadata.is_ascii() || metadata.as_bytes().contains(&0) {
        return Err(Error::Invalid);
    }
    encode(image, format, limits, Some(metadata), output, cancel)
}

fn encode(
    image: &DecodedImage,
    format: ExportFormat,
    limits: Limits,
    metadata: Option<&str>,
    output: &mut dyn WriteSeek,
    cancel: &dyn Fn() -> bool,
) -> Result<ExportReceipt> {
    check(cancel)?;
    limits.validate()?;
    if !image.original_available {
        return Err(Error::Unsupported);
    }
    let depth = image.pixels.bit_depth();
    if (format == ExportFormat::Tiff16) != (depth == 16) {
        return Err(Error::Depth);
    }
    let samples = u64::from(image.width)
        .checked_mul(u64::from(image.height))
        .and_then(|n| n.checked_mul(4))
        .ok_or(Error::Limit)?;
    if samples != image.pixels.len() as u64 {
        return Err(Error::Invalid);
    }
    let estimated = limits.admit(
        image.width,
        image.height,
        0,
        image.icc.as_ref().map_or(0, Vec::len),
    )?;
    // Callers may retain a Vec with much more capacity than its logical samples.
    // Charge that allocation as well as the bounded codec/profile workspace.
    let excess = match &image.pixels {
        vw_raster::Pixels::Rgba8(p) => (p.capacity() - p.len()) as u64,
        vw_raster::Pixels::Rgba16(p) => ((p.capacity() - p.len()) as u64)
            .checked_mul(2)
            .ok_or(Error::Limit)?,
    }
    .checked_add(
        image
            .icc
            .as_ref()
            .map_or(0, |p| (p.capacity() - p.len()) as u64),
    )
    .ok_or(Error::Limit)?;
    if estimated
        .checked_add(excess)
        .is_none_or(|n| n > limits.memory_bytes)
    {
        return Err(Error::Limit);
    }
    // This reserve includes retained input, profile parsing, row buffers and
    // complete output metadata; the encoder streams rows, never a full clone.
    image.validate().map_err(|_| Error::Color)?;
    let mut bounded = io::Output::new(output, limits.output_bytes, cancel)?;
    match format {
        ExportFormat::Bmp8 => bmp::encode(image, &mut bounded)?,
        ExportFormat::Tiff8 | ExportFormat::Tiff16 => tiff::encode(image, &mut bounded, metadata)?,
    }
    let bytes = bounded.finish()?;
    check(cancel)?;
    Ok(ExportReceipt {
        format,
        bytes,
        width: image.width,
        height: image.height,
        bit_depth: depth,
        source_asset: image.source_asset.clone(),
    })
}
