use super::{
    Error, Result,
    capture::{self, CapturePair, Hierarchy},
    device::{self, DeviceToken, SelectedDevice, UsbSelection},
    mirror::{self, CaptureSource, WindowProbe},
};
use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock, atomic::{AtomicBool, Ordering}},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(12);
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_COMMAND_REPLY: usize = 64 * 1024;
static ACTIVE: AtomicBool = AtomicBool::new(false);
static RETRYING: AtomicBool = AtomicBool::new(false);
#[cfg(all(test, not(windows)))]
static SYNTHETIC_STARTUPS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(all(test, not(windows)))]
pub(super) fn synthetic_startups_for_test(count: usize) {
    SYNTHETIC_STARTUPS.store(count, Ordering::Release);
}

#[derive(Default)]
pub struct Cancellation(AtomicBool);

impl Cancellation {
    pub fn cancel(&self) { self.0.store(true, Ordering::Release); }
    pub fn check(&self) -> Result<()> {
        if self.0.load(Ordering::Acquire) { Err(Error::Cancelled) } else { Ok(()) }
    }
}

/// This has no public constructor. Only root's verified, closed runtime/license
/// admission may create it; SHA strings and source recipes cannot self-admit.
pub struct AdmittedRuntime {
    pub(crate) client: PathBuf,
    pub(crate) server: PathBuf,
    pub(crate) client_sha256: [u8; 32],
    pub(crate) server_sha256: [u8; 32],
    pub(crate) dlls: Vec<(PathBuf, [u8; 32])>,
}

impl AdmittedRuntime {
    /// Root must call only after exact-byte DLL/native aggregate, replaceability
    /// and notices admission. No supplied bool or report file is accepted here.
    pub(crate) fn from_root_admission(
        client: PathBuf, server: PathBuf, client_sha256: [u8; 32],
        server_sha256: [u8; 32], dlls: Vec<(PathBuf, [u8; 32])>,
    ) -> Result<Self> {
        if !client.is_absolute() || !server.is_absolute() || dlls.len() != 6
            || server.parent() != client.parent()
            || !client.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case("scrcpy.exe"))
            || !server.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case("scrcpy-server"))
            || dlls.iter().any(|(p, _)| !p.is_absolute() || p.parent() != client.parent()
                || p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("adb.exe")))
        {
            return Err(Error::RuntimeUnavailable);
        }
        Ok(Self { client, server, client_sha256, server_sha256, dlls })
    }
}

/// Implemented by root's exact-byte policy gate, never by peer/project input.
pub trait RuntimeAdmission: Send + Sync {
    fn acquire(&self) -> Result<AdmittedRuntime>;
}

/// Explicit ordinary owner-selected platform-tools binary. Its exact opened
/// file and ancestors are pinned by the Windows runner before first use.
pub struct OwnerAdb {
    pub(crate) path: PathBuf,
}

impl OwnerAdb {
    pub fn select(path: PathBuf) -> Result<Self> {
        if !path.is_absolute() || !path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case("adb.exe"))
            || !path.parent().and_then(|p| p.file_name()).is_some_and(|n|
                n.to_string_lossy().eq_ignore_ascii_case("platform-tools")) {
            return Err(Error::Invalid);
        }
        Ok(Self { path })
    }
}

pub(crate) struct CommandOutput {
    pub code: u32,
    pub bytes: Vec<u8>,
    pub started_ms: u64,
    pub finished_ms: u64,
}

pub(crate) trait Runner: Send {
    fn run(&mut self, args: &[String], cap: usize, timeout: Duration, cancel: &Cancellation)
        -> Result<CommandOutput>;
    fn launch_mirror(&mut self, serial: &str, cancel: &Cancellation) -> Result<u32>;
    /// Join an uncertain in-flight command before cleanup issues another one.
    fn settle_io(&mut self) -> Result<()>;
    fn close(&mut self) -> Result<()>;
}

