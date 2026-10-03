use super::{ConnectionAssistError as Error, ConnectionRequest, Result};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;

const MAX_REPLY: usize = 65_535;
const MAX_DEVICES: usize = 64;
pub(super) const CYCLE: Duration = Duration::from_millis(600);
pub(super) const POLL: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum AdbDeviceState {
    Available,
    Offline,
    Unauthorized,
    Unavailable,
}
#[derive(Clone, uniffi::Record)]
pub struct AdbDevice {
    pub serial: String,
    pub state: AdbDeviceState,
}
#[derive(Clone, uniffi::Record)]
pub struct AdbToolInfo {
    pub protocol_version: u32,
}
#[derive(Clone, uniffi::Record)]
pub struct AdbReverseConfig {
    pub adb_path: String,
    pub serial: String,
    pub phone_port: u16,
    pub host_port: u16,
}
impl AdbReverseConfig {
    pub(super) fn validate(&self) -> Result<()> {
        validate_path(&self.adb_path)?;
        validate_serial(&self.serial)?;
        if self.phone_port < 1024 || self.host_port < 1024 {
            return Err(Error::InvalidSelection);
        }
        Ok(())
    }
}
pub(super) fn validate_path(path: &str) -> Result<()> {
    if path.len() > 32_000
        || path.contains('\0')
        || !Path::new(path).is_absolute()
        || !Path::new(path)
            .file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("adb.exe"))
    {
        return Err(Error::InvalidAdbTool);
    }
    Ok(())
}
fn validate_serial(serial: &str) -> Result<()> {
    // The exact selected token is placed after host:transport:. Control bytes,
    // whitespace and protocol separators are refused, not escaped/interpreted.
    if serial.is_empty()
        || serial.len() > 128
        || !serial
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.:[]%".contains(&c))
    {
        Err(Error::InvalidSelection)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AdbReverseState {
    Checking,
    ServerUnavailable,
    UnsupportedServer,
    MissingDevice,
    Offline,
    Unauthorized,
    MappingCreated,
    ExistingMapping,
    ContestedMapping,
    Refused,
    TimedOut,
    InvalidReply,
    Stopped,
}
#[derive(Clone, uniffi::Record)]
pub struct AdbReverseSnapshot {
    pub state: AdbReverseState,
    pub created_count: u64,
    pub cycle_millis: u32,
    /// True once this watch has successfully created a mapping. Stop does not
    /// delete it: another client may have adopted/replaced it in the meantime.
    pub created_mapping_may_remain: bool,
    pub phone_port: u16,
    pub host_port: u16,
}

pub(super) fn parse_devices(bytes: &[u8]) -> Result<Vec<AdbDevice>> {
    if bytes.len() > MAX_REPLY {
        return Err(Error::AdbProtocol);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Error::AdbProtocol)?;
    let mut found = BTreeSet::new();
    let mut result = Vec::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        if result.len() >= MAX_DEVICES {
            return Err(Error::AdbProtocol);
        }
        let (serial, state) = line.split_once('\t').ok_or(Error::AdbProtocol)?;
        validate_serial(serial).map_err(|_| Error::AdbProtocol)?;
        if !found.insert(serial) || state.is_empty() || state.len() > 64 {
            return Err(Error::AdbProtocol);
        }
        let state = match state {
            "device" => AdbDeviceState::Available,
            "offline" => AdbDeviceState::Offline,
            "unauthorized" => AdbDeviceState::Unauthorized,
            _ => AdbDeviceState::Unavailable,
        };
        result.push(AdbDevice {
            serial: serial.into(),
            state,
        });
    }
    Ok(result)
}

#[derive(Debug, PartialEq, Eq)]
enum Mapping {
    Missing,
    Same,
    Other,
}
fn mapping(bytes: &[u8], phone: u16, host: u16) -> Result<Mapping> {
    if bytes.len() > MAX_REPLY {
        return Err(Error::AdbProtocol);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Error::AdbProtocol)?;
    let mut result = Mapping::Missing;
    let mut count = 0;
    let selected = format!("tcp:{phone}");
    let destination = format!("tcp:{host}");
    for line in text.lines().filter(|s| !s.is_empty()) {
        count += 1;
        if count > 256 || line.len() > 1024 {
            return Err(Error::AdbProtocol);
        }
        let fields: Vec<_> = line.split_ascii_whitespace().collect();
        if fields.len() != 3 || fields[0] != "host" {
            return Err(Error::AdbProtocol);
        }
        if fields[1] == selected {
            if result != Mapping::Missing {
                return Err(Error::AdbProtocol);
            }
            result = if fields[2] == destination {
                Mapping::Same
            } else {
                Mapping::Other
            };
        }
    }
    Ok(result)
}

// All networking is to the already-running IPv4 loopback server, protocol41.
// No environment variable, hostname, owner input or remote address can redirect
// the server. No ADB CLI connection command (with auto-restart behavior) is used.
pub(super) struct SmartSocket<'a> {
    request: &'a ConnectionRequest,
    deadline: Instant,
}
impl<'a> SmartSocket<'a> {
    pub(super) fn new(request: &'a ConnectionRequest, limit: Duration) -> Result<Self> {
        let value = Self {
            request,
            deadline: Instant::now() + limit,
        };
        let version = value.host("host:version")?;
        if version != b"0029" {
            return Err(Error::UnsupportedAdb);
        }
        Ok(value)
    }
    fn remaining(&self) -> Result<Duration> {
        self.request.check()?;
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or(Error::Timeout)
    }
    fn connect(&self) -> Result<TcpStream> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, 5037));
        TcpStream::connect_timeout(&address, self.remaining()?.min(Duration::from_millis(80)))
            .map_err(|_| Error::AdbServerUnavailable)
    }
    fn io(&self, stream: &TcpStream) -> Result<()> {
        let timeout = self.remaining()?.min(Duration::from_millis(50));
        stream
            .set_read_timeout(Some(timeout))
            .and_then(|()| stream.set_write_timeout(Some(timeout)))
            .map_err(|_| Error::AdbProtocol)
    }
    fn read(&self, stream: &mut TcpStream, bytes: &mut [u8]) -> Result<()> {
        let mut at = 0;
        while at < bytes.len() {
            self.io(stream)?;
            match stream.read(&mut bytes[at..]) {
                Ok(0) => return Err(Error::AdbProtocol),
                Ok(n) => at += n,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => return Err(Error::AdbProtocol),
            }
        }
        Ok(())
    }
    fn write(&self, stream: &mut TcpStream, bytes: &[u8]) -> Result<()> {
        let mut at = 0;
        while at < bytes.len() {
            self.io(stream)?;
            match stream.write(&bytes[at..]) {
                Ok(0) => return Err(Error::AdbProtocol),
                Ok(n) => at += n,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => return Err(Error::AdbProtocol),
            }
        }
        Ok(())
    }
    fn request(&self, stream: &mut TcpStream, service: &str) -> Result<()> {
        self.write(stream, &encode_request(service)?)?;
        let mut status = [0; 4];
        self.read(stream, &mut status)?;
        match &status {
            b"OKAY" => Ok(()),
            b"FAIL" => {
                let _redacted = self.payload(stream)?;
                Err(Error::AdbRefused)
            }
            _ => Err(Error::AdbProtocol),
        }
    }
    fn payload(&self, stream: &mut TcpStream) -> Result<Vec<u8>> {
        let mut length = [0; 4];
        self.read(stream, &mut length)?;
        let length = decode_length(length)?;
        let mut bytes = vec![0; length];
        self.read(stream, &mut bytes)?;
        Ok(bytes)
    }
    fn host(&self, command: &str) -> Result<Vec<u8>> {
        let mut stream = self.connect()?;
        self.request(&mut stream, command)?;
        self.payload(&mut stream)
    }
    pub(super) fn devices(&self) -> Result<Vec<AdbDevice>> {
        parse_devices(&self.host("host:devices")?)
    }
    fn device(&self, serial: &str) -> Result<TcpStream> {
        validate_serial(serial)?;
        let mut stream = self.connect()?;
        self.request(&mut stream, &format!("host:transport:{serial}"))?;
        Ok(stream)
    }
    fn reverse_list(&self, serial: &str) -> Result<Vec<u8>> {
        let mut stream = self.device(serial)?;
        self.request(&mut stream, "reverse:list-forward")?;
        self.payload(&mut stream)
    }
    fn create(&self, config: &AdbReverseConfig) -> Result<()> {
        let mut stream = self.device(&config.serial)?;
        // adbd first acknowledges the transport service; forward itself then
        // returns OKAY/FAIL. A fixed nonzero port has no allocated-port payload.
        self.request(
            &mut stream,
            &format!(
                "reverse:forward:norebind:tcp:{};tcp:{}",
                config.phone_port, config.host_port
            ),
        )?;
        let mut status = [0; 4];
        self.read(&mut stream, &mut status)?;
        match &status {
            b"OKAY" => Ok(()),
            b"FAIL" => {
                let _redacted = self.payload(&mut stream)?;
                Err(Error::AdbRefused)
            }
            _ => Err(Error::AdbProtocol),
        }
    }
}
fn encode_request(service: &str) -> Result<Vec<u8>> {
    if service.is_empty()
        || service.len() > 1024
        || !service.is_ascii()
        || service.bytes().any(|b| b < 32 || b == 127)
    {
        return Err(Error::AdbProtocol);
    }
    Ok(format!("{:04x}{service}", service.len()).into_bytes())
}
fn decode_length(header: [u8; 4]) -> Result<usize> {
    let text = std::str::from_utf8(&header).map_err(|_| Error::AdbProtocol)?;
    if !header.iter().all(u8::is_ascii_hexdigit) {
        return Err(Error::AdbProtocol);
    }
    usize::from_str_radix(text, 16)
        .ok()
        .filter(|n| *n <= MAX_REPLY)
        .ok_or(Error::AdbProtocol)
}

