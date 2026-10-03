//! Pairing/OPS/revocation harness. Output contains only numeric stage markers;
//! certificates, keys, endpoints and QR payloads never go to logs. The phone CLI
//! uses an explicitly ephemeral in-memory identity. Android app persistence is
//! the Keystore callback implementation owned by T1.10.
#[cfg(windows)]
use std::time::{Duration, Instant};
use std::{
    collections::BTreeSet,
    io::{self, BufRead, IsTerminal, Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
};
use vw_model::{AssetId, DeviceId, Id, Project};
use vw_net::{
    CarrierIo, HostSync, LocalHello, Receive, SecureConnection,
    carrier::{PinnedTls, QuicCarrier, TcpCarrier},
    pairing::{
        ClientPairing, CodeDisplay, DeviceIdentity, FingerprintConfirmation, InMemoryTrustStore,
        PairingError, PairingManager, PendingPairing, QrPayload,
    },
};
#[cfg(windows)]
use vw_net::{
    carrier::QuicListener,
    pairing::{PairingHost, PairingListener},
};
use vw_ops::HostSequencer;
use vw_proto::v1::{self, envelope::Body, op::Kind};
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("invalid arguments; see tools/pair-cli/README.md")]
    Usage,
    #[error("private transfer or terminal operation failed")]
    Io,
    #[error(transparent)]
    Pairing(#[from] PairingError),
    #[error("authenticated carrier operation failed")]
    Transport,
    #[error("OPS or revocation verification failed")]
    Verification,
}
type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Copy)]
enum Carrier {
    Quic,
    Tcp,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("PAIR_CLI_FAILED: {error}");
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let command = args.first().ok_or(Error::Usage)?;
    let carrier = if command.ends_with("-tcp") {
        Carrier::Tcp
    } else {
        Carrier::Quic
    };
    match command.trim_end_matches("-tcp") {
        "host-qr" if args.len() == 4 => {
            host(
                address(&args[1])?,
                address(&args[2])?,
                Some(&args[3]),
                carrier,
            )
            .await
        }
        "host-code" if args.len() == 3 => {
            host(address(&args[1])?, address(&args[2])?, None, carrier).await
        }
        "phone-qr" if args.len() == 3 => {
            phone_qr(Path::new(&args[1]), address(&args[2])?, carrier).await
        }
        "phone-code" if args.len() == 3 => {
            phone_code(address(&args[1])?, address(&args[2])?, carrier).await
        }
        _ => Err(Error::Usage),
    }
}
fn address(value: &str) -> Result<SocketAddr> {
    value.parse().map_err(|_| Error::Usage)
}
fn hello(manager: &PairingManager) -> Result<LocalHello> {
    Ok(LocalHello {
        device: manager.identity()?.device().clone(),
        platform: if cfg!(target_os = "android") {
            v1::Platform::Android
        } else {
            v1::Platform::Windows
        },
        app_version: env!("CARGO_PKG_VERSION").into(),
        label: String::new(),
        capabilities: BTreeSet::new(),
    })
}
fn phone_manager() -> Result<PairingManager> {
    Ok(PairingManager::new(Arc::new(InMemoryTrustStore::new(
        DeviceIdentity::generate()?,
    )?))?)
}

