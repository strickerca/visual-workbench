use super::*;
use crate::{ProjectSession, project::ProjectState, workflow};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, atomic::AtomicUsize},
};
use vw_model::{AssetId, Id};
use vw_proto::v1::object_state::Shape;
static COMPILED: AtomicUsize = AtomicUsize::new(0);
pub(super) struct Permit;
impl Permit {
    fn acquire() -> PackageResult<Self> {
        COMPILED
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 2).then_some(n + 1)
            })
            .map_err(|_| PackageError::Busy)?;
        Ok(Self)
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        COMPILED.fetch_sub(1, Ordering::AcqRel);
    }
}
pub(super) struct Prepared {
    pub package: vw_package::Package,
    _permit: Permit,
}
#[derive(uniffi::Object)]
pub struct CompiledPackage {
    prepared: Mutex<Option<Arc<Prepared>>>,
}
impl CompiledPackage {
    pub(super) fn get(&self) -> PackageResult<Arc<Prepared>> {
        self.prepared
            .lock()
            .map_err(|_| PackageError::Closed)?
            .as_ref()
            .cloned()
            .ok_or(PackageError::Closed)
    }
}
#[uniffi::export]
impl CompiledPackage {
    pub fn describe(&self) -> PackageResult<PackageInfo> {
        dto::describe(&self.get()?.package)
    }
    pub fn dispose(&self) {
        if let Ok(mut p) = self.prepared.lock() {
            *p = None;
        }
    }
}
impl Drop for CompiledPackage {
    fn drop(&mut self) {
        self.dispose();
    }
}
struct Stop<'a> {
    closed: &'a AtomicBool,
    cancel: &'a Cancellation,
}
impl vw_package::Cancellation for Stop<'_> {
    fn is_cancelled(&self) -> bool {
        self.closed.load(Ordering::Acquire) || self.cancel.is_cancelled()
    }
}
fn originals(
    state: &ProjectState,
    doc: &Id,
    budget: u64,
    cancel: &Stop<'_>,
) -> PackageResult<BTreeMap<AssetId, Vec<u8>>> {
    let project = state.project()?;
    let definition = &project
        .documents
        .get(doc)
        .ok_or(PackageError::Stale)?
        .definition;
    let mut ids = BTreeSet::from([AssetId::try_from(definition.primary_asset_id.clone())?]);
    for object in project
        .objects
        .values()
        .filter(|o| o.document_id == *doc && !o.state.hidden)
    {
        let layer = project
            .layers
            .get(&Id::from_proto(object.state.layer_id.as_ref())?)
            .ok_or(PackageError::Integrity)?;
        if !layer.visible || layer.opacity == 0.0 {
            continue;
        }
        if let Some(Shape::ResultId(result)) = &object.state.shape {
            let result = project
                .results
                .get(&Id::from_proto(Some(result))?)
                .ok_or(PackageError::Integrity)?;
            ids.insert(AssetId::try_from(
                result.definition.composite_asset_id.clone(),
            )?);
        }
    }
    if ids.len() > 65 {
        return Err(PackageError::Limit);
    }
    let total = ids.iter().try_fold(0u64, |n, id| {
        let a = project.assets.get(id).ok_or(PackageError::Original)?;
        if a.byte_size > MAX_SOURCE as u64 {
            return Err(PackageError::Limit);
        }
        n.checked_add(a.byte_size).ok_or(PackageError::Limit)
    })?;
    // Compiler reserves package/JSON projections itself. This early lower bound
    // precedes every source read; file reads also enforce metadata length.
    if total > MAX_SOURCE as u64
        || total
            .checked_add(2 * MAX_PACKAGE as u64 + 96 * 1024 * 1024)
            .is_none_or(|n| n >= budget)
    {
        return Err(PackageError::Limit);
    }
    let mut out = BTreeMap::new();
    for id in ids {
        if vw_package::Cancellation::is_cancelled(cancel) {
            return Err(PackageError::Cancelled);
        }
        let expected = project
            .assets
            .get(&id)
            .ok_or(PackageError::Original)?
            .byte_size;
        let bytes = files::read_exact(&state.blobs.path(&id)?, expected, MAX_SOURCE)?;
        if AssetId::hash(&bytes) != id {
            return Err(PackageError::Original);
        }
        out.insert(id, bytes);
    }
    Ok(out)
}
pub(super) fn compile(
    state: &ProjectState,
    options: PackageCompileOptions,
    closed: &AtomicBool,
    cancel: &Cancellation,
    permit: Permit,
) -> PackageResult<Arc<CompiledPackage>> {
    check(closed, cancel)?;
    options.binding.validate()?;
    if options.memory_budget_bytes > MAX_MEMORY || options.memory_budget_bytes < 16 * 1024 * 1024 {
        return Err(PackageError::Invalid);
    }
    let package_id = Id::try_from(options.package_id)?;
    let document = Id::try_from(options.binding.document_id.clone())?;
    let semantic = options.semantic_snapshot_id.map(Id::try_from).transpose()?;
    let target = options.target.native()?;
    if options.created_at_ms < 0 || options.created_at_ms > 253_402_300_799_999 {
        return Err(PackageError::Invalid);
    }
    let canonical = workflow::canonical_workspace(state, options.memory_budget_bytes, false)?;
    let budget = options
        .memory_budget_bytes
        .checked_sub(canonical)
        .ok_or(PackageError::Limit)?;
    workflow::check_binding(state, &options.binding)?;
    let stop = Stop { closed, cancel };
    let originals = originals(state, &document, budget, &stop)?;
    let revision = state.view_revision()?;
    let package = vw_package::compile(
        vw_package::CompileInput {
            project: state.project()?,
            revision: &revision,
            document: &document,
            originals: &originals,
        },
        vw_package::CompileOptions {
            package_id,
            created_at_ms: options.created_at_ms,
            target,
            semantic_snapshot: semantic,
            include_window_title: options.include_window_title,
            assume_untagged_srgb: options.assume_untagged_srgb,
            allow_depth_reduction: options.allow_depth_reduction,
            limits: limits(budget),
        },
        &stop,
    )?;
    if dto::describe(&package)?.binding != options.binding {
        return Err(PackageError::Stale);
    }
    check(closed, cancel)?;
    Ok(Arc::new(CompiledPackage {
        prepared: Mutex::new(Some(Arc::new(Prepared {
            package,
            _permit: permit,
        }))),
    }))
}
#[uniffi::export]
impl ProjectSession {
    /// The project worker retains the complete source for this call. There is no
    /// stale snapshot reattachment or mutation of canonical project state.
    pub async fn compile_package(
        &self,
        options: PackageCompileOptions,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<Arc<CompiledPackage>> {
        self.check_open()?;
        options.binding.validate()?;
        options.target.native()?;
        let permit = Permit::acquire()?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| Ok(compile(state, options, &closed, &cancellation, permit)))
            .await?
    }
}
