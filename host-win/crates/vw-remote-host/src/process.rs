//! Exact ancestor/file namespace, suspended parent Job launch and typed retained
//! retirement. Slots/leases release only after whole-tree exit and actual IO join.
mod finite;
mod input_retirement;
#[cfg(windows)]
pub(crate) mod windows;
use crate::{Error, Result};
#[cfg(not(windows))]
use std::fs::File;
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use vw_remote::wire::{self, Command, Header, Packet};
static VIDEO: AtomicBool = AtomicBool::new(false);
static INPUT: AtomicBool = AtomicBool::new(false);
#[derive(Clone, Copy)]
pub enum Kind {
    Video,
    Input,
}
impl Kind {
    fn slot(self) -> &'static AtomicBool {
        match self {
            Self::Video => &VIDEO,
            Self::Input => &INPUT,
        }
    }
    fn filename(self) -> &'static str {
        match self {
            Self::Video => "vw-hevc-helper.exe",
            Self::Input => "vw-input-helper.exe",
        }
    }
}
struct Permit(Kind);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.slot().store(false, Ordering::Release);
    }
}
#[derive(Default)]
pub struct Cancellation(AtomicBool);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release)
    }
    pub fn check(&self) -> Result<()> {
        if self.0.load(Ordering::Acquire) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}
