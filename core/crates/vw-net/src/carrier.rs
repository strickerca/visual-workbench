//! TLS 1.3 carriers. The caller supplies a locally trusted certificate/device
//! binding; T1.06b owns how that binding is established and stored. Both peers
//! present certificates and the exact expected leaf is checked after TLS.
use crate::{
    Frame, HEADER_BYTES, NetError, Result, decode_frame, encode_frame, frame::read_header,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use std::{
    collections::BTreeMap,
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, Notify},
    task::{AbortHandle, JoinSet},
};
use vw_model::DeviceId;
use vw_proto::v1::{self, envelope::Body};

const ALPN: &[u8] = b"visual-workbench/1";
const DEADLINE: Duration = Duration::from_secs(15);
#[derive(Clone)]
pub struct AuthenticatedPeer {
    device: DeviceId,
    binding: [u8; 32],
    authorization: Option<Arc<crate::carrier_authorization::Authorization>>,
}
impl AuthenticatedPeer {
    pub const fn device(&self) -> &DeviceId {
        &self.device
    }
    pub const fn channel_binding(&self) -> &[u8; 32] {
        &self.binding
    }
    /// Explicit simulation-only entry; never enabled by the application crate.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_simulation(device: DeviceId) -> Self {
        Self {
            device,
            binding: [0x53; 32],
            authorization: None,
        }
    }
    fn authorize(&self) -> Result<()> {
        if let Some(authorization) = &self.authorization {
            authorization.check(authorization.peer.certificate.as_ref())?;
        }
        Ok(())
    }
    async fn authorize_async(&self) -> Result<()> {
        if self
            .authorization
            .as_ref()
            .is_none_or(|authorization| authorization.policy.is_none())
        {
            return self.authorize();
        }
        let peer = self.clone();
        bounded(tokio::task::spawn_blocking(move || peer.authorize()))
            .await?
            .map_err(|_| NetError::Authentication)?
    }
}
impl std::fmt::Debug for AuthenticatedPeer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AuthenticatedPeer([REDACTED])")
    }
}
/// Live app-level authorization, checked in ordinary TLS handshakes and before
/// each exposed frame. It supplements WebPKI certificate/signature verification.
pub trait PeerAuthorization: Send + Sync {
    fn authorize(&self, device: &DeviceId, certificate: &[u8]) -> Result<()>;
}
pub struct Identity {
    pub certificate: CertificateDer<'static>,
    pub key: PrivateKeyDer<'static>,
}
#[derive(Clone)]
pub struct TrustedPeer {
    pub device: DeviceId,
    pub certificate: CertificateDer<'static>,
    pub server_name: String,
}
pub struct PinnedTls {
    client: Arc<rustls::ClientConfig>,
    server: Arc<rustls::ServerConfig>,
    peer: TrustedPeer,
    authorization: Arc<crate::carrier_authorization::Authorization>,
}
impl PinnedTls {
    /// Explicit caller-owned trust binding. Applications with a mutable trust
    /// store use new_authorized (or PairingManager::pinned_tls) for live revocation.
    pub fn new(identity: Identity, peer: TrustedPeer) -> Result<Self> {
        Self::build(identity, peer, None)
    }
    pub fn new_authorized(
        identity: Identity,
        peer: TrustedPeer,
        authorization: Arc<dyn PeerAuthorization>,
    ) -> Result<Self> {
        Self::build(identity, peer, Some(authorization))
    }
    fn build(
        identity: Identity,
        peer: TrustedPeer,
        policy: Option<Arc<dyn PeerAuthorization>>,
    ) -> Result<Self> {
        if peer.certificate.is_empty()
            || peer.certificate.len() > 65536
            || identity.certificate.is_empty()
            || identity.certificate.len() > 65536
        {
            return Err(NetError::Authentication);
        }
        let _ =
            ServerName::try_from(peer.server_name.clone()).map_err(|_| NetError::Authentication)?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(peer.certificate.clone())
            .map_err(|_| NetError::Authentication)?;
        let authorization = Arc::new(crate::carrier_authorization::Authorization {
            peer: peer.clone(),
            policy,
        });
        authorization.check(peer.certificate.as_ref())?;
        let roots = Arc::new(roots);
        let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
            roots.clone(),
            provider.clone(),
        )
        .build()
        .map_err(|_| NetError::Authentication)?;
        let verifier = Arc::new(crate::carrier_authorization::ClientVerifier {
            cryptographic: verifier,
            authorization: authorization.clone(),
        });
        let server_verifier =
            rustls::client::WebPkiServerVerifier::builder_with_provider(roots, provider.clone())
                .build()
                .map_err(|_| NetError::Authentication)?;
        let server_verifier = Arc::new(crate::carrier_authorization::ServerVerifier {
            cryptographic: server_verifier,
            authorization: authorization.clone(),
        });
        let mut client = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|_| NetError::Authentication)?
            .dangerous()
            .with_custom_certificate_verifier(server_verifier)
            .with_client_auth_cert(vec![identity.certificate.clone()], identity.key.clone_key())
            .map_err(|_| NetError::Authentication)?;
        client.alpn_protocols = vec![ALPN.to_vec()];
        client.enable_early_data = false;
        client.resumption = rustls::client::Resumption::disabled();
        let mut server = rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|_| NetError::Authentication)?
            .with_client_cert_verifier(verifier)
            .with_single_cert(vec![identity.certificate], identity.key)
            .map_err(|_| NetError::Authentication)?;
        server.alpn_protocols = vec![ALPN.to_vec()];
        server.max_early_data_size = 0;
        server.send_tls13_tickets = 0;
        Ok(Self {
            client: Arc::new(client),
            server: Arc::new(server),
            peer,
            authorization,
        })
    }
    fn authenticate(
        &self,
        certs: Option<&[CertificateDer<'_>]>,
        binding: [u8; 32],
    ) -> Result<AuthenticatedPeer> {
        if certs.and_then(|c| c.first()) != Some(&self.peer.certificate) {
            return Err(NetError::Authentication);
        }
        self.authorization.check(self.peer.certificate.as_ref())?;
        Ok(AuthenticatedPeer {
            device: self.peer.device.clone(),
            binding,
            authorization: Some(self.authorization.clone()),
        })
    }
    fn quic_client(&self) -> Result<quinn::ClientConfig> {
        let crypto = quinn::crypto::rustls::QuicClientConfig::try_from((*self.client).clone())
            .map_err(|_| NetError::Authentication)?;
        let mut config = quinn::ClientConfig::new(Arc::new(crypto));
        config.transport_config(Arc::new(transport()));
        Ok(config)
    }
    fn quic_server(&self) -> Result<quinn::ServerConfig> {
        let crypto = quinn::crypto::rustls::QuicServerConfig::try_from((*self.server).clone())
            .map_err(|_| NetError::Authentication)?;
        let mut config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
        config.transport_config(Arc::new(transport()));
        Ok(config)
    }
}
fn transport() -> quinn::TransportConfig {
    let mut config = quinn::TransportConfig::default();
    config
        .max_concurrent_bidi_streams(4_u32.into())
        .max_concurrent_uni_streams(16_u32.into())
        .stream_receive_window((8_u32 * 1024 * 1024 + 12).into())
        .receive_window((24_u32 * 1024 * 1024).into())
        .send_window(16 * 1024 * 1024)
        .datagram_receive_buffer_size(Some(64 * 1024))
        .datagram_send_buffer_size(64 * 1024)
        .keep_alive_interval(Some(Duration::from_secs(2)));
    config
}
async fn bounded<T>(future: impl std::future::Future<Output = T>) -> Result<T> {
    tokio::time::timeout(DEADLINE, future)
        .await
        .map_err(|_| NetError::Timeout)
}
async fn read_frame<R: AsyncRead + Unpin>(read: &mut R) -> Result<Option<Frame>> {
    let mut header = [0_u8; HEADER_BYTES];
    if read.read(&mut header[..1]).await? == 0 {
        return Ok(None);
    }
    read.read_exact(&mut header[1..]).await?;
    let (_, length) = read_header(&header)?;
    let mut bytes = vec![0; HEADER_BYTES + length];
    bytes[..HEADER_BYTES].copy_from_slice(&header);
    read.read_exact(&mut bytes[HEADER_BYTES..]).await?;
    Ok(Some(decode_frame(&bytes)?))
}
async fn write_frame<W: AsyncWrite + Unpin>(write: &mut W, frame: &Frame) -> Result<()> {
    let bytes = encode_frame(frame)?;
    write.write_all(&bytes).await?;
    write.flush().await?;
    Ok(())
}
async fn read_loop<R: AsyncRead + Unpin>(
    mut read: R,
    channel: Option<v1::Channel>,
    ingress: crate::ingress::Ingress,
) {
    loop {
        let frame = read_frame(&mut read).await;
        if matches!(&frame, Ok(None)) && channel == Some(v1::Channel::Blob) {
            break;
        }
        let frame = frame
            .and_then(|f| f.ok_or(NetError::Carrier))
            .and_then(|f| {
                if channel.is_some_and(|c| c != f.channel) {
                    Err(NetError::Invalid("QUIC stream channel"))
                } else {
                    Ok(f)
                }
            });
        let result = frame.and_then(|frame| ingress.push(frame));
        if let Err(error) = result {
            ingress.fail(error);
            break;
        }
    }
}

