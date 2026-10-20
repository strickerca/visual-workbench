//! Bounded native owners; neither peer messages nor imported data enter here.
use super::{
    contract::{Response, Work},
    *,
};
use std::{
    io::Write,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use worker::Worker;

static GENERATION: AtomicU64 = AtomicU64::new(1);

#[uniffi::export]
pub fn android_tunnel_available() -> bool {
    platform::available()
}

fn validate(options: &AndroidTunnelOptions) -> AndroidTunnelResult<[u8; 16]> {
    if !options.explicit_owner_action {
        return Err(AndroidTunnelError::Consent);
    }
    for path in [&options.runtime_directory, &options.owner_adb_path] {
        if path.is_empty() || path.len() > 4096 || path.contains('\0') {
            return Err(AndroidTunnelError::Invalid);
        }
    }
    if options.selected_model.is_empty()
        || options.selected_model.len() > 64
        || !options
            .selected_model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        || options.selected_transport_id == 0
    {
        return Err(AndroidTunnelError::Invalid);
    }
    vw_model::Id::try_from(options.session_id.clone())
        .map_err(|_| AndroidTunnelError::Invalid)?
        .to_proto()
        .value
        .try_into()
        .map_err(|_| AndroidTunnelError::Invalid)
}

struct Pending(Option<Arc<Worker>>);
impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(worker) = self.0.take() {
            worker.cancel();
        }
    }
}

#[derive(uniffi::Object)]
pub struct AndroidTunnelSession {
    worker: Arc<Worker>,
    session_id: String,
    generation: u64,
    frame: AtomicU64,
}
#[cfg(test)]
pub(super) fn synthetic_session(worker: Arc<Worker>) -> Arc<AndroidTunnelSession> {
    Arc::new(AndroidTunnelSession {
        worker,
        session_id: "019f0000-0000-7000-8000-000000000001".into(),
        generation: 1,
        frame: AtomicU64::new(1),
    })
}
impl Drop for AndroidTunnelSession {
    fn drop(&mut self) {
        self.worker.cancel();
    }
}

#[uniffi::export]
pub async fn open_android_tunnel(
    options: AndroidTunnelOptions,
    cancellation: Arc<crate::Cancellation>,
) -> AndroidTunnelResult<Arc<AndroidTunnelSession>> {
    let session = validate(&options)?;
    // Refuse before registry installation, filesystem access, adb or OS work.
    if !platform::available() {
        return Err(AndroidTunnelError::RuntimeUnavailable);
    }
    if cancellation.is_cancelled() {
        return Err(AndroidTunnelError::Cancelled);
    }
    let generation = GENERATION
        .try_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_add(1))
        .map_err(|_| AndroidTunnelError::Limit)?;
    let session_id = options.session_id.clone();
    let (factory, stop) = platform::factory(options, session, generation);
    let budget =
        crate::session_resources::shared_budget().map_err(|_| AndroidTunnelError::Limit)?;
    let worker = Worker::start(factory, stop, budget)?;
    let mut pending = Pending(Some(worker.clone()));
    let ready = tokio::select! {
        result=worker.ready()=>result,
        ()=async{while !cancellation.is_cancelled(){tokio::time::sleep(std::time::Duration::from_millis(10)).await;}}=>Err(AndroidTunnelError::Cancelled),
    };
    if let Err(error) = ready {
        worker.cancel();
        let _ = worker.close().await;
        return Err(error);
    }
    if cancellation.is_cancelled() {
        worker.cancel();
        worker.close().await?;
        return Err(AndroidTunnelError::Cancelled);
    }
    let result = Arc::new(AndroidTunnelSession {
        worker,
        session_id,
        generation,
        frame: AtomicU64::new(1),
    });
    pending.0 = None;
    Ok(result)
}

async fn read_state(worker: &Arc<Worker>, work: Work) -> AndroidTunnelResult<AndroidTunnelState> {
    let response = worker.exchange(work).await?;
    let state = match &response.value {
        Response::State(state) => Ok(state.clone()),
        _ => Err(AndroidTunnelError::Platform),
    };
    response.close();
    state
}

