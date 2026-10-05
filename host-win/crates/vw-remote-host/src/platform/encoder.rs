//! Media Foundation / D3D11 boundary: hardware-only, one frame in flight.
use super::{Gpu, api, pump, required};
use crate::{Error, Result};
use std::{
    mem::ManuallyDrop,
    ptr,
    time::{Duration, Instant},
};
use vw_remote::{BITRATE, EncoderCapabilities, MAX_ACCESS_UNIT, Target};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{Direct3D11::*, Dxgi::Common::*},
        Media::MediaFoundation::*,
        System::{Com::CoTaskMemFree, Variant::VARIANT},
    },
    core::{GUID, Interface},
};

pub struct Encoded {
    pub bytes: Vec<u8>,
    pub pts: i64,
}
struct Activation(IMFActivate);
impl Drop for Activation {
    fn drop(&mut self) {
        // SAFETY: uniquely owned successful activation, including configure failure.
        unsafe {
            let _ = self.0.ShutdownObject();
        }
    }
}
pub struct Encoder {
    transform: IMFTransform,
    _activation: Activation,
    events: Option<IMFMediaEventGenerator>,
    gpu_device: ID3D11Device,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    processor: ID3D11VideoProcessor,
    enumerator: ID3D11VideoProcessorEnumerator,
    _manager: IMFDXGIDeviceManager,
    width: u32,
    height: u32,
    pending_input: bool,
    pending_output: bool,
    output_count: u32,
    name: String,
    vendor: String,
    adapter_vendor: u32,
}

