use serde::Serialize;
use std::{
    fs::OpenOptions,
    io::Write,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::net::{TcpListener, TcpStream};
use vw_transport_bench::{
    Error, Result, Stats, StreamResult, bounded, finish, measure_stream, serve_stream, summarize,
};

const NAME: &str = "vw-bench.invalid";
const ALPN: &[u8] = b"vw-transport-spike/1";

fn create(path: &Path) -> Result<std::fs::File> {
    Ok(OpenOptions::new().write(true).create_new(true).open(path)?)
}
fn json_file(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut file = create(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    Ok(())
}
fn address(value: &str, allow_lan: bool) -> Result<SocketAddr> {
    let addr: SocketAddr = value.parse().map_err(|_| Error::Invalid)?;
    if addr.ip().is_unspecified()
        || addr.ip().is_multicast()
        || (!allow_lan && !addr.ip().is_loopback())
    {
        return Err(Error::Invalid);
    }
    Ok(addr)
}
fn transport_config() -> quinn::TransportConfig {
    let mut config = quinn::TransportConfig::default();
    config.max_concurrent_bidi_streams(1u8.into());
    config.max_concurrent_uni_streams(0u8.into());
    config.datagram_receive_buffer_size(Some(128 * 1024));
    config.datagram_send_buffer_size(128 * 1024);
    config.keep_alive_interval(Some(Duration::from_secs(2)));
    config
}
fn server_config(folder: &Path) -> Result<quinn::ServerConfig> {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec![NAME.into()]).map_err(|_| Error::Certificate)?;
    // Only public DER is written. Throwaway private key bytes remain in memory.
    create(&folder.join("server.der"))?.write_all(cert.der().as_ref())?;
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der());
    let mut tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .map_err(|_| Error::Certificate)?
    .with_no_client_auth()
    .with_single_cert(vec![cert.der().clone()], key.into())
    .map_err(|_| Error::Certificate)?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let tls =
        quinn::crypto::rustls::QuicServerConfig::try_from(tls).map_err(|_| Error::Certificate)?;
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(tls));
    config.transport = Arc::new(transport_config());
    Ok(config)
}
fn client_config(cert_path: &Path) -> Result<quinn::ClientConfig> {
    if std::fs::metadata(cert_path)?.len() > 65536 {
        return Err(Error::Certificate);
    }
    let cert = rustls::pki_types::CertificateDer::from(std::fs::read(cert_path)?);
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert).map_err(|_| Error::Certificate)?;
    let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .map_err(|_| Error::Certificate)?
    .with_root_certificates(roots)
    .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let tls =
        quinn::crypto::rustls::QuicClientConfig::try_from(tls).map_err(|_| Error::Certificate)?;
    let mut config = quinn::ClientConfig::new(Arc::new(tls));
    config.transport_config(Arc::new(transport_config()));
    Ok(config)
}

