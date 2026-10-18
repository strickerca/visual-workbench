//! Pinned packaged Windows runner. This module alone can mint its completion.
use super::{Error, Result, cache, cleanup, input};
use crate::{Cancellation, giant_images::source};
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};
use vw_stream::{MemoryCharge, MemoryKind, SessionBudget};

/// Construct only from root's trusted packaged inventory. The expected helper
/// digest is not a file-picker argument or a persisted build approval marker.
pub(crate) struct PinnedWindowsRuntime {
    helper: vw_assets_win::LockedHelper,
    runtime: vw_assets_win::LockedRuntime,
    _charge: MemoryCharge,
}
/// Synchronous: root must run pinning on a retained native worker. Pins hold
/// file/ancestor handles against replacement through actual worker completion.
pub(crate) fn pin_windows_runtime(
    helper: &Path,
    expected_helper_sha256: &str,
    directory: &Path,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<Arc<PinnedWindowsRuntime>> {
    cancel.check().map_err(Error::from)?;
    if [helper, directory]
        .iter()
        .any(|path| !path.is_absolute() || path.as_os_str().as_encoded_bytes().len() > 8192)
    {
        return Err(Error::Invalid);
    }
    let charge = source::reserve(budget, MemoryKind::Transfer, 2 * 1024 * 1024)?;
    let helper = vw_assets_win::LockedHelper::open(helper, expected_helper_sha256)
        .map_err(platform_error)?;
    let runtime = vw_assets_win::LockedRuntime::open(directory).map_err(platform_error)?;
    cancel.check().map_err(Error::from)?;
    Ok(Arc::new(PinnedWindowsRuntime {
        helper,
        runtime,
        _charge: charge,
    }))
}
pub(super) fn build(
    prepared: Arc<input::PreparedSource>,
    runtime: Arc<PinnedWindowsRuntime>,
    timeout_ms: u64,
    disk_cache_bytes: u64,
    closed: Arc<AtomicBool>,
    cancel: Arc<Cancellation>,
    budget: SessionBudget,
) -> Result<cache::ApprovedBuild> {
    if !(1000..=600_000).contains(&timeout_ms) {
        return Err(Error::Limit);
    }
    if !runtime._charge.belongs_to(&budget) {
        return Err(Error::Invalid);
    }
    prepared.verify(&closed, &cancel, &budget)?;
    if prepared.required_work_bytes > prepared.max_work_bytes {
        return Err(Error::Limit);
    }
    input::admit_space(
        prepared.staging.path(),
        prepared.required_work_bytes,
        &vw_store::RealFreeSpace,
    )?;
    // Admission before directory/path construction, retained with an exact
    // operation directory even if worker startup or output verification fails.
    let directory_charge = source::reserve(&budget, MemoryKind::Transfer, 1024 * 1024)?;
    let temporary = tempfile::Builder::new()
        .prefix("vw-giant-windows-")
        .tempdir_in(prepared.staging.path())
        .map_err(super::io_error)?;
    let output = cleanup::OwnedDirectory::new(temporary, (prepared.clone(), directory_charge));
    let destination = output.path().join("pyramid");
    let request = vw_assets_win::BuildRequest {
        original_hash: prepared.source.original,
        original_bytes: prepared.source.original_bytes,
        width: prepared.source.width,
        height: prepared.source.height,
        orientation: prepared.source.orientation,
        disk_budget: prepared.max_work_bytes,
        timeout_ms,
    };
    let probe_closed = closed.clone();
    let probe_cancel = cancel.clone();
    let platform_cancel = vw_assets_win::Cancellation::from_probe(Arc::new(move || {
        probe_closed.load(std::sync::atomic::Ordering::Acquire) || probe_cancel.is_cancelled()
    }));
    let result = vw_assets_win::build_budgeted(
        &runtime.helper,
        &runtime.runtime,
        &prepared.info.source_path,
        &destination,
        &request,
        &budget,
        &platform_cancel,
    )
    .map_err(platform_error)?;
    let (manifest, manifest_charge) = result.into_parts();
    // Attach the returned inventory credit before any fallible validation. The
    // actual manifest drops before the credit retained in the same typed build owner.
    let build = cache::ApprovedBuild {
        manifest,
        output,
        source: prepared,
        platform: "windows-libvips-8.18.7",
        disk_cache_bytes,
        _manifest_charge: manifest_charge,
    };
    cache::verify_output(&build, &closed, None, &cancel, &budget)?;
    source::live(&closed, &cancel)?;
    Ok(build)
}
fn platform_error(error: vw_assets_win::Error) -> Error {
    use vw_assets_win::Error as E;
    match error {
        E::Invalid => Error::Invalid,
        E::Unsupported | E::WorkerUnavailable | E::Codec => Error::Unsupported,
        E::Limit => Error::Limit,
        E::Memory { estimated, budget } => Error::Memory { estimated, budget },
        E::Binding => Error::Stale,
        E::Storage => Error::Storage,
        E::Cancelled => Error::Cancelled,
        E::Busy => Error::Backpressure,
        E::Timeout => Error::Limit,
    }
}

static PACKAGED: std::sync::OnceLock<Arc<PinnedWindowsRuntime>> = std::sync::OnceLock::new();
/// Root packaging admission only, after reviewed pins/notices and actual Windows
/// execution receipt. Not exported to foreign/user input APIs.
pub(crate) fn admit_packaged_windows_runtime(
    runtime: Arc<PinnedWindowsRuntime>,
    budget: &SessionBudget,
) -> Result<()> {
    if !runtime._charge.belongs_to(budget) {
        return Err(Error::Invalid);
    }
    PACKAGED.set(runtime).map_err(|_| Error::Backpressure)
}
pub(super) fn packaged() -> Result<Arc<PinnedWindowsRuntime>> {
    PACKAGED.get().cloned().ok_or(Error::Unsupported)
}