struct Permit;
impl Drop for Permit { fn drop(&mut self) { ACTIVE.store(false, Ordering::Release); } }

#[derive(Clone)]
enum Scratch {
    Unknown { directory: String },
    Owned { directory: String, file: String, phase: ScratchPhase },
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ScratchPhase { FilePresent, FileRemoved, DirectoryRemoved, UncertainFile, UncertainDirectory }

struct Retained {
    runner: Box<dyn Runner>,
    scratch: Option<Scratch>,
    selected: Option<SelectedDevice>,
    _permit: Permit,
}

fn retained() -> &'static Mutex<Vec<Retained>> {
    static OWNERS: OnceLock<Mutex<Vec<Retained>>> = OnceLock::new();
    OWNERS.get_or_init(|| Mutex::new(Vec::new()))
}

fn args(id: u32, command: &[&str]) -> Vec<String> {
    let mut out = vec!["-t".to_owned(), id.to_string()];
    out.extend(command.iter().map(|v| (*v).to_owned()));
    out
}

fn cleanup(runner: &mut dyn Runner, selected: &SelectedDevice, scratch: &mut Scratch) -> Result<()> {
    runner.settle_io().map_err(|_| Error::RetirementPending)?;
    let fresh = Cancellation::default();
    let current = runner.run(&["devices".into(), "-l".into()], MAX_COMMAND_REPLY,
        COMMAND_TIMEOUT, &fresh)?;
    if current.code != 0 { return Err(Error::RetirementPending); }
    device::still_selected(selected, &current.bytes).map_err(|_| Error::RetirementPending)?;
    let id = selected.transport_id;
    if let Scratch::Owned { directory, file, phase } = scratch {
        if *phase == ScratchPhase::FilePresent {
            let result = runner.run(&args(id, &["shell", "rm", "-f", file]), MAX_COMMAND_REPLY,
                COMMAND_TIMEOUT, &fresh);
            match result {
                Ok(reply) if reply.code == 0 => *phase = ScratchPhase::FileRemoved,
                _ => { *phase = ScratchPhase::UncertainFile; return Err(Error::RetirementPending); }
            }
        }
        if *phase == ScratchPhase::FileRemoved {
            let exists = runner.run(&args(id, &["shell", "test", "-e", directory]),
                MAX_COMMAND_REPLY, COMMAND_TIMEOUT, &fresh)?;
            if exists.code == 1 && exists.bytes.is_empty() {
                *phase = ScratchPhase::DirectoryRemoved;
            } else if exists.code == 0 && exists.bytes.is_empty() {
                let removed = runner.run(&args(id, &["shell", "rmdir", directory]),
                    MAX_COMMAND_REPLY, COMMAND_TIMEOUT, &fresh);
                match removed {
                    Ok(reply) if reply.code == 0 => *phase = ScratchPhase::DirectoryRemoved,
                    _ => { *phase = ScratchPhase::UncertainDirectory;
                        return Err(Error::RetirementPending); }
                }
            } else { return Err(Error::RetirementPending); }
        }
    }
    let directory = match scratch {
        Scratch::Unknown { directory } | Scratch::Owned { directory, .. } => directory,
    };
    let exists = runner.run(&args(id, &["shell", "test", "-e", directory]), MAX_COMMAND_REPLY,
        COMMAND_TIMEOUT, &fresh)?;
    if exists.code != 1 || !exists.bytes.is_empty() { return Err(Error::RetirementPending); }
    Ok(())
}

