use crate::{
    BoundsSpace, Cancellation, CapturedElement, Error, Platform, Result, Snapshot, check, codec,
    snapshot,
};
use serde::{Deserialize, Serialize};
use vw_model::{DeviceId, Id, Project, StateHash};
use vw_ops::{Acceptance, HostSequencer};
use vw_proto::v1;

/// Exact local capture identity. Native handles/monitor identity remain local;
/// ReferenceExport deliberately excludes this metadata and window titles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureContext {
    session_id: Id,
    frame_id: u64,
    primary_asset_id: String,
    geometry: v1::CaptureGeometry,
    capture_platform: String,
    captured_at_ms: i64,
}
impl CaptureContext {
    fn for_document(project: &Project, document: &Id) -> Result<Self> {
        let definition = &project
            .documents
            .get(document)
            .ok_or(Error::Missing)?
            .definition;
        if definition.kind != v1::DocumentKind::Capture as i32 {
            return Err(Error::Invalid("capture document"));
        }
        let capture = definition.capture.as_ref().ok_or(Error::Missing)?;
        vw_model::validate_capture(capture)?;
        Ok(Self {
            session_id: Id::from_proto(capture.capture_session_id.as_ref())?,
            frame_id: capture.frame_id,
            primary_asset_id: definition.primary_asset_id.clone(),
            geometry: capture.geometry.as_ref().ok_or(Error::Missing)?.clone(),
            capture_platform: capture.platform.clone(),
            captured_at_ms: capture.captured_at_ms,
        })
    }
    pub fn session_id(&self) -> &Id {
        &self.session_id
    }
    pub fn frame_id(&self) -> u64 {
        self.frame_id
    }
    pub fn primary_asset_id(&self) -> &str {
        &self.primary_asset_id
    }
    pub fn geometry_revision(&self) -> u32 {
        self.geometry.geometry_revision
    }
    pub fn captured_at_ms(&self) -> i64 {
        self.captured_at_ms
    }
    pub fn extent(&self) -> Result<[f64; 2]> {
        let rect = self
            .geometry
            .client_rect_host
            .as_ref()
            .ok_or(Error::Invalid("capture rectangle"))?;
        Ok([f64::from(rect.w), f64::from(rect.h)])
    }
    pub(crate) fn host_origin(&self) -> Result<[f64; 2]> {
        let rect = self
            .geometry
            .client_rect_host
            .as_ref()
            .ok_or(Error::Invalid("capture rectangle"))?;
        Ok([f64::from(rect.x), f64::from(rect.y)])
    }
    pub(crate) fn validate(&self, platform: Platform) -> Result<()> {
        codec::admission(&self.geometry, 16 * 1024)?;
        crate::text(&self.capture_platform, 16, true)?;
        crate::text(&self.primary_asset_id, 64, false)?;
        if !self.primary_asset_id.is_empty() {
            vw_model::AssetId::try_from(self.primary_asset_id.clone())?;
        }
        if (platform == Platform::AndroidAx) != (self.capture_platform == "android") {
            return Err(Error::Invalid("semantic platform binding"));
        }
        vw_model::validate_capture(&v1::CaptureInfo {
            capture_session_id: Some(self.session_id.to_proto()),
            frame_id: self.frame_id,
            geometry: Some(self.geometry.clone()),
            platform: self.capture_platform.clone(),
            captured_at_ms: self.captured_at_ms,
            ..Default::default()
        })?;
        let [width, height] = self.extent()?;
        if width > 1_000_000.0 || height > 1_000_000.0 {
            return Err(Error::Limit("capture extent"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureBinding {
    pub project_id: Id,
    pub document_id: Id,
    pub host_seq: u64,
    pub state_hash: StateHash,
}
impl CaptureBinding {
    pub fn revision(&self) -> v1::Revision {
        v1::Revision {
            host_seq: self.host_seq,
            state_hash: self.state_hash.bytes().to_vec(),
        }
    }
}
pub struct CaptureView<'a> {
    project: &'a Project,
    binding: CaptureBinding,
    context: CaptureContext,
}
impl<'a> CaptureView<'a> {
    pub fn new(project: &'a Project, revision: &v1::Revision, document: &Id) -> Result<Self> {
        // Bound the model serialization before canonical hashing allocates.
        codec::admission(project, 64 * 1024 * 1024)?;
        let context = CaptureContext::for_document(project, document)?;
        let hash = project.state_hash()?;
        if revision.state_hash != hash.bytes() {
            return Err(Error::Stale);
        }
        Ok(Self {
            project,
            binding: CaptureBinding {
                project_id: project.id.clone(),
                document_id: document.clone(),
                host_seq: revision.host_seq,
                state_hash: hash,
            },
            context,
        })
    }
    pub fn binding(&self) -> &CaptureBinding {
        &self.binding
    }
    pub fn context(&self) -> &CaptureContext {
        &self.context
    }
    /// The adapter must retain the context captured BEFORE platform collection;
    /// passing it back prevents a late tree from attaching to a new frame.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &self,
        observed: &CaptureContext,
        snapshot_id: Id,
        platform: Platform,
        frame_delta_ms: i32,
        collection_elapsed_ms: u64,
        space: BoundsSpace,
        elements: Vec<CapturedElement>,
        cancel: &dyn Cancellation,
    ) -> Result<Snapshot> {
        if observed != &self.context {
            return Err(Error::Stale);
        }
        observed.validate(platform)?;
        if self.project.semantic_snapshots.contains_key(&snapshot_id) {
            return Err(Error::Invalid("duplicate snapshot"));
        }
        snapshot::capture(
            self.binding.project_id.clone(),
            self.binding.document_id.clone(),
            snapshot_id,
            self.context.clone(),
            platform,
            frame_delta_ms,
            collection_elapsed_ms,
            space,
            elements,
            cancel,
        )
    }
    pub fn plan(
        &self,
        snapshot: &Snapshot,
        metadata: EditMetadata,
        cancel: &dyn Cancellation,
    ) -> Result<SnapshotPlan> {
        check(cancel)?;
        if snapshot.project_id() != &self.binding.project_id
            || snapshot.document_id() != &self.binding.document_id
            || snapshot.context() != &self.context
        {
            return Err(Error::Stale);
        }
        if self.project.semantic_snapshots.contains_key(snapshot.id()) {
            return Err(Error::Invalid("duplicate snapshot"));
        }
        if metadata.lamport == 0 {
            return Err(Error::Invalid("Lamport counter"));
        }
        let next_lamport = metadata
            .lamport
            .checked_add(1)
            .ok_or(Error::Limit("Lamport counter"))?;
        let operation = v1::AddSemanticSnapshot {
            snapshot_id: Some(snapshot.id().to_proto()),
            document_id: Some(self.binding.document_id.to_proto()),
            platform: snapshot.platform().as_str().into(),
            frame_delta_ms: snapshot.frame_delta_ms(),
            elements_json_zstd: snapshot.encode(cancel)?,
        };
        Ok(SnapshotPlan {
            binding: self.binding.clone(),
            next_lamport,
            transaction: v1::Transaction {
                txn_id: Some(metadata.transaction_id.to_proto()),
                project_id: Some(self.binding.project_id.to_proto()),
                device_id: metadata.device.to_string(),
                base_revision: Some(self.binding.revision()),
                created_at_wall_ms: metadata.created_at_ms,
                gesture_id: None,
                ops: vec![v1::Op {
                    op_id: Some(v1::OpId {
                        device_id: metadata.device.to_string(),
                        lamport: metadata.lamport,
                    }),
                    kind: Some(v1::op::Kind::AddSemanticSnapshot(operation)),
                }],
            },
        })
    }
    pub fn load(&self, id: &Id, cancel: &dyn Cancellation) -> Result<Snapshot> {
        let stored = self
            .project
            .semantic_snapshots
            .get(id)
            .ok_or(Error::Missing)?;
        let data = Snapshot::decode(&stored.definition.elements_json_zstd, cancel)?;
        if data.project_id() != &self.binding.project_id
            || data.id() != id
            || stored.definition.snapshot_id != Some(id.to_proto())
            || stored.definition.document_id != Some(self.binding.document_id.to_proto())
            || data.document_id() != &self.binding.document_id
            || data.context() != &self.context
            || stored.capture_session_id.as_ref() != Some(self.context.session_id())
            || stored.frame_id != Some(self.context.frame_id())
            || data.platform().as_str() != stored.definition.platform
            || data.frame_delta_ms() != stored.definition.frame_delta_ms
        {
            return Err(Error::Stale);
        }
        Ok(data)
    }
}
pub struct EditMetadata {
    pub transaction_id: Id,
    pub device: DeviceId,
    pub lamport: u64,
    pub created_at_ms: i64,
}
pub struct SnapshotPlan {
    binding: CaptureBinding,
    transaction: v1::Transaction,
    next_lamport: u64,
}
impl SnapshotPlan {
    pub fn binding(&self) -> &CaptureBinding {
        &self.binding
    }
    pub fn transaction(&self) -> &v1::Transaction {
        &self.transaction
    }
    pub fn next_lamport(&self) -> u64 {
        self.next_lamport
    }
    /// Call under the same project mutation lock as the durable store commit.
    pub fn check_current(&self, project: &Project, revision: &v1::Revision) -> Result<()> {
        if CaptureView::new(project, revision, &self.binding.document_id)?.binding != self.binding {
            return Err(Error::Stale);
        }
        Ok(())
    }
    /// Reference in-memory application, never a claim of durable persistence.
    /// Exact accepted retries use the canonical host's byte-identity check.
    pub fn submit(
        &self,
        host: &mut HostSequencer,
        authenticated: &DeviceId,
        now_ms: i64,
    ) -> Result<Acceptance> {
        if self.transaction.device_id != authenticated.as_str() {
            return Err(Error::Unauthorized);
        }
        let id = Id::from_proto(self.transaction.txn_id.as_ref())?;
        if host.accepted_transaction(&id).is_none() {
            self.check_current(host.project(), &host.revision()?)?;
        }
        Ok(host.submit(self.transaction.clone(), authenticated, now_ms)?)
    }
}
