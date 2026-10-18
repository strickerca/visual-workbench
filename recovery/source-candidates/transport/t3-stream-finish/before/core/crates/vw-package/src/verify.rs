use crate::{
    Cancellation, Error, ImageMapping, Limits, Manifest, Result, Target, UNTRUSTED_TEXT_NOTICE,
    bounded, check, geometry,
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use vw_model::Id;

/// Byte-integrity checked package. This does not authenticate an imported
/// author's claims or grant permission to send/capture anything.
pub struct Package {
    manifest: Manifest,
    files: BTreeMap<String, Vec<u8>>,
    total_bytes: usize,
    manifest_sha256: String,
}
impl Package {
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }
    pub fn files(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.files.iter().map(|(p, b)| (p.as_str(), b.as_slice()))
    }
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }
    /// Input file names are data, never filesystem paths. Admission happens
    /// before deserialization or codec work; duplicate names fail explicitly.
    pub fn from_files(
        files: Vec<(String, Vec<u8>)>,
        limits: Limits,
        cancel: &dyn Cancellation,
    ) -> Result<Self> {
        limits.validate()?;
        check(cancel)?;
        if files.len() > limits.markers + 5 {
            return Err(Error::Limit("file count"));
        }
        let mut unique = BTreeMap::new();
        let mut total = 0usize;
        for (name, bytes) in files {
            check(cancel)?;
            path(&name)?;
            total = total
                .checked_add(bytes.len())
                .ok_or(Error::Limit("package size"))?;
            if total > limits.package_bytes {
                return Err(Error::Limit("package size"));
            }
            if unique.insert(name, bytes).is_some() {
                return Err(Error::Invalid("duplicate file"));
            }
        }
        from_compiler(unique, limits, 0, cancel)
    }
}
fn path(value: &str) -> Result<()> {
    if matches!(value, "manifest.json" | "prompt.md" | "semantic.json") {
        return Ok(());
    }
    let name = value
        .strip_prefix("images/")
        .and_then(|s| s.strip_suffix(".png"))
        .ok_or(Error::Invalid("package path"))?;
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
    {
        return Err(Error::Invalid("package path"));
    }
    Ok(())
}
fn timestamp(value: &str) -> bool {
    let b = value.as_bytes();
    if b.len() != 24
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'.'
        || b[23] != b'Z'
    {
        return false;
    }
    let part = |a, b| value.get(a..b).and_then(|v| v.parse::<u32>().ok());
    let (Some(y), Some(m), Some(d), Some(h), Some(min), Some(s), Some(_ms)) = (
        part(0, 4),
        part(5, 7),
        part(8, 10),
        part(11, 13),
        part(14, 16),
        part(17, 19),
        part(20, 23),
    ) else {
        return false;
    };
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => 0,
    };
    (1970..=9999).contains(&y) && (1..=days).contains(&d) && h < 24 && min < 60 && s < 60
}
fn text(value: &str, max: usize) -> Result<()> {
    if value.len() > max || value.contains('\0') {
        Err(Error::Limit("text"))
    } else {
        Ok(())
    }
}
fn bounds(b: [f64; 4], width: u32, height: u32) -> bool {
    b.iter().all(|v| v.is_finite())
        && b[0] >= 0.0
        && b[1] >= 0.0
        && b[2] >= 0.0
        && b[3] >= 0.0
        && b[0] + b[2] <= f64::from(width)
        && b[1] + b[3] <= f64::from(height)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SemanticData {
    schema_version: u32,
    project_id: Id,
    document_id: Id,
    #[serde(default)]
    snapshot_id: Option<Id>,
    #[serde(default)]
    platform: Option<vw_semantics::Platform>,
    #[serde(default)]
    frame_delta_ms: Option<i32>,
    text_is_untrusted: bool,
    untrusted_notice: String,
    elements: Vec<vw_semantics::Element>,
}
fn semantic(
    files: &BTreeMap<String, Vec<u8>>,
    m: &Manifest,
    cancel: &dyn Cancellation,
) -> Result<()> {
    let bytes = files.get("semantic.json").ok_or(Error::Integrity)?;
    if bytes.len() > vw_semantics::MAX_JSON_BYTES {
        return Err(Error::Limit("semantic JSON"));
    }
    crate::admission::semantic(bytes, cancel)?;
    let data: SemanticData = serde_json::from_slice(bytes).map_err(|_| Error::Integrity)?;
    if data.schema_version != 1
        || !data.text_is_untrusted
        || data.untrusted_notice != UNTRUSTED_TEXT_NOTICE
        || data.project_id != m.source.project_id
        || data.document_id != m.source.document_id
        || data.snapshot_id != m.extensions.semantic_snapshot
        || data.elements.len() > vw_semantics::MAX_ELEMENTS
    {
        return Err(Error::Integrity);
    }
    if data.snapshot_id.is_some() {
        if data.platform.is_none() || data.frame_delta_ms.is_none() || m.source.capture.is_none() {
            return Err(Error::Integrity);
        }
    } else if data.platform.is_some() || data.frame_delta_ms.is_some() || !data.elements.is_empty()
    {
        return Err(Error::Integrity);
    }
    let mut elements = BTreeMap::new();
    for e in &data.elements {
        text(&e.eid, 300)?;
        text(&e.name, 4096)?;
        text(&e.role, 256)?;
        text(&e.text, 800)?;
        for id in [&e.automation_id, &e.resource_id, &e.html_id]
            .into_iter()
            .flatten()
        {
            text(id, 1024)?;
        }
        if e.text.chars().count() > 200
            || !bounds(e.bounds_document, m.source.width, m.source.height)
            || elements.insert(e.eid.as_str(), e).is_some()
        {
            return Err(Error::Integrity);
        }
        let snapshot = data.snapshot_id.as_ref().ok_or(Error::Integrity)?;
        if !e.eid.starts_with(&format!("{snapshot}/")) {
            return Err(Error::Integrity);
        }
    }
    for e in &data.elements {
        let mut parent = e.parent.as_deref();
        let mut depth = 0;
        while let Some(id) = parent {
            depth += 1;
            if depth > vw_semantics::MAX_DEPTH || id == e.eid {
                return Err(Error::Integrity);
            }
            parent = elements.get(id).ok_or(Error::Integrity)?.parent.as_deref();
        }
    }
    for marker in &m.markers {
        let mut ids = BTreeSet::new();
        if marker.element_refs.len() > vw_semantics::MAX_REFERENCES {
            return Err(Error::Limit("references"));
        }
        for reference in &marker.element_refs {
            let e = elements
                .get(reference.eid.as_str())
                .ok_or(Error::Integrity)?;
            if !ids.insert(reference.eid.as_str())
                || data.platform.map(|p| p.as_str()) != Some(reference.platform.as_str())
                || reference.name != e.name
                || reference.role != e.role
                || reference.automation_id != e.automation_id
                || reference.resource_id != e.resource_id
                || reference.html_id != e.html_id
                || reference.bounds_document != e.bounds_document
            {
                return Err(Error::Integrity);
            }
        }
    }
    Ok(())
}
pub(crate) fn from_compiler(
    files: BTreeMap<String, Vec<u8>>,
    limits: Limits,
    retained: u64,
    cancel: &dyn Cancellation,
) -> Result<Package> {
    limits.validate()?;
    check(cancel)?;
    let working = limits.working(retained)?;
    if files.len() > limits.markers + 5 {
        return Err(Error::Limit("file count"));
    }
    let mut total = 0usize;
    let mut capacity = 0u64;
    for (name, bytes) in &files {
        path(name)?;
        total = total
            .checked_add(bytes.len())
            .ok_or(Error::Limit("package size"))?;
        capacity = capacity
            .checked_add(bytes.capacity() as u64)
            .ok_or(Error::Limit("package allocation"))?;
        if total > limits.package_bytes || capacity > limits.package_bytes as u64 * 2 {
            return Err(Error::Limit("package size"));
        }
    }
    let bytes = files.get("manifest.json").ok_or(Error::Integrity)?;
    if bytes.len() > bounded::MAX_TEXT {
        return Err(Error::Limit("manifest"));
    }
    crate::admission::manifest(bytes, limits, cancel)?;
    let m: Manifest = serde_json::from_slice(bytes).map_err(|_| Error::Integrity)?;
    let target = &m.extensions.target_profile;
    target.validate()?;
    if m.schema_version != "vip-1"
        || !timestamp(&m.created_at)
        || m.source.units != "px"
        || !matches!(m.source.document_kind.as_str(), "image" | "capture")
        || !bounded::hash_valid(&m.source.asset_hash)
        || !bounded::hash_valid(&m.extensions.state_hash)
        || m.source.width == 0
        || m.source.height == 0
        || u64::from(m.source.width) * u64::from(m.source.height) > limits.source_pixels
        || m.source.revision
            != format!(
                "r{}-{}",
                m.extensions.host_seq,
                &m.extensions.state_hash[..8]
            )
        || m.prompt_file != "prompt.md"
        || m.semantic_file != "semantic.json"
        || m.untrusted_text_notice != UNTRUSTED_TEXT_NOTICE
        || m.extensions.compiler != "vw-package-1-area-integer"
        || !matches!(m.extensions.source_bit_depth, 8 | 16)
        || m.extensions.derivative_bit_depth != 8
    {
        return Err(Error::Integrity);
    }
    let overview = ImageMapping::new(
        [0, 0, i64::from(m.source.width), i64::from(m.source.height)],
        target.max_long_edge(),
        64,
    )?;
    if m.extensions.overview_mapping != overview
        || m.compiled_for.target != target.name()
        || m.compiled_for.model.as_deref() != target.model()
        || m.compiled_for.max_long_edge != target.max_long_edge()
        || m.compiled_for.coordinate_convention != target.convention()
        || m.compiled_for.scale != overview.scale()
    {
        return Err(Error::Integrity);
    }
    if (m.source.document_kind == "capture") != m.source.capture.is_some() {
        return Err(Error::Integrity);
    }
    if let Some(c) = &m.source.capture {
        if !matches!(c.platform.as_str(), "windows" | "android" | "import")
            || !timestamp(&c.captured_at)
        {
            return Err(Error::Integrity);
        }
        text(&c.app_name, 4096)?;
        if let Some(title) = &c.window_title {
            text(title, 4096)?;
        }
    }
    if m.markers.len() > limits.markers
        || m.images.len() != m.markers.len() + 2
        || m.extensions.crops.len() != m.markers.len()
        || m.files.len() + 1 != files.len()
    {
        return Err(Error::Integrity);
    }
    let mut inventory = BTreeSet::new();
    for f in &m.files {
        if f.path == "manifest.json"
            || !inventory.insert(f.path.as_str())
            || !bounded::hash_valid(&f.sha256)
            || files
                .get(&f.path)
                .is_none_or(|b| bounded::hash(b) != f.sha256)
        {
            return Err(Error::Integrity);
        }
    }
    if !inventory.contains("prompt.md") || !inventory.contains("semantic.json") {
        return Err(Error::Integrity);
    }
    let mut image_ids = BTreeMap::new();
    let mut image_paths = BTreeSet::new();
    let standard_profile = crate::image_receipt::standard_profile(working)?;
    for i in &m.images {
        check(cancel)?;
        path(&i.path)?;
        if i.path != format!("images/{}.png", i.id)
            || i.width == 0
            || i.height == 0
            || i.width > target.max_long_edge()
            || i.height > target.max_long_edge()
            || i.color_space != "sRGB"
            || !image_paths.insert(i.path.as_str())
            || image_ids.insert(i.id.as_str(), i).is_some()
        {
            return Err(Error::Integrity);
        }
        let encoded = files.get(&i.path).ok_or(Error::Integrity)?;
        if encoded.len() > limits.image_bytes
            || bounded::hash(encoded) != i.sha256
            || !inventory.contains(i.path.as_str())
            || !encoded.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10])
        {
            return Err(Error::Integrity);
        }
        crate::image_receipt::verify(encoded, &m, i, cancel)?;
        let decoded = vw_raster::decode(
            encoded,
            vw_raster::DecodeLimits {
                max_encoded_bytes: limits.image_bytes,
                max_pixels: limits.source_pixels.max(256 * 256),
                max_memory_bytes: working,
            },
        )?;
        if decoded.width != i.width
            || decoded.height != i.height
            || decoded.pixels.bit_depth() != 8
            || decoded.orientation_applied != 1
            || decoded.icc.as_deref() != Some(standard_profile.as_slice())
        {
            return Err(Error::Integrity);
        }
    }
    for (id, role) in [("clean_source", "clean_source"), ("overview", "overview")] {
        let image = image_ids.get(id).ok_or(Error::Integrity)?;
        if image.role != role
            || image.marker.is_some()
            || image.width != overview.width
            || image.height != overview.height
        {
            return Err(Error::Integrity);
        }
    }
    if files.len() != m.images.len() + 3 {
        return Err(Error::Integrity);
    }
    for (index, (marker, crop)) in m.markers.iter().zip(&m.extensions.crops).enumerate() {
        check(cancel)?;
        let number = index as u32 + 1;
        if marker.number != number
            || crop.marker != number
            || !matches!(marker.kind.as_str(), "point" | "box")
            || !matches!(
                marker.role.as_str(),
                "change" | "preserve" | "reference" | "explain" | "none"
            )
        {
            return Err(Error::Integrity);
        }
        geometry::marker_bounds(
            marker.point_document,
            Some(marker.bbox_document),
            m.source.width,
            m.source.height,
        )?;
        if marker.bbox_compiled != overview.bounds(marker.bbox_document, target)
            || marker.point_compiled != overview.point(marker.point_document, target)
        {
            return Err(Error::Integrity);
        }
        let mapping = geometry::crop(
            marker.bbox_document,
            m.source.width,
            m.source.height,
            target.max_long_edge(),
        )?;
        if crop.mapping != mapping
            || crop.image != format!("marker_{number}")
            || marker.crop_image != crop.image
            || crop.bbox_crop_pixels
                != mapping.bounds(
                    marker.bbox_document,
                    &Target::Generic {
                        max_long_edge: 8192,
                    },
                )
        {
            return Err(Error::Integrity);
        }
        let image = image_ids.get(crop.image.as_str()).ok_or(Error::Integrity)?;
        if image.marker != Some(number)
            || image.role != "marker_crop"
            || image.width != mapping.width
            || image.height != mapping.height
        {
            return Err(Error::Integrity);
        }
        text(&marker.instruction, 32 * 1024)?;
    }
    semantic(&files, &m, cancel)?;
    let prompt = files.get("prompt.md").ok_or(Error::Integrity)?;
    if prompt.len() > bounded::MAX_TEXT
        || !std::str::from_utf8(prompt)
            .map_err(|_| Error::Integrity)?
            .ends_with(&format!("{UNTRUSTED_TEXT_NOTICE}\n"))
    {
        return Err(Error::Integrity);
    }
    let manifest_sha256 = bounded::hash(bytes);
    check(cancel)?;
    Ok(Package {
        manifest: m,
        files,
        total_bytes: total,
        manifest_sha256,
    })
}