/// Retry unknown cleanup/tree retirement before admitting another session.
pub fn retry_retirements() -> Result<usize> {
    if RETRYING.swap(true, Ordering::AcqRel) { return Ok(1); }
    struct RetryGuard;
    impl Drop for RetryGuard {
        fn drop(&mut self) { RETRYING.store(false, Ordering::Release); }
    }
    let _retry_guard = RetryGuard;
    // Never hold a global registry mutex across device IO or process joins.
    let mut list = {
        let mut guard = retained().lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *guard)
    };
    let mut index = 0;
    while index < list.len() {
        let Retained { runner, scratch, selected, .. } = &mut list[index];
        let cleaned = match (scratch.as_mut(), selected.as_ref()) {
            (Some(scratch), Some(selected)) => cleanup(&mut **runner, selected, scratch).is_ok(),
            (None, _) => runner.settle_io().is_ok(),
            _ => false,
        };
        if cleaned {
            *scratch = None;
            if runner.close().is_ok() {
                list.remove(index);
                continue;
            }
        }
        index += 1;
    }
    let mut guard = retained().lock().unwrap_or_else(|e| e.into_inner());
    guard.extend(list);
    let pending = guard.len();
    drop(guard);
    #[cfg(windows)]
    let unresolved = super::windows::retry_unresolved();
    #[cfg(all(test, not(windows)))]
    let unresolved = SYNTHETIC_STARTUPS.load(Ordering::Acquire);
    #[cfg(all(not(test), not(windows)))]
    let unresolved = 0;
    Ok(pending + unresolved)
}

pub struct AndroidTunnelOwner {
    runner: Option<Box<dyn Runner>>,
    probe: Box<dyn WindowProbe>,
    permit: Option<Permit>,
    selected: SelectedDevice,
    token: DeviceToken,
    session: [u8; 16],
    generation: u64,
    mirror_pid: u32,
    scratch: Option<Scratch>,
    cancel: Cancellation,
    sealed: bool,
}

fn now_ms() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| Error::Platform)?.as_millis()
        .try_into().map_err(|_| Error::Limit)?)
}

pub(super) fn checked_completed_age(image_finished: u64, hierarchy_started: u64,
    hierarchy_completed: u64, observed: u64) -> Result<u64> {
    if hierarchy_started < image_finished || hierarchy_completed < hierarchy_started
        || observed < hierarchy_completed { return Err(Error::Stale); }
    Ok(observed - image_finished)
}

impl AndroidTunnelOwner {
    /// Windows host entry: root's admission is mandatory and cannot be replaced
    /// by a JSON receipt, name, hash string, or client-supplied approval flag.
    pub fn open(
        admission: Option<&dyn RuntimeAdmission>, adb: OwnerAdb, selection: UsbSelection,
        session: [u8; 16], generation: u64, cancel: &Cancellation,
    ) -> Result<Self> {
        let gate = admission.ok_or(Error::RuntimeUnavailable)?;
        let runtime = gate.acquire()?;
        #[cfg(windows)]
        {
            let (runner, probe) = super::windows::open(runtime, adb)?;
            Self::open_with(runner, probe, selection, session, generation, cancel)
        }
        #[cfg(not(windows))]
        {
            let _ = (runtime, adb, selection, session, generation, cancel);
            Err(Error::Platform)
        }
    }

