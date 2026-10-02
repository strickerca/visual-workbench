use std::path::Path;
use windows::{
    Win32::{Media::MediaFoundation::*, System::Com::*},
    core::{GUID, PCWSTR},
};

struct Runtime;
impl Drop for Runtime {
    fn drop(&mut self) {
        // SAFETY: Balances successful startup on this thread after COM objects drop.
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}
fn media(subtype: &GUID) -> windows::core::Result<IMFMediaType> {
    // SAFETY: Fresh media type, bounded 64x64 dimensions at 30 progressive fps.
    unsafe {
        let ty = MFCreateMediaType()?;
        ty.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        ty.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        ty.SetUINT64(&MF_MT_FRAME_SIZE, (64u64 << 32) | 64)?;
        ty.SetUINT64(&MF_MT_FRAME_RATE, (30u64 << 32) | 1)?;
        ty.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
        ty.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        Ok(ty)
    }
}
pub fn generate(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if path.exists() {
        return Err("fixture output must be absent".into());
    }
    let wide: Vec<u16> = path
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: Single owned COM/MF runtime, NUL-terminated local filename, valid
    // interface outputs; this function neither captures nor injects any input.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
            CoUninitialize();
            return Err(e.into());
        }
    }
    let _runtime = Runtime;
    // SAFETY: Buffers remain alive and locked while their bounded bytes are
    // filled. Each unlock/sample timestamp precedes submission to the sink.
    unsafe {
        let writer = MFCreateSinkWriterFromURL(PCWSTR(wide.as_ptr()), None, None)?;
        let output = media(&MFVideoFormat_H264)?;
        output.SetUINT32(&MF_MT_AVG_BITRATE, 100_000)?;
        let stream = writer.AddStream(&output)?;
        let input = media(&MFVideoFormat_NV12)?;
        writer.SetInputMediaType(stream, &input, None)?;
        writer.BeginWriting()?;
        for frame in 0..30u32 {
            let buffer = MFCreateMemoryBuffer(64 * 64 * 3 / 2)?;
            let mut pointer = std::ptr::null_mut();
            let mut capacity = 0;
            buffer.Lock(&mut pointer, Some(&mut capacity), None)?;
            if pointer.is_null() || capacity < 6144 {
                let _ = buffer.Unlock();
                return Err("invalid fixture buffer".into());
            }
            let bytes = std::slice::from_raw_parts_mut(pointer, 6144);
            bytes[..4096].fill(32 + (frame * 5) as u8);
            bytes[4096..].fill(128);
            buffer.Unlock()?;
            buffer.SetCurrentLength(6144)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            let start = i64::from(frame) * 10_000_000 / 30;
            let end = i64::from(frame + 1) * 10_000_000 / 30;
            sample.SetSampleTime(start)?;
            sample.SetSampleDuration(end - start)?;
            writer.WriteSample(stream, &sample)?;
        }
        writer.Finalize()?;
        drop(writer);
        // A distinct decoder verifies that the resulting file contains all 30
        // actual video frames, not just plausible MP4 box headers.
        let reader = MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), None)?;
        let decoded = MFCreateMediaType()?;
        decoded.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        decoded.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        reader.SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, None, &decoded)?;
        let mut frames = 0;
        for _ in 0..100 {
            let mut flags = 0;
            let mut sample = None;
            reader.ReadSample(
                MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                0,
                None,
                Some(&mut flags),
                None,
                Some(&mut sample),
            )?;
            if let Some(sample) = sample {
                let buffer = sample.ConvertToContiguousBuffer()?;
                if buffer.GetCurrentLength()? < 6144 {
                    return Err("short decoded frame".into());
                }
                frames += 1;
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
        }
        if frames != 30 {
            return Err("MP4 fixture frame census differs".into());
        }
        println!("T1.03_MP4_GENERATED_AND_DECODED frames=30 width=64 height=64 duration_ms=1000");
    }
    Ok(())
}
