//! Additive retained-worker bridge for the approved Windows pyramid producer.
//! Paths/errors are never logged; a successful receipt is derived data only.
use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}};
use tokio::sync::Notify;
use vw_assets_win as assets;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum PyramidError {
    #[error("pyramid operation refused: {kind}")]
    Refused { kind: String },
}
type Result<T> = std::result::Result<T, PyramidError>;
fn failure(error: assets::Error) -> PyramidError { PyramidError::Refused { kind: format!("{error:?}") } }
fn refused(kind: &str) -> PyramidError { PyramidError::Refused { kind: kind.into() } }

#[derive(uniffi::Object)]
pub struct PyramidOperation { cancel: Arc<assets::Cancellation> }
#[uniffi::export]
impl PyramidOperation {
    #[uniffi::constructor]
    pub fn new() -> Self { Self { cancel: Arc::new(assets::Cancellation::default()) } }
    pub fn cancel(&self) { self.cancel.cancel(); }
}
impl Default for PyramidOperation { fn default() -> Self { Self::new() } }
#[derive(uniffi::Record)]
pub struct PyramidRequest {
    /// Exact native GiantInput.describe() fields; not a user-supplied overview.
    pub source_path: String, pub destination: String, pub source_asset_id: String,
    pub original_bytes: u64, pub width: u32, pub height: u32, pub orientation: u8,
    pub disk_budget: u64, pub timeout_ms: u64,
}
#[derive(uniffi::Record)]
pub struct PyramidReceipt {
    pub directory: String, pub source_asset_id: String, pub manifest_hash: String,
    pub tile_count: u32, pub encoded_bytes: u64, pub width: u32, pub height: u32,
}
static BUILDERS: AtomicUsize = AtomicUsize::new(0);
struct BuilderSlot;
impl BuilderSlot {
    fn acquire() -> Result<Self> {
        BUILDERS.try_update(Ordering::AcqRel,Ordering::Acquire,|value| (value < 2).then_some(value+1)).map_err(|_|refused("Busy"))?;
        Ok(Self)
    }
}
impl Drop for BuilderSlot { fn drop(&mut self) { BUILDERS.fetch_sub(1,Ordering::AcqRel); } }
#[derive(Default)]
struct State { closing: bool, active: Option<Arc<assets::Cancellation>> }
#[derive(Default)]
struct Lifetime {
    state: Mutex<State>, changed: Notify,
    // The actual worker's Permit retains this Lifetime. Dropping the public
    // builder AND its awaiting UniFFI future cannot credit a running builder.
    _builder_slot: Option<BuilderSlot>,
}
struct Permit(Arc<Lifetime>);
impl Drop for Permit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.lock() { state.active = None; }
        self.0.changed.notify_waiters();
    }
}
impl Lifetime {
    fn admitted() -> Result<Arc<Self>> {
        Ok(Arc::new(Self { _builder_slot: Some(BuilderSlot::acquire()?), ..Self::default() }))
    }
    fn begin(self: &Arc<Self>, cancel: Arc<assets::Cancellation>) -> Result<Permit> {
        cancel.check().map_err(failure)?;
        let mut state = self.state.lock().map_err(|_| refused("WorkerUnavailable"))?;
        if state.closing { return Err(refused("Closed")); }
        if state.active.is_some() { return Err(refused("Busy")); }
        state.active = Some(cancel); Ok(Permit(self.clone()))
    }
    fn seal(&self) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| refused("WorkerUnavailable"))?;
        state.closing = true;
        if let Some(cancel) = &state.active { cancel.cancel(); }
        Ok(())
    }
    async fn shutdown(&self) -> Result<()> {
        self.seal()?;
        loop {
            // Enable before the observed state read; notify_waiters does not
            // retain a permit for futures that have not registered yet.
            let notified = self.changed.notified();
            tokio::pin!(notified); notified.as_mut().enable();
            if self.state.lock().map_err(|_| refused("WorkerUnavailable"))?.active.is_none() { return Ok(()); }
            notified.await;
        }
    }
}
struct CancelOnDrop(Arc<assets::Cancellation>);
impl Drop for CancelOnDrop { fn drop(&mut self) { self.0.cancel(); } }
async fn owned<T: Send + 'static>(life: Arc<Lifetime>, cancel: Arc<assets::Cancellation>, work: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    let permit = life.begin(cancel.clone())?;
    let guard = CancelOnDrop(cancel);
    let (tx, rx) = tokio::sync::oneshot::channel();
    // The worker owns its slot and all captured file/runtime pins until work
    // returns and its captures are destroyed, even if the UniFFI future drops.
    std::thread::Builder::new().name("vw-pyramid-owner".into()).spawn(move || {
        let result = work(); drop(permit); let _ = tx.send(result);
    }).map_err(|_| refused("WorkerUnavailable"))?;
    let result = rx.await.map_err(|_| refused("WorkerUnavailable"))?;
    drop(guard); result
}
#[derive(uniffi::Object)]
pub struct PyramidBuilder {
    lifetime: Arc<Lifetime>,
    #[cfg(windows)] helper: Arc<assets::LockedHelper>,
    #[cfg(windows)] runtime: Arc<assets::LockedRuntime>,
}
impl Drop for PyramidBuilder { fn drop(&mut self) { let _ = self.lifetime.seal(); } }
fn local_path(value: &str) -> Result<std::path::PathBuf> {
    if value.is_empty() || value.len() > 32_760 || value.contains('\0') { return Err(refused("Invalid")); }
    let path = std::path::PathBuf::from(value);
    if !path.is_absolute() { return Err(refused("Invalid")); }
    #[cfg(windows)] {
        use std::path::{Component, Prefix};
        if !matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(),Prefix::Disk(_)|Prefix::VerbatimDisk(_))) {
            return Err(refused("Invalid"));
        }
    }
    Ok(path)
}
fn request(value: &PyramidRequest) -> Result<assets::BuildRequest> {
    local_path(&value.source_path)?; local_path(&value.destination)?;
    if value.source_asset_id.len()!=64 || !value.source_asset_id.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)) { return Err(refused("Invalid")); }
    let hash=blake3::Hash::from_hex(&value.source_asset_id).map_err(|_|refused("Invalid"))?;
    let request=assets::BuildRequest { original_hash:*hash.as_bytes(), original_bytes:value.original_bytes,
        width:value.width,height:value.height,orientation:value.orientation,disk_budget:value.disk_budget,timeout_ms:value.timeout_ms };
    request.validate().map_err(failure)?; Ok(request)
}
#[uniffi::export]
impl PyramidBuilder {
    pub async fn build(&self, value: PyramidRequest, operation: Arc<PyramidOperation>) -> Result<PyramidReceipt> {
        let request=request(&value)?;
        #[cfg(windows)] {
            let helper=self.helper.clone();let runtime=self.runtime.clone();let cancel=operation.cancel.clone();
            owned(self.lifetime.clone(),cancel.clone(),move || {
                let manifest=assets::build(&helper,&runtime,std::path::Path::new(&value.source_path),std::path::Path::new(&value.destination),&request,&cancel).map_err(failure)?;
                Ok(PyramidReceipt {directory:value.destination,source_asset_id:value.source_asset_id,
                    manifest_hash:blake3::hash(&manifest.encode()).to_hex().to_string(),
                    tile_count:u32::try_from(manifest.tiles().len()).map_err(|_|refused("Limit"))?,
                    encoded_bytes:manifest.encoded_bytes(),width:manifest.source().width,height:manifest.source().height})
            }).await
        }
        #[cfg(not(windows))] { let _=(request,operation);Err(refused("UnsupportedPlatform")) }
    }
    /// Seals/cancels admission and waits for actual worker/process termination.
    /// All simultaneous/cancelled Kotlin closers must await this before destroy.
    pub async fn shutdown(&self) -> Result<()> { self.lifetime.shutdown().await }
}
#[uniffi::export]
pub async fn create_pyramid_builder(helper_path: String, helper_sha256: String, runtime_directory: String, operation: Arc<PyramidOperation>) -> Result<Arc<PyramidBuilder>> {
    let helper_path=local_path(&helper_path)?;let runtime_directory=local_path(&runtime_directory)?;
    #[cfg(windows)] {
        let lifetime=Lifetime::admitted()?;let cancel=operation.cancel.clone();
        // Startup disk/hash work has the same retained lifetime and cancellation
        // guard as build. No DLL/codec is loaded in this application process.
        owned(lifetime.clone(),cancel.clone(),move || {
            cancel.check().map_err(failure)?;
            let helper=Arc::new(assets::LockedHelper::open(&helper_path,&helper_sha256).map_err(failure)?);
            let runtime=Arc::new(assets::LockedRuntime::open(&runtime_directory).map_err(failure)?);
            cancel.check().map_err(failure)?;
            Ok(Arc::new(PyramidBuilder{lifetime,helper,runtime}))
        }).await
    }
    #[cfg(not(windows))] { let _=(helper_path,helper_sha256,runtime_directory,operation);Err(refused("UnsupportedPlatform")) }
}
#[cfg(test)]
#[path="pyramid_tests.rs"]
mod tests;