trait ReverseBackend {
    fn devices(&mut self) -> Result<Vec<AdbDevice>>;
    fn list(&mut self, serial: &str) -> Result<Vec<u8>>;
    fn create(&mut self, config: &AdbReverseConfig) -> Result<()>;
}
impl ReverseBackend for SmartSocket<'_> {
    fn devices(&mut self) -> Result<Vec<AdbDevice>> {
        SmartSocket::devices(self)
    }
    fn list(&mut self, serial: &str) -> Result<Vec<u8>> {
        self.reverse_list(serial)
    }
    fn create(&mut self, config: &AdbReverseConfig) -> Result<()> {
        SmartSocket::create(self, config)
    }
}
fn cycle(
    backend: &mut impl ReverseBackend,
    config: &AdbReverseConfig,
) -> Result<(AdbReverseState, bool)> {
    let devices = backend.devices()?;
    let Some(device) = devices.iter().find(|d| d.serial == config.serial) else {
        return Ok((AdbReverseState::MissingDevice, false));
    };
    match device.state {
        AdbDeviceState::Offline => return Ok((AdbReverseState::Offline, false)),
        AdbDeviceState::Unauthorized => return Ok((AdbReverseState::Unauthorized, false)),
        AdbDeviceState::Unavailable => return Ok((AdbReverseState::MissingDevice, false)),
        AdbDeviceState::Available => {}
    }
    match mapping(
        &backend.list(&config.serial)?,
        config.phone_port,
        config.host_port,
    )? {
        Mapping::Same => Ok((AdbReverseState::ExistingMapping, false)),
        Mapping::Other => Ok((AdbReverseState::ContestedMapping, false)),
        Mapping::Missing => {
            // If another client wins the race, norebind fails without replacing
            // it. A subsequent read identifies matching versus contested state.
            match backend.create(config) {
                Ok(()) => Ok((AdbReverseState::MappingCreated, true)),
                Err(Error::AdbRefused) => match mapping(
                    &backend.list(&config.serial)?,
                    config.phone_port,
                    config.host_port,
                )? {
                    Mapping::Same => Ok((AdbReverseState::ExistingMapping, false)),
                    Mapping::Other => Ok((AdbReverseState::ContestedMapping, false)),
                    Mapping::Missing => Ok((AdbReverseState::Refused, false)),
                },
                Err(error) => Err(error),
            }
        }
    }
}
fn state_for(error: Error) -> AdbReverseState {
    match error {
        Error::AdbServerUnavailable => AdbReverseState::ServerUnavailable,
        Error::UnsupportedAdb => AdbReverseState::UnsupportedServer,
        Error::Timeout => AdbReverseState::TimedOut,
        Error::AdbRefused => AdbReverseState::Refused,
        Error::Cancelled => AdbReverseState::Stopped,
        _ => AdbReverseState::InvalidReply,
    }
}

