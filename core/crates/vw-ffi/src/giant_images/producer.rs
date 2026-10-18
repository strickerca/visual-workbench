//! Approved platform producer/import ownership. A completed runner, rather than
//! a caller-selected directory or persisted approval marker, mints a build.
#![allow(
    dead_code,
    reason = "Private producer interfaces are admitted; root foreign registration and packaged platform evidence remain pending"
)]
mod android_bridge;
mod cache;
mod cleanup;
mod input;
mod publish;
#[cfg(windows)]
mod windows;

use crate::{WorkflowBinding, project::ProjectState, worker::Worker};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use vw_stream::SessionBudget;

pub(crate) use input::{GiantInputLimits, GiantSourceInfo, RootOwnedPickedOriginal};
pub(crate) use publish::{GiantPublishOptions, PublishedProject};
#[cfg(windows)]
#[allow(
    unused_imports,
    reason = "Root packaged-runtime registration awaits the Windows execution receipt"
)]
pub(crate) use windows::{
    PinnedWindowsRuntime, admit_packaged_windows_runtime, pin_windows_runtime,
};

use super::source::{self, CapturedSource};
use super::{GiantImageError as Error, GiantImageResult as Result};
use crate::Cancellation;
use vw_assets::{MAX_ENCODED_TILE_BYTES, MAX_MANIFEST_BYTES, Manifest, TILE_SIZE};

pub(super) const MAX_DISK_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const BAND_WORKSPACE_BYTES: u64 = 32 * 1024 * 1024;

/// Internal planning data, not an approval token or a claim of runtime support.
/// Root may lower this through its own explicit producer/import API later.
#[allow(
    dead_code,
    reason = "Bounded staged plan awaits the root-owned producer/import API; no runtime producer is admitted"
)]
pub(crate) struct StagedProducerPlan {
    pub asset_id: String,
    pub original_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub orientation: u8,
    pub format: &'static str,
    pub tile_size: u32,
    pub tile_count: usize,
    pub disk_cache_bytes: u64,
    pub maximum_tile_bytes: usize,
    pub maximum_manifest_bytes: usize,
    pub maximum_band_workspace_bytes: u64,
    pub android_band_fits: bool,
}

pub(super) fn stage(source: &CapturedSource, disk_cache_bytes: u64) -> Result<StagedProducerPlan> {
    if !(1..=MAX_DISK_BYTES).contains(&disk_cache_bytes) {
        return Err(Error::Limit);
    }
    let pyramid = source.source.validate().map_err(|_| Error::Limit)?;
    if !matches!(source.format, "png" | "jpeg")
        || source.source.bit_depth != 8
        || source.source.icc.is_some()
        || source.source.display_conversion
    {
        return Err(Error::Unsupported);
    }
    let band = u64::from(source.source.width)
        .checked_mul(u64::from(source.source.height.min(TILE_SIZE)))
        .and_then(|bytes| bytes.checked_mul(8))
        .ok_or(Error::Limit)?;
    Ok(StagedProducerPlan {
        asset_id: super::tiles::hex(&source.source.original),
        original_bytes: source.source.original_bytes,
        width: source.source.width,
        height: source.source.height,
        orientation: source.source.orientation,
        format: source.format,
        tile_size: TILE_SIZE,
        tile_count: pyramid.tile_count(),
        disk_cache_bytes,
        maximum_tile_bytes: MAX_ENCODED_TILE_BYTES,
        maximum_manifest_bytes: MAX_MANIFEST_BYTES,
        maximum_band_workspace_bytes: BAND_WORKSPACE_BYTES,
        android_band_fits: band <= BAND_WORKSPACE_BYTES,
    })
}

/// Legacy negative-test seam. A raw manifest has no scoped runner witness.
/// Production reads use approved_manifest and its exact project root.
#[cfg(test)]
pub(super) fn require_approved_producer(
    _manifest: &Manifest,
    cancellation: &Cancellation,
) -> Result<()> {
    cancellation.check().map_err(Error::from)?;
    Err(Error::Unsupported)
}

fn io_error(error: std::io::Error) -> Error {
    match error.kind() {
        std::io::ErrorKind::NotFound => Error::Missing,
        std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::InvalidInput => Error::Invalid,
        _ => Error::Storage,
    }
}

