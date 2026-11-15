use crate::{
    worker::{Upkeep, Worker, startup},
    *,
};
use std::{
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use vw_model::{DeviceId, Document, Id, Layer, Project};
use vw_proto::v1 as pb;
use vw_store::{BlobStore, CheckpointNeed, OpenReport, ProjectStore, Upkeeping};

const MAX_SOURCE: usize = 64 * 1024 * 1024;
static SESSIONS: AtomicUsize = AtomicUsize::new(0);
pub(crate) struct SessionPermit;
impl SessionPermit {
    pub(crate) fn acquire() -> Result<Self> {
        SESSIONS
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 8).then_some(n + 1)
            })
            .map_err(|_| CoreError::Backpressure)?;
        Ok(Self)
    }
}
impl Drop for SessionPermit {
    fn drop(&mut self) {
        SESSIONS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[uniffi::export(callback_interface)]
pub trait StateListener: Send + Sync {
    fn on_change(&self, change: StateChange);
}

struct EventState {
    latest: StateChange,
    stopped: bool,
}
pub(crate) struct EventHub {
    state: Mutex<EventState>,
    changed: Condvar,
    subscribers: AtomicUsize,
}
impl EventHub {
    fn new(info: ProjectInfo) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(EventState {
                latest: StateChange {
                    sequence: 1,
                    kind: ChangeKind::Opened,
                    project: info,
                },
                stopped: false,
            }),
            changed: Condvar::new(),
            subscribers: AtomicUsize::new(0),
        })
    }
    pub(crate) fn publish(&self, kind: ChangeKind, project: ProjectInfo) {
        if let Ok(mut state) = self.state.lock() {
            state.latest.sequence = state.latest.sequence.saturating_add(1);
            state.latest.kind = kind;
            state.latest.project = project;
            self.changed.notify_all();
        }
    }
    fn stop(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.stopped = true;
            self.changed.notify_all();
        }
    }
}
#[derive(uniffi::Object)]
pub struct Subscription {
    stop: Arc<AtomicBool>,
    hub: Arc<EventHub>,
}
#[uniffi::export]
impl Subscription {
    /// Stops future delivery. A callback already in flight may finish; callbacks
    /// run without project/ink locks and may reenter async project APIs.
    pub fn unsubscribe(&self) {
        self.stop.store(true, Ordering::Release);
        self.hub.changed.notify_all();
    }
}
impl Drop for Subscription {
    fn drop(&mut self) {
        self.unsubscribe();
    }
}

/// How often an open project looks whether its store needs a checkpoint.
const UPKEEP_EVERY: Duration = Duration::from_secs(5);
/// Looks in a row without a new revision before the project counts as left
/// alone: ten seconds.
const QUIET_LOOKS: u32 = 2;

/// Writes the recovery checkpoint a costly history needs while the project
/// is open, so that an application that is killed, or never closes the
/// project, does not leave the next open with all of it to replay.
///
/// A checkpoint takes as long as the history is large and nothing else in
/// the project runs meanwhile, so it is written when that disturbs least:
/// once it is overdue and the project has been left alone for ten seconds,
/// or, when it is urgent, as soon as nothing is waiting. Never while a
/// stroke is being drawn, whose save would wait for it, and never for a
/// history of strokes, which needs none. Closing writes one that is merely
/// due.
struct CheckpointUpkeep {
    drawing: Arc<AtomicUsize>,
    /// The revision at the last look, and how many looks it has stayed.
    revision: Option<u64>,
    quiet: u32,
    /// The revision a checkpoint failed at: not tried again until it moves.
    failed: Option<u64>,
}
impl CheckpointUpkeep {
    fn look(&mut self, state: &mut ProjectState) {
        let Some(store) = state.store.as_mut() else {
            return;
        };
        let Ok(revision) = store.revision() else {
            return;
        };
        if self.revision == Some(revision.host_seq) {
            self.quiet = self.quiet.saturating_add(1);
        } else {
            self.revision = Some(revision.host_seq);
            self.quiet = 0;
        }
        if self.drawing.load(Ordering::Acquire) != 0 || self.failed == self.revision {
            return;
        }
        let need = if self.quiet >= QUIET_LOOKS {
            CheckpointNeed::Overdue
        } else {
            CheckpointNeed::Urgent
        };
        if !write_checkpoint(store, need, "checkpoint while open") {
            self.failed = self.revision;
        }
    }
}

