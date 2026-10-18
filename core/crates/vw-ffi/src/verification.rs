//! Manual installation-local verification reviews over existing joined owners.
//!
//! The trusted Desktop workflow retains the exact capture selector/grant and
//! both source projects for the complete call. Catalog bytes, inbox PNGs and
//! matching source state cannot establish that causal capture association by
//! themselves. Imported lineage and agent result assertions never establish it.
//! See verification/INTEGRATION.md for the three minimal reserved owner hooks,
//! registration and deferred execution prerequisites.
mod checked;
mod dto;
mod prepare;
#[cfg(test)]
mod tests;

use crate::{
    Cancellation, CompiledPackage, McpResultInbox, PackageCatalog, PackageError, PackageResult,
    ProjectSession,
};
use checked::{CheckedPair, Stop};
pub use dto::*;
use std::sync::Arc;

const MAX_MEMORY: u64 = 256 * 1024 * 1024;
const MAX_PACKAGE: usize = 32 * 1024 * 1024;
fn limits(memory_bytes: u64) -> vw_package::Limits {
    vw_package::Limits {
        memory_bytes,
        source_bytes: 64 * 1024 * 1024,
        source_pixels: 50_000_000,
        package_bytes: MAX_PACKAGE,
        image_bytes: 4 * 1024 * 1024,
        markers: 64,
    }
}
fn retained_bytes(checked: &CheckedPair) -> PackageResult<u64> {
    let encoded = checked
        .original
        .files()
        .chain(checked.after.files())
        .try_fold(0_u64, |total, (_, bytes)| {
            total
                .checked_add(bytes.len() as u64)
                .ok_or(PackageError::Limit)
        })?;
    // The checked files owner permits capacity slack; parsed manifest strings
    // and bounded private review/projection metadata also remain resident.
    encoded
        .checked_mul(2)
        .and_then(|n| n.checked_add(8 * 1024 * 1024))
        .ok_or(PackageError::Limit)
}
fn store_error(error: vw_store::VerificationReviewError) -> PackageError {
    use vw_store::VerificationReviewError as E;
    match error {
        E::Invalid | E::Pending => PackageError::Invalid,
        E::Limit => PackageError::Limit,
        E::Conflict => PackageError::Stale,
        E::Cancelled => PackageError::Cancelled,
        E::Corrupt => PackageError::Integrity,
        E::Storage(_) => PackageError::Storage,
    }
}
fn instruction_error(error: vw_instructions::Error) -> PackageError {
    use vw_instructions::Error as E;
    match error {
        E::Limit(_) | E::Exhausted => PackageError::Limit,
        E::Stale => PackageError::Stale,
        E::Closed => PackageError::Closed,
        _ => PackageError::Invalid,
    }
}
async fn after_current(
    after: &ProjectSession,
    checked: Arc<CheckedPair>,
    original_closed: Arc<std::sync::atomic::AtomicBool>,
    cancellation: Arc<Cancellation>,
) -> PackageResult<()> {
    let after_closed = after.closed.clone();
    after
        .worker
        .call(move |state| {
            Ok((|| {
                Stop {
                    original_closed: &original_closed,
                    after_closed: &after_closed,
                    cancellation: &cancellation,
                }
                .check()?;
                checked::source_current(state, &checked.after)
            })())
        })
        .await?
}
/// Reopen every real catalog/inbox owner after work. The caller joins these
/// checks with its existing serialized recapture/retirement lifecycle; these
/// APIs do not mint a second lease or imply authority over an OS target.
async fn external_current(
    after: &ProjectSession,
    catalog: &PackageCatalog,
    inbox: &McpResultInbox,
    pair: &VerificationPair,
    checked: Arc<CheckedPair>,
    original_closed: Arc<std::sync::atomic::AtomicBool>,
    cancellation: Arc<Cancellation>,
) -> PackageResult<()> {
    checked::catalog_current(catalog, pair, cancellation.clone()).await?;
    checked::receipt_current(inbox, pair, &checked, cancellation.clone()).await?;
    after_current(after, checked, original_closed, cancellation).await
}

