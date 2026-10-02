use crate::{AssetId, DeviceId, Id, ModelError, OrderKey, Project};
use std::collections::BTreeSet;
use vw_proto::v1;

impl Project {
    /// Validate supported kinds, finite values, references and acyclic groups.
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.schema_version != 1 {
            return Err(ModelError::UnsupportedSchema(self.schema_version));
        }
        text(&self.title, 4096)?;
        for (id, asset) in &self.assets {
            if id.as_str() != asset.asset_id {
                return Err(ModelError::Invalid("asset map key"));
            }
            validate_asset(asset)?;
        }
        for (id, document) in &self.documents {
            let definition = &document.definition;
            same_id(id, definition.document_id.as_ref())?;
            let kind = v1::DocumentKind::try_from(definition.kind)
                .map_err(|_| ModelError::UnsupportedKind(definition.kind))?;
            if matches!(
                kind,
                v1::DocumentKind::Unspecified | v1::DocumentKind::Timeline
            ) {
                return Err(ModelError::UnsupportedKind(definition.kind));
            }
            if definition.schema_version != 1 {
                return Err(ModelError::UnsupportedSchema(definition.schema_version));
            }
            text(&definition.title, 4096)?;
            self.asset_ref(&definition.primary_asset_id, true)?;
            match (&definition.capture, kind) {
                (Some(capture), v1::DocumentKind::Capture) => validate_capture(capture)?,
                (None, v1::DocumentKind::Capture) => {
                    return Err(ModelError::Missing("capture info"));
                }
                (Some(_), _) => return Err(ModelError::Invalid("capture document kind")),
                _ => {}
            }
            if kind != v1::DocumentKind::Pdf && !document.pages.is_empty() {
                return Err(ModelError::Invalid("PDF pages"));
            }
            let mut pages = BTreeSet::new();
            for page in &document.pages {
                if !pages.insert(page.page_index) {
                    return Err(ModelError::Invalid("duplicate PDF page"));
                }
                let crop = rectangle(&page.crop_box)?;
                vw_geom::PdfPage::new(
                    crop,
                    vw_geom::QuarterTurn::from_degrees(page.rotation_degrees)?,
                )?;
            }
        }
        for (id, layer) in &self.layers {
            same_id(id, layer.definition.layer_id.as_ref())?;
            let doc = self.document(layer.definition.document_id.as_ref())?;
            one_of(
                &layer.definition.kind,
                &["annotation", "mask", "result", "adjustment"],
            )?;
            OrderKey::try_from(layer.definition.order_key.clone())?;
            text(&layer.definition.name, 4096)?;
            range(layer.opacity, 0.0, 1.0)?;
            one_of(&layer.blend, &["normal", "multiply"])?;
            if layer.definition.page_index < -1
                || (doc.definition.kind != v1::DocumentKind::Pdf as i32
                    && layer.definition.page_index != -1)
            {
                return Err(ModelError::Invalid("layer page index"));
            }
            if layer.definition.page_index >= 0
                && !doc
                    .pages
                    .iter()
                    .any(|p| p.page_index == layer.definition.page_index as u32)
            {
                return Err(ModelError::Missing("layer PDF page"));
            }
        }
        for (id, group) in &self.groups {
            if id != &group.id || self.objects.contains_key(id) {
                return Err(ModelError::Invalid("group identity"));
            }
            if !self.documents.contains_key(&group.document_id) {
                return Err(ModelError::Missing("group document"));
            }
            let mut seen = BTreeSet::from([id.clone()]);
            let mut parent = group.parent.as_ref();
            while let Some(parent_id) = parent {
                if !seen.insert(parent_id.clone()) {
                    return Err(ModelError::Invalid("group cycle"));
                }
                let ancestor = self
                    .groups
                    .get(parent_id)
                    .ok_or(ModelError::Missing("parent group"))?;
                if ancestor.document_id != group.document_id {
                    return Err(ModelError::Invalid("cross-document group"));
                }
                parent = ancestor.parent.as_ref();
            }
        }
        for (id, object) in &self.objects {
            same_id(id, object.state.object_id.as_ref())?;
            validate_object_state(&object.state)?;
            if !self.documents.contains_key(&object.document_id) {
                return Err(ModelError::Missing("object document"));
            }
            let layer_id = Id::from_proto(object.state.layer_id.as_ref())?;
            let layer = self
                .layers
                .get(&layer_id)
                .ok_or(ModelError::Missing("object layer"))?;
            if Id::from_proto(layer.definition.document_id.as_ref())? != object.document_id {
                return Err(ModelError::Invalid("cross-document object layer"));
            }
            if let Some(group) = &object.state.group_id {
                let group = self
                    .groups
                    .get(&Id::from_proto(Some(group))?)
                    .ok_or(ModelError::Missing("object group"))?;
                if group.document_id != object.document_id {
                    return Err(ModelError::Invalid("cross-document object group"));
                }
            }
            if let Some(v1::object_state::Shape::SelectionRaster(mask)) = &object.state.shape {
                self.asset_ref(&mask.mask_asset_id, false)?;
            }
            if let Some(v1::object_state::Shape::ResultId(result)) = &object.state.shape {
                let result = self
                    .results
                    .get(&Id::from_proto(Some(result))?)
                    .ok_or(ModelError::Missing("object result"))?;
                if Id::from_proto(result.definition.document_id.as_ref())? != object.document_id {
                    return Err(ModelError::Invalid("cross-document result"));
                }
            }
        }
        for (id, instruction) in &self.instructions {
            let value = &instruction.definition;
            same_id(id, value.instruction_id.as_ref())?;
            self.document(value.document_id.as_ref())?;
            role(value.role)?;
            one_of(
                &value.entry_method,
                &["pc_keyboard", "phone_keyboard", "voice", "handwriting"],
            )?;
            text(&value.text, 1_000_000)?;
            text(&value.language, 128)?;
            // Deleted targets remain historical references so undo can restore
            // them; creation/update handlers reject never-existing target IDs.
            for target in &value.target_object_ids {
                Id::from_proto(Some(target))?;
            }
        }
        for (id, snapshot) in &self.semantic_snapshots {
            let value = &snapshot.definition;
            same_id(id, value.snapshot_id.as_ref())?;
            let document = self.document(value.document_id.as_ref())?;
            one_of(&value.platform, &["uia", "android_ax", "chromium_uia"])?;
            if value.elements_json_zstd.len() > 16 * 1024 * 1024 {
                return Err(ModelError::Invalid("semantic payload size"));
            }
            let capture = document.definition.capture.as_ref();
            let expected_session = capture
                .map(|info| Id::from_proto(info.capture_session_id.as_ref()))
                .transpose()?;
            if snapshot.capture_session_id != expected_session
                || snapshot.frame_id != capture.map(|info| info.frame_id)
            {
                return Err(ModelError::Invalid("semantic capture identity"));
            }
        }
        for (id, result) in &self.results {
            let value = &result.definition;
            same_id(id, value.result_id.as_ref())?;
            self.document(value.document_id.as_ref())?;
            if let Some(package) = &value.package_id {
                Id::from_proto(Some(package))?;
            }
            text(&value.provider, 256)?;
            text(&value.model, 256)?;
            if value.provider.is_empty() || value.model.is_empty() {
                return Err(ModelError::Invalid("result provider"));
            }
            nonnegative(value.cost_estimate_usd)?;
            json(&value.request_json, false)?;
            json(&value.metrics_json, true)?;
            let proof = json(&value.proof_json, true)?;
            one_of(
                &result.status,
                &[
                    "pending", "ready", "accepted", "rejected", "partial", "error",
                ],
            )?;
            self.asset_ref(&value.output_asset_id, true)?;
            self.asset_ref(&value.composite_asset_id, true)?;
            if let Some(mask) = &result.acceptance_mask_asset_id {
                self.asset_ref(mask.as_str(), false)?;
            }
            if matches!(result.status.as_str(), "ready" | "accepted" | "partial") {
                let proof = proof.ok_or(ModelError::Missing("result proof"))?;
                if proof
                    .get("changed_outside")
                    .and_then(serde_json::Value::as_u64)
                    != Some(0)
                {
                    return Err(ModelError::Invalid("changed-outside proof"));
                }
                self.asset_ref(&value.output_asset_id, false)?;
                self.asset_ref(&value.composite_asset_id, false)?;
            }
        }
        for (output, versions) in &self.mask_versions {
            // History may outlive the visible selection after deletion.
            for version in versions {
                same_id(output, version.output_object_id.as_ref())?;
                one_of(
                    &version.op,
                    &[
                        "add",
                        "subtract",
                        "intersect",
                        "invert",
                        "expand",
                        "shrink",
                        "feather",
                    ],
                )?;
                nonnegative(version.amount)?;
                for input in &version.input_object_ids {
                    Id::from_proto(Some(input))?;
                }
            }
        }
        Ok(())
    }

    fn asset_ref(&self, id: &str, optional: bool) -> Result<(), ModelError> {
        if optional && id.is_empty() {
            return Ok(());
        }
        if !self.assets.contains_key(&AssetId::try_from(id.to_owned())?) {
            return Err(ModelError::Missing("asset reference"));
        }
        Ok(())
    }
    fn document(&self, id: Option<&v1::Uuid>) -> Result<&crate::Document, ModelError> {
        self.documents
            .get(&Id::from_proto(id)?)
            .ok_or(ModelError::Missing("document reference"))
    }
}

