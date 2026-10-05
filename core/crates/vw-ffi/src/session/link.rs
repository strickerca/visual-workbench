use super::{
    SessionError, SessionResult, SessionService, device, endpoint, replication as sync,
    runtime::NetworkRuntime,
};
use crate::ProjectSession;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};
use vw_model::{AssetId, DeviceId, Id};
use vw_net::{
    CarrierIo, ConnectionSender, LocalHello, Receive, SecureConnection,
    carrier::{PinnedTls, QuicCarrier, QuicListener, TcpCarrier},
};
use vw_proto::v1::{self as pb, envelope::Body};

#[path = "focus.rs"]
mod focus;
pub use focus::{FocusSignal, PeerMarkerFocus};
#[path = "agent_capture.rs"]
mod agent_capture;
pub use agent_capture::AgentCaptureDisplay;
#[path = "remote_edit.rs"]
mod remote_edit;
pub use remote_edit::{
    RemoteAcknowledgment, RemoteCommandResult, RemoteDisplay, RemoteFrame, RemoteInputAdmitted,
    RemoteRuntimeFiles, RemoteWindow, remote_helpers_retirement_count,
};

pub(super) const MAX_TRANSFER: u64 = 64 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum AppCarrier {
    QuicTether,
    QuicWifi,
    TcpAdb,
}
impl AppCarrier {
    fn wire(self) -> pb::Carrier {
        match self {
            Self::QuicTether => pb::Carrier::QuicTether,
            Self::QuicWifi => pb::Carrier::QuicWifi,
            Self::TcpAdb => pb::Carrier::TcpAdb,
        }
    }
    fn rank(self) -> u8 {
        match self {
            Self::QuicTether => 0,
            Self::QuicWifi => 1,
            Self::TcpAdb => 2,
        }
    }
}
#[derive(Clone, uniffi::Record)]
pub struct SessionEndpoint {
    pub carrier: AppCarrier,
    pub address: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SyncStatus {
    Reconnecting,
    Syncing,
    Synced,
    Offline,
}
#[derive(Clone, uniffi::Record)]
pub struct SessionStatus {
    pub sequence: u64,
    pub status: SyncStatus,
    pub carrier: Option<AppCarrier>,
    pub pending: u32,
    pub blocked: u32,
    pub host_seq: u64,
    pub state_hash: String,
    pub peer_device_id: String,
    pub failure: Option<String>,
    pub peer_viewport: Option<PeerViewport>,
    pub echo_samples: u32,
    pub echo_rtt_p50_ms: Option<f64>,
    pub echo_rtt_p95_ms: Option<f64>,
    pub clock_offset_ms: Option<f64>,
}
#[derive(Clone, uniffi::Record)]
pub struct PeerViewport {
    pub document_id: String,
    pub corners: Vec<crate::Point>,
}
pub(super) enum Listener {
    Tcp(tokio::net::TcpListener, PinnedTls),
    Quic(QuicListener),
}
impl Listener {
    async fn accept(&self) -> SessionResult<CarrierIo> {
        match self {
            Self::Tcp(listener, tls) => TcpCarrier::accept(listener, tls).await.map(CarrierIo::Tcp),
            Self::Quic(listener) => listener.accept().await.map(CarrierIo::Quic),
        }
        .map_err(incoming_handshake_error)
    }
}
pub(super) enum Route {
    Host(Vec<(AppCarrier, Arc<Listener>)>),
    Client(Vec<(AppCarrier, SocketAddr)>),
}
struct LinkPermit(Arc<AtomicBool>);
impl Drop for LinkPermit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
/// Odd tokens admit commands; even tokens represent no live carrier. A command
/// captures its token before any bounded queue await and cannot cross reconnect.
struct ConnectionEpoch {
    owner: Arc<AtomicU64>,
    value: u64,
}
impl ConnectionEpoch {
    fn enter(owner: Arc<AtomicU64>) -> SessionResult<Self> {
        let prior = owner
            .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                (value & 1 == 0 && value < u64::MAX - 1).then(|| value + 1)
            })
            .map_err(|_| SessionError::Invalid)?;
        Ok(Self {
            owner,
            value: prior + 1,
        })
    }
}
impl Drop for ConnectionEpoch {
    fn drop(&mut self) {
        self.owner.store(self.value + 1, Ordering::Release);
    }
}
#[derive(uniffi::Object)]
pub struct LiveSession {
    runtime: NetworkRuntime,
    status: watch::Receiver<SessionStatus>,
    commands: mpsc::Sender<super::preview::Command>,
    epoch: Arc<AtomicU64>,
    endpoints: Vec<SessionEndpoint>,
    closed: AtomicBool,
    previews: Arc<Mutex<super::preview::Previews>>,
    project: Weak<ProjectSession>,
    local: DeviceId,
    focus: Arc<focus::FocusHub>,
    agent_capture: Arc<agent_capture::CaptureStatusHub>,
    remote: Arc<remote_edit::RemoteHub>,
}
impl Drop for LiveSession {
    fn drop(&mut self) {
        self.focus.stop();
        self.agent_capture.stop();
        self.remote.stop();
        self.runtime.stop();
    }
}
#[uniffi::export]
impl LiveSession {
    pub fn status(&self) -> SessionStatus {
        let mut status = self.status.borrow().clone();
        if self.closed.load(Ordering::Acquire) {
            status.status = SyncStatus::Offline;
            status.carrier = None;
        }
        status
    }
    pub fn endpoints(&self) -> Vec<SessionEndpoint> {
        self.endpoints.clone()
    }
    /// Admission is bounded. Preserve/retry the exact batch on Backpressure.
    /// first_sample is the number of real samples preceding this batch.
    pub fn stream_stroke(
        &self,
        stroke: Arc<crate::StrokeGesture>,
        batch: crate::SampleBatch,
        first_sample: u32,
    ) -> SessionResult<()> {
        let entered_epoch = self.epoch.load(Ordering::Acquire);
        if self.closed.load(Ordering::Acquire) {
            return Err(SessionError::Closed);
        }
        if !matches!(
            self.status.borrow().status,
            SyncStatus::Syncing | SyncStatus::Synced
        ) {
            return Ok(());
        }
        if batch.x.is_empty() || batch.x.len() > 512 {
            return Err(SessionError::Invalid);
        }
        let project = self.project.upgrade().ok_or(SessionError::Closed)?;
        let options = stroke.session_preview_options(&project)?;
        let Some(command) = super::preview::Command::capture(
            entered_epoch,
            super::preview::CommandKind::Stroke {
                options,
                batch,
                offset: first_sample,
            },
        ) else {
            return Ok(());
        };
        self.commands.try_send(command).map_err(command_error)
    }
    pub fn stream_object(&self, preview: super::preview::ObjectPreview) -> SessionResult<()> {
        let entered_epoch = self.epoch.load(Ordering::Acquire);
        if self.closed.load(Ordering::Acquire) {
            return Err(SessionError::Closed);
        }
        if !matches!(
            self.status.borrow().status,
            SyncStatus::Syncing | SyncStatus::Synced
        ) {
            return Ok(());
        }
        let Some(command) = super::preview::Command::capture(
            entered_epoch,
            super::preview::CommandKind::Object(preview),
        ) else {
            return Ok(());
        };
        self.commands.try_send(command).map_err(command_error)
    }
    pub async fn finish_preview(&self, gesture_id: String, cancel: bool) -> SessionResult<()> {
        let entered_epoch = self.epoch.load(Ordering::Acquire);
        if self.closed.load(Ordering::Acquire) {
            return Err(SessionError::Closed);
        }
        if !matches!(
            self.status.borrow().status,
            SyncStatus::Syncing | SyncStatus::Synced
        ) {
            return Ok(());
        }
        let gesture = Id::try_from(gesture_id).map_err(|_| SessionError::Invalid)?;
        let Some(command) = super::preview::Command::capture(
            entered_epoch,
            super::preview::CommandKind::Finish { gesture, cancel },
        ) else {
            return Ok(());
        };
        self.commands
            .send(command)
            .await
            .map_err(|_| SessionError::Closed)
    }
    /// New object identity and tool kind stay fixed. Geometry may evolve within
    /// that kind; bind the reliable Create transaction to the same gesture_id.
    pub fn stream_new_object(
        &self,
        preview: super::preview::NewObjectPreview,
    ) -> SessionResult<()> {
        let entered_epoch = self.epoch.load(Ordering::Acquire);
        if self.closed.load(Ordering::Acquire) {
            return Err(SessionError::Closed);
        }
        if !matches!(
            self.status.borrow().status,
            SyncStatus::Syncing | SyncStatus::Synced
        ) {
            return Ok(());
        }
        super::preview::validate_new_preview(&preview, &self.local)?;
        let Some(command) = super::preview::Command::capture(
            entered_epoch,
            super::preview::CommandKind::NewObject(preview),
        ) else {
            return Ok(());
        };
        self.commands.try_send(command).map_err(command_error)
    }
    pub async fn peer_previews(
        &self,
        document_id: String,
    ) -> SessionResult<super::preview::PeerPreviews> {
        let document = Id::try_from(document_id).map_err(|_| SessionError::Invalid)?;
        let project = self.project.upgrade().ok_or(SessionError::Closed)?;
        let (sequence, values) = {
            let preview = self.previews.lock().map_err(|_| SessionError::Worker)?;
            (
                preview.sequence,
                preview
                    .received
                    .iter()
                    .filter(|(_, p)| p.document == document)
                    .map(|(id, p)| (id.clone(), p.clone()))
                    .collect::<Vec<_>>(),
            )
        };
        sync::call(&project, move |state| {
            let mut items = Vec::new();
            let mut gesture_ids = Vec::new();
            let mut bytes = 0;
            for (id, preview) in values {
                let object_id = Id::from_proto(preview.state.object_id.as_ref())
                    .map_err(|_| SessionError::Invalid)?;
                if let Some(current) = state.project()?.objects.get(&object_id)
                    && ((preview.new_object)
                        || (current.state.transform == preview.state.transform
                            && current.state.style == preview.state.style))
                {
                    continue;
                }
                let item = crate::queries::preview_item(state, &preview.state, &document)?;
                crate::payload::accumulate(&mut bytes, crate::payload::item_bytes(&item)?)?;
                gesture_ids.push(id.to_string());
                items.push(item);
            }
            Ok(super::preview::PeerPreviews {
                sequence,
                gesture_ids,
                items,
            })
        })
        .await
    }
    /// Latest-value watch, with a bounded wait and no per-frame foreign callback
    /// allocation. Slow UIs skip obsolete status/viewport updates.
    pub async fn wait_status(&self, after_sequence: u64) -> SessionResult<SessionStatus> {
        if self.closed.load(Ordering::Acquire) {
            return Err(SessionError::Closed);
        }
        let mut status = self.status.clone();
        if status.borrow().sequence > after_sequence {
            return Ok(status.borrow().clone());
        }
        self.runtime
            .call(async move {
                match tokio::time::timeout(Duration::from_secs(30), status.changed()).await {
                    Ok(Ok(())) => Ok(status.borrow().clone()),
                    Ok(Err(_)) => Err(SessionError::Closed),
                    Err(_) => Ok(status.borrow().clone()),
                }
            })
            .await
    }
    pub fn send_viewport(
        &self,
        document_id: String,
        corners: Vec<crate::Point>,
    ) -> SessionResult<()> {
        let entered_epoch = self.epoch.load(Ordering::Acquire);
        if self.closed.load(Ordering::Acquire) {
            return Err(SessionError::Closed);
        }
        if !matches!(
            self.status.borrow().status,
            SyncStatus::Syncing | SyncStatus::Synced
        ) {
            return Ok(());
        }
        if corners.len() != 4 || corners.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
            return Err(SessionError::Invalid);
        }
        let id = Id::try_from(document_id).map_err(|_| SessionError::Invalid)?;
        let Some(command) = super::preview::Command::capture(
            entered_epoch,
            super::preview::CommandKind::Wire(Body::ViewportOutline(pb::ViewportOutline {
                document_id: Some(id.to_proto()),
                corners: corners
                    .into_iter()
                    .map(|p| pb::PointD { x: p.x, y: p.y })
                    .collect(),
            })),
        ) else {
            return Ok(());
        };
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => SessionError::Backpressure,
                mpsc::error::TrySendError::Closed(_) => SessionError::Closed,
            })
    }
    pub async fn close(&self) -> SessionResult<()> {
        self.closed.store(true, Ordering::Release);
        self.focus.stop();
        self.agent_capture.stop();
        // Seal and retire real producers before carrier/runtime close completes.
        // Typed pending keeps this foreign owner retryable and its leases held.
        self.remote.close()?;
        self.runtime.close().await
    }
}
fn command_error(error: mpsc::error::TrySendError<super::preview::Command>) -> SessionError {
    match error {
        mpsc::error::TrySendError::Full(_) => SessionError::Backpressure,
        mpsc::error::TrySendError::Closed(_) => SessionError::Closed,
    }
}
pub(super) fn hello(device: DeviceId) -> LocalHello {
    LocalHello {
        device,
        platform: if cfg!(target_os = "android") {
            pb::Platform::Android
        } else {
            pb::Platform::Windows
        },
        app_version: env!("CARGO_PKG_VERSION").into(),
        label: String::new(),
        capabilities: BTreeSet::from([
            "app_sync_v1".into(),
            "preview_lifecycle_v2".into(),
            "original_upload_v1".into(),
            "new_shape_preview_v1".into(),
            "marker_focus_v1".into(),
            agent_capture::CAPABILITY.into(),
            vw_remote::CAPABILITY.into(),
        ]),
    }
}
pub(super) fn routes(
    endpoints: Vec<SessionEndpoint>,
    bind: bool,
) -> SessionResult<Vec<(AppCarrier, SocketAddr)>> {
    if endpoints.is_empty() || endpoints.len() > 3 {
        return Err(SessionError::Invalid);
    }
    let mut result = Vec::new();
    let mut kinds = BTreeSet::new();
    for value in endpoints {
        let address = endpoint(&value.address, bind)?;
        if !kinds.insert(value.carrier.rank())
            || (value.carrier == AppCarrier::TcpAdb && !address.ip().is_loopback())
        {
            return Err(SessionError::Invalid);
        }
        result.push((value.carrier, address));
    }
    result.sort_by_key(|(carrier, _)| carrier.rank());
    Ok(result)
}
#[uniffi::export]
impl SessionService {
    pub async fn host_project(
        &self,
        project: Arc<ProjectSession>,
        peer_device_id: String,
        bind_endpoints: Vec<SessionEndpoint>,
    ) -> SessionResult<Arc<LiveSession>> {
        self.start_link(
            project,
            device(peer_device_id)?,
            routes(bind_endpoints, true)?,
            true,
        )
        .await
    }
    pub async fn connect_project(
        &self,
        project: Arc<ProjectSession>,
        peer_device_id: String,
        endpoints: Vec<SessionEndpoint>,
    ) -> SessionResult<Arc<LiveSession>> {
        self.start_link(
            project,
            device(peer_device_id)?,
            routes(endpoints, false)?,
            false,
        )
        .await
    }
}
impl SessionService {
    async fn start_link(
        &self,
        project: Arc<ProjectSession>,
        peer: DeviceId,
        addresses: Vec<(AppCarrier, SocketAddr)>,
        host: bool,
    ) -> SessionResult<Arc<LiveSession>> {
        let initial = sync::state(&project).await?;
        let identity = self.local_device().await?;
        if initial.local.as_str() != identity.device_id
            || ((initial.local == initial.host) != host)
            || (!host && initial.host != peer)
        {
            return Err(SessionError::Authentication);
        }
        project
            .network_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| SessionError::Backpressure)?;
        let permit = LinkPermit(project.network_active.clone());
        let manager = self.manager.clone();
        let local = initial.local.clone();
        let runtime = crate::worker::startup(|| Ok(NetworkRuntime::new())).await??;
        let owned = runtime.clone();
        runtime
            .call(async move {
                let mut public_endpoints = Vec::new();
                let route = if host {
                    let mut listeners = Vec::new();
                    for (carrier, address) in addresses {
                        let tls = manager.pinned_tls(&peer)?;
                        let listener = if carrier == AppCarrier::TcpAdb {
                            let listener = tokio::net::TcpListener::bind(address)
                                .await
                                .map_err(|_| SessionError::Transport)?;
                            public_endpoints.push(SessionEndpoint {
                                carrier,
                                address: listener
                                    .local_addr()
                                    .map_err(|_| SessionError::Transport)?
                                    .to_string(),
                            });
                            Listener::Tcp(listener, tls)
                        } else {
                            let listener = QuicListener::bind(address, tls)?;
                            public_endpoints.push(SessionEndpoint {
                                carrier,
                                address: listener.local_addr()?.to_string(),
                            });
                            Listener::Quic(listener)
                        };
                        listeners.push((carrier, Arc::new(listener)));
                    }
                    Route::Host(listeners)
                } else {
                    public_endpoints = addresses
                        .iter()
                        .map(|(carrier, address)| SessionEndpoint {
                            carrier: *carrier,
                            address: address.to_string(),
                        })
                        .collect();
                    Route::Client(addresses)
                };
                let status = SessionStatus {
                    sequence: 1,
                    status: SyncStatus::Reconnecting,
                    carrier: None,
                    pending: initial.pending as u32,
                    blocked: initial.blocked as u32,
                    host_seq: initial.revision.host_seq,
                    state_hash: hex(&initial.revision.state_hash),
                    peer_device_id: peer.to_string(),
                    failure: None,
                    peer_viewport: None,
                    echo_samples: 0,
                    echo_rtt_p50_ms: None,
                    echo_rtt_p95_ms: None,
                    clock_offset_ms: None,
                };
                let (changes, status) = watch::channel(status);
                let (commands, receive) = mpsc::channel(32);
                let epoch = Arc::new(AtomicU64::new(0));
                let incoming_epoch = epoch.clone();
                let focus = focus::FocusHub::new(initial.project.clone());
                let incoming_focus = focus.clone();
                let agent_capture = agent_capture::CaptureStatusHub::new(host);
                let incoming_capture = agent_capture.clone();
                let remote = remote_edit::RemoteHub::new(host);
                let incoming_remote = remote.clone();
                let previews = Arc::new(Mutex::new(super::preview::Previews::default()));
                let incoming_previews = previews.clone();
                let project_ref = Arc::downgrade(&project);
                let local_identity = local.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    drive(Driver {
                        project,
                        peer,
                        local,
                        manager,
                        route,
                        status: changes,
                        commands: receive,
                        epoch: incoming_epoch,
                        previews: incoming_previews,
                        focus: incoming_focus,
                        agent_capture: incoming_capture,
                        remote: incoming_remote,
                    })
                    .await;
                });
                Ok(Arc::new(LiveSession {
                    runtime: owned,
                    status,
                    commands,
                    epoch,
                    endpoints: public_endpoints,
                    closed: AtomicBool::new(false),
                    previews,
                    project: project_ref,
                    local: local_identity,
                    focus,
                    agent_capture,
                    remote,
                }))
            })
            .await
    }
}
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    out
}
fn publish(status: &watch::Sender<SessionStatus>, change: impl FnOnce(&mut SessionStatus)) {
    let mut next = status.borrow().clone();
    change(&mut next);
    next.sequence = next.sequence.saturating_add(1);
    status.send_replace(next);
}
pub(super) async fn establish(
    route: &Route,
    manager: &vw_net::pairing::PairingManager,
    peer: &DeviceId,
    local: &LocalHello,
) -> SessionResult<(AppCarrier, SecureConnection)> {
    match route {
        Route::Host(listeners) => {
            let mut accepts = tokio::task::JoinSet::new();
            for (carrier, listener) in listeners {
                let carrier = *carrier;
                let listener = listener.clone();
                let local = local.clone();
                accepts.spawn(async move {
                    Ok::<_, SessionError>((
                        carrier,
                        SecureConnection::server(listener.accept().await?, &local)
                            .await
                            .map_err(incoming_handshake_error)?,
                    ))
                });
            }
            let mut error = SessionError::Transport;
            while let Some(result) = accepts.join_next().await {
                match result {
                    Ok(Ok(connection)) => {
                        accepts.abort_all();
                        return Ok(connection);
                    }
                    Ok(Err(value)) => error = value,
                    Err(_) => error = SessionError::Worker,
                }
            }
            Err(error)
        }
        Route::Client(addresses) => {
            let mut error = SessionError::Transport;
            for (carrier, address) in addresses {
                let tls = manager.pinned_tls(peer)?;
                let io = if *carrier == AppCarrier::TcpAdb {
                    TcpCarrier::connect(*address, &tls)
                        .await
                        .map(CarrierIo::Tcp)
                } else {
                    let bind = SocketAddr::new(
                        if address.is_ipv4() {
                            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
                        } else {
                            IpAddr::V6(Ipv6Addr::UNSPECIFIED)
                        },
                        0,
                    );
                    QuicCarrier::connect(bind, *address, &tls)
                        .await
                        .map(CarrierIo::Quic)
                };
                match io {
                    Ok(io) => match SecureConnection::client(io, local).await {
                        Ok(connection) => return Ok((*carrier, connection)),
                        Err(value) => error = value.into(),
                    },
                    Err(value) => error = value.into(),
                }
            }
            Err(error)
        }
    }
}
async fn establish_for_project(
    driver: &Driver,
    local: &LocalHello,
    first_loss: Instant,
) -> SessionResult<(AppCarrier, SecureConnection)> {
    let attempt = establish(&driver.route, &driver.manager, &driver.peer, local);
    tokio::pin!(attempt);
    let deadline = tokio::time::Instant::from_std(first_loss + Duration::from_secs(2));
    let mut reported = false;
    loop {
        if driver.project.check_open().is_err() {
            return Err(SessionError::Closed);
        }
        tokio::select! {
            result=&mut attempt=>return result,
            _=driver.project.network_changed.notified()=>{if let Ok(current)=sync::state(&driver.project).await{publish(&driver.status,|s|{s.pending=current.pending as u32;s.blocked=current.blocked as u32;s.host_seq=current.revision.host_seq;s.state_hash=hex(&current.revision.state_hash);});}},
            _=tokio::time::sleep_until(deadline),if !reported=>{reported=true;publish(&driver.status,|s|{s.status=SyncStatus::Offline;s.carrier=None;});},
        }
    }
}
struct Driver {
    project: Arc<ProjectSession>,
    peer: DeviceId,
    local: DeviceId,
    manager: vw_net::pairing::PairingManager,
    route: Route,
    status: watch::Sender<SessionStatus>,
    commands: mpsc::Receiver<super::preview::Command>,
    epoch: Arc<AtomicU64>,
    previews: Arc<Mutex<super::preview::Previews>>,
    focus: Arc<focus::FocusHub>,
    agent_capture: Arc<agent_capture::CaptureStatusHub>,
    remote: Arc<remote_edit::RemoteHub>,
}
async fn drive(mut driver: Driver) {
    let local = hello(driver.local.clone());
    let mut first_loss = Instant::now();
    let mut delay = Duration::from_millis(250);
    let mut incoming = None;
    loop {
        if driver.project.check_open().is_err() {
            break;
        }
        let connection = establish_for_project(&driver, &local, first_loss).await;
        let Driver {
            project,
            peer,
            route,
            status,
            commands,
            epoch,
            previews,
            focus,
            agent_capture,
            remote,
            ..
        } = &mut driver;
        match connection {
            Ok((carrier, connection)) => {
                if [
                    "app_sync_v1",
                    "preview_lifecycle_v2",
                    "original_upload_v1",
                    "new_shape_preview_v1",
                    "marker_focus_v1",
                ]
                .iter()
                .any(|capability| !connection.session().capabilities().contains(*capability))
                {
                    publish(status, |s| {
                        s.status = SyncStatus::Offline;
                        s.failure = Some("upgrade_required".into());
                    });
                    break;
                }
                publish(status, |s| {
                    s.carrier = Some(carrier);
                    s.status = SyncStatus::Syncing;
                    s.failure = None;
                    s.peer_viewport = None;
                });
                delay = Duration::from_millis(250);
                match connected(
                    connection,
                    Connected {
                        project,
                        peer,
                        carrier,
                        status,
                        commands,
                        epoch,
                        host: matches!(route, Route::Host(_)),
                        incoming: &mut incoming,
                        previews,
                        focus,
                        agent_capture,
                        remote,
                    },
                )
                .await
                {
                    Ok(()) => break,
                    Err(error) => {
                        first_loss = Instant::now();
                        publish(status, |s| {
                            s.status = SyncStatus::Reconnecting;
                            s.carrier = None;
                            s.peer_viewport = None;
                            s.failure = Some(failure(&error).into());
                        });
                    }
                }
            }
            Err(error) => {
                if let Ok(current) = sync::state(project).await {
                    publish(status, |s| {
                        s.status = if first_loss.elapsed() >= Duration::from_secs(2) {
                            SyncStatus::Offline
                        } else {
                            SyncStatus::Reconnecting
                        };
                        s.carrier = None;
                        s.pending = current.pending as u32;
                        s.blocked = current.blocked as u32;
                        s.failure = Some(failure(&error).into());
                        s.peer_viewport = None;
                    });
                }
                if terminal_establish_failure(matches!(route, Route::Host(_)), &error) {
                    break;
                }
            }
        }
        // Volatile commands never survive a carrier boundary or get replayed.
        while commands.try_recv().is_ok() {}
        if let Ok(mut preview) = previews.lock() {
            preview.clear();
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(2));
    }
    driver.remote.stop();
    driver.focus.stop();
    publish(&driver.status, |s| {
        s.status = SyncStatus::Offline;
        s.carrier = None;
        s.peer_viewport = None;
    });
}
// This boundary contains incoming carrier setup and protocol Hello/HelloAck,
// not project filesystem operations. Preserve network Io as transport failure
// before the generic application conversion loses that provenance. Explicit
// NetError::Store and authentication errors keep their original categories.
fn incoming_handshake_error(error: vw_net::NetError) -> SessionError {
    match error {
        vw_net::NetError::Io(_) => SessionError::Transport,
        other => other.into(),
    }
}
// A rejected incoming peer is not ownership loss of the reusable host listener.
// Every retry still performs pinned TLS authentication; client refusal and local
// Closed/Storage terminal behavior remain unchanged, as does bounded backoff.
fn terminal_establish_failure(host: bool, error: &SessionError) -> bool {
    matches!(error, SessionError::Storage | SessionError::Closed)
        || (!host && matches!(error, SessionError::Authentication))
}
fn failure(error: &SessionError) -> &'static str {
    match error {
        SessionError::Authentication => "untrusted_or_revoked",
        SessionError::Storage => "storage",
        SessionError::Backpressure => "capacity",
        SessionError::Timeout => "timeout",
        SessionError::Invalid => "invalid_peer_message",
        SessionError::Closed => "closed",
        _ => "connection",
    }
}
struct Outbound {
    sender: vw_net::BlobSender<File>,
    held: Option<pb::BlobChunk>,
    _temporary: Option<tempfile::NamedTempFile>,
}
struct Incoming {
    asset: AssetId,
    size: u64,
    receiver: vw_net::BlobReceiver<File>,
    _temporary: tempfile::NamedTempFile,
    checkpoint: bool,
}
impl Incoming {
    fn new(
        asset: AssetId,
        size: u64,
        checkpoint: bool,
        temporary: tempfile::NamedTempFile,
    ) -> SessionResult<Self> {
        if size == 0 || size > MAX_TRANSFER {
            return Err(SessionError::Backpressure);
        }
        let receiver = vw_net::BlobReceiver::resume(
            temporary.reopen().map_err(|_| SessionError::Storage)?,
            asset.clone(),
            size,
            0,
            MAX_TRANSFER,
        )?;
        Ok(Self {
            asset,
            size,
            receiver,
            _temporary: temporary,
            checkpoint,
        })
    }
}
fn fill(sender: &ConnectionSender, outbound: &mut BTreeMap<String, Outbound>) -> SessionResult<()> {
    for transfer in outbound.values_mut() {
        loop {
            let next = if let Some(chunk) = transfer.held.take() {
                Some(chunk)
            } else {
                transfer.sender.next_chunk()?
            };
            let Some(chunk) = next else {
                break;
            };
            match sender.enqueue(Body::BlobChunk(chunk.clone())) {
                Ok(()) => {}
                Err(vw_net::NetError::Backpressure) => {
                    transfer.held = Some(chunk);
                    break;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}
struct Connected<'a> {
    project: &'a Arc<ProjectSession>,
    peer: &'a DeviceId,
    carrier: AppCarrier,
    status: &'a watch::Sender<SessionStatus>,
    commands: &'a mut mpsc::Receiver<super::preview::Command>,
    epoch: &'a Arc<AtomicU64>,
    host: bool,
    incoming: &'a mut Option<Incoming>,
    previews: &'a Arc<Mutex<super::preview::Previews>>,
    focus: &'a Arc<focus::FocusHub>,
    agent_capture: &'a Arc<agent_capture::CaptureStatusHub>,
    remote: &'a Arc<remote_edit::RemoteHub>,
}
async fn connected(connection: SecureConnection, context: Connected<'_>) -> SessionResult<()> {
    if !connection
        .session()
        .capabilities()
        .contains("preview_lifecycle_v2")
    {
        return Err(SessionError::Invalid);
    }
    let Connected {
        project,
        peer,
        carrier,
        status,
        commands,
        epoch,
        host,
        incoming,
        previews,
        focus,
        agent_capture,
        remote,
    } = context;
    let active_epoch = ConnectionEpoch::enter(epoch.clone())?;
    let _focus_epoch = focus.activate(active_epoch.value)?;
    let _capture_epoch = agent_capture.activate(
        active_epoch.value,
        connection
            .session()
            .capabilities()
            .contains(agent_capture::CAPABILITY),
    )?;
    let negotiated_remote = connection
        .session()
        .capabilities()
        .contains(vw_remote::CAPABILITY);
    let (sender, mut receive) = connection.into_duplex()?;
    let _closing = Closing(sender.clone());
    let start = Instant::now();
    let _remote_epoch = remote.activate(
        active_epoch.value,
        negotiated_remote && carrier == AppCarrier::QuicTether,
        sender.clone(),
        start,
    )?;
    let mut interval = tokio::time::interval(Duration::from_nanos(8_333_334));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut outbound = BTreeMap::<String, Outbound>::new();
    let mut completed_blobs = BTreeMap::<String, u64>::new();
    let mut checkpoint: Option<(AssetId, Vec<u8>)> = None;
    let mut sent = BTreeSet::<Id>::new();
    let mut rejected = BTreeSet::<Id>::new();
    let mut waiting = false;
    let mut complete = false;
    let mut missing = Vec::<(AssetId, u64)>::new();
    let mut previous_revision = None;
    let mut last_receive = Instant::now();
    let mut echo = Echo::new();
    let mut resumed = false;
    let mut peer_ready = None;
    let mut reported_ready = None;
    // One foreign transaction at a time keeps asset authorization exact and
    // bounded. The client retains it durably until its accepted journal arrives.
    let mut acquiring: Option<pb::Transaction> = None;
    if host {
        send_state(project, &sender, carrier).await?;
    } else {
        sender.enqueue(Body::SyncRequest(sync::request(project).await?))?;
        waiting = true;
    }
    loop {
        if project.check_open().is_err() {
            return Ok(());
        }
        tokio::select! {
            body=receive.receive(start.elapsed().as_millis().min(u128::from(u64::MAX))as u64)=>{
                let body=match body {Ok(value)=>value,
                    Err(vw_net::NetError::KeyframeRequired) if negotiated_remote=>{remote.recover()?;continue;},
                    Err(error)=>return Err(error.into())};
                let Receive::Deliver(body)=body else{continue;};last_receive=Instant::now();
                match body{
                    Body::Ping(ping)=>{let received=wall_ns()?;sender.enqueue(Body::Pong(pb::Pong{nonce:ping.nonce,t_sent_ns:ping.t_sent_ns,t_recv_ns:received,t_reply_ns:wall_ns()?}))?;},
                    Body::Pong(pong)=>echo.receive(pong,status)?,
                    Body::SessionState(state)=>{let current=sync::state(project).await?;if state.open_project_id.as_ref()!=Some(&current.project.to_proto()){return Err(SessionError::Authentication);}let revision=state.host_revision.ok_or(SessionError::Invalid)?;if host{if revision.host_seq>current.revision.host_seq||revision.state_hash.len()!=32{return Err(SessionError::Invalid);}peer_ready=if state.sync_complete&&state.pending_txns==0{Some(revision)}else{None};}else if !waiting&&incoming.as_ref().is_none_or(|v|!v.checkpoint)&&revision!=current.revision{sender.enqueue(Body::SyncRequest(sync::request(project).await?))?;waiting=true;complete=false;}},
                    Body::OpenProject(open)=>{let current=sync::state(project).await?;if !host||open.project_id.as_ref()!=Some(&current.project.to_proto())||open.create_if_missing{return Err(SessionError::Invalid);}send_state(project,&sender,carrier).await?;},
                    Body::SyncRequest(request)=>{if !host{return Err(SessionError::Authentication);}let response=sync::respond(project,request).await?;if let Some(bytes)=response.checkpoint{if bytes.len()as u64>MAX_TRANSFER{return Err(SessionError::Backpressure);}checkpoint=Some((AssetId::hash(&bytes),bytes));}sender.enqueue(Body::SyncBatch(response.batch))?;},
                    Body::Txn(txn)=>{
                        if !host{return Err(SessionError::Authentication);}
                        if let Some(current)=&acquiring { if current!=&txn { return Err(SessionError::Backpressure); } }
                        else { match sync::transaction_missing(project,txn.clone(),peer.clone()).await {
                            Ok(required)=>{missing=required;acquiring=Some(txn);if let Some(transfer)=incoming.as_ref(){if transfer.checkpoint||!missing.iter().any(|(id,size)|id==&transfer.asset&&*size==transfer.size){*incoming=None;}else{let offset=transfer.receiver.offset();sender.enqueue(Body::BlobRequest(pb::BlobRequest{asset_id:transfer.asset.to_string(),offset,length:transfer.size-offset,priority:1}))?;missing.retain(|(id,_)|id!=&transfer.asset);}}},
                            Err(SessionError::Invalid)=>sender.enqueue(Body::TxnReject(pb::TxnReject{txn_id:txn.txn_id,code:"invalid_or_conflicting".into(),message:String::new()}))?,
                            Err(error)=>return Err(error),
                        }}
                    },
                    Body::TxnAck(_)=>{if host{return Err(SessionError::Authentication);}
                        if !waiting&&incoming.as_ref().is_none_or(|v|!v.checkpoint){sender.enqueue(Body::SyncRequest(sync::request(project).await?))?;waiting=true;}},
                    Body::TxnReject(reject)=>{if host{return Err(SessionError::Authentication);}let id=Id::from_proto(reject.txn_id.as_ref()).map_err(|_|SessionError::Invalid)?;sent.remove(&id);rejected.insert(id);complete=false;if !waiting&&incoming.as_ref().is_none_or(|v|!v.checkpoint){sender.enqueue(Body::SyncRequest(sync::request(project).await?))?;waiting=true;}},
                    Body::SyncBatch(batch)=>{
                        if host||!waiting{return Err(SessionError::Invalid);}waiting=false;complete=batch.complete;
                        let checkpoint_id=if batch.checkpoint_asset_id.is_empty(){None}else{Some(AssetId::try_from(batch.checkpoint_asset_id.clone()).map_err(|_|SessionError::Invalid)?)};let size=batch.checkpoint_size;
                        let gestures=batch.txns.iter().filter_map(|entry|entry.txn.as_ref()).map(super::preview::AcceptedGesture::from_transaction).collect::<SessionResult<Vec<_>>>()?;
                        let installed=sync::receive(project,batch,peer.clone()).await?;if installed{let local=sync::state(project).await?.local;for receipt in gestures.into_iter().flatten(){let closes=previews.lock().map_err(|_|SessionError::Worker)?.finish_accepted(receipt,&local)?;for close in closes{sender.enqueue(close)?;}}}
                        if let Some(asset)=checkpoint_id{if incoming.as_ref().is_none_or(|v|!v.checkpoint||v.asset!=asset||v.size!=size){*incoming=Some(Incoming::new(asset.clone(),size,true,sync::transfer_file(project).await?)?);}let offset=incoming.as_ref().ok_or(SessionError::Invalid)?.receiver.offset();sender.enqueue(Body::BlobRequest(pb::BlobRequest{asset_id:asset.to_string(),offset,length:size-offset,priority:0}))?;resumed=true;complete=false;}
                        else if installed{missing=sync::missing_assets(project).await?;if !resumed&&let Some(transfer)=incoming.as_ref(){if transfer.checkpoint||!missing.iter().any(|(id,size)|id==&transfer.asset&&*size==transfer.size){*incoming=None;}else{let offset=transfer.receiver.offset();sender.enqueue(Body::BlobRequest(pb::BlobRequest{asset_id:transfer.asset.to_string(),offset,length:transfer.size-offset,priority:1}))?;missing.retain(|(id,_)|id!=&transfer.asset);resumed=true;}}
                            if let Some(transfer)=incoming.as_ref(){missing.retain(|(id,_)|id!=&transfer.asset);}
                            if !complete{sender.enqueue(Body::SyncRequest(sync::request(project).await?))?;waiting=true;}}
                    },
                    Body::BlobRequest(request)=>{
                        if outbound.len()>=2||request.length==0||request.length>MAX_TRANSFER{return Err(SessionError::Backpressure);}let asset=AssetId::try_from(request.asset_id.clone()).map_err(|_|SessionError::Invalid)?;
                        let(file,size,temporary)=if let Some((id,bytes))=&checkpoint&&*id==asset{let mut temp=sync::transfer_file(project).await?;temp.write_all(bytes).map_err(|_|SessionError::Storage)?;(temp.reopen().map_err(|_|SessionError::Storage)?,bytes.len()as u64,Some(temp))}else{let(path,size)=sync::asset_path(project,asset.clone()).await?;(File::open(path).map_err(|_|SessionError::Storage)?,size,None)};
                        if request.length!=size.checked_sub(request.offset).ok_or(SessionError::Invalid)?{return Err(SessionError::Invalid);}let transfer=vw_net::BlobSender::resume(file,asset,size,request.offset)?;
                        if outbound.insert(request.asset_id,Outbound{sender:transfer,held:None,_temporary:temporary}).is_some(){return Err(SessionError::Invalid);}fill(&sender,&mut outbound)?;
                    },
                    Body::BlobAck(ack)=>{if let Some(transfer)=outbound.get_mut(&ack.asset_id){transfer.sender.acknowledge(&ack)?;if transfer.sender.is_verified(){if completed_blobs.len()>=4096&&!completed_blobs.contains_key(&ack.asset_id){return Err(SessionError::Backpressure);}completed_blobs.insert(ack.asset_id.clone(),ack.next_offset);outbound.remove(&ack.asset_id);}fill(&sender,&mut outbound)?;}else if completed_blobs.get(&ack.asset_id).is_none_or(|size|ack.next_offset>*size||(ack.verified&&ack.next_offset!=*size)){return Err(SessionError::Invalid);}},
                    Body::BlobChunk(chunk)=>{
                        let transfer=incoming.as_mut().ok_or(SessionError::Invalid)?;let ack=transfer.receiver.receive(&chunk)?;
                        if ack.verified{let transfer=incoming.take().ok_or(SessionError::Invalid)?;let mut file=transfer.receiver.finish()?;file.sync_all().map_err(|_|SessionError::Storage)?;
                            if transfer.checkpoint{file.seek(SeekFrom::Start(0)).map_err(|_|SessionError::Storage)?;let mut bytes=Vec::new();bytes.try_reserve_exact(transfer.size as usize).map_err(|_|SessionError::Backpressure)?;file.take(transfer.size+1).read_to_end(&mut bytes).map_err(|_|SessionError::Storage)?;if bytes.len()as u64!=transfer.size{return Err(SessionError::Invalid);}sync::checkpoint(project,bytes,peer.clone()).await?;missing=sync::missing_assets(project).await?;sender.enqueue(Body::SyncRequest(sync::request(project).await?))?;waiting=true;}
                            else if host { sync::install_transaction_asset(project,acquiring.as_ref().ok_or(SessionError::Invalid)?.clone(),peer.clone(),transfer.asset,file,transfer.size).await?; }
                            else{sync::install_asset(project,transfer.asset,file,transfer.size).await?;}
                        }sender.enqueue(Body::BlobAck(ack))?;
                    },
                    Body::ViewportOutline(view)=>{if view.corners.len()!=4||view.corners.iter().any(|p|!p.x.is_finite()||!p.y.is_finite()){return Err(SessionError::Invalid);}let id=Id::from_proto(view.document_id.as_ref()).map_err(|_|SessionError::Invalid)?;publish(status,|s|s.peer_viewport=Some(PeerViewport{document_id:id.to_string(),corners:view.corners.into_iter().map(|p|crate::Point{x:p.x,y:p.y}).collect()}));},
                    Body::MarkerFocus(value)=>focus.receive(value,active_epoch.value)?,
                    Body::AgentCaptureStatus(value)=>agent_capture.receive(value,active_epoch.value)?,
                    body @ (Body::RemoteControl(_)|Body::RemoteVideoConfig(_))=>remote.receive(body,active_epoch.value)?,
                    Body::VideoFrame(value) if value.remote_scope.is_some()=>remote.receive(Body::VideoFrame(value),active_epoch.value)?,
                    Body::InputEvent(value) if value.remote_scope.is_some()=>remote.receive(Body::InputEvent(value),active_epoch.value)?,
                    Body::InputStatus(value) if value.remote_scope.is_some()=>remote.receive(Body::InputStatus(value),active_epoch.value)?,
                    Body::GestureUpdate(update)=>receive_preview(project,peer,&sender,previews,*update,false,false).await?,
                    Body::GestureReplay(replay)=>receive_preview(project,peer,&sender,previews,replay.update.ok_or(SessionError::Invalid)?,true,replay.open).await?,
                    Body::GestureRepair(request)=>{let reply=previews.lock().map_err(|_|SessionError::Worker)?.repair(request)?;if let Some(reply)=reply{sender.enqueue(reply)?;}},
                    // EPHEMERAL cancellation has no reliable ordering authority.
                    Body::GestureCancel(_)=>{},
                    Body::GestureAbort(close)=>{let id=Id::from_proto(close.gesture_id.as_ref()).map_err(|_|SessionError::Invalid)?;let first=previews.lock().map_err(|_|SessionError::Worker)?.retire(&close)?;if first&&host&&!close.committed{sync::cancel_gesture(project,peer.clone(),id).await?;}},
                    _=>return Err(SessionError::Invalid),
                }
            },
            command=commands.recv()=>{let Some(command)=command else{return Ok(());};if !command.belongs_to(active_epoch.value){continue;}match command.kind{
                super::preview::CommandKind::Wire(body)=>sender.enqueue(body)?,
                super::preview::CommandKind::Stroke{options,batch,offset}=>{let local=sync::state(project).await?.local;let gesture=Id::try_from(options.gesture_id.clone()).map_err(|_|SessionError::Invalid)?;if !sync::gesture_closed(project,local.clone(),gesture).await?{previews.lock().map_err(|_|SessionError::Worker)?.record(options,batch,offset,&local)?;}},
                super::preview::CommandKind::Object(value)=>{let local=sync::state(project).await?.local;let gesture=Id::try_from(value.gesture_id.clone()).map_err(|_|SessionError::Invalid)?;if !sync::gesture_closed(project,local,gesture).await?{previews.lock().map_err(|_|SessionError::Worker)?.record_object(value)?;}},
                super::preview::CommandKind::NewObject(value)=>{let local=sync::state(project).await?.local;let gesture=Id::try_from(value.gesture_id.clone()).map_err(|_|SessionError::Invalid)?;if !sync::gesture_closed(project,local.clone(),gesture).await?{previews.lock().map_err(|_|SessionError::Worker)?.record_new_object(value,&local)?;}},
                super::preview::CommandKind::Finish{gesture,cancel}=>{let closes=previews.lock().map_err(|_|SessionError::Worker)?.finish(gesture.clone(),cancel)?;if host&&cancel{sync::cancel_gesture(project,sync::state(project).await?.local,gesture).await?;}for close in closes{sender.enqueue(close)?;}},
            }},
            _=project.network_changed.notified()=>{},
            _=interval.tick()=>{},
        }
        if last_receive.elapsed() > Duration::from_secs(15) {
            return Err(SessionError::Timeout);
        }
        echo.tick(&sender)?;
        focus.flush(&sender)?;
        agent_capture.flush(&sender)?;
        remote.flush()?;
        fill(&sender, &mut outbound)?;
        let updates = {
            let mut previews = previews.lock().map_err(|_| SessionError::Worker)?;
            previews.expire();
            previews.updates()?
        };
        for update in updates {
            sender.enqueue(update)?;
        }
        if host
            && incoming.is_none()
            && missing.is_empty()
            && let Some(txn) = acquiring.take()
        {
            let id = txn.txn_id.clone();
            let gesture = txn.gesture_id.clone();
            match sync::submit(project, txn, peer.clone()).await {
                Ok(accepted) => {
                    if let Some(gesture) = gesture {
                        previews
                            .lock()
                            .map_err(|_| SessionError::Worker)?
                            .finish_remote(
                                Id::from_proto(Some(&gesture))
                                    .map_err(|_| SessionError::Invalid)?,
                            )?;
                    }
                    sender.enqueue(Body::TxnAck(accepted.ack))?;
                    send_state(project, &sender, carrier).await?;
                }
                Err(SessionError::Invalid) => sender.enqueue(Body::TxnReject(pb::TxnReject {
                    txn_id: id,
                    code: "invalid_or_conflicting".into(),
                    message: String::new(),
                }))?,
                Err(error) => return Err(error),
            }
        }
        let current = sync::state(project).await?;
        if previous_revision.as_ref() != Some(&current.revision) {
            rejected.clear();
            if host {
                send_state(project, &sender, carrier).await?;
            }
            previous_revision = Some(current.revision.clone());
        }
        if !host {
            let pending = sync::pending(project).await?;
            let current_ids = pending
                .iter()
                .map(|t| Id::from_proto(t.txn_id.as_ref()).map_err(|_| SessionError::Invalid))
                .collect::<SessionResult<BTreeSet<_>>>()?;
            sent.retain(|id| current_ids.contains(id));
            for txn in pending {
                if !sent.is_empty() {
                    break;
                }
                let id = Id::from_proto(txn.txn_id.as_ref()).map_err(|_| SessionError::Invalid)?;
                if !sent.contains(&id) && !rejected.contains(&id) {
                    match sender.enqueue(Body::Txn(txn)) {
                        Ok(()) => {
                            sent.insert(id);
                        }
                        Err(vw_net::NetError::Backpressure) => break,
                        Err(error) => return Err(error.into()),
                    }
                }
            }
        }
        if incoming.is_none()
            && let Some((asset, size)) = missing.pop()
        {
            *incoming = Some(Incoming::new(
                asset.clone(),
                size,
                false,
                sync::transfer_file(project).await?,
            )?);
            sender.enqueue(Body::BlobRequest(pb::BlobRequest {
                asset_id: asset.to_string(),
                offset: 0,
                length: size,
                priority: 1,
            }))?;
            resumed = true;
        }
        let ready = current.pending == 0
            && incoming.is_none()
            && missing.is_empty()
            && complete
            && !waiting;
        if !host
            && reported_ready.as_ref() != Some(&(current.revision.clone(), ready, current.pending))
        {
            sender.enqueue(Body::SessionState(pb::SessionState {
                carrier: carrier.wire() as i32,
                connected_since_ms: sync::now()?,
                open_project_id: Some(current.project.to_proto()),
                host_revision: Some(current.revision.clone()),
                sync_complete: ready,
                pending_txns: current.pending as u32,
            }))?;
            reported_ready = Some((current.revision.clone(), ready, current.pending));
        }
        publish(status, |s| {
            s.pending = current.pending as u32;
            s.blocked = current.blocked as u32;
            s.host_seq = current.revision.host_seq;
            s.state_hash = hex(&current.revision.state_hash);
            s.status = if current.pending == 0
                && incoming.is_none()
                && missing.is_empty()
                && (if host {
                    peer_ready.as_ref() == Some(&current.revision)
                        && outbound.is_empty()
                        && acquiring.is_none()
                } else {
                    complete && !waiting
                }) {
                SyncStatus::Synced
            } else {
                SyncStatus::Syncing
            };
        });
    }
}
async fn receive_preview(
    project: &ProjectSession,
    peer: &DeviceId,
    sender: &ConnectionSender,
    previews: &Arc<Mutex<super::preview::Previews>>,
    update: pb::GestureUpdate,
    reliable: bool,
    opening: bool,
) -> SessionResult<()> {
    let gesture = Id::from_proto(update.gesture_id.as_ref()).map_err(|_| SessionError::Invalid)?;
    let closed = sync::gesture_closed(project, peer.clone(), gesture.clone()).await?;
    if opening {
        if !reliable {
            return Err(SessionError::Invalid);
        }
        if !previews
            .lock()
            .map_err(|_| SessionError::Worker)?
            .open(&update, closed)?
        {
            return Ok(());
        }
    }
    if closed {
        previews
            .lock()
            .map_err(|_| SessionError::Worker)?
            .finish_remote(gesture)?;
        return Ok(());
    }
    if !previews
        .lock()
        .map_err(|_| SessionError::Worker)?
        .admitted(&update)?
    {
        return Ok(());
    }
    if update.kind == pb::GestureKind::Stroke as i32 {
        let repair = previews
            .lock()
            .map_err(|_| SessionError::Worker)?
            .update(update, peer, reliable)?;
        if let Some(repair) = repair {
            sender.enqueue(repair)?;
        }
    } else {
        if reliable && !opening {
            return Err(SessionError::Invalid);
        }
        let id =
            Id::from_proto(update.target_object_id.as_ref()).map_err(|_| SessionError::Invalid)?;
        let document =
            Id::from_proto(update.document_id.as_ref()).map_err(|_| SessionError::Invalid)?;
        let expected = document.clone();
        let template = update.preview_state.clone();
        let new_object = template.is_some();
        if new_object && update.kind != pb::GestureKind::Shape as i32 {
            return Err(SessionError::Invalid);
        }
        let author = peer.clone();
        let object = sync::call(project, move |state| {
            if let Some(template) = template {
                vw_model::validate_object_state(&template).map_err(|_| SessionError::Invalid)?;
                let project = state.project()?;
                let layer = Id::from_proto(template.layer_id.as_ref())
                    .map_err(|_| SessionError::Invalid)?;
                if template.object_id.as_ref() != Some(&id.to_proto())
                    || template.created_by != author.as_str()
                    || template.locked
                    || project.objects.contains_key(&id)
                    || project.layers.get(&layer).is_none_or(|layer| {
                        layer.definition.document_id.as_ref() != Some(&expected.to_proto())
                            || layer.locked
                            || !layer.visible
                    })
                    || !matches!(
                        template.shape,
                        Some(
                            pb::object_state::Shape::Line(_)
                                | pb::object_state::Shape::Arrow(_)
                                | pb::object_state::Shape::Rect(_)
                                | pb::object_state::Shape::Ellipse(_)
                                | pb::object_state::Shape::Text(_)
                        )
                    )
                {
                    return Err(SessionError::Invalid);
                }
                return Ok(template);
            }
            let object = state
                .project()?
                .objects
                .get(&id)
                .ok_or(SessionError::Invalid)?;
            if object.document_id != expected
                || object.state.locked
                || state
                    .project()?
                    .layers
                    .get(
                        &Id::from_proto(object.state.layer_id.as_ref())
                            .map_err(|_| SessionError::Invalid)?,
                    )
                    .is_none_or(|layer| layer.locked || !layer.visible)
            {
                return Err(SessionError::Invalid);
            }
            Ok(object.state.clone())
        })
        .await?;
        previews
            .lock()
            .map_err(|_| SessionError::Worker)?
            .object(update, object, document, new_object)?;
    }
    Ok(())
}
struct Closing(ConnectionSender);
impl Drop for Closing {
    fn drop(&mut self) {
        self.0.revoke_input();
        self.0.close();
    }
}
async fn send_state(
    project: &ProjectSession,
    sender: &ConnectionSender,
    carrier: AppCarrier,
) -> SessionResult<()> {
    let state = sync::state(project).await?;
    sender.enqueue(Body::SessionState(pb::SessionState {
        carrier: carrier.wire() as i32,
        connected_since_ms: sync::now()?,
        open_project_id: Some(state.project.to_proto()),
        host_revision: Some(state.revision),
        ..Default::default()
    }))?;
    Ok(())
}
fn wall_ns() -> SessionResult<i64> {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| SessionError::Storage)?
            .as_nanos(),
    )
    .map_err(|_| SessionError::Storage)
}
struct Echo {
    last: Instant,
    next: u64,
    pending: Option<(u64, i64, Instant)>,
    samples: Vec<f64>,
    offsets: Vec<f64>,
}
impl Echo {
    fn new() -> Self {
        Self {
            last: Instant::now() - Duration::from_secs(1),
            next: 1,
            pending: None,
            samples: Vec::new(),
            offsets: Vec::new(),
        }
    }
    fn tick(&mut self, sender: &ConnectionSender) -> SessionResult<()> {
        if self.last.elapsed() < Duration::from_millis(500) {
            return Ok(());
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|(_, _, time)| time.elapsed() < Duration::from_secs(2))
        {
            return Ok(());
        }
        let sent = wall_ns()?;
        let nonce = self.next;
        self.next = self.next.checked_add(1).ok_or(SessionError::Invalid)?;
        sender.enqueue(Body::Ping(pb::Ping {
            nonce,
            t_sent_ns: sent,
        }))?;
        self.last = Instant::now();
        self.pending = Some((nonce, sent, self.last));
        Ok(())
    }
    fn receive(
        &mut self,
        pong: pb::Pong,
        status: &watch::Sender<SessionStatus>,
    ) -> SessionResult<()> {
        let Some((nonce, sent, instant)) = self.pending else {
            return Ok(());
        };
        if pong.nonce != nonce {
            return Ok(());
        }
        if pong.t_sent_ns != sent || pong.t_reply_ns < pong.t_recv_ns {
            return Err(SessionError::Invalid);
        }
        self.pending = None;
        let received = wall_ns()?;
        let elapsed = instant.elapsed().as_secs_f64() * 1000.0;
        let processing = (pong.t_reply_ns as i128 - pong.t_recv_ns as i128) as f64 / 1e6;
        if processing > elapsed + 1000.0 {
            return Err(SessionError::Invalid);
        }
        let rtt = (elapsed - processing).max(0.0);
        let offset = ((pong.t_recv_ns as i128 - sent as i128)
            + (pong.t_reply_ns as i128 - received as i128)) as f64
            / 2e6;
        if self.samples.len() == 2048 {
            self.samples.remove(0);
            self.offsets.remove(0);
        }
        self.samples.push(rtt);
        self.offsets.push(offset);
        let mut values = self.samples.clone();
        values.sort_by(f64::total_cmp);
        let mut offsets = self.offsets.clone();
        offsets.sort_by(f64::total_cmp);
        publish(status, |s| {
            s.echo_samples = values.len() as u32;
            s.echo_rtt_p50_ms = Some(values[(values.len() - 1) / 2]);
            s.echo_rtt_p95_ms =
                Some(values[((values.len() * 95).div_ceil(100) - 1).min(values.len() - 1)]);
            s.clock_offset_ms = Some(offsets[(offsets.len() - 1) / 2]);
        });
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod command_epoch_tests {
    use super::*;
    use crate::session::preview::{Command, CommandKind};
    fn finish(epoch: u64) -> Option<Command> {
        Command::capture(
            epoch,
            CommandKind::Finish {
                gesture: Id::from_parts(1_700_000_000_000, [1; 10]).unwrap(),
                cancel: false,
            },
        )
    }
    #[tokio::test]
    async fn blocked_queue_sender_and_cross_thread_capture_cannot_cross_carrier_epoch() {
        let epoch = Arc::new(AtomicU64::new(0));
        assert!(finish(epoch.load(Ordering::Acquire)).is_none());
        let old = ConnectionEpoch::enter(epoch.clone()).unwrap();
        let at_method_entry = epoch.load(Ordering::Acquire);
        let stale = finish(old.value).unwrap();
        let (send, mut receive) = mpsc::channel(1);
        assert!(send.send(finish(old.value).unwrap()).await.is_ok());
        let waiting = tokio::spawn(async move { send.send(stale).await });
        tokio::task::yield_now().await;
        drop(old);
        assert!(finish(epoch.load(Ordering::Acquire)).is_none());
        let drained = receive.try_recv().unwrap();
        // Drain unblocks the old producer, but it can enqueue after the drain
        // and even after the new connection has already become active.
        let current = ConnectionEpoch::enter(epoch.clone()).unwrap();
        let paused_during_validation = finish(at_method_entry).unwrap();
        assert!(!paused_during_validation.belongs_to(current.value));
        assert!(waiting.await.unwrap().is_ok());
        let delayed = receive.recv().await.unwrap();
        assert!(!drained.belongs_to(current.value));
        assert!(!delayed.belongs_to(current.value));
        assert!(finish(current.value).unwrap().belongs_to(current.value));
        drop(current);
        assert!(!delayed.belongs_to(epoch.load(Ordering::Acquire)));
    }
    #[test]
    fn active_or_exhausted_epoch_cannot_be_reentered_or_wrap() {
        let epoch = Arc::new(AtomicU64::new(0));
        let active = ConnectionEpoch::enter(epoch.clone()).unwrap();
        assert!(ConnectionEpoch::enter(epoch.clone()).is_err());
        assert_eq!(epoch.load(Ordering::Acquire), active.value);
        drop(active);
        epoch.store(u64::MAX - 1, Ordering::Release);
        assert!(ConnectionEpoch::enter(epoch.clone()).is_err());
        assert!(finish(epoch.load(Ordering::Acquire)).is_none());
    }
}

#[cfg(test)]
mod listener_admission_tests {
    use super::{SessionError, terminal_establish_failure};
    #[test]
    fn rejected_incoming_peer_does_not_retire_host_listener() {
        assert!(!terminal_establish_failure(
            true,
            &SessionError::Authentication
        ));
        assert!(terminal_establish_failure(
            false,
            &SessionError::Authentication
        ));
    }
    #[test]
    fn local_terminal_failures_remain_terminal_on_both_routes() {
        for host in [false, true] {
            assert!(terminal_establish_failure(host, &SessionError::Closed));
            assert!(terminal_establish_failure(host, &SessionError::Storage));
            assert!(!terminal_establish_failure(host, &SessionError::Timeout));
            assert!(!terminal_establish_failure(host, &SessionError::Transport));
            assert!(!terminal_establish_failure(host, &SessionError::Invalid));
        }
    }
}

#[cfg(test)]
mod incoming_handshake_provenance_tests {
    use super::{SessionError, incoming_handshake_error, terminal_establish_failure};
    #[test]
    fn reset_broken_pipe_and_truncated_handshake_remain_retryable_transport() {
        for kind in [
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::UnexpectedEof,
        ] {
            let mapped = incoming_handshake_error(vw_net::NetError::Io(std::io::Error::from(kind)));
            assert!(matches!(mapped, SessionError::Transport));
            assert!(!terminal_establish_failure(true, &mapped));
        }
    }
    #[test]
    fn explicit_store_io_and_generic_application_io_remain_storage() {
        let stored = vw_net::NetError::Store(vw_store::StoreError::Io(std::io::Error::from(
            std::io::ErrorKind::PermissionDenied,
        )));
        let mapped = incoming_handshake_error(stored);
        assert!(matches!(mapped, SessionError::Storage));
        assert!(terminal_establish_failure(true, &mapped));
        let generic = SessionError::from(vw_net::NetError::Io(std::io::Error::from(
            std::io::ErrorKind::BrokenPipe,
        )));
        assert!(matches!(generic, SessionError::Storage));
    }
    #[test]
    fn incoming_authentication_and_closed_policies_are_not_weakened() {
        let mapped = incoming_handshake_error(vw_net::NetError::Authentication);
        assert!(matches!(mapped, SessionError::Authentication));
        assert!(!terminal_establish_failure(true, &mapped));
        assert!(terminal_establish_failure(false, &mapped));
        assert!(terminal_establish_failure(true, &SessionError::Closed));
        assert!(terminal_establish_failure(false, &SessionError::Closed));
    }
}