/// Write the checkpoint the store needs at least as much as `need` and
/// record how long it took, or why it failed. False only for a failure.
fn write_checkpoint(store: &mut ProjectStore, need: CheckpointNeed, name: &'static str) -> bool {
    if store.checkpoint_need() < need {
        return true;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok());
    let Some(now) = now else {
        return true;
    };
    let started = Instant::now();
    let written = store.checkpoint_if(need, now);
    crate::diagnostics::duration("project", name, started.elapsed());
    match written {
        Ok(_) => true,
        Err(error) => {
            crate::diagnostics::failure(name, crate::diagnostics::store_cause(&error));
            false
        }
    }
}

/// Record what opening the store did besides loading the project. None of
/// it can fail an open, so without this a clean-up that never succeeds, or a
/// damaged checkpoint that was passed over, would go unseen.
fn record_open(report: &OpenReport) {
    if report.skipped_checkpoints > 0 {
        crate::diagnostics::count(
            "project",
            "damaged checkpoints passed over",
            report.skipped_checkpoints,
        );
    }
    for (name, step) in [
        ("open tidy", &report.tidy),
        ("open checkpoint", &report.checkpoint),
        ("open compaction", &report.compaction),
    ] {
        match step {
            Upkeeping::NotDue => {}
            Upkeeping::Done(elapsed) => crate::diagnostics::duration("project", name, *elapsed),
            Upkeeping::Failed(elapsed, error) => {
                crate::diagnostics::duration("project", name, *elapsed);
                crate::diagnostics::failure(name, crate::diagnostics::store_cause(error));
            }
            Upkeeping::Skipped => crate::diagnostics::failure(name, "not attempted"),
        }
    }
}

pub(crate) struct ProjectState {
    pub store: Option<ProjectStore>,
    pub blobs: BlobStore,
    pub events: Arc<EventHub>,
    pub history: crate::editor::History,
    pub replica: Option<vw_ops::Replica>,
    pub remote: Option<vw_net::SyncReceiver>,
    pub changed: Arc<tokio::sync::Notify>,
    _permit: Option<SessionPermit>,
    // Last: store/model/history/replica/remote allocations must actually retire
    // before the giant publication's admission is returned to the shared pool.
    _memory_charge: Option<vw_stream::MemoryCharge>,
    pub(crate) draft_edit_sealed: Option<Arc<AtomicBool>>,
    _temporary_root: Option<tempfile::TempDir>,
}
impl ProjectState {
    pub fn store(&self) -> Result<&ProjectStore> {
        self.store.as_ref().ok_or(CoreError::Closed)
    }
    pub fn project(&self) -> Result<&Project> {
        self.store()?;
        Ok(self
            .replica
            .as_ref()
            .map_or(self.store()?.project(), vw_ops::Replica::project))
    }
    pub fn info(&self) -> Result<ProjectInfo> {
        let mut info = project_info(self.store()?, &self.history)?;
        // The store already holds the hash of its own project; only a replica
        // shows a different one, with its pending edits applied.
        if let Some(replica) = &self.replica {
            info.state_hash = replica.state_hash().to_string();
        }
        Ok(info)
    }
    pub fn view_revision(&self) -> Result<pb::Revision> {
        let mut revision = self.store()?.revision()?;
        if let Some(replica) = &self.replica {
            revision.state_hash = replica.state_hash().bytes().to_vec();
        }
        Ok(revision)
    }
    pub(crate) fn previous(&self, id: &pb::Uuid) -> Result<Option<&pb::Transaction>> {
        if let Some(replica) = &self.replica
            && let Some(pending) = replica
                .pending()
                .iter()
                .find(|p| p.transaction.txn_id.as_ref() == Some(id))
        {
            return Ok(Some(&pending.transaction));
        }
        Ok(self
            .store()?
            .accepted_transaction(&Id::from_proto(Some(id))?)
            .map(|(txn, _)| txn))
    }
}
impl Drop for ProjectState {
    fn drop(&mut self) {
        self.events.stop();
    }
}