pub struct LockedHelper {
    path: PathBuf,
    kind: Kind,
    #[cfg(windows)]
    pins: windows::Pins,
}
impl LockedHelper {
    /// Hash from trusted packaged inventory, never from peer/project content.
    pub fn open(path: &Path, expected: &str, kind: Kind) -> Result<Arc<Self>> {
        if expected.len() != 64
            || !expected
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || !path.is_absolute()
            || !path
                .file_name()
                .and_then(|v| v.to_str())
                .is_some_and(|v| v.eq_ignore_ascii_case(kind.filename()))
        {
            return Err(Error::Invalid);
        }
        #[cfg(not(windows))]
        {
            let _ = (path, expected, kind);
            Err(Error::Unavailable)
        }
        #[cfg(windows)]
        {
            let pins = windows::Pins::open(path)?;
            let mut file = pins.executable.file().try_clone()?;
            if file.metadata()?.len() > 128 * 1024 * 1024 {
                return Err(Error::Limit);
            }
            let mut hash = ring::digest::Context::new(&ring::digest::SHA256);
            let mut buf = [0; 65536];
            let mut count = 0;
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                count += n;
                if count > 128 * 1024 * 1024 {
                    return Err(Error::Limit);
                }
                hash.update(&buf[..n]);
            }
            let digest = hash.finish();
            let actual = digest
                .as_ref()
                .iter()
                .map(|v| format!("{v:02x}"))
                .collect::<String>();
            if actual != expected {
                return Err(Error::Invalid);
            }
            pins.verify()?;
            Ok(Arc::new(Self {
                path: path.to_owned(),
                kind,
                pins,
            }))
        }
    }
}
#[cfg(windows)]
type Child = windows::Child;
#[cfg(not(windows))]
struct Child {
    stdin: Option<File>,
    stdout: Option<File>,
    startup_error: Option<Error>,
}
#[cfg(not(windows))]
impl Child {
    fn kill_tree(&self) -> Result<()> {
        Err(Error::Unavailable)
    }
    fn exited(&self) -> Result<bool> {
        Err(Error::Unavailable)
    }
}
struct Resources {
    child: Child,
    send: Option<mpsc::SyncSender<Command>>,
    receive: mpsc::Receiver<Result<Packet>>,
    io: Option<thread::JoinHandle<()>>,
    _helper: Arc<LockedHelper>,
    _permit: Permit,
    finite: finite::Fence,
    last_request: u64,
    input: Option<input_retirement::Fence>,
}
impl Resources {
    fn observe(&mut self, packet: &Packet) -> Result<()> {
        self.finite.observe(&packet.header)?;
        if let Some(input) = &mut self.input {
            input.observe(&packet.header);
        }
        Ok(())
    }
    fn sent_stop(&mut self, sequence: u64) -> Result<()> {
        if self.finite.pending() && self.finite.stop_sequence().is_none() {
            self.finite.sent_stop(sequence)?;
        }
        if let Some(input) = &mut self.input {
            input.sent_stop(sequence)?;
        }
        Ok(())
    }
    fn settle(&mut self, deadline: Instant) -> Result<()> {
        // OpenInput may have created a scoped synthetic device before its reply.
        // Request release-only Stop and retain every owner until its exact reply.
        // Finite SendInput uncertainty remains stricter than scoped pen teardown.
        while self.finite.pending() || self.input.as_ref().is_some_and(|v| v.pending()) {
            let stop = self
                .input
                .as_ref()
                .and_then(|v| v.stop_sequence())
                .or(self.finite.stop_sequence());
            if stop.is_none() {
                let sequence = self.last_request.checked_add(1).ok_or(Error::Limit)?;
                if let Some(send) = &self.send {
                    match send.try_send(Command::Stop { sequence }) {
                        Ok(()) => {
                            self.sent_stop(sequence)?;
                            self.last_request = sequence;
                        }
                        Err(mpsc::TrySendError::Full(_) | mpsc::TrySendError::Disconnected(_)) => {}
                    }
                }
            }
            match self.receive.recv_timeout(Duration::from_millis(5)) {
                Ok(Ok(packet)) => self.observe(&packet)?,
                Ok(Err(_))
                | Err(mpsc::RecvTimeoutError::Disconnected | mpsc::RecvTimeoutError::Timeout) => {}
            }
            // Actual whole-Job exit plus IO join retires an already-gone scoped
            // pen owner. It never proves global SendInput release or a causal
            // fixture witness. Missing replies alone grant no such permission.
            let finite_pending = self.finite.pending();
            if input_retirement::already_gone(self, finite_pending)? {
                return Ok(());
            }
            if (self.finite.pending() || self.input.as_ref().is_some_and(|v| v.pending()))
                && Instant::now() >= deadline
            {
                return Err(Error::RetirementPending);
            }
        }
        self.send.take();
        let _ = self.child.kill_tree();
        loop {
            // Release was already proven above (or this was input-free). Drain
            // the bounded producer so actual IO completion remains observable.
            while self.receive.try_recv().is_ok() {}
            if crate::retirement::settle_once(self)? {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(Error::RetirementPending);
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}
impl crate::retirement::Owner for Resources {
    fn tree_exited(&self) -> Result<bool> {
        self.child.exited()
    }
    fn io_finished(&self) -> bool {
        self.io.as_ref().is_none_or(thread::JoinHandle::is_finished)
    }
    fn join_io(&mut self) -> Result<()> {
        if let Some(io) = self.io.take() {
            io.join().map_err(|_| Error::Io)?;
        }
        Ok(())
    }
}
fn retained() -> &'static Mutex<Vec<Resources>> {
    static OWNERS: OnceLock<Mutex<Vec<Resources>>> = OnceLock::new();
    OWNERS.get_or_init(|| Mutex::new(Vec::new()))
}
/// Dropped failed-close owners remain retained. Native link/library teardown
/// must refuse while this count is nonzero. At most the two held slots exist.
pub fn retry_retirements() -> Result<usize> {
    let mut owners = retained().lock().unwrap_or_else(|e| e.into_inner());
    let mut index = 0;
    while index < owners.len() {
        if owners[index].settle(Instant::now()).is_ok() {
            owners.remove(index);
        } else {
            index += 1
        }
    }
    Ok(owners.len())
}
pub struct Process {
    owned: Option<Box<Resources>>,
    last: u64,
    sealed: bool,
    pen_release: Option<wire::PenReleaseWitness>,
}
impl Process {
    pub fn open(
        helper: Arc<LockedHelper>,
        request: Command,
        cancel: &Cancellation,
    ) -> Result<(Self, Packet)> {
        cancel.check()?;
        retry_retirements()?;
        if request.sequence() != 1
            || !matches!(
                (helper.kind, &request),
                (Kind::Video, Command::OpenVideo { .. }) | (Kind::Input, Command::OpenInput { .. })
            )
        {
            return Err(Error::Invalid);
        }
        helper
            .kind
            .slot()
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::Limit)?;
        let permit = Permit(helper.kind);
        #[cfg(not(windows))]
        {
            let _ = (helper, request, cancel, permit);
            Err(Error::Unavailable)
        }
        #[cfg(windows)]
        {
            let child = Child::launch(&helper.path, &helper.pins, cancel)?;
            let (send, requests) = mpsc::sync_channel::<Command>(1);
            let (results, receive) = mpsc::sync_channel(1);
            let resources = Resources {
                child,
                send: Some(send),
                receive,
                io: None,
                _helper: helper,
                _permit: permit,
                finite: finite::Fence::with_binding(match &request {
                    Command::OpenInput { binding, .. } => Some(binding.clone()),
                    _ => None,
                }),
                last_request: 0,
                input: None,
            };
            let mut owner = Self {
                owned: Some(Box::new(resources)),
                last: 0,
                sealed: false,
                pen_release: None,
            };
            if let Some(error) = owner.resource()?.child.startup_error.clone() {
                let retired = owner.close();
                return Err(retired.err().unwrap_or(error));
            }
            let io = (|| {
                let r = owner.resource()?;
                let mut input = r.child.stdin.take().ok_or(Error::Io)?;
                let mut output = r.child.stdout.take().ok_or(Error::Io)?;
                thread::Builder::new()
                    .name("vw-remote-pipe".into())
                    .spawn(move || {
                        while let Ok(request) = requests.recv() {
                            let result = wire::write_command(&mut input, &request)
                                .and_then(|()| wire::read_packet(&mut output));
                            let failed = result.is_err();
                            if results.send(result).is_err() || failed {
                                break;
                            }
                        }
                    })
                    .map_err(|_| Error::Io)
            })();
            match io {
                Ok(io) => owner.resource()?.io = Some(io),
                Err(error) => {
                    let retired = owner.close();
                    return Err(retired.err().unwrap_or(error));
                }
            }
            let ready = owner.exchange(request, cancel)?;
            if !matches!(ready.header, Header::Ready { .. }) {
                let retired = owner.close();
                return Err(retired.err().unwrap_or(Error::Unavailable));
            }
            Ok((owner, ready))
        }
    }
    fn resource(&mut self) -> Result<&mut Resources> {
        self.owned.as_deref_mut().ok_or(Error::Unavailable)
    }
    pub fn next_sequence(&self) -> Result<u64> {
        self.last.checked_add(1).ok_or(Error::Limit)
    }
    fn failed(&mut self, error: Error) -> Error {
        self.sealed = true;
        // Preserve the primary command/cancellation error. Actual pending owner
        // remains in self/retained registry and still blocks native unload.
        let _retirement = self.close();
        error
    }
    pub fn exchange(&mut self, request: Command, cancel: &Cancellation) -> Result<Packet> {
        if self.sealed || self.owned.is_none() {
            return Err(Error::Unavailable);
        }
        if let Err(e) = cancel.check() {
            return Err(self.failed(e));
        }
        let sequence = request.sequence();
        if sequence != self.next_sequence()? {
            return Err(Error::Invalid);
        }
        let timeout = if matches!(
            &request,
            Command::Pen { .. }
                | Command::Finite { .. }
                | Command::Check { .. }
                | Command::RefreshAuthority { .. }
        ) {
            250
        } else {
            6000
        };
        let finite_command = matches!(request, Command::Finite { .. });
        if let Command::Finite { input_seq, .. } = &request {
            self.resource()?.finite.begin(sequence, *input_seq)?
        }
        if self
            .resource()?
            .send
            .as_ref()
            .ok_or(Error::Io)?
            .try_send(request.clone())
            .is_err()
        {
            if finite_command {
                self.resource()?.finite.not_queued(sequence)
            }
            return Err(self.failed(Error::Limit));
        }
        // Only successful enqueue can make OpenInput possibly input-capable.
        // Pre-enqueue startup failures retain the original input-free cleanup.
        if let Command::OpenInput { binding, .. } = &request {
            self.resource()?.input = Some(input_retirement::Fence::new(binding.clone()));
        }
        if matches!(request, Command::Stop { .. }) {
            self.resource()?.sent_stop(sequence)?;
        }
        self.resource()?.last_request = sequence;
        self.last = sequence;
        let start = Instant::now();
        loop {
            if let Err(e) = cancel.check() {
                return Err(self.failed(e));
            }
            if start.elapsed() >= Duration::from_millis(timeout) {
                return Err(self.failed(Error::Timeout));
            }
            match self
                .resource()?
                .receive
                .recv_timeout(Duration::from_millis(5))
            {
                Ok(Ok(packet)) => {
                    if packet.header.sequence() != sequence {
                        return Err(self.failed(Error::Invalid));
                    }
                    let allowed = matches!(
                        (&request, &packet.header),
                        (
                            Command::OpenVideo { .. } | Command::OpenInput { .. },
                            Header::Ready { .. }
                        ) | (
                            Command::Poll { .. },
                            Header::Video { .. } | Header::Idle { .. }
                        ) | (
                            Command::Pen { .. } | Command::Finite { .. },
                            Header::Injected { .. }
                        ) | (Command::RefreshAuthority { .. }, Header::Authority { .. })
                            | (Command::Check { .. }, Header::Idle { .. })
                            | (Command::Stop { .. }, Header::Stopped { .. })
                    ) || matches!(packet.header, Header::Refused { .. });
                    if !allowed {
                        return Err(self.failed(Error::Invalid));
                    }
                    if let Err(error) = self.resource()?.observe(&packet) {
                        return Err(self.failed(error));
                    }
                    if let Header::Refused { error, .. } = &packet.header {
                        return Err(self.failed(error.clone()));
                    }
                    if matches!(request, Command::Stop { .. }) {
                        self.close()?;
                    }
                    return Ok(packet);
                }
                Ok(Err(e)) => return Err(self.failed(e)),
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(self.failed(Error::Io)),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
    /// Present only after actual OS/IO settlement, never merely after Stopped.
    pub fn pen_release_witness(&self) -> Option<wire::PenReleaseWitness> {
        if self.owned.is_none() {
            self.pen_release.clone()
        } else {
            None
        }
    }
    pub fn close(&mut self) -> Result<()> {
        self.sealed = true;
        if let Some(resources) = &mut self.owned {
            resources.settle(Instant::now() + Duration::from_millis(500))?;
            self.pen_release = resources.input.as_ref().and_then(|v| v.witness()).cloned();
        }
        self.owned.take();
        Ok(())
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if self.close().is_err()
            && let Some(owner) = self.owned.take()
        {
            // Move the entire owned value. It has no escaping self-reference;
            // every child/Job/pipe/thread/helper/lease/permit remains retained.
            retained()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(*owner);
        }
    }
}
/// Only the local trusted package inventory supplies expected. Remote/project
/// data never selects a profile path or manufactures that inventory binding.
pub fn load_packaged_profile(
    path: &Path,
    expected: &str,
) -> Result<vw_remote::profile::PackagedProfile> {
    if !vw_remote::profile::digest(expected) || !path.is_absolute() {
        return Err(Error::Invalid);
    }
    #[cfg(not(windows))]
    {
        let _ = (path, expected);
        Err(Error::Unavailable)
    }
    #[cfg(windows)]
    {
        let pins = windows::Pins::open(path)?;
        let file = pins.executable.file().try_clone()?;
        if file.metadata()?.len() > 32 * 1024 {
            return Err(Error::Limit);
        }
        let mut bytes = Vec::new();
        file.take(32 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 32 * 1024 {
            return Err(Error::Limit);
        }
        let actual = ring::digest::digest(&ring::digest::SHA256, &bytes)
            .as_ref()
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect::<String>();
        if actual != expected {
            return Err(Error::Invalid);
        }
        pins.verify()?;
        let json = String::from_utf8(bytes).map_err(|_| Error::Invalid)?;
        Ok(vw_remote::profile::PackagedProfile {
            digest: expected.into(),
            json,
        })
    }
}

/// Slots remain held through actual whole-Job exit and IO join. Used by the
/// foreign runtime unload fence; an empty retirement list alone is insufficient.
pub fn active_owner_count() -> usize {
    usize::from(VIDEO.load(Ordering::Acquire)) + usize::from(INPUT.load(Ordering::Acquire))
}
