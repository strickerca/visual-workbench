use crate::{PackageError, PackageResult};
use vw_model::Id;

#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct VerificationPackageKey {
    pub package_id: String,
    pub target: String,
    pub manifest_sha256: String,
}
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct VerificationPair {
    pub original: VerificationPackageKey,
    pub after: VerificationPackageKey,
    pub receipt_id: String,
    pub receipt_blake3: String,
}
/// Checked package image geometry in physical retained-PNG pixels. The affine
/// coefficients are in a,b,c,d,e,f order, with x'=a*x+c*y+e and y'=b*x+d*y+f.
/// Content dimensions exclude minimum 64px PNG padding but can round up a
/// fractional edge. This does not prove that the images are registered or
/// that either capture source is current.
#[derive(Clone, PartialEq, uniffi::Record)]
pub struct VerificationImageCoordinates {
    pub key: VerificationPackageKey,
    pub binding: crate::WorkflowBinding,
    pub source_asset_id: String,
    pub source_width: u32,
    pub source_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub image_width: u32,
    pub image_height: u32,
    pub content_width: u32,
    pub content_height: u32,
    /// Continuous mapped crop edges [left, top, width, height]. The rounded-up
    /// content dimensions can contain fractional padding at the far edges.
    pub content_bounds: Vec<f64>,
    pub crop_document: Vec<f64>,
    pub document_to_image: Vec<f64>,
    pub image_to_document: Vec<f64>,
    pub has_output_frame: bool,
}
/// Geometry and exact package identities only. Receipt, source, alignment and
/// owner permissions remain separately checked by the verification workflow.
#[derive(Clone, PartialEq, uniffi::Record)]
pub struct VerificationCoordinatePair {
    pub pair: VerificationPair,
    pub original: VerificationImageCoordinates,
    pub after: VerificationImageCoordinates,
}
#[derive(Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum VerificationVerdict {
    Pending,
    Pass,
    Fail,
}
// Deliberately no Debug: manual notes are private installation-local content.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct VerificationMarkerReview {
    pub original_number: u32,
    pub verdict: VerificationVerdict,
    pub note: String,
}
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct VerificationReviewSnapshot {
    pub pair: VerificationPair,
    pub project_id: String,
    pub marker_count: u32,
    pub version: u64,
    pub markers: Vec<VerificationMarkerReview>,
}
/// Private saved review data only. It grants no source mutation or capture.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct VerificationReviewHistoryPage {
    pub snapshots: Vec<VerificationReviewSnapshot>,
    pub next_cursor: Option<String>,
}
/// Geometry supplied by the private owner workflow's checked alignment. This is
/// neither an agent verdict nor proof of capture permission or image alignment.
#[derive(Clone, Copy, PartialEq, uniffi::Record)]
pub struct VerificationMarkerTranslation {
    pub original_number: u32,
    pub dx: f64,
    pub dy: f64,
}
pub(super) fn hash(value: &str) -> PackageResult<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(PackageError::Invalid);
    }
    Ok(())
}
impl VerificationPackageKey {
    pub(super) fn native(&self) -> PackageResult<vw_store::VerificationPackageKey> {
        if self.package_id.len() != 36 {
            return Err(PackageError::Invalid);
        }
        hash(&self.manifest_sha256)?;
        if !matches!(
            self.target.as_str(),
            "claude" | "openai" | "gemini" | "generic"
        ) {
            return Err(PackageError::Invalid);
        }
        Ok(vw_store::VerificationPackageKey {
            package_id: Id::try_from(self.package_id.clone())?,
            target: self.target.clone(),
            manifest_sha256: self.manifest_sha256.clone(),
        })
    }
    pub(super) fn matches(&self, package: &vw_package::Package) -> bool {
        self.package_id == package.manifest().package_id.as_str()
            && self.target == package.manifest().compiled_for.target
            && self.manifest_sha256 == package.manifest_sha256()
    }
}
impl VerificationPair {
    pub(super) fn validate(&self) -> PackageResult<()> {
        self.original.native()?;
        self.after.native()?;
        if self.receipt_id.len() != 36 {
            return Err(PackageError::Invalid);
        }
        Id::try_from(self.receipt_id.clone())?;
        hash(&self.receipt_blake3)?;
        if self.original.package_id == self.after.package_id {
            return Err(PackageError::Invalid);
        }
        Ok(())
    }
}
impl VerificationVerdict {
    pub(super) fn native(self) -> vw_store::VerificationVerdict {
        match self {
            Self::Pending => vw_store::VerificationVerdict::Pending,
            Self::Pass => vw_store::VerificationVerdict::Pass,
            Self::Fail => vw_store::VerificationVerdict::Fail,
        }
    }
    fn from_native(value: vw_store::VerificationVerdict) -> Self {
        match value {
            vw_store::VerificationVerdict::Pending => Self::Pending,
            vw_store::VerificationVerdict::Pass => Self::Pass,
            vw_store::VerificationVerdict::Fail => Self::Fail,
        }
    }
}
pub(super) fn snapshot(
    pair: &VerificationPair,
    value: vw_store::VerificationReviewSnapshot,
) -> VerificationReviewSnapshot {
    VerificationReviewSnapshot {
        pair: pair.clone(),
        project_id: value.pair.project_id.to_string(),
        marker_count: value.pair.marker_count,
        version: value.version,
        markers: value
            .markers
            .into_iter()
            .map(|m| VerificationMarkerReview {
                original_number: m.original_number,
                verdict: VerificationVerdict::from_native(m.verdict),
                note: m.note,
            })
            .collect(),
    }
}
impl VerificationReviewSnapshot {
    pub(super) fn native(
        &self,
        pair: &VerificationPair,
        checked: &vw_store::VerificationReviewPair,
    ) -> PackageResult<vw_store::VerificationReviewSnapshot> {
        if &self.pair != pair
            || self.project_id != checked.project_id.as_str()
            || self.marker_count != checked.marker_count
            || self.markers.len() > vw_store::VERIFICATION_REVIEW_MAX_MARKERS
        {
            return Err(PackageError::Stale);
        }
        // Reject before cloning any caller-supplied private note allocation.
        let mut notes = 0_usize;
        for marker in &self.markers {
            if marker.note.len() > 32 * 1024 || marker.note.contains('\0') {
                return Err(PackageError::Invalid);
            }
            notes = notes
                .checked_add(marker.note.len())
                .ok_or(PackageError::Limit)?;
            if notes > 128 * 1024 {
                return Err(PackageError::Limit);
            }
        }
        let value = vw_store::VerificationReviewSnapshot {
            pair: checked.clone(),
            version: self.version,
            markers: self
                .markers
                .iter()
                .map(|m| vw_store::VerificationMarkerReview {
                    original_number: m.original_number,
                    verdict: m.verdict.native(),
                    note: m.note.clone(),
                })
                .collect(),
        };
        value.validate().map_err(super::store_error)?;
        Ok(value)
    }
}
pub(super) fn failure_review(
    snapshot: &vw_store::VerificationReviewSnapshot,
) -> vw_package::FailureReview {
    fn identity(key: &vw_store::VerificationPackageKey) -> vw_package::VerificationPackageIdentity {
        vw_package::VerificationPackageIdentity {
            package_id: key.package_id.clone(),
            target: key.target.clone(),
            manifest_sha256: key.manifest_sha256.clone(),
        }
    }
    vw_package::FailureReview {
        fence: vw_package::FailureReviewFence {
            original: identity(&snapshot.pair.original),
            after: identity(&snapshot.pair.after),
            comparison_receipt_id: snapshot.pair.comparison_receipt_id.clone(),
            comparison_receipt_blake3: snapshot.pair.comparison_receipt_blake3.clone(),
            version: snapshot.version,
        },
        markers: snapshot
            .markers
            .iter()
            .map(|m| vw_package::FailureMarkerAssessment {
                original_number: m.original_number,
                verdict: match m.verdict {
                    vw_store::VerificationVerdict::Pending => vw_package::FailureVerdict::Pending,
                    vw_store::VerificationVerdict::Pass => vw_package::FailureVerdict::Pass,
                    vw_store::VerificationVerdict::Fail => vw_package::FailureVerdict::Fail,
                },
            })
            .collect(),
    }
}