#[derive(uniffi::Object)]
pub struct ProjectSession {
    pub(crate) worker: Worker<ProjectState>,
    pub(crate) closed: Arc<AtomicBool>,
    pub(crate) active_gestures: Arc<AtomicUsize>,
    events: Arc<EventHub>,
    pub(crate) network_active: Arc<AtomicBool>,
    pub(crate) network_changed: Arc<tokio::sync::Notify>,
}
impl Drop for ProjectSession {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        self.events.stop();
    }
}
/// One constructor payload before history restore, checkpoint parsing or worker
/// startup can fail. Field order also protects failed-constructor destruction.
struct StoreInput {
    store: ProjectStore,
    blobs: BlobStore,
    permit: SessionPermit,
    charge: Option<vw_stream::MemoryCharge>,
    sealed: Option<Arc<AtomicBool>>,
    temporary: Option<tempfile::TempDir>,
}
impl ProjectSession {
    pub(crate) fn from_store(
        store: ProjectStore,
        blobs: BlobStore,
        permit: SessionPermit,
    ) -> Result<Arc<Self>> {
        Self::from_store_owned(StoreInput {
            store,
            blobs,
            permit,
            charge: None,
            sealed: None,
            temporary: None,
        })
    }
    pub(crate) fn from_store_budgeted(
        store: ProjectStore,
        blobs: BlobStore,
        permit: SessionPermit,
        charge: vw_stream::MemoryCharge,
    ) -> Result<Arc<Self>> {
        // No fallible work precedes aggregation of all four publication parts.
        let input = StoreInput {
            store,
            blobs,
            permit,
            charge: Some(charge),
            sealed: None,
            temporary: None,
        };
        if input.charge.as_ref().is_none_or(|charge| {
            charge.bytes() != 32 * 1024 * 1024
                || charge.kind() != vw_stream::MemoryKind::CodecWorkspace
        }) {
            return Err(CoreError::Invalid);
        }
        Self::from_store_owned(input)
    }
    pub(crate) fn from_temporary_store(
        store: ProjectStore,
        blobs: BlobStore,
        permit: SessionPermit,
        temporary: tempfile::TempDir,
        sealed: Arc<AtomicBool>,
        charge: vw_stream::MemoryCharge,
    ) -> Result<Arc<Self>> {
        Self::from_store_owned(StoreInput {
            store,
            blobs,
            permit,
            charge: Some(charge),
            sealed: Some(sealed),
            temporary: Some(temporary),
        })
    }
    fn from_store_owned(input: StoreInput) -> Result<Arc<Self>> {
        let store = &input.store;
        record_open(store.open_report());
        let history = crate::editor::History::restore(store)?;
        let mut remote = None;
        let replica = if store.local_device()? != *store.host_device() {
            let host = vw_ops::HostSequencer::from_checkpoint_bytes(&store.checkpoint_bytes()?)?;
            let mut replica = vw_ops::Replica::new(store.local_device()?, host.snapshot()?)?;
            replica.queue_recovered_batch(store.pending()?)?;
            remote = Some(vw_net::SyncReceiver::new(host));
            Some(replica)
        } else {
            None
        };
        let mut initial = project_info(store, &history)?;
        if let Some(replica) = &replica {
            initial.state_hash = replica.state_hash().to_string();
        }
        let events = EventHub::new(initial);
        let changed = Arc::new(tokio::sync::Notify::new());
        let active_gestures = Arc::new(AtomicUsize::new(0));
        let mut upkeep = CheckpointUpkeep {
            drawing: active_gestures.clone(),
            revision: None,
            quiet: 0,
            failed: None,
        };
        let worker = Worker::with_upkeep(
            "vw-core-store",
            ProjectState {
                store: Some(input.store),
                blobs: input.blobs,
                events: events.clone(),
                history,
                replica,
                remote,
                changed: changed.clone(),
                _permit: Some(input.permit),
                _memory_charge: input.charge,
                draft_edit_sealed: input.sealed,
                _temporary_root: input.temporary,
            },
            16,
            Some(Upkeep {
                every: UPKEEP_EVERY,
                run: Box::new(move |state| upkeep.look(state)),
            }),
        )?;
        Ok(Arc::new(Self {
            worker,
            closed: Arc::new(AtomicBool::new(false)),
            active_gestures,
            events,
            network_active: Arc::new(AtomicBool::new(false)),
            network_changed: changed,
        }))
    }
    pub(crate) fn check_open(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            Err(CoreError::Closed)
        } else {
            Ok(())
        }
    }
}
#[uniffi::export]
impl ProjectSession {
    pub async fn info(&self) -> Result<ProjectInfo> {
        self.check_open()?;
        self.worker.call(|state| state.info()).await
    }
    pub async fn close(&self) -> Result<()> {
        self.closed.store(true, Ordering::Release);
        self.network_changed.notify_one();
        self.network_changed.notify_waiters();
        self.worker
            .shutdown(|state| {
                let info = state
                    .store
                    .as_ref()
                    .map(|store| project_info(store, &state.history))
                    .transpose();
                // A checkpoint takes as long as the history is large. No save
                // ever takes that pause; a project left alone has usually
                // taken it already, and closing takes what is left. Failing
                // to write one loses nothing: the log holds every edit, and
                // the next open writes the checkpoint instead. How long it
                // took and why it failed are recorded.
                if let Some(store) = state.store.as_mut() {
                    write_checkpoint(store, CheckpointNeed::Due, "checkpoint on close");
                }
                state.store.take();
                // The permit and retained publication charge retire with the
                // actual state after its model/history/replica, not this callback.
                if let Ok(Some(info)) = &info {
                    state.events.publish(ChangeKind::Closed, info.clone());
                }
                state.events.stop();
                info.map(|_| ())
            })
            .await
    }
    pub fn subscribe(&self, listener: Box<dyn StateListener>) -> Result<Arc<Subscription>> {
        self.check_open()?;
        self.events
            .subscribers
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 4).then_some(n + 1)
            })
            .map_err(|_| CoreError::Backpressure)?;
        let hub = self.events.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let spawned = std::thread::Builder::new()
            .name("vw-core-events".into())
            .spawn(move || {
                let mut seen = 0;
                loop {
                    let Ok(mut state) = hub.state.lock() else {
                        break;
                    };
                    while !state.stopped
                        && !stopping.load(Ordering::Acquire)
                        && state.latest.sequence == seen
                    {
                        match hub.changed.wait(state) {
                            Ok(value) => state = value,
                            Err(_) => {
                                hub.subscribers.fetch_sub(1, Ordering::AcqRel);
                                return;
                            }
                        }
                    }
                    if stopping.load(Ordering::Acquire) {
                        break;
                    }
                    let done = state.stopped;
                    let change = state.latest.clone();
                    drop(state);
                    if change.sequence != seen {
                        seen = change.sequence;
                        listener.on_change(change);
                    }
                    if done {
                        break;
                    }
                }
                hub.subscribers.fetch_sub(1, Ordering::AcqRel);
            });
        if spawned.is_err() {
            self.events.subscribers.fetch_sub(1, Ordering::AcqRel);
            return Err(CoreError::Worker);
        }
        Ok(Arc::new(Subscription {
            stop,
            hub: self.events.clone(),
        }))
    }
    pub async fn render_list(
        &self,
        document_id: String,
        rectangle: Option<QueryRect>,
        cancellation: Arc<Cancellation>,
    ) -> Result<RenderList> {
        self.check_open()?;
        self.worker
            .call(move |state| {
                cancellation.check()?;
                let result = crate::queries::render_list(state, &document_id, rectangle)?;
                cancellation.check()?;
                Ok(result)
            })
            .await
    }
    pub async fn export_image(
        &self,
        options: ExportOptions,
        cancellation: Arc<Cancellation>,
    ) -> Result<ExportResult> {
        self.check_open()?;
        self.worker
            .call(move |state| {
                cancellation.check()?;
                let info = state.info()?;
                state
                    .events
                    .publish(ChangeKind::ExportStarted, info.clone());
                let timer = crate::diagnostics::timer("export", "image");
                let result =
                    crate::queries::export(state, options, &cancellation).and_then(|value| {
                        cancellation.check()?;
                        Ok(value)
                    });
                drop(timer);
                if let Ok(exported) = &result {
                    crate::diagnostics::count(
                        "export",
                        "image bytes",
                        u64::try_from(exported.bytes.len()).unwrap_or(u64::MAX),
                    );
                }
                state.events.publish(
                    if result.is_ok() {
                        ChangeKind::ExportReady
                    } else {
                        ChangeKind::ExportFailed
                    },
                    info,
                );
                result
            })
            .await
    }
}

