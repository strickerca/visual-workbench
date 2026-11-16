//! Automatic routes: where to listen and where to dial without the owner
//! typing an address. Every address in this module is only a hint for where to
//! try. A route counts once the pinned TLS handshake has authenticated the
//! paired device on it; a name, an address or an advertisement grants nothing.
use super::{
    SessionEndpoint, SessionError, SessionResult, SessionService, device, endpoint,
    link::{AppCarrier, Established, Listener},
};
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, mpsc};
use vw_model::DeviceId;
use vw_net::{
    CarrierIo, LocalHello, NetError, SecureConnection,
    carrier::{PinnedTls, QuicCarrier, QuicListener, TcpCarrier},
    pairing::PairingManager,
};

/// Both sides must offer this before a client looks for another route while
/// one is live: only an automatic host keeps accepting during a connection.
pub(super) const HANDOFF_CAPABILITY: &str = "route_handoff_v1";
/// Shown in the session status when there is nowhere to dial yet.
pub(super) const NO_ROUTE: &str = "no known route";
const MAX_BOOK_PEERS: usize = 64;
const MAX_REMEMBERED: usize = 6;
pub(super) const MAX_HINTS: usize = 16;
const MAX_CANDIDATES: usize = 12;
pub(super) const MAX_LISTENERS: usize = 8;
const ACCEPTORS_PER_LISTENER: usize = 2;
/// One address may stay silent this long before the attempt moves on.
pub(super) const DIAL_TIMEOUT: Duration = Duration::from_secs(3);
/// After the first address answers, a better carrier gets this long to answer too.
pub(super) const DIAL_GRACE: Duration = Duration::from_millis(250);
pub(super) const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const PROBE_TICK: Duration = Duration::from_millis(100);
const PROBE_FIRST: Duration = Duration::from_secs(1);
const PROBE_SLOWEST: Duration = Duration::from_secs(8);
const RESCUE_EVERY: Duration = Duration::from_secs(1);
/// A live link normally hears from its peer at least twice a second.
const SILENT_AFTER: Duration = Duration::from_secs(1);
/// How long the route in use may take to answer a fresh handshake, once
/// another route has answered, before it counts as dead rather than busy.
const RESCUE_GRACE: Duration = Duration::from_millis(300);
/// Time the link loop spent unable to read does not count as peer silence.
pub(super) const TURN_CAP: Duration = Duration::from_millis(50);
/// Diagnostics area for what this module measures about a live link.
const AREA: &str = "session";
/// When a route ends, its replacement is looked for in steps of this length:
/// once if none is being confirmed, and until the Hello ends if one is.
pub(super) const HANDOVER_POLL: Duration = Duration::from_millis(10);

pub(super) type Path = (AppCarrier, SocketAddr);

/// Why attempts are failing. Each distinct cause is recorded once per episode
/// in the core diagnostics, so a retry loop cannot flood the bounded ring.
pub(super) struct Causes {
    area: &'static str,
    state: Mutex<(Vec<&'static str>, Option<&'static str>)>,
}
impl Causes {
    pub fn new(area: &'static str) -> Self {
        Self {
            area,
            state: Mutex::new((Vec::new(), None)),
        }
    }
    pub fn note(&self, cause: &'static str) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.1 = Some(cause);
        if !state.0.contains(&cause) && state.0.len() < 16 {
            state.0.push(cause);
            crate::diagnostics::failure(self.area, cause);
        }
    }
    /// A route authenticated: whatever fails next is news again.
    pub fn settle(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.0.clear();
            state.1 = None;
        }
    }
    pub fn last(&self) -> Option<&'static str> {
        self.state.lock().ok().and_then(|state| state.1)
    }
}