static WATCHING: AtomicBool = AtomicBool::new(false);
struct WatchPermit;
impl Drop for WatchPermit {
    fn drop(&mut self) {
        WATCHING.store(false, Ordering::Release);
    }
}
#[derive(uniffi::Object)]
pub struct AdbReverseWatch {
    stop: Arc<ConnectionRequest>,
    snapshot: Arc<Mutex<AdbReverseSnapshot>>,
    done: watch::Receiver<bool>,
}
#[uniffi::export]
impl AdbReverseWatch {
    pub fn snapshot(&self) -> Result<AdbReverseSnapshot> {
        self.snapshot
            .lock()
            .map(|s| s.clone())
            .map_err(|_| Error::WorkerUnavailable)
    }
    pub async fn shutdown(&self) -> Result<()> {
        self.stop.cancel();
        let mut done = self.done.clone();
        while !*done.borrow_and_update() {
            done.changed().await.map_err(|_| Error::WorkerUnavailable)?;
        }
        Ok(())
    }
}
impl Drop for AdbReverseWatch {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
pub(super) fn start(
    config: AdbReverseConfig,
    tool: super::windows::CheckedTool,
) -> Result<Arc<AdbReverseWatch>> {
    WATCHING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| Error::Busy)?;
    let permit = WatchPermit;
    let stop = ConnectionRequest::new();
    let snapshot = Arc::new(Mutex::new(AdbReverseSnapshot {
        state: AdbReverseState::Checking,
        created_count: 0,
        cycle_millis: 0,
        created_mapping_may_remain: false,
        phone_port: config.phone_port,
        host_port: config.host_port,
    }));
    let (done, completion) = watch::channel(false);
    let result = Arc::new(AdbReverseWatch {
        stop: stop.clone(),
        snapshot: snapshot.clone(),
        done: completion,
    });
    std::thread::Builder::new()
        .name("vw-adb-reverse".into())
        .spawn(move || {
            let _tool_file_lease = tool;
            while stop.check().is_ok() {
                let started = Instant::now();
                let outcome = SmartSocket::new(&stop, CYCLE)
                    .and_then(|mut backend| cycle(&mut backend, &config));
                if let Ok(mut state) = snapshot.lock() {
                    let (next, created) = outcome.unwrap_or_else(|error| (state_for(error), false));
                    state.state = next;
                    state.cycle_millis =
                        u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
                    if created {
                        state.created_count = state.created_count.saturating_add(1);
                        state.created_mapping_may_remain = true;
                    }
                }
                // A cancellable short wait bounds stop even while the phone is absent.
                let until = Instant::now() + POLL;
                while stop.check().is_ok() && Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            if let Ok(mut state) = snapshot.lock() {
                state.state = AdbReverseState::Stopped;
            }
            drop(_tool_file_lease);
            drop(permit);
            let _ = done.send(true);
        })
        .map_err(|_| Error::WorkerUnavailable)?;
    Ok(result)
}

#[cfg(test)]
mod tests;
