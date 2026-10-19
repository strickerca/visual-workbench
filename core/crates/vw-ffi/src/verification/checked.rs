use super::*;
use crate::{McpInboxReceipt, McpInboxSide, WorkflowBinding, project::ProjectState, workflow};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};

// Decode only the checked manifest's extension presence. The positive O
// manifest field is composed by the package owner; this leaf also builds on
// the preceding package base, where that field is not in the Rust type yet.
#[derive(Deserialize)]
struct FrameProbe {
    extensions: FrameExtensions,
}
#[derive(Deserialize)]
struct FrameExtensions {
    image_output: Option<serde::de::IgnoredAny>,
}
fn manifest_output_frame(manifest: &[u8]) -> PackageResult<bool> {
    let probe: FrameProbe =
        serde_json::from_slice(manifest).map_err(|_| PackageError::Integrity)?;
    Ok(probe.extensions.image_output.is_some())
}
fn output_frame(package: &vw_package::Package) -> PackageResult<bool> {
    manifest_output_frame(
        package
            .file("manifest.json")
            .ok_or(PackageError::Integrity)?,
    )
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    #[test]
    fn checked_manifest_frame_presence() {
        assert!(!manifest_output_frame(br#"{"extensions":{}}"#).unwrap());
        assert!(!manifest_output_frame(br#"{"extensions":{"image_output":null}}"#).unwrap());
        assert!(manifest_output_frame(br#"{"extensions":{"image_output":{"clockwise_degrees":90,"output_width":256,"output_height":256}}}"#).unwrap());
        assert!(manifest_output_frame(br#"{"extensions":{"image_output":false}}"#).unwrap());
        assert!(manifest_output_frame(br#"{}"#).is_err());
    }
}

/// Owned checked catalog bytes. Imported lineage is deliberately never read as
/// evidence of capture identity, owner permission or alignment.
pub(super) struct CheckedPair {
    pub original: vw_package::Package,
    pub after: vw_package::Package,
    pub store_pair: vw_store::VerificationReviewPair,
}
pub(super) struct Stop<'a> {
    pub original_closed: &'a AtomicBool,
    pub after_closed: &'a AtomicBool,
    pub cancellation: &'a Cancellation,
}
impl vw_package::Cancellation for Stop<'_> {
    fn is_cancelled(&self) -> bool {
        self.original_closed.load(Ordering::Acquire)
            || self.after_closed.load(Ordering::Acquire)
            || self.cancellation.is_cancelled()
    }
}
impl Stop<'_> {
    pub fn check(&self) -> PackageResult<()> {
        if self.original_closed.load(Ordering::Acquire) || self.after_closed.load(Ordering::Acquire)
        {
            return Err(PackageError::Closed);
        }
        self.cancellation.check()?;
        Ok(())
    }
}
pub(super) fn binding(package: &vw_package::Package) -> WorkflowBinding {
    let m = package.manifest();
    WorkflowBinding {
        project_id: m.source.project_id.to_string(),
        document_id: m.source.document_id.to_string(),
        host_seq: m.extensions.host_seq,
        state_hash: m.extensions.state_hash.clone(),
    }
}
fn clean(package: &vw_package::Package) -> PackageResult<&[u8]> {
    let mut images = package
        .manifest()
        .images
        .iter()
        .filter(|i| i.role == "clean_source");
    let image = images.next().ok_or(PackageError::Integrity)?;
    if images.next().is_some() {
        return Err(PackageError::Integrity);
    }
    package.file(&image.path).ok_or(PackageError::Integrity)
}
fn check_receipt(
    pair: &VerificationPair,
    receipt: &McpInboxReceipt,
    checked: &CheckedPair,
) -> PackageResult<()> {
    let after_hash = blake3::hash(clean(&checked.after)?).to_hex().to_string();
    if receipt.retired
        || receipt.receipt_id != pair.receipt_id
        || receipt.receipt_blake3 != pair.receipt_blake3
        || receipt.package_id != pair.original.package_id
        || receipt.target != pair.original.target
        || receipt.manifest_sha256 != pair.original.manifest_sha256
        || receipt.binding != binding(&checked.original)
        || receipt.source_asset_id != checked.original.manifest().source.asset_hash
        || receipt.after.is_none()
        || receipt.text.is_some()
        || receipt.before.blake3 != blake3::hash(clean(&checked.original)?).to_hex().as_str()
        || receipt
            .after
            .as_ref()
            .is_none_or(|image| image.blake3 != after_hash)
    {
        return Err(PackageError::Stale);
    }
    Ok(())
}
pub(super) async fn receipt_current(
    inbox: &McpResultInbox,
    pair: &VerificationPair,
    checked: &CheckedPair,
    cancellation: Arc<Cancellation>,
) -> PackageResult<()> {
    let receipts = inbox.list(cancellation.clone()).await?;
    let mut exact = receipts
        .iter()
        .filter(|receipt| receipt.receipt_id == pair.receipt_id);
    let receipt = exact.next().ok_or(PackageError::Stale)?;
    if exact.next().is_some() {
        return Err(PackageError::Integrity);
    }
    check_receipt(pair, receipt, checked)?;
    // The record digest alone is insufficient: read_png reopens and validates
    // the retained PNG owner and its encoded hash, including retirement refusal.
    for (side, package) in [
        (McpInboxSide::Before, &checked.original),
        (McpInboxSide::After, &checked.after),
    ] {
        let png = inbox
            .read_png(
                pair.receipt_id.clone(),
                pair.receipt_blake3.clone(),
                side,
                cancellation.clone(),
            )
            .await?;
        if png.as_slice() != clean(package)? {
            return Err(PackageError::Stale);
        }
    }
    Ok(())
}
pub(super) async fn catalog_current(
    catalog: &PackageCatalog,
    pair: &VerificationPair,
    cancellation: Arc<Cancellation>,
) -> PackageResult<()> {
    for key in [&pair.original, &pair.after] {
        catalog
            .lookup(
                key.package_id.clone(),
                key.target.clone(),
                key.manifest_sha256.clone(),
                cancellation.clone(),
            )
            .await?;
    }
    Ok(())
}
pub(super) async fn load(
    catalog: &PackageCatalog,
    inbox: &McpResultInbox,
    pair: &VerificationPair,
    cancellation: Arc<Cancellation>,
) -> PackageResult<Arc<CheckedPair>> {
    pair.validate()?;
    let original = catalog
        .verification_package(
            pair.original.package_id.clone(),
            pair.original.target.clone(),
            pair.original.manifest_sha256.clone(),
            cancellation.clone(),
        )
        .await?;
    let after = catalog
        .verification_package(
            pair.after.package_id.clone(),
            pair.after.target.clone(),
            pair.after.manifest_sha256.clone(),
            cancellation.clone(),
        )
        .await?;
    if !pair.original.matches(&original)
        || !pair.after.matches(&after)
        || original.manifest().extensions.target_profile
            != after.manifest().extensions.target_profile
    {
        return Err(PackageError::Stale);
    }
    if output_frame(&original)? || output_frame(&after)? {
        return Err(PackageError::Unsupported);
    }
    let store_pair = vw_store::VerificationReviewPair {
        project_id: original.manifest().source.project_id.clone(),
        original: pair.original.native()?,
        after: pair.after.native()?,
        comparison_receipt_id: vw_model::Id::try_from(pair.receipt_id.clone())?,
        comparison_receipt_blake3: pair.receipt_blake3.clone(),
        marker_count: u32::try_from(original.manifest().markers.len())
            .map_err(|_| PackageError::Limit)?,
    };
    store_pair.validate().map_err(store_error)?;
    let checked = Arc::new(CheckedPair {
        original,
        after,
        store_pair,
    });
    receipt_current(inbox, pair, &checked, cancellation).await?;
    Ok(checked)
}
/// Runs only on the corresponding existing project worker. Canonical workspace
/// admission precedes state hashing and instruction projection.
pub(super) fn source_current(
    state: &ProjectState,
    package: &vw_package::Package,
) -> PackageResult<()> {
    if output_frame(package)? {
        return Err(PackageError::Unsupported);
    }
    // Both checked packages and a possible prepared follow-up remain owned
    // across the final fences; reserve their complete maxima plus metadata.
    workflow::canonical_workspace(
        state,
        MAX_MEMORY - 3 * MAX_PACKAGE as u64 - 8 * 1024 * 1024,
        false,
    )?;
    workflow::check_binding(state, &binding(package))?;
    let m = package.manifest();
    let project = state.project()?;
    let document = project
        .documents
        .get(&m.source.document_id)
        .ok_or(PackageError::Stale)?;
    let definition = &document.definition;
    let asset = project
        .assets
        .get(&vw_model::AssetId::try_from(
            definition.primary_asset_id.clone(),
        )?)
        .ok_or(PackageError::Original)?;
    let dimensions = if asset.orientation >= 5 {
        (asset.height, asset.width)
    } else {
        (asset.width, asset.height)
    };
    let kind = match vw_proto::v1::DocumentKind::try_from(definition.kind) {
        Ok(vw_proto::v1::DocumentKind::Capture) => "capture",
        Ok(vw_proto::v1::DocumentKind::Image) => "image",
        _ => return Err(PackageError::Unsupported),
    };
    if definition.primary_asset_id != m.source.asset_hash
        || dimensions != (m.source.width, m.source.height)
        || kind != m.source.document_kind
        || m.source.units != "px"
        || asset.bit_depth.max(8) != u32::from(m.extensions.source_bit_depth)
        || m.extensions.derivative_bit_depth != 8
    {
        return Err(PackageError::Stale);
    }
    match (&definition.capture, &m.source.capture) {
        (None, None) => {}
        (Some(actual), Some(expected))
            if actual.capture_session_id == Some(expected.session_id.to_proto())
                && actual.frame_id == expected.frame_id
                && actual.platform == expected.platform
                && actual.app_name == expected.app_name
                && actual.lossless == expected.lossless
                && actual.degraded == expected.degraded
                && expected
                    .window_title
                    .as_ref()
                    .is_none_or(|title| title == &actual.window_title) => {}
        _ => return Err(PackageError::Stale),
    }
    let revision = state.view_revision()?;
    let view = vw_instructions::DocumentView::new(project, &revision, &m.source.document_id)
        .map_err(instruction_error)?;
    let instructions =
        vw_instructions::InstructionExport::from_view(&view).map_err(instruction_error)?;
    if instructions.markers().len() != m.markers.len()
        || instructions.global().map(|g| &g.text) != m.global_instruction.as_ref()
    {
        return Err(PackageError::Stale);
    }
    for (canonical, marker) in instructions.markers().iter().zip(&m.markers) {
        let instruction = instructions
            .instructions()
            .iter()
            .find(|i| i.id == canonical.instruction_id)
            .ok_or(PackageError::Integrity)?;
        if canonical.number != marker.number
            || canonical.point_document != marker.point_document
            || canonical.role.as_str() != marker.role
            || instruction.text != marker.instruction
            || canonical.bounds_document.is_some() != (marker.kind == "box")
            || canonical
                .bounds_document
                .is_some_and(|bounds| bounds != marker.bbox_document)
        {
            return Err(PackageError::Stale);
        }
    }
    Ok(())
}
