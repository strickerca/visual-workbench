//! Borrowed JPEG previews from classic TIFF-based RAW/DNG containers. No RAW
//! development, arbitrary byte scanning, maker-note interpretation or OS codec.
//! TIFF 6 / DNG: https://helpx.adobe.com/camera-raw/desktop/dng-and-file-formats/digital-negative.html
//! The retained original identity is always the whole container, never the JPEG.
use crate::admission::range;
use crate::{Error, Limits, Result, check};
use vw_model::AssetId;
use vw_raster::DecodedImage;

pub const PREVIEW_NOTICE: &str = "Rendered embedded RAW preview only; sensor data is retained unchanged. RAW development and full-resolution export are unavailable.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dng,
    Cr2,
    TiffCfa,
}
pub struct Preview<'a> {
    encoded: &'a [u8],
    source_asset: AssetId,
    kind: Kind,
    width: u32,
    height: u32,
    /// Orientation to apply once; decode reconciles the JPEG's own EXIF value.
    orientation: u8,
    estimated_peak_bytes: u64,
    srgb: bool,
    source_bytes: u64,
    profile: JpegProfile<'a>,
}

// Borrowed, bounded profile declaration. No ICC allocation before admission.
struct JpegProfile<'a> {
    chunks: [Option<&'a [u8]>; 255],
    count: u8,
    bytes: usize,
}
impl JpegProfile<'_> {
    fn complete(&self) -> Result<()> {
        if self.chunks[..usize::from(self.count)]
            .iter()
            .any(Option::is_none)
        {
            return Err(Error::Color);
        }
        Ok(())
    }
    fn matches(&self, decoded: Option<&[u8]>) -> bool {
        if self.count == 0 {
            return decoded.is_none();
        }
        let Some(decoded) = decoded else {
            return false;
        };
        if decoded.len() != self.bytes {
            return false;
        }
        let mut at = 0;
        for chunk in &self.chunks[..usize::from(self.count)] {
            let Some(chunk) = chunk else {
                return false;
            };
            if decoded.get(at..at + chunk.len()) != Some(*chunk) {
                return false;
            }
            at += chunk.len();
        }
        true
    }
}

struct Tiff<'a> {
    bytes: &'a [u8],
    le: bool,
}
impl<'a> Tiff<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self> {
        let le = match bytes.get(..2) {
            Some(b"II") => true,
            Some(b"MM") => false,
            _ => return Err(Error::Unsupported),
        };
        let t = Self { bytes, le };
        if t.u16(2)? != 42 {
            return Err(Error::Unsupported);
        }
        Ok(t)
    }
    fn u16(&self, at: usize) -> Result<u16> {
        let a = range(self.bytes, at as u64, 2)?
            .try_into()
            .map_err(|_| Error::Invalid)?;
        Ok(if self.le {
            u16::from_le_bytes(a)
        } else {
            u16::from_be_bytes(a)
        })
    }
    fn u32(&self, at: usize) -> Result<u32> {
        let a = range(self.bytes, at as u64, 4)?
            .try_into()
            .map_err(|_| Error::Invalid)?;
        Ok(if self.le {
            u32::from_le_bytes(a)
        } else {
            u32::from_be_bytes(a)
        })
    }
    fn entries(&self, offset: u32) -> Result<(usize, usize)> {
        if offset < 8 {
            return Err(Error::Invalid);
        }
        let start = offset as usize;
        let n = usize::from(self.u16(start)?);
        if n > 4096 {
            return Err(Error::Limit);
        }
        range(self.bytes, start as u64 + 2, (n * 12 + 4) as u64)?;
        Ok((start + 2, n))
    }
    fn values(&self, entry: usize) -> Result<(&'a [u8], usize)> {
        let size = match self.u16(entry + 2)? {
            1 => 1,
            3 => 2,
            4 => 4,
            _ => return Err(Error::Unsupported),
        };
        let count = self.u32(entry + 4)? as usize;
        if count == 0 || count > 64 {
            return Err(Error::Limit);
        }
        let n = count * size;
        let at = if n <= 4 {
            entry + 8
        } else {
            self.u32(entry + 8)? as usize
        };
        Ok((range(self.bytes, at as u64, n as u64)?, size))
    }
    fn scalar(&self, entry: usize) -> Result<u32> {
        let (b, size) = self.values(entry)?;
        if b.len() != size {
            return Err(Error::Unsupported);
        }
        self.number(b, size)
    }
    fn number(&self, b: &[u8], size: usize) -> Result<u32> {
        Ok(match size {
            1 => u32::from(b[0]),
            2 => u32::from(if self.le {
                u16::from_le_bytes(b.try_into().map_err(|_| Error::Invalid)?)
            } else {
                u16::from_be_bytes(b.try_into().map_err(|_| Error::Invalid)?)
            }),
            4 => {
                if self.le {
                    u32::from_le_bytes(b.try_into().map_err(|_| Error::Invalid)?)
                } else {
                    u32::from_be_bytes(b.try_into().map_err(|_| Error::Invalid)?)
                }
            }
            _ => return Err(Error::Invalid),
        })
    }
}