pub(super) fn approved_manifest(
    root: &Path,
    manifest: &Manifest,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<()> {
    cache::approved(root, manifest, budget, cancel)
}

#[derive(Clone, Copy)]
pub(crate) enum GiantProducerPlatform {
    AndroidRegionJpeg,
    WindowsPinnedVips,
}
pub(crate) struct GiantProducerOptions {
    pub platform: GiantProducerPlatform,
    pub disk_cache_bytes: u64,
    pub timeout_ms: u64,
    pub assume_untagged_srgb: bool,
}

struct PreparedState {
    source: Arc<input::PreparedSource>,
    build_started: bool,
}
#[derive(uniffi::Object)]
pub struct PreparedOriginal {
    worker: Worker<Option<PreparedState>>,
    closed: Arc<AtomicBool>,
    budget: SessionBudget,
    info: GiantSourceInfo,
}
/// Root lowers a picker-owned source, exact private work root and explicit
/// untagged-sRGB consent. No original is read into a byte vector.
pub(crate) async fn prepare_original(
    original: RootOwnedPickedOriginal,
    limits: GiantInputLimits,
    budget: SessionBudget,
    cancel: Arc<Cancellation>,
) -> Result<Arc<PreparedOriginal>> {
    struct Preparing {
        closed: Arc<AtomicBool>,
        armed: bool,
    }
    impl Drop for Preparing {
        fn drop(&mut self) {
            if self.armed {
                self.closed.store(true, Ordering::Release);
            }
        }
    }
    let closed = Arc::new(AtomicBool::new(false));
    let mut preparing = Preparing {
        closed: closed.clone(),
        armed: true,
    };
    // The existing global four-startup limit applies before a native thread is
    // created. The input permit further limits live original/output owners to
    // two. Abandoning the foreign future closes the actual streaming copy.
    let input = crate::worker::startup(move || {
        Ok((|| -> Result<Arc<PreparedOriginal>> {
            let source = input::prepare(
                original,
                limits,
                &closed,
                &cancel,
                &budget,
                &vw_store::RealFreeSpace,
            )?;
            let info = source.info.clone();
            let state = PreparedState {
                source,
                build_started: false,
            };
            let worker = Worker::new("vw-giant-input", Some(state), 2).map_err(Error::from)?;
            Ok(Arc::new(PreparedOriginal {
                worker,
                closed,
                budget,
                info,
            }))
        })())
    })
    .await
    .map_err(Error::from)??;
    preparing.armed = false;
    Ok(input)
}
impl Drop for PreparedOriginal {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
    }
}
impl PreparedOriginal {
    pub(crate) fn describe(&self) -> GiantSourceInfo {
        self.info.clone()
    }
    pub(crate) async fn build_approved(
        &self,
        options: GiantProducerOptions,
        cancel: Arc<Cancellation>,
    ) -> Result<Arc<GiantBuiltCache>> {
        source::live(&self.closed, &cancel)?;
        if !options.assume_untagged_srgb {
            return Err(Error::Unsupported);
        }
        if !(1..=MAX_DISK_BYTES).contains(&options.disk_cache_bytes)
            || !(1..=600_000).contains(&options.timeout_ms)
        {
            return Err(Error::Limit);
        }
        match options.platform {
            GiantProducerPlatform::AndroidRegionJpeg => {
                #[cfg(target_os = "android")]
                {
                    self.build_android(options.timeout_ms, options.disk_cache_bytes, cancel)
                        .await
                }
                #[cfg(not(target_os = "android"))]
                {
                    Err(Error::Unsupported)
                }
            }
            GiantProducerPlatform::WindowsPinnedVips => {
                #[cfg(windows)]
                {
                    self.build_windows(
                        windows::packaged()?,
                        options.timeout_ms,
                        options.disk_cache_bytes,
                        cancel,
                    )
                    .await
                }
                #[cfg(not(windows))]
                {
                    Err(Error::Unsupported)
                }
            }
        }
    }
    #[cfg(target_os = "android")]
    async fn build_android(
        &self,
        timeout_ms: u64,
        disk_cache_bytes: u64,
        cancel: Arc<Cancellation>,
    ) -> Result<Arc<GiantBuiltCache>> {
        let closed = self.closed.clone();
        let budget = self.budget.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    source::live(&closed, &cancel)?;
                    let state = state.as_mut().ok_or(Error::Closed)?;
                    if state.build_started {
                        return Err(Error::Backpressure);
                    }
                    state.build_started = true;
                    let build = android_bridge::build(
                        state.source.clone(),
                        disk_cache_bytes,
                        timeout_ms,
                        closed,
                        cancel,
                        budget.clone(),
                    )?;
                    let worker = Worker::new(
                        "vw-giant-built",
                        Some(BuiltState {
                            build: Arc::new(build),
                            publication_started: false,
                        }),
                        2,
                    )
                    .map_err(Error::from)?;
                    Ok(Arc::new(GiantBuiltCache {
                        worker,
                        closed: Arc::new(AtomicBool::new(false)),
                        budget,
                    }))
                })())
            })
            .await
            .map_err(Error::from)?
    }
    #[cfg(windows)]
    async fn build_windows(
        &self,
        runtime: Arc<PinnedWindowsRuntime>,
        timeout_ms: u64,
        disk_cache_bytes: u64,
        cancel: Arc<Cancellation>,
    ) -> Result<Arc<GiantBuiltCache>> {
        let closed = self.closed.clone();
        let budget = self.budget.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    source::live(&closed, &cancel)?;
                    let state = state.as_mut().ok_or(Error::Closed)?;
                    if state.build_started {
                        return Err(Error::Backpressure);
                    }
                    // Exactly one output directory per admitted original. A failed
                    // attempt requires fresh preparation; quarantined cleanup retains
                    // this input's permit and charges.
                    state.build_started = true;
                    let build = windows::build(
                        state.source.clone(),
                        runtime,
                        timeout_ms,
                        disk_cache_bytes,
                        closed.clone(),
                        cancel,
                        budget.clone(),
                    )?;
                    let worker = Worker::new(
                        "vw-giant-built",
                        Some(BuiltState {
                            build: Arc::new(build),
                            publication_started: false,
                        }),
                        2,
                    )
                    .map_err(Error::from)?;
                    Ok(Arc::new(GiantBuiltCache {
                        worker,
                        closed: Arc::new(AtomicBool::new(false)),
                        budget,
                    }))
                })())
            })
            .await
            .map_err(Error::from)?
    }
    pub(crate) async fn close_worker(&self) -> Result<()> {
        self.closed.store(true, Ordering::Release);
        self.worker
            .shutdown(|state| match state.take() {
                Some(state) => input::release(state.source).map_err(core_error),
                None => Ok(()),
            })
            .await
            .map_err(Error::from)
    }
}

