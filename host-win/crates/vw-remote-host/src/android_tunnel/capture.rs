use super::{Error, Result, device::DeviceToken};

pub(crate) const MAX_PNG: usize = 16 * 1024 * 1024;
pub(crate) const MAX_XML: usize = 2 * 1024 * 1024;
const MAX_PIXELS: u64 = 16_000_000;
const MAX_DECODED: u64 = 64 * 1024 * 1024;
const MAX_XML_NODES: u32 = 10_000;
const MAX_XML_DEPTH: usize = 32;
const MAX_XML_TEXT: usize = 128 * 1024;

pub struct PngImage {
    pub(crate) encoded: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub decoded_budget: u64,
}

impl std::fmt::Debug for PngImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PngImage")
            .field("encoded_bytes", &self.encoded.len())
            .field("width", &self.width)
            .field("height", &self.height)
            .field("decoded_budget", &self.decoded_budget)
            .finish()
    }
}

impl PngImage {
    pub fn encoded(&self) -> &[u8] {
        &self.encoded
    }
}

/// Validate the complete bounded PNG container and memory envelope before any
/// image decoder receives its original bytes. Decoding belongs to root import.
pub(crate) fn png(bytes: Vec<u8>) -> Result<PngImage> {
    if bytes.len() > MAX_PNG {
        return Err(Error::Limit);
    }
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.len() < 45 {
        return Err(Error::Invalid);
    }
    let mut pos = 8usize;
    let mut dimensions = None;
    let mut ended = false;
    let mut idat = false;
    let mut chunks = 0usize;
    while pos < bytes.len() {
        chunks += 1;
        if chunks > 4096 || bytes.len() - pos < 12 {
            return Err(Error::Limit);
        }
        let length = u32::from_be_bytes(bytes[pos..pos + 4].try_into().map_err(|_| Error::Invalid)?)
            as usize;
        let end = pos
            .checked_add(length)
            .and_then(|v| v.checked_add(12))
            .ok_or(Error::Limit)?;
        if end > bytes.len() {
            return Err(Error::Invalid);
        }
        let kind = &bytes[pos + 4..pos + 8];
        let actual_crc =
            u32::from_be_bytes(bytes[end - 4..end].try_into().map_err(|_| Error::Invalid)?);
        if crc32(&bytes[pos + 4..end - 4]) != actual_crc {
            return Err(Error::Invalid);
        }
        if chunks == 1 {
            if kind != b"IHDR" || length != 13 {
                return Err(Error::Invalid);
            }
            let width = u32::from_be_bytes(
                bytes[pos + 8..pos + 12]
                    .try_into()
                    .map_err(|_| Error::Invalid)?,
            );
            let height = u32::from_be_bytes(
                bytes[pos + 12..pos + 16]
                    .try_into()
                    .map_err(|_| Error::Invalid)?,
            );
            let pixels = u64::from(width)
                .checked_mul(u64::from(height))
                .ok_or(Error::Limit)?;
            let decoded = pixels.checked_mul(4).ok_or(Error::Limit)?;
            if width == 0 || height == 0 || pixels > MAX_PIXELS || decoded > MAX_DECODED {
                return Err(Error::Limit);
            }
            if bytes[pos + 16] != 8
                || ![2, 6].contains(&bytes[pos + 17])
                || bytes[pos + 18..pos + 21] != [0, 0, 0]
            {
                return Err(Error::Invalid);
            }
            dimensions = Some((width, height, decoded));
        } else if kind == b"IHDR" {
            return Err(Error::Invalid);
        }
        if kind == b"IDAT" {
            idat = true;
        }
        if kind == b"IEND" {
            if length != 0 || end != bytes.len() {
                return Err(Error::Invalid);
            }
            ended = true;
        }
        pos = end;
    }
    let (width, height, decoded_budget) = dimensions.ok_or(Error::Invalid)?;
    if !ended || !idat || chunks < 3 {
        return Err(Error::Invalid);
    }
    Ok(PngImage {
        encoded: bytes,
        width,
        height,
        decoded_budget,
    })
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320u32 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

#[derive(Eq, PartialEq)]
pub struct SemanticElement {
    pub bounds: [u32; 4],
    pub class: String,
    pub resource_id: String,
    pub description: String,
    pub text: String,
}

pub enum Hierarchy {
    Present {
        elements: Vec<SemanticElement>,
        age_ms: u64,
    },
    Missing,
    Stale {
        age_ms: u64,
    },
}

impl std::fmt::Debug for Hierarchy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Present { elements, age_ms } => f
                .debug_struct("Present")
                .field("elements", &elements.len())
                .field("age_ms", age_ms)
                .finish(),
            Self::Missing => f.write_str("Missing"),
            Self::Stale { age_ms } => f.debug_struct("Stale").field("age_ms", age_ms).finish(),
        }
    }
}