/// Identity by default; only the first TIFF IFD's orientation is relevant here.
pub(crate) fn exif_orientation(bytes: &[u8]) -> Result<u8> {
    let t = Tiff::new(bytes)?;
    let (start, n) = t.entries(t.u32(4)?)?;
    let mut orientation = None;
    for i in 0..n {
        let at = start + i * 12;
        if t.u16(at)? == 274 {
            if orientation.is_some() {
                return Err(Error::Invalid);
            }
            let value = t.scalar(at)?;
            if !(1..=8).contains(&value) {
                return Err(Error::Invalid);
            }
            orientation = Some(value as u8);
        }
    }
    Ok(orientation.unwrap_or(1))
}

#[derive(Default)]
struct Fields {
    subfile_type: Option<u32>,
    jpeg: Option<u32>,
    length: Option<u32>,
    strip: Option<u32>,
    strip_len: Option<u32>,
    compression: Option<u32>,
    photometric: Option<u32>,
    orientation: Option<u8>,
    color: Option<u32>,
    ifd_icc: bool,
}

struct Candidate<'a> {
    encoded: &'a [u8],
    width: u32,
    height: u32,
    orientation: u8,
    srgb: bool,
    peak: u64,
    profile: JpegProfile<'a>,
    ifd_icc: bool,
}

