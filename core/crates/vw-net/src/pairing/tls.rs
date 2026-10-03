//! Restricted onboarding TLS. These private verifiers are never installed into
//! an ordinary carrier. A channel exposes only bounded pairing wire messages and
//! cannot construct AuthenticatedPeer; normal traffic requires a fresh pinned TLS
//! connection after both applications have persisted trust.
use super::{
    DeviceIdentity, Fingerprint, PairingError, PairingManager, QrPayload, Result, SERVER_NAME,
    identity::validate_certificate, offer::validate_endpoints,
};
use rustls::{
    DigitallySignedStruct, DistinguishedName, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
    server::danger::{ClientCertVerified, ClientCertVerifier},
};
use std::{fmt, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{OwnedSemaphorePermit, Semaphore},
    time::Instant,
};
use vw_proto::{Message, v1};
use zeroize::Zeroizing;

const ALPN: &[u8] = b"visual-workbench-pairing/1";
const MAX_MESSAGE_BYTES: usize = 20 * 1024;
const MAX_MESSAGES: u8 = 16;
const NETWORK_TIMEOUT: Duration = Duration::from_secs(15);
const CHANNEL_LIFETIME: Duration = Duration::from_secs(300);
pub(crate) const EXPORTER_LABEL: &[u8] = b"EXPORTER-vw-pairing";

fn tls_error() -> rustls::Error {
    rustls::Error::General("pairing certificate rejected".into())
}
fn certificate(
    leaf: &CertificateDer<'_>,
    chain: &[CertificateDer<'_>],
) -> std::result::Result<(), rustls::Error> {
    if !chain.is_empty() {
        return Err(tls_error());
    }
    validate_certificate(leaf.as_ref()).map_err(|_| tls_error())
}
fn signature(
    message: &[u8],
    cert: &CertificateDer<'_>,
    dss: &DigitallySignedStruct,
) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
    rustls::crypto::verify_tls13_signature(
        message,
        cert,
        dss,
        &rustls::crypto::ring::default_provider().signature_verification_algorithms,
    )
}
fn schemes() -> Vec<SignatureScheme> {
    rustls::crypto::ring::default_provider()
        .signature_verification_algorithms
        .supported_schemes()
}

#[derive(Debug)]
struct PairingServerVerifier {
    pin: Option<Fingerprint>,
}
impl ServerCertVerifier for PairingServerVerifier {
    fn verify_server_cert(
        &self,
        leaf: &CertificateDer<'_>,
        chain: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        certificate(leaf, chain)?;
        if self
            .pin
            .is_some_and(|pin| Fingerprint::certificate(leaf.as_ref()).ok() != Some(pin))
        {
            return Err(tls_error());
        }
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(tls_error())
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        signature(message, cert, dss)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        schemes()
    }
}
#[derive(Debug)]
struct PairingClientVerifier;
impl ClientCertVerifier for PairingClientVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(
        &self,
        leaf: &CertificateDer<'_>,
        chain: &[CertificateDer<'_>],
        _: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        certificate(leaf, chain)?;
        Ok(ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(tls_error())
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        signature(message, cert, dss)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        schemes()
    }
}

pub struct PairingListener {
    listener: TcpListener,
    tls: Arc<rustls::ServerConfig>,
    local_certificate: Vec<u8>,
    permits: Arc<Semaphore>,
}
impl PairingListener {
    /// Bind only an explicitly selected local address. Port zero is allowed for
    /// an OS-assigned port; the advertised endpoint must use local_addr().port().
    pub async fn bind(address: SocketAddr, identity: &DeviceIdentity) -> Result<Self> {
        validate_endpoints(&[SocketAddr::new(address.ip(), address.port().max(1))])?;
        let identity = identity.transport_identity()?;
        let local_certificate = identity.certificate.to_vec();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut tls = rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|_| PairingError::Authentication)?
            .with_client_cert_verifier(Arc::new(PairingClientVerifier))
            .with_single_cert(vec![identity.certificate], identity.key)
            .map_err(|_| PairingError::Authentication)?;
        tls.alpn_protocols = vec![ALPN.to_vec()];
        tls.max_early_data_size = 0;
        tls.send_tls13_tickets = 0;
        let listener = TcpListener::bind(address)
            .await
            .map_err(|_| PairingError::Connection)?;
        Ok(Self {
            listener,
            tls: Arc::new(tls),
            local_certificate,
            permits: Arc::new(Semaphore::new(4)),
        })
    }
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.listener
            .local_addr()
            .map_err(|_| PairingError::Connection)
    }
    pub async fn accept(&self) -> Result<PairingChannel> {
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| PairingError::Capacity)?;
        let (socket, _) = tokio::time::timeout(NETWORK_TIMEOUT, self.listener.accept())
            .await
            .map_err(|_| PairingError::Timeout)?
            .map_err(|_| PairingError::Connection)?;
        socket
            .set_nodelay(true)
            .map_err(|_| PairingError::Connection)?;
        let stream = tokio::time::timeout(
            NETWORK_TIMEOUT,
            tokio_rustls::TlsAcceptor::from(self.tls.clone()).accept(socket),
        )
        .await
        .map_err(|_| PairingError::Timeout)?
        .map_err(|_| PairingError::Authentication)?;
        PairingChannel::established(stream.into(), self.local_certificate.clone(), Some(permit))
    }
}
impl fmt::Debug for PairingListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PairingListener([REDACTED])")
    }
}