#[cfg(windows)]
async fn host(
    pair_address: SocketAddr,
    ops_address: SocketAddr,
    transfer_name: Option<&str>,
    carrier: Carrier,
) -> Result<()> {
    use vw_host_win::trust::DpapiTrustStore;
    if pair_address.port() == 0 || ops_address.port() == 0 || pair_address == ops_address {
        return Err(Error::Usage);
    }
    // Reserve TCP before onboarding can acknowledge pairing. The client may
    // start ordinary TLS immediately; its socket waits in this listener's backlog
    // until the durable trust binding supplies the authenticated TLS config.
    let prebound_tcp = match carrier {
        Carrier::Tcp => Some(
            tokio::net::TcpListener::bind(ops_address)
                .await
                .map_err(|_| Error::Transport)?,
        ),
        Carrier::Quic => None,
    };
    let store = Arc::new(DpapiTrustStore::open_current_user()?);
    let manager = PairingManager::new(store.clone())?;
    let listener = PairingListener::bind(pair_address, &manager.identity()?).await?;
    let _transfer = if let Some(name) = transfer_name {
        let qr = manager.issue_qr(vec![listener.local_addr()?])?;
        Some(store.create_qr_transfer(&qr, name)?)
    } else {
        require_terminal()?;
        let code = manager.issue_code()?;
        // Explicit interactive UI only. is_terminal prevents retained pipe logs.
        eprintln!("Pairing code: {}", code.digits_for_display()?);
        None
    };
    println!("PAIRING_READY");
    let deadline = Instant::now() + Duration::from_secs(300);
    let peer = loop {
        match PairingHost::accept(manager.clone(), &listener).await {
            Ok(PairingHost::Paired(peer)) => break peer,
            Ok(PairingHost::AwaitingConfirmation(pending)) => {
                break confirm_interactively(*pending).await?;
            }
            Err(PairingError::Timeout) if Instant::now() < deadline => {
                println!("PAIRING_WAIT");
            }
            Err(error) => return Err(error.into()),
        }
    };
    drop(_transfer);
    drop(listener);
    println!("PAIR_OK");
    let tls = manager.pinned_tls(&peer)?;
    let listener = match prebound_tcp {
        Some(listener) => OpsListener::Tcp(listener, tls),
        None => {
            OpsListener::Quic(QuicListener::bind(ops_address, tls).map_err(|_| Error::Transport)?)
        }
    };
    println!("OPS_READY");
    let mut connection = SecureConnection::server(listener.accept().await?, &hello(&manager)?)
        .await
        .map_err(|_| Error::Transport)?;
    let host_device = manager.identity()?.device().clone();
    let mut ops = fixture(&host_device)?;
    let txn = match connection.receive(0).await.map_err(|_| Error::Transport)? {
        Receive::Deliver(Body::Txn(txn)) => txn,
        _ => return Err(Error::Verification),
    };
    let result = ops
        .submit(txn, &peer, 1_700_000_000_002)
        .map_err(|_| Error::Verification)?;
    if result.ack.host_seq != 1 {
        return Err(Error::Verification);
    }
    connection
        .session_mut()
        .enqueue(Body::TxnAck(result.ack))
        .map_err(|_| Error::Transport)?;
    if !connection.send_next().await.map_err(|_| Error::Transport)? {
        return Err(Error::Verification);
    }
    match connection.receive(0).await.map_err(|_| Error::Transport)? {
        Receive::Deliver(Body::Ping(ping)) if ping.nonce == 1 && ping.t_sent_ns == 0 => {}
        _ => return Err(Error::Verification),
    }
    println!("OPS_OK count=1");
    manager.revoke(&peer)?;
    println!("HOST_REVOKED");
    // Reuse the already constructed listener/config to prove live revocation,
    // not merely refusal to construct a new configuration.
    match listener.accept().await {
        Err(Error::Pairing(PairingError::Authentication)) => {}
        _ => return Err(Error::Verification),
    }
    println!("COMPLETE role=host ops=1 refused=1");
    Ok(())
}
#[cfg(not(windows))]
async fn host(_: SocketAddr, _: SocketAddr, _: Option<&str>, _: Carrier) -> Result<()> {
    Err(Error::Usage)
}