/// No allocation or codec call until the complete bounded IFD graph is admitted.
/// Selects the largest declared primary JPEG preview (NewSubFileType=1).
/// DNG 1.7.1 p20 alternatives/masks/enhanced images are never automatic choices.
/// Non-DNG legacy IFDs without NewSubFileType are eligible only if no primary
/// exists. No explicit alternative-rendering selector is exposed by this API.
/// Ordinary TIFFs, BigTIFF and proprietary non-TIFF RAW containers are refused.
pub fn embedded<'a>(
    bytes: &'a [u8],
    limits: Limits,
    cancel: &dyn Fn() -> bool,
) -> Result<Preview<'a>> {
    check(cancel)?;
    limits.input(bytes.len())?;
    let t = Tiff::new(bytes)?;
    let root_orientation = exif_orientation(bytes)?;
    let mut queue = [0_u32; 64];
    queue[0] = t.u32(4)?;
    let mut queued = 1;
    let mut dng = false;
    let mut cfa = false;
    let cr2 = bytes.get(8..12) == Some(b"CR\x02\0");
    let mut primary: Option<Candidate<'_>> = None;
    let mut legacy: Option<Candidate<'_>> = None;
    let mut entry_total = 0;
    let mut scanned_bytes = 0_u64;
    for index in 0..64 {
        if index == queued {
            break;
        }
        check(cancel)?;
        let (start, n) = t.entries(queue[index])?;
        entry_total += n;
        if entry_total > 4096 {
            return Err(Error::Limit);
        }
        let mut fields = Fields::default();
        let mut previous = None;
        for i in 0..n {
            if i % 32 == 0 {
                check(cancel)?;
            }
            let at = start + i * 12;
            let tag = t.u16(at)?;
            if previous.is_some_and(|p| p >= tag) {
                return Err(Error::Invalid);
            }
            previous = Some(tag);
            match tag {
                254 => {
                    if t.u16(at + 2)? != 4 {
                        return Err(Error::Invalid);
                    }
                    fields.subfile_type = Some(t.scalar(at)?);
                }
                262 => {
                    let v = t.scalar(at)?;
                    cfa |= matches!(v, 32803 | 34892);
                    fields.photometric = Some(v);
                }
                259 => fields.compression = Some(t.scalar(at)?),
                273 => {
                    if t.u32(at + 4)? == 1 {
                        fields.strip = Some(t.scalar(at)?);
                    }
                }
                274 => {
                    let v = t.scalar(at)?;
                    if !(1..=8).contains(&v) {
                        return Err(Error::Invalid);
                    }
                    fields.orientation = Some(v as u8);
                }
                279 => {
                    if t.u32(at + 4)? == 1 {
                        fields.strip_len = Some(t.scalar(at)?);
                    }
                }
                330 => {
                    if t.u16(at + 2)? != 4 {
                        return Err(Error::Unsupported);
                    }
                    let (b, size) = t.values(at)?;
                    for v in b.chunks_exact(size) {
                        enqueue(&mut queue, &mut queued, t.number(v, size)?)?;
                    }
                }
                513 => fields.jpeg = Some(t.scalar(at)?),
                514 => fields.length = Some(t.scalar(at)?),
                // A containing-IFD profile cannot be silently discarded or
                // treated as the JPEG's own profile. Selected tagged previews
                // explicitly refuse until reconciliation is implemented.
                34675 => fields.ifd_icc = true,
                50706 => {
                    if dng || t.u16(at + 2)? != 1 || t.u32(at + 4)? != 4 {
                        return Err(Error::Invalid);
                    }
                    let (v, _) = t.values(at)?;
                    if v[0] != 1 || v[1] > 7 {
                        return Err(Error::Unsupported);
                    }
                    dng = true;
                }
                // DNG PreviewColorSpace: support unknown (explicit untagged)
                // and sRGB only. Never label AdobeRGB/ProPhoto as sRGB.
                50970 => fields.color = Some(t.scalar(at)?),
                _ => {}
            }
        }
        enqueue(&mut queue, &mut queued, t.u32(start + n * 12)?)?;
        if fields.subfile_type.is_some_and(|v| v != 1) || (dng && fields.subfile_type != Some(1)) {
            continue;
        }
        let pair = match (fields.jpeg, fields.length) {
            (Some(a), Some(b)) => Some((a, b)),
            (None, None)
                if matches!(fields.compression, Some(6 | 7))
                    && matches!(fields.photometric, Some(2 | 6)) =>
            {
                fields.strip.zip(fields.strip_len)
            }
            (None, None) => None,
            _ => return Err(Error::Invalid),
        };
        if let Some((at, len)) = pair {
            scanned_bytes = scanned_bytes
                .checked_add(u64::from(len))
                .ok_or(Error::Limit)?;
            if scanned_bytes > limits.encoded_bytes {
                return Err(Error::Limit);
            }
            let preview = range(bytes, u64::from(at), u64::from(len))?;
            let (width, height, profile) = jpeg_extent(preview, cancel)?;
            let peak = limits.admit(width, height, bytes.len() as u64, 4 * 1024 * 1024)?;
            if fields.color.is_some_and(|v| v != 0 && v != 2) {
                return Err(Error::Color);
            }
            let area = u64::from(width) * u64::from(height);
            let best = if fields.subfile_type == Some(1) {
                &mut primary
            } else {
                &mut legacy
            };
            if best
                .as_ref()
                .is_none_or(|p| area > u64::from(p.width) * u64::from(p.height))
            {
                *best = Some(Candidate {
                    encoded: preview,
                    width,
                    height,
                    orientation: fields.orientation.unwrap_or(root_orientation),
                    srgb: fields.color == Some(2),
                    peak,
                    profile,
                    ifd_icc: fields.ifd_icc,
                });
            }
        }
    }
    let kind = if dng {
        Kind::Dng
    } else if cr2 {
        Kind::Cr2
    } else if cfa {
        Kind::TiffCfa
    } else {
        return Err(Error::Unsupported);
    };
    let Candidate {
        encoded,
        width,
        height,
        orientation,
        srgb,
        peak,
        profile,
        ifd_icc,
    } = primary
        .or(if dng { None } else { legacy })
        .ok_or(Error::Unsupported)?;
    if ifd_icc {
        return Err(Error::Color);
    }
    check(cancel)?;
    Ok(Preview {
        encoded,
        source_asset: AssetId::hash(bytes),
        kind,
        width,
        height,
        orientation,
        srgb,
        estimated_peak_bytes: peak,
        source_bytes: bytes.len() as u64,
        profile,
    })
}

