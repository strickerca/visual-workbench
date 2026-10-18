//! Additive BMP/TIFF admission and private-file encoding. Encoded originals
//! remain authoritative; this adapter never replaces them with a derivative.
use crate::{Cancellation, ProjectSession, WorkflowBinding, WorkflowError};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    sync::Arc,
};
use vw_model::{AssetId, Id};
use vw_raster::{
    DecodeLimits, DecodedImage, ExportFormat, ExportRequest, ExportedImage, RasterError,
};

const MIB: u64 = 1024 * 1024;
const BUFFERED_LIMIT: u64 = 64 * MIB;

pub(crate) fn is_format(bytes: &[u8]) -> bool {
    bytes.starts_with(b"BM") || bytes.starts_with(b"II") || bytes.starts_with(b"MM")
}
pub(crate) fn label(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"BM") {
        Some("bmp")
    } else if bytes.starts_with(b"II") || bytes.starts_with(b"MM") {
        Some("tiff")
    } else {
        None
    }
}
fn limits(l: DecodeLimits) -> vw_formats::Limits {
    vw_formats::Limits {
        encoded_bytes: l.max_encoded_bytes as u64,
        output_bytes: 512 * MIB,
        pixels: l.max_pixels,
        memory_bytes: l.max_memory_bytes,
    }
}
fn error(e: vw_formats::Error) -> RasterError {
    match e {
        vw_formats::Error::Invalid => RasterError::Invalid("BMP/TIFF container"),
        vw_formats::Error::Unsupported => RasterError::Unsupported("BMP/TIFF representation"),
        vw_formats::Error::Depth => RasterError::Depth,
        vw_formats::Error::Color => RasterError::Color,
        vw_formats::Error::Limit => RasterError::Allocation,
        vw_formats::Error::Allocation => RasterError::Allocation,
        vw_formats::Error::Cancelled => RasterError::Cancelled,
        vw_formats::Error::Codec => RasterError::Codec,
        vw_formats::Error::Io(_) => RasterError::Io,
    }
}
pub(crate) fn decode(
    bytes: &[u8],
    l: DecodeLimits,
    cancel: &dyn Fn() -> bool,
) -> Result<DecodedImage, RasterError> {
    if cancel() {
        return Err(RasterError::Cancelled);
    }
    let decoded = if !is_format(bytes) {
        crate::avif_images::decode(bytes, l, cancel)
    } else {
        // The format crate charges encoded storage and header/profile work before
        // invoking either third-party codec. Its bounds may never enlarge a caller.
        if l.max_memory_bytes < 64 * MIB || l.max_memory_bytes > 1024 * MIB {
            return Err(RasterError::Memory {
                estimated: 64 * MIB,
                budget: l.max_memory_bytes,
            });
        }
        vw_formats::decode(bytes, limits(l), cancel)
            .map(|result| result.image)
            .map_err(error)
    };
    // Generic PNG/JPEG/WebP and installed OS decoders may be non-interruptible.
    // Their result cannot escape after the caller cancelled while they ran.
    if cancel() {
        return Err(RasterError::Cancelled);
    }
    decoded
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct ImportNotices {
    first_tiff_page_only: bool,
}

/// Called before decoding, while no decoded pixels/profile clones are held.
/// The notice becomes canonical, revision-covered asset metadata.
pub(crate) fn import_metadata(
    bytes: &[u8],
    l: DecodeLimits,
    cancel: &Cancellation,
) -> Result<String, RasterError> {
    cancel.check().map_err(|_| RasterError::Cancelled)?;
    if !is_format(bytes) {
        return Ok("{}".into());
    }
    let header = vw_formats::inspect(bytes, limits(l)).map_err(error)?;
    cancel.check().map_err(|_| RasterError::Cancelled)?;
    serde_json::to_string(&ImportNotices {
        first_tiff_page_only: header
            .notices
            .contains(&vw_formats::Notice::FirstTiffPageOnly),
    })
    .map_err(|_| RasterError::Metadata)
}

pub(crate) fn external(format: ExportFormat) -> bool {
    matches!(
        format,
        ExportFormat::Bmp8 | ExportFormat::Tiff8 | ExportFormat::Tiff16
    )
}
fn external_format(format: ExportFormat) -> Result<vw_formats::ExportFormat, RasterError> {
    match format {
        ExportFormat::Bmp8 => Ok(vw_formats::ExportFormat::Bmp8),
        ExportFormat::Tiff8 => Ok(vw_formats::ExportFormat::Tiff8),
        ExportFormat::Tiff16 => Ok(vw_formats::ExportFormat::Tiff16),
        _ => Err(RasterError::Invalid("external format")),
    }
}
fn output_bound(source: &DecodedImage, request: &ExportRequest) -> Result<u64, RasterError> {
    let region = request.region.unwrap_or(vw_raster::Region {
        x: 0,
        y: 0,
        width: source.width,
        height: source.height,
    });
    region.validate(source.width, source.height)?;
    // Includes uncompressed RGBA16, ICC and all bounded strip/header metadata.
    u64::from(region.width)
        .checked_mul(u64::from(region.height))
        .and_then(|n| n.checked_mul(8))
        .and_then(|n| n.checked_add(source.icc.as_ref().map_or(0, |p| p.len() as u64)))
        .and_then(|n| n.checked_add(MIB))
        .ok_or(RasterError::Allocation)
}
fn retained_source_bytes(source: &DecodedImage) -> Result<u64, RasterError> {
    let pixels = match &source.pixels {
        vw_raster::Pixels::Rgba8(p) => p.capacity() as u64,
        vw_raster::Pixels::Rgba16(p) => (p.capacity() as u64)
            .checked_mul(2)
            .ok_or(RasterError::Allocation)?,
    };
    pixels
        .checked_add(source.icc.as_ref().map_or(0, |p| p.capacity() as u64))
        .ok_or(RasterError::Allocation)
}
fn subtract_retained(budget: u64, retained: u64) -> Result<u64, RasterError> {
    let estimated = retained
        .checked_add(64 * MIB)
        .ok_or(RasterError::Allocation)?;
    if estimated > budget {
        return Err(RasterError::Memory { estimated, budget });
    }
    Ok(budget - retained)
}
pub(crate) fn encode_private(
    source: &DecodedImage,
    request: &ExportRequest,
    cap: u64,
    file: &mut File,
    cancel: &Cancellation,
) -> Result<(u64, vw_raster::ExportMetadata), RasterError> {
    if output_bound(source, request)? > cap {
        return Err(RasterError::EncodedLimit { limit: cap });
    }
    cancel.check().map_err(|_| RasterError::Cancelled)?;
    // The original full allocation stays alive even when exporting a tiny crop.
    // Preparation and encoding receive only the remaining workspace. Preserve
    // the user's original settings in the final receipt, not this internal cap.
    let mut admitted = request.clone();
    admitted.memory_budget_bytes =
        subtract_retained(request.memory_budget_bytes, retained_source_bytes(source)?)?;
    let mut prepared =
        vw_raster::prepare_export_pixels(source, &admitted, &|| cancel.is_cancelled())?;
    prepared.metadata.settings = request.clone();
    let limits = vw_formats::Limits {
        output_bytes: cap,
        memory_bytes: admitted.memory_budget_bytes,
        ..vw_formats::Limits::default()
    };
    let result = vw_formats::encode_to(
        &prepared.image,
        external_format(request.format)?,
        limits,
        file,
        &|| cancel.is_cancelled(),
    )
    .map_err(error)?;
    if file.metadata().map_err(|_| RasterError::Io)?.len() != result.bytes {
        return Err(RasterError::Io);
    }
    cancel.check().map_err(|_| RasterError::Cancelled)?;
    Ok((result.bytes, prepared.metadata))
}
pub(crate) fn encode_buffered(
    source: &DecodedImage,
    request: &ExportRequest,
    cancel: &Cancellation,
) -> Result<ExportedImage, RasterError> {
    let bound = output_bound(source, request)?;
    if bound > BUFFERED_LIMIT {
        return Err(RasterError::EncodedLimit {
            limit: BUFFERED_LIMIT,
        });
    }
    // The seekable encoder writes only to owned disk staging. The reply is read
    // once after success; four copies conservatively cover UniFFI transfer.
    let work = vw_raster::preflight(
        source.width,
        source.height,
        source.pixels.bit_depth(),
        source.pixels.has_alpha(),
        request,
    )?;
    let copies = bound.checked_mul(4).ok_or(RasterError::Allocation)?;
    let peak = work
        .checked_add(retained_source_bytes(source)?)
        .and_then(|n| n.checked_add(copies))
        .ok_or(RasterError::Allocation)?;
    if peak > request.memory_budget_bytes {
        return Err(RasterError::Memory {
            estimated: peak,
            budget: request.memory_budget_bytes,
        });
    }
    let mut file = tempfile::tempfile().map_err(|_| RasterError::Io)?;
    let mut admitted = request.clone();
    admitted.memory_budget_bytes = subtract_retained(request.memory_budget_bytes, copies)?;
    let (length, mut metadata) = encode_private(source, &admitted, bound, &mut file, cancel)?;
    metadata.settings = request.clone();
    file.seek(SeekFrom::Start(0)).map_err(|_| RasterError::Io)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length as usize)
        .map_err(|_| RasterError::Allocation)?;
    bytes.resize(length as usize, 0);
    for chunk in bytes.chunks_mut(65536) {
        cancel.check().map_err(|_| RasterError::Cancelled)?;
        file.read_exact(chunk).map_err(|_| RasterError::Io)?;
    }
    if file.read(&mut [0u8; 1]).map_err(|_| RasterError::Io)? != 0 {
        return Err(RasterError::Io);
    }
    cancel.check().map_err(|_| RasterError::Cancelled)?;
    Ok(ExportedImage { bytes, metadata })
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct OriginalFormatInfo {
    pub binding: WorkflowBinding,
    pub source_asset_id: String,
    pub format: String,
    pub source_bit_depth: u8,
    pub untagged: bool,
    pub first_tiff_page_only: bool,
}

#[uniffi::export]
impl ProjectSession {
    pub async fn original_format_info(
        &self,
        binding: WorkflowBinding,
        cancellation: Arc<Cancellation>,
    ) -> Result<OriginalFormatInfo, WorkflowError> {
        self.check_open()?;
        binding.validate()?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    cancellation.check()?;
                    if closed.load(std::sync::atomic::Ordering::Acquire) {
                        return Err(WorkflowError::Closed);
                    }
                    crate::workflow::canonical_workspace(state, 256 * MIB, false)?;
                    crate::workflow::check_binding(state, &binding)?;
                    let project = state.project()?;
                    let document = Id::try_from(binding.document_id.clone())
                        .map_err(|_| WorkflowError::Invalid)?;
                    let definition = &project
                        .documents
                        .get(&document)
                        .ok_or(WorkflowError::Stale)?
                        .definition;
                    let asset_id = AssetId::try_from(definition.primary_asset_id.clone())
                        .map_err(|_| WorkflowError::Storage)?;
                    let asset = project
                        .assets
                        .get(&asset_id)
                        .ok_or(WorkflowError::Storage)?;
                    if !matches!(asset.bit_depth, 8 | 16)
                        || !matches!(
                            asset.format.as_str(),
                            "png" | "jpeg" | "webp" | "heic" | "heif" | "avif" | "bmp" | "tiff"
                        )
                    {
                        return Err(WorkflowError::Invalid);
                    }
                    if asset.metadata_json.len() > 4096 {
                        return Err(WorkflowError::Limit);
                    }
                    let notices: ImportNotices = if asset.format == "tiff" {
                        // Old projects may lack the additive field. Report conservative
                        // visibility rather than silently promising a complete TIFF.
                        serde_json::from_str(&asset.metadata_json).unwrap_or(ImportNotices {
                            first_tiff_page_only: true,
                        })
                    } else {
                        ImportNotices::default()
                    };
                    cancellation.check()?;
                    if closed.load(std::sync::atomic::Ordering::Acquire) {
                        return Err(WorkflowError::Closed);
                    }
                    Ok(OriginalFormatInfo {
                        binding,
                        source_asset_id: asset_id.to_string(),
                        format: asset.format.clone(),
                        source_bit_depth: asset.bit_depth as u8,
                        untagged: asset.icc_profile.is_empty(),
                        first_tiff_page_only: notices.first_tiff_page_only,
                    })
                })())
            })
            .await?
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn image() -> DecodedImage {
        DecodedImage {
            width: 2,
            height: 1,
            pixels: vw_raster::Pixels::Rgba8(vec![1, 2, 3, 0, 4, 5, 6, 255]),
            icc: None,
            source_asset: AssetId::hash(b"immutable-format-adapter-fixture"),
            original_available: true,
            orientation_applied: 1,
        }
    }
    fn request() -> ExportRequest {
        ExportRequest {
            format: ExportFormat::Bmp8,
            region: None,
            revision: vw_proto::v1::Revision {
                host_seq: 1,
                state_hash: vec![1; 32],
            },
            alpha: vw_raster::AlphaPolicy::Preserve,
            color: vw_raster::ColorPolicy::Preserve,
            allow_depth_reduction: false,
            memory_budget_bytes: 256 * MIB,
            capture_session: None,
            frame_id: None,
        }
    }
    #[test]
    fn cancellation_after_the_generic_decoder_returns_discards_its_pixels() {
        let mut r = request();
        r.format = ExportFormat::Png8;
        let encoded = vw_raster::export(&image(), &r).unwrap().bytes;
        let calls = Cell::new(0);
        let result = decode(&encoded, DecodeLimits::default(), &|| {
            calls.set(calls.get() + 1);
            calls.get() >= 4
        });
        assert!(matches!(result, Err(RasterError::Cancelled)));
    }
    #[test]
    fn a_tiny_crop_still_charges_the_entire_retained_source_allocation() {
        let mut source = image();
        if let vw_raster::Pixels::Rgba8(p) = &mut source.pixels {
            p.reserve_exact(4 * MIB as usize);
        }
        let mut r = request();
        r.region = Some(vw_raster::Region {
            x: 1,
            y: 0,
            width: 1,
            height: 1,
        });
        r.memory_budget_bytes = 65 * MIB;
        let mut file = tempfile::tempfile().unwrap();
        assert!(matches!(
            encode_private(&source, &r, 8 * MIB, &mut file, &Cancellation::new()),
            Err(RasterError::Memory { .. })
        ));
        assert_eq!(file.metadata().unwrap().len(), 0);
        assert!(matches!(
            encode_buffered(&source, &r, &Cancellation::new()),
            Err(RasterError::Memory { .. })
        ));
    }
    #[test]
    fn buffered_receipt_keeps_the_requested_policy_and_original_identity() {
        let source = image();
        let r = request();
        let result = encode_buffered(&source, &r, &Cancellation::new()).unwrap();
        assert_eq!(
            result.metadata.settings.memory_budget_bytes,
            r.memory_budget_bytes
        );
        assert_eq!(
            result.metadata.source_asset,
            source.source_asset.to_string()
        );
        assert!(!result.metadata.settings.allow_depth_reduction);
        let decoded =
            vw_formats::decode(&result.bytes, vw_formats::Limits::default(), &|| false).unwrap();
        assert_eq!(decoded.image.pixels, source.pixels);
    }
}
