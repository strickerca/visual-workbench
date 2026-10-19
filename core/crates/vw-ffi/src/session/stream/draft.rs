//! Project-owned, unsaved annotations over the exact acknowledged stream frame.
//! The paired project and authenticated link stay alive; Save appends one normal
//! transaction only after the original is verified. Preview bytes never qualify.
mod calls;
mod pixels;
mod save;
#[cfg(test)]
mod tests;

use super::{
    api::{CaptureBackground, CaptureStream, StreamFrameInfo},
    hub::Lease,
};
use crate::{Cancellation, ProjectSession, project::SessionPermit, workflow::*};
pub use save::{CaptureDraftSave, CaptureDraftSaveInfo};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Mutex;
use vw_model::{DeviceId, Document, Id, Layer, Project};
use vw_proto::v1 as pb;

const CREATE_WORK: u64 = 96 * 1024 * 1024;
const PLAN_WORK: u64 = 64 * 1024 * 1024;

#[derive(Clone, uniffi::Record)]
pub struct CaptureDraftOptions {
    pub document_id: String,
    pub layer_id: String,
    pub memory_budget_bytes: u64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct CaptureDraftInfo {
    pub draft_id: String,
    pub document_id: String,
    pub frame: StreamFrameInfo,
    pub preview_asset_id: String,
    pub connection_epoch: u64,
}

struct Life {
    disposed: AtomicBool,
    saved: AtomicBool,
    sealed: Arc<AtomicBool>,
    project: Arc<ProjectSession>,
    target: Arc<ProjectSession>,
    lease: Arc<Lease>,
    info: CaptureDraftInfo,
    capture: pb::CaptureInfo,
    calls: Arc<calls::Calls>,
}
impl Life {
    fn check(&self, cancel: &Cancellation) -> WorkflowResult<()> {
        cancel.check()?;
        if self.disposed.load(Ordering::Acquire)
            || self.project.closed.load(Ordering::Acquire)
            || self.target.closed.load(Ordering::Acquire)
        {
            return Err(WorkflowError::Closed);
        }
        self.lease.check().map_err(map_session)
    }
}

/// Native lifetime retains the admission seal through queued/running work.
/// Dropping a waiting future alone cannot re-enable editing beneath a snapshot.
struct Seal {
    life: Arc<Life>,
}
impl Seal {
    fn take(life: Arc<Life>) -> WorkflowResult<Arc<Self>> {
        life.sealed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| WorkflowError::Backpressure)?;
        Ok(Arc::new(Self { life }))
    }
}
impl Drop for Seal {
    fn drop(&mut self) {
        if !self.life.saved.load(Ordering::Acquire) && !self.life.disposed.load(Ordering::Acquire) {
            self.life.sealed.store(false, Ordering::Release);
        }
    }
}