struct BuiltState {
    build: Arc<cache::ApprovedBuild>,
    publication_started: bool,
}
#[derive(uniffi::Object)]
pub struct GiantBuiltCache {
    worker: Worker<Option<BuiltState>>,
    closed: Arc<AtomicBool>,
    budget: SessionBudget,
}
impl Drop for GiantBuiltCache {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
    }
}
impl GiantBuiltCache {
    pub(crate) async fn publish(
        &self,
        options: GiantPublishOptions,
        cancel: Arc<Cancellation>,
    ) -> Result<PublishedProject> {
        let closed = self.closed.clone();
        let budget = self.budget.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    source::live(&closed, &cancel)?;
                    let state = state.as_mut().ok_or(Error::Closed)?;
                    if state.publication_started {
                        return Err(Error::Backpressure);
                    }
                    state.publication_started = true;
                    publish::publish(&state.build, options, &closed, &cancel, &budget)
                })())
            })
            .await
            .map_err(Error::from)?
    }
    pub(crate) async fn rebuild(
        &self,
        project: Worker<ProjectState>,
        project_closed: Arc<AtomicBool>,
        binding: WorkflowBinding,
        cancel: Arc<Cancellation>,
    ) -> Result<()> {
        let closed = self.closed.clone();
        let budget = self.budget.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    source::live(&closed, &cancel)?;
                    let build = state.as_ref().ok_or(Error::Closed)?.build.clone();
                    let (send, receive) = std::sync::mpsc::sync_channel(1);
                    // This is the separate built-cache worker, never the project
                    // worker itself. Join actual accepted work even if its foreign
                    // future is abandoned, so close cannot retire live owners.
                    project
                        .enqueue(Box::new(move |state| {
                            let result = publish::rebuild_in_project_worker(
                                state,
                                &build,
                                &project_closed,
                                &binding,
                                &closed,
                                &cancel,
                                &budget,
                            );
                            let _ = send.send(result);
                        }))
                        .map_err(Error::from)?;
                    receive.recv().map_err(|_| Error::Storage)?
                })())
            })
            .await
            .map_err(Error::from)?
    }
    pub(crate) async fn close_worker(&self) -> Result<()> {
        self.closed.store(true, Ordering::Release);
        self.worker
            .shutdown(|state| match state.take() {
                Some(state) => cache::release(state.build).map_err(core_error),
                None => Ok(()),
            })
            .await
            .map_err(Error::from)
    }
}
// Shutdown completion must preserve a storage/cancellation failure without
// teaching shared worker ownership about platform producer types.
fn core_error(error: Error) -> crate::CoreError {
    match error {
        Error::Closed => crate::CoreError::Closed,
        Error::Cancelled => crate::CoreError::Cancelled,
        Error::Backpressure => crate::CoreError::Backpressure,
        _ => crate::CoreError::Storage,
    }
}

#[cfg(test)]
mod producer_tests;