pub(super) fn dial_cause(error: &NetError) -> &'static str {
    match error {
        NetError::Timeout => "route did not answer in time",
        NetError::Authentication => "route did not prove the paired device",
        NetError::Io(_) | NetError::Carrier => "route unreachable",
        NetError::Backpressure => "route busy",
        _ => "route handshake rejected",
    }
}
pub(super) fn incoming_cause(error: &SessionError) -> &'static str {
    match error {
        SessionError::Authentication => "incoming device not verified",
        SessionError::Timeout => "incoming handshake timed out",
        SessionError::Transport => "incoming connection dropped",
        SessionError::Storage => "trust store unavailable",
        _ => "incoming handshake rejected",
    }
}
pub(super) fn link_cause(error: &SessionError) -> &'static str {
    match error {
        SessionError::Authentication => "paired device revoked or not verified",
        SessionError::Storage => "project storage failed during sync",
        SessionError::Backpressure => "link capacity exceeded",
        SessionError::Timeout => "paired device silent too long",
        SessionError::Invalid => "paired device sent an invalid message",
        SessionError::Closed => "link closed",
        _ => "link lost",
    }
}

/// Where each paired device was last authenticated, newest first. Held in
/// memory by the app-level session service; the app saves it with its other
/// app-level pairing data and restores it at start. Never part of a project.
#[derive(Default)]
pub(crate) struct RouteBook(Mutex<BTreeMap<DeviceId, Vec<Path>>>);
impl RouteBook {
    pub fn known(&self, peer: &DeviceId) -> Vec<Path> {
        self.0
            .lock()
            .ok()
            .and_then(|book| book.get(peer).cloned())
            .unwrap_or_default()
    }
    pub fn remember(&self, peer: &DeviceId, path: Path) {
        let Ok(mut book) = self.0.lock() else {
            return;
        };
        if !book.contains_key(peer) && book.len() >= MAX_BOOK_PEERS {
            return;
        }
        let known = book.entry(peer.clone()).or_default();
        known.retain(|(_, address)| *address != path.1);
        known.insert(0, path);
        known.truncate(MAX_REMEMBERED);
    }
    pub fn restore(&self, peer: &DeviceId, paths: Vec<Path>) {
        let Ok(mut book) = self.0.lock() else {
            return;
        };
        if paths.is_empty() {
            book.remove(peer);
        } else if book.contains_key(peer) || book.len() < MAX_BOOK_PEERS {
            book.insert(peer.clone(), paths);
        }
    }
    pub fn forget(&self, peer: &DeviceId) {
        if let Ok(mut book) = self.0.lock() {
            book.remove(peer);
        }
    }
}

/// Numeric private, link-local or loopback addresses only, at most `limit`.
/// Several addresses may share a carrier. An address to dial appears once; an
/// address to listen on may serve a TCP and a QUIC carrier, or name port 0.
pub(super) fn paths(
    endpoints: Vec<SessionEndpoint>,
    limit: usize,
    bind: bool,
) -> SessionResult<Vec<Path>> {
    if endpoints.len() > limit {
        return Err(SessionError::Invalid);
    }
    let mut result: Vec<Path> = Vec::new();
    for value in endpoints {
        let address = endpoint(&value.address, bind)?;
        if let SocketAddr::V6(v6) = address
            && v6.ip().is_unicast_link_local()
            && v6.scope_id() == 0
        {
            return Err(SessionError::Invalid);
        }
        if value.carrier == AppCarrier::TcpAdb && !address.ip().is_loopback() {
            return Err(SessionError::Invalid);
        }
        let repeated = result.iter().any(|(carrier, known)| {
            *known == address && (!bind || *carrier == value.carrier) && address.port() != 0
        });
        if !repeated {
            result.push((value.carrier, address));
        }
    }
    Ok(result)
}
fn endpoints(paths: &[Path]) -> Vec<SessionEndpoint> {
    paths
        .iter()
        .map(|(carrier, address)| SessionEndpoint {
            carrier: *carrier,
            address: address.to_string(),
        })
        .collect()
}

/// Dial order: best carrier first; within a carrier, remembered routes before
/// new hints. A current hint's carrier replaces the remembered carrier for the
/// same address, because the app classifies hints against today's interfaces.
pub(super) fn plan(remembered: Vec<Path>, hints: &[Path]) -> Vec<Path> {
    let mut result: Vec<Path> = Vec::new();
    for (carrier, address) in remembered {
        if result.iter().any(|(_, known)| *known == address) {
            continue;
        }
        let carrier = hints
            .iter()
            .find(|(_, hinted)| *hinted == address)
            .map_or(carrier, |(hinted, _)| *hinted);
        result.push((carrier, address));
    }
    for (carrier, address) in hints {
        if !result.iter().any(|(_, known)| known == address) {
            result.push((*carrier, *address));
        }
    }
    result.sort_by_key(|(carrier, _)| carrier.rank());
    result.truncate(MAX_CANDIDATES);
    result
}