async fn phone_qr(path: &Path, ops_address: SocketAddr, carrier: Carrier) -> Result<()> {
    let mut transfer = ReadTransfer::open(path)?;
    let qr = QrPayload::decode_qr(&transfer.bytes)?;
    transfer.validated = true;
    let pair_address = *qr.endpoints().first().ok_or(Error::Verification)?;
    let manager = phone_manager()?;
    let peer = ClientPairing::qr(manager.clone(), &qr, pair_address).await?;
    drop(transfer);
    println!("PAIR_OK");
    phone_ops(manager, peer, ops_address, carrier).await
}
async fn phone_code(
    pair_address: SocketAddr,
    ops_address: SocketAddr,
    carrier: Carrier,
) -> Result<()> {
    require_terminal()?;
    eprint!("Enter the 8-digit code: ");
    io::stderr().flush().map_err(|_| Error::Io)?;
    let entry = read_terminal_line(16)?;
    let code = CodeDisplay::parse_for_entry(entry.trim_end_matches(['\r', '\n']))?;
    let manager = phone_manager()?;
    let pending = ClientPairing::code(manager.clone(), &code, pair_address).await?;
    let peer = confirm_interactively(pending).await?;
    println!("PAIR_OK");
    phone_ops(manager, peer, ops_address, carrier).await
}
async fn confirm_interactively(pending: PendingPairing) -> Result<DeviceId> {
    require_terminal()?;
    let fingerprint = pending.fingerprint().display_hex();
    eprintln!("Compare this PC fingerprint on both devices: {fingerprint}");
    eprint!("Type yes only if they match: ");
    io::stderr().flush().map_err(|_| Error::Io)?;
    let answer = read_terminal_line(8)?;
    if answer.trim() != "yes" {
        let _ = pending.decline().await;
        return Err(PairingError::Declined.into());
    }
    Ok(pending
        .confirm(FingerprintConfirmation::user_confirmed(&fingerprint)?)
        .await?)
}
fn require_terminal() -> Result<()> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(Error::Io);
    }
    Ok(())
}
fn read_terminal_line(maximum: u64) -> Result<Zeroizing<String>> {
    let mut bytes = Zeroizing::new(Vec::new());
    io::stdin()
        .lock()
        .take(maximum + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| Error::Io)?;
    if bytes.len() as u64 > maximum {
        return Err(Error::Io);
    }
    Ok(Zeroizing::new(
        std::str::from_utf8(&bytes)
            .map_err(|_| Error::Io)?
            .to_owned(),
    ))
}
async fn phone_ops(
    manager: PairingManager,
    host: DeviceId,
    address: SocketAddr,
    carrier: Carrier,
) -> Result<()> {
    let tls = manager.pinned_tls(&host)?;
    let mut connection =
        SecureConnection::client(connect(address, &tls, carrier).await?, &hello(&manager)?)
            .await
            .map_err(|_| Error::Transport)?;
    let mut ops = fixture(&host)?;
    let txn = transaction(&ops, manager.identity()?.device())?;
    let expected = ops
        .submit(txn.clone(), manager.identity()?.device(), 1_700_000_000_002)
        .map_err(|_| Error::Verification)?
        .ack;
    connection
        .session_mut()
        .enqueue(Body::Txn(txn))
        .map_err(|_| Error::Transport)?;
    if !connection.send_next().await.map_err(|_| Error::Transport)? {
        return Err(Error::Verification);
    }
    let ack = match connection.receive(0).await.map_err(|_| Error::Transport)? {
        Receive::Deliver(Body::TxnAck(ack)) => ack,
        _ => return Err(Error::Verification),
    };
    if ack != expected {
        return Err(Error::Verification);
    }
    connection
        .session_mut()
        .enqueue(Body::Ping(v1::Ping {
            nonce: 1,
            t_sent_ns: 0,
        }))
        .map_err(|_| Error::Transport)?;
    if !connection.send_next().await.map_err(|_| Error::Transport)? {
        return Err(Error::Verification);
    }
    println!("OPS_OK count=1");
    match connect(address, &tls, carrier).await {
        Err(Error::Pairing(PairingError::Authentication)) => {}
        // A TLS 1.3 client may locally complete its last flight before reading
        // the server alert. It must still fail the authenticated app handshake.
        Ok(io) => {
            if SecureConnection::client(io, &hello(&manager)?)
                .await
                .is_ok()
            {
                return Err(Error::Verification);
            }
        }
        _ => return Err(Error::Verification),
    }
    manager.revoke(&host)?;
    if manager.pinned_tls(&host).is_ok() {
        return Err(Error::Verification);
    }
    println!("COMPLETE role=phone ops=1 refused=1 local_revoked=1");
    Ok(())
}