    pub(super) fn open_with(
        mut runner: Box<dyn Runner>, probe: Box<dyn WindowProbe>, selection: UsbSelection,
        session: [u8; 16], generation: u64, cancel: &Cancellation,
    ) -> Result<Self> {
        cancel.check()?;
        if session == [0; 16] || generation == 0 { return Err(Error::Invalid); }
        if retry_retirements()? != 0 { return Err(Error::RetirementPending); }
        ACTIVE.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::RetirementPending)?;
        let permit = Permit;
        let result = (|| {
            let listing = runner.run(&["devices".into(), "-l".into()], MAX_COMMAND_REPLY,
                COMMAND_TIMEOUT, cancel)?;
            if listing.code != 0 { return Err(Error::Platform); }
            let selected = device::select(&listing.bytes, &selection)?;
            let mirror_pid = runner.launch_mirror(&selected.serial, cancel)?;
            if mirror_pid == 0 { return Err(Error::Platform); }
            let mut context = blake3::Hasher::new();
            context.update(&session);
            context.update(&generation.to_be_bytes());
            context.update(selected.serial.as_bytes());
            context.update(&selected.transport_id.to_be_bytes());
            let hash = context.finalize();
            let mut token = [0; 16];
            token.copy_from_slice(&hash.as_bytes()[..16]);
            Ok((selected, mirror_pid, DeviceToken(token)))
        })();
        match result {
            Ok((selected, mirror_pid, token)) => Ok(Self {
                runner: Some(runner), probe, permit: Some(permit), selected, token,
                session, generation, mirror_pid, scratch: None,
                cancel: Cancellation::default(), sealed: false,
            }),
            Err(error) => {
                if runner.close().is_err() {
                    retained().lock().unwrap_or_else(|e| e.into_inner()).push(Retained {
                        runner, scratch: None, selected: None, _permit: permit,
                    });
                    Err(Error::RetirementPending)
                } else { Err(error) }
            }
        }
    }

    fn runner(&mut self) -> Result<&mut (dyn Runner + '_)> {
        match self.runner.as_deref_mut() { Some(r) => Ok(r), None => Err(Error::RetirementPending) }
    }

    fn settle_scratch(&mut self) -> Result<()> {
        let Some(mut scratch) = self.scratch.take() else {
            return self.runner()?.settle_io();
        };
        let selected = self.selected.clone();
        let result = match self.runner() {
            Ok(runner) => cleanup(runner, &selected, &mut scratch),
            Err(error) => Err(error),
        };
        if result.is_err() { self.scratch = Some(scratch); }
        result
    }

    fn selected_current(&mut self, cancel: &Cancellation) -> Result<()> {
        cancel.check()?;
        let reply = self.runner()?.run(&["devices".into(), "-l".into()],
            MAX_COMMAND_REPLY, COMMAND_TIMEOUT, cancel)?;
        if reply.code != 0 { return Err(Error::Platform); }
        device::still_selected(&self.selected, &reply.bytes)
    }

    pub fn check_source(&mut self, source: CaptureSource) -> Result<()> {
        if self.sealed { return Err(Error::RetirementPending); }
        mirror::check(&self.probe.current(source, self.mirror_pid)?)
    }

    pub fn capture_pair(&mut self, cancel: &Cancellation) -> Result<CapturePair> {
        if self.sealed || self.scratch.is_some() { return Err(Error::RetirementPending); }
        self.cancel.check()?;
        cancel.check()?;
        self.selected_current(cancel)?;
        let id = self.selected.transport_id;
        let image = self.runner()?.run(&args(id, &["exec-out", "screencap", "-p"]),
            capture::MAX_PNG + 1, CAPTURE_TIMEOUT, cancel)?;
        if image.code != 0 { return Err(Error::Platform); }
        if image.finished_ms < image.started_ms { return Err(Error::Stale); }
        let png = capture::png(image.bytes)?;
        self.selected_current(cancel)?;
        // The session ID is root's fresh opaque UUID. mkdir atomically claims
        // a new directory; dump can only write the file inside that owner.
        let nonce = self.session.iter().map(|v| format!("{v:02x}")).collect::<String>();
        let directory = format!("/sdcard/vw-{nonce}-{}", self.generation);
        let file = format!("{directory}/hierarchy.xml");
        let mut hierarchy_start = now_ms()?;
        if hierarchy_start < image.finished_ms { return Err(Error::Stale); }
        self.scratch = Some(Scratch::Unknown { directory: directory.clone() });
        let claimed = self.runner()?.run(&args(id, &["shell", "mkdir", &directory]),
            MAX_COMMAND_REPLY, COMMAND_TIMEOUT, cancel);
        let claimed = match claimed {
            Ok(reply) if reply.code == 0 => reply,
            Ok(_) => {
                self.sealed = true;
                if self.settle_scratch().is_ok() {
                    return Err(Error::Storage);
                }
                return Err(Error::RetirementPending);
            },
            Err(_) => { self.sealed = true; return Err(Error::RetirementPending); },
        };
        self.scratch = Some(Scratch::Owned { directory: directory.clone(), file: file.clone(),
            phase: ScratchPhase::FilePresent });
        if claimed.started_ms < image.finished_ms || claimed.finished_ms < claimed.started_ms {
            self.sealed = true;
            return if self.settle_scratch().is_ok() { Err(Error::Stale) }
                else { Err(Error::RetirementPending) };
        }
        let mut completed_ms = hierarchy_start;
        let mut acquisition: Result<Option<Vec<capture::SemanticElement>>> = (|| {
            let dumped = self.runner()?.run(&args(id, &["shell", "uiautomator", "dump", &file]),
                MAX_COMMAND_REPLY, CAPTURE_TIMEOUT, cancel)?;
            hierarchy_start = dumped.started_ms;
            completed_ms = dumped.finished_ms;
            if hierarchy_start < claimed.finished_ms || completed_ms < hierarchy_start {
                return Err(Error::Stale);
            }
            if dumped.code != 0 { return Ok(None); }
            self.selected_current(cancel)?;
            let xml = self.runner()?.run(&args(id, &["exec-out", "cat", &file]),
                capture::MAX_XML + 1, CAPTURE_TIMEOUT, cancel)?;
            if xml.started_ms < completed_ms || xml.finished_ms < xml.started_ms {
                return Err(Error::Stale);
            }
            completed_ms = xml.finished_ms;
            if xml.code != 0 { return Ok(None); }
            let elements = capture::hierarchy(&xml.bytes, png.width, png.height)?;
            Ok(Some(elements))
        })();
        let hierarchy_finish = match now_ms() {
            Ok(value) => value,
            Err(error) => { acquisition = Err(error); hierarchy_start },
        };
        let age = match checked_completed_age(image.finished_ms, hierarchy_start,
            completed_ms, hierarchy_finish) {
            Ok(value) => value,
            Err(error) => { acquisition = Err(error); 0 },
        };
        // Cleanup is mandatory on every success/refusal/cancellation branch.
        if self.settle_scratch().is_err() {
            self.sealed = true;
            return Err(Error::RetirementPending);
        }
        self.selected_current(cancel)?;
        cancel.check()?;
        let hierarchy = match acquisition? {
            None => Hierarchy::Missing,
            Some(elements) => {
                if age > 2_000 { Hierarchy::Stale { age_ms: age } }
                else { Hierarchy::Present { elements, age_ms: age } }
            }
        };
        Ok(CapturePair {
            session: self.session, device: self.token, source_generation: self.generation,
            image: png, hierarchy, image_started_ms: image.started_ms,
            image_finished_ms: image.finished_ms, hierarchy_started_ms: Some(hierarchy_start),
            hierarchy_finished_ms: Some(hierarchy_finish),
        })
    }

    pub fn close(&mut self) -> Result<()> {
        self.cancel.cancel();
        if self.runner.is_none() {
            return if retry_retirements()? == 0 { Ok(()) } else { Err(Error::RetirementPending) };
        }
        self.settle_scratch().map_err(|_| Error::RetirementPending)?;
        self.runner()?.close().map_err(|_| Error::RetirementPending)?;
        self.runner = None;
        self.permit = None;
        self.sealed = true;
        if retry_retirements()? == 0 { Ok(()) } else { Err(Error::RetirementPending) }
    }
}

impl Drop for AndroidTunnelOwner {
    fn drop(&mut self) {
        if self.close().is_err() {
            if let (Some(runner), Some(permit)) = (self.runner.take(), self.permit.take()) {
                retained().lock().unwrap_or_else(|e| e.into_inner()).push(Retained {
                    runner, scratch: self.scratch.take(),
                    selected: Some(self.selected.clone()), _permit: permit,
                });
            }
        }
    }
}
