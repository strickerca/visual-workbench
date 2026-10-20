use super::{checked::*, *};
use crate::{project::ProjectState, workflow};
use std::collections::BTreeMap;
use vw_instructions::{
    DocumentView, EditMetadata, EntryMethod, InstructionEdit, MarkerPlacement, Role,
};
use vw_model::Id;
use vw_proto::v1;

pub(super) struct MarkerAuthoring {
    pub original_number: u32,
    pub style: v1::Style,
    pub entry_method: EntryMethod,
    pub language: String,
}
pub(super) struct OriginalAuthoring {
    pub markers: Vec<MarkerAuthoring>,
    pub global: Option<(Role, EntryMethod, String)>,
}
pub(super) fn authoring(
    state: &ProjectState,
    package: &vw_package::Package,
    failed: &[u32],
) -> PackageResult<OriginalAuthoring> {
    source_current(state, package)?;
    let revision = state.view_revision()?;
    let view = DocumentView::new(
        state.project()?,
        &revision,
        &package.manifest().source.document_id,
    )
    .map_err(instruction_error)?;
    let export = vw_instructions::InstructionExport::from_view(&view).map_err(instruction_error)?;
    let mut markers = Vec::new();
    for number in failed {
        let marker = export
            .markers()
            .get(*number as usize - 1)
            .ok_or(PackageError::Invalid)?;
        let instruction = view
            .instruction(&marker.instruction_id)
            .map_err(instruction_error)?;
        let object = view.object(&marker.object_id).map_err(instruction_error)?;
        markers.push(MarkerAuthoring {
            original_number: *number,
            style: object.state.style.ok_or(PackageError::Integrity)?,
            entry_method: EntryMethod::from_canonical(&instruction.definition.entry_method)
                .map_err(instruction_error)?,
            language: instruction.definition.language.clone(),
        });
    }
    Ok(OriginalAuthoring {
        markers,
        global: export
            .global()
            .map(|g| (g.role, g.entry_method, g.language.clone())),
    })
}
pub(super) struct TranslatedGeometry {
    pub point: [f64; 2],
    pub bounds: [f64; 4],
}
fn geometry_fits(point: [f64; 2], bounds: [f64; 4], width: u32, height: u32) -> bool {
    point.iter().chain(bounds.iter()).all(|v| v.is_finite())
        && point[0] >= 0.0
        && point[1] >= 0.0
        && point[0] < f64::from(width)
        && point[1] < f64::from(height)
        && bounds[0] >= 0.0
        && bounds[1] >= 0.0
        && bounds[2] > 0.0
        && bounds[3] > 0.0
        && bounds[0] + bounds[2] <= f64::from(width)
        && bounds[1] + bounds[3] <= f64::from(height)
}
/// Pixel documents author integer edges and half-pixel centers. A single
/// canonical edge delta preserves the original box extent and point offset.
pub(super) fn translated(
    marker: &vw_package::PackageMarker,
    translation: &VerificationMarkerTranslation,
    width: u32,
    height: u32,
) -> PackageResult<TranslatedGeometry> {
    if translation.original_number != marker.number
        || !translation.dx.is_finite()
        || !translation.dy.is_finite()
    {
        return Err(PackageError::Invalid);
    }
    // Quantization is ordinary pixel authoring, never a way to repair an
    // out-of-source caller translation. Both representations must fit.
    if !geometry_fits(
        [
            marker.point_document[0] + translation.dx,
            marker.point_document[1] + translation.dy,
        ],
        [
            marker.bbox_document[0] + translation.dx,
            marker.bbox_document[1] + translation.dy,
            marker.bbox_document[2],
            marker.bbox_document[3],
        ],
        width,
        height,
    ) {
        return Err(PackageError::Invalid);
    }
    let dx = vw_geom::snap_edge(translation.dx).map_err(|_| PackageError::Invalid)?;
    let dy = vw_geom::snap_edge(translation.dy).map_err(|_| PackageError::Invalid)?;
    let point = [marker.point_document[0] + dx, marker.point_document[1] + dy];
    let bounds = [
        marker.bbox_document[0] + dx,
        marker.bbox_document[1] + dy,
        marker.bbox_document[2],
        marker.bbox_document[3],
    ];
    if !geometry_fits(point, bounds, width, height) {
        return Err(PackageError::Invalid);
    }
    Ok(TranslatedGeometry { point, bounds })
}
pub(super) fn translations(
    failed: &[u32],
    translations: Vec<VerificationMarkerTranslation>,
) -> PackageResult<BTreeMap<u32, VerificationMarkerTranslation>> {
    if translations.len() != failed.len() || translations.len() > 64 {
        return Err(PackageError::Invalid);
    }
    let mut out = BTreeMap::new();
    for translation in translations {
        if !translation.dx.is_finite()
            || !translation.dy.is_finite()
            || failed.binary_search(&translation.original_number).is_err()
            || out
                .insert(translation.original_number, translation)
                .is_some()
        {
            return Err(PackageError::Invalid);
        }
    }
    Ok(out)
}
fn admit_failure_region(
    image: vw_package::VerificationImageGeometry,
    bounds: [f64; 4],
) -> PackageResult<()> {
    let region = vw_geom::Rect::new(bounds[0], bounds[1], bounds[2], bounds[3])
        .map_err(|_| PackageError::Invalid)?;
    image.document_region(region)?;
    Ok(())
}
/// A saved crop must contain the entire original failed marker and its
/// translated after marker. This runs before any isolated host/compiler work;
/// the normal compiler still validates every instruction and output image.
fn admit_failure_crops(checked: &CheckedPair, request: &CompileRequest) -> PackageResult<()> {
    let original = vw_package::verification_image_geometry(&checked.original)?;
    let after = vw_package::verification_image_geometry(&checked.after)?;
    for authored in &request.authoring.markers {
        let marker = checked
            .original
            .manifest()
            .markers
            .get(authored.original_number as usize - 1)
            .ok_or(PackageError::Invalid)?;
        let translation = request
            .translations
            .get(&authored.original_number)
            .ok_or(PackageError::Invalid)?;
        let moved = translated(
            marker,
            translation,
            checked.after.manifest().source.width,
            checked.after.manifest().source.height,
        )?;
        admit_failure_region(original, marker.bbox_document)?;
        admit_failure_region(after, moved.bounds)?;
    }
    Ok(())
}
fn role(value: &str) -> PackageResult<Role> {
    match value {
        "none" => Ok(Role::None),
        "change" => Ok(Role::Change),
        "preserve" => Ok(Role::Preserve),
        "reference" => Ok(Role::Reference),
        "explain" => Ok(Role::Explain),
        _ => Err(PackageError::Integrity),
    }
}
/// Stable private authoring IDs make retry compilation deterministic for the
/// same requested package. Existing DocumentView still checks every ID is fresh.
fn edit_id(package: &Id, number: u32, purpose: &str, time: i64) -> PackageResult<Id> {
    let mut hash = blake3::Hasher::new();
    hash.update(b"vw.verification.canonical-edit.v1\0");
    hash.update(package.as_str().as_bytes());
    hash.update(&number.to_be_bytes());
    hash.update(purpose.as_bytes());
    let mut entropy = [0; 10];
    entropy.copy_from_slice(&hash.finalize().as_bytes()[..10]);
    Ok(Id::from_parts(time as u64, entropy)?)
}
fn metadata(
    package: &Id,
    number: u32,
    purpose: &str,
    device: &vw_model::DeviceId,
    lamport: u64,
    time: i64,
) -> PackageResult<EditMetadata> {
    Ok(EditMetadata {
        transaction_id: edit_id(package, number, purpose, time)?,
        device: device.clone(),
        first_lamport: lamport,
        created_at_ms: time,
    })
}
fn admit_plan(
    host: &vw_ops::HostSequencer,
    plan: &vw_instructions::EditPlan,
    available: u64,
    canonical: u64,
    source_bytes: u64,
) -> PackageResult<u64> {
    let estimate = host
        .workspace_estimate_bytes(available / 16)
        .map_err(|_| PackageError::Invalid)?;
    let transaction = vw_ops::HostSequencer::transaction_workspace_estimate_bytes(
        plan.transaction(),
        available / 16,
    )
    .map_err(|_| PackageError::Invalid)?;
    let needed = estimate
        .checked_add(transaction)
        .and_then(|n| n.checked_mul(16))
        .and_then(|n| n.checked_add(4 * 1024 * 1024))
        .ok_or(PackageError::Limit)?
        .max(canonical);
    // The full encoded source is resident while host submission clones. Keep
    // the ordinary compiler's package/projection reserve available as this
    // isolated host grows; reduce its later budget before accepting more ops.
    if needed
        .checked_add(source_bytes)
        .and_then(|n| n.checked_add(2 * MAX_PACKAGE as u64 + 96 * 1024 * 1024))
        .is_none_or(|n| n >= available)
    {
        return Err(PackageError::Limit);
    }
    Ok(needed)
}
pub(super) struct CompileRequest {
    pub package_id: Id,
    pub created_at_ms: i64,
    pub memory_budget_bytes: u64,
    pub authoring: OriginalAuthoring,
    pub translations: BTreeMap<u32, VerificationMarkerTranslation>,
}
/// The existing after worker holds the exact full original while preparing an
/// isolated canonical host. No portable derivative is used as compiler source,
/// and no transaction touches either live project/store/history.
pub(super) fn compile(
    state: &ProjectState,
    checked: &CheckedPair,
    request: CompileRequest,
    stop: &Stop<'_>,
) -> PackageResult<vw_package::Package> {
    stop.check()?;
    source_current(state, &checked.after)?;
    admit_failure_crops(checked, &request)?;
    if state.replica.is_some() || !checked.after.manifest().markers.is_empty() {
        return Err(PackageError::Stale);
    }
    let retained = retained_bytes(checked)?;
    let available = request
        .memory_budget_bytes
        .checked_sub(retained)
        .ok_or(PackageError::Limit)?;
    let mut canonical = workflow::canonical_workspace(state, available, true)?;
    let budget = available
        .checked_sub(canonical)
        .ok_or(PackageError::Limit)?;
    let document = &checked.after.manifest().source.document_id;
    let originals = crate::packages::verification_originals(
        state,
        document,
        budget,
        stop.after_closed,
        stop.cancellation,
    )?;
    let source_bytes = originals.values().try_fold(0_u64, |total, bytes| {
        total
            .checked_add(bytes.capacity() as u64)
            .ok_or(PackageError::Limit)
    })?;
    let mut host =
        vw_ops::HostSequencer::from_checkpoint_bytes(&state.store()?.checkpoint_bytes()?)
            .map_err(|_| PackageError::Integrity)?;
    let device = state.store()?.local_device()?;
    if host.host_device() != &device {
        return Err(PackageError::Stale);
    }
    let revision = host.revision().map_err(|_| PackageError::Integrity)?;
    let view = DocumentView::new(host.project(), &revision, document).map_err(instruction_error)?;
    if view.instruction_ids().next().is_some() || view.marker_ids().next().is_some() {
        return Err(PackageError::Stale);
    }
    let layer = host
        .project()
        .layers
        .iter()
        .find(|(_, layer)| {
            layer.definition.document_id == Some(document.to_proto())
                && layer.definition.kind != vw_model::IMAGE_OUTPUT_LAYER_KIND
                && layer.visible
                && !layer.locked
                && layer.opacity > 0.0
        })
        .map(|(id, _)| id.clone())
        .ok_or(PackageError::Invalid)?;
    let mut lamport = state.history.next_lamport;
    if let (Some(text), Some((role, entry_method, language))) = (
        &checked.original.manifest().global_instruction,
        &request.authoring.global,
    ) {
        let revision = host.revision().map_err(|_| PackageError::Integrity)?;
        let plan = DocumentView::new(host.project(), &revision, document)
            .map_err(instruction_error)?
            .set_instruction(
                metadata(
                    &request.package_id,
                    0,
                    "global transaction",
                    &device,
                    lamport,
                    request.created_at_ms,
                )?,
                InstructionEdit {
                    instruction_id: edit_id(
                        &request.package_id,
                        0,
                        "global instruction",
                        request.created_at_ms,
                    )?,
                    targets: Vec::new(),
                    role: *role,
                    text: text.clone(),
                    entry_method: *entry_method,
                    language: language.clone(),
                },
            )
            .map_err(instruction_error)?;
        stop.check()?;
        canonical = admit_plan(&host, &plan, available, canonical, source_bytes)?;
        plan.submit(&mut host, &device, request.created_at_ms)
            .map_err(instruction_error)?;
        lamport = plan.next_lamport();
    }
    for authored in &request.authoring.markers {
        stop.check()?;
        let marker = checked
            .original
            .manifest()
            .markers
            .get(authored.original_number as usize - 1)
            .ok_or(PackageError::Invalid)?;
        let translation = request
            .translations
            .get(&authored.original_number)
            .ok_or(PackageError::Invalid)?;
        let geometry = translated(
            marker,
            translation,
            checked.after.manifest().source.width,
            checked.after.manifest().source.height,
        )?;
        let bounds = match marker.kind.as_str() {
            "point" => None,
            "box" => Some(v1::RectD {
                x: geometry.bounds[0],
                y: geometry.bounds[1],
                w: geometry.bounds[2],
                h: geometry.bounds[3],
            }),
            _ => return Err(PackageError::Unsupported),
        };
        let revision = host.revision().map_err(|_| PackageError::Integrity)?;
        let plan = DocumentView::new(host.project(), &revision, document)
            .map_err(instruction_error)?
            .place_marker(
                metadata(
                    &request.package_id,
                    marker.number,
                    "marker transaction",
                    &device,
                    lamport,
                    request.created_at_ms,
                )?,
                MarkerPlacement {
                    object_id: edit_id(
                        &request.package_id,
                        marker.number,
                        "object",
                        request.created_at_ms,
                    )?,
                    instruction_id: edit_id(
                        &request.package_id,
                        marker.number,
                        "instruction",
                        request.created_at_ms,
                    )?,
                    layer_id: layer.clone(),
                    point: v1::PointD {
                        x: geometry.point[0],
                        y: geometry.point[1],
                    },
                    bounds,
                    // Original EIDs belong to the old capture. Geometry has already
                    // been aligned by the checked caller; never relabel old EIDs as
                    // after-capture semantics or reattach imported lineage.
                    element_eids: Vec::new(),
                    style: authored.style,
                    role: role(&marker.role)?,
                    text: marker.instruction.clone(),
                    entry_method: authored.entry_method,
                    language: authored.language.clone(),
                },
            )
            .map_err(instruction_error)?;
        // Existing host submission clones/apply/validates atomically. Reserve
        // growth before that clone, including the newly prepared transaction.
        canonical = admit_plan(&host, &plan, available, canonical, source_bytes)?;
        plan.submit(&mut host, &device, request.created_at_ms)
            .map_err(instruction_error)?;
        lamport = plan.next_lamport();
    }
    stop.check()?;
    let revision = host.revision().map_err(|_| PackageError::Integrity)?;
    let original = checked.original.manifest();
    let after = checked.after.manifest();
    let budget = available
        .checked_sub(canonical)
        .ok_or(PackageError::Limit)?;
    let package = vw_package::compile(
        vw_package::CompileInput {
            project: host.project(),
            revision: &revision,
            document,
            originals: &originals,
        },
        vw_package::CompileOptions {
            package_id: request.package_id,
            created_at_ms: request.created_at_ms,
            target: original.extensions.target_profile.clone(),
            semantic_snapshot: after.extensions.semantic_snapshot.clone(),
            include_window_title: after
                .source
                .capture
                .as_ref()
                .is_some_and(|c| c.window_title.is_some()),
            // These choices were explicitly admitted for the already compiled
            // exact after source. Reuse them only at the same depth/profile.
            assume_untagged_srgb: true,
            allow_depth_reduction: after.extensions.source_bit_depth
                > after.extensions.derivative_bit_depth,
            limits: limits(budget),
        },
        stop,
    )?;
    // Canonical pixel operations must retain the accepted translation exactly.
    for (output, authored) in package
        .manifest()
        .markers
        .iter()
        .zip(&request.authoring.markers)
    {
        let marker = original
            .markers
            .get(authored.original_number as usize - 1)
            .ok_or(PackageError::Integrity)?;
        let geometry = translated(
            marker,
            request
                .translations
                .get(&authored.original_number)
                .ok_or(PackageError::Invalid)?,
            after.source.width,
            after.source.height,
        )?;
        if output.point_document != geometry.point || output.bbox_document != geometry.bounds {
            return Err(PackageError::Invalid);
        }
    }
    // Release isolated clone/original allocations before the final live-owner
    // source hash check, which has its own canonical workspace reservation.
    drop(host);
    drop(originals);
    stop.check()?;
    source_current(state, &checked.after)?;
    Ok(package)
}

#[cfg(test)]
mod frame_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn failure_region_requires_the_entire_saved_crop_even_with_padded_image_pixels() {
        let image = vw_package::VerificationImageGeometry {
            source_size: [100, 80],
            output_size: [30, 40],
            image_size: [64, 64],
            content_size: [30, 40],
            crop_document: vw_geom::Rect::new(10.0, 20.0, 30.0, 40.0).unwrap(),
            document_to_image: vw_geom::Affine::IDENTITY,
            image_to_document: vw_geom::Affine::IDENTITY,
            has_output_frame: true,
        };
        assert!(admit_failure_region(image, [12.0, 25.0, 10.0, 12.0]).is_ok());
        assert!(admit_failure_region(image, [9.0, 25.0, 10.0, 12.0]).is_err());
        assert!(admit_failure_region(image, [35.0, 25.0, 10.0, 12.0]).is_err());
        assert!(admit_failure_region(image, [12.0, 25.0, 0.0, 12.0]).is_err());
    }
}
