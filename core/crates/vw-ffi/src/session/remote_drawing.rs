//! Normal application-owned Paint experiment. Foreign bytes never create proof.
use super::*;
#[derive(Clone, Copy, uniffi::Enum)]
pub enum RemoteDrawingCanvas {
    Unchanged,
    Restored,
    RecoveryNeeded,
    Unknown,
}
#[derive(uniffi::Object)]
pub struct RemoteDrawingPreparation {
    #[cfg(windows)]
    owner: Arc<Owner>,
}
#[cfg(windows)]
use {
    compatibility::ObservationContext,
    std::sync::{
        OnceLock, Weak,
        atomic::{AtomicBool, AtomicU8},
    },
    vw_remote_host::{
        editor_probe_runner as producer, editor_source::SourceLease, process::Cancellation,
    },
};
#[cfg(windows)]
pub(super) struct Files {
    pub observation: String,
    pub observation_sha: String,
    pub drawing: String,
    pub drawing_sha: String,
}
#[cfg(windows)]
pub(super) struct Evidence {
    pub context: ObservationContext,
    pub source: Arc<SourceLease>,
    pub completed: producer::CompletedPaintDrawingObservation,
    pub receipt: Vec<u8>,
}
#[cfg(windows)]
struct StateOwned {
    preparing: bool,
    running: bool,
    prepared: Option<producer::PaintDrawingPreparation>,
    source: Option<Arc<SourceLease>>,
    failed: bool,
    canvas: RemoteDrawingCanvas,
    retirement_handle: Option<producer::PaintDrawingRetirementHandle>,
    query_pending: bool,
}
#[cfg(windows)]
struct Owner {
    hub: Weak<RemoteHub>,
    context: ObservationContext,
    files: Arc<Files>,
    gate: Arc<drawing_gate::Gate>,
    cancel: Arc<Cancellation>,
    worker_cancel: Arc<Cancellation>,
    step: AtomicU8,
    detached: AtomicBool,
    retired: AtomicBool,
    state: Mutex<StateOwned>,
    retirement: Mutex<()>,
}
#[cfg(windows)]
fn owners() -> &'static Mutex<Vec<Arc<Owner>>> {
    static OWNERS: OnceLock<Mutex<Vec<Arc<Owner>>>> = OnceLock::new();
    OWNERS.get_or_init(|| Mutex::new(Vec::new()))
}
#[cfg(windows)]
pub(super) fn busy(_hub: &Arc<RemoteHub>) -> SessionResult<bool> {
    // The old global input-helper permit is temporarily retired. Fence every
    // session in this process until the actual drawing owner has retired.
    retirement_count()?;
    Ok(owners()
        .lock()
        .map_err(|_| SessionError::Worker)?
        .iter()
        .any(|o| !o.retired.load(Ordering::Acquire)))
}
#[cfg(windows)]
pub(super) fn retirement_count() -> SessionResult<usize> {
    let pending = owners().lock().map_err(|_| SessionError::Worker)?.clone();
    for owner in pending {
        if owner.detached.load(Ordering::Acquire) {
            let _ = owner.retire();
        }
    }
    let mut all = owners().lock().map_err(|_| SessionError::Worker)?;
    all.retain(|o| !o.retired.load(Ordering::Acquire));
    Ok(all.len())
}
#[cfg(windows)]
impl Owner {
    fn signal_cancel(&self) {
        if let Some(hub) = self.hub.upgrade() {
            if let Ok(_state) = hub.state.lock() {
                self.cancel.cancel();
                return;
            }
        }
        self.cancel.cancel();
    }
    fn current(&self, allow_release: bool) -> SessionResult<Arc<RemoteHub>> {
        let hub = self.hub.upgrade().ok_or(SessionError::Closed)?;
        self.context.current(&hub)?;
        let s = hub.state.lock().map_err(|_| SessionError::Worker)?;
        if self.gate.cancelled.load(Ordering::Acquire)
            || (!allow_release && self.gate.release.load(Ordering::Acquire))
            || s.drawing
                .as_ref()
                .is_none_or(|g| !Arc::ptr_eq(g, &self.gate))
            || s.runtime
                .as_ref()
                .and_then(|r| r.drawing.as_ref())
                .is_none_or(|f| !Arc::ptr_eq(f, &self.files))
        {
            return Err(SessionError::Closed);
        }
        drop(s);
        Ok(hub)
    }
    fn prepare(self: &Arc<Self>) -> SessionResult<()> {
        let result = (|| {
            // The owner's PC action may have focused the palette. Restore only
            // its exact granted target; Windows may refuse, without a bypass.
            self.cancel.check().map_err(failure)?;
            self.current(false)?;
            vw_remote_host::platform::focus_for_owner_grant(
                &self.context.target,
                std::process::id(),
            )
            .map_err(failure)?;
            let started = Instant::now();
            while !self.gate.parked.load(Ordering::Acquire) {
                self.cancel.check().map_err(failure)?;
                self.current(false)?;
                if self.gate.refused.load(Ordering::Acquire) {
                    return Err(SessionError::Backpressure);
                }
                if started.elapsed() > Duration::from_millis(2500) {
                    return Err(SessionError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            self.cancel.check().map_err(failure)?;
            self.current(false)?;
            let source = Arc::new(
                SourceLease::open_paint_observation(
                    &self.context.target,
                    std::process::id(),
                    &self.context.identity,
                )
                .map_err(failure)?,
            );
            self.state.lock().map_err(|_| SessionError::Worker)?.source = Some(source.clone());
            let context_bytes = serde_json::to_vec(&(
                self.context.local_epoch,
                self.context.source_epoch,
                self.context.grant_revision,
                &self.context.binding,
                &self.context.target,
                &self.context.identity,
                &self.context.profile_digest,
                &self.files.observation_sha,
                &self.files.drawing_sha,
            ))
            .map_err(|_| SessionError::Invalid)?;
            use sha2::{Digest, Sha256};
            let context_sha = format!("{:x}", Sha256::digest(context_bytes));
            let weak = Arc::downgrade(self);
            let current: producer::Current = Arc::new(move || {
                let o = weak.upgrade().ok_or(vw_remote::Error::Cancelled)?;
                o.cancel.check()?;
                o.current(false)
                    .map_err(|_| vw_remote::Error::TargetChanged)?;
                Ok(())
            });
            let weak = Arc::downgrade(self);
            let authorize: producer::PaintDrawingAuthorize = Arc::new(move |step, write| {
                let o = weak.upgrade().ok_or(vw_remote::Error::Cancelled)?;
                o.cancel.check()?;
                let hub = o.hub.upgrade().ok_or(vw_remote::Error::Cancelled)?;
                // Actual state guard protects this fixed ready-pipe write from
                // source/grant revocation. No provider or join under this guard.
                let s = hub.state.lock().map_err(|_| vw_remote::Error::Io)?;
                let index = o.step.load(Ordering::Acquire);
                let expected = match index {
                    0 => producer::PaintDrawingInputStep::CanvasDot,
                    1 | 3 => producer::PaintDrawingInputStep::Undo,
                    2 => producer::PaintDrawingInputStep::Redo,
                    _ => return Err(vw_remote::Error::Invalid),
                };
                if step != expected
                    || !o.gate.parked.load(Ordering::Acquire)
                    || o.gate.release.load(Ordering::Acquire)
                    || o.gate.cancelled.load(Ordering::Acquire)
                    || s.host_retiring
                    || s.peer_retiring
                    || s.closed
                    || !s.enabled
                    || s.epoch != o.context.local_epoch
                    || s.source_epoch != o.context.source_epoch
                    || s.grant_revision != o.context.grant_revision
                    || s.binding.as_ref() != Some(&o.context.binding)
                    || s.drawing.as_ref().is_none_or(|g| !Arc::ptr_eq(g, &o.gate))
                    || s.admission.drawing != Some(o.gate.nonce)
                    || s.selected.as_ref().is_none_or(|v| {
                        v.target != o.context.target || v.scope != o.context.binding.scope
                    })
                    || s.authority.as_ref().is_none_or(|a| {
                        a.identity != o.context.identity
                            || a.profile_digest != o.context.profile_digest
                    })
                    || s.runtime
                        .as_ref()
                        .and_then(|r| r.observation.as_ref())
                        .is_none_or(|h| !Arc::ptr_eq(h, &o.context.helper))
                    || s.runtime
                        .as_ref()
                        .and_then(|r| r.drawing.as_ref())
                        .is_none_or(|f| !Arc::ptr_eq(f, &o.files))
                {
                    return Err(vw_remote::Error::TargetChanged);
                }
                o.cancel.check()?;
                o.step.store(index + 1, Ordering::Release);
                write()
            });
            let prepared = producer::prepare_paint_drawing(
                producer::PaintDrawingPaths {
                    observation_helper: self.files.observation.clone().into(),
                    observation_sha256: self.files.observation_sha.clone(),
                    drawing_helper: self.files.drawing.clone().into(),
                    drawing_sha256: self.files.drawing_sha.clone(),
                },
                self.context.target.clone(),
                source,
                context_sha,
                self.context.binding.scope.capture_session_id.clone(),
                fresh_id()?,
                fresh_id()?,
                self.cancel.clone(),
                current,
                authorize,
            )
            .map_err(failure)?;
            self.current(false)?;
            self.cancel.check().map_err(failure)?;
            let handle = prepared.retirement_handle();
            let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
            s.retirement_handle = Some(handle);
            s.prepared = Some(prepared);
            Ok(())
        })();
        let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
        s.preparing = false;
        if result.is_err() {
            s.failed = true;
        }
        result
    }
    fn run(&self) -> SessionResult<Vec<u8>> {
        self.cancel.check().map_err(failure)?;
        self.current(false)?;
        let prepared = {
            let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
            if s.preparing || s.running || s.failed || self.retired.load(Ordering::Acquire) {
                return Err(SessionError::Closed);
            }
            let p = s.prepared.take().ok_or(SessionError::Closed)?;
            s.running = true;
            p
        };
        let result = (|| {
            // Confirmation's actual window is disposed before this call. Bring
            // the same source forward, then let the helper independently guard
            // foreground/UIA/pointer state before every capture and input.
            self.cancel.check().map_err(failure)?;
            self.current(false)?;
            vw_remote_host::platform::focus_for_owner_grant(
                &self.context.target,
                std::process::id(),
            )
            .map_err(failure)?;
            let completed = prepared.run_confirmed().map_err(failure)?;
            // Completed getters carry evidence, no currentness. Recheck native
            // source and actual session/grant/cancellation after native return.
            self.cancel.check().map_err(failure)?;
            let hub = self.current(false)?;
            let source = self
                .state
                .lock()
                .map_err(|_| SessionError::Worker)?
                .source
                .clone()
                .ok_or(SessionError::Closed)?;
            source
                .verify_observation(&self.context.target, std::process::id())
                .map_err(failure)?;
            self.state
                .lock()
                .map_err(|_| SessionError::Worker)?
                .query_pending = true;
            let controls = self
                .context
                .helper
                .observe(&self.context.target, source.clone(), &self.cancel)
                .map_err(failure)?;
            self.state
                .lock()
                .map_err(|_| SessionError::Worker)?
                .query_pending = false;
            self.cancel.check().map_err(failure)?;
            self.current(false)?;
            source
                .verify_observation(&self.context.target, std::process::id())
                .map_err(failure)?;
            if controls.tool_id != completed.tool_id()
                || controls.settings_digest != completed.settings_digest()
            {
                return Err(SessionError::Authentication);
            }
            if completed.target() != &self.context.target
                || completed.image() != &self.context.identity
                || completed.tool_id() != "paint.brush"
                || !vw_remote::profile::digest(completed.settings_digest())
                || !vw_remote::profile::digest(completed.evidence_sha256())
                || completed.disposed_private_files() != 10
            {
                return Err(SessionError::Authentication);
            }
            let receipt = drawing_receipt(&self.context, &completed)?;
            self.context.current(&hub)?;
            let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
            if self.cancel.check().is_err()
                || s.epoch != self.context.local_epoch
                || s.grant_revision != self.context.grant_revision
                || s.source_epoch != self.context.source_epoch
                || s.host_retiring
                || s.peer_retiring
                || s.closed
                || !s.enabled
                || s.binding.as_ref() != Some(&self.context.binding)
                || s.drawing
                    .as_ref()
                    .is_none_or(|g| !Arc::ptr_eq(g, &self.gate))
                || s.selected.as_ref().is_none_or(|v| {
                    v.target != self.context.target || v.scope != self.context.binding.scope
                })
                || s.authority.as_ref().is_none_or(|a| {
                    a.identity != self.context.identity
                        || a.profile_digest != self.context.profile_digest
                })
                || s.runtime
                    .as_ref()
                    .and_then(|r| r.drawing.as_ref())
                    .is_none_or(|f| !Arc::ptr_eq(f, &self.files))
                || self.gate.cancelled.load(Ordering::Acquire)
                || self.gate.release.load(Ordering::Acquire)
            {
                return Err(SessionError::Closed);
            }
            s.drawing_evidence = Some(Arc::new(Evidence {
                context: self.context.clone(),
                source,
                completed,
                receipt: receipt.clone(),
            }));
            Ok(receipt)
        })();
        let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
        s.running = false;
        match &result {
            Ok(_bytes) => {
                s.canvas = RemoteDrawingCanvas::Restored;
            }
            Err(_) => s.failed = true,
        };
        result
    }
    fn abort(&self) {
        self.gate.cancelled.store(true, Ordering::Release);
        self.worker_cancel.cancel();
        if let Some(hub) = self.hub.upgrade() {
            if let Ok(mut s) = hub.state.lock() {
                // Never pause a successor. Only this actual gate/binding may retire.
                if s.drawing
                    .as_ref()
                    .is_some_and(|g| Arc::ptr_eq(g, &self.gate))
                    && s.binding.as_ref() == Some(&self.context.binding)
                {
                    let _ = hub.send_control(&mut s, "pause", "owner_pause");
                    RemoteHub::retire_authority(&mut s);
                    let _ = hub.publish(&mut s, "paused", Some("owner_pause"));
                }
            }
        }
    }
    fn retire(&self) -> SessionResult<RemoteDrawingCanvas> {
        let _retirement = self
            .retirement
            .try_lock()
            .map_err(|_| SessionError::RemoteRetirementPending)?;
        if self.retired.load(Ordering::Acquire) {
            return Ok(self.state.lock().map_err(|_| SessionError::Worker)?.canvas);
        }
        self.signal_cancel();
        let (running, failed, prepared) = {
            let mut s = self.state.lock().map_err(|_| SessionError::Worker)?;
            (s.preparing || s.running, s.failed, s.prepared.take())
        };
        if running {
            return Err(SessionError::RemoteRetirementPending);
        }
        drop(prepared);
        let mut canvas = self.state.lock().map_err(|_| SessionError::Worker)?.canvas;
        let (handle, has_source, query_pending) = {
            let s = self.state.lock().map_err(|_| SessionError::Worker)?;
            (
                s.retirement_handle.clone(),
                s.source.is_some(),
                s.query_pending,
            )
        };
        if let Some(handle) = handle {
            let actual = match handle.retire() {
                Ok(status) => status,
                Err(error) => {
                    let status = handle.status();
                    self.state.lock().map_err(|_| SessionError::Worker)?.canvas =
                        canvas_state(status.canvas);
                    self.abort();
                    return Err(failure(error));
                }
            };
            canvas = canvas_state(actual.canvas);
            if actual.owner_retained
                || matches!(
                    actual.canvas,
                    producer::PaintDrawingCanvasState::RecoveryNeeded
                        | producer::PaintDrawingCanvasState::Unknown
                )
            {
                self.state.lock().map_err(|_| SessionError::Worker)?.canvas = canvas;
                self.abort();
                return Err(SessionError::RemoteRetirementPending);
            }
            if matches!(actual.canvas, producer::PaintDrawingCanvasState::Restored)
                && (!actual.process_and_job_retired
                    || !actual.io_joined
                    || actual.private_files_disposed != 10)
            {
                self.abort();
                return Err(SessionError::RemoteRetirementPending);
            }
        } else if failed
            && has_source
            && producer::retry_paint_observation_retirements().map_err(failure)? != 0
        {
            // A query-only failure before a drawing preparation exists still
            // belongs to the established source-held observation owner.
            self.abort();
            return Err(SessionError::RemoteRetirementPending);
        }
        if query_pending && producer::retry_paint_observation_retirements().map_err(failure)? != 0 {
            self.abort();
            return Err(SessionError::RemoteRetirementPending);
        }
        if self.gate.refused.load(Ordering::Acquire)
            && !self.gate.peer_ready.load(Ordering::Acquire)
        {
            if let Some(hub) = self.hub.upgrade() {
                let mut s = hub.state.lock().map_err(|_| SessionError::Worker)?;
                if s.drawing
                    .as_ref()
                    .is_some_and(|g| Arc::ptr_eq(g, &self.gate))
                {
                    s.drawing = None;
                    hub.publish(&mut s, "controlling", None)?;
                }
            }
        } else if self.gate.parked.load(Ordering::Acquire) && self.current(true).is_ok() {
            // Return the previous input helper only to the same foreground
            // source. A refused focus request seals this grant, without bypass.
            if vw_remote_host::platform::focus_for_owner_grant(
                &self.context.target,
                std::process::id(),
            )
            .is_err()
            {
                self.abort();
                self.state
                    .lock()
                    .map_err(|_| SessionError::Worker)?
                    .source
                    .take();
                self.state.lock().map_err(|_| SessionError::Worker)?.canvas = canvas;
                self.retired.store(true, Ordering::Release);
                owners()
                    .lock()
                    .map_err(|_| SessionError::Worker)?
                    .retain(|o| !o.retired.load(Ordering::Acquire));
                return Ok(canvas);
            }
            self.current(true)?;
            self.gate.release.store(true, Ordering::Release);
            let start = Instant::now();
            while !self.gate.resumed.load(Ordering::Acquire) {
                if self.gate.cancelled.load(Ordering::Acquire) {
                    break;
                }
                if start.elapsed() > Duration::from_millis(2500) {
                    self.abort();
                    return Err(SessionError::RemoteRetirementPending);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        } else {
            self.abort();
        }
        self.state
            .lock()
            .map_err(|_| SessionError::Worker)?
            .source
            .take();
        self.state.lock().map_err(|_| SessionError::Worker)?.canvas = canvas;
        self.retired.store(true, Ordering::Release);
        owners()
            .lock()
            .map_err(|_| SessionError::Worker)?
            .retain(|o| !o.retired.load(Ordering::Acquire));
        Ok(canvas)
    }
}
#[cfg(windows)]
fn canvas_state(value: producer::PaintDrawingCanvasState) -> RemoteDrawingCanvas {
    match value {
        producer::PaintDrawingCanvasState::Unchanged => RemoteDrawingCanvas::Unchanged,
        producer::PaintDrawingCanvasState::Restored => RemoteDrawingCanvas::Restored,
        producer::PaintDrawingCanvasState::RecoveryNeeded => RemoteDrawingCanvas::RecoveryNeeded,
        producer::PaintDrawingCanvasState::Unknown => RemoteDrawingCanvas::Unknown,
    }
}
#[cfg(windows)]
fn drawing_receipt(
    ctx: &ObservationContext,
    completed: &producer::CompletedPaintDrawingObservation,
) -> SessionResult<Vec<u8>> {
    let initial = compatibility::receipt(
        &ctx.identity,
        &ctx.profile_digest,
        completed.tool_id(),
        completed.settings_digest(),
    )?;
    let mut value: serde_json::Value =
        serde_json::from_slice(&initial).map_err(|_| SessionError::Invalid)?;
    let rows = value["observations"]
        .as_array_mut()
        .ok_or(SessionError::Invalid)?;
    let row = rows
        .iter_mut()
        .find(|r| r["aspect"] == "Drawing")
        .ok_or(SessionError::Invalid)?;
    *row = serde_json::json!({"aspect":"Drawing","result":"Supported","method":"EditorObserved","inputApi":"Mouse","evidenceSha256":completed.evidence_sha256(),"deviceModel":null});
    let bytes = serde_json::to_vec(&value).map_err(|_| SessionError::Invalid)?;
    if bytes.len() > 32 * 1024 {
        return Err(SessionError::Backpressure);
    }
    Ok(bytes)
}
#[uniffi::export]
impl RemoteDrawingPreparation {
    pub fn binding_json(&self) -> SessionResult<String> {
        #[cfg(windows)]
        {
            json(&self.owner.context.binding)
        }
        #[cfg(not(windows))]
        {
            Err(SessionError::RemoteUnavailable)
        }
    }
    pub fn identity_json(&self) -> SessionResult<String> {
        #[cfg(windows)]
        {
            json(&self.owner.context.identity)
        }
        #[cfg(not(windows))]
        {
            Err(SessionError::RemoteUnavailable)
        }
    }
    pub fn profile_digest(&self) -> SessionResult<String> {
        #[cfg(windows)]
        {
            Ok(self.owner.context.profile_digest.clone())
        }
        #[cfg(not(windows))]
        {
            Err(SessionError::RemoteUnavailable)
        }
    }
    pub fn prepared_ready(&self) -> bool {
        #[cfg(windows)]
        {
            self.owner
                .state
                .lock()
                .is_ok_and(|s| !s.failed && !s.preparing && s.prepared.is_some())
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
    pub fn canvas_state(&self) -> SessionResult<RemoteDrawingCanvas> {
        #[cfg(windows)]
        {
            let state = self.owner.state.lock().map_err(|_| SessionError::Worker)?;
            Ok(state
                .retirement_handle
                .as_ref()
                .map_or(state.canvas, |h| canvas_state(h.status().canvas)))
        }
        #[cfg(not(windows))]
        {
            Err(SessionError::RemoteUnavailable)
        }
    }
    pub fn run_confirmed(&self) -> SessionResult<Vec<u8>> {
        #[cfg(windows)]
        {
            self.owner.run()
        }
        #[cfg(not(windows))]
        {
            Err(SessionError::RemoteUnavailable)
        }
    }
    pub fn cancel(&self) {
        #[cfg(windows)]
        self.owner.signal_cancel();
    }
    pub fn retire(&self) -> SessionResult<RemoteDrawingCanvas> {
        #[cfg(windows)]
        {
            self.owner.retire()
        }
        #[cfg(not(windows))]
        {
            Err(SessionError::RemoteUnavailable)
        }
    }
}
impl Drop for RemoteDrawingPreparation {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            if !self.owner.retired.load(Ordering::Acquire) {
                self.owner.signal_cancel();
                self.owner.detached.store(true, Ordering::Release);
            }
        }
    }
}
#[uniffi::export]
impl LiveSession {
    pub fn remote_drawing_check_available(&self) -> bool {
        #[cfg(windows)]
        {
            self.remote.host
                && self.remote.state.lock().is_ok_and(|s| {
                    !s.closed
                        && s.drawing_enabled
                        && s.runtime.as_ref().is_some_and(|r| r.drawing.is_some())
                })
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
    pub fn remote_configure_drawing_helper(
        &self,
        path: String,
        sha256: String,
    ) -> SessionResult<()> {
        #[cfg(windows)]
        {
            if !self.remote.host
                || !std::path::Path::new(&path).is_absolute()
                || std::path::Path::new(&path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    != Some("vw-paint-drawing-helper.exe")
                || !vw_remote::profile::digest(&sha256)
            {
                return Err(SessionError::Invalid);
            }
            let mut s = self.remote.state.lock().map_err(|_| SessionError::Worker)?;
            if s.closed || s.drawing.is_some() {
                return Err(SessionError::Closed);
            }
            let runtime = s.runtime.as_mut().ok_or(SessionError::RemoteUnavailable)?;
            let (observation, observation_sha) = runtime
                .observation_files
                .clone()
                .ok_or(SessionError::RemoteUnavailable)?;
            runtime.drawing = Some(Arc::new(Files {
                observation,
                observation_sha,
                drawing: path,
                drawing_sha: sha256,
            }));
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = (path, sha256);
            Err(SessionError::RemoteUnavailable)
        }
    }
    pub fn remote_prepare_drawing_check(
        &self,
        binding_json: String,
    ) -> SessionResult<Arc<RemoteDrawingPreparation>> {
        #[cfg(windows)]
        {
            if !self.remote.host || self.closed.load(Ordering::Acquire) || busy(&self.remote)? {
                return Err(SessionError::RemoteRetirementPending);
            }
            let binding: Binding = parse(&binding_json, 2048)?;
            binding.validate().map_err(failure)?;
            let cancel = Arc::new(Cancellation::default());
            {
                let mut registered = self
                    .remote
                    .cancellations
                    .lock()
                    .map_err(|_| SessionError::Worker)?;
                registered.retain(|v| v.strong_count() != 0);
                if registered.len() >= 8 {
                    return Err(SessionError::Backpressure);
                }
                registered.push(Arc::downgrade(&cancel));
            }
            let owner = {
                let mut s = self.remote.state.lock().map_err(|_| SessionError::Worker)?;
                RemoteHub::current(&s)?;
                if !s.drawing_enabled
                    || s.drawing.is_some()
                    || s.admission.busy()
                    || s.binding.as_ref() != Some(&binding)
                    || s.host_retiring
                    || s.peer_retiring
                {
                    return Err(SessionError::Backpressure);
                }
                let runtime = s.runtime.as_ref().ok_or(SessionError::RemoteUnavailable)?;
                let files = runtime
                    .drawing
                    .clone()
                    .ok_or(SessionError::RemoteUnavailable)?;
                let authority = s
                    .authority
                    .as_ref()
                    .ok_or(SessionError::RemoteUnavailable)?;
                if authority.identity.executable_name != "mspaint.exe" {
                    return Err(SessionError::RemoteUnavailable);
                }
                let context = ObservationContext {
                    local_epoch: s.epoch,
                    source_epoch: s.source_epoch,
                    grant_revision: s.grant_revision,
                    binding: binding.clone(),
                    target: s
                        .selected
                        .as_ref()
                        .ok_or(SessionError::Closed)?
                        .target
                        .clone(),
                    identity: authority.identity.clone(),
                    profile_digest: authority.profile_digest.clone(),
                    helper: runtime
                        .observation
                        .clone()
                        .ok_or(SessionError::RemoteUnavailable)?,
                };
                s.next_command = s
                    .next_command
                    .checked_add(1)
                    .ok_or(SessionError::Backpressure)?;
                let gate = Arc::new(drawing_gate::Gate::new(s.next_command, binding.clone()));
                let worker_cancel = self
                    .remote
                    .workers
                    .lock()
                    .map_err(|_| SessionError::Worker)?
                    .drawing_input_cancel()?;
                let owner = Arc::new(Owner {
                    hub: Arc::downgrade(&self.remote),
                    context,
                    files,
                    gate: gate.clone(),
                    cancel,
                    worker_cancel,
                    step: AtomicU8::new(0),
                    detached: AtomicBool::new(false),
                    retired: AtomicBool::new(false),
                    state: Mutex::new(StateOwned {
                        preparing: true,
                        running: false,
                        prepared: None,
                        source: None,
                        failed: false,
                        canvas: RemoteDrawingCanvas::Unchanged,
                        retirement_handle: None,
                        query_pending: false,
                    }),
                    retirement: Mutex::new(()),
                });
                {
                    let mut all = owners().lock().map_err(|_| SessionError::Worker)?;
                    all.retain(|o| !o.retired.load(Ordering::Acquire));
                    if !all.is_empty() {
                        return Err(SessionError::Backpressure);
                    }
                    all.push(owner.clone());
                }
                s.drawing = Some(gate.clone());
                if let Err(error) = self.remote.drawing_control(
                    &mut s,
                    &binding,
                    "drawing_reserve",
                    gate.nonce,
                    0,
                    None,
                    "",
                ) {
                    owner
                        .state
                        .lock()
                        .map_err(|_| SessionError::Worker)?
                        .preparing = false;
                    owner.state.lock().map_err(|_| SessionError::Worker)?.failed = true;
                    owner.detached.store(true, Ordering::Release);
                    return Err(error);
                }
                if let Err(error) = self.remote.publish(&mut s, "controlling", None) {
                    let mut state = owner.state.lock().map_err(|_| SessionError::Worker)?;
                    state.preparing = false;
                    state.failed = true;
                    owner.detached.store(true, Ordering::Release);
                    return Err(error);
                }
                owner
            };
            // Even a failed preparation is delivered as its actual owner. The
            // gateway adopts it before reading ready=false and retiring it.
            let _ = owner.prepare();
            Ok(Arc::new(RemoteDrawingPreparation { owner }))
        }
        #[cfg(not(windows))]
        {
            let _ = binding_json;
            Err(SessionError::RemoteUnavailable)
        }
    }
}