#[uniffi::export]
impl AndroidTunnelSession {
    pub async fn state(&self) -> AndroidTunnelResult<AndroidTunnelState> {
        read_state(&self.worker, Work::State).await
    }
    pub async fn begin_control_confirmation(&self) -> AndroidTunnelResult<AndroidTunnelTarget> {
        let response = self.worker.exchange(Work::BeginConfirmation).await?;
        let target = match &response.value {
            Response::Target(target) => Ok(target.clone()),
            _ => Err(AndroidTunnelError::Platform),
        };
        response.close();
        target
    }
    pub async fn confirm_control(
        &self,
        target: AndroidTunnelTarget,
    ) -> AndroidTunnelResult<AndroidTunnelState> {
        read_state(&self.worker, Work::Confirm(target)).await
    }
    pub async fn cancel_control_confirmation(&self) -> AndroidTunnelResult<()> {
        let response = self.worker.exchange(Work::CancelConfirmation).await?;
        let result = if matches!(&response.value, Response::Done) {
            Ok(())
        } else {
            Err(AndroidTunnelError::Platform)
        };
        response.close();
        result
    }
    pub async fn revoke_control(&self) -> AndroidTunnelResult<AndroidTunnelState> {
        let state = read_state(&self.worker, Work::RevokeControl).await?;
        if state.mode == AndroidTunnelMode::Closed {
            self.worker.close().await?;
        }
        Ok(state)
    }
    pub async fn capture(&self) -> AndroidTunnelResult<Arc<AndroidTunnelCapture>> {
        let response = self.worker.exchange(Work::Capture).await?;
        let frame = self
            .frame
            .try_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_add(1))
            .map_err(|_| AndroidTunnelError::Limit)?;
        let captured = match &response.value {
            Response::Captured(captured) => captured,
            _ => return Err(AndroidTunnelError::Platform),
        };
        let info = AndroidTunnelCaptureInfo {
            capture_session_id: self.session_id.clone(),
            source_generation: self.generation,
            frame_id: frame,
            width: captured.width,
            height: captured.height,
            encoded_bytes: captured.png.len() as u64,
            asset_id: blake3::hash(&captured.png).to_hex().to_string(),
            image_started_ms: captured.image_started_ms,
            image_finished_ms: captured.image_finished_ms,
            image_finished_timestamp_ns: captured.image_finished_timestamp_ns,
            hierarchy: captured.hierarchy.clone(),
            hierarchy_age_ms: captured.hierarchy_age_ms,
            hierarchy_started_ms: captured.hierarchy_started_ms,
            hierarchy_finished_ms: captured.hierarchy_finished_ms,
        };
        Ok(Arc::new(AndroidTunnelCapture {
            payload: Mutex::new(Some(Arc::new(Payload {
                response: Some(response),
                _worker: self.worker.clone(),
                info,
            }))),
        }))
    }
    pub fn cancel(&self) {
        self.worker.cancel();
    }
    pub async fn close_session(&self) -> AndroidTunnelResult<()> {
        self.worker.close().await
    }
}

struct Payload {
    response: Option<worker::OwnedResponse>,
    _worker: Arc<Worker>,
    info: AndroidTunnelCaptureInfo,
}
impl Payload {
    fn captured(&self) -> AndroidTunnelResult<&contract::Captured> {
        match &self
            .response
            .as_ref()
            .ok_or(AndroidTunnelError::Closed)?
            .value
        {
            Response::Captured(captured) => Ok(captured),
            _ => Err(AndroidTunnelError::Platform),
        }
    }
}
impl Drop for Payload {
    fn drop(&mut self) {
        if let Some(response) = self.response.take() {
            response.close();
        }
    }
}

#[derive(uniffi::Object)]
pub struct AndroidTunnelCapture {
    payload: Mutex<Option<Arc<Payload>>>,
}
impl AndroidTunnelCapture {
    fn acquire(&self) -> AndroidTunnelResult<Arc<Payload>> {
        self.payload
            .lock()
            .map_err(|_| AndroidTunnelError::Platform)?
            .clone()
            .ok_or(AndroidTunnelError::Closed)
    }
}
#[uniffi::export]
impl AndroidTunnelCapture {
    pub fn info(&self) -> AndroidTunnelResult<AndroidTunnelCaptureInfo> {
        Ok(self.acquire()?.info.clone())
    }
    pub fn elements(&self) -> AndroidTunnelResult<Vec<AndroidTunnelElement>> {
        Ok(self.acquire()?.captured()?.elements.clone())
    }
    pub fn close_capture(&self) -> AndroidTunnelResult<()> {
        self.payload
            .lock()
            .map_err(|_| AndroidTunnelError::Platform)?
            .take();
        Ok(())
    }
    pub async fn create_project(
        &self,
        mut options: crate::CreateFileImageProject,
        cancellation: Arc<crate::Cancellation>,
    ) -> crate::Result<Arc<crate::ProjectSession>> {
        let payload = self.acquire().map_err(core_error)?;
        let captured = payload.captured().map_err(core_error)?;
        if cancellation.is_cancelled() {
            return Err(crate::CoreError::Cancelled);
        }
        if options.now_ms
            < i64::try_from(captured.image_finished_ms).map_err(|_| crate::CoreError::Invalid)?
            || options.memory_budget_bytes == 0
            || options.memory_budget_bytes > 256 * 1024 * 1024
            || options.max_encoded_bytes < captured.png.len() as u64
            || options.max_scratch_bytes < captured.png.len() as u64
        {
            return Err(crate::CoreError::Invalid);
        }
        // One process pool covers the live producer, escaped capture and the
        // existing decoder workspace. Ownership moves into the accepted job.
        let charge = crate::session_resources::shared_budget()
            .map_err(|_| crate::CoreError::Backpressure)?
            .reserve(
                vw_stream::MemoryKind::CodecWorkspace,
                options.memory_budget_bytes,
            )
            .map_err(|_| crate::CoreError::Backpressure)?;
        let work = crate::streaming_ffi::work_directory(&options.work_directory)?;
        let mut scratch = tempfile::Builder::new()
            .prefix("vw-android-capture-")
            .tempfile_in(work)?;
        scratch.write_all(&captured.png)?;
        scratch.as_file().sync_all()?;
        options.source_path = scratch
            .path()
            .to_str()
            .ok_or(crate::CoreError::Invalid)?
            .to_owned();
        let info = &payload.info;
        let geometry_revision =
            u32::try_from(info.source_generation).map_err(|_| crate::CoreError::Invalid)?;
        let descriptor = crate::CaptureImportDescriptor {
            explicit_owner_action: true,
            capture_session_id: info.capture_session_id.clone(),
            frame_id: info.frame_id,
            geometry_revision,
            platform: "android".into(),
            source_kind: "monitor".into(),
            physical_x: 0,
            physical_y: 0,
            width: info.width,
            height: info.height,
            dpi_scale: 1.,
            monotonic_timestamp_ns: info.image_finished_timestamp_ns,
            captured_at_ms: i64::try_from(info.image_finished_ms)
                .map_err(|_| crate::CoreError::Invalid)?,
            window_handle: 0,
            monitor_id: String::new(),
            expected_source_asset_id: Some(info.asset_id.clone()),
        };
        let admitted = crate::capture_import::ValidatedCapture::new(descriptor)?;
        let ownership = Box::new((payload, scratch, charge));
        crate::streaming_ffi::create_owned_capture_project(
            options,
            cancellation,
            admitted,
            ownership,
        )
        .await
    }
}