#[uniffi::export]
pub async fn open_project(
    path: String,
    cancellation: Arc<Cancellation>,
) -> Result<Arc<ProjectSession>> {
    valid_path(&path)?;
    startup(move || {
        cancellation.check()?;
        let _timer = crate::diagnostics::timer("project", "open");
        let permit = SessionPermit::acquire()?;
        let store = ProjectStore::open(Path::new(&path))?;
        let blobs = BlobStore::new(Path::new(&path))?;
        cancellation.check()?;
        ProjectSession::from_store(store, blobs, permit)
    })
    .await
}
#[uniffi::export]
pub async fn create_image_project(
    options: CreateImageProject,
    cancellation: Arc<Cancellation>,
) -> Result<Arc<ProjectSession>> {
    valid_path(&options.path)?;
    if options.source.is_empty()
        || options.source.len() > MAX_SOURCE
        || options.title.len() > 4096
        || options.now_ms < 0
    {
        return Err(CoreError::Invalid);
    }
    startup(move || {
        cancellation.check()?;
        let permit = SessionPermit::acquire()?;
        let project_id = Id::try_from(options.project_id)?;
        let doc_id = Id::try_from(options.document_id)?;
        let layer_id = Id::try_from(options.layer_id)?;
        let device = DeviceId::try_from(options.device_id)?;
        let import_metadata =
            crate::format_images::import_metadata(&options.source, decode_limits(), &cancellation)?;
        let image = crate::format_images::decode(&options.source, decode_limits(), &|| {
            cancellation.is_cancelled()
        })?;
        let mut project = Project::new(project_id, options.title.clone(), device.clone());
        let (width, height) = if image.orientation_applied >= 5 {
            (image.height, image.width)
        } else {
            (image.width, image.height)
        };
        let format = crate::original_image_label(&options.source);
        project.assets.insert(
            image.source_asset.clone(),
            pb::AddAsset {
                asset_id: image.source_asset.to_string(),
                format: format.into(),
                width,
                height,
                orientation: u32::from(image.orientation_applied),
                bit_depth: u32::from(image.pixels.bit_depth()),
                has_alpha: image.pixels.has_alpha(),
                color_space: if image.icc.is_some() {
                    "ICC"
                } else {
                    "untagged"
                }
                .into(),
                icc_profile: image.icc.clone().unwrap_or_default(),
                byte_size: options.source.len() as u64,
                source: "import".into(),
                metadata_json: import_metadata,
                ..Default::default()
            },
        );
        project.documents.insert(
            doc_id.clone(),
            Document {
                definition: pb::CreateDocument {
                    document_id: Some(doc_id.to_proto()),
                    kind: pb::DocumentKind::Image as i32,
                    schema_version: 1,
                    title: options.title,
                    primary_asset_id: image.source_asset.to_string(),
                    ..Default::default()
                },
                pages: vec![],
                created_at_ms: options.now_ms,
            },
        );
        project.layers.insert(
            layer_id.clone(),
            Layer {
                definition: pb::CreateLayer {
                    layer_id: Some(layer_id.to_proto()),
                    document_id: Some(doc_id.to_proto()),
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
        cancellation.check()?;
        let asset = image.source_asset.clone();
        drop(image);
        let (store, blobs) = crate::creation::create_complete(
            Path::new(&options.path),
            crate::creation::InitialProject {
                project,
                device,
                time: options.now_ms,
            },
            &options.source,
            &asset,
            &cancellation,
            |_| Ok(()),
        )?;
        ProjectSession::from_store(store, blobs, permit)
    })
    .await
}
pub(crate) fn decode_limits() -> vw_raster::DecodeLimits {
    vw_raster::DecodeLimits {
        max_encoded_bytes: MAX_SOURCE,
        max_pixels: 50_000_000,
        max_memory_bytes: 512 * 1024 * 1024,
    }
}
fn valid_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.len() > 32767
        || path.contains('\0')
        || !Path::new(path).is_absolute()
    {
        Err(CoreError::Invalid)
    } else {
        Ok(())
    }
}
pub(crate) fn project_info(
    store: &ProjectStore,
    history: &crate::editor::History,
) -> Result<ProjectInfo> {
    let revision = store.revision()?;
    Ok(ProjectInfo {
        project_id: store.project().id.to_string(),
        title: store.project().title.clone(),
        device_id: store.local_device()?.to_string(),
        next_lamport: history.next_lamport,
        can_undo: history.undo.operation(false, history.next_lamport).is_ok(),
        can_redo: history.undo.operation(true, history.next_lamport).is_ok(),
        host_seq: revision.host_seq,
        state_hash: store.state_hash()?.to_string(),
        document_ids: store
            .project()
            .documents
            .keys()
            .map(ToString::to_string)
            .collect(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::future::{Future, poll_fn};
    use std::task::Poll;
    #[tokio::test]
    async fn saturated_project_close_releases_the_database_even_after_waiter_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project");
        let device = DeviceId::from_bytes([3; 16]);
        let model = Project::new(
            Id::from_parts(1700000000000, [4; 10]).unwrap(),
            "close fixture".into(),
            device.clone(),
        );
        let store = ProjectStore::create(&path, model, device, 0).unwrap();
        let blobs = BlobStore::new(&path).unwrap();
        let project =
            ProjectSession::from_store(store, blobs, SessionPermit::acquire().unwrap()).unwrap();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        project
            .worker
            .enqueue(Box::new(move |_| {
                let _ = started.send(());
                wait.recv().unwrap();
            }))
            .unwrap();
        ready.await.unwrap();
        for _ in 0..16 {
            project.worker.enqueue(Box::new(|_| {})).unwrap();
        }
        let mut close = Box::pin(project.close());
        assert!(poll_fn(|cx| Poll::Ready(close.as_mut().poll(cx).is_pending())).await);
        drop(close);
        assert!(matches!(project.info().await, Err(CoreError::Closed)));
        release.send(()).unwrap();
        project.close().await.unwrap();
        // Keep the closed facade alive: its explicit close must already have
        // released the SQLite writer/lock, independently of object destruction.
        let reopened = ProjectStore::open(&path).unwrap();
        reopened.integrity_check().unwrap();
    }

    #[test]
    fn what_an_open_did_besides_loading_the_project_is_on_record() {
        let before = crate::core_diagnostics(0).latest_sequence;
        record_open(&OpenReport {
            skipped_checkpoints: 2,
            tidy: Upkeeping::Done(Duration::from_micros(3_001)),
            checkpoint: Upkeeping::Failed(Duration::from_micros(5_003), vw_store::StoreError::Busy),
            compaction: Upkeeping::Skipped,
        });
        record_open(&OpenReport::default());
        let events = crate::core_diagnostics(before).events;
        let has = |kind, area: &str, name: &str, value| {
            events.iter().any(|event| {
                event.kind == kind
                    && event.area == area
                    && event.name == name
                    && event.value == value
            })
        };
        use crate::CoreDiagnosticKind::{Count, DurationMicros, Failure};
        assert!(has(Count, "project", "damaged checkpoints passed over", 2));
        assert!(has(DurationMicros, "project", "open tidy", 3_001));
        assert!(has(DurationMicros, "project", "open checkpoint", 5_003));
        assert!(has(Failure, "open checkpoint", "store busy", 0));
        assert!(has(Failure, "open compaction", "not attempted", 0));
        // An open that had nothing to tidy records nothing.
        assert!(!has(Failure, "open tidy", "not attempted", 0));
    }

    /// The look an open project takes every few seconds: what it asks of the
    /// store depends on how long the project has been left alone, and it
    /// asks nothing while a stroke is being drawn or after a failure.
    #[tokio::test]
    async fn an_open_project_asks_for_an_overdue_checkpoint_only_once_it_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project");
        let device = DeviceId::from_bytes([3; 16]);
        let model = Project::new(
            Id::from_parts(1700000000000, [4; 10]).unwrap(),
            "upkeep fixture".into(),
            device.clone(),
        );
        let store = ProjectStore::create(&path, model, device, 0).unwrap();
        let blobs = BlobStore::new(&path).unwrap();
        let project =
            ProjectSession::from_store(store, blobs, SessionPermit::acquire().unwrap()).unwrap();
        let drawing = project.active_gestures.clone();
        let looked = project
            .worker
            .call(move |state| {
                let mut upkeep = CheckpointUpkeep {
                    drawing,
                    revision: None,
                    quiet: 0,
                    failed: None,
                };
                let mut seen = Vec::new();
                for look in 0..4 {
                    if look == 3 {
                        upkeep.drawing.store(1, Ordering::Release);
                    }
                    upkeep.look(state);
                    seen.push((upkeep.revision, upkeep.quiet, upkeep.failed));
                }
                upkeep.drawing.store(0, Ordering::Release);
                Ok(seen)
            })
            .await
            .unwrap();
        // The first look only notes the revision; each later one without a
        // new revision counts as quiet. Nothing is due in an empty project,
        // so nothing was written and nothing failed.
        assert_eq!(
            looked,
            [
                (Some(0), 0, None),
                (Some(0), 1, None),
                (Some(0), 2, None),
                (Some(0), 3, None)
            ]
        );
        project.close().await.unwrap();
    }
}

#[cfg(test)]
#[path = "project_budget_tests.rs"]
mod budget_tests;