/// Validate metadata; originals are addressed by hash, never overwritten here.
pub fn validate_asset(value: &v1::AddAsset) -> Result<(), ModelError> {
    AssetId::try_from(value.asset_id.clone())?;
    one_of(
        &value.format,
        &[
            "png", "jpeg", "jpg", "webp", "avif", "heic", "heif", "tiff", "bmp", "gif", "pdf",
            "svg", "mp4", "mov",
        ],
    )?;
    one_of(&value.source, &["camera", "import", "capture", "ai_result"])?;
    if !(1..=8).contains(&value.orientation) || value.bit_depth > 64 || value.byte_size == 0 {
        return Err(ModelError::Invalid("asset metadata"));
    }
    if !matches!(value.format.as_str(), "mp4" | "mov" | "pdf" | "svg")
        && (value.width == 0 || value.height == 0)
    {
        return Err(ModelError::Invalid("asset dimensions"));
    }
    if value.icc_profile.len() > 16 * 1024 * 1024 {
        return Err(ModelError::Invalid("ICC profile size"));
    }
    text(&value.color_space, 256)?;
    if let Some(metadata) = json(&value.metadata_json, true)? {
        let entries = metadata
            .as_object()
            .ok_or(ModelError::Invalid("metadata object"))?;
        for (key, value) in entries {
            one_of(
                key,
                &["description", "copyright", "software", "color_profile"],
            )?;
            // Metadata is a flat allowlist of descriptive strings. Allowing an
            // arbitrary object under an allowed key would preserve unfiltered
            // location/account fields hidden below that key.
            text(
                value
                    .as_str()
                    .ok_or(ModelError::Invalid("metadata value"))?,
                65_536,
            )?;
        }
    }
    Ok(())
}