async fn datagram_server(connection: &quinn::Connection) -> Result<()> {
    loop {
        // The stream benchmark runs before datagrams and can legitimately take
        // more than IO_DEADLINE. The enclosing session deadline bounds this
        // wait; a per-datagram idle timeout would truncate healthy stream work.
        let packet = connection.read_datagram().await.map_err(|_| Error::Quic)?;
        if packet.len() != 1024 {
            return Err(Error::Invalid);
        }
        connection.send_datagram(packet).map_err(|_| Error::Quic)?;
    }
}
#[derive(Serialize)]
struct DatagramResult {
    bytes: usize,
    scheduled_hz: u32,
    sent: usize,
    received: usize,
    duplicate_echoes: usize,
    missing_roundtrip_echoes: usize,
    loss_fraction: f64,
    rtt: Option<Stats>,
    raw_rtt_ms: Vec<f64>,
    send_lateness: Stats,
    interpretation: &'static str,
}
async fn datagrams(connection: &quinn::Connection, count: usize) -> Result<DatagramResult> {
    if !(1..=12000).contains(&count) {
        return Err(Error::Invalid);
    }
    let start = tokio::time::Instant::now();
    let mut times = vec![None; count];
    let mut seen = vec![false; count];
    let mut sent = 0;
    let mut duplicates = 0;
    let mut samples = Vec::new();
    let mut late = Vec::new();
    let stop = start + Duration::from_secs_f64(count as f64 / 120.0 + 2.0);
    while samples.len() < count {
        let due = start + Duration::from_secs_f64(sent as f64 / 120.0);
        tokio::select! {
            _ = tokio::time::sleep_until(stop) => break,
            _ = tokio::time::sleep_until(due), if sent < count => {
                let mut payload = vec![0x5a; 1024];
                payload[..8].copy_from_slice(&(sent as u64).to_be_bytes());
                times[sent] = Some(Instant::now());
                late.push(tokio::time::Instant::now().saturating_duration_since(due).as_secs_f64() * 1000.0);
                connection.send_datagram(payload.into()).map_err(|_| Error::Quic)?;
                sent += 1;
            }
            packet = connection.read_datagram() => {
                let packet = packet.map_err(|_| Error::Quic)?;
                if packet.len() != 1024 || packet[8..].iter().any(|b| *b != 0x5a) { return Err(Error::Corrupt); }
                let seq = u64::from_be_bytes(packet[..8].try_into().map_err(|_| Error::Corrupt)?) as usize;
                if seq >= sent { return Err(Error::Corrupt); }
                if seen[seq] { duplicates += 1; continue; }
                let time = times[seq].ok_or(Error::Corrupt)?;
                seen[seq] = true;
                samples.push(time.elapsed().as_secs_f64() * 1000.0);
            }
        }
    }
    if sent != count {
        return Err(Error::Timeout);
    }
    Ok(DatagramResult {
        bytes: 1024,
        scheduled_hz: 120,
        sent,
        received: samples.len(),
        duplicate_echoes: duplicates,
        missing_roundtrip_echoes: sent - samples.len(),
        loss_fraction: (sent - samples.len()) as f64 / sent as f64,
        rtt: if samples.is_empty() {
            None
        } else {
            Some(summarize(&samples)?)
        },
        raw_rtt_ms: samples,
        send_lateness: summarize(&late)?,
        interpretation: "Echo RTT on one monotonic clock; loss includes either direction; not one-way latency or device scheduling proof.",
    })
}

async fn serve(kind: &str, bind: SocketAddr, folder: &Path) -> Result<()> {
    std::fs::create_dir(folder)?;
    if kind == "tcp" {
        let listener = TcpListener::bind(bind).await?;
        json_file(
            &folder.join("ready.json"),
            &serde_json::json!({"schema":1,"transport":kind,"port":listener.local_addr()?.port()}),
        )?;
        eprintln!("TCP listener ready; one bounded synthetic session");
        let (socket, _) = bounded(listener.accept()).await??;
        socket.set_nodelay(true)?;
        let (read, write) = socket.into_split();
        serve_stream(read, write).await?;
    } else if kind == "quic" {
        let endpoint = quinn::Endpoint::server(server_config(folder)?, bind)?;
        json_file(
            &folder.join("ready.json"),
            &serde_json::json!({"schema":1,"transport":kind,"port":endpoint.local_addr()?.port()}),
        )?;
        eprintln!("QUIC listener ready; public certificate exported, private key memory only");
        let incoming = bounded(endpoint.accept()).await?.ok_or(Error::Quic)?;
        let connection = bounded(async { incoming.await })
            .await?
            .map_err(|_| Error::Quic)?;
        let (write, read) = bounded(connection.accept_bi())
            .await?
            .map_err(|_| Error::Quic)?;
        tokio::select! {
            result = serve_stream(read, write) => result?,
            result = datagram_server(&connection) => result?,
        }
        // Let the client receive DONE before closing; no unbounded draining.
        let _ = tokio::time::timeout(Duration::from_secs(3), connection.closed()).await;
        endpoint.close(0u32.into(), b"done");
        bounded(endpoint.wait_idle()).await?;
    } else {
        return Err(Error::Invalid);
    }
    json_file(
        &folder.join("complete.json"),
        &serde_json::json!({"schema":1,"completed":true,"transport":kind}),
    )?;
    Ok(())
}

