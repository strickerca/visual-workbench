use crate::{Decoded, Error, Result, inspect};
use ::windows::Win32::{Foundation::*, Graphics::Imaging::*, System::Com::*};
use std::sync::atomic::{AtomicBool, Ordering};
static ACTIVE: AtomicBool = AtomicBool::new(false);
struct Slot;
impl Drop for Slot {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        /* SAFETY: this worker successfully initialized COM. */
        unsafe {
            CoUninitialize();
        }
    }
}
/// Real installed WIC decoder. The caller runs this on an owned native worker.
/// A cancellation request is checked between API calls; a stuck OS codec retains
/// the one decoder slot until it actually returns, rather than releasing a live
/// decoder and admitting replacements. No caller-owned buffer is exposed to it.
pub fn decode(bytes: &[u8], budget: u64, cancel: &dyn Fn() -> bool) -> Result<Decoded> {
    if cancel() {
        return Err(Error::Cancelled);
    }
    let header = inspect(bytes, budget)?;
    ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| Error::Busy)?;
    let _slot = Slot;
    // SAFETY: COM objects stay on this worker; input is borrowed and remains
    // alive until the stream/decoder are dropped; output slices are admitted.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|_| Error::Decode)?;
        let _apartment = Apartment;
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory2, None, CLSCTX_INPROC_SERVER)
                .map_err(|_| Error::CodecUnavailable)?;
        let stream = factory.CreateStream().map_err(|_| Error::Decode)?;
        stream
            .InitializeFromMemory(bytes)
            .map_err(|_| Error::Decode)?;
        let decoder = factory
            .CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
            .map_err(|error| {
                if matches!(
                    error.code(),
                    WINCODEC_ERR_COMPONENTNOTFOUND | WINCODEC_ERR_COMPONENTINITIALIZEFAILURE
                ) {
                    Error::CodecUnavailable
                } else {
                    Error::Decode
                }
            })?;
        if decoder.GetContainerFormat().map_err(|_| Error::Decode)? != GUID_ContainerFormatHeif
            || decoder.GetFrameCount().map_err(|_| Error::Decode)? != 1
        {
            return Err(Error::Unsupported);
        }
        let frame = decoder.GetFrame(0).map_err(|_| Error::Decode)?;
        let mut width = 0;
        let mut height = 0;
        frame
            .GetSize(&mut width, &mut height)
            .map_err(|_| Error::Decode)?;
        if width != header.width || height != header.height {
            return Err(Error::Orientation);
        }
        let format = frame.GetPixelFormat().map_err(|_| Error::Decode)?;
        if ![
            GUID_WICPixelFormat24bppRGB,
            GUID_WICPixelFormat24bppBGR,
            GUID_WICPixelFormat32bppRGB,
            GUID_WICPixelFormat32bppBGR,
            GUID_WICPixelFormat32bppRGBA,
            GUID_WICPixelFormat32bppBGRA,
            GUID_WICPixelFormat32bppPBGRA,
        ]
        .contains(&format)
        {
            return Err(Error::Depth);
        }
        if let Some(profile) = &header.icc {
            let mut actual = 0;
            frame
                .GetColorContexts(&mut [], &mut actual)
                .map_err(|_| Error::Color)?;
            if actual != 1 {
                return Err(Error::Color);
            }
            let context = factory.CreateColorContext().map_err(|_| Error::Color)?;
            let mut contexts = [Some(context.clone())];
            frame
                .GetColorContexts(&mut contexts, &mut actual)
                .map_err(|_| Error::Color)?;
            if actual != 1 || context.GetType().map_err(|_| Error::Color)? != WICColorContextProfile
            {
                return Err(Error::Color);
            }
            let mut size = 0;
            context
                .GetProfileBytes(&mut [], &mut size)
                .map_err(|_| Error::Color)?;
            if size as usize != profile.len() {
                return Err(Error::Color);
            }
            let mut value = vec![0; size as usize];
            context
                .GetProfileBytes(&mut value, &mut size)
                .map_err(|_| Error::Color)?;
            if value != *profile || size as usize != value.len() {
                return Err(Error::Color);
            }
        }
        if cancel() {
            return Err(Error::Cancelled);
        }
        let converter = factory.CreateFormatConverter().map_err(|_| Error::Decode)?;
        if !converter
            .CanConvert(&format, &GUID_WICPixelFormat32bppRGBA)
            .map_err(|_| Error::Decode)?
            .as_bool()
        {
            return Err(Error::Depth);
        }
        converter
            .Initialize(
                &frame,
                &GUID_WICPixelFormat32bppRGBA,
                WICBitmapDitherTypeNone,
                None::<&IWICPalette>,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .map_err(|_| Error::Decode)?;
        let length =
            usize::try_from(u64::from(width) * u64::from(height) * 4).map_err(|_| Error::Limit)?;
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(length).map_err(|_| Error::Limit)?;
        rgba.resize(length, 0);
        // Bounded row batches provide cancellation opportunities when the codec
        // itself cooperates; no resampling, CMS or orientation transform is used.
        let stride = width.checked_mul(4).ok_or(Error::Limit)?;
        for y in (0..height).step_by(32) {
            if cancel() {
                return Err(Error::Cancelled);
            }
            let rows = 32.min(height - y);
            let region = WICRect {
                X: 0,
                Y: y as i32,
                Width: width as i32,
                Height: rows as i32,
            };
            let start = y as usize * stride as usize;
            let end = start + rows as usize * stride as usize;
            converter
                .CopyPixels(&region, stride, &mut rgba[start..end])
                .map_err(|_| Error::Decode)?;
        }
        let result = Decoded {
            header: header.clone(),
            rgba,
        };
        result.validate(&header, budget)?;
        if cancel() {
            return Err(Error::Cancelled);
        }
        Ok(result)
    }
}