#[uniffi::export]
impl ProjectSession {
    pub async fn verification_review(
        &self,
        after: Arc<ProjectSession>,
        catalog: Arc<PackageCatalog>,
        inbox: Arc<McpResultInbox>,
        pair: VerificationPair,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<VerificationReviewSnapshot> {
        self.check_open()?;
        after.check_open()?;
        let checked = checked::load(&catalog, &inbox, &pair, cancellation.clone()).await?;
        after_current(
            &after,
            checked.clone(),
            self.closed.clone(),
            cancellation.clone(),
        )
        .await?;
        let read_checked = checked.clone();
        let original_closed = self.closed.clone();
        let after_closed = after.closed.clone();
        let read_cancel = cancellation.clone();
        let snapshot = self
            .worker
            .call(move |state| {
                Ok((|| {
                    Stop {
                        original_closed: &original_closed,
                        after_closed: &after_closed,
                        cancellation: &read_cancel,
                    }
                    .check()?;
                    checked::source_current(state, &read_checked.original)?;
                    state
                        .store
                        .as_mut()
                        .ok_or(PackageError::Closed)?
                        .verification_review(&read_checked.store_pair)
                        .map_err(store_error)
                })())
            })
            .await??;
        external_current(
            &after,
            &catalog,
            &inbox,
            &pair,
            checked.clone(),
            self.closed.clone(),
            cancellation.clone(),
        )
        .await?;
        self.verification_snapshot_current(&after, checked, snapshot.clone(), cancellation)
            .await?;
        Ok(dto::snapshot(&pair, snapshot))
    }

    /// Owner UI action only. No MCP handler or agent-facing verdict setter is
    /// registered. Existing CAS operation identity recovers a lost response.
    #[allow(clippy::too_many_arguments)]
    pub async fn change_verification_review(
        &self,
        after: Arc<ProjectSession>,
        catalog: Arc<PackageCatalog>,
        inbox: Arc<McpResultInbox>,
        pair: VerificationPair,
        expected_version: u64,
        operation_id: String,
        original_number: u32,
        verdict: VerificationVerdict,
        note: String,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<VerificationReviewSnapshot> {
        self.check_open()?;
        after.check_open()?;
        if expected_version > i64::MAX as u64
            || operation_id.len() != 36
            || note.len() > 32 * 1024
            || note.contains('\0')
        {
            return Err(PackageError::Invalid);
        }
        let operation = vw_model::Id::try_from(operation_id)?;
        let checked = checked::load(&catalog, &inbox, &pair, cancellation.clone()).await?;
        after_current(
            &after,
            checked.clone(),
            self.closed.clone(),
            cancellation.clone(),
        )
        .await?;
        let change_checked = checked.clone();
        let original_closed = self.closed.clone();
        let after_closed = after.closed.clone();
        let change_cancel = cancellation.clone();
        let snapshot = self
            .worker
            .call(move |state| {
                Ok((|| {
                    let stop = Stop {
                        original_closed: &original_closed,
                        after_closed: &after_closed,
                        cancellation: &change_cancel,
                    };
                    stop.check()?;
                    checked::source_current(state, &change_checked.original)?;
                    state
                        .store
                        .as_mut()
                        .ok_or(PackageError::Closed)?
                        .change_verification_review(
                            &change_checked.store_pair,
                            expected_version,
                            &operation,
                            vw_store::VerificationReviewChange {
                                original_number,
                                verdict: verdict.native(),
                                note,
                            },
                            || vw_package::Cancellation::is_cancelled(&stop),
                        )
                        .map_err(store_error)
                })())
            })
            .await??;
        external_current(
            &after,
            &catalog,
            &inbox,
            &pair,
            checked.clone(),
            self.closed.clone(),
            cancellation.clone(),
        )
        .await?;
        self.verification_snapshot_current(&after, checked, snapshot.clone(), cancellation)
            .await?;
        Ok(dto::snapshot(&pair, snapshot))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn compile_verification_failures(
        &self,
        after: Arc<ProjectSession>,
        catalog: Arc<PackageCatalog>,
        inbox: Arc<McpResultInbox>,
        pair: VerificationPair,
        expected: VerificationReviewSnapshot,
        translations: Vec<VerificationMarkerTranslation>,
        package_id: String,
        created_at_ms: i64,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<Option<Arc<CompiledPackage>>> {
        self.check_open()?;
        after.check_open()?;
        pair.validate()?;
        if !(16 * 1024 * 1024..=MAX_MEMORY).contains(&memory_budget_bytes)
            || !(0..=253_402_300_799_999).contains(&created_at_ms)
        {
            return Err(PackageError::Invalid);
        }
        let package_id = vw_model::Id::try_from(package_id)?;
        if package_id.as_str() == pair.original.package_id
            || package_id.as_str() == pair.after.package_id
        {
            return Err(PackageError::Invalid);
        }
        let checked = checked::load(&catalog, &inbox, &pair, cancellation.clone()).await?;
        let expected = expected.native(&pair, &checked.store_pair)?;
        let failed = expected
            .failed_markers()
            .map_err(store_error)?
            .unwrap_or_default();
        let translations = prepare::translations(&failed, translations)?;
        // All-pass needs no new compiled owner. Failures retain the existing
        // capacity before full originals/clone/compiler work; every error,
        // cancellation or dropped future retires that same permit naturally.
        let builder = if failed.is_empty() {
            None
        } else {
            Some(CompiledPackage::verification_builder()?)
        };
        after_current(
            &after,
            checked.clone(),
            self.closed.clone(),
            cancellation.clone(),
        )
        .await?;
        let read_checked = checked.clone();
        let read_expected = expected.clone();
        let read_failed = failed.clone();
        let original_closed = self.closed.clone();
        let after_closed = after.closed.clone();
        let read_cancel = cancellation.clone();
        let authored = self
            .worker
            .call(move |state| {
                Ok((|| {
                    Stop {
                        original_closed: &original_closed,
                        after_closed: &after_closed,
                        cancellation: &read_cancel,
                    }
                    .check()?;
                    checked::source_current(state, &read_checked.original)?;
                    state
                        .store
                        .as_mut()
                        .ok_or(PackageError::Closed)?
                        .check_verification_review(&read_expected)
                        .map_err(store_error)?;
                    prepare::authoring(state, &read_checked.original, &read_failed)
                })())
            })
            .await??;
        let prepared = if failed.is_empty() {
            None
        } else {
            let compile_checked = checked.clone();
            let original_closed = self.closed.clone();
            let after_closed = after.closed.clone();
            let compile_cancel = cancellation.clone();
            let request = prepare::CompileRequest {
                package_id,
                created_at_ms,
                memory_budget_bytes,
                authoring: authored,
                translations,
            };
            Some(
                after
                    .worker
                    .call(move |state| {
                        Ok(prepare::compile(
                            state,
                            &compile_checked,
                            request,
                            &Stop {
                                original_closed: &original_closed,
                                after_closed: &after_closed,
                                cancellation: &compile_cancel,
                            },
                        ))
                    })
                    .await??,
            )
        };
        // Final ordinary compiler output is admitted by the existing failure
        // compiler while original review/source stay serialized on its worker.
        let final_checked = checked.clone();
        let final_expected = expected.clone();
        let original_closed = self.closed.clone();
        let after_closed = after.closed.clone();
        let final_cancel = cancellation.clone();
        let package = self
            .worker
            .call(move |state| {
                Ok((|| {
                    let stop = Stop {
                        original_closed: &original_closed,
                        after_closed: &after_closed,
                        cancellation: &final_cancel,
                    };
                    stop.check()?;
                    let review = dto::failure_review(&final_expected);
                    // All-pass runs only the admitted cheap preflight; no compiler
                    // workspace or new image allocation is required at the low budget.
                    let remaining = if prepared.is_none() {
                        MAX_MEMORY
                    } else {
                        memory_budget_bytes
                            .checked_sub(workflow_workspace(
                                state,
                                &final_checked,
                                memory_budget_bytes,
                            )?)
                            .ok_or(PackageError::Limit)?
                    };
                    let mut fence_error = None;
                    let result = vw_package::compile_verification_failure_package(
                        &final_checked.original,
                        &final_checked.after,
                        &review,
                        prepared,
                        limits(remaining),
                        |candidate| {
                            if candidate != &review {
                                return false;
                            }
                            let current = (|| {
                                stop.check()?;
                                checked::source_current(state, &final_checked.original)?;
                                state
                                    .store
                                    .as_mut()
                                    .ok_or(PackageError::Closed)?
                                    .check_verification_review(&final_expected)
                                    .map_err(store_error)
                            })();
                            match current {
                                Ok(()) => true,
                                Err(error) => {
                                    fence_error = Some(error);
                                    false
                                }
                            }
                        },
                        &stop,
                    );
                    // The pure package callback is boolean; retain the actual
                    // native storage/cancellation/closed error privately.
                    if let Some(error) = fence_error {
                        return Err(error);
                    }
                    let result = result?;
                    stop.check()?;
                    checked::source_current(state, &final_checked.original)?;
                    state
                        .store
                        .as_mut()
                        .ok_or(PackageError::Closed)?
                        .check_verification_review(&final_expected)
                        .map_err(store_error)?;
                    Ok(result)
                })())
            })
            .await??;
        external_current(
            &after,
            &catalog,
            &inbox,
            &pair,
            checked.clone(),
            self.closed.clone(),
            cancellation.clone(),
        )
        .await?;
        self.verification_snapshot_current(&after, checked, expected, cancellation)
            .await?;
        match (package, builder) {
            (Some(package), Some(builder)) => Ok(Some(builder(package))),
            (None, _) => Ok(None),
            (Some(_), None) => Err(PackageError::Integrity),
        }
    }
}
fn workflow_workspace(
    state: &crate::project::ProjectState,
    checked: &CheckedPair,
    budget: u64,
) -> PackageResult<u64> {
    let available = budget
        .checked_sub(retained_bytes(checked)?)
        .ok_or(PackageError::Limit)?;
    Ok(crate::workflow::canonical_workspace(
        state, available, false,
    )?)
}
impl ProjectSession {
    async fn verification_snapshot_current(
        &self,
        after: &ProjectSession,
        checked: Arc<CheckedPair>,
        expected: vw_store::VerificationReviewSnapshot,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<()> {
        let original_closed = self.closed.clone();
        let after_closed = after.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    Stop {
                        original_closed: &original_closed,
                        after_closed: &after_closed,
                        cancellation: &cancellation,
                    }
                    .check()?;
                    checked::source_current(state, &checked.original)?;
                    state
                        .store
                        .as_mut()
                        .ok_or(PackageError::Closed)?
                        .check_verification_review(&expected)
                        .map_err(store_error)
                })())
            })
            .await?
    }
}
