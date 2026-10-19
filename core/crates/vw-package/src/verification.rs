//! Failure follow-ups reuse normal compiler output, PNG receipts and verifier.
//! No manual notes, private selector, capture target or new pixel authority.
//!
//! Root activation hunks (not applied by this four-file lane):
//! - lib.rs: `mod verification; pub use verification::*;`
//! - Extensions: `#[serde(default, skip_serializing_if = "Option::is_none")]`
//!   `pub verification_origin: Option<crate::VerificationOrigin>,`
//!   Use this leaf's record; do not also define the earlier proposal's duplicate
//!   VerificationOrigin in manifest.rs.
//! - compiler's ordinary Extensions literal: `verification_origin: None,`
//! - admission::array_limit: `b"original_marker_numbers" => self.markers,`
//! - verify::from_compiler: after manifest marker-count admission, call
//!   `crate::validate_verification_origin(&m)?;`
//! - impl Package in verify.rs (moves owners, never clones encoded images):
//! ```ignore
//! pub(crate) fn into_verification_parts(self) -> (Manifest, BTreeMap<String, Vec<u8>>) {
//!     (self.manifest, self.files)
//! }
//! pub(crate) fn verification_allocation_bytes(&self) -> Result<u64> {
//!     self.files.values().try_fold(0_u64, |n, v|
//!         n.checked_add(v.capacity() as u64).ok_or(Error::Limit("package allocation")))
//! }
//! ```
//! Map the store's checked snapshot to FailureReview without its notes. Root
//! retains the original/after catalog and receipt retirement checks and current
//! store CAS version under its existing serial owner, including before final
//! catalog publication. This pure compiler's return is not permission to publish.
//!
//! Handoff: six deterministic integration regressions are written; syntax and
//! formatting pass. Compiler/native execution is unrun while the image lane
//! holds the slot. Compose the exact hooks above, then run
//! `cargo test -p vw-package --test verification` and strict affected Clippy.
//! No coordinator, FFI, UI, alignment-quality or hardware acceptance is claimed.
use crate::{
    Cancellation, Error, FileEntry, Limits, Manifest, Package, Result, UNTRUSTED_TEXT_NOTICE,
    bounded, check,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::Write;
use vw_model::Id;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationOrigin {
    pub original_package_id: Id,
    pub original_target: String,
    pub original_manifest_sha256: String,
    pub comparison_receipt_id: Id,
    pub comparison_receipt_blake3: String,
    /// Entry n maps follow-up marker n+1 to its original number.
    pub original_marker_numbers: Vec<u32>,
}
pub fn validate_verification_origin(manifest: &Manifest) -> Result<()> {
    if let Some(origin) = &manifest.extensions.verification_origin {
        if origin.original_marker_numbers.is_empty()
            || origin.original_marker_numbers.len() > 512
            || origin.original_marker_numbers.len() != manifest.markers.len()
        {
            return Err(Error::Integrity);
        }
        let numbers: BTreeSet<_> = origin.original_marker_numbers.iter().copied().collect();
        if origin.original_package_id == manifest.package_id
            || origin.original_target != manifest.compiled_for.target
            || !matches!(
                origin.original_target.as_str(),
                "claude" | "openai" | "gemini" | "generic"
            )
            || !bounded::hash_valid(&origin.original_manifest_sha256)
            || !bounded::hash_valid(&origin.comparison_receipt_blake3)
            || numbers.len() != origin.original_marker_numbers.len()
            || numbers.iter().any(|n| !(1..=512).contains(n))
        {
            return Err(Error::Integrity);
        }
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationPackageIdentity {
    pub package_id: Id,
    pub target: String,
    pub manifest_sha256: String,
}
impl VerificationPackageIdentity {
    pub fn from_package(value: &Package) -> Self {
        Self {
            package_id: value.manifest().package_id.clone(),
            target: value.manifest().compiled_for.target.clone(),
            manifest_sha256: value.manifest_sha256().to_owned(),
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct FailureReviewFence {
    pub original: VerificationPackageIdentity,
    pub after: VerificationPackageIdentity,
    pub comparison_receipt_id: Id,
    pub comparison_receipt_blake3: String,
    pub version: u64,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FailureVerdict {
    Pending,
    Pass,
    Fail,
}
#[derive(Clone, PartialEq, Eq)]
pub struct FailureMarkerAssessment {
    pub original_number: u32,
    pub verdict: FailureVerdict,
}
#[derive(Clone, PartialEq, Eq)]
pub struct FailureReview {
    pub fence: FailureReviewFence,
    pub markers: Vec<FailureMarkerAssessment>,
}

/// Cheap preflight lets all-pass callers avoid even preparing compiler input.
/// Inputs are checked Packages, and the review is the current private store
/// snapshot projected without notes. No agent assertion can establish this pair.
pub fn select_verification_failures(
    original: &Package,
    after: &Package,
    review: &FailureReview,
) -> Result<Option<Vec<u32>>> {
    if review.fence.original != VerificationPackageIdentity::from_package(original)
        || review.fence.after != VerificationPackageIdentity::from_package(after)
        || review.fence.original.package_id == review.fence.after.package_id
        || !bounded::hash_valid(&review.fence.comparison_receipt_blake3)
        || review.fence.version > i64::MAX as u64
    {
        return Err(Error::Stale);
    }
    let markers = &original.manifest().markers;
    if markers.len() > 512
        || review.markers.len() != markers.len()
        || (!markers.is_empty() && review.fence.version == 0)
    {
        return Err(Error::Invalid("review markers"));
    }
    let mut failed = Vec::new();
    for (marker, assessment) in markers.iter().zip(&review.markers) {
        if marker.number != assessment.original_number {
            return Err(Error::Invalid("review marker identity"));
        }
        match assessment.verdict {
            FailureVerdict::Pending => return Err(Error::Invalid("pending owner review")),
            FailureVerdict::Pass => {}
            FailureVerdict::Fail => failed.push(marker.number),
        }
    }
    Ok((!failed.is_empty()).then_some(failed))
}

fn current(
    review: &FailureReview,
    still_current: &mut impl FnMut(&FailureReview) -> bool,
    cancel: &dyn Cancellation,
) -> Result<()> {
    check(cancel)?;
    if !still_current(review) {
        return Err(Error::Stale);
    }
    check(cancel)
}

/// Attach public lineage to normal compiler output over the exact recapture's
/// source. Caller prepares only the failed markers through existing canonical
/// instruction operations and normal compile, retaining original text/roles.
/// Translated geometry/element references are caller-supplied existing alignment
/// results, never inferred or authorized by lineage. No alternative renderer.
/// `still_current` compares the actual supplied pair/version AND number/verdict
/// projection with the current store snapshot, and checks catalog/receipt retirement,
/// without reacquiring the caller's already-held edit mutex.
pub fn compile_verification_failure_package(
    original: &Package,
    after: &Package,
    review: &FailureReview,
    prepared: Option<Package>,
    limits: Limits,
    mut still_current: impl FnMut(&FailureReview) -> bool,
    cancel: &dyn Cancellation,
) -> Result<Option<Package>> {
    limits.validate()?;
    current(review, &mut still_current, cancel)?;
    let Some(failed) = select_verification_failures(original, after, review)? else {
        return Ok(None);
    };
    let prepared = prepared.ok_or(Error::Invalid("missing prepared failure package"))?;
    let m = prepared.manifest();
    let a = after.manifest();
    let o = original.manifest();
    if m.package_id == o.package_id
        || m.package_id == a.package_id
        || m.extensions.verification_origin.is_some()
        || m.extensions.target_profile != o.extensions.target_profile
        || m.markers.len() != failed.len()
        || m.source.project_id != a.source.project_id
        || m.source.document_id != a.source.document_id
        || m.source.asset_hash != a.source.asset_hash
        || m.source.width != a.source.width
        || m.source.height != a.source.height
        || m.extensions.image_output != a.extensions.image_output
        || m.source.document_kind != a.source.document_kind
        || m.source.units != a.source.units
        || m.extensions.host_seq < a.extensions.host_seq
        || m.global_instruction != o.global_instruction
        || serde_json::to_vec(&m.source.capture).map_err(|_| Error::Encoding)?
            != serde_json::to_vec(&a.source.capture).map_err(|_| Error::Encoding)?
    {
        return Err(Error::Stale);
    }
    for (marker, number) in m.markers.iter().zip(&failed) {
        let old = o
            .markers
            .get(*number as usize - 1)
            .ok_or(Error::Integrity)?;
        if marker.instruction != old.instruction
            || marker.role != old.role
            || marker.kind != old.kind
        {
            return Err(Error::Invalid("changed failed instruction"));
        }
    }
    // Account actual retained encoded owners and conservative admitted manifest
    // projections before moving output owners or allocating the new prompt.
    let retained = original
        .verification_allocation_bytes()?
        .checked_add(after.verification_allocation_bytes()?)
        .and_then(|n| n.checked_add(64 * 1024 * 1024))
        .ok_or(Error::Limit("memory"))?;
    limits.working(retained)?;
    if failed.len() > limits.markers
        || prepared.total_bytes() > limits.package_bytes
        || prepared.verification_allocation_bytes()? > limits.package_bytes as u64 * 2
    {
        return Err(Error::Limit("package size"));
    }
    let (mut manifest, mut files) = prepared.into_verification_parts();
    manifest.extensions.verification_origin = Some(VerificationOrigin {
        original_package_id: o.package_id.clone(),
        original_target: o.compiled_for.target.clone(),
        original_manifest_sha256: original.manifest_sha256().to_owned(),
        comparison_receipt_id: review.fence.comparison_receipt_id.clone(),
        comparison_receipt_blake3: review.fence.comparison_receipt_blake3.clone(),
        original_marker_numbers: failed.clone(),
    });
    validate_verification_origin(&manifest)?;
    let old_prompt = files.remove("prompt.md").ok_or(Error::Integrity)?;
    let notice = format!("{UNTRUSTED_TEXT_NOTICE}\n");
    if !old_prompt.ends_with(notice.as_bytes()) {
        return Err(Error::Integrity);
    }
    let mut prompt = bounded::Buffer::new(bounded::MAX_TEXT);
    prompt
        .write_all(&old_prompt[..old_prompt.len() - notice.len()])
        .map_err(|_| Error::Limit("prompt"))?;
    writeln!(&mut prompt, "\n## Verification follow-up\nOriginal package {} target {}; manifest SHA-256 {}.\nComparison receipt {}; BLAKE3 {}.",
        o.package_id, o.compiled_for.target, original.manifest_sha256(), review.fence.comparison_receipt_id,
        review.fence.comparison_receipt_blake3).map_err(|_| Error::Limit("prompt"))?;
    for (index, number) in failed.iter().enumerate() {
        check(cancel)?;
        writeln!(
            &mut prompt,
            "Follow-up marker {} refers to original marker {}.",
            index + 1,
            number
        )
        .map_err(|_| Error::Limit("prompt"))?;
    }
    prompt
        .write_all(notice.as_bytes())
        .map_err(|_| Error::Limit("prompt"))?;
    drop(old_prompt);
    files.insert("prompt.md".to_owned(), prompt.bytes);
    files.remove("manifest.json");
    manifest.files = files
        .iter()
        .map(|(path, bytes)| FileEntry {
            path: path.clone(),
            sha256: bounded::hash(bytes),
        })
        .collect();
    files.insert(
        "manifest.json".to_owned(),
        bounded::json(&manifest, bounded::MAX_TEXT)?,
    );
    current(review, &mut still_current, cancel)?;
    // Invoke the normal public verifier, reducing its available memory by the
    // still-retained checked input owners. Encoded file owners move, not clone.
    let mut final_limits = limits;
    final_limits.memory_bytes = limits
        .memory_bytes
        .checked_sub(retained)
        .ok_or(Error::Limit("memory"))?;
    let result = Package::from_files(files.into_iter().collect(), final_limits, cancel)?;
    current(review, &mut still_current, cancel)?;
    Ok(Some(result))
}