/// Validate CaptureInfo and its full source/revision binding before storing it.
pub fn validate_capture(value: &v1::CaptureInfo) -> Result<(), ModelError> {
    Id::from_proto(value.capture_session_id.as_ref())?;
    one_of(&value.platform, &["windows", "android"])?;
    text(&value.app_name, 4096)?;
    text(&value.window_title, 4096)?;
    if value.lossless && value.degraded {
        return Err(ModelError::Invalid("capture quality flags"));
    }
    let geometry = value
        .geometry
        .as_ref()
        .ok_or(ModelError::Missing("capture geometry"))?;
    one_of(
        &geometry.source_kind,
        &["window", "monitor", "region", "virtual_display"],
    )?;
    text(&geometry.monitor_id, 4096)?;
    let rect = geometry
        .client_rect_host
        .as_ref()
        .ok_or(ModelError::Missing("capture client rectangle"))?;
    let width = u32::try_from(rect.w).map_err(|_| ModelError::Invalid("capture width"))?;
    let height = u32::try_from(rect.h).map_err(|_| ModelError::Invalid("capture height"))?;
    vw_geom::CaptureGeometry::new(
        vw_geom::HostRect::new(
            vw_geom::PixelPoint {
                x: rect.x,
                y: rect.y,
            },
            width,
            height,
        )?,
        geometry.dpi_scale,
        vw_geom::GeometryRevision(u64::from(geometry.geometry_revision)),
        vw_geom::QuarterTurn::Zero,
    )?;
    Ok(())
}

