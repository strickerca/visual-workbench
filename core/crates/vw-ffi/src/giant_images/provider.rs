//! Binding-fenced worker ownership for bounded derived image reads.
//!
//! Registration/foreign lowering belongs to the root adapter. In particular,
//! ChargedTile must not be flattened to a byte Vec while dropping its charge.
use super::source::{self, CapturedSource, live, reserve, store_error};
use super::tiles::{self, CachedTile};
use super::{
    GiantImageError as Error, GiantImageResult as Result, GiantOpenOptions, GiantTileAddress,
    GiantTilePixels, GiantViewport, GiantViewportPlan,
};
use crate::{Cancellation, WorkflowBinding, project::ProjectState, worker::Worker};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use vw_assets::{AssetError, BudgetCache, CacheLease, Manifest, TileKey, Viewport};
use vw_store::DiskCache;
use vw_stream::{MemoryCharge, MemoryKind, SessionBudget};

#[path = "editing.rs"]
mod editing;

fn asset_error(error: AssetError) -> Error {
    match error {
        AssetError::Busy | AssetError::Poisoned => Error::Backpressure,
        AssetError::Limit(_) => Error::Limit,
        AssetError::Stale | AssetError::Binding => Error::Stale,
        AssetError::Cancelled => Error::Cancelled,
        AssetError::Storage => Error::Storage,
        AssetError::Invalid(_) => Error::Invalid,
    }
}

struct ImageState {
    disk: DiskCache,
    manifest: Manifest,
    memory: BudgetCache<TileKey, CachedTile>,
    identity: String,
    // Declared after every owned allocation so credits follow destruction.
    _manifest_charge: MemoryCharge,
    _store_charge: MemoryCharge,
}
impl ImageState {
    fn open(
        source: &CapturedSource,
        options: &GiantOpenOptions,
        closed: &AtomicBool,
        cancel: &Cancellation,
        budget: &SessionBudget,
    ) -> Result<Self> {
        live(closed, cancel)?;
        source.verify(closed, cancel, budget)?;
        let _plan = super::producer::stage(source, options.disk_cache_bytes)?;
        if options.ram_cache_bytes == 0 || options.ram_cache_bytes > budget.limit_bytes() {
            return Err(Error::Limit);
        }
        // DiskCache validates up to 16 MiB of JSON plus its bounded map/index
        // serialization. This conservative envelope is shared-session admission,
        // not a measurement of native or process RSS.
        let store_charge = reserve(budget, MemoryKind::CodecWorkspace, tiles::STORE_RESIDENT)?;
        let mut disk = {
            let _index = reserve(budget, MemoryKind::CodecWorkspace, tiles::STORE_WORKSPACE)?;
            DiskCache::new(&source.root, options.disk_cache_bytes).map_err(store_error)?
        };
        let admitted =
            tiles::manifest_scoped(&mut disk, &source.root, &source.source, cancel, budget)?;
        if admitted.manifest.encoded_bytes() > options.disk_cache_bytes {
            return Err(Error::Limit);
        }
        let identity = tiles::hex(&admitted.manifest.identity());
        let memory = BudgetCache::new(options.ram_cache_bytes).map_err(asset_error)?;
        live(closed, cancel)?;
        let tiles::AdmittedManifest {
            manifest,
            charge: manifest_charge,
        } = admitted;
        Ok(Self {
            disk,
            manifest,
            memory,
            identity,
            _manifest_charge: manifest_charge,
            _store_charge: store_charge,
        })
    }
    fn viewport(&self, view: GiantViewport, budget: &SessionBudget) -> Result<GiantViewportPlan> {
        let _plan = reserve(budget, MemoryKind::Transfer, 1024 * 1024)?;
        let scale = physical_scale(&view)?;
        let plan = self
            .manifest
            .pyramid()
            .viewport(Viewport {
                left: view.left,
                top: view.top,
                right: view.right,
                bottom: view.bottom,
                physical_scale: scale,
            })
            .map_err(asset_error)?;
        Ok(GiantViewportPlan {
            asset_id: tiles::hex(&self.manifest.source().original),
            manifest_hash: self.identity.clone(),
            level: plan.level,
            visible: plan.visible.into_iter().map(address).collect(),
            prefetch: plan.prefetch.into_iter().map(address).collect(),
        })
    }
    fn lease(
        &mut self,
        key: TileKey,
        cancel: &Cancellation,
        budget: &SessionBudget,
    ) -> Result<CacheLease<TileKey, CachedTile>> {
        self.manifest.pyramid().region(key).map_err(asset_error)?;
        if let Some(lease) = self.memory.get(&key).map_err(asset_error)? {
            return Ok(lease);
        }
        for attempt in 0..2 {
            let slot = self
                .memory
                .reserve(key, tiles::TILE_RGBA_BYTES)
                .map_err(asset_error)?;
            match CachedTile::decode(&mut self.disk, &self.manifest, key, cancel, budget) {
                Ok(payload) => {
                    return slot
                        .commit(payload, tiles::TILE_RGBA_BYTES)
                        .map_err(asset_error);
                }
                Err(Error::Memory { .. }) if attempt == 0 => {
                    drop(slot);
                    // Shared pressure can occur before this local cache is full.
                    // Clear releases only unpinned actual payloads/charges; a
                    // pinned resource continues to refuse new admission.
                    self.memory.clear().map_err(asset_error)?;
                }
                Err(error) => return Err(error),
            }
        }
        Err(Error::Backpressure)
    }
    fn tile(
        &mut self,
        requested: GiantTileAddress,
        cancel: &Cancellation,
        budget: &SessionBudget,
    ) -> Result<ChargedTile> {
        let key = key(requested);
        let region = self.manifest.pyramid().region(key).map_err(asset_error)?;
        let lease = self.lease(key, cancel, budget)?;
        let payload = lease.payload().map_err(asset_error)?;
        let charge = reserve(budget, MemoryKind::Transfer, payload.rgba.len() as u64)?;
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(payload.rgba.len())
            .map_err(|_| Error::Limit)?;
        rgba.extend_from_slice(&payload.rgba);
        if rgba.capacity() as u64 > charge.bytes() {
            return Err(Error::Limit);
        }
        let (x, y, width, height) = document_region(&self.manifest, key)?;
        cancel.check().map_err(Error::from)?;
        Ok(ChargedTile {
            pixels: GiantTilePixels {
                asset_id: tiles::hex(&self.manifest.source().original),
                manifest_hash: self.identity.clone(),
                address: requested,
                document_x: x,
                document_y: y,
                document_width: width,
                document_height: height,
                width: region.width,
                height: region.height,
                rgba,
            },
            charge,
        })
    }
}