pub(super) struct Dialled {
    pub carrier: AppCarrier,
    pub address: SocketAddr,
    pub io: CarrierIo,
}
async fn open(
    carrier: AppCarrier,
    address: SocketAddr,
    tls: &PinnedTls,
) -> vw_net::Result<CarrierIo> {
    if carrier == AppCarrier::TcpAdb {
        return TcpCarrier::connect(address, tls).await.map(CarrierIo::Tcp);
    }
    let bind = SocketAddr::new(
        if address.is_ipv4() {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V6(Ipv6Addr::UNSPECIFIED)
        },
        0,
    );
    QuicCarrier::connect(bind, address, tls)
        .await
        .map(CarrierIo::Quic)
}
/// Open pinned TLS carriers to every address at once and keep the one that
/// authenticated and comes first in `paths`. Once any address has answered,
/// the addresses before it get `earlier_within` to answer too, or as long
/// again as that first answer took if that is longer: on a slow network or a
/// busy machine every handshake is slow, and a slow answer is not a dead
/// route. Nothing is said on the losers: the application Hello goes only to
/// the chosen one, so the peer never sees two sessions. An address that fails
/// authentication is simply not the paired device.
pub(super) async fn dial(
    paths: &[Path],
    manager: &PairingManager,
    peer: &DeviceId,
    (limit, earlier_within): (Duration, Duration),
    causes: Option<&Causes>,
) -> SessionResult<Dialled> {
    let note = |cause: &'static str| {
        if let Some(causes) = causes {
            causes.note(cause);
        }
    };
    if paths.is_empty() {
        note(NO_ROUTE);
        return Err(SessionError::Transport);
    }
    // Revoked or unknown here is final for the caller; no address is tried.
    let tls = Arc::new(manager.pinned_tls(peer).map_err(|error| {
        note("paired device revoked or unknown");
        SessionError::from(error)
    })?);
    let started = tokio::time::Instant::now();
    let mut attempts = tokio::task::JoinSet::new();
    for (index, (carrier, address)) in paths.iter().copied().enumerate() {
        let tls = tls.clone();
        attempts.spawn(async move {
            let opened = tokio::time::timeout(limit, open(carrier, address, &tls)).await;
            (
                index,
                opened.unwrap_or(Err(NetError::Timeout)).map(|io| Dialled {
                    carrier,
                    address,
                    io,
                }),
            )
        });
    }
    let mut pending: Vec<usize> = (0..paths.len()).collect();
    let mut best: Option<(usize, Dialled)> = None;
    let mut grace: Option<tokio::time::Instant> = None;
    let mut error = SessionError::Timeout;
    loop {
        let finished = match grace {
            Some(deadline) => match tokio::time::timeout_at(deadline, attempts.join_next()).await {
                Ok(finished) => finished,
                Err(_) => break,
            },
            None => attempts.join_next().await,
        };
        let Some(finished) = finished else {
            break;
        };
        let Ok((index, opened)) = finished else {
            error = SessionError::Worker;
            continue;
        };
        pending.retain(|waiting| *waiting != index);
        match opened {
            Ok(dialled) => {
                if best.as_ref().is_none_or(|(chosen, _)| index < *chosen) {
                    best = Some((index, dialled));
                }
                let chosen = best.as_ref().map_or(usize::MAX, |(chosen, _)| *chosen);
                if !pending.iter().any(|waiting| *waiting < chosen) {
                    break;
                }
                grace.get_or_insert_with(|| {
                    let now = tokio::time::Instant::now();
                    now + earlier_within.max(now.duration_since(started))
                });
            }
            Err(failed) => {
                note(dial_cause(&failed));
                if !matches!(failed, NetError::Timeout) {
                    error = SessionError::Transport;
                }
            }
        }
    }
    attempts.abort_all();
    best.map(|(_, dialled)| dialled).ok_or(error)
}

