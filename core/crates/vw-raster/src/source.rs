use crate::{DecodeLimits, DecodedImage, Pixels, RasterError, Region, checked_samples};
use std::{
    fs::File,
    io::{Cursor, Read, Seek, SeekFrom, Write},
};
use vw_model::AssetId;

/// Full oriented source metadata. Region buffers keep this immutable identity.
#[derive(Debug, Clone)]
pub struct SourceInfo {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub icc: Option<Vec<u8>>,
    pub source_asset: AssetId,
    pub original_available: bool,
    pub orientation_applied: u8,
}
impl SourceInfo {
    pub fn validate(&self) -> Result<(), RasterError> {
        checked_samples(self.width, self.height)?;
        if !matches!(self.bit_depth, 8 | 16) || !(1..=8).contains(&self.orientation_applied) {
            return Err(RasterError::Invalid("source depth/orientation"));
        }
        if !self.original_available {
            return Err(RasterError::OriginalRequired);
        }
        if self.icc.as_ref().is_some_and(|p| p.len() > 4 * 1024 * 1024) {
            return Err(RasterError::Invalid("ICC size"));
        }
        if let Some(bytes) = &self.icc {
            let profile = crate::pixels::parse_color_profile(bytes)?;
            if !matches!(
                profile.color_space,
                moxcms::DataColorSpace::Rgb | moxcms::DataColorSpace::Gray
            ) {
                return Err(RasterError::Unsupported(
                    "ICC color space; RGB and grayscale are supported",
                ));
            }
        }
        Ok(())
    }
    pub(crate) fn image(&self, region: Region, pixels: Pixels) -> DecodedImage {
        DecodedImage {
            width: region.width,
            height: region.height,
            pixels,
            icc: self.icc.clone(),
            source_asset: self.source_asset.clone(),
            original_available: self.original_available,
            orientation_applied: self.orientation_applied,
        }
    }
}