#[derive(uniffi::Object)]
pub struct CaptureDraft {
    life: Arc<Life>,
    closing: Mutex<()>,
}
#[uniffi::export]
impl CaptureDraft {
    pub fn info(&self) -> CaptureDraftInfo {
        self.life.info.clone()
    }
    /// Borrowed project facade uses all ordinary stroke/shape/undo APIs. The app
    /// must keep its parent draft owner and close it, never open another link.
    pub fn project(&self) -> WorkflowResult<Arc<ProjectSession>> {
        if self.life.disposed.load(Ordering::Acquire) {
            return Err(WorkflowError::Closed);
        }
        Ok(self.life.project.clone())
    }
    /// Preparation seals mutations before the worker snapshot. Dispose an
    /// uncommitted plan to resume editing; after Save only joined close applies.
    pub async fn prepare_save(
        &self,
        source: WorkflowBinding,
        target: WorkflowBinding,
        metadata: WorkflowMetadata,
        original: Arc<CaptureBackground>,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<Arc<CaptureDraftSave>> {
        save::prepare(
            self.life.clone(),
            source,
            target,
            metadata,
            original,
            false,
            memory_budget_bytes,
            cancellation,
        )
        .await
    }
    /// Explicit degraded choice after the exact-frame timeout. The saved PNG
    /// represents decoded preview pixels, never a verified original. Capture
    /// metadata and the returned receipt permanently retain that distinction.
    pub async fn prepare_degraded_save(
        &self,
        source: WorkflowBinding,
        target: WorkflowBinding,
        metadata: WorkflowMetadata,
        preview: Arc<CaptureBackground>,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<Arc<CaptureDraftSave>> {
        save::prepare(
            self.life.clone(),
            source,
            target,
            metadata,
            preview,
            true,
            memory_budget_bytes,
            cancellation,
        )
        .await
    }
    /// Seal first, then join the actual project actor and its filesystem owners.
    /// Repeated close joins the same worker destruction. No live-link close or
    /// automatic Resume is hidden here; the app resumes only after this returns.
    pub async fn close_draft(&self) -> WorkflowResult<()> {
        self.life.disposed.store(true, Ordering::Release);
        self.life.sealed.store(true, Ordering::Release);
        self.life.calls.seal();
        let _close = self.closing.lock().await;
        self.life.calls.join().await?;
        self.life.project.close().await.map_err(Into::into)
    }
}
impl Drop for CaptureDraft {
    fn drop(&mut self) {
        self.life.disposed.store(true, Ordering::Release);
        self.life.sealed.store(true, Ordering::Release);
        self.life.calls.seal();
        // Final ProjectSession/Worker ownership closes SQLite before TempDir.
        // Explicit close is required before the application reports Resume.
    }
}

pub(in crate::session) async fn create(
    target: Arc<ProjectSession>,
    stream: Arc<CaptureStream>,
    options: CaptureDraftOptions,
    cancellation: Arc<Cancellation>,
) -> WorkflowResult<Arc<CaptureDraft>> {
    cancellation.check()?;
    target.check_open()?;
    stream.lease.check().map_err(map_session)?;
    let background = stream
        .frozen_background()
        .await
        .map_err(map_session)?
        .ok_or(WorkflowError::Missing)?;
    let draft_id = Id::try_from(background.draft_id())?;
    let document = Id::try_from(options.document_id)?;
    let layer = Id::try_from(options.layer_id)?;
    if draft_id == document || draft_id == layer || document == layer {
        return Err(WorkflowError::Invalid);
    }
    // Reserve before PNG/profile allocation; source owners already retain their
    // receiver reservation and cannot be credited until their actual destruction.
    let budget =
        super::memory::ByteBudget::new(options.memory_budget_bytes).map_err(map_session)?;
    if options.memory_budget_bytes > MAX_MEMORY {
        return Err(WorkflowError::Invalid);
    }
    let working = budget.reserve(CREATE_WORK).map_err(map_session)?;
    let token = cancellation.clone();
    let owner = target.clone();
    let (temporary, device) = target
        .worker
        .call(move |state| {
            Ok((|| {
                check_live(&owner.closed, &token)?;
                admit(state, options.memory_budget_bytes, false, CREATE_WORK)?;
                let probe = state.store()?.transfer_file()?;
                let parent = probe.path().parent().ok_or(WorkflowError::Storage)?;
                let temporary = tempfile::Builder::new()
                    .prefix("stream-draft-")
                    .tempdir_in(parent)
                    .map_err(|_| WorkflowError::Storage)?;
                Ok::<_, WorkflowError>((temporary, state.store()?.local_device()?))
            })())
        })
        .await??;
    let retained_target = target.clone();
    let lease = stream.lease.clone();
    crate::worker::startup(move || {
        Ok((|| {
            let working = working;
            cancellation.check()?;
            lease.check().map_err(map_session)?;
            check_live(&retained_target.closed, &cancellation)?;
            let permit = SessionPermit::acquire()?;
            let frame = background.info();
            let png = pixels::preview_png(&background, &cancellation)?;
            let asset = pixels::asset(&png, &frame, &cancellation)?;
            let capture = background.capture_info(false)?;
            let mut project = Project::new(
                draft_id.clone(),
                "Unsaved stream capture".into(),
                device.clone(),
            );
            project.assets.insert(
                vw_model::AssetId::try_from(asset.asset_id.clone())?,
                asset.clone(),
            );
            project.documents.insert(
                document.clone(),
                Document {
                    definition: pb::CreateDocument {
                        document_id: Some(document.to_proto()),
                        kind: pb::DocumentKind::Capture as i32,
                        schema_version: 1,
                        title: "Frozen stream frame".into(),
                        primary_asset_id: asset.asset_id.clone(),
                        capture: Some(capture.clone()),
                    },
                    pages: vec![],
                    created_at_ms: frame.captured_at_ms,
                },
            );
            project.layers.insert(
                layer.clone(),
                Layer {
                    definition: pb::CreateLayer {
                        layer_id: Some(layer.to_proto()),
                        document_id: Some(document.to_proto()),
                        page_index: -1,
                        name: "Annotations".into(),
                        kind: "annotation".into(),
                        order_key: "V".into(),
                    },
                    visible: true,
                    locked: false,
                    opacity: 1.0,
                    blend: "normal".into(),
                },
            );
            project.validate()?;
            let path = temporary.path().join("project");
            let (store, blobs) = crate::creation::create_complete(
                &path,
                crate::creation::InitialProject {
                    project,
                    device,
                    time: frame.captured_at_ms,
                },
                &png,
                &vw_model::AssetId::try_from(asset.asset_id.clone())?,
                &cancellation,
                |_| Ok(()),
            )?;
            cancellation.check()?;
            lease.check().map_err(map_session)?;
            let sealed = Arc::new(AtomicBool::new(false));
            let draft = ProjectSession::from_temporary_store(
                store,
                blobs,
                permit,
                temporary,
                sealed.clone(),
                working.into_charge(),
            )?;
            Ok(Arc::new(CaptureDraft {
                life: Arc::new(Life {
                    disposed: AtomicBool::new(false),
                    saved: AtomicBool::new(false),
                    sealed,
                    project: draft,
                    target: retained_target,
                    lease: lease.clone(),
                    info: CaptureDraftInfo {
                        draft_id: draft_id.to_string(),
                        document_id: document.to_string(),
                        frame,
                        preview_asset_id: asset.asset_id,
                        connection_epoch: lease.endpoint.epoch,
                    },
                    capture,
                    calls: Arc::new(calls::Calls::default()),
                }),
                closing: Mutex::new(()),
            }))
        })())
    })
    .await?
}

fn map_session(error: crate::session::SessionError) -> WorkflowError {
    match error {
        crate::session::SessionError::Closed => WorkflowError::Closed,
        crate::session::SessionError::Cancelled => WorkflowError::Cancelled,
        crate::session::SessionError::Storage => WorkflowError::Storage,
        crate::session::SessionError::Backpressure => WorkflowError::Backpressure,
        _ => WorkflowError::Invalid,
    }
}
