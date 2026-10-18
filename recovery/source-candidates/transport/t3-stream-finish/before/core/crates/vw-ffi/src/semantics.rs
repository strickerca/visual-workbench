//! Explicit capture-bound collection and semantic queries. Platform collection
//! itself, owner opt-in and foreground/own-window checks are provider-owned.
mod dto;
use crate::{Cancellation, ProjectSession, QueryRect, Transform, workflow::*};
pub use dto::*;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};
use vw_model::Id;
use vw_semantics::{CaptureContext, CaptureView, Snapshot};

fn id(value: &str) -> WorkflowResult<Id> {
    if value.len() != 36 {
        return Err(WorkflowError::Invalid);
    }
    Ok(Id::try_from(value.to_owned())?)
}
fn view<'a>(
    state: &'a crate::project::ProjectState,
    binding: &WorkflowBinding,
) -> WorkflowResult<CaptureView<'a>> {
    check_binding(state, binding)?;
    Ok(CaptureView::new(
        state.project()?,
        &state.view_revision()?,
        &id(&binding.document_id)?,
    )?)
}
fn rect(v: [f64; 4]) -> QueryRect {
    QueryRect {
        x: v[0],
        y: v[1],
        width: v[2],
        height: v[3],
    }
}
fn element(value: &vw_semantics::Element) -> SemanticElement {
    SemanticElement {
        eid: value.eid.clone(),
        parent: value.parent.clone(),
        name: value.name.clone(),
        role: value.role.clone(),
        automation_id: value.automation_id.clone(),
        resource_id: value.resource_id.clone(),
        html_id: value.html_id.clone(),
        bounds_document: rect(value.bounds_document),
        bounds_clipped: value.bounds_clipped,
        text: value.text.clone(),
        enabled: value.enabled,
        focused: value.focused,
    }
}
fn admit_elements(values: &[CapturedSemanticElement]) -> WorkflowResult<()> {
    if values.len() > vw_semantics::MAX_ELEMENTS {
        return Err(WorkflowError::Limit);
    }
    let mut bytes = 0usize;
    for value in values {
        for (s, limit) in [
            (value.local_id.as_str(), 256),
            (value.name.as_str(), 4096),
            (value.role.as_str(), 256),
            (value.text.as_str(), 800),
        ] {
            if s.len() > limit || s.contains('\0') {
                return Err(WorkflowError::Limit);
            }
            bytes = bytes.checked_add(s.len()).ok_or(WorkflowError::Limit)?;
        }
        for (s, limit) in [
            (value.parent_local_id.as_deref(), 256),
            (value.automation_id.as_deref(), 1024),
            (value.resource_id.as_deref(), 1024),
            (value.html_id.as_deref(), 1024),
        ] {
            if let Some(s) = s {
                if s.len() > limit || s.contains('\0') {
                    return Err(WorkflowError::Limit);
                }
                bytes = bytes.checked_add(s.len()).ok_or(WorkflowError::Limit)?;
            }
        }
        if bytes > vw_semantics::MAX_TEXT_BYTES {
            return Err(WorkflowError::Limit);
        }
    }
    Ok(())
}
fn core_element(value: CapturedSemanticElement) -> vw_semantics::CapturedElement {
    vw_semantics::CapturedElement {
        local_id: value.local_id,
        parent_local_id: value.parent_local_id,
        name: value.name,
        role: value.role,
        automation_id: value.automation_id,
        resource_id: value.resource_id,
        html_id: value.html_id,
        bounds: [
            value.bounds.x,
            value.bounds.y,
            value.bounds.width,
            value.bounds.height,
        ],
        text: value.text,
        enabled: value.enabled,
        focused: value.focused,
    }
}
struct CapturedContext {
    context: CaptureContext,
    info: SemanticCaptureInfo,
    _permit: HandlePermit,
}
#[derive(uniffi::Object)]
pub struct SemanticCollection {
    project: Weak<ProjectSession>,
    project_closed: Arc<AtomicBool>,
    disposed: Arc<AtomicBool>,
    context: Mutex<Option<Arc<CapturedContext>>>,
    memory_budget_bytes: u64,
}
impl SemanticCollection {
    fn context(&self) -> WorkflowResult<Arc<CapturedContext>> {
        if self.disposed.load(Ordering::Acquire) || self.project_closed.load(Ordering::Acquire) {
            return Err(WorkflowError::Closed);
        }
        self.context
            .lock()
            .map_err(|_| WorkflowError::Closed)?
            .as_ref()
            .cloned()
            .ok_or(WorkflowError::Closed)
    }
}
#[uniffi::export]
impl SemanticCollection {
    pub fn describe(&self) -> WorkflowResult<SemanticCaptureInfo> {
        Ok(self.context()?.info.clone())
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare(
        &self,
        metadata: WorkflowMetadata,
        snapshot_id: String,
        platform: SemanticPlatform,
        frame_delta_ms: i32,
        collection_elapsed_ms: u64,
        bounds_space: SemanticBoundsSpace,
        elements: Vec<CapturedSemanticElement>,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<Arc<WorkflowPlan>> {
        metadata.validate()?;
        id(&snapshot_id)?;
        admit_elements(&elements)?;
        let observed = self.context()?;
        let project = self.project.upgrade().ok_or(WorkflowError::Closed)?;
        let disposed = self.disposed.clone();
        let worker = project.worker.clone();
        let budget = self.memory_budget_bytes;
        let permit = HandlePermit::acquire()?;
        worker
            .call(move |state| {
                Ok((|| {
                    check_live(&project.closed, &cancellation)?;
                    if disposed.load(Ordering::Acquire) {
                        return Err(WorkflowError::Closed);
                    }
                    admit(state, budget, true, SEMANTIC_WORK)?;
                    check_metadata(state, &metadata)?;
                    let view = view(state, &observed.info.binding)?;
                    let snapshot = view.prepare(
                        &observed.context,
                        id(&snapshot_id)?,
                        platform.core(),
                        frame_delta_ms,
                        collection_elapsed_ms,
                        match bounds_space {
                            SemanticBoundsSpace::HostPhysical => {
                                vw_semantics::BoundsSpace::HostPhysical
                            }
                            SemanticBoundsSpace::CapturePixels => {
                                vw_semantics::BoundsSpace::CapturePixels
                            }
                        },
                        elements.into_iter().map(core_element).collect(),
                        cancellation.as_ref(),
                    )?;
                    let plan = view.plan(
                        &snapshot,
                        vw_semantics::EditMetadata {
                            transaction_id: id(&metadata.transaction_id)?,
                            device: vw_model::DeviceId::try_from(metadata.device_id)?,
                            lamport: metadata.first_lamport,
                            created_at_ms: metadata.created_at_ms,
                        },
                        cancellation.as_ref(),
                    )?;
                    admit_transaction(state, plan.transaction(), budget, SEMANTIC_WORK)?;
                    cancellation.check()?;
                    if disposed.load(Ordering::Acquire) {
                        return Err(WorkflowError::Closed);
                    }
                    WorkflowPlan::new(
                        &project,
                        state,
                        observed.info.binding.clone(),
                        plan.transaction().clone(),
                        plan.next_lamport(),
                        budget,
                        SEMANTIC_WORK,
                        permit,
                    )
                })())
            })
            .await?
    }
    pub fn dispose(&self) {
        self.disposed.store(true, Ordering::Release);
        if let Ok(mut context) = self.context.lock() {
            *context = None;
        }
    }
}
impl Drop for SemanticCollection {
    fn drop(&mut self) {
        self.dispose();
    }
}
#[uniffi::export]
impl ProjectSession {
    /// Begin before collecting the platform tree, after the exact screenshot is
    /// persisted as a canonical capture document. No "latest capture" fallback.
    pub async fn begin_semantic_capture(
        self: Arc<Self>,
        binding: WorkflowBinding,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<Arc<SemanticCollection>> {
        self.check_open()?;
        binding.validate()?;
        let permit = HandlePermit::acquire()?;
        let project = self.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&project.closed, &cancellation)?;
                    admit(state, memory_budget_bytes, false, SEMANTIC_WORK)?;
                    let view = view(state, &binding)?;
                    let context = view.context().clone();
                    let [width, height] = context.extent()?;
                    let info = SemanticCaptureInfo {
                        binding,
                        capture_session_id: context.session_id().to_string(),
                        frame_id: context.frame_id(),
                        geometry_revision: context.geometry_revision(),
                        source_asset_id: context.primary_asset_id().to_owned(),
                        captured_at_ms: context.captured_at_ms(),
                        width,
                        height,
                    };
                    cancellation.check()?;
                    Ok(Arc::new(SemanticCollection {
                        project: Arc::downgrade(&project),
                        project_closed: project.closed.clone(),
                        disposed: Arc::new(AtomicBool::new(false)),
                        context: Mutex::new(Some(Arc::new(CapturedContext {
                            context,
                            info,
                            _permit: permit,
                        }))),
                        memory_budget_bytes,
                    }))
                })())
            })
            .await?
    }
    pub async fn semantic_document(
        &self,
        binding: WorkflowBinding,
        snapshot_id: String,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<SemanticDocument> {
        self.check_open()?;
        binding.validate()?;
        id(&snapshot_id)?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&closed, &cancellation)?;
                    admit(state, memory_budget_bytes, false, SEMANTIC_WORK)?;
                    let snapshot =
                        view(state, &binding)?.load(&id(&snapshot_id)?, cancellation.as_ref())?;
                    let elements = snapshot.elements().iter().map(element).collect();
                    cancellation.check()?;
                    Ok(SemanticDocument {
                        binding,
                        snapshot_id,
                        platform: SemanticPlatform::from_core(snapshot.platform()),
                        frame_delta_ms: snapshot.frame_delta_ms(),
                        collection_elapsed_ms: snapshot.collection_elapsed_ms(),
                        elements,
                    })
                })())
            })
            .await?
    }
    pub async fn snap_semantic(
        &self,
        binding: WorkflowBinding,
        snapshot_id: String,
        query: SemanticSnapQuery,
        document_to_screen: Transform,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<Option<SemanticSnap>> {
        self.check_open()?;
        binding.validate()?;
        id(&snapshot_id)?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&closed, &cancellation)?;
                    admit(state, memory_budget_bytes, false, SEMANTIC_WORK)?;
                    let snapshot =
                        view(state, &binding)?.load(&id(&snapshot_id)?, cancellation.as_ref())?;
                    let m = document_to_screen;
                    let transform = vw_geom::Affine::new(m.a, m.b, m.c, m.d, m.e, m.f)?;
                    let query = match query {
                        SemanticSnapQuery::Point { point } => {
                            vw_semantics::SnapQuery::Point(vw_geom::Point::new(point.x, point.y)?)
                        }
                        SemanticSnapQuery::Box { bounds } => vw_semantics::SnapQuery::Box([
                            bounds.x,
                            bounds.y,
                            bounds.width,
                            bounds.height,
                        ]),
                    };
                    let snap = snapshot.snap(query, transform, cancellation.as_ref())?;
                    cancellation.check()?;
                    Ok(snap.map(|v| SemanticSnap {
                        snapshot_id,
                        binding,
                        eid: v.element.eid.clone(),
                        bounds_document: rect(v.bounds_document),
                        distance_screen_pixels: v.distance_screen_pixels,
                    }))
                })())
            })
            .await?
    }
    pub async fn export_semantics(
        &self,
        binding: WorkflowBinding,
        snapshot_id: String,
        element_eids: Option<Vec<String>>,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<SemanticProjection> {
        self.check_open()?;
        binding.validate()?;
        id(&snapshot_id)?;
        if element_eids.as_ref().is_some_and(|eids| {
            eids.len() > vw_semantics::MAX_REFERENCES || eids.iter().any(|e| e.len() > 300)
        }) {
            return Err(WorkflowError::Limit);
        }
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&closed, &cancellation)?;
                    admit(state, memory_budget_bytes, false, SEMANTIC_WORK)?;
                    let snapshot: Snapshot =
                        view(state, &binding)?.load(&id(&snapshot_id)?, cancellation.as_ref())?;
                    let export = match element_eids {
                        Some(eids) => snapshot.export_references(&eids, cancellation.as_ref())?,
                        None => snapshot.export_all(cancellation.as_ref())?,
                    };
                    let references = export
                        .references()
                        .iter()
                        .map(|v| SemanticReference {
                            platform: SemanticPlatform::from_core(v.platform),
                            eid: v.eid.clone(),
                            name: v.name.clone(),
                            role: v.role.clone(),
                            automation_id: v.automation_id.clone(),
                            resource_id: v.resource_id.clone(),
                            html_id: v.html_id.clone(),
                            bounds_document: rect(v.bounds_document),
                        })
                        .collect();
                    cancellation.check()?;
                    Ok(SemanticProjection {
                        binding,
                        snapshot_id,
                        references,
                        semantic_json: export.semantic_json().to_vec(),
                        quoted_prompt_data: export.prompt_data().to_owned(),
                    })
                })())
            })
            .await?
    }
}
