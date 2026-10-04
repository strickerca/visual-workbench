//! OS image decoder integration. Original encoded bytes always remain the
//! canonical primary asset. The callback is installed once by Android startup;
//! Windows uses the installed WIC implementation directly on the core worker.
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Debug, thiserror::Error, uniffi::Error)]
pub enum OsImageError {
    #[error("invalid HEIC original")]
    Invalid,
    #[error("installed HEIC codec is unavailable")]
    Unavailable,
    #[error("HEIC layout is not supported without conversion")]
    Unsupported,
    #[error("HEIC source depth cannot be preserved")]
    Depth,
    #[error("HEIC color profile cannot be preserved")]
    Color,
    #[error("HEIC orientation cannot be preserved")]
    Orientation,
    #[error("HEIC memory admission failed")]
    Memory,
    #[error("HEIC decoder busy")]
    Busy,
    #[error("HEIC decode cancelled")]
    Cancelled,
    #[error("OS HEIC decoder failed")]
    Decode,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct OsImageInfo {
    pub source_asset_id: String,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub orientation: u8,
    pub icc_profile: Vec<u8>,
    pub estimated_peak_bytes: u64,
}
#[derive(uniffi::Record)]
pub struct OsImageRequest {
    pub encoded: Vec<u8>,
    pub info: OsImageInfo,
    pub memory_budget_bytes: u64,
}
#[derive(uniffi::Record)]
pub struct OsImagePixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[uniffi::export(callback_interface)]
pub trait AndroidImageDecoder: Send + Sync {
    fn decode(&self, request: OsImageRequest) -> std::result::Result<OsImagePixels, OsImageError>;
}
static ANDROID: OnceLock<Arc<dyn AndroidImageDecoder>> = OnceLock::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);
struct Slot;
impl Drop for Slot {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}
#[uniffi::export]
pub fn install_android_image_decoder(
    decoder: Box<dyn AndroidImageDecoder>,
) -> std::result::Result<(), OsImageError> {
    if !cfg!(target_os = "android") {
        return Err(OsImageError::Unsupported);
    }
    ANDROID
        .set(Arc::from(decoder))
        .map_err(|_| OsImageError::Busy)
}
#[uniffi::export]
pub fn inspect_os_image(
    encoded: Vec<u8>,
    memory_budget_bytes: u64,
) -> std::result::Result<OsImageInfo, OsImageError> {
    let value = vw_codec_os::inspect(&encoded, memory_budget_bytes).map_err(os_error)?;
    info(&value, memory_budget_bytes)
}
fn info(
    value: &vw_codec_os::Header,
    budget: u64,
) -> std::result::Result<OsImageInfo, OsImageError> {
    Ok(OsImageInfo {
        source_asset_id: value.source_asset.to_string(),
        width: value.width,
        height: value.height,
        bit_depth: value.bit_depth,
        orientation: value.orientation,
        icc_profile: value.icc.clone().unwrap_or_default(),
        estimated_peak_bytes: value.admit(budget).map_err(os_error)?,
    })
}
fn os_error(value: vw_codec_os::Error) -> OsImageError {
    use vw_codec_os::Error as E;
    match value {
        E::Invalid => OsImageError::Invalid,
        E::Unsupported => OsImageError::Unsupported,
        E::CodecUnavailable => OsImageError::Unavailable,
        E::Depth => OsImageError::Depth,
        E::Color => OsImageError::Color,
        E::Orientation => OsImageError::Orientation,
        E::Limit => OsImageError::Memory,
        E::Busy => OsImageError::Busy,
        E::Cancelled => OsImageError::Cancelled,
        E::Decode => OsImageError::Decode,
    }
}
fn raster_error(value: OsImageError) -> vw_raster::RasterError {
    use vw_raster::RasterError as R;
    match value {
        OsImageError::Memory | OsImageError::Busy => R::Allocation,
        OsImageError::Cancelled => R::Cancelled,
        OsImageError::Depth => R::Depth,
        OsImageError::Color => R::Color,
        OsImageError::Unavailable => R::Unsupported("installed OS HEIC codec unavailable"),
        OsImageError::Orientation => R::Unsupported("HEIC orientation"),
        OsImageError::Unsupported => R::Unsupported("HEIC layout"),
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
    if !vw_codec_os::is_heic(bytes) {
        return vw_raster::decode(bytes, limits);
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
    let header = vw_codec_os::inspect(bytes, limits.max_memory_bytes)
        .map_err(os_error)
        .map_err(raster_error)?;
    if u64::from(header.width) * u64::from(header.height) > limits.max_pixels {
        return Err(vw_raster::RasterError::Allocation);
    }
    #[cfg(windows)]
    let pixels = vw_codec_os::decode(bytes, limits.max_memory_bytes, cancel)
        .map_err(os_error)
        .map_err(raster_error)?
        .rgba;
    #[cfg(not(windows))]
    let pixels = {
        let adapter = ANDROID.get().ok_or(vw_raster::RasterError::Unsupported(
            "installed OS HEIC adapter unavailable",
        ))?;
        let result = adapter
            .decode(OsImageRequest {
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
    let decoded = vw_codec_os::Decoded {
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
    Ok(result)
}