fn enqueue(queue: &mut [u32; 64], len: &mut usize, value: u32) -> Result<()> {
    if value == 0 {
        return Ok(());
    }
    if queue[..*len].contains(&value) {
        return Err(Error::Invalid);
    }
    let target = queue.get_mut(*len).ok_or(Error::Limit)?;
    *target = value;
    *len += 1;
    Ok(())
}

// Admit extent, precision, channel count and bounded JPEG metadata before the
// existing raster decoder can allocate a native buffer/profile. No bytes search.
fn jpeg_extent<'a>(
    bytes: &'a [u8],
    cancel: &dyn Fn() -> bool,
) -> Result<(u32, u32, JpegProfile<'a>)> {
    if !bytes.starts_with(&[255, 216]) || !bytes.ends_with(&[255, 217]) {
        return Err(Error::Invalid);
    }
    let mut at = 2_usize;
    let mut extent = None;
    let mut profile = JpegProfile {
        chunks: [None; 255],
        count: 0,
        bytes: 0,
    };
    let mut scans = 0;
    for _ in 0..1024 {
        check(cancel)?;
        if bytes.get(at) != Some(&255) {
            return Err(Error::Invalid);
        }
        while bytes.get(at) == Some(&255) {
            at += 1;
        }
        let marker = *bytes.get(at).ok_or(Error::Invalid)?;
        at += 1;
        if marker == 0xd9 {
            return if at == bytes.len() && scans > 0 {
                profile.complete()?;
                let (width, height) = extent.ok_or(Error::Invalid)?;
                Ok((width, height, profile))
            } else {
                Err(Error::Invalid)
            };
        }
        if !matches!(
            marker,
            0xc0 | 0xc2 | 0xc4 | 0xda | 0xdb | 0xdd | 0xe0..=0xef | 0xfe
        ) {
            return Err(Error::Unsupported);
        }
        let b = range(bytes, at as u64, 2)?;
        let len = usize::from(u16::from_be_bytes([b[0], b[1]]));
        if len < 2 {
            return Err(Error::Invalid);
        }
        let data = range(bytes, at as u64 + 2, (len - 2) as u64)?;
        at = at.checked_add(len).ok_or(Error::Limit)?;
        match marker {
            0xc0 | 0xc2 => {
                if extent.is_some()
                    || data.len() < 6
                    || data[0] != 8
                    || !matches!(data[5], 1 | 3)
                    || data.len() != 6 + usize::from(data[5]) * 3
                {
                    return Err(Error::Unsupported);
                }
                let h = u32::from(u16::from_be_bytes([data[1], data[2]]));
                let w = u32::from(u16::from_be_bytes([data[3], data[4]]));
                if w == 0 || h == 0 {
                    return Err(Error::Invalid);
                }
                extent = Some((w, h));
            }
            0xda => {
                if extent.is_none()
                    || data.is_empty()
                    || !matches!(data[0], 1..=3)
                    || data.len() != 4 + usize::from(data[0]) * 2
                {
                    return Err(Error::Invalid);
                }
                scans += 1;
                // Scan entropy only to the next structural marker, accepting
                // stuffing and restarts. A later frame/DNL cannot revise extent.
                loop {
                    if at.is_multiple_of(65536) {
                        check(cancel)?;
                    }
                    let b = *bytes.get(at).ok_or(Error::Invalid)?;
                    if b != 255 {
                        at += 1;
                        continue;
                    }
                    let start = at;
                    while bytes.get(at) == Some(&255) {
                        at += 1;
                    }
                    let next = *bytes.get(at).ok_or(Error::Invalid)?;
                    if next == 0 || (0xd0..=0xd7).contains(&next) {
                        at += 1;
                    } else {
                        at = start;
                        break;
                    }
                }
            }
            0xe2 if data.starts_with(b"ICC_PROFILE") => {
                if data.len() <= 14 || !data.starts_with(b"ICC_PROFILE\0") {
                    return Err(Error::Color);
                }
                let sequence = data[12];
                let count = data[13];
                if count == 0
                    || sequence == 0
                    || sequence > count
                    || (profile.count != 0 && profile.count != count)
                {
                    return Err(Error::Color);
                }
                let slot = &mut profile.chunks[usize::from(sequence - 1)];
                if slot.is_some() {
                    return Err(Error::Color);
                }
                *slot = Some(&data[14..]);
                profile.count = count;
                profile.bytes = profile
                    .bytes
                    .checked_add(data.len() - 14)
                    .ok_or(Error::Limit)?;
                if profile.bytes > 4 * 1024 * 1024 {
                    return Err(Error::Color);
                }
            }
            0xe1 if data.starts_with(b"Exif\0\0") => {
                exif_orientation(&data[6..])?;
            }
            _ => {}
        }
    }
    Err(Error::Limit)
}