pub struct CapturePair {
    pub session: [u8; 16],
    pub(crate) device: DeviceToken,
    pub source_generation: u64,
    pub image: PngImage,
    pub hierarchy: Hierarchy,
    pub image_started_ms: u64,
    pub image_finished_ms: u64,
    /// Host command completion time; it is not a phone photon timestamp.
    pub image_finished_timestamp_ns: u64,
    pub hierarchy_started_ms: Option<u64>,
    pub hierarchy_finished_ms: Option<u64>,
}

impl std::fmt::Debug for CapturePair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapturePair")
            .field("source_generation", &self.source_generation)
            .field("image", &self.image)
            .field("hierarchy", &self.hierarchy)
            .field("image_started_ms", &self.image_started_ms)
            .field("image_finished_ms", &self.image_finished_ms)
            .finish()
    }
}

impl CapturePair {
    pub fn device_token(&self) -> DeviceToken {
        self.device
    }
}

fn bounds(value: &str, width: u32, height: u32) -> Result<[u32; 4]> {
    let trimmed = value.strip_prefix('[').ok_or(Error::Invalid)?;
    let (left, tail) = trimmed.split_once("][").ok_or(Error::Invalid)?;
    let right = tail.strip_suffix(']').ok_or(Error::Invalid)?;
    let (x1, y1) = left.split_once(',').ok_or(Error::Invalid)?;
    let (x2, y2) = right.split_once(',').ok_or(Error::Invalid)?;
    let parse = |part: &str| part.parse::<u32>().map_err(|_| Error::Invalid);
    let (x1, y1, x2, y2) = (parse(x1)?, parse(y1)?, parse(x2)?, parse(y2)?);
    if x1 >= x2 || y1 >= y2 || x2 > width || y2 > height {
        return Err(Error::Invalid);
    }
    Ok([x1, y1, x2, y2])
}

/// roxmltree is pinned in the workspace lockfile. The root-owned manifest must
/// add its exact direct dependency when this module is registered.
pub(crate) fn hierarchy(bytes: &[u8], width: u32, height: u32) -> Result<Vec<SemanticElement>> {
    if bytes.len() > MAX_XML {
        return Err(Error::Limit);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Error::Invalid)?;
    let options = roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: MAX_XML_NODES,
        entity_resolver: None,
    };
    let document =
        roxmltree::Document::parse_with_options(text, options).map_err(|_| Error::Invalid)?;
    if document.root_element().tag_name().name() != "hierarchy" {
        return Err(Error::Invalid);
    }
    let mut elements = Vec::new();
    let mut total_text = 0usize;
    for node in document.root().descendants() {
        if node.ancestors().count() > MAX_XML_DEPTH {
            return Err(Error::Limit);
        }
        if !node.is_element() || node.tag_name().name() != "node" {
            continue;
        }
        if elements.len() >= MAX_XML_NODES as usize {
            return Err(Error::Limit);
        }
        let get = |name: &str| -> Result<String> {
            let value = node.attribute(name).unwrap_or("");
            if value.len() > 4096 || value.contains('\0') {
                return Err(Error::Limit);
            }
            Ok(value.to_owned())
        };
        let element = SemanticElement {
            bounds: bounds(
                node.attribute("bounds").ok_or(Error::Invalid)?,
                width,
                height,
            )?,
            class: get("class")?,
            resource_id: get("resource-id")?,
            description: get("content-desc")?,
            text: get("text")?,
        };
        total_text = total_text
            .checked_add(
                element.class.len()
                    + element.resource_id.len()
                    + element.description.len()
                    + element.text.len(),
            )
            .ok_or(Error::Limit)?;
        if total_text > MAX_XML_TEXT {
            return Err(Error::Limit);
        }
        elements.push(element);
    }
    Ok(elements)
}
