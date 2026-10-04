//! Atomic vector erasure materialized as ordinary filled Polygon objects.
use crate::{
    Cancellation, Point, ProjectInfo, ProjectSession,
    project::ProjectState,
    workflow::{self, WorkflowBinding, WorkflowError, WorkflowMetadata, WorkflowResult},
};
use std::{
    collections::BTreeSet,
    sync::{Arc, atomic::AtomicBool},
};
use vw_ink::eraser::{self, EraseError, EraseLimits};
use vw_model::{DeviceId, Id};
use vw_proto::v1 as pb;

#[derive(Clone, Debug, uniffi::Record)]
pub struct VectorEraseTarget {
    pub object_id: String,
    pub replacement_id: String,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct VectorEraseOptions {
    pub binding: WorkflowBinding,
    pub metadata: WorkflowMetadata,
    pub targets: Vec<VectorEraseTarget>,
    pub centers: Vec<Point>,
    pub radius: f64,
    pub memory_budget_bytes: u64,
    pub max_output_vertices: u32,
    pub max_work_units: u64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct VectorEraseReplacement {
    pub original_id: String,
    pub outline_id: Option<String>,
    pub contour_count: u32,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct VectorEraseReceipt {
    pub transaction_id: String,
    pub changed: Vec<VectorEraseReplacement>,
    pub revision: ProjectInfo,
    pub estimated_peak_bytes: u64,
}
fn ink_error(value: EraseError) -> WorkflowError {
    match value {
        EraseError::Limit | EraseError::Allocation => WorkflowError::Limit,
        EraseError::Cancelled => WorkflowError::Cancelled,
        EraseError::Invalid => WorkflowError::Invalid,
    }
}
fn memory(estimated: u64, budget: u64) -> WorkflowResult<()> {
    if estimated > budget {
        Err(WorkflowError::Memory { estimated, budget })
    } else {
        Ok(())
    }
}
fn add(a: u64, b: u64) -> WorkflowResult<u64> {
    a.checked_add(b).ok_or(WorkflowError::Limit)
}
fn multiply(a: u64, b: u64) -> WorkflowResult<u64> {
    a.checked_mul(b).ok_or(WorkflowError::Limit)
}
fn options(value: &VectorEraseOptions) -> WorkflowResult<EraseLimits> {
    value.binding.validate()?;
    value.metadata.validate()?;
    if value.memory_budget_bytes == 0
        || value.memory_budget_bytes > workflow::MAX_MEMORY
        || value.targets.is_empty()
        || value.targets.len() > 32
        || value.centers.is_empty()
        || value.centers.len() > eraser::MAX_ERASER_POINTS
        || !value.radius.is_finite()
        || !(0.5..=eraser::MAX_ERASER_RADIUS).contains(&value.radius)
        || value.centers.iter().any(|p| {
            !p.x.is_finite()
                || !p.y.is_finite()
                || p.x.abs() > vw_ink::MAX_COORDINATE
                || p.y.abs() > vw_ink::MAX_COORDINATE
        })
    {
        return Err(WorkflowError::Invalid);
    }
    let limits = EraseLimits {
        max_vertices: value.max_output_vertices as usize,
        max_work: value.max_work_units,
    };
    limits.workspace_bytes().map_err(ink_error)?;
    let mut identifiers = BTreeSet::new();
    for target in &value.targets {
        for name in [&target.object_id, &target.replacement_id] {
            if name.len() != 36 || !identifiers.insert(name) {
                return Err(WorkflowError::Invalid);
            }
            Id::try_from(name.clone())?;
        }
    }
    Ok(limits)
}
#[uniffi::export]
impl ProjectSession {
    /// Replace affected raw strokes with compound vector outlines. Disconnected
    /// pieces share one NONZERO fill so highlighter opacity never compounds.
    /// Original raw strokes remain in undo history. Repeat erasing accepts the
    /// same canonical outline representation, not arbitrary user polygons.
    pub async fn erase_vector_strokes(
        &self,
        request: VectorEraseOptions,
        cancellation: Arc<Cancellation>,
    ) -> WorkflowResult<VectorEraseReceipt> {
        self.check_open()?;
        cancellation.check()?;
        let limits = options(&request)?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    workflow::check_live(&closed, &cancellation)?;
                    apply(state, request, limits, &closed, &cancellation)
                })())
            })
            .await?
    }
}
fn apply(
    state: &mut ProjectState,
    request: VectorEraseOptions,
    limits: EraseLimits,
    closed: &AtomicBool,
    cancel: &Cancellation,
) -> WorkflowResult<VectorEraseReceipt> {
    // Admission precedes info/hash and all speculative clones. Fresh outline
    // transaction staging receives its own additional admission below.
    let canonical = workflow::canonical_workspace(state, request.memory_budget_bytes, true)?;
    let mut construction = add(
        limits.workspace_bytes().map_err(ink_error)?,
        request.centers.len() as u64 * 64,
    )?;
    memory(add(canonical, construction)?, request.memory_budget_bytes)?;
    workflow::check_binding(state, &request.binding)?;
    workflow::check_metadata(state, &request.metadata)?;
    let document = Id::try_from(request.binding.document_id.clone())?;
    let device = DeviceId::try_from(request.metadata.device_id.clone())?;
    let project = state.project()?;
    let mut total_source_vertices = 0usize;
    for target in &request.targets {
        let id = Id::try_from(target.object_id.clone())?;
        let replacement = Id::try_from(target.replacement_id.clone())?;
        let object = project.objects.get(&id).ok_or(WorkflowError::Missing)?;
        if object.document_id != document {
            return Err(WorkflowError::Invalid);
        }
        let layer = project
            .layers
            .get(&Id::from_proto(object.state.layer_id.as_ref())?)
            .ok_or(WorkflowError::Missing)?;
        if object.state.locked || object.state.hidden || layer.locked || !layer.visible {
            return Err(WorkflowError::Locked);
        }
        if project.objects.contains_key(&replacement)
            || project.groups.contains_key(&replacement)
            || project.mask_versions.contains_key(&replacement)
        {
            return Err(WorkflowError::Invalid);
        }
        // A replacement has a fresh ID. Equal order keys are sorted by ID, so
        // refuse ambiguous ties instead of silently moving a translucent object.
        if project.objects.iter().any(|(other, peer)| {
            other != &id
                && peer.state.layer_id == object.state.layer_id
                && peer.state.order_key == object.state.order_key
        }) {
            return Err(WorkflowError::Invalid);
        }
        let (vertices, raw_bytes) = source_bound(&object.state)?;
        total_source_vertices = total_source_vertices
            .checked_add(vertices)
            .ok_or(WorkflowError::Limit)?;
        if total_source_vertices > limits.max_vertices {
            return Err(WorkflowError::Limit);
        }
        construction = add(construction, raw_bytes)?;
    }
    let initial_peak = add(canonical, construction)?;
    memory(initial_peak, request.memory_budget_bytes)?;
    cancel.check()?;
    let centers = request
        .centers
        .iter()
        .map(|p| pb::PointD { x: p.x, y: p.y })
        .collect::<Vec<_>>();
    let mut kinds = Vec::new();
    let mut changed = Vec::new();
    let mut used_vertices = 0usize;
    let mut work_left = limits.max_work;
    for target in &request.targets {
        cancel.check()?;
        let old_id = Id::try_from(target.object_id.clone())?;
        let object = project.objects.get(&old_id).ok_or(WorkflowError::Missing)?;
        let source = &object.state;
        let affine = source.transform.as_ref().ok_or(WorkflowError::Invalid)?;
        let inverse =
            vw_geom::Affine::new(affine.a, affine.b, affine.c, affine.d, affine.e, affine.f)?
                .inverse()?
                .coefficients();
        // Covers input hull/replay and final bridge validation, separate from
        // the measured half-plane clipping work. Global across all targets.
        let construction_work = multiply(source_bound(source)?.0 as u64, 256)?;
        work_left = work_left
            .checked_sub(construction_work)
            .filter(|n| *n > 0)
            .ok_or(WorkflowError::Limit)?;
        let per_object = EraseLimits {
            max_vertices: limits.max_vertices,
            max_work: work_left,
        };
        let source_contours = match source.shape.as_ref().ok_or(WorkflowError::Invalid)? {
            pb::object_state::Shape::Stroke(stroke) => vw_ink::geometry_from_stroke(stroke)
                .map_err(|_| WorkflowError::Invalid)?
                .polygons()
                .to_vec(),
            pb::object_state::Shape::Polygon(outline) => {
                eraser::contours_from_compound(outline, per_object, &|| cancel.check().is_err())
                    .map_err(ink_error)?
            }
            _ => return Err(WorkflowError::Invalid),
        };
        let result = eraser::erase_contours(
            &source_contours,
            &centers,
            request.radius,
            inverse,
            per_object,
            &|| cancel.check().is_err(),
        )
        .map_err(ink_error)?;
        work_left = work_left
            .checked_sub(result.work_units)
            .filter(|n| *n > 0)
            .ok_or(WorkflowError::Limit)?;
        if !result.changed {
            continue;
        }
        kinds.push(pb::op::Kind::DeleteObject(pb::DeleteObject {
            object_id: Some(old_id.to_proto()),
        }));
        let count = u32::try_from(result.contours.len()).map_err(|_| WorkflowError::Limit)?;
        let replacement = if result.contours.is_empty() {
            None
        } else {
            let outline = eraser::compound_polygon(
                &result.contours,
                EraseLimits {
                    max_vertices: limits.max_vertices,
                    max_work: work_left,
                },
                &|| cancel.check().is_err(),
            )
            .map_err(ink_error)?;
            used_vertices = used_vertices
                .checked_add(outline.points.len())
                .ok_or(WorkflowError::Limit)?;
            if used_vertices > limits.max_vertices {
                return Err(WorkflowError::Limit);
            }
            work_left = work_left
                .checked_sub(multiply(outline.points.len() as u64, 128)?)
                .filter(|n| *n > 0)
                .ok_or(WorkflowError::Limit)?;
            let id = Id::try_from(target.replacement_id.clone())?;
            let color = match source.shape.as_ref() {
                Some(pb::object_state::Shape::Stroke(_)) => {
                    source.style.as_ref().and_then(|s| s.stroke)
                }
                _ => source.style.as_ref().and_then(|s| s.fill),
            }
            .unwrap_or(pb::Color { rgba: 0 });
            let replacement = pb::ObjectState {
                object_id: Some(id.to_proto()),
                layer_id: source.layer_id.clone(),
                order_key: source.order_key.clone(),
                transform: source.transform,
                style: Some(pb::Style {
                    stroke: Some(color),
                    width: 0.0,
                    screen_constant_width: false,
                    fill: Some(color),
                    has_fill: true,
                }),
                role: source.role,
                group_id: source.group_id.clone(),
                locked: false,
                hidden: false,
                created_by: device.to_string(),
                created_at_ms: request.metadata.created_at_ms,
                shape: Some(pb::object_state::Shape::Polygon(outline)),
            };
            vw_model::validate_object_state(&replacement)?;
            kinds.push(pb::op::Kind::CreateObject(pb::CreateObject {
                document_id: Some(document.to_proto()),
                state: Some(replacement),
            }));
            Some(id.to_string())
        };
        changed.push(VectorEraseReplacement {
            original_id: target.object_id.clone(),
            outline_id: replacement,
            contour_count: count,
        });
    }
    workflow::check_live(closed, cancel)?;
    if kinds.is_empty() {
        return Ok(VectorEraseReceipt {
            transaction_id: request.metadata.transaction_id,
            changed,
            revision: state.info()?,
            estimated_peak_bytes: initial_peak,
        });
    }
    let ops = kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            Ok(pb::Op {
                op_id: Some(pb::OpId {
                    device_id: device.to_string(),
                    lamport: request
                        .metadata
                        .first_lamport
                        .checked_add(index as u64)
                        .ok_or(WorkflowError::Limit)?,
                }),
                kind: Some(kind),
            })
        })
        .collect::<WorkflowResult<Vec<_>>>()?;
    let transaction = pb::Transaction {
        txn_id: Some(Id::try_from(request.metadata.transaction_id.clone())?.to_proto()),
        project_id: Some(project.id.to_proto()),
        device_id: device.to_string(),
        base_revision: Some(state.store()?.revision()?),
        created_at_wall_ms: request.metadata.created_at_ms,
        gesture_id: None,
        ops,
    };
    let remaining = request
        .memory_budget_bytes
        .checked_sub(initial_peak)
        .ok_or(WorkflowError::Limit)?;
    let single =
        vw_ops::HostSequencer::transaction_workspace_estimate_bytes(&transaction, remaining / 16)
            .map_err(|_| WorkflowError::Invalid)?;
    let estimated_peak = add(initial_peak, multiply(single, 16)?)?;
    memory(estimated_peak, request.memory_budget_bytes)?;
    if let Some(replica) = &state.replica {
        if transaction.base_revision.as_ref() != Some(replica.revision()) {
            return Err(WorkflowError::Stale);
        }
        let mut candidate = replica.clone();
        candidate
            .queue(transaction.clone())
            .map_err(|_| WorkflowError::Invalid)?;
    } else {
        state
            .store()?
            .validate_transaction(&transaction, &device, request.metadata.created_at_ms)
            .map_err(|_| WorkflowError::Invalid)?;
    }
    workflow::check_live(closed, cancel)?;
    let revision = state.commit(&transaction, &device, request.metadata.created_at_ms)?;
    // Durable commit is the point of no return; no cancellation check afterward.
    Ok(VectorEraseReceipt {
        transaction_id: request.metadata.transaction_id,
        changed,
        revision,
        estimated_peak_bytes: estimated_peak,
    })
}
fn source_bound(source: &pb::ObjectState) -> WorkflowResult<(usize, u64)> {
    match source.shape.as_ref().ok_or(WorkflowError::Invalid)? {
        pb::object_state::Shape::Stroke(stroke) => Ok((
            eraser::stroke_vertex_bound(stroke).map_err(ink_error)?,
            multiply(stroke.x.len() as u64, 128)?,
        )),
        pb::object_state::Shape::Polygon(value) => {
            let style = source.style.as_ref().ok_or(WorkflowError::Invalid)?;
            if !value.closed || style.width != 0.0 || !style.has_fill || style.fill.is_none() {
                return Err(WorkflowError::Invalid);
            }
            Ok((value.points.len(), multiply(value.points.len() as u64, 32)?))
        }
        _ => Err(WorkflowError::Invalid),
    }
}