impl Preview<'_> {
    pub fn encoded(&self) -> &[u8] {
        self.encoded
    }
    pub fn source_asset(&self) -> &AssetId {
        &self.source_asset
    }
    pub fn kind(&self) -> Kind {
        self.kind
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn orientation(&self) -> u8 {
        self.orientation
    }
    pub fn estimated_peak_bytes(&self) -> u64 {
        self.estimated_peak_bytes
    }

    /// A preview is never a full-resolution export source. The result keeps the
    /// original container hash but original_available=false, preserving existing
    /// export refusal. Root must adopt it as an explicitly labelled preview.
    pub fn decode(&self, limits: Limits, cancel: &dyn Fn() -> bool) -> Result<DecodedImage> {
        check(cancel)?;
        limits.input(usize::try_from(self.source_bytes).map_err(|_| Error::Limit)?)?;
        limits.admit(self.width, self.height, self.source_bytes, 4 * 1024 * 1024)?;
        let mut image = vw_raster::decode(
            self.encoded,
            vw_raster::DecodeLimits {
                max_encoded_bytes: limits.encoded_bytes as usize,
                max_pixels: limits.pixels,
                max_memory_bytes: limits.memory_bytes.saturating_sub(self.source_bytes * 2),
            },
        )
        .map_err(|e| match e {
            vw_raster::RasterError::Color => Error::Color,
            vw_raster::RasterError::Allocation | vw_raster::RasterError::Memory { .. } => {
                Error::Allocation
            }
            _ => Error::Codec,
        })?;
        check(cancel)?;
        if !self.profile.matches(image.icc.as_deref()) {
            return Err(Error::Color);
        }
        let decoded_extent = if image.orientation_applied >= 5 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        };
        if (image.width, image.height) != decoded_extent {
            return Err(Error::Invalid);
        }
        if image.orientation_applied != 1
            && self.orientation != 1
            && image.orientation_applied != self.orientation
        {
            return Err(Error::Invalid);
        }
        let orientation = if image.orientation_applied != 1 {
            image.orientation_applied
        } else {
            self.orientation
        };
        if image.orientation_applied == 1 && orientation != 1 {
            let (w, h, pixels) = crate::orientation::apply(
                image.width,
                image.height,
                orientation,
                image.pixels,
                cancel,
            )?;
            image.width = w;
            image.height = h;
            image.pixels = pixels;
        }
        if self.srgb {
            let standard = vw_raster::standard_srgb_profile().map_err(|_| Error::Color)?;
            if image.icc.as_ref().is_some_and(|p| p != &standard) {
                return Err(Error::Color);
            }
            image.icc = Some(standard);
        }
        image.source_asset = self.source_asset.clone();
        image.original_available = false;
        image.orientation_applied = orientation;
        image.validate().map_err(|_| Error::Color)?;
        check(cancel)?;
        Ok(image)
    }
}
