//! Explicit input-free installed-Paint observation. Never an ordinary-pin fallback.
use super::*;
use vw_host::editor_paint_package::{PaintPackageError, PaintPackageEvidence, PaintPackageLease};

#[derive(Serialize)]
struct PackageReceipt<'a, T: Serialize> {
    schema: u32,
    target: Target,
    image: ImageIdentity,
    parent_job_present: bool,
    source_namespace_policy: &'static str,
    held_process_and_final_image: bool,
    held_ancestor_namespace: bool,
    namespace_lease_equivalence_claim: bool,
    os_package_namespace_validated: bool,
    package_evidence: &'a PaintPackageEvidence,
    observed_start_qpc_100ns: u64,
    observed_end_qpc_100ns: u64,
    foreground_same_selected_window_before: bool,
    foreground_same_selected_window_after: bool,
    observation: T,
    input_sent: bool,
    profile_authority: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlsRequest {
    schema: u32,
    owner_pid: u32,
    target: Target,
    image: ImageIdentity,
}
#[derive(Serialize)]
struct ControlsReceipt {
    source_proof: crate::editor_source::SourceProof,
    controls: platform::paint_controls::Snapshot,
}
fn error(error: PaintPackageError) -> Error {
    match error {
        PaintPackageError::Policy => Error::Unavailable,
        PaintPackageError::TargetChanged => Error::TargetChanged,
        PaintPackageError::Cancelled => Error::Cancelled,
        PaintPackageError::Timeout => Error::Timeout,
        PaintPackageError::Limit => Error::Limit,
        PaintPackageError::Platform { stage, code } => Error::Platform {
            phase: format!("paint-package/{stage}"),
            code: code as u32,
        },
    }
}
fn capture_target(target: &Target, owner: u32) -> Result<vw_host::CaptureTarget> {
    platform::unchanged(target, owner)?;
    Ok(vw_capture::windows::fixed_target(target.window, owner)
        .map_err(|_| Error::TargetChanged)?
        .into())
}
fn held_image(lease: &PaintPackageLease) -> ImageIdentity {
    let v = lease.identity();
    ImageIdentity {
        executable_name: v.executable_name.clone(),
        executable_blake3: v.executable_blake3.clone(),
        executable_bytes: v.executable_bytes,
        file_version: v
            .file_version
            .map(|v| format!("{}.{}.{}.{}", v.major, v.minor, v.build, v.revision)),
        package_full_name: v.package_full_name.clone(),
        package_version: v.package_version.clone(),
    }
}
fn verify(lease: &PaintPackageLease, target: &Target, owner: u32) -> Result<()> {
    lease
        .verify(capture_target(target, owner)?, owner)
        .map_err(error)?;
    platform::unchanged(target, owner)
}
pub(super) fn run(mode: &str, request: &str) -> Result<()> {
    // Parent run() already proved Job containment before foreign IO and owns
    // DPI/one-process confinement. No alternate direct executable entry exists.
    match mode {
        "--observe-paint-controls" => {
            use crate::editor_source::{SourceBudget, SourceLease, SourcePolicy};
            let request: ControlsRequest =
                serde_json::from_str(request).map_err(|_| Error::Invalid)?;
            request.target.validate()?;
            if request.schema != 1 || request.owner_pid == 0 {
                return Err(Error::Invalid);
            }
            // One cancellation owner and absolute budget includes open, every UIA
            // call and both source verifications. Slow calls never renew it.
            let budget = SourceBudget::new(
                Arc::new(vw_capture::Cancellation::default()),
                std::time::Duration::from_secs(5),
            )?;
            let source = SourceLease::open(
                &request.target,
                request.owner_pid,
                &request.image,
                SourcePolicy::InstalledPaint,
                &budget,
            )?;
            let _apartment = budget.call(platform::EffectApartment::initialize)?;
            source.verify(&request.target, request.owner_pid, &budget)?;
            let start = budget.call(platform::clock_100ns)?;
            let before = budget.call(|| Ok(foreground(&request.target)))?;
            let controls = platform::paint_controls::observe(&request.target, &budget)?;
            source.verify(&request.target, request.owner_pid, &budget)?;
            let end = budget.call(platform::clock_100ns)?;
            let after = budget.call(|| Ok(foreground(&request.target)))?;
            // The borrowed evidence belongs to this actual retained lease; caller
            // text, package metadata and this observation never grant actions.
            let package_evidence = source
                .installed_package_evidence()
                .ok_or(Error::Unavailable)?;
            emit(&PackageReceipt {
                schema: 1,
                target: request.target,
                image: request.image,
                parent_job_present: true,
                source_namespace_policy: "installed-paint-v1",
                held_process_and_final_image: true,
                held_ancestor_namespace: false,
                namespace_lease_equivalence_claim: false,
                os_package_namespace_validated: true,
                package_evidence,
                observed_start_qpc_100ns: start,
                observed_end_qpc_100ns: end,
                foreground_same_selected_window_before: before,
                foreground_same_selected_window_after: after,
                observation: ControlsReceipt {
                    source_proof: source.proof(),
                    controls,
                },
                input_sent: false,
                profile_authority: false,
            })
        }
        "--bind-paint" => {
            let selected: Selection = serde_json::from_str(request).map_err(|_| Error::Invalid)?;
            if selected.schema != 1 || selected.owner_pid == 0 {
                return Err(Error::Invalid);
            }
            let target =
                platform::query_target(selected.window, selected.owner_pid, selected.token)?;
            if target.process_id != selected.process_id {
                return Err(Error::TargetChanged);
            }
            let start = platform::clock_100ns()?;
            let before = foreground(&target);
            let lease = PaintPackageLease::open(
                capture_target(&target, selected.owner_pid)?,
                selected.owner_pid,
                Arc::new(vw_capture::Cancellation::default()),
            )
            .map_err(error)?;
            verify(&lease, &target, selected.owner_pid)?;
            let image = held_image(&lease);
            let end = platform::clock_100ns()?;
            let after = foreground(&target);
            emit(&PackageReceipt {
                schema: 1,
                target,
                image,
                parent_job_present: true,
                source_namespace_policy: "installed-paint-v1",
                held_process_and_final_image: true,
                held_ancestor_namespace: false,
                namespace_lease_equivalence_claim: false,
                os_package_namespace_validated: true,
                package_evidence: lease.evidence(),
                observed_start_qpc_100ns: start,
                observed_end_qpc_100ns: end,
                foreground_same_selected_window_before: before,
                foreground_same_selected_window_after: after,
                observation: "native installed-package binding only; no UIA",
                input_sent: false,
                profile_authority: false,
            })
        }
        "--observe-paint" => {
            let r: Request = serde_json::from_str(request).map_err(|_| Error::Invalid)?;
            r.target.validate()?;
            if r.schema != 1 || r.owner_pid == 0 || !(1..=uia::MAX_NODES).contains(&r.max_nodes) {
                return Err(Error::Invalid);
            }
            let start = platform::clock_100ns()?;
            let before = foreground(&r.target);
            let lease = PaintPackageLease::open(
                capture_target(&r.target, r.owner_pid)?,
                r.owner_pid,
                Arc::new(vw_capture::Cancellation::default()),
            )
            .map_err(error)?;
            if held_image(&lease) != r.image {
                return Err(Error::TargetChanged);
            }
            verify(&lease, &r.target, r.owner_pid)?;
            let observation = uia::observe(&r.target, r.max_nodes)?;
            verify(&lease, &r.target, r.owner_pid)?;
            if held_image(&lease) != r.image {
                return Err(Error::TargetChanged);
            }
            let end = platform::clock_100ns()?;
            let after = foreground(&r.target);
            emit(&PackageReceipt {
                schema: 1,
                target: r.target,
                image: r.image,
                parent_job_present: true,
                source_namespace_policy: "installed-paint-v1",
                held_process_and_final_image: true,
                held_ancestor_namespace: false,
                namespace_lease_equivalence_claim: false,
                os_package_namespace_validated: true,
                package_evidence: lease.evidence(),
                observed_start_qpc_100ns: start,
                observed_end_qpc_100ns: end,
                foreground_same_selected_window_before: before,
                foreground_same_selected_window_after: after,
                observation,
                input_sent: false,
                profile_authority: false,
            })
        }
        _ => Err(Error::Invalid),
    }
}
