//! Stored semantic inventory and reference edits. No provider, implicit latest
//! snapshot, second canonical store, or execution of captured text lives here.
use crate::{Cancellation, InstructionCommand, ProjectSession, workflow::*};
use std::sync::Arc;
use vw_model::{Id, Project, SemanticSnapshot};
use vw_proto::v1 as pb;
use vw_semantics::CaptureView;

#[derive(Clone, Debug, uniffi::Record)]
pub struct SemanticCatalogCursor {
    pub binding: WorkflowBinding,
    pub after_snapshot_id: String,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct SemanticSnapshotSummary {
    pub snapshot_id: String,
    pub platform: String,
    pub frame_delta_ms: i32,
    pub created_at_ms: i64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct SemanticCatalog {
    pub binding: WorkflowBinding,
    pub is_capture: bool,
    pub snapshots: Vec<SemanticSnapshotSummary>,
    pub next: Option<SemanticCatalogCursor>,
}
fn id(value: &str) -> WorkflowResult<Id> {
    if value.len() != 36 {
        return Err(WorkflowError::Invalid);
    }
    Ok(Id::try_from(value.to_owned())?)
}
fn refs(values: &[String]) -> WorkflowResult<()> {
    if values.len() > vw_semantics::MAX_REFERENCES
        || values
            .iter()
            .any(|s| s.len() > 300 || s.is_empty() || s.contains('\0'))
    {
        return Err(WorkflowError::Limit);
    }
    // <=64 entries: avoid allocating another unbounded caller collection.
    if values
        .iter()
        .enumerate()
        .any(|(i, s)| values[..i].contains(s))
    {
        return Err(WorkflowError::Invalid);
    }
    Ok(())
}
fn belongs(
    value: &SemanticSnapshot,
    document: &Id,
    view: &CaptureView<'_>,
) -> WorkflowResult<bool> {
    Ok(
        Id::from_proto(value.definition.document_id.as_ref())? == *document
            && value.capture_session_id.as_ref() == Some(view.context().session_id())
            && value.frame_id == Some(view.context().frame_id()),
    )
}
fn catalog(
    project: &Project,
    revision: &pb::Revision,
    binding: WorkflowBinding,
    cursor: Option<SemanticCatalogCursor>,
    limit: u32,
    cancel: &Cancellation,
) -> WorkflowResult<SemanticCatalog> {
    let document = id(&binding.document_id)?;
    let definition = &project
        .documents
        .get(&document)
        .ok_or(WorkflowError::Missing)?
        .definition;
    if definition.kind != pb::DocumentKind::Capture as i32 {
        if cursor.is_some() {
            return Err(WorkflowError::Stale);
        }
        return Ok(SemanticCatalog {
            binding,
            is_capture: false,
            snapshots: vec![],
            next: None,
        });
    }
    let view = CaptureView::new(project, revision, &document)?;
    let after = cursor
        .map(|c| -> WorkflowResult<Id> {
            if c.binding != binding {
                return Err(WorkflowError::Stale);
            }
            let key = id(&c.after_snapshot_id)?;
            let stored = project
                .semantic_snapshots
                .get(&key)
                .ok_or(WorkflowError::Stale)?;
            if !belongs(stored, &document, &view)? {
                return Err(WorkflowError::Stale);
            }
            Ok(key)
        })
        .transpose()?;
    let mut snapshots = Vec::with_capacity(limit as usize);
    let mut more = false;
    for (key, stored) in &project.semantic_snapshots {
        cancel.check()?;
        if after.as_ref().is_some_and(|a| key <= a) || !belongs(stored, &document, &view)? {
            continue;
        }
        if snapshots.len() == limit as usize {
            more = true;
            break;
        }
        if !matches!(
            stored.definition.platform.as_str(),
            "uia" | "android_ax" | "chromium_uia"
        ) {
            return Err(WorkflowError::Invalid);
        }
        snapshots.push(SemanticSnapshotSummary {
            snapshot_id: key.to_string(),
            platform: stored.definition.platform.clone(),
            frame_delta_ms: stored.definition.frame_delta_ms,
            created_at_ms: stored.created_at_ms,
        });
    }
    let next = if more {
        snapshots.last().map(|v| SemanticCatalogCursor {
            binding: binding.clone(),
            after_snapshot_id: v.snapshot_id.clone(),
        })
    } else {
        None
    };
    cancel.check()?;
    Ok(SemanticCatalog {
        binding,
        is_capture: true,
        snapshots,
        next,
    })
}
#[uniffi::export]
impl ProjectSession {
    /// A page is an explicit inventory, never a choice of latest. Selecting a row
    /// still calls semantic_document, which validates the compressed payload and
    /// full source/frame/geometry binding. Cursors fail after any visible edit.
    pub async fn semantic_catalog(
        &self,
        binding: WorkflowBinding,
        cursor: Option<SemanticCatalogCursor>,
        limit: u32,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<SemanticCatalog> {
        self.check_open()?;
        binding.validate()?;
        if !(1..=32).contains(&limit) {
            return Err(WorkflowError::Limit);
        }
        if let Some(c) = &cursor {
            c.binding.validate()?;
            id(&c.after_snapshot_id)?;
        }
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&closed, &cancellation)?;
                    admit(state, memory_budget_bytes, false, SEMANTIC_WORK)?;
                    check_binding(state, &binding)?;
                    catalog(
                        state.project()?,
                        &state.view_revision()?,
                        binding,
                        cursor,
                        limit,
                        &cancellation,
                    )
                })())
            })
            .await?
    }
    /// Canonical marker placement after validating its exact EIDs against the
    /// explicitly selected stored snapshot. The second worker call rechecks the
    /// same complete visible CAS; no intervening edit can inherit this proof.
    pub async fn prepare_semantic_marker(
        self: Arc<Self>,
        binding: WorkflowBinding,
        snapshot_id: String,
        metadata: WorkflowMetadata,
        command: InstructionCommand,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<Arc<WorkflowPlan>> {
        self.check_open()?;
        binding.validate()?;
        metadata.validate()?;
        let snapshot = id(&snapshot_id)?;
        let InstructionCommand::PlaceMarker { element_eids, .. } = &command else {
            return Err(WorkflowError::Invalid);
        };
        refs(element_eids)?;
        let eids = element_eids.clone();
        let expected = binding.clone();
        let closed = self.closed.clone();
        let token = cancellation.clone();
        self.worker
            .call(move |state| {
                Ok((|| -> WorkflowResult<()> {
                    check_live(&closed, &token)?;
                    admit(state, memory_budget_bytes, false, SEMANTIC_WORK)?;
                    check_binding(state, &expected)?;
                    let view = CaptureView::new(
                        state.project()?,
                        &state.view_revision()?,
                        &id(&expected.document_id)?,
                    )?;
                    view.load(&snapshot, token.as_ref())?
                        .export_references(&eids, token.as_ref())?;
                    token.check()?;
                    Ok(())
                })())
            })
            .await??;
        self.prepare_instruction(
            binding,
            metadata,
            command,
            memory_budget_bytes,
            cancellation,
        )
        .await
    }
    /// Replace references only, preserving marker number, geometry, identity,
    /// order, style and its linked instruction. Normal canonical undo applies.
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_semantic_references(
        self: Arc<Self>,
        binding: WorkflowBinding,
        snapshot_id: String,
        object_id: String,
        element_eids: Vec<String>,
        metadata: WorkflowMetadata,
        memory_budget_bytes: u64,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<Arc<WorkflowPlan>> {
        self.check_open()?;
        binding.validate()?;
        metadata.validate()?;
        refs(&element_eids)?;
        let object = id(&object_id)?;
        let snapshot = id(&snapshot_id)?;
        let permit = HandlePermit::acquire()?;
        let project = self.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    check_live(&project.closed, &cancellation)?;
                    admit(state, memory_budget_bytes, true, SEMANTIC_WORK)?;
                    check_binding(state, &binding)?;
                    check_metadata(state, &metadata)?;
                    let document = id(&binding.document_id)?;
                    let canonical = state.project()?;
                    let revision = state.view_revision()?;
                    let capture = CaptureView::new(canonical, &revision, &document)?;
                    capture
                        .load(&snapshot, cancellation.as_ref())?
                        .export_references(&element_eids, cancellation.as_ref())?;
                    let view = vw_instructions::DocumentView::new(canonical, &revision, &document)?;
                    view.marker_instruction(&object)?;
                    let saved = &view.object(&object)?.state;
                    let layer = canonical
                        .layers
                        .get(&Id::from_proto(saved.layer_id.as_ref())?)
                        .ok_or(WorkflowError::Missing)?;
                    if saved.locked || layer.locked {
                        return Err(WorkflowError::Locked);
                    }
                    let Some(pb::object_state::Shape::Marker(marker)) = &saved.shape else {
                        return Err(WorkflowError::Invalid);
                    };
                    let mut bytes = element_eids.iter().map(String::len).sum::<usize>();
                    for key in view.marker_ids() {
                        cancellation.check()?;
                        if key != &object
                            && let Some(pb::object_state::Shape::Marker(other)) =
                                &view.object(key)?.state.shape
                        {
                            for eid in &other.element_eids {
                                bytes = bytes.checked_add(eid.len()).ok_or(WorkflowError::Limit)?;
                            }
                        }
                    }
                    if bytes > vw_instructions::MAX_TOTAL_TEXT_BYTES {
                        return Err(WorkflowError::Limit);
                    }
                    let mut changed = marker.clone();
                    changed.element_eids = element_eids;
                    let raw = serde_json::to_vec(&pb::object_state::Shape::Marker(changed))
                        .map_err(|_| WorkflowError::Invalid)?;
                    let next = metadata
                        .first_lamport
                        .checked_add(1)
                        .ok_or(WorkflowError::Limit)?;
                    let transaction = pb::Transaction {
                        txn_id: Some(id(&metadata.transaction_id)?.to_proto()),
                        project_id: Some(canonical.id.to_proto()),
                        device_id: metadata.device_id.clone(),
                        base_revision: Some(revision),
                        created_at_wall_ms: metadata.created_at_ms,
                        gesture_id: None,
                        ops: vec![pb::Op {
                            op_id: Some(pb::OpId {
                                device_id: metadata.device_id,
                                lamport: metadata.first_lamport,
                            }),
                            kind: Some(pb::op::Kind::SetProperty(pb::SetProperty {
                                object_id: Some(object.to_proto()),
                                property: "shape".into(),
                                value: Some(pb::PropertyValue {
                                    value: Some(pb::property_value::Value::Raw(raw)),
                                }),
                            })),
                        }],
                    };
                    admit_transaction(state, &transaction, memory_budget_bytes, SEMANTIC_WORK)?;
                    check_live(&project.closed, &cancellation)?;
                    WorkflowPlan::new(
                        &project,
                        state,
                        binding,
                        transaction,
                        next,
                        memory_budget_bytes,
                        SEMANTIC_WORK,
                        permit,
                    )
                })())
            })
            .await?
    }
}