#[derive(Serialize)]
struct Report {
    schema: u32,
    completed: bool,
    transport: String,
    profile: String,
    stream: StreamResult,
    datagrams: Option<DatagramResult>,
    environment: &'static str,
    encryption: &'static str,
}
async fn run_client(
    kind: &str,
    peer: SocketAddr,
    cert: &Path,
    path: &Path,
    profile: &str,
) -> Result<()> {
    // Reserve a new file before traffic; interrupted runs cannot claim completion.
    let mut file = create(path)?;
    let (iterations, bulk, count) = match profile {
        "full" => (1000, 256 * 1024 * 1024, 1200),
        "smoke" => (10, 1024 * 1024, 120),
        _ => return Err(Error::Invalid),
    };
    let (stream, datagrams, encryption) = if kind == "tcp" {
        let mut socket = bounded(TcpStream::connect(peer)).await??;
        socket.set_nodelay(true)?;
        let (mut read, mut write) = socket.split();
        let result = measure_stream(&mut read, &mut write, iterations, bulk).await?;
        finish(&mut read, &mut write).await?;
        (
            result,
            None,
            "plain TCP benchmark only; not production TLS/mux",
        )
    } else if kind == "quic" {
        let bind = SocketAddr::new(
            if peer.is_ipv4() {
                IpAddr::V4(Ipv4Addr::UNSPECIFIED)
            } else {
                IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED)
            },
            0,
        );
        let mut endpoint = quinn::Endpoint::client(bind)?;
        endpoint.set_default_client_config(client_config(cert)?);
        let connection = bounded(endpoint.connect(peer, NAME).map_err(|_| Error::Quic)?)
            .await?
            .map_err(|_| Error::Quic)?;
        let (mut write, mut read) = bounded(connection.open_bi())
            .await?
            .map_err(|_| Error::Quic)?;
        let result = measure_stream(&mut read, &mut write, iterations, bulk).await?;
        let dg = datagrams(&connection, count).await?;
        finish(&mut read, &mut write).await?;
        connection.close(0u32.into(), b"done");
        bounded(endpoint.wait_idle()).await?;
        (
            result,
            Some(dg),
            "TLS1.3; ring; trust only the supplied throwaway public certificate",
        )
    } else {
        return Err(Error::Invalid);
    };
    let report = Report {
        schema: 1,
        completed: true,
        transport: kind.into(),
        profile: profile.into(),
        stream,
        datagrams,
        environment: "Caller must bind this receipt to carrier, hardware and build hashes; loopback is not phone evidence.",
        encryption,
    };
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    eprintln!("Benchmark completed with verified payloads; report written");
    Ok(())
}

async fn dispatch(args: &[String]) -> Result<()> {
    let lan = args.last().is_some_and(|v| v == "--allow-lan");
    let args = if lan { &args[..args.len() - 1] } else { args };
    match args {
        [role, kind, bind, folder, seconds] if role == "serve" => {
            let seconds: u64 = seconds.parse().map_err(|_| Error::Invalid)?;
            if !(30..=1800).contains(&seconds) {
                return Err(Error::Invalid);
            }
            tokio::time::timeout(
                Duration::from_secs(seconds),
                serve(kind, address(bind, lan)?, Path::new(folder)),
            )
            .await
            .map_err(|_| Error::Timeout)?
        }
        [role, kind, peer, cert, path, profile] if role == "run" => tokio::time::timeout(
            Duration::from_secs(1200),
            run_client(
                kind,
                address(peer, lan)?,
                Path::new(cert),
                Path::new(path),
                profile,
            ),
        )
        .await
        .map_err(|_| Error::Timeout)?,
        _ => Err(Error::Invalid),
    }
}
fn main() {
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(Error::from)
        .and_then(|runtime| {
            runtime.block_on(dispatch(&std::env::args().skip(1).collect::<Vec<_>>()))
        });
    if let Err(error) = result {
        eprintln!(
            "transport-bench: {error}. Usage: serve <tcp|quic> <bind> <new-dir> <30..1800 seconds> [--allow-lan] | run <tcp|quic> <peer> <public-cert|-> <new.json> <smoke|full> [--allow-lan]"
        );
        std::process::exit(1);
    }
}