pub struct PairingChannel {
    stream: tokio_rustls::TlsStream<TcpStream>,
    exporter: Zeroizing<[u8; 32]>,
    local_certificate: Vec<u8>,
    peer_certificate: Vec<u8>,
    expires: Instant,
    sent: u8,
    received: u8,
    failed: bool,
    _permit: Option<OwnedSemaphorePermit>,
}
impl PairingChannel {
    pub(crate) async fn connect_qr(
        manager: &PairingManager,
        qr: &QrPayload,
        address: SocketAddr,
    ) -> Result<Self> {
        qr.validate(manager.clock.now_ms()?)?;
        if !qr.endpoints.contains(&address) {
            return Err(PairingError::Invalid("QR endpoint"));
        }
        Self::connect(&manager.identity()?, address, Some(qr.certificate_sha256)).await
    }
    pub(crate) async fn connect_code(
        manager: &PairingManager,
        address: SocketAddr,
    ) -> Result<Self> {
        Self::connect(&manager.identity()?, address, None).await
    }
    async fn connect(
        identity: &DeviceIdentity,
        address: SocketAddr,
        pin: Option<Fingerprint>,
    ) -> Result<Self> {
        validate_endpoints(&[address])?;
        let identity = identity.transport_identity()?;
        let local_certificate = identity.certificate.to_vec();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut tls = rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|_| PairingError::Authentication)?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PairingServerVerifier { pin }))
            .with_client_auth_cert(vec![identity.certificate], identity.key)
            .map_err(|_| PairingError::Authentication)?;
        tls.alpn_protocols = vec![ALPN.to_vec()];
        tls.enable_early_data = false;
        tls.resumption = rustls::client::Resumption::disabled();
        let socket = tokio::time::timeout(NETWORK_TIMEOUT, TcpStream::connect(address))
            .await
            .map_err(|_| PairingError::Timeout)?
            .map_err(|_| PairingError::Connection)?;
        socket
            .set_nodelay(true)
            .map_err(|_| PairingError::Connection)?;
        let name = ServerName::try_from(SERVER_NAME).map_err(|_| PairingError::Authentication)?;
        let stream = tokio::time::timeout(
            NETWORK_TIMEOUT,
            tokio_rustls::TlsConnector::from(Arc::new(tls)).connect(name, socket),
        )
        .await
        .map_err(|_| PairingError::Timeout)?
        .map_err(|_| PairingError::Authentication)?;
        Self::established(stream.into(), local_certificate, None)
    }
    fn established(
        stream: tokio_rustls::TlsStream<TcpStream>,
        local_certificate: Vec<u8>,
        permit: Option<OwnedSemaphorePermit>,
    ) -> Result<Self> {
        let (_, connection) = stream.get_ref();
        if connection.protocol_version() != Some(rustls::ProtocolVersion::TLSv1_3)
            || connection.alpn_protocol() != Some(ALPN)
        {
            return Err(PairingError::Authentication);
        }
        let certificates = connection
            .peer_certificates()
            .ok_or(PairingError::Authentication)?;
        if certificates.len() != 1 {
            return Err(PairingError::Authentication);
        }
        let peer_certificate = certificates[0].to_vec();
        validate_certificate(&peer_certificate)?;
        let exporter =
            match &stream {
                tokio_rustls::TlsStream::Client(client) => client
                    .get_ref()
                    .1
                    .export_keying_material([0; 32], EXPORTER_LABEL, None),
                tokio_rustls::TlsStream::Server(server) => server
                    .get_ref()
                    .1
                    .export_keying_material([0; 32], EXPORTER_LABEL, None),
            }
            .map_err(|_| PairingError::Authentication)?;
        Ok(Self {
            stream,
            exporter: Zeroizing::new(exporter),
            local_certificate,
            peer_certificate,
            expires: Instant::now() + CHANNEL_LIFETIME,
            sent: 0,
            received: 0,
            failed: false,
            _permit: permit,
        })
    }
    pub(crate) fn exporter(&self) -> &[u8; 32] {
        &self.exporter
    }
    pub(crate) fn peer_certificate(&self) -> &[u8] {
        &self.peer_certificate
    }
    pub(crate) fn local_certificate(&self) -> &[u8] {
        &self.local_certificate
    }
    fn timeout(&self, confirmation: bool) -> Result<Duration> {
        if self.failed {
            return Err(PairingError::Connection);
        }
        let remaining = self
            .expires
            .checked_duration_since(Instant::now())
            .ok_or(PairingError::Expired)?;
        Ok(remaining.min(if confirmation {
            Duration::from_secs(60)
        } else {
            NETWORK_TIMEOUT
        }))
    }
    pub(crate) async fn send(&mut self, wire: Wire) -> Result<()> {
        let timeout = self.timeout(false)?;
        if self.sent >= MAX_MESSAGES {
            self.failed = true;
            return Err(PairingError::Invalid("pairing message count"));
        }
        let (tag, payload) = wire.encode();
        let length = payload.len().checked_add(1).ok_or(PairingError::Capacity)?;
        if length > MAX_MESSAGE_BYTES {
            self.failed = true;
            return Err(PairingError::Invalid("pairing message size"));
        }
        self.sent += 1;
        let result = tokio::time::timeout(timeout, async {
            self.stream
                .write_all(&(length as u32).to_be_bytes())
                .await
                .map_err(|_| PairingError::Connection)?;
            self.stream
                .write_all(&[tag])
                .await
                .map_err(|_| PairingError::Connection)?;
            self.stream
                .write_all(&payload)
                .await
                .map_err(|_| PairingError::Connection)?;
            self.stream
                .flush()
                .await
                .map_err(|_| PairingError::Connection)
        })
        .await
        .map_err(|_| PairingError::Timeout)
        .and_then(|value| value);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    pub(crate) async fn receive(&mut self, confirmation: bool) -> Result<Wire> {
        let timeout = self.timeout(confirmation)?;
        if self.received >= MAX_MESSAGES {
            self.failed = true;
            return Err(PairingError::Invalid("pairing message count"));
        }
        self.received += 1;
        let result = tokio::time::timeout(timeout, async {
            let mut header = [0; 4];
            self.stream
                .read_exact(&mut header)
                .await
                .map_err(|_| PairingError::Connection)?;
            let length = u32::from_be_bytes(header) as usize;
            if !(1..=MAX_MESSAGE_BYTES).contains(&length) {
                return Err(PairingError::Invalid("pairing message size"));
            }
            let mut bytes = vec![0; length];
            self.stream
                .read_exact(&mut bytes)
                .await
                .map_err(|_| PairingError::Connection)?;
            Wire::decode(bytes[0], &bytes[1..])
        })
        .await
        .map_err(|_| PairingError::Timeout)
        .and_then(|value| value);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    pub async fn close(mut self) -> Result<()> {
        tokio::time::timeout(NETWORK_TIMEOUT, self.stream.shutdown())
            .await
            .map_err(|_| PairingError::Timeout)?
            .map_err(|_| PairingError::Connection)
    }
}
impl fmt::Debug for PairingChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PairingChannel([REDACTED])")
    }
}
pub(crate) enum Wire {
    Proof(v1::PairingProof),
    Pake(v1::PakeMessage),
    Result(v1::PairingResult),
}
impl Wire {
    fn encode(self) -> (u8, Vec<u8>) {
        match self {
            Self::Proof(message) => (1, message.encode_to_vec()),
            Self::Pake(message) => (2, message.encode_to_vec()),
            Self::Result(message) => (3, message.encode_to_vec()),
        }
    }
    fn decode(tag: u8, bytes: &[u8]) -> Result<Self> {
        match tag {
            1 => v1::PairingProof::decode(bytes).map(Self::Proof),
            2 => v1::PakeMessage::decode(bytes).map(Self::Pake),
            3 => v1::PairingResult::decode(bytes).map(Self::Result),
            _ => return Err(PairingError::Invalid("pairing message type")),
        }
        .map_err(|_| PairingError::Invalid("pairing message"))
    }
}