fn core_error(error: AndroidTunnelError) -> crate::CoreError {
    match error {
        AndroidTunnelError::Cancelled => crate::CoreError::Cancelled,
        AndroidTunnelError::Closed => crate::CoreError::Closed,
        AndroidTunnelError::Limit | AndroidTunnelError::RetirementPending => {
            crate::CoreError::Backpressure
        }
        AndroidTunnelError::Storage => crate::CoreError::Storage,
        _ => crate::CoreError::Invalid,
    }
}

#[uniffi::export]
pub async fn retry_android_tunnel_retirement() -> AndroidTunnelResult<()> {
    Worker::retry_retirement().await
}
#[uniffi::export]
pub fn assert_android_tunnel_retired() -> AndroidTunnelResult<()> {
    if Worker::current().is_some() {
        Err(AndroidTunnelError::RetirementPending)
    } else {
        Ok(())
    }
}

/// A source guard owns the exact registry epoch and mirror revision. An entire
/// open/close between checks is detected; checking only current presence loses it.
#[derive(uniffi::Object)]
pub struct AndroidTunnelCaptureGuard {
    binding: Mutex<Option<GuardBinding>>,
}
struct GuardBinding {
    epoch: u64,
    worker: Option<Arc<Worker>>,
    revision: Option<u64>,
    source: AndroidTunnelSource,
}
#[uniffi::export]
pub fn begin_android_tunnel_capture_guard(
    source: AndroidTunnelSource,
) -> AndroidTunnelResult<Arc<AndroidTunnelCaptureGuard>> {
    let epoch = worker::registry_epoch();
    let worker = Worker::current();
    let revision = worker.as_ref().map(|w| w.mirror_revision()).transpose()?;
    let result = Arc::new(AndroidTunnelCaptureGuard {
        binding: Mutex::new(Some(GuardBinding {
            epoch,
            worker,
            revision,
            source,
        })),
    });
    result.check()?;
    Ok(result)
}
#[uniffi::export]
impl AndroidTunnelCaptureGuard {
    pub fn check(&self) -> AndroidTunnelResult<()> {
        let binding = self
            .binding
            .lock()
            .map_err(|_| AndroidTunnelError::Platform)?;
        let binding = binding.as_ref().ok_or(AndroidTunnelError::Closed)?;
        if worker::registry_epoch() != binding.epoch {
            return Err(AndroidTunnelError::Stale);
        }
        if let Some(owner) = &binding.worker {
            owner.check_source(
                binding.source.clone(),
                binding.revision.ok_or(AndroidTunnelError::Stale)?,
            )?;
        }
        if worker::registry_epoch() != binding.epoch {
            return Err(AndroidTunnelError::Stale);
        }
        Ok(())
    }
    pub fn close_guard(&self) {
        // Join a synchronous check before releasing its actual worker owner.
        // A poisoned lock remains fenced rather than pretending retirement.
        if let Ok(mut binding) = self.binding.lock() {
            binding.take();
        }
    }
}