/// Validate all object variants without invoking renderers or changing originals.
pub fn validate_object_state(value: &v1::ObjectState) -> Result<(), ModelError> {
    Id::from_proto(value.object_id.as_ref())?;
    Id::from_proto(value.layer_id.as_ref())?;
    if let Some(group) = &value.group_id {
        Id::from_proto(Some(group))?;
    }
    DeviceId::try_from(value.created_by.clone())?;
    OrderKey::try_from(value.order_key.clone())?;
    role(value.role)?;
    affine(
        value
            .transform
            .as_ref()
            .ok_or(ModelError::Missing("object transform"))?,
    )?;
    style(
        value
            .style
            .as_ref()
            .ok_or(ModelError::Missing("object style"))?,
    )?;
    match value
        .shape
        .as_ref()
        .ok_or(ModelError::Missing("object shape"))?
    {
        v1::object_state::Shape::Stroke(stroke) => validate_stroke(stroke)?,
        v1::object_state::Shape::Line(line) => polyline(line, 2, Some(2))?,
        v1::object_state::Shape::Arrow(line) => polyline(line, 2, None)?,
        v1::object_state::Shape::Polygon(line) => polyline(line, 3, None)?,
        v1::object_state::Shape::SelectionVector(line) => polyline(line, 3, None)?,
        v1::object_state::Shape::Rect(rect)
        | v1::object_state::Shape::Ellipse(rect)
        | v1::object_state::Shape::Crop(rect) => {
            rectangle(rect)?;
        }
        v1::object_state::Shape::Text(value) => {
            text(&value.text, 1_000_000)?;
            text(&value.font_family, 256)?;
            if value.font_family.is_empty() || value.font_size <= 0.0 {
                return Err(ModelError::Invalid("text font"));
            }
            finite(value.font_size)?;
            point(
                value
                    .anchor
                    .as_ref()
                    .ok_or(ModelError::Missing("text anchor"))?,
            )?;
        }
        v1::object_state::Shape::Marker(value) => {
            if value.number == 0 {
                return Err(ModelError::Invalid("marker number"));
            }
            point(
                value
                    .point
                    .as_ref()
                    .ok_or(ModelError::Missing("marker point"))?,
            )?;
            if let Some(rect) = &value.r#box {
                rectangle(rect)?;
            }
            for eid in &value.element_eids {
                text(eid, 4096)?;
            }
        }
        v1::object_state::Shape::SelectionRaster(value) => {
            AssetId::try_from(value.mask_asset_id.clone())?;
            rectangle(
                value
                    .bounds
                    .as_ref()
                    .ok_or(ModelError::Missing("mask bounds"))?,
            )?;
            nonnegative(value.feather)?;
        }
        v1::object_state::Shape::ResultId(value) => {
            Id::from_proto(Some(value))?;
        }
        v1::object_state::Shape::Adjustment(value) => {
            range(value.brightness, -1.0, 1.0)?;
            range(value.contrast, -1.0, 1.0)?;
            if value.levels.len() != 5 {
                return Err(ModelError::Invalid("adjustment levels"));
            }
            for index in [0, 1, 3, 4] {
                range(value.levels[index], 0.0, 1.0)?;
            }
            finite(value.levels[2])?;
            if value.levels[0] >= value.levels[1]
                || value.levels[2] <= 0.0
                || value.levels[3] > value.levels[4]
            {
                return Err(ModelError::Invalid("adjustment levels"));
            }
        }
    }
    Ok(())
}