fn activations() -> Result<Vec<IMFActivate>> {
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_HEVC,
    };
    let mut raw = ptr::null_mut();
    let mut count = 0;
    // SAFETY: MFTEnumEx allocates an initialized COM-pointer array with count;
    // each pointer is moved into an owning wrapper before the allocation is freed.
    unsafe {
        api(
            "MFTEnumEx hardware HEVC",
            MFTEnumEx(
                MFT_CATEGORY_VIDEO_ENCODER,
                MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER,
                None,
                Some(&output),
                &mut raw,
                &mut count,
            ),
        )?;
        if raw.is_null() || count == 0 {
            return Err(unsupported("no hardware HEVC MFT registered"));
        }
        let items = std::slice::from_raw_parts_mut(raw, count as usize);
        let result = items.iter_mut().filter_map(Option::take).collect();
        CoTaskMemFree(Some(raw.cast()));
        Ok(result)
    }
}
impl Encoder {
    pub fn new(gpu: &Gpu, target: &Target) -> Result<Self> {
        let (width, height) = target.coded_size()?;
        let bitrate = BITRATE;
        let mut last = unsupported("no configured hardware HEVC encoder");
        for activation in activations()? {
            match Self::configure(gpu, activation, target, width, height, bitrate) {
                Ok(encoder) => return Ok(encoder),
                Err(error) => {
                    // Preserve typed refusal; never log captured target data.
                    last = error;
                }
            }
        }
        Err(last)
    }
    fn configure(
        gpu: &Gpu,
        activation: IMFActivate,
        target: &Target,
        width: u32,
        height: u32,
        bitrate: u32,
    ) -> Result<Self> {
        // SAFETY: All interfaces are owned COM objects and initialized arguments.
        // Only registered hardware MFTs from MFTEnumEx reach this function.
        unsafe {
            let length = api(
                "encoder name length",
                activation.GetStringLength(&MFT_FRIENDLY_NAME_Attribute),
            )?;
            if length > 256 {
                return Err(Error::Invalid);
            }
            let mut name = vec![0u16; length as usize + 1];
            api(
                "encoder name",
                activation.GetString(&MFT_FRIENDLY_NAME_Attribute, &mut name, None),
            )?;
            let name = String::from_utf16_lossy(&name[..length as usize]);
            let vendor_length = activation
                .GetStringLength(&MFT_ENUM_HARDWARE_VENDOR_ID_Attribute)
                .map_err(|_| Error::Unavailable)?;
            if vendor_length > 128 {
                return Err(Error::Limit);
            }
            let mut vendor = vec![0u16; vendor_length as usize + 1];
            api(
                "MFT vendor",
                activation.GetString(&MFT_ENUM_HARDWARE_VENDOR_ID_Attribute, &mut vendor, None),
            )?;
            let vendor = String::from_utf16_lossy(&vendor[..vendor_length as usize]);
            if !matches!(
                vendor.to_ascii_uppercase().as_str(),
                "VEN_8086" | "8086" | "0X8086"
            ) || gpu.adapter_vendor != 0x8086
            {
                return Err(Error::Unavailable);
            }
            let transform: IMFTransform = api("activate HEVC", activation.ActivateObject())?;
            let activation = Activation(activation);
            let attributes = api("encoder attributes", transform.GetAttributes())?;
            let asynchronous = attributes.GetUINT32(&MF_TRANSFORM_ASYNC).unwrap_or(0) != 0;
            if asynchronous {
                api(
                    "async unlock",
                    attributes.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1),
                )?;
            }
            if attributes.GetUINT32(&MF_SA_D3D11_AWARE).unwrap_or(0) == 0 {
                return Err(unsupported("hardware MFT is not D3D11 aware"));
            }
            let (mut token, mut manager) = (0, None);
            api(
                "DXGI device manager",
                MFCreateDXGIDeviceManager(&mut token, &mut manager),
            )?;
            let manager = required(manager)?;
            api("DXGI reset device", manager.ResetDevice(&gpu.device, token))?;
            api(
                "encoder D3D manager",
                transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize),
            )?;
            let codec: ICodecAPI = api("ICodecAPI", transform.cast())?;
            let controls = [
                (
                    "low latency",
                    &CODECAPI_AVLowLatencyMode,
                    VARIANT::from(true),
                ),
                (
                    "zero B frames",
                    &CODECAPI_AVEncMPVDefaultBPictureCount,
                    VARIANT::from(0u32),
                ),
                (
                    "CBR rate control",
                    &CODECAPI_AVEncCommonRateControlMode,
                    VARIANT::from(0u32),
                ),
                (
                    "mean bitrate",
                    &CODECAPI_AVEncCommonMeanBitRate,
                    VARIANT::from(bitrate),
                ),
                (
                    "maximum bitrate",
                    &CODECAPI_AVEncCommonMaxBitRate,
                    VARIANT::from(bitrate),
                ),
            ];
            for (phase, key, value) in &controls {
                api(phase, codec.SetValue(*key, value))?;
            }
            let output = media_type(width, height, &MFVideoFormat_HEVC)?;
            api(
                "HEVC bitrate",
                output.SetUINT32(&MF_MT_AVG_BITRATE, bitrate),
            )?;
            api("HEVC profile", output.SetUINT32(&MF_MT_MPEG2_PROFILE, 1))?;
            api(
                "encoder output type",
                transform.SetOutputType(0, &output, 0),
            )?;
            let input = media_type(width, height, &MFVideoFormat_NV12)?;
            api("NV12 input", input.SetUINT32(&MF_MT_DEFAULT_STRIDE, width))?;
            api("encoder input type", transform.SetInputType(0, &input, 0))?;
            let video_device: ID3D11VideoDevice = api("D3D video device", gpu.device.cast())?;
            let video_context: ID3D11VideoContext = api("D3D video context", gpu.context.cast())?;
            let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: DXGI_RATIONAL {
                    Numerator: 30,
                    Denominator: 1,
                },
                InputWidth: target.frame_rect.width,
                InputHeight: target.frame_rect.height,
                OutputFrameRate: DXGI_RATIONAL {
                    Numerator: 30,
                    Denominator: 1,
                },
                OutputWidth: width,
                OutputHeight: height,
                Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
            };
            let enumerator = api(
                "video processor enumerator",
                video_device.CreateVideoProcessorEnumerator(&desc),
            )?;
            let processor = api(
                "video processor",
                video_device.CreateVideoProcessor(&enumerator, 0),
            )?;
            let source_rect = RECT {
                left: target.client_rect.x - target.frame_rect.x,
                top: target.client_rect.y - target.frame_rect.y,
                right: target.client_rect.x - target.frame_rect.x + target.client_rect.width as i32,
                bottom: target.client_rect.y - target.frame_rect.y
                    + target.client_rect.height as i32,
            };
            let rect = RECT {
                left: 0,
                top: 0,
                right: target.client_rect.width as i32,
                bottom: target.client_rect.height as i32,
            };
            let coded_rect = RECT {
                left: 0,
                top: 0,
                right: width as i32,
                bottom: height as i32,
            };
            let background = D3D11_VIDEO_COLOR {
                Anonymous: D3D11_VIDEO_COLOR_0 {
                    RGBA: D3D11_VIDEO_COLOR_RGBA {
                        R: 0.0,
                        G: 0.0,
                        B: 0.0,
                        A: 1.0,
                    },
                },
            };
            video_context.VideoProcessorSetOutputBackgroundColor(&processor, false, &background);
            let color_context: ID3D11VideoContext1 =
                api("video color context", video_context.cast())?;
            color_context.VideoProcessorSetStreamColorSpace1(
                &processor,
                0,
                DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
            );
            color_context.VideoProcessorSetOutputColorSpace1(
                &processor,
                DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
            );
            video_context.VideoProcessorSetStreamSourceRect(
                &processor,
                0,
                true,
                Some(&source_rect),
            );
            video_context.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&rect));
            video_context.VideoProcessorSetOutputTargetRect(&processor, true, Some(&coded_rect));
            video_context.VideoProcessorSetStreamFrameFormat(
                &processor,
                0,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            );
            let events = if asynchronous {
                Some(api("async encoder events", transform.cast())?)
            } else {
                None
            };
            api(
                "encoder begin streaming",
                transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0),
            )?;
            api(
                "encoder start stream",
                transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0),
            )?;
            Ok(Self {
                transform,
                _activation: activation,
                events,
                gpu_device: gpu.device.clone(),
                video_device,
                video_context,
                processor,
                enumerator,
                _manager: manager,
                width,
                height,
                pending_input: !asynchronous,
                pending_output: false,
                output_count: 0,
                name,
                vendor,
                adapter_vendor: gpu.adapter_vendor,
            })
        }
    }
    pub fn capabilities(&self) -> EncoderCapabilities {
        EncoderCapabilities {
            name: self.name.clone(),
            vendor_attribute: Some(self.vendor.clone()),
            adapter_vendor: self.adapter_vendor,
            hardware_enumerated: true,
            intel_vendor_confirmed: true,
            low_latency_control_accepted: true,
            zero_b_control_accepted: true,
            cbr_control_accepted: true,
            maximum_bitrate_control_accepted: true,
            bitrate: BITRATE,
            one_frame_in_flight: 1,
        }
    }
    pub fn force_keyframe(&self) -> Result<()> {
        // SAFETY: owned transform; control applies solely to its next input sample.
        unsafe {
            let codec: ICodecAPI = api("keyframe codec", self.transform.cast())?;
            api(
                "force IDR",
                codec.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &VARIANT::from(1u32)),
            )
        }
    }
    fn poll_events(&mut self) -> Result<()> {
        if let Some(events) = &self.events {
            // SAFETY: Nonblocking event retrieval on this owned asynchronous MFT.
            unsafe {
                loop {
                    let event = match events.GetEvent(MF_EVENT_FLAG_NO_WAIT) {
                        Ok(event) => event,
                        Err(e) if e.code() == MF_E_NO_EVENTS_AVAILABLE => break,
                        Err(e) => return api("encoder event", Err(e)),
                    };
                    api(
                        "encoder event status",
                        api("event status read", event.GetStatus())?.ok(),
                    )?;
                    let kind = api("encoder event type", event.GetType())?;
                    if kind == METransformNeedInput.0 as u32 {
                        self.pending_input = true;
                    }
                    if kind == METransformHaveOutput.0 as u32 {
                        self.pending_output = true;
                    }
                }
            }
        }
        Ok(())
    }
    fn convert(&self, input: &ID3D11Texture2D) -> Result<ID3D11Texture2D> {
        // SAFETY: Fresh output texture is not reused until the matching encoded
        // sample is received. GPU video processing performs BGRA-to-NV12 conversion.
        unsafe {
            let mut input_desc = D3D11_TEXTURE2D_DESC::default();
            input.GetDesc(&mut input_desc);
            if input_desc.Width == 0
                || input_desc.Height == 0
                || input_desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
            {
                return Err(Error::Invalid);
            }
            let output_desc = D3D11_TEXTURE2D_DESC {
                Width: self.width,
                Height: self.height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_NV12,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                ..Default::default()
            };
            let mut output = None;
            api(
                "NV12 GPU texture",
                self.gpu_device
                    .CreateTexture2D(&output_desc, None, Some(&mut output)),
            )?;
            let output = required(output)?;
            let input_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                FourCC: 0,
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPIV {
                        MipSlice: 0,
                        ArraySlice: 0,
                    },
                },
            };
            let output_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            };
            let (mut iv, mut ov) = (None, None);
            api(
                "video input view",
                self.video_device.CreateVideoProcessorInputView(
                    input,
                    &self.enumerator,
                    &input_desc,
                    Some(&mut iv),
                ),
            )?;
            api(
                "video output view",
                self.video_device.CreateVideoProcessorOutputView(
                    &output,
                    &self.enumerator,
                    &output_desc,
                    Some(&mut ov),
                ),
            )?;
            let ov = required(ov)?;
            let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: ManuallyDrop::new(iv),
                ..Default::default()
            };
            let result = api(
                "GPU color conversion",
                self.video_context.VideoProcessorBlt(
                    &self.processor,
                    &ov,
                    0,
                    std::slice::from_ref(&stream),
                ),
            );
            ManuallyDrop::drop(&mut stream.pInputSurface);
            result?;
            Ok(output)
        }
    }
    pub fn encode(&mut self, input: &ID3D11Texture2D, pts: i64) -> Result<Encoded> {
        let wait = Instant::now();
        while !self.pending_input {
            self.poll_events()?;
            pump();
            if wait.elapsed() > Duration::from_secs(5) {
                return Err(timeout("encoder input readiness"));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let nv12 = self.convert(input)?;
        // SAFETY: Sample owns its DXGI buffer and texture reference. Submit one
        // frame only; no reuse while asynchronous MFT work is outstanding.
        let sample = unsafe {
            let sample = api("MF sample", MFCreateSample())?;
            let buffer = api(
                "MF DXGI surface buffer",
                MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, &nv12, 0, false),
            )?;
            let b2: IMF2DBuffer = api("NV12 buffer size", buffer.cast())?;
            api(
                "NV12 current length",
                buffer.SetCurrentLength(api("NV12 contiguous length", b2.GetContiguousLength())?),
            )?;
            api("sample buffer", sample.AddBuffer(&buffer))?;
            api("sample time", sample.SetSampleTime(pts))?;
            api("sample duration", sample.SetSampleDuration(333_333))?;
            sample
        };
        let started = Instant::now();
        // SAFETY: Valid configured MFT/sample, matching stream 0. No aliased CPU data.
        unsafe {
            api(
                "HEVC ProcessInput",
                self.transform.ProcessInput(0, &sample, 0),
            )?;
        }
        self.pending_input = self.events.is_none();
        loop {
            pump();
            self.poll_events()?;
            if (self.events.is_none() || self.pending_output)
                && let Some((bytes, output_pts)) = self.output()?
            {
                if output_pts != pts {
                    return Err(unsupported(
                        "encoder reordered or mismatched frame timestamps",
                    ));
                }
                self.output_count += 1;
                return Ok(Encoded {
                    bytes,
                    pts: output_pts,
                });
            }
            if started.elapsed() > Duration::from_secs(5) {
                return Err(timeout("HEVC output at one frame in flight"));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn output(&mut self) -> Result<Option<(Vec<u8>, i64)>> {
        // SAFETY: Output sample/collection pointers are consumed exactly once on
        // both success and failure. Media buffer is unlocked before returning.
        unsafe {
            let info = api("output stream info", self.transform.GetOutputStreamInfo(0))?;
            let sample = if info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0 {
                None
            } else {
                if info.cbSize == 0 || info.cbSize as usize > MAX_ACCESS_UNIT {
                    return Err(Error::Invalid);
                }
                let sample = api("output sample", MFCreateSample())?;
                let buffer = api("output memory buffer", MFCreateMemoryBuffer(info.cbSize))?;
                api("output sample buffer", sample.AddBuffer(&buffer))?;
                Some(sample)
            };
            let mut data = MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: ManuallyDrop::new(sample),
                dwStatus: 0,
                pEvents: ManuallyDrop::new(None),
            };
            let mut status = 0;
            let result =
                self.transform
                    .ProcessOutput(0, std::slice::from_mut(&mut data), &mut status);
            let sample = ManuallyDrop::take(&mut data.pSample);
            let _events = ManuallyDrop::take(&mut data.pEvents);
            self.pending_output = false;
            match result {
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(None),
                value => api("HEVC ProcessOutput", value)?,
            }
            let sample = required(sample)?;
            let pts = api("output timestamp", sample.GetSampleTime())?;
            let buffer = api(
                "output contiguous buffer",
                sample.ConvertToContiguousBuffer(),
            )?;
            let (mut pointer, mut length) = (ptr::null_mut(), 0);
            api(
                "output buffer lock",
                buffer.Lock(&mut pointer, None, Some(&mut length)),
            )?;
            let bytes = if pointer.is_null() || length == 0 || length as usize > MAX_ACCESS_UNIT {
                Err(Error::Invalid)
            } else {
                Ok(std::slice::from_raw_parts(pointer, length as usize).to_vec())
            };
            let unlock = api("output buffer unlock", buffer.Unlock());
            let bytes = bytes?;
            unlock?;
            if !bytes.starts_with(&[0, 0, 1]) && !bytes.starts_with(&[0, 0, 0, 1]) {
                return Err(unsupported("MFT output is not Annex B"));
            }
            Ok(Some((bytes, pts)))
        }
    }
}
impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: Stop only this owned MFT; every submitted frame was handled or
        // failed explicitly. No success receipt is inferred from shutdown.
        unsafe {
            let _ = self
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
            let _ = self
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
        }
    }
}
fn media_type(width: u32, height: u32, subtype: &GUID) -> Result<IMFMediaType> {
    // SAFETY: Fresh media type with valid bounded dimensions and progressive 30 Hz.
    unsafe {
        let media = api("media type", MFCreateMediaType())?;
        api(
            "media major type",
            media.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video),
        )?;
        api("media subtype", media.SetGUID(&MF_MT_SUBTYPE, subtype))?;
        api(
            "media frame size",
            media.SetUINT64(&MF_MT_FRAME_SIZE, ((width as u64) << 32) | height as u64),
        )?;
        api(
            "media frame rate",
            media.SetUINT64(&MF_MT_FRAME_RATE, (30u64 << 32) | 1),
        )?;
        api(
            "pixel aspect",
            media.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1),
        )?;
        api(
            "progressive frames",
            media.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32),
        )?;
        Ok(media)
    }
}

fn unsupported(_: &str) -> Error {
    Error::Unavailable
}
fn timeout(_: &str) -> Error {
    Error::Timeout
}
