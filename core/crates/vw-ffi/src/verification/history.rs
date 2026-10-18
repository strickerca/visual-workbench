//! Read private saved reviews from the currently borrowed original project.
//! Historical display does not require its old source revision to stay current;
//! ordinary review mutation and failure compilation retain their exact fences.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

fn current(closed: &AtomicBool, cancellation: &Cancellation) -> PackageResult<()> {
    if closed.load(Ordering::Acquire) {
        return Err(PackageError::Closed);
    }
    cancellation.check()?;
    Ok(())
}
fn pair(value: &vw_store::VerificationReviewPair) -> VerificationPair {
    fn key(value: &vw_store::VerificationPackageKey) -> VerificationPackageKey {
        VerificationPackageKey {
            package_id: value.package_id.to_string(),
            target: value.target.clone(),
            manifest_sha256: value.manifest_sha256.clone(),
        }
    }
    VerificationPair {
        original: key(&value.original),
        after: key(&value.after),
        receipt_id: value.comparison_receipt_id.to_string(),
        receipt_blake3: value.comparison_receipt_blake3.clone(),
    }
}
fn stored(
    value: &VerificationReviewSnapshot,
) -> PackageResult<vw_store::VerificationReviewSnapshot> {
    value.pair.validate()?;
    if value.version == 0 {
        return Err(PackageError::Stale);
    }
    let binding = vw_store::VerificationReviewPair {
        project_id: vw_model::Id::try_from(value.project_id.clone())?,
        original: value.pair.original.native()?,
        after: value.pair.after.native()?,
        comparison_receipt_id: vw_model::Id::try_from(value.pair.receipt_id.clone())?,
        comparison_receipt_blake3: value.pair.receipt_blake3.clone(),
        marker_count: value.marker_count,
    };
    value.native(&value.pair, &binding)
}
#[uniffi::export]
impl ProjectSession {
    /// Enumeration comes from the existing private settings owner, never a
    /// portable manifest or agent-provided pair. Store admission bounds every
    /// record and the complete page before it is lowered across the ABI.
    pub async fn verification_review_history(
        &self,
        after: Option<String>,
        limit: u32,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<VerificationReviewHistoryPage> {
        self.check_open()?;
        if !(1..=8).contains(&limit) {
            return Err(PackageError::Invalid);
        }
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    current(&closed, &cancellation)?;
                    let page = state
                        .store
                        .as_mut()
                        .ok_or(PackageError::Closed)?
                        .verification_review_history(after, limit)
                        .map_err(store_error)?;
                    let snapshots = page
                        .snapshots
                        .into_iter()
                        .map(|value| dto::snapshot(&pair(&value.pair), value))
                        .collect();
                    current(&closed, &cancellation)?;
                    Ok(VerificationReviewHistoryPage {
                        snapshots,
                        next_cursor: page.next_cursor,
                    })
                })())
            })
            .await?
    }
    /// A history selection must still match the exact saved private record in
    /// this original store. This read-only check neither borrows an after source
    /// nor grants permission to change reviews, compile or recapture.
    pub async fn check_verification_history(
        &self,
        expected: VerificationReviewSnapshot,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<()> {
        self.check_open()?;
        let expected = stored(&expected)?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    current(&closed, &cancellation)?;
                    state
                        .store
                        .as_mut()
                        .ok_or(PackageError::Closed)?
                        .check_verification_review(&expected)
                        .map_err(store_error)?;
                    current(&closed, &cancellation)
                })())
            })
            .await?
    }
}