/// What a live connection reports to the search for another route.
#[derive(Default)]
pub(super) struct Pulse {
    silent_ms: AtomicU64,
    settled: AtomicBool,
    confirming: AtomicBool,
}
impl Pulse {
    /// True while another route has authenticated and its Hello is under way:
    /// the peer may hand over, and close the old route, at any moment now.
    pub fn confirming(&self) -> bool {
        self.confirming.load(Ordering::Acquire)
    }
    /// `settled` is false while a transfer or live media would be interrupted.
    pub fn report(&self, silence: Duration, settled: bool) {
        self.silent_ms.store(
            u64::try_from(silence.as_millis()).unwrap_or(u64::MAX),
            Ordering::Release,
        );
        self.settled.store(settled, Ordering::Release);
    }
    fn silent(&self) -> bool {
        Duration::from_millis(self.silent_ms.load(Ordering::Acquire)) >= SILENT_AFTER
    }
}
pub(super) struct Watch<'a> {
    pub manager: &'a PairingManager,
    pub peer: &'a DeviceId,
    pub local: &'a LocalHello,
    pub carrier: AppCarrier,
    pub address: Option<SocketAddr>,
    pub pulse: &'a Pulse,
    pub handoff: bool,
    pub causes: &'a Causes,
}

/// Automatic client: addresses come from the route book and from hints the
/// app offers (a scanned pairing address, discovery). None is required.
pub(super) struct Seeker {
    book: Arc<RouteBook>,
    hints: Mutex<Vec<Path>>,
    /// When the newest offer was made, for timing a move from that moment.
    offered: Mutex<Option<tokio::time::Instant>>,
    changed: Notify,
}
impl Seeker {
    pub fn new(book: Arc<RouteBook>, hints: Vec<Path>) -> Arc<Self> {
        Arc::new(Self {
            book,
            hints: Mutex::new(hints),
            offered: Mutex::new(None),
            changed: Notify::new(),
        })
    }
    pub fn offer(&self, hints: Vec<Path>) {
        if let Ok(mut current) = self.hints.lock() {
            *current = hints;
        }
        if let Ok(mut offered) = self.offered.lock() {
            *offered = Some(tokio::time::Instant::now());
        }
        self.changed.notify_one();
    }
    pub fn candidates(&self, peer: &DeviceId) -> Vec<Path> {
        let hints = self
            .hints
            .lock()
            .map(|hints| hints.clone())
            .unwrap_or_default();
        plan(self.book.known(peer), &hints)
    }
    pub async fn changed(&self) {
        self.changed.notified().await;
    }
    /// Runs beside a live connection and returns a connection that should
    /// replace it. Two cases only:
    /// - a better carrier answers while the link is otherwise idle;
    /// - the link has gone silent, the address in use does not answer a fresh
    ///   handshake, and another address does. A silent peer whose address
    ///   still answers is busy, not gone, and is left alone.
    ///
    /// The peer is asked only when it also keeps accepting during a connection.
    ///
    /// The core times both moves itself and records them in its diagnostics
    /// (area `session`): `route upgrade` from the newest offer, or from the
    /// start of the attempt when nothing was offered, and `route rescue` from
    /// when the silence was noticed; each ends once the pinned handshake and
    /// Hello have completed on the new route. `silent route still answers` is
    /// recorded once for each silence that turned out to be a busy peer.
    pub async fn successor(&self, watch: Watch<'_>) -> Established {
        if !watch.handoff {
            return std::future::pending().await;
        }
        let began = tokio::time::Instant::now();
        let mut wait = PROBE_FIRST;
        let mut upgrade = began + wait;
        let mut rescue = began;
        let mut silent_since: Option<tokio::time::Instant> = None;
        let mut kept = false;
        loop {
            tokio::select! {
                _ = self.changed.notified() => {
                    let now = tokio::time::Instant::now();
                    wait = PROBE_FIRST;
                    upgrade = now;
                    rescue = now;
                }
                _ = tokio::time::sleep(PROBE_TICK) => {}
            }
            let now = tokio::time::Instant::now();
            let silent = watch.pulse.silent();
            let mut paths = self.candidates(watch.peer);
            if silent {
                silent_since.get_or_insert(now);
                if now < rescue {
                    continue;
                }
                rescue = now + RESCUE_EVERY;
                // The address in use is asked first and given time to answer:
                // while it does, the peer is only busy and nothing may move.
                paths.sort_by_key(|(_, address)| Some(*address) != watch.address);
            } else {
                silent_since = None;
                kept = false;
                if now < upgrade || !watch.pulse.settled.load(Ordering::Acquire) {
                    continue;
                }
                paths.retain(|(carrier, _)| carrier.rank() < watch.carrier.rank());
                if paths.is_empty() {
                    continue;
                }
                upgrade = now + wait;
                wait = (wait * 2).min(PROBE_SLOWEST);
            }
            // A better carrier that is not there yet is the normal case, not a
            // failure worth recording; a silent link with no way out is.
            let (grace, causes) = if silent {
                (RESCUE_GRACE, Some(watch.causes))
            } else {
                (DIAL_GRACE, None)
            };
            let limits = (PROBE_TIMEOUT, grace);
            let dialled = tokio::select! {
                dialled = dial(&paths, watch.manager, watch.peer, limits, causes) => dialled,
                // A new offer never waits behind an attempt that may be aimed
                // at a dead address: the attempt is dropped (nothing was said
                // on it) and the search starts over with what is known now.
                _ = self.changed.notified() => {
                    self.changed.notify_one();
                    continue;
                }
            };
            let Ok(dialled) = dialled else {
                continue;
            };
            if Some(dialled.address) == watch.address {
                if silent && !kept {
                    kept = true;
                    crate::diagnostics::count(AREA, "silent route still answers", 1);
                }
                continue;
            }
            watch.pulse.confirming.store(true, Ordering::Release);
            let hello = SecureConnection::client(dialled.io, watch.local);
            let hello = tokio::time::timeout(HELLO_TIMEOUT, hello).await;
            watch.pulse.confirming.store(false, Ordering::Release);
            if let Ok(Ok(connection)) = hello {
                let (name, since) = if silent {
                    ("route rescue", silent_since.unwrap_or(now))
                } else {
                    // An offer made on this connection; an older one says
                    // nothing about how long this move took.
                    let offered = self.offered.lock().ok().and_then(|offered| *offered);
                    (
                        "route upgrade",
                        offered.filter(|at| *at >= began).unwrap_or(now),
                    )
                };
                crate::diagnostics::duration(AREA, name, since.elapsed());
                return Established {
                    carrier: dialled.carrier,
                    address: Some(dialled.address),
                    connection,
                };
            }
        }
    }
}

