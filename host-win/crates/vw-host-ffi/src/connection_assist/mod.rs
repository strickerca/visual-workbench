//! Explicit Windows connection assistance. Import as a separate module beside
//! the reviewed clipboard/DPI service; it has no load-time OS side effects.
mod adb;
mod routes;
#[cfg(windows)]
mod windows;
#[cfg(not(windows))]
#[path = "unsupported.rs"]
mod windows;

pub use adb::{
    AdbDevice, AdbDeviceState, AdbReverseConfig, AdbReverseSnapshot, AdbReverseState,
    AdbReverseWatch, AdbToolInfo,
};
pub use routes::{
    MetricActionDetails, MetricActionKind, RouteFamily, RouteMetricAction, RouteMetricReceipt,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::oneshot;

type Result<T> = std::result::Result<T, ConnectionAssistError>;

/// Messages deliberately contain no executable path, serial, endpoint or OS
/// output. Callers may show explicit owner selections, but must not log them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum ConnectionAssistError {
    #[error("Windows connection assistance is unavailable on this platform")]
    Unsupported,
    #[error("The selected installed adb executable is unavailable or invalid")]
    InvalidAdbTool,
    #[error("Only Android Debug Bridge protocol 41 is supported")]
    UnsupportedAdb,
    #[error("The existing local adb server is unavailable; start it separately")]
    AdbServerUnavailable,
    #[error("The selected device or port is invalid")]
    InvalidSelection,
    #[error("The bounded adb reply was invalid")]
    AdbProtocol,
    #[error("The selected adb operation was refused")]
    AdbRefused,
    #[error("Another connection assistance operation is active")]
    Busy,
    #[error("The connection assistance request was cancelled")]
    Cancelled,
    #[error("The connection assistance deadline expired")]
    Timeout,
    #[error("This connection assistance request was already used")]
    AlreadyUsed,
    #[error("The routing table could not be read within its bound")]
    RouteUnavailable,
    #[error("This interface does not need a safe metric fix")]
    NoMetricFix,
    #[error("Routes or interface metrics changed; inspect them again")]
    StaleRoute,
    #[error("The packaged connection helper is unavailable")]
    HelperUnavailable,
    #[error("Administrator approval was declined")]
    ElevationDeclined,
    #[error("The metric change did not complete")]
    MetricFailed,
    #[error("The metric change outcome is uncertain; refresh routes before acting again")]
    MetricOutcomeUnknown,
    #[error("The bounded worker is unavailable")]
    WorkerUnavailable,
}

#[derive(uniffi::Object)]
pub struct ConnectionRequest {
    cancelled: AtomicBool,
    used: AtomicBool,
}
#[uniffi::export]
impl ConnectionRequest {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            cancelled: AtomicBool::new(false),
            used: AtomicBool::new(false),
        })
    }
    /// Cancellation is cooperative. An already-entered Windows metric setter
    /// may complete; such interruption is reported as an uncertain outcome.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
impl ConnectionRequest {
    fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(ConnectionAssistError::Cancelled)
        } else {
            Ok(())
        }
    }
    fn begin(&self) -> Result<()> {
        self.check()?;
        if self.used.swap(true, Ordering::AcqRel) {
            Err(ConnectionAssistError::AlreadyUsed)
        } else {
            Ok(())
        }
    }
}
struct CancelOnDrop(Arc<ConnectionRequest>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

// No unbounded spawning or queue. The watch has one separate admission slot.
static WORKERS: AtomicUsize = AtomicUsize::new(0);
struct WorkerPermit;
impl WorkerPermit {
    fn acquire() -> Result<Self> {
        WORKERS
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 3).then_some(n + 1)
            })
            .map(|_| Self)
            .map_err(|_| ConnectionAssistError::Busy)
    }
}
impl Drop for WorkerPermit {
    fn drop(&mut self) {
        WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}

async fn offload<T: Send + 'static>(
    request: Arc<ConnectionRequest>,
    work: impl FnOnce(&ConnectionRequest) -> Result<T> + Send + 'static,
) -> Result<T> {
    request.begin()?;
    let cancel = CancelOnDrop(request.clone());
    let permit = WorkerPermit::acquire()?;
    let (reply, received) = oneshot::channel();
    std::thread::Builder::new()
        .name("vw-connection-request".into())
        .spawn(move || {
            let result = request.check().and_then(|()| work(&request));
            // A late owned result is dropped here if the Kotlin future disappeared.
            let _ = reply.send(result);
            drop(permit);
        })
        .map_err(|_| ConnectionAssistError::WorkerUnavailable)?;
    let result = received
        .await
        .map_err(|_| ConnectionAssistError::WorkerUnavailable)?;
    drop(cancel);
    result
}

#[uniffi::export]
pub async fn inspect_adb_tool(
    path: String,
    request: Arc<ConnectionRequest>,
) -> Result<AdbToolInfo> {
    adb::validate_path(&path)?;
    offload(request, move |request| {
        windows::inspect_tool(&path, request).map(|tool| tool.info)
    })
    .await
}

#[uniffi::export]
pub async fn list_adb_devices(
    path: String,
    request: Arc<ConnectionRequest>,
) -> Result<Vec<AdbDevice>> {
    adb::validate_path(&path)?;
    offload(request, move |request| {
        let _tool = windows::inspect_tool(&path, request)?;
        adb::SmartSocket::new(request, std::time::Duration::from_millis(600))?.devices()
    })
    .await
}

#[uniffi::export]
pub async fn start_adb_reverse(
    config: AdbReverseConfig,
    request: Arc<ConnectionRequest>,
) -> Result<Arc<AdbReverseWatch>> {
    config.validate()?;
    offload(request, move |request| {
        let tool = windows::inspect_tool(&config.adb_path, request)?;
        request.check()?;
        adb::start(config, tool)
    })
    .await
}

#[uniffi::export]
pub async fn prepare_route_metric_fix(
    interface_index: u32,
    family: RouteFamily,
    request: Arc<ConnectionRequest>,
) -> Result<Arc<RouteMetricAction>> {
    if interface_index == 0 {
        return Err(ConnectionAssistError::InvalidSelection);
    }
    offload(request, move |request| {
        request.check()?;
        let snapshot = windows::read_routes()?;
        request.check()?;
        routes::prepare(&snapshot, interface_index, family).map(RouteMetricAction::new)
    })
    .await
}

/// Rust entry point for the separately packaged helper binary. Not a UniFFI
/// export and never called while loading the DLL.
#[doc(hidden)]
pub fn connection_assist_helper_main() -> i32 {
    windows::helper_main()
}