struct Lifetime {
    failed: AtomicBool,
    closed: Notify,
    ingress: crate::ingress::Ingress,
    tasks: StdMutex<Vec<AbortHandle>>,
    quic: Option<quinn::Connection>,
    _endpoint: Option<quinn::Endpoint>,
    tcp_shutdown: Option<std::net::TcpStream>,
}
impl Lifetime {
    fn new(
        ingress: crate::ingress::Ingress,
        quic: Option<quinn::Connection>,
        endpoint: Option<quinn::Endpoint>,
        tcp_shutdown: Option<std::net::TcpStream>,
    ) -> Arc<Self> {
        Arc::new(Self {
            failed: AtomicBool::new(false),
            closed: Notify::new(),
            ingress,
            tasks: StdMutex::new(Vec::new()),
            quic,
            _endpoint: endpoint,
            tcp_shutdown,
        })
    }
    fn add(&self, task: tokio::task::JoinHandle<()>) -> Result<()> {
        let abort = task.abort_handle();
        match self.tasks.lock() {
            Ok(mut tasks) => tasks.push(abort),
            Err(_) => {
                task.abort();
                return Err(NetError::Carrier);
            }
        }
        Ok(())
    }
    fn fail(&self, error: NetError) {
        if !self.failed.swap(true, Ordering::AcqRel) {
            self.ingress.fail(error);
            if let Ok(tasks) = self.tasks.lock() {
                for task in tasks.iter() {
                    task.abort();
                }
            }
            if let Some(connection) = &self.quic {
                connection.close(0_u32.into(), b"session closed");
            }
            if let Some(socket) = &self.tcp_shutdown {
                let _ = socket.shutdown(std::net::Shutdown::Both);
            }
            self.closed.notify_waiters();
        }
    }
    fn check(&self) -> Result<()> {
        if self.ingress.is_failed() {
            self.fail(NetError::Carrier);
        }
        if self.failed.load(Ordering::Acquire) {
            Err(NetError::Carrier)
        } else {
            Ok(())
        }
    }
}
impl Drop for Lifetime {
    fn drop(&mut self) {
        if let Ok(tasks) = self.tasks.get_mut() {
            for task in tasks.iter() {
                task.abort();
            }
        }
        if let Some(connection) = &self.quic {
            connection.close(0_u32.into(), b"session closed");
        }
        if let Some(socket) = &self.tcp_shutdown {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
    }
}
struct WriteGuard {
    life: Arc<Lifetime>,
    completed: bool,
}
impl Drop for WriteGuard {
    // A cancelled partially-written framed stream must never be reused.
    fn drop(&mut self) {
        if !self.completed {
            self.life.fail(NetError::Carrier);
        }
    }
}
type TcpWrite = tokio::io::WriteHalf<tokio_rustls::TlsStream<TcpStream>>;
enum SendIo {
    Tcp(Mutex<TcpWrite>),
    Quic {
        connection: quinn::Connection,
        streams: BTreeMap<i32, Mutex<quinn::SendStream>>,
        blobs: Mutex<BTreeMap<String, quinn::SendStream>>,
    },
}
/// Cloneable write half. QUIC channels own independent locks; TCP keeps one
/// framing lock (TCP's current frame cannot be preempted on the wire).
#[derive(Clone)]
pub struct CarrierSender {
    io: Arc<SendIo>,
    life: Arc<Lifetime>,
    peer: AuthenticatedPeer,
}
impl CarrierSender {
    pub const fn peer(&self) -> &AuthenticatedPeer {
        &self.peer
    }
    pub fn independent_channels(&self) -> bool {
        matches!(&*self.io, SendIo::Quic { .. })
    }
    pub fn close(&self) {
        self.life.fail(NetError::Carrier);
    }
    pub async fn send(&self, frame: &Frame) -> Result<()> {
        self.life.check()?;
        if let Err(error) = self.peer.authorize_async().await {
            self.life.fail(NetError::Authentication);
            return Err(error);
        }
        let mut guard = WriteGuard {
            life: self.life.clone(),
            completed: false,
        };
        let closed = self.life.closed.notified();
        // Register before the second check, preventing a close/notify race.
        tokio::pin!(closed);
        closed.as_mut().enable();
        self.life.check()?;
        let result = tokio::select! {
            _ = &mut closed => Err(NetError::Carrier),
            result = bounded(self.send_inner(frame)) => result.and_then(|result| result),
        };
        guard.completed = true;
        if result.is_err() && !matches!(result, Err(NetError::Backpressure)) {
            self.life.fail(NetError::Carrier);
        }
        result
    }
    async fn send_inner(&self, frame: &Frame) -> Result<()> {
        match &*self.io {
            SendIo::Tcp(write) => {
                let mut write = write.lock().await;
                self.life.check()?;
                self.peer.authorize_async().await?;
                write_frame(&mut *write, frame).await
            }
            SendIo::Quic {
                connection,
                streams,
                blobs,
            } => {
                if frame.channel == v1::Channel::Ephemeral {
                    connection
                        .send_datagram(bytes::Bytes::from(encode_frame(frame)?))
                        .map_err(|_| NetError::Carrier)?;
                    return Ok(());
                }
                if frame.channel == v1::Channel::Blob {
                    let mut blobs = blobs.lock().await;
                    self.life.check()?;
                    self.peer.authorize_async().await?;
                    if let Some(Body::BlobChunk(chunk)) = &frame.envelope.body {
                        if !blobs.contains_key(&chunk.asset_id) {
                            if blobs.len() >= 16 {
                                return Err(NetError::Backpressure);
                            }
                            let stream =
                                connection.open_uni().await.map_err(|_| NetError::Carrier)?;
                            blobs.insert(chunk.asset_id.clone(), stream);
                        }
                        let stream = blobs
                            .get_mut(&chunk.asset_id)
                            .ok_or(NetError::Invalid("blob stream"))?;
                        write_frame(stream, frame).await?;
                        if chunk.last
                            && let Some(mut stream) = blobs.remove(&chunk.asset_id)
                        {
                            stream.finish().map_err(|_| NetError::Carrier)?;
                        }
                    } else {
                        let mut stream =
                            connection.open_uni().await.map_err(|_| NetError::Carrier)?;
                        write_frame(&mut stream, frame).await?;
                        stream.finish().map_err(|_| NetError::Carrier)?;
                    }
                    return Ok(());
                }
                let stream = streams
                    .get(&(frame.channel as i32))
                    .ok_or(NetError::Invalid("stream channel"))?;
                let mut stream = stream.lock().await;
                self.life.check()?;
                self.peer.authorize_async().await?;
                write_frame(&mut *stream, frame).await
            }
        }
    }
}
/// Independently owned receive half, with per-channel priority and volatile
/// coalescing applied before the application consumes frames.
pub struct CarrierReceiver {
    ingress: crate::ingress::Ingress,
    life: Arc<Lifetime>,
    peer: AuthenticatedPeer,
    // Receive is raced against timers and application commands. Keep both the
    // popped result and each live policy check here, so cancelling that future
    // neither loses a reliable frame nor restarts a slow blocking check.
    pending: Option<Result<Frame>>,
    authorization: Option<Pin<Box<dyn Future<Output = Result<()>> + Send>>>,
    read_authorized: bool,
}
impl CarrierReceiver {
    pub const fn peer(&self) -> &AuthenticatedPeer {
        &self.peer
    }
    pub async fn receive(&mut self) -> Result<Frame> {
        self.life.check()?;
        if self.pending.is_none() {
            if !self.read_authorized {
                self.authorize_receive().await?;
                self.read_authorized = true;
            }
            self.pending = Some(bounded(self.ingress.receive()).await.and_then(|r| r));
            self.read_authorized = false;
        }
        // Revocation may occur while ingress is idle. Always check again after
        // dequeue, including when a previous caller left this result pending.
        self.authorize_receive().await?;
        self.life.check()?;
        // There must be no suspension between taking and returning the result.
        let result = self.pending.take().ok_or(NetError::Carrier)?;
        // A discarded predictive chain is recoverable by a fresh keyframe; do
        // not disconnect reliable OPS/CONTROL or input solely for this notice.
        if result.is_err() && !matches!(result, Err(NetError::KeyframeRequired)) {
            self.life.fail(NetError::Carrier);
        }
        result
    }
    async fn authorize_receive(&mut self) -> Result<()> {
        let check = self.authorization.get_or_insert_with(|| {
            let peer = self.peer.clone();
            Box::pin(async move { peer.authorize_async().await })
        });
        let result = check.as_mut().await;
        self.authorization = None;
        if result.is_err() {
            self.pending = None;
            self.read_authorized = false;
            self.life.fail(NetError::Authentication);
        }
        result
    }
    pub fn close(&self) {
        self.life.fail(NetError::Carrier);
    }
}
pub struct TcpCarrier {
    send: CarrierSender,
    receive: CarrierReceiver,
}
impl TcpCarrier {
    pub async fn connect(address: SocketAddr, tls: &PinnedTls) -> Result<Self> {
        let socket = bounded(TcpStream::connect(address)).await??;
        socket.set_nodelay(true)?;
        let (socket, shutdown) = tcp_shutdown_handle(socket)?;
        let name = ServerName::try_from(tls.peer.server_name.clone())
            .map_err(|_| NetError::Authentication)?;
        let stream =
            bounded(tokio_rustls::TlsConnector::from(tls.client.clone()).connect(name, socket))
                .await?
                .map_err(|_| NetError::Authentication)?;
        Self::from_tls(stream.into(), tls, shutdown)
    }
    /// ADB supplies TCP only; mutually authenticated application TLS is mandatory.
    pub async fn accept(listener: &TcpListener, tls: &PinnedTls) -> Result<Self> {
        let (socket, _) = bounded(listener.accept()).await??;
        socket.set_nodelay(true)?;
        let (socket, shutdown) = tcp_shutdown_handle(socket)?;
        let stream = bounded(tokio_rustls::TlsAcceptor::from(tls.server.clone()).accept(socket))
            .await?
            .map_err(|_| NetError::Authentication)?;
        Self::from_tls(stream.into(), tls, shutdown)
    }
    fn from_tls(
        stream: tokio_rustls::TlsStream<TcpStream>,
        tls: &PinnedTls,
        shutdown: std::net::TcpStream,
    ) -> Result<Self> {
        let peer = match &stream {
            tokio_rustls::TlsStream::Client(client) => {
                let (_, connection) = client.get_ref();
                if connection.alpn_protocol() != Some(ALPN) {
                    return Err(NetError::Authentication);
                }
                let binding = connection
                    .export_keying_material([0_u8; 32], b"VW-T1.06a", None)
                    .map_err(|_| NetError::Authentication)?;
                tls.authenticate(connection.peer_certificates(), binding)?
            }
            tokio_rustls::TlsStream::Server(server) => {
                let (_, connection) = server.get_ref();
                if connection.alpn_protocol() != Some(ALPN) {
                    return Err(NetError::Authentication);
                }
                let binding = connection
                    .export_keying_material([0_u8; 32], b"VW-T1.06a", None)
                    .map_err(|_| NetError::Authentication)?;
                tls.authenticate(connection.peer_certificates(), binding)?
            }
        };
        let (read, write) = tokio::io::split(stream);
        let ingress = crate::ingress::Ingress::new();
        let life = Lifetime::new(ingress.clone(), None, None, Some(shutdown));
        life.add(tokio::spawn(read_loop(read, None, ingress.clone())))?;
        Ok(Self {
            send: CarrierSender {
                io: Arc::new(SendIo::Tcp(Mutex::new(write))),
                life: life.clone(),
                peer: peer.clone(),
            },
            receive: CarrierReceiver {
                ingress,
                life,
                peer,
                pending: None,
                authorization: None,
                read_authorized: false,
            },
        })
    }
    pub const fn peer(&self) -> &AuthenticatedPeer {
        self.send.peer()
    }
    pub async fn send(&mut self, frame: &Frame) -> Result<()> {
        self.send.send(frame).await
    }
    pub async fn receive(&mut self) -> Result<Frame> {
        self.receive.receive().await
    }
    pub fn split(self) -> (CarrierSender, CarrierReceiver) {
        (self.send, self.receive)
    }
    pub async fn close(self) -> Result<()> {
        self.send.close();
        Ok(())
    }
}
fn tcp_shutdown_handle(socket: TcpStream) -> Result<(TcpStream, std::net::TcpStream)> {
    let standard = socket.into_std()?;
    let shutdown = standard.try_clone()?;
    Ok((TcpStream::from_std(standard)?, shutdown))
}
pub struct QuicListener {
    endpoint: quinn::Endpoint,
    tls: PinnedTls,
}
impl QuicListener {
    pub fn bind(address: SocketAddr, tls: PinnedTls) -> Result<Self> {
        Ok(Self {
            endpoint: quinn::Endpoint::server(tls.quic_server()?, address)?,
            tls,
        })
    }
    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.endpoint.local_addr()?)
    }
    pub async fn accept(&self) -> Result<QuicCarrier> {
        let incoming = bounded(self.endpoint.accept())
            .await?
            .ok_or(NetError::Carrier)?;
        let connection = bounded(async { incoming.await })
            .await?
            .map_err(|_| NetError::Authentication)?;
        QuicCarrier::establish(self.endpoint.clone(), connection, &self.tls, false).await
    }
}
pub struct QuicCarrier {
    send: CarrierSender,
    receive: CarrierReceiver,
}
impl QuicCarrier {
    pub async fn connect(bind: SocketAddr, address: SocketAddr, tls: &PinnedTls) -> Result<Self> {
        let mut endpoint = quinn::Endpoint::client(bind)?;
        endpoint.set_default_client_config(tls.quic_client()?);
        let connection = bounded(
            endpoint
                .connect(address, &tls.peer.server_name)
                .map_err(|_| NetError::Carrier)?,
        )
        .await?
        .map_err(|_| NetError::Authentication)?;
        Self::establish(endpoint, connection, tls, true).await
    }
    async fn establish(
        endpoint: quinn::Endpoint,
        connection: quinn::Connection,
        tls: &PinnedTls,
        client: bool,
    ) -> Result<Self> {
        let identity = connection.peer_identity().ok_or(NetError::Authentication)?;
        let certs = identity
            .downcast::<Vec<CertificateDer<'static>>>()
            .map_err(|_| NetError::Authentication)?;
        let mut binding = [0_u8; 32];
        connection
            .export_keying_material(&mut binding, b"VW-T1.06a", b"")
            .map_err(|_| NetError::Authentication)?;
        let peer = tls.authenticate(Some(&certs), binding)?;
        let ingress = crate::ingress::Ingress::new();
        let life = Lifetime::new(
            ingress.clone(),
            Some(connection.clone()),
            Some(endpoint),
            None,
        );
        let mut streams = BTreeMap::new();
        for expected in [
            v1::Channel::Control,
            v1::Channel::Ops,
            v1::Channel::Input,
            v1::Channel::Media,
        ] {
            let (mut write, mut read) = if client {
                bounded(connection.open_bi())
                    .await?
                    .map_err(|_| NetError::Carrier)?
            } else {
                bounded(connection.accept_bi())
                    .await?
                    .map_err(|_| NetError::Carrier)?
            };
            if client {
                bounded(write.write_all(&[expected as u8]))
                    .await?
                    .map_err(|_| NetError::Carrier)?;
            } else {
                let mut prefix = [0];
                bounded(read.read_exact(&mut prefix))
                    .await?
                    .map_err(|_| NetError::Carrier)?;
                if prefix[0] != expected as u8 {
                    return Err(NetError::Invalid("stream order"));
                }
            }
            let priority = match expected {
                v1::Channel::Control => 30,
                v1::Channel::Input => 20,
                v1::Channel::Ops => 10,
                _ => 0,
            };
            write
                .set_priority(priority)
                .map_err(|_| NetError::Carrier)?;
            streams.insert(expected as i32, Mutex::new(write));
            life.add(tokio::spawn(read_loop(
                read,
                Some(expected),
                ingress.clone(),
            )))?;
        }
        let datagram_connection = connection.clone();
        let datagram_ingress = ingress.clone();
        life.add(tokio::spawn(async move {
            loop {
                let result = datagram_connection
                    .read_datagram()
                    .await
                    .map_err(|_| NetError::Carrier)
                    .and_then(|b| decode_frame(&b))
                    .and_then(|f| {
                        if f.channel == v1::Channel::Ephemeral {
                            datagram_ingress.push(f)
                        } else {
                            Err(NetError::Invalid("datagram channel"))
                        }
                    });
                if let Err(error) = result {
                    datagram_ingress.fail(error);
                    break;
                }
            }
        }))?;
        let blob_connection = connection.clone();
        let blob_ingress = ingress.clone();
        life.add(tokio::spawn(async move {
            let mut readers = JoinSet::new();
            loop { tokio::select! {
                result = readers.join_next(), if !readers.is_empty() => { if matches!(result, Some(Err(_))) { blob_ingress.fail(NetError::Carrier); break; } },
                result = blob_connection.accept_uni(), if readers.len() < 16 => {
                    match result { Ok(read) => { readers.spawn(read_loop(read, Some(v1::Channel::Blob), blob_ingress.clone())); }, Err(_) => { blob_ingress.fail(NetError::Carrier); break; } }
                }
            } }
        }))?;
        Ok(Self {
            send: CarrierSender {
                io: Arc::new(SendIo::Quic {
                    connection,
                    streams,
                    blobs: Mutex::new(BTreeMap::new()),
                }),
                life: life.clone(),
                peer: peer.clone(),
            },
            receive: CarrierReceiver {
                ingress,
                life,
                peer,
                pending: None,
                authorization: None,
                read_authorized: false,
            },
        })
    }
    pub const fn peer(&self) -> &AuthenticatedPeer {
        self.send.peer()
    }
    pub async fn send(&mut self, frame: &Frame) -> Result<()> {
        self.send.send(frame).await
    }
    pub async fn receive(&mut self) -> Result<Frame> {
        self.receive.receive().await
    }
    pub fn split(self) -> (CarrierSender, CarrierReceiver) {
        (self.send, self.receive)
    }
    pub fn close(&mut self) {
        self.send.close();
    }
}

#[cfg(test)]
#[path = "duplex_tests.rs"]
mod duplex_tests;

#[cfg(test)]
#[path = "carrier_cancellation_tests.rs"]
mod cancellation_tests;