struct Bound {
    carrier: AppCarrier,
    requested: SocketAddr,
    local: SocketAddr,
    acceptors: Vec<tokio::task::AbortHandle>,
}
impl Drop for Bound {
    fn drop(&mut self) {
        for acceptor in &self.acceptors {
            acceptor.abort();
        }
    }
}
/// Automatic host: every listener keeps accepting, also while a connection is
/// live, so the paired device can come back on another route at once. Every
/// arrival has passed the same pinned mutual TLS and Hello as a first
/// connection; an arrival that fails either never disturbs the live one.
pub(super) struct Standby {
    manager: PairingManager,
    peer: DeviceId,
    local: LocalHello,
    bound: Mutex<Vec<Bound>>,
    public: Arc<Mutex<Vec<SessionEndpoint>>>,
    gate: tokio::sync::Mutex<()>,
    deliver: mpsc::Sender<Established>,
    arrivals: tokio::sync::Mutex<mpsc::Receiver<Established>>,
    causes: Arc<Causes>,
    retired: AtomicBool,
}
impl Standby {
    pub fn new(
        manager: PairingManager,
        peer: DeviceId,
        local: LocalHello,
        public: Arc<Mutex<Vec<SessionEndpoint>>>,
        causes: Arc<Causes>,
    ) -> Arc<Self> {
        let (deliver, arrivals) = mpsc::channel(1);
        Arc::new(Self {
            manager,
            peer,
            local,
            bound: Mutex::new(Vec::new()),
            public,
            gate: tokio::sync::Mutex::new(()),
            deliver,
            arrivals: tokio::sync::Mutex::new(arrivals),
            causes,
            retired: AtomicBool::new(false),
        })
    }
    /// The session that served these listeners is over. Nothing listens from
    /// here on and an arrival nobody will serve is closed, so the paired
    /// device is never left holding a connection that stays mute.
    pub fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        if let Ok(mut bound) = self.bound.lock() {
            bound.clear();
        }
        if let Ok(mut public) = self.public.lock() {
            public.clear();
        }
        if let Ok(mut arrivals) = self.arrivals.try_lock() {
            while arrivals.try_recv().is_ok() {}
        }
    }
    /// Make the listeners match `wanted`: bind what is new, retire what is no
    /// longer wanted and keep the rest untouched. An address that cannot be
    /// bound is skipped. Returns what is bound now.
    pub async fn listen(&self, wanted: Vec<Path>) -> SessionResult<Vec<SessionEndpoint>> {
        let _gate = self.gate.lock().await;
        if self.retired.load(Ordering::Acquire) {
            return Err(SessionError::Closed);
        }
        // A listener serves a wanted address whether the caller names what it
        // asked for (possibly port 0) or the address that was actually bound.
        let serves = |entry: &Bound, (carrier, address): &Path| {
            entry.carrier == *carrier && (entry.requested == *address || entry.local == *address)
        };
        let (missing, retired): (Vec<Path>, bool) = {
            let mut bound = self.bound.lock().map_err(|_| SessionError::Worker)?;
            let before = bound.len();
            bound.retain(|entry| wanted.iter().any(|path| serves(entry, path)));
            let missing = wanted
                .iter()
                .filter(|path| !bound.iter().any(|entry| serves(entry, path)))
                .copied()
                .collect();
            (missing, bound.len() != before)
        };
        if retired {
            // Let the retired acceptors and their sockets go before an address
            // is bound again under another carrier.
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
        }
        for (carrier, address) in missing {
            let room = self.bound.lock().map_err(|_| SessionError::Worker)?.len() < MAX_LISTENERS;
            if !room {
                break;
            }
            match self.bind(carrier, address).await {
                // Retired while this address was being bound: it goes at once.
                Ok(_) if self.retired.load(Ordering::Acquire) => return Err(SessionError::Closed),
                Ok(entry) => self
                    .bound
                    .lock()
                    .map_err(|_| SessionError::Worker)?
                    .push(entry),
                // The paired device is revoked or unknown: nothing may listen.
                Err(SessionError::Authentication) => return Err(SessionError::Authentication),
                Err(_) => self.causes.note("listener bind failed"),
            }
        }
        let current: Vec<Path> = self
            .bound
            .lock()
            .map_err(|_| SessionError::Worker)?
            .iter()
            .map(|entry| (entry.carrier, entry.local))
            .collect();
        let current = endpoints(&current);
        *self.public.lock().map_err(|_| SessionError::Worker)? = current.clone();
        Ok(current)
    }
    async fn bind(&self, carrier: AppCarrier, address: SocketAddr) -> SessionResult<Bound> {
        let tls = self.manager.pinned_tls(&self.peer)?;
        let listener = if carrier == AppCarrier::TcpAdb {
            let listener = tokio::net::TcpListener::bind(address)
                .await
                .map_err(|_| SessionError::Transport)?;
            Listener::Tcp(listener, tls)
        } else {
            Listener::Quic(QuicListener::bind(address, tls).map_err(|_| SessionError::Transport)?)
        };
        let local = listener.local_addr()?;
        let listener = Arc::new(listener);
        let acceptors = (0..ACCEPTORS_PER_LISTENER)
            .map(|_| {
                tokio::spawn(accept(
                    listener.clone(),
                    carrier,
                    self.local.clone(),
                    self.deliver.clone(),
                    self.causes.clone(),
                ))
                .abort_handle()
            })
            .collect();
        Ok(Bound {
            carrier,
            requested: address,
            local,
            acceptors,
        })
    }
    /// The next authenticated connection from the paired device.
    pub async fn next(&self) -> Established {
        match self.arrivals.lock().await.recv().await {
            Some(arrival) => arrival,
            // Unreachable while this owner holds a sender; never fabricate one.
            None => std::future::pending().await,
        }
    }
}
async fn accept(
    listener: Arc<Listener>,
    carrier: AppCarrier,
    local: LocalHello,
    deliver: mpsc::Sender<Established>,
    causes: Arc<Causes>,
) {
    let mut pause = Duration::from_millis(50);
    loop {
        match listener.accept().await {
            Ok(io) => {
                pause = Duration::from_millis(50);
                let hello = SecureConnection::server(io, &local);
                match tokio::time::timeout(HELLO_TIMEOUT, hello).await {
                    Ok(Ok(connection)) => {
                        let arrival = Established {
                            carrier,
                            address: None,
                            connection,
                        };
                        if deliver.send(arrival).await.is_err() {
                            return;
                        }
                    }
                    // A carrier the client opened and did not choose ends here.
                    Ok(Err(NetError::Carrier | NetError::Io(_))) => {}
                    Ok(Err(error)) => causes.note(incoming_cause(&error.into())),
                    Err(_) => causes.note("incoming hello timed out"),
                }
            }
            // Nothing arrived within the accept bound.
            Err(SessionError::Timeout) => {}
            Err(error) => {
                causes.note(incoming_cause(&error));
                // A refused stranger or a dead socket must not spin this task.
                tokio::time::sleep(pause).await;
                pause = (pause * 2).min(Duration::from_secs(1));
            }
        }
    }
}