fn project_id() -> Result<Id> {
    Id::from_parts(1_700_000_000_000, [0x31; 10]).map_err(|_| Error::Verification)
}
fn fixture(host: &DeviceId) -> Result<HostSync> {
    HostSync::new(
        HostSequencer::new(
            Project::new(
                project_id()?,
                "Pairing CLI synthetic OPS fixture".into(),
                host.clone(),
            ),
            host.clone(),
        )
        .map_err(|_| Error::Verification)?,
    )
    .map_err(|_| Error::Verification)
}
fn transaction(host: &HostSync, client: &DeviceId) -> Result<v1::Transaction> {
    let bytes = b"pair-cli-synthetic-original";
    Ok(v1::Transaction {
        txn_id: Some(
            Id::from_parts(1_700_000_000_001, [0x32; 10])
                .map_err(|_| Error::Verification)?
                .to_proto(),
        ),
        project_id: Some(project_id()?.to_proto()),
        device_id: client.to_string(),
        base_revision: Some(host.host().revision().map_err(|_| Error::Verification)?),
        created_at_wall_ms: 1_700_000_000_001,
        gesture_id: None,
        ops: vec![v1::Op {
            op_id: Some(v1::OpId {
                device_id: client.to_string(),
                lamport: 1,
            }),
            kind: Some(Kind::AddAsset(v1::AddAsset {
                asset_id: AssetId::hash(bytes).to_string(),
                format: "png".into(),
                width: 1,
                height: 1,
                orientation: 1,
                bit_depth: 8,
                byte_size: bytes.len() as u64,
                color_space: "sRGB".into(),
                source: "import".into(),
                metadata_json: "{}".into(),
                ..Default::default()
            })),
        }],
    })
}
#[cfg(windows)]
enum OpsListener {
    Tcp(tokio::net::TcpListener, PinnedTls),
    Quic(QuicListener),
}
#[cfg(windows)]
impl OpsListener {
    async fn accept(&self) -> Result<CarrierIo> {
        let result = match self {
            Self::Tcp(listener, tls) => TcpCarrier::accept(listener, tls).await.map(CarrierIo::Tcp),
            Self::Quic(listener) => listener.accept().await.map(CarrierIo::Quic),
        };
        result.map_err(transport_error)
    }
}
async fn connect(address: SocketAddr, tls: &PinnedTls, carrier: Carrier) -> Result<CarrierIo> {
    let result = match carrier {
        Carrier::Tcp => TcpCarrier::connect(address, tls).await.map(CarrierIo::Tcp),
        Carrier::Quic => {
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
    };
    result.map_err(transport_error)
}
fn transport_error(error: vw_net::NetError) -> Error {
    if matches!(error, vw_net::NetError::Authentication) {
        PairingError::Authentication.into()
    } else {
        Error::Transport
    }
}

struct ReadTransfer {
    path: PathBuf,
    bytes: Zeroizing<Vec<u8>>,
    validated: bool,
}
impl ReadTransfer {
    fn open(path: &Path) -> Result<Self> {
        let metadata = std::fs::symlink_metadata(path).map_err(|_| Error::Io)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 4096 {
            return Err(Error::Io);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(Error::Io);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(Error::Io);
            }
        }
        let path = std::fs::canonicalize(path).map_err(|_| Error::Io)?;
        for parent in path.ancestors().skip(1) {
            if parent.join(".git").exists() || parent.join("manifest.vwb.json").exists() {
                return Err(Error::Io);
            }
        }
        let mut bytes = Zeroizing::new(Vec::new());
        std::fs::File::open(&path)
            .map_err(|_| Error::Io)?
            .take(4097)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Io)?;
        if bytes.len() > 4096 {
            return Err(Error::Io);
        }
        Ok(Self {
            path,
            bytes,
            validated: false,
        })
    }
}
impl Drop for ReadTransfer {
    fn drop(&mut self) {
        if self.validated {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
