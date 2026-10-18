//! OS image decoder integration. Original encoded bytes always remain the
//! canonical primary asset. The callback is installed once by Android startup;
//! Windows uses the installed WIC implementation directly on the core worker.
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Debug, thiserror::Error, uniffi::Error)]
pub enum AvifImageError {
    #[error("invalid AVIF original")]
    Invalid,
    #[error("installed AVIF codec is unavailable")]
    Unavailable,
    #[error("AVIF layout is not supported without conversion")]
    Unsupported,
    #[error("AVIF source depth cannot be preserved")]
    Depth,
    #[error("AVIF color profile cannot be preserved")]
    Color,
    #[error("AVIF orientation cannot be preserved")]
    Orientation,
    #[error("AVIF memory admission failed")]
    Memory,
    #[error("AVIF decoder busy")]
    Busy,
    #[error("AVIF decode cancelled")]
    Cancelled,
    #[error("OS AVIF decoder failed")]
    Decode,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AvifColorInfo {
    pub primaries: u16,
    pub transfer: u16,
    pub matrix: u16,
    pub full_range: bool,
}
#[derive(Clone, uniffi::Record)]
pub struct AvifImageInfo {
    pub source_asset_id: String,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub orientation: u8,
    pub icc_profile: Vec<u8>,
    pub nclx: Option<AvifColorInfo>,
    pub estimated_peak_bytes: u64,
}
#[derive(uniffi::Record)]
pub struct AvifImageRequest {
    pub encoded: Vec<u8>,
    pub info: AvifImageInfo,
    pub memory_budget_bytes: u64,
}
#[derive(uniffi::Record)]
pub struct AvifImagePixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[uniffi::export(callback_interface)]
pub trait AndroidAvifDecoder: Send + Sync {
    fn decode(
        &self,
        request: AvifImageRequest,
    ) -> std::result::Result<AvifImagePixels, AvifImageError>;
}
static ANDROID: OnceLock<Arc<dyn AndroidAvifDecoder>> = OnceLock::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);
struct Slot;
impl Drop for Slot {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}
#[uniffi::export]
pub fn install_android_avif_decoder(
    decoder: Box<dyn AndroidAvifDecoder>,
) -> std::result::Result<(), AvifImageError> {
    if !cfg!(target_os = "android") {
        return Err(AvifImageError::Unsupported);
    }
    ANDROID
        .set(Arc::from(decoder))
        .map_err(|_| AvifImageError::Busy)
}
#[uniffi::export]
pub fn inspect_avif_image(
    encoded: Vec<u8>,
    memory_budget_bytes: u64,
) -> std::result::Result<AvifImageInfo, AvifImageError> {
    let value = vw_avif_os::inspect(&encoded, memory_budget_bytes).map_err(os_error)?;
    info(&value, memory_budget_bytes)
}
fn info(
    value: &vw_avif_os::Header,
    budget: u64,
) -> std::result::Result<AvifImageInfo, AvifImageError> {
    Ok(AvifImageInfo {
        source_asset_id: value.source_asset.to_string(),
        width: value.width,
        height: value.height,
        bit_depth: value.bit_depth,
        orientation: value.orientation,
        icc_profile: value.icc.clone().unwrap_or_default(),
        nclx: value.nclx.map(|v| AvifColorInfo {
            primaries: v.primaries,
            transfer: v.transfer,
            matrix: v.matrix,
            full_range: v.full_range,
        }),
        estimated_peak_bytes: value.admit(budget).map_err(os_error)?,
    })
}
fn os_error(value: vw_avif_os::Error) -> AvifImageError {
    use vw_avif_os::Error as E;
    match value {
        E::Invalid => AvifImageError::Invalid,
        E::Unsupported => AvifImageError::Unsupported,
        E::CodecUnavailable => AvifImageError::Unavailable,
        E::Depth => AvifImageError::Depth,
        E::Color => AvifImageError::Color,
        E::Orientation => AvifImageError::Orientation,
        E::Limit => AvifImageError::Memory,
        E::Busy => AvifImageError::Busy,
        E::Cancelled => AvifImageError::Cancelled,
        E::Decode => AvifImageError::Decode,
    }
}
fn raster_error(value: AvifImageError) -> vw_raster::RasterError {
    use vw_raster::RasterError as R;
    match value {
        AvifImageError::Memory | AvifImageError::Busy => R::Allocation,
        AvifImageError::Cancelled => R::Cancelled,
        AvifImageError::Depth => R::Depth,
        AvifImageError::Color => R::Color,
        AvifImageError::Unavailable => R::CodecUnavailable,
        AvifImageError::Orientation => R::Unsupported("AVIF orientation"),
        AvifImageError::Unsupported => R::Unsupported("AVIF layout"),
        _ => R::Codec,
    }
}
pub(crate) fn decode(
    bytes: &[u8],
    limits: vw_raster::DecodeLimits,
    cancel: &dyn Fn() -> bool,
) -> std::result::Result<vw_raster::DecodedImage, vw_raster::RasterError> {
    if cancel() {
        return Err(vw_raster::RasterError::Cancelled);
    }
    if !vw_avif_os::is_avif(bytes) {
        return crate::os_images::decode(bytes, limits, cancel);
    }
    if bytes.len() > limits.max_encoded_bytes {
        return Err(vw_raster::RasterError::EncodedLimit {
            limit: limits.max_encoded_bytes as u64,
        });
    }
    ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| vw_raster::RasterError::Allocation)?;
    let _slot = Slot;
    let header = vw_avif_os::inspect(bytes, limits.max_memory_bytes)
        .map_err(os_error)
        .map_err(raster_error)?;
    if u64::from(header.width) * u64::from(header.height) > limits.max_pixels {
        return Err(vw_raster::RasterError::Allocation);
    }
    #[cfg(windows)]
    let pixels = vw_avif_os::decode(bytes, limits.max_memory_bytes, cancel)
        .map_err(os_error)
        .map_err(raster_error)?
        .rgba;
    #[cfg(not(windows))]
    let pixels = {
        let adapter = ANDROID
            .get()
            .ok_or(vw_raster::RasterError::CodecUnavailable)?;
        let result = adapter
            .decode(AvifImageRequest {
                encoded: bytes.to_vec(),
                info: info(&header, limits.max_memory_bytes).map_err(raster_error)?,
                memory_budget_bytes: limits.max_memory_bytes,
            })
            .map_err(raster_error)?;
        if result.width != header.width || result.height != header.height {
            return Err(vw_raster::RasterError::Invalid("OS image extent"));
        }
        result.rgba
    };
    if cancel() {
        return Err(vw_raster::RasterError::Cancelled);
    }
    let decoded = vw_avif_os::Decoded {
        header: header.clone(),
        rgba: pixels,
    };
    decoded
        .validate(&header, limits.max_memory_bytes)
        .map_err(os_error)
        .map_err(raster_error)?;
    // The only accepted profile-free color declaration is exact nclx sRGB.
    // Resolve that declaration the same way PNG sRGB is resolved by vw-raster;
    // keep every original encoded byte and any original ICC profile unchanged.
    let icc = match header.icc {
        Some(value) => value,
        None => vw_raster::standard_srgb_profile()?,
    };
    let result = vw_raster::DecodedImage {
        width: header.width,
        height: header.height,
        pixels: vw_raster::Pixels::Rgba8(decoded.rgba),
        icc: Some(icc),
        source_asset: header.source_asset,
        original_available: true,
        orientation_applied: header.orientation,
    };
    // Container memory admission reserved the full existing ICC parser cap.
    result.validate()?;
    if cancel() {
        return Err(vw_raster::RasterError::Cancelled);
    }
    Ok(result)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn missing_codec_is_typed_separately_from_corrupt_or_unsupported_data() {
        assert!(matches!(
            raster_error(AvifImageError::Unavailable),
            vw_raster::RasterError::CodecUnavailable
        ));
        assert!(matches!(
            raster_error(AvifImageError::Decode),
            vw_raster::RasterError::Codec
        ));
        assert!(matches!(
            raster_error(AvifImageError::Unsupported),
            vw_raster::RasterError::Unsupported(_)
        ));
    }
    #[test]
    fn inspection_and_cancel_refusal_do_not_need_an_os_decoder() {
        assert!(matches!(
            inspect_avif_image(Vec::new(), 256 * 1024 * 1024),
            Err(AvifImageError::Memory)
        ));
        assert!(matches!(
            decode(&[], vw_raster::DecodeLimits::default(), &|| true),
            Err(vw_raster::RasterError::Cancelled)
        ));
    }
}