#[uniffi::export]
impl SessionService {
    /// Where this paired device was last authenticated, newest first. These
    /// are hints for the next dial; save them only with app-level pairing data.
    pub fn known_routes(&self, peer_device_id: String) -> SessionResult<Vec<SessionEndpoint>> {
        Ok(endpoints(&self.routes.known(&device(peer_device_id)?)))
    }
    /// Give back hints saved by an earlier run. Accepted only for a device
    /// that is paired and not revoked; an empty list forgets its routes.
    pub async fn restore_routes(
        &self,
        peer_device_id: String,
        endpoints: Vec<SessionEndpoint>,
    ) -> SessionResult<()> {
        let peer = device(peer_device_id)?;
        let paths = paths(endpoints, MAX_REMEMBERED, false)?;
        let store = self.store.clone();
        let trusted = peer.clone();
        crate::worker::startup(move || {
            Ok((|| -> SessionResult<()> {
                store.load()?.trusted_peer(&trusted)?;
                Ok(())
            })())
        })
        .await??;
        self.routes.restore(&peer, paths);
        Ok(())
    }
    pub fn forget_routes(&self, peer_device_id: String) -> SessionResult<()> {
        self.routes.forget(&device(peer_device_id)?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }
    fn hint(carrier: AppCarrier, address: &str) -> SessionEndpoint {
        SessionEndpoint {
            carrier,
            address: address.into(),
        }
    }
    #[test]
    fn the_book_keeps_the_newest_few_routes_per_device_and_forgets_on_request() {
        let book = RouteBook::default();
        let peer = DeviceId::from_bytes([3; 16]);
        for port in 1..=8 {
            book.remember(&peer, (AppCarrier::QuicWifi, at(port)));
        }
        book.remember(&peer, (AppCarrier::QuicTether, at(5)));
        let known = book.known(&peer);
        assert_eq!(known.len(), MAX_REMEMBERED);
        assert_eq!(known[0], (AppCarrier::QuicTether, at(5)));
        assert_eq!(known.iter().filter(|(_, a)| *a == at(5)).count(), 1);
        assert!(!known.iter().any(|(_, a)| *a == at(1)));
        assert!(book.known(&DeviceId::from_bytes([4; 16])).is_empty());
        book.restore(&peer, vec![(AppCarrier::QuicWifi, at(9))]);
        assert_eq!(book.known(&peer), vec![(AppCarrier::QuicWifi, at(9))]);
        book.forget(&peer);
        assert!(book.known(&peer).is_empty());
        for n in 0..=MAX_BOOK_PEERS {
            let id = DeviceId::from_bytes([n as u8 + 10; 16]);
            book.remember(&id, (AppCarrier::QuicWifi, at(1)));
        }
        let last = DeviceId::from_bytes([MAX_BOOK_PEERS as u8 + 10; 16]);
        assert!(book.known(&last).is_empty());
    }
    #[test]
    fn dial_order_prefers_the_best_carrier_then_remembered_routes_and_trusts_current_labels() {
        let remembered = vec![
            (AppCarrier::QuicWifi, at(1)),
            (AppCarrier::QuicTether, at(2)),
            (AppCarrier::QuicWifi, at(3)),
        ];
        let hints = [
            (AppCarrier::QuicWifi, at(4)),
            (AppCarrier::QuicTether, at(5)),
            // The interface this address is on is no longer a USB link.
            (AppCarrier::QuicWifi, at(2)),
            (AppCarrier::TcpAdb, at(6)),
        ];
        assert_eq!(
            plan(remembered, &hints),
            vec![
                (AppCarrier::QuicTether, at(5)),
                (AppCarrier::QuicWifi, at(1)),
                (AppCarrier::QuicWifi, at(2)),
                (AppCarrier::QuicWifi, at(3)),
                (AppCarrier::QuicWifi, at(4)),
                (AppCarrier::TcpAdb, at(6)),
            ]
        );
        let many: Vec<Path> = (1..=40)
            .map(|port| (AppCarrier::QuicWifi, at(port)))
            .collect();
        assert_eq!(plan(Vec::new(), &many).len(), MAX_CANDIDATES);
        assert!(plan(Vec::new(), &[]).is_empty());
    }
    #[test]
    fn hints_accept_only_bounded_numeric_local_addresses() {
        let accepted = paths(
            vec![
                hint(AppCarrier::QuicWifi, "127.0.0.1:7"),
                hint(AppCarrier::QuicWifi, "127.0.0.1:7"),
                hint(AppCarrier::QuicWifi, "127.0.0.1:8"),
            ],
            MAX_HINTS,
            false,
        );
        assert_eq!(
            accepted.ok(),
            Some(vec![
                (AppCarrier::QuicWifi, at(7)),
                (AppCarrier::QuicWifi, at(8))
            ])
        );
        for refused in [
            hint(AppCarrier::QuicWifi, "192.0.2.1:1"),
            hint(AppCarrier::QuicWifi, "example.test:1"),
            hint(AppCarrier::QuicWifi, "127.0.0.1:0"),
            hint(AppCarrier::QuicWifi, "0.0.0.0:1"),
            hint(AppCarrier::QuicWifi, "[fe80::1]:1"),
            hint(AppCarrier::TcpAdb, "[fe80::1%7]:1"),
        ] {
            assert!(paths(vec![refused], MAX_HINTS, false).is_err());
        }
        assert!(paths(vec![hint(AppCarrier::QuicWifi, "127.0.0.1:0")], 1, true).is_ok());
        let crowd = (1..=MAX_HINTS as u16 + 1)
            .map(|port| hint(AppCarrier::QuicWifi, &format!("127.0.0.1:{port}")))
            .collect();
        assert!(paths(crowd, MAX_HINTS, false).is_err());
        assert!(paths(Vec::new(), MAX_HINTS, false).is_ok_and(|none| none.is_empty()));
    }
    #[test]
    fn a_repeated_cause_is_recorded_once_until_a_route_authenticates() {
        let before = crate::core_diagnostics(0).latest_sequence;
        let causes = Causes::new("route test");
        let mine = || {
            crate::core_diagnostics(before)
                .events
                .iter()
                .filter(|event| event.area == "route test")
                .count()
        };
        for _ in 0..50 {
            causes.note(NO_ROUTE);
            causes.note("route unreachable");
        }
        assert_eq!(mine(), 2);
        assert_eq!(causes.last(), Some("route unreachable"));
        causes.settle();
        assert_eq!(causes.last(), None);
        causes.note(NO_ROUTE);
        assert_eq!(mine(), 3);
        for cause in [
            NO_ROUTE,
            dial_cause(&NetError::Timeout),
            dial_cause(&NetError::Authentication),
            dial_cause(&NetError::Carrier),
            dial_cause(&NetError::Backpressure),
            dial_cause(&NetError::Resync),
            incoming_cause(&SessionError::Authentication),
            incoming_cause(&SessionError::Timeout),
            incoming_cause(&SessionError::Transport),
            incoming_cause(&SessionError::Storage),
            incoming_cause(&SessionError::Invalid),
            link_cause(&SessionError::Authentication),
            link_cause(&SessionError::Storage),
            link_cause(&SessionError::Backpressure),
            link_cause(&SessionError::Timeout),
            link_cause(&SessionError::Invalid),
            link_cause(&SessionError::Closed),
            link_cause(&SessionError::Transport),
        ] {
            assert!(
                cause.len() <= 64
                    && cause.bytes().all(|byte| byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || b" -_.".contains(&byte)),
                "{cause}"
            );
        }
    }
}