/// Validate a complete stroke or nonempty provisional sample delta.
/// Array lengths, finite samples and brush parameters share one contract.
pub fn validate_stroke(stroke: &v1::Stroke) -> Result<(), ModelError> {
    let count = stroke.x.len();
    if count == 0
        || count > 1_000_000
        || [stroke.y.len(), stroke.t_ms.len(), stroke.pressure.len()]
            .iter()
            .any(|n| *n != count)
        || (!stroke.tilt.is_empty() && stroke.tilt.len() != count)
        || (!stroke.orientation.is_empty() && stroke.orientation.len() != count)
        || stroke.t_ms.windows(2).any(|v| v[0] > v[1])
    {
        return Err(ModelError::Invalid("stroke samples"));
    }
    for (x, y) in stroke.x.iter().zip(&stroke.y) {
        vw_geom::Point::new(*x, *y)?;
    }
    for pressure in &stroke.pressure {
        range(f64::from(*pressure), 0.0, 1.0)?;
    }
    for tilt in &stroke.tilt {
        range(f64::from(*tilt), 0.0, std::f64::consts::FRAC_PI_2 + 1e-6)?;
    }
    for orientation in &stroke.orientation {
        finite(f64::from(*orientation))?;
    }
    let brush = stroke.brush.as_ref().ok_or(ModelError::Missing("brush"))?;
    one_of(
        &brush.family,
        &["pen", "marker", "highlighter", "vector_eraser"],
    )?;
    if brush.algorithm_version == 0 || brush.pressure_curve.len() != 8 {
        return Err(ModelError::Invalid("brush version/curve"));
    }
    nonnegative(brush.base_width)?;
    range(f64::from(brush.stabilization), 0.0, 1.0)?;
    for control in &brush.pressure_curve {
        range(*control, 0.0, 1.0)?;
    }
    Ok(())
}

pub(crate) fn affine(value: &v1::Affine) -> Result<(), ModelError> {
    vw_geom::Affine::new(value.a, value.b, value.c, value.d, value.e, value.f)?.inverse()?;
    Ok(())
}
pub(crate) fn style(value: &v1::Style) -> Result<(), ModelError> {
    nonnegative(value.width)?;
    if value.stroke.is_none() || (value.has_fill && value.fill.is_none()) {
        return Err(ModelError::Missing("style color"));
    }
    Ok(())
}
fn rectangle(value: &v1::RectD) -> Result<vw_geom::Rect, ModelError> {
    Ok(vw_geom::Rect::new(value.x, value.y, value.w, value.h)?)
}
fn point(value: &v1::PointD) -> Result<(), ModelError> {
    vw_geom::Point::new(value.x, value.y)?;
    Ok(())
}
fn polyline(
    value: &v1::Polyline,
    minimum: usize,
    maximum: Option<usize>,
) -> Result<(), ModelError> {
    if value.points.len() < minimum || value.points.len() > maximum.unwrap_or(1_000_000) {
        return Err(ModelError::Invalid("polyline count"));
    }
    for value in &value.points {
        point(value)?;
    }
    Ok(())
}
fn same_id(id: &Id, wire: Option<&v1::Uuid>) -> Result<(), ModelError> {
    if *id != Id::from_proto(wire)? {
        return Err(ModelError::Invalid("entity map key"));
    }
    Ok(())
}
fn role(value: i32) -> Result<(), ModelError> {
    if !(1..=5).contains(&value) {
        return Err(ModelError::Invalid("role"));
    }
    Ok(())
}
fn one_of(value: &str, allowed: &[&str]) -> Result<(), ModelError> {
    if !allowed.contains(&value) {
        return Err(ModelError::Invalid("enumerated field"));
    }
    Ok(())
}
fn text(value: &str, max: usize) -> Result<(), ModelError> {
    if value.len() > max || value.contains('\0') {
        return Err(ModelError::Invalid("text size/content"));
    }
    Ok(())
}
fn finite(value: f64) -> Result<(), ModelError> {
    if !value.is_finite() {
        return Err(ModelError::Invalid("nonfinite number"));
    }
    Ok(())
}
fn nonnegative(value: f64) -> Result<(), ModelError> {
    finite(value)?;
    if value < 0.0 {
        return Err(ModelError::Invalid("negative number"));
    }
    Ok(())
}
fn range(value: f64, minimum: f64, maximum: f64) -> Result<(), ModelError> {
    finite(value)?;
    if value < minimum || value > maximum {
        return Err(ModelError::Invalid("number range"));
    }
    Ok(())
}
fn json(value: &str, optional: bool) -> Result<Option<serde_json::Value>, ModelError> {
    if optional && value.is_empty() {
        return Ok(None);
    }
    text(value, 1_000_000)?;
    Ok(Some(serde_json::from_str(value)?))
}