/// Native delivery owner. Root must retain this charge through UniFFI buffer
/// destruction or transfer it with the exact same allocation to its owned
/// foreign token. A physical Kotlin/renderer copy needs another reservation.
/// The fields are private so a caller cannot accidentally discard the charge.
pub(crate) struct ChargedTile {
    pixels: GiantTilePixels,
    charge: MemoryCharge,
}
impl ChargedTile {
    #[cfg(test)]
    pub(crate) fn pixels(&self) -> &GiantTilePixels {
        &self.pixels
    }
    /// Only the root lowering adapter may split ownership. It must keep both
    /// returned values alive until the byte-bearing allocation is destroyed.
    pub(crate) fn into_parts(self) -> (GiantTilePixels, MemoryCharge) {
        (self.pixels, self.charge)
    }
}

#[derive(uniffi::Object)]
pub struct GiantImage {
    project: Worker<ProjectState>,
    project_closed: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    binding: WorkflowBinding,
    source: CapturedSource,
    budget: SessionBudget,
    worker: Worker<Option<ImageState>>,
}
impl GiantImage {
    async fn check(&self, cancel: Arc<Cancellation>) -> Result<()> {
        live(&self.closed, &cancel)?;
        let binding = self.binding.clone();
        let expected = self.source.clone();
        let closed = self.project_closed.clone();
        let budget = self.budget.clone();
        let worker_cancel = cancel.clone();
        self.project
            .call(move |state| {
                Ok(
                    source::capture(state, &binding, &closed, &worker_cancel, &budget).and_then(
                        |source| {
                            if source.same_source(&expected) {
                                Ok(())
                            } else {
                                Err(Error::Stale)
                            }
                        },
                    ),
                )
            })
            .await
            .map_err(Error::from)??;
        live(&self.project_closed, &cancel)?;
        live(&self.closed, &cancel)
    }
    pub(crate) async fn read_viewport(
        &self,
        view: GiantViewport,
        cancellation: Arc<Cancellation>,
    ) -> Result<GiantViewportPlan> {
        self.check(cancellation.clone()).await?;
        let source = self.source.clone();
        let closed = self.project_closed.clone();
        let local_closed = self.closed.clone();
        let budget = self.budget.clone();
        let cancel = cancellation.clone();
        let result = self
            .worker
            .call(move |state| {
                Ok((|| {
                    live(&local_closed, &cancel)?;
                    source.verify_owned(&closed, &local_closed, &cancel, &budget)?;
                    let plan = state
                        .as_ref()
                        .ok_or(Error::Closed)?
                        .viewport(view, &budget)?;
                    source.verify_owned(&closed, &local_closed, &cancel, &budget)?;
                    live(&local_closed, &cancel)?;
                    Ok::<_, Error>(plan)
                })())
            })
            .await
            .map_err(Error::from)??;
        self.check(cancellation).await?;
        Ok(result)
    }
    pub(crate) async fn read_tile(
        &self,
        address: GiantTileAddress,
        cancellation: Arc<Cancellation>,
    ) -> Result<ChargedTile> {
        self.check(cancellation.clone()).await?;
        let source = self.source.clone();
        let closed = self.project_closed.clone();
        let local_closed = self.closed.clone();
        let budget = self.budget.clone();
        let cancel = cancellation.clone();
        let result = self
            .worker
            .call(move |state| {
                Ok((|| {
                    live(&local_closed, &cancel)?;
                    source.verify_owned(&closed, &local_closed, &cancel, &budget)?;
                    let tile = state
                        .as_mut()
                        .ok_or(Error::Closed)?
                        .tile(address, &cancel, &budget)?;
                    source.verify_owned(&closed, &local_closed, &cancel, &budget)?;
                    live(&local_closed, &cancel)?;
                    Ok::<_, Error>(tile)
                })())
            })
            .await
            .map_err(Error::from)??;
        self.check(cancellation).await?;
        Ok(result)
    }
    pub(crate) async fn close_worker(&self) -> Result<()> {
        self.closed.store(true, Ordering::Release);
        self.worker
            .shutdown(|state| {
                drop(state.take());
                Ok(())
            })
            .await
            .map_err(Error::from)
    }
}

