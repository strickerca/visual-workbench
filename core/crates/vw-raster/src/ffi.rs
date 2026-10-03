//! Sole native boundary: libwebp's allocating RGBA lossy encoder. PNG/JPEG/WebP
//! decoding and lossless WebP encoding remain Rust implementations.
use crate::{RasterError, checked_samples};
struct NativeOutput(*mut u8);
impl Drop for NativeOutput {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: pointer comes only from WebPEncodeRGBA; that API requires
            // WebPFree, exactly once, for successful allocations. No borrowed views
            // outlive this guard, including on all error returns below.
            unsafe {
                libwebp_sys::WebPFree(self.0.cast());
            }
        }
    }
}
/// Runtime native codec version (major, minor, revision), useful in receipts.
pub fn webp_encoder_version() -> (u8, u8, u8) {
    // SAFETY: version getter has no arguments, allocation or mutable state.
    let version = unsafe { libwebp_sys::WebPGetEncoderVersion() } as u32;
    ((version >> 16) as u8, (version >> 8) as u8, version as u8)
}
pub(crate) fn encode_lossy_webp(
    rgba: &[u8],
    width: u32,
    height: u32,
    quality: u8,
    budget: u64,
) -> Result<Vec<u8>, RasterError> {
    if width == 0 || height == 0 || width > 16383 || height > 16383 {
        return Err(RasterError::Dimensions {
            format: "WebP",
            limit: 16383,
        });
    }
    if !(1..=100).contains(&quality) || rgba.len() != checked_samples(width, height)? {
        return Err(RasterError::Invalid("WebP quality/buffer"));
    }
    let estimate = u64::from(width) * u64::from(height) * 160 + 32 * 1024 * 1024;
    if estimate > budget {
        return Err(RasterError::Memory {
            estimated: estimate,
            budget,
        });
    }
    let mut output = NativeOutput(std::ptr::null_mut());
    // SAFETY: dimensions are nonzero and <=16383; stride width*4 fits i32.
    // The validated slice holds every RGBA row, remains borrowed until the call
    // returns, and output is writable pointer storage. This synchronous API
    // does not retain input. Default-features=false disables native threading.
    let size = unsafe {
        libwebp_sys::WebPEncodeRGBA(
            rgba.as_ptr(),
            width as i32,
            height as i32,
            (width * 4) as i32,
            f32::from(quality),
            &mut output.0,
        )
    };
    if size == 0 || output.0.is_null() {
        return Err(RasterError::Codec);
    }
    if size as u64 > budget || size > isize::MAX as usize {
        return Err(RasterError::Memory {
            estimated: size as u64,
            budget,
        });
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| RasterError::Allocation)?;
    // SAFETY: successful WebPEncodeRGBA returns exactly `size` initialized bytes
    // owned by output. Bounds above satisfy Rust slice size requirements. Copy
    // completes while the RAII guard still owns and retains the native buffer.
    bytes.extend_from_slice(unsafe { std::slice::from_raw_parts(output.0, size) });
    Ok(bytes)
}