/// Caller implementations must account all retained source allocations and the
/// largest extra row buffer. Read errors/cancellation must not return partial
/// regions. The supplied adapters enforce these rules without downsampling.
pub trait RasterSource {
    fn info(&self) -> &SourceInfo;
    fn resident_bytes(&self) -> u64;
    fn read_workspace_bytes(&self) -> u64;
    fn read_region(
        &mut self,
        region: Region,
        memory_budget: u64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<DecodedImage, RasterError>;
}

/// Borrows an already decoded original, avoiding a second full rendered image.
/// Its budget includes the complete retained source buffer and ICC allocation.
pub struct BorrowedSource<'a> {
    image: &'a DecodedImage,
    info: SourceInfo,
}
impl<'a> BorrowedSource<'a> {
    pub fn new(image: &'a DecodedImage) -> Result<Self, RasterError> {
        image.validate()?;
        if !image.original_available {
            return Err(RasterError::OriginalRequired);
        }
        Ok(Self {
            image,
            info: SourceInfo {
                width: image.width,
                height: image.height,
                bit_depth: image.pixels.bit_depth(),
                icc: image.icc.clone(),
                source_asset: image.source_asset.clone(),
                original_available: true,
                orientation_applied: image.orientation_applied,
            },
        })
    }
}
impl RasterSource for BorrowedSource<'_> {
    fn info(&self) -> &SourceInfo {
        &self.info
    }
    fn resident_bytes(&self) -> u64 {
        let pixels = match &self.image.pixels {
            Pixels::Rgba8(p) => p.capacity() as u64,
            Pixels::Rgba16(p) => p.capacity() as u64 * 2,
        };
        pixels
            + self.image.icc.as_ref().map_or(0, |p| p.capacity() as u64)
            + self.info.icc.as_ref().map_or(0, |p| p.capacity() as u64)
    }
    fn read_workspace_bytes(&self) -> u64 {
        0
    }
    fn read_region(
        &mut self,
        region: Region,
        budget: u64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<DecodedImage, RasterError> {
        region.validate(self.info.width, self.info.height)?;
        let bytes = region_bytes(region, self.info.bit_depth)?;
        check_memory(
            self.resident_bytes()
                .checked_add(bytes)
                .ok_or(RasterError::Allocation)?,
            budget,
        )?;
        let count = checked_samples(region.width, region.height)?;
        let pixels = match &self.image.pixels {
            Pixels::Rgba8(p) => {
                let mut out = reserve::<u8>(count)?;
                for y in region.y..region.y + region.height {
                    check_cancel(cancelled)?;
                    let start = ((u64::from(y) * u64::from(self.info.width) + u64::from(region.x))
                        * 4) as usize;
                    out.extend_from_slice(&p[start..start + region.width as usize * 4]);
                }
                Pixels::Rgba8(out)
            }
            Pixels::Rgba16(p) => {
                let mut out = reserve::<u16>(count)?;
                for y in region.y..region.y + region.height {
                    check_cancel(cancelled)?;
                    let start = ((u64::from(y) * u64::from(self.info.width) + u64::from(region.x))
                        * 4) as usize;
                    out.extend_from_slice(&p[start..start + region.width as usize * 4]);
                }
                Pixels::Rgba16(out)
            }
        };
        Ok(self.info.image(region, pixels))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SpoolLimits {
    pub decode: DecodeLimits,
    pub max_scratch_bytes: u64,
}
impl Default for SpoolLimits {
    fn default() -> Self {
        Self {
            decode: DecodeLimits::default(),
            max_scratch_bytes: 400_000_000,
        }
    }
}

/// Owns no path and never removes/publishes files. The caller supplies a fresh,
/// empty read/write file, keeps it private, and closes/removes it after dropping
/// this adapter on success, failure or cancellation. The original encoded input
/// is read-only. Scratch is raw un-oriented RGBA (16-bit samples big-endian).
pub struct PngSpool<'a> {
    file: &'a mut File,
    info: SourceInfo,
    raw_width: u32,
    raw_height: u32,
    input_reservation: u64,
    scratch_bytes: u64,
}
impl<'a> PngSpool<'a> {
    pub fn decode(
        bytes: &[u8],
        file: &'a mut File,
        limits: SpoolLimits,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, RasterError> {
        check_cancel(cancelled)?;
        if bytes.len() > limits.decode.max_encoded_bytes {
            return Err(RasterError::Invalid("encoded size"));
        }
        if bytes.get(..8) != Some(&[137, 80, 78, 71, 13, 10, 26, 10])
            || bytes.get(12..16) != Some(b"IHDR")
            || bytes.get(8..12) != Some(&13u32.to_be_bytes())
        {
            return Err(RasterError::Codec);
        }
        let read_u32 = |start| -> Result<u32, RasterError> {
            Ok(u32::from_be_bytes(
                bytes
                    .get(start..start + 4)
                    .ok_or(RasterError::Codec)?
                    .try_into()
                    .map_err(|_| RasterError::Codec)?,
            ))
        };
        let (width, height) = (read_u32(16)?, read_u32(20)?);
        checked_samples(width, height)?;
        if u64::from(width) * u64::from(height) > limits.decode.max_pixels {
            return Err(RasterError::Unsupported(
                "image exceeds pixel limit; split or use tiled import",
            ));
        }
        let depth = if bytes.get(24) == Some(&16) { 16 } else { 8 };
        let row_bytes = u64::from(width) * if depth == 16 { 8 } else { 4 };
        let decoder_bytes = 16 * 1024 * 1024 + row_bytes * 4;
        let estimated = bytes.len() as u64
            + decoder_bytes * 2
            + row_bytes * 2
            + crate::pixels::MAX_ICC_BYTES as u64
            + crate::pixels::MAX_ICC_PARSE_BYTES;
        check_memory(estimated, limits.decode.max_memory_bytes)?;
        if file.metadata().map_err(|_| RasterError::Io)?.len() != 0
            || file.stream_position().map_err(|_| RasterError::Io)? != 0
        {
            return Err(RasterError::Invalid("scratch must be fresh and empty"));
        }
        let icc = crate::pixels::png_profile_preflight(
            bytes,
            limits.decode.max_memory_bytes - bytes.len() as u64,
        )?;
        let mut decoder = png::Decoder::new_with_limits(
            Cursor::new(bytes),
            png::Limits {
                bytes: usize::try_from(decoder_bytes).map_err(|_| RasterError::Allocation)?,
            },
        );
        decoder.set_ignore_text_chunk(true);
        // Bounded preflight owns the admitted profile. Avoid a second expansion
        // or clone under this decoder's larger row-workspace allowance.
        decoder.set_ignore_iccp_chunk(true);
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().map_err(|_| RasterError::Codec)?;
        if reader.info().animation_control.is_some() {
            return Err(RasterError::Unsupported(
                "animated PNG requires an explicit frame import",
            ));
        }
        let (color, bits) = reader.output_color_type();
        let actual_depth = match bits {
            png::BitDepth::Eight => 8,
            png::BitDepth::Sixteen => 16,
            _ => return Err(RasterError::Codec),
        };
        if actual_depth != depth || reader.info().width != width || reader.info().height != height {
            return Err(RasterError::Codec);
        }
        let orientation = reader
            .info()
            .exif_metadata
            .as_ref()
            .and_then(|p| image::metadata::Orientation::from_exif_chunk(p))
            .map_or(1, |o| o.to_exif());
        let (oriented_width, oriented_height) = if orientation >= 5 {
            (height, width)
        } else {
            (width, height)
        };
        let info = SourceInfo {
            width: oriented_width,
            height: oriented_height,
            bit_depth: depth,
            icc,
            source_asset: AssetId::hash(bytes),
            original_available: true,
            orientation_applied: orientation,
        };
        info.validate()?;
        let gray = if let Some(p) = &info.icc {
            crate::pixels::parse_color_profile(p)?.color_space == moxcms::DataColorSpace::Gray
        } else {
            false
        };
        let scratch_bytes = row_bytes
            .checked_mul(u64::from(height))
            .ok_or(RasterError::Allocation)?;
        if scratch_bytes > limits.max_scratch_bytes {
            return Err(RasterError::ScratchLimit {
                limit: limits.max_scratch_bytes,
            });
        }
        file.set_len(scratch_bytes).map_err(|_| RasterError::Io)?;
        let pixel_bytes = if depth == 16 { 8 } else { 4 };
        if reader.info().interlaced {
            // Standard Adam7 pass coordinates. Partial rows are merged into one
            // existing scratch row, never a full decoded frame in RAM.
            for (index, (x0, y0, dx, dy)) in [
                (0u32, 0u32, 8u32, 8u32),
                (4, 0, 8, 8),
                (0, 4, 4, 8),
                (2, 0, 4, 4),
                (0, 2, 2, 4),
                (1, 0, 2, 2),
                (0, 1, 1, 2),
            ]
            .into_iter()
            .enumerate()
            {
                if x0 >= width || y0 >= height {
                    continue;
                }
                for (line, y) in (y0..height).step_by(dy as usize).enumerate() {
                    check_cancel(cancelled)?;
                    let row = reader
                        .next_interlaced_row()
                        .map_err(|_| RasterError::Codec)?
                        .ok_or(RasterError::Codec)?;
                    let expected = png::Adam7Info::new(index as u8 + 1, line as u32, width);
                    if !matches!(row.interlace(), png::InterlaceInfo::Adam7(actual) if *actual == expected)
                    {
                        return Err(RasterError::Codec);
                    }
                    let expanded = expand_row(row.data(), color, depth)?;
                    validate_gray(&expanded, depth, gray)?;
                    let mut merged = zeroes::<u8>(row_bytes as usize)?;
                    file.seek(SeekFrom::Start(u64::from(y) * row_bytes))
                        .map_err(|_| RasterError::Io)?;
                    file.read_exact(&mut merged).map_err(|_| RasterError::Io)?;
                    let positions = (x0..width).step_by(dx as usize);
                    if expanded.len() != positions.clone().count() * pixel_bytes {
                        return Err(RasterError::Codec);
                    }
                    for (x, p) in positions.zip(expanded.chunks_exact(pixel_bytes)) {
                        merged[x as usize * pixel_bytes..(x as usize + 1) * pixel_bytes]
                            .copy_from_slice(p);
                    }
                    file.seek(SeekFrom::Start(u64::from(y) * row_bytes))
                        .map_err(|_| RasterError::Io)?;
                    file.write_all(&merged).map_err(|_| RasterError::Io)?;
                }
            }
        } else {
            for y in 0..height {
                check_cancel(cancelled)?;
                let row = reader
                    .next_row()
                    .map_err(|_| RasterError::Codec)?
                    .ok_or(RasterError::Codec)?;
                let expanded = expand_row(row.data(), color, depth)?;
                if expanded.len() as u64 != row_bytes {
                    return Err(RasterError::Codec);
                }
                validate_gray(&expanded, depth, gray)?;
                file.seek(SeekFrom::Start(u64::from(y) * row_bytes))
                    .map_err(|_| RasterError::Io)?;
                file.write_all(&expanded).map_err(|_| RasterError::Io)?;
            }
        }
        if reader.next_row().map_err(|_| RasterError::Codec)?.is_some() {
            return Err(RasterError::Codec);
        }
        reader.finish().map_err(|_| RasterError::Codec)?;
        check_cancel(cancelled)?;
        file.flush().map_err(|_| RasterError::Io)?;
        Ok(Self {
            file,
            info,
            raw_width: width,
            raw_height: height,
            input_reservation: bytes.len() as u64,
            scratch_bytes,
        })
    }
    pub fn scratch_bytes(&self) -> u64 {
        self.scratch_bytes
    }
}

impl RasterSource for PngSpool<'_> {
    fn info(&self) -> &SourceInfo {
        &self.info
    }
    fn resident_bytes(&self) -> u64 {
        self.input_reservation + self.info.icc.as_ref().map_or(0, |p| p.capacity() as u64)
    }
    fn read_workspace_bytes(&self) -> u64 {
        u64::from(self.raw_width) * if self.info.bit_depth == 16 { 8 } else { 4 }
    }
    fn read_region(
        &mut self,
        region: Region,
        budget: u64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<DecodedImage, RasterError> {
        region.validate(self.info.width, self.info.height)?;
        check_memory(
            self.resident_bytes()
                + self.read_workspace_bytes()
                + region_bytes(region, self.info.bit_depth)?,
            budget,
        )?;
        let n = checked_samples(region.width, region.height)?;
        let mut pixels = if self.info.bit_depth == 16 {
            Pixels::Rgba16(zeroes(n)?)
        } else {
            Pixels::Rgba8(zeroes(n)?)
        };
        let raw = |x, y| {
            oriented_to_raw(
                x,
                y,
                self.raw_width,
                self.raw_height,
                self.info.orientation_applied,
            )
        };
        let a = raw(region.x, region.y);
        let b = raw(region.x + region.width - 1, region.y + region.height - 1);
        let (left, right, top, bottom) = (a.0.min(b.0), a.0.max(b.0), a.1.min(b.1), a.1.max(b.1));
        let pixel_bytes = if self.info.bit_depth == 16 { 8 } else { 4 };
        let mut row = zeroes::<u8>((right - left + 1) as usize * pixel_bytes)?;
        for y in top..=bottom {
            check_cancel(cancelled)?;
            self.file
                .seek(SeekFrom::Start(
                    (u64::from(y) * u64::from(self.raw_width) + u64::from(left))
                        * pixel_bytes as u64,
                ))
                .map_err(|_| RasterError::Io)?;
            self.file
                .read_exact(&mut row)
                .map_err(|_| RasterError::Io)?;
            for (x, pixel) in (left..=right).zip(row.chunks_exact(pixel_bytes)) {
                let (ox, oy) = raw_to_oriented(
                    x,
                    y,
                    self.raw_width,
                    self.raw_height,
                    self.info.orientation_applied,
                );
                let at = ((u64::from(oy - region.y) * u64::from(region.width)
                    + u64::from(ox - region.x))
                    * 4) as usize;
                match &mut pixels {
                    Pixels::Rgba8(out) => out[at..at + 4].copy_from_slice(pixel),
                    Pixels::Rgba16(out) => {
                        for c in 0..4 {
                            out[at + c] = u16::from_be_bytes([pixel[c * 2], pixel[c * 2 + 1]]);
                        }
                    }
                }
            }
        }
        Ok(self.info.image(region, pixels))
    }
}

/// Baseline, single interleaved scan JPEG adapter. The pinned decoder writes a
/// compact RGB/Luma frame; row conversion goes directly to caller-owned scratch
/// instead of allocating a second full RGBA frame. Progressive/multi-scan input
/// is refused before constructing its full coefficient-plane decoder.
pub struct JpegSpool<'a>(PngSpool<'a>);
impl<'a> JpegSpool<'a> {
    pub fn decode(
        bytes: &[u8],
        file: &'a mut File,
        limits: SpoolLimits,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, RasterError> {
        use image::ImageDecoder;
        check_cancel(cancelled)?;
        if bytes.len() > limits.decode.max_encoded_bytes {
            return Err(RasterError::Invalid("encoded size"));
        }
        let (width, height, channels) = baseline_jpeg_header(bytes, cancelled)?;
        checked_samples(width, height)?;
        let pixels = u64::from(width) * u64::from(height);
        if pixels > limits.decode.max_pixels {
            return Err(RasterError::Unsupported(
                "image exceeds pixel limit; split or use tiled import",
            ));
        }
        // image's JPEG wrapper retains encoded bytes, zune parses APP metadata,
        // and baseline decoding retains MCU-row workspaces. Only the admitted
        // 1x/2x sampling and <=3 components are covered by this estimate. Four
        // encoded lengths cover input/copy/header parsing, with 32MiB for codec,
        // ICC and allocator slack plus a deliberately generous 4096B/source-x.
        let estimate = pixels * u64::from(channels)
            + bytes.len() as u64 * 4
            + u64::from(width) * 4096
            + 32 * 1024 * 1024;
        check_memory(estimate, limits.decode.max_memory_bytes)?;
        let scratch_bytes = pixels * 4;
        if scratch_bytes > limits.max_scratch_bytes {
            return Err(RasterError::ScratchLimit {
                limit: limits.max_scratch_bytes,
            });
        }
        if file.metadata().map_err(|_| RasterError::Io)?.len() != 0
            || file.stream_position().map_err(|_| RasterError::Io)? != 0
        {
            return Err(RasterError::Invalid("scratch must be fresh and empty"));
        }
        let mut decoder = image::codecs::jpeg::JpegDecoder::new(Cursor::new(bytes))
            .map_err(|_| RasterError::Codec)?;
        if decoder.dimensions() != (width, height) {
            return Err(RasterError::Codec);
        }
        let color = match decoder.color_type() {
            image::ColorType::L8 if channels == 1 => png::ColorType::Grayscale,
            image::ColorType::Rgb8 if channels == 3 => png::ColorType::Rgb,
            _ => {
                return Err(RasterError::Unsupported(
                    "bounded JPEG requires grayscale or RGB baseline output",
                ));
            }
        };
        let icc = decoder.icc_profile().map_err(|_| RasterError::Codec)?;
        let color_bytes = icc.as_ref().map_or(Ok(0), |profile| {
            crate::pixels::admit_icc(profile).and_then(|parsed| {
                parsed
                    .checked_add(profile.capacity() as u64)
                    .ok_or(RasterError::Allocation)
            })
        })?;
        check_memory(
            estimate
                .checked_add(color_bytes)
                .ok_or(RasterError::Allocation)?,
            limits.decode.max_memory_bytes,
        )?;
        let orientation = decoder
            .orientation()
            .map_err(|_| RasterError::Codec)?
            .to_exif();
        let (oriented_width, oriented_height) = if orientation >= 5 {
            (height, width)
        } else {
            (width, height)
        };
        let info = SourceInfo {
            width: oriented_width,
            height: oriented_height,
            bit_depth: 8,
            icc,
            source_asset: AssetId::hash(bytes),
            original_available: true,
            orientation_applied: orientation,
        };
        info.validate()?;
        let gray = info
            .icc
            .as_ref()
            .map(|p| {
                crate::pixels::parse_color_profile(p)
                    .map(|p| p.color_space == moxcms::DataColorSpace::Gray)
            })
            .transpose()?
            .unwrap_or(false);
        let mut decoded = zeroes::<u8>(
            usize::try_from(decoder.total_bytes()).map_err(|_| RasterError::Allocation)?,
        )?;
        // The upstream decoder is synchronous, so cancellation is checked before
        // and immediately after this bounded single-frame call, then every row.
        check_cancel(cancelled)?;
        decoder
            .read_image(&mut decoded)
            .map_err(|_| RasterError::Codec)?;
        check_cancel(cancelled)?;
        for row in decoded.chunks_exact(width as usize * channels as usize) {
            check_cancel(cancelled)?;
            let rgba = expand_row(row, color, 8)?;
            validate_gray(&rgba, 8, gray)?;
            file.write_all(&rgba).map_err(|_| RasterError::Io)?;
        }
        file.flush().map_err(|_| RasterError::Io)?;
        if file.metadata().map_err(|_| RasterError::Io)?.len() != scratch_bytes {
            return Err(RasterError::Io);
        }
        Ok(Self(PngSpool {
            file,
            info,
            raw_width: width,
            raw_height: height,
            input_reservation: bytes.len() as u64,
            scratch_bytes,
        }))
    }
    pub fn scratch_bytes(&self) -> u64 {
        self.0.scratch_bytes()
    }
}
impl RasterSource for JpegSpool<'_> {
    fn info(&self) -> &SourceInfo {
        self.0.info()
    }
    fn resident_bytes(&self) -> u64 {
        self.0.resident_bytes()
    }
    fn read_workspace_bytes(&self) -> u64 {
        self.0.read_workspace_bytes()
    }
    fn read_region(
        &mut self,
        r: Region,
        budget: u64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<DecodedImage, RasterError> {
        self.0.read_region(r, budget, cancelled)
    }
}
fn baseline_jpeg_header(
    bytes: &[u8],
    cancelled: &dyn Fn() -> bool,
) -> Result<(u32, u32, u8), RasterError> {
    if bytes.get(..2) != Some(&[0xff, 0xd8]) {
        return Err(RasterError::Codec);
    }
    let mut position = 2;
    let mut frame = None;
    for _ in 0..65536 {
        check_cancel(cancelled)?;
        if bytes.get(position) != Some(&0xff) {
            return Err(RasterError::Codec);
        }
        while bytes.get(position) == Some(&0xff) {
            position += 1;
        }
        let marker = *bytes.get(position).ok_or(RasterError::Codec)?;
        position += 1;
        let length = u16::from_be_bytes(
            bytes
                .get(position..position + 2)
                .ok_or(RasterError::Codec)?
                .try_into()
                .map_err(|_| RasterError::Codec)?,
        ) as usize;
        if length < 2 {
            return Err(RasterError::Codec);
        }
        let end = position
            .checked_add(length)
            .ok_or(RasterError::Allocation)?;
        let body = bytes.get(position + 2..end).ok_or(RasterError::Codec)?;
        match marker {
            0xc0 => {
                if frame.is_some() || body.len() < 6 || body[0] != 8 {
                    return Err(RasterError::Codec);
                }
                let channels = body[5];
                if !matches!(channels, 1 | 3) || body.len() != 6 + usize::from(channels) * 3 {
                    return Err(RasterError::Unsupported(
                        "bounded JPEG requires baseline 1 or 3 components",
                    ));
                }
                for c in body[6..].as_chunks::<3>().0.iter() {
                    if !matches!(c[1] >> 4, 1 | 2) || !matches!(c[1] & 15, 1 | 2) {
                        return Err(RasterError::Unsupported(
                            "bounded JPEG supports 1x and 2x sampling",
                        ));
                    }
                }
                let height = u32::from(u16::from_be_bytes([body[1], body[2]]));
                let width = u32::from(u16::from_be_bytes([body[3], body[4]]));
                if width == 0 || height == 0 {
                    return Err(RasterError::Codec);
                }
                frame = Some((width, height, channels));
            }
            0xda => {
                let dimensions = frame.ok_or(RasterError::Unsupported(
                    "bounded JPEG requires baseline single-scan encoding",
                ))?;
                if body.first() != Some(&dimensions.2)
                    || body.len() != 4 + usize::from(dimensions.2) * 2
                    || body[body.len() - 3..] != [0, 63, 0]
                {
                    return Err(RasterError::Unsupported(
                        "bounded JPEG requires one complete interleaved scan",
                    ));
                }
                position = end;
                while position < bytes.len() {
                    if position.is_multiple_of(65536) {
                        check_cancel(cancelled)?;
                    }
                    if bytes[position] != 0xff {
                        position += 1;
                        continue;
                    }
                    position += 1;
                    while bytes.get(position) == Some(&0xff) {
                        position += 1;
                    }
                    let next = *bytes.get(position).ok_or(RasterError::Codec)?;
                    position += 1;
                    match next {
                        0x00 | 0xd0..=0xd7 => {}
                        0xd9 if position == bytes.len() => return Ok(dimensions),
                        _ => {
                            return Err(RasterError::Unsupported(
                                "bounded JPEG requires a single scan without trailing data",
                            ));
                        }
                    }
                }
                return Err(RasterError::Codec);
            }
            0xc4 | 0xdb | 0xdd | 0xe0..=0xef | 0xfe => {}
            _ => {
                return Err(RasterError::Unsupported(
                    "progressive or unusual JPEG requires buffered decoding or a larger budget",
                ));
            }
        }
        position = end;
    }
    Err(RasterError::Invalid("JPEG header segment limit"))
}

fn raw_to_oriented(x: u32, y: u32, w: u32, h: u32, orientation: u8) -> (u32, u32) {
    match orientation {
        2 => (w - 1 - x, y),
        3 => (w - 1 - x, h - 1 - y),
        4 => (x, h - 1 - y),
        5 => (y, x),
        6 => (h - 1 - y, x),
        7 => (h - 1 - y, w - 1 - x),
        8 => (y, w - 1 - x),
        _ => (x, y),
    }
}
fn oriented_to_raw(x: u32, y: u32, w: u32, h: u32, orientation: u8) -> (u32, u32) {
    match orientation {
        2 => (w - 1 - x, y),
        3 => (w - 1 - x, h - 1 - y),
        4 => (x, h - 1 - y),
        5 => (y, x),
        6 => (y, h - 1 - x),
        7 => (w - 1 - y, h - 1 - x),
        8 => (w - 1 - y, x),
        _ => (x, y),
    }
}
fn expand_row(row: &[u8], color: png::ColorType, depth: u8) -> Result<Vec<u8>, RasterError> {
    let sample_bytes = usize::from(depth / 8);
    let channels = match color {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        _ => return Err(RasterError::Codec),
    };
    let stride = channels * sample_bytes;
    if !row.len().is_multiple_of(stride) {
        return Err(RasterError::Codec);
    }
    let mut out = reserve(row.len() / stride * sample_bytes * 4)?;
    for p in row.chunks_exact(stride) {
        if channels <= 2 {
            for _ in 0..3 {
                out.extend_from_slice(&p[..sample_bytes]);
            }
        } else {
            out.extend_from_slice(&p[..sample_bytes * 3]);
        }
        if channels == 2 || channels == 4 {
            out.extend_from_slice(&p[(channels - 1) * sample_bytes..]);
        } else {
            out.extend_from_slice(&[255, 255][..sample_bytes]);
        }
    }
    Ok(out)
}
fn validate_gray(bytes: &[u8], depth: u8, gray: bool) -> Result<(), RasterError> {
    if gray {
        let s = usize::from(depth / 8);
        if bytes
            .chunks_exact(s * 4)
            .any(|p| p[..s] != p[s..2 * s] || p[..s] != p[2 * s..3 * s])
        {
            return Err(RasterError::Invalid("grayscale ICC with colored pixels"));
        }
    }
    Ok(())
}
pub(crate) fn region_bytes(r: Region, depth: u8) -> Result<u64, RasterError> {
    u64::from(r.width)
        .checked_mul(u64::from(r.height))
        .and_then(|v| v.checked_mul(if depth == 16 { 8 } else { 4 }))
        .ok_or(RasterError::Allocation)
}
pub(crate) fn check_memory(estimated: u64, budget: u64) -> Result<(), RasterError> {
    if estimated > budget {
        Err(RasterError::Memory { estimated, budget })
    } else {
        Ok(())
    }
}
pub(crate) fn check_cancel(cancelled: &dyn Fn() -> bool) -> Result<(), RasterError> {
    if cancelled() {
        Err(RasterError::Cancelled)
    } else {
        Ok(())
    }
}
pub(crate) fn reserve<T>(count: usize) -> Result<Vec<T>, RasterError> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(count)
        .map_err(|_| RasterError::Allocation)?;
    Ok(result)
}
pub(crate) fn zeroes<T: Default + Clone>(count: usize) -> Result<Vec<T>, RasterError> {
    let mut result = reserve(count)?;
    result.resize(count, T::default());
    Ok(result)
}