pub(crate) async fn open_image(
    project: Worker<ProjectState>,
    project_closed: Arc<AtomicBool>,
    options: GiantOpenOptions,
    budget: SessionBudget,
    cancellation: Arc<Cancellation>,
) -> Result<Arc<GiantImage>> {
    live(&project_closed, &cancellation)?;
    let binding = options.binding.clone();
    let capture_binding = binding.clone();
    let capture_closed = project_closed.clone();
    let capture_budget = budget.clone();
    let capture_cancel = cancellation.clone();
    let source = project
        .call(move |state| {
            Ok(source::capture(
                state,
                &capture_binding,
                &capture_closed,
                &capture_cancel,
                &capture_budget,
            ))
        })
        .await
        .map_err(Error::from)??;
    let worker = Worker::new("vw-giant-image", None, 2).map_err(Error::from)?;
    let image = Arc::new(GiantImage {
        project,
        project_closed,
        closed: Arc::new(AtomicBool::new(false)),
        binding,
        source,
        budget,
        worker,
    });
    let source = image.source.clone();
    let closed = image.project_closed.clone();
    let budget = image.budget.clone();
    let cancel = cancellation.clone();
    let opened = image
        .worker
        .call(move |state| {
            Ok((|| {
                *state = Some(ImageState::open(
                    &source, &options, &closed, &cancel, &budget,
                )?);
                Ok(())
            })())
        })
        .await
        .map_err(Error::from)
        .and_then(|result| result);
    if let Err(error) = opened {
        image.close_worker().await?;
        return Err(error);
    }
    if let Err(error) = image.check(cancellation).await {
        image.close_worker().await?;
        return Err(error);
    }
    Ok(image)
}

fn key(address: GiantTileAddress) -> TileKey {
    TileKey {
        level: address.level,
        x: address.x,
        y: address.y,
    }
}
fn address(key: TileKey) -> GiantTileAddress {
    GiantTileAddress {
        level: key.level,
        x: key.x,
        y: key.y,
    }
}
fn physical_scale(view: &GiantViewport) -> Result<f64> {
    let scale =
        vw_assets::maximum_physical_scale(view.a, view.b, view.c, view.d).map_err(asset_error)?;
    let m = view
        .a
        .abs()
        .max(view.b.abs())
        .max(view.c.abs())
        .max(view.d.abs());
    let determinant = (view.a / m) * (view.d / m) - (view.b / m) * (view.c / m);
    if !determinant.is_finite() || determinant == 0.0 {
        return Err(Error::Invalid);
    }
    Ok(scale)
}
fn document_region(manifest: &Manifest, key: TileKey) -> Result<(u32, u32, u32, u32)> {
    let region = manifest.pyramid().region(key).map_err(asset_error)?;
    let factor = 1u32.checked_shl(u32::from(key.level)).ok_or(Error::Limit)?;
    let x = region.x.checked_mul(factor).ok_or(Error::Limit)?;
    let y = region.y.checked_mul(factor).ok_or(Error::Limit)?;
    let source = manifest.source();
    let width = region
        .width
        .checked_mul(factor)
        .ok_or(Error::Limit)?
        .min(source.width.checked_sub(x).ok_or(Error::Invalid)?);
    let height = region
        .height
        .checked_mul(factor)
        .ok_or(Error::Limit)?
        .min(source.height.checked_sub(y).ok_or(Error::Invalid)?);
    if width == 0 || height == 0 {
        return Err(Error::Invalid);
    }
    Ok((x, y, width, height))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
