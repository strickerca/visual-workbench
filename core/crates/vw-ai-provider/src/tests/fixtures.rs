use super::super::*;
use base64::Engine;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use vw_ai::{budget::*, config::*, *};
use zeroize::Zeroizing;
pub(super) type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

pub(super) fn temp() -> std::io::Result<tempfile::TempDir> {
    let base = if cfg!(target_os = "android") {
        std::env::current_dir()?
    } else {
        std::env::temp_dir()
    };
    tempfile::Builder::new()
        .prefix("vw-provider-owned-")
        .tempdir_in(base)
}
pub(super) fn prepared(
    intent: u8,
    response_limit: usize,
) -> std::result::Result<Prepared, Box<dyn std::error::Error>> {
    let source = png(16, 16)?;
    let mask = vw_mask::Mask::from_dense(vw_mask::Size::new(16, 16)?, &[255; 256])?;
    Ok(Prepared::new(
        &source,
        &[mask],
        PrepareOptions {
            provider: ProviderConfig {
                schema: 1,
                kind: ProviderKind::OpenAiImageEdits,
                verified_on: "2026-10-03".into(),
                endpoint: http::ENDPOINT.into(),
                model: "offline-fixture".into(),
                quality: "offline".into(),
                capabilities: Capabilities {
                    mask_support: MaskSupport::AlphaPngGuidance,
                    max_long_edge: 256,
                    min_pixels: 256,
                    max_pixels: 65536,
                    size_multiple: 16,
                    max_aspect_ratio: 3,
                    formats: vec![Format::Png],
                    max_image_bytes_exclusive: MAX_ENCODED,
                    max_mask_bytes_exclusive: MAX_ENCODED,
                    experimental_above_pixels: 65536,
                },
                prices: Prices {
                    text_input_microusd_per_million: 1_000_000,
                    image_input_microusd_per_million: 2_000_000,
                    image_output_microusd_per_million: 3_000_000,
                },
                timeout_seconds: 2,
                max_response_bytes: response_limit,
            },
            revision: RevisionBinding {
                project_id: vw_model::Id::from_parts(1, [0; 10])?,
                host_seq: 1,
                state_hash: "a".repeat(64),
            },
            intent_id: vw_model::Id::from_parts(1, [intent; 10])?,
            instructions: vec![Instruction {
                role: InstructionRole::Change,
                text: "Synthetic fixture instruction".into(),
            }],
            feather_px: 0,
            estimated_tokens: Some(Tokens {
                text_input: 10,
                image_input: 20,
                image_output: 30,
            }),
            source_policy: SourcePolicy {
                assume_untagged_srgb: true,
                allow_16bit_provider_copy: false,
            },
            limits: Limits::default(),
        },
        &NeverCancel,
    )?)
}
fn png(width: u32, height: u32) -> std::result::Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&vec![128; (width * height * 4) as usize])?;
        writer.finish()?;
    }
    Ok(bytes)
}
pub(super) fn response(
    p: &Prepared,
    usage: bool,
) -> std::result::Result<Vec<u8>, Box<dyn std::error::Error>> {
    let crop = p.review().crop();
    let image =
        base64::engine::general_purpose::STANDARD.encode(png(crop.model_width, crop.model_height)?);
    let usage = if usage {
        r#", "usage":{"input_tokens":30,"input_tokens_details":{"text_tokens":10,"image_tokens":20},"output_tokens":30,"total_tokens":60}"#
    } else {
        ""
    };
    Ok(format!("{{\"data\":[{{\"b64_json\":\"{image}\"}}]{usage}}}").into_bytes())
}
pub(super) fn authorized_bytes(
    p: &Prepared,
    path: &Path,
) -> std::result::Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut ledger = SqliteBudgetLedger::create(path)?;
    ledger.reserve(
        p.review(),
        Confirmation::explicit_send(p.review(), 20000, false)?,
        BudgetPolicy::default(),
    )?;
    Ok(
        p.authorize(ledger.begin_attempt(p.review())?, &NeverCancel)?
            .body()
            .to_vec(),
    )
}
#[derive(Default)]
pub(super) struct FakeSecret {
    pub calls: AtomicUsize,
    pub missing: bool,
}
impl SecretProvider for FakeSecret {
    fn load(
        &self,
        _: &dyn Cancellation,
        _: Instant,
    ) -> std::result::Result<Secret, CredentialFailure> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        if self.missing {
            return Err(CredentialFailure::Missing);
        }
        Secret::from_protected_bytes(Zeroizing::new(b"synthetic-fixture-only".to_vec()))
            .map_err(|_| CredentialFailure::Invalid)
    }
}
pub(super) fn adapter(
    server: &Server,
    secrets: Arc<dyn SecretProvider>,
) -> crate::Result<OpenAiAdapter> {
    Ok(OpenAiAdapter {
        client: http::test_client()?,
        secrets,
        fixture_target: Some(server.target.clone()),
    })
}
#[derive(Clone, Copy)]
pub(super) enum ReplyMode {
    Normal,
    Chunks,
    Redirect,
    RateLimit,
    Truncated,
    Stall,
}
pub(super) struct Observed {
    pub requests: usize,
    pub head: String,
    pub body: Vec<u8>,
}
pub(super) struct Server {
    target: String,
    thread: Option<JoinHandle<std::io::Result<Observed>>>,
    received: std::sync::mpsc::Receiver<()>,
    release: std::sync::mpsc::SyncSender<()>,
}
impl Server {
    pub(super) fn start(reply: Vec<u8>, mode: ReplyMode) -> std::io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let (received_tx, received) = std::sync::mpsc::sync_channel(1);
        let (release, release_rx) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::spawn(move || {
            let limit = Instant::now() + Duration::from_secs(4);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < limit =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => return Err(error),
                }
            };
            // Windows accepted sockets inherit the listener nonblocking flag.
            // This fixture parses with blocking reads under an explicit deadline.
            stream.set_nonblocking(false)?;
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            stream.set_write_timeout(Some(Duration::from_secs(2)))?;
            let (head, body) = read_request(&mut stream)?;
            let _ = received_tx.send(());
            if matches!(mode, ReplyMode::Stall) {
                let _ = release_rx.recv_timeout(Duration::from_secs(3));
                return Ok(Observed {
                    requests: 1,
                    head,
                    body,
                });
            }
            let status = match mode {
                ReplyMode::Redirect => "307 Temporary Redirect",
                ReplyMode::RateLimit => "429 Too Many Requests",
                _ => "200 OK",
            };
            let extra = match mode {
                ReplyMode::Redirect => format!("Location: http://{address}/unexpected\r\n"),
                ReplyMode::RateLimit => "Retry-After: 0\r\n".into(),
                _ => String::new(),
            };
            let framing = match mode {
                ReplyMode::Chunks => "Transfer-Encoding: chunked\r\n".into(),
                ReplyMode::Truncated => format!("Content-Length: {}\r\n", reply.len() + 100),
                _ => format!("Content-Length: {}\r\n", reply.len()),
            };
            stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{framing}{extra}Connection: close\r\n\r\n").as_bytes())?;
            if matches!(mode, ReplyMode::Chunks) {
                for chunk in reply.chunks(7) {
                    if write!(stream, "{:x}\r\n", chunk.len())
                        .and_then(|_| stream.write_all(chunk))
                        .and_then(|_| stream.write_all(b"\r\n"))
                        .is_err()
                    {
                        break;
                    }
                }
                let _ = stream.write_all(b"0\r\n\r\n");
            } else {
                let _ = stream.write_all(&reply);
            }
            drop(stream);
            let retry_limit = Instant::now() + Duration::from_millis(150);
            let mut requests = 1;
            while Instant::now() < retry_limit {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        requests += 1;
                        let _ = stream.write_all(
                            b"HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(Observed {
                requests,
                head,
                body,
            })
        });
        Ok(Self {
            target: format!("http://{address}/fixture"),
            thread: Some(thread),
            received,
            release,
        })
    }
    pub(super) fn finish(mut self) -> std::result::Result<Observed, Box<dyn std::error::Error>> {
        let _ = self.release.try_send(());
        let thread = self.thread.take().ok_or("owned server handle")?;
        Ok(thread.join().map_err(|_| "fixture server panicked")??)
    }
    pub(super) fn wait_received(
        &self,
    ) -> std::result::Result<(), std::sync::mpsc::RecvTimeoutError> {
        self.received.recv_timeout(Duration::from_secs(2))
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.release.try_send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn read_request(stream: &mut TcpStream) -> std::io::Result<(String, Vec<u8>)> {
    let mut bytes = Vec::new();
    let mut one = [0; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() >= 16 * 1024 {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        stream.read_exact(&mut one)?;
        bytes.push(one[0]);
    }
    let head = String::from_utf8(bytes).map_err(|_| std::io::ErrorKind::InvalidData)?;
    let length = head
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, size)| size.trim().parse::<usize>().ok())
        })
        .ok_or(std::io::ErrorKind::InvalidData)?;
    if length > 1024 * 1024 {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    Ok((head, body))
}
