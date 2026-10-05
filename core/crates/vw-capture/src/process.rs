//! One retained child per request. Cancellation never releases a live process.
use crate::*;
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
const RESPONSE_LIMIT: usize = 4 * 1024 * 1024;
static ACTIVE: AtomicBool = AtomicBool::new(false);
struct Permit;
impl Drop for Permit {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}
struct ChildLease(std::process::Child);
impl ChildLease {
    /// Retain the caller's slot even if the OS temporarily refuses a wait/kill.
    /// Capacity is released only after an affirmative exit observation.
    fn settle(&mut self) {
        loop {
            match self.0.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) | Err(_) => {
                    let _ = self.0.kill();
                    thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }
}
impl Drop for ChildLease {
    fn drop(&mut self) {
        self.settle();
    }
}
pub struct LockedHelper {
    path: PathBuf,
    _file: File,
}
impl LockedHelper {
    /// `expected` must come from the app's trusted packaged runtime inventory,
    /// never from the selected file or request. Keep this object alive through run.
    pub fn open(path: &Path, expected: &str) -> Result<Self> {
        plain(path, false)?;
        if expected.len() != 64 {
            return Err(Error::Invalid);
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        let mut file = options.open(path).map_err(|_| Error::Storage)?;
        if file.metadata().map_err(|_| Error::Storage)?.len() > 128 * 1024 * 1024 {
            return Err(Error::Limit);
        }
        let mut hash = ring::digest::Context::new(&ring::digest::SHA256);
        let mut count = 0u64;
        let mut chunk = [0u8; 65536];
        loop {
            let n = file.read(&mut chunk).map_err(|_| Error::Storage)?;
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > 128 * 1024 * 1024 {
                return Err(Error::Limit);
            }
            hash.update(&chunk[..n]);
        }
        let actual = hash
            .finish()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if actual != expected {
            return Err(Error::Invalid);
        }
        Ok(Self {
            path: path.to_owned(),
            _file: file,
        })
    }
}
pub fn run(helper: &LockedHelper, request: Request, cancel: Arc<Cancellation>) -> Result<Response> {
    cancel.check()?;
    let timeout = match &request {
        Request::Capture(value) => {
            value.limits.validate()?;
            value.limits.capture_ms + 2000
        }
        // Only the retained suspended harness parent owns canvas fixtures.
        Request::CanvasFixture(_) => return Err(Error::Invalid),
        Request::Tree(value) => {
            value.limits.validate()?;
            value.limits.tree_ms + 1000
        }
    };
    struct BoundedRequest(Vec<u8>);
    impl Write for BoundedRequest {
        fn write(&mut self, value: &[u8]) -> std::io::Result<usize> {
            if self.0.len().saturating_add(value.len()) > 16 * 1024 - 1 {
                return Err(std::io::Error::other("request limit"));
            }
            self.0.extend_from_slice(value);
            Ok(value.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut encoded = BoundedRequest(Vec::new());
    serde_json::to_writer(&mut encoded, &request).map_err(|_| Error::Limit)?;
    let mut bytes = encoded.0;
    bytes.push(b'\n');
    ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| Error::Busy)?;
    let _permit = Permit;
    let mut command = Command::new(&helper.path);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_clear();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut lease = ChildLease(command.spawn().map_err(|_| Error::Platform)?);
    let child = &mut lease.0;
    let result = (|| {
        let mut input = child.stdin.take().ok_or(Error::Platform)?;
        let output = child.stdout.take().ok_or(Error::Platform)?;
        let reader = thread::Builder::new()
            .name("vw-capture-receipt".into())
            .spawn(move || -> Result<Vec<u8>> {
                let mut bytes = Vec::new();
                output
                    .take(RESPONSE_LIMIT as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| Error::Platform)?;
                if bytes.len() > RESPONSE_LIMIT {
                    Err(Error::Limit)
                } else {
                    Ok(bytes)
                }
            })
            .map_err(|_| Error::Platform)?;
        let sent = input.write_all(&bytes).and_then(|()| input.flush());
        drop(input);
        let start = Instant::now();
        let mut failure = sent.err().map(|_| Error::Platform);
        let status = loop {
            if failure.is_none() {
                failure = cancel.check().err();
                if start.elapsed() > Duration::from_millis(timeout) {
                    failure = Some(Error::Timeout);
                }
            }
            if failure.is_some() {
                let _ = child.kill();
            }
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => thread::sleep(Duration::from_millis(5)),
                Err(_) => break Err(Error::Platform),
            }
        };
        // The exact process must be dead before joining its bounded stdout and
        // releasing the global slot. No image, node text or path enters logs.
        if status.is_err() {
            loop {
                let _ = child.kill();
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) | Err(_) => thread::sleep(Duration::from_millis(5)),
                }
            }
        }
        let captured = reader.join().map_err(|_| Error::Platform)?;
        if let Some(error) = failure {
            return Err(error);
        }
        if !status?.success() {
            return Err(Error::Platform);
        }
        let bytes = captured?;
        let response: Response = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
        match response {
            Response::Refused { error, .. } => Err(error),
            response => Ok(response),
        }
    })();
    lease.settle();
    result
}

/// The helper adds a hard native allocation/process bound before COM providers
/// run. ActiveProcessLimit=1 prevents descendants. The parent owns termination;
/// this self-owned Job deliberately omits KILL_ON_CLOSE (which would kill the
/// helper during successful Rust stack unwinding before its zero exit status).
#[cfg(windows)]
pub fn confine_helper() -> Result<impl Drop> {
    use ::windows::Win32::{
        Foundation::*,
        System::{JobObjects::*, Threading::*},
    };
    struct Job(HANDLE);
    impl Drop for Job {
        fn drop(&mut self) {
            /* SAFETY: unique Job handle. */
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    // SAFETY: initialized policy and valid current-process pseudo handle. The
    // helper owns no other application processes and the Job is unnamed.
    unsafe {
        let job = Job(CreateJobObjectW(None, None).map_err(|_| Error::Platform)?);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        limits.BasicLimitInformation.ActiveProcessLimit = 1;
        limits.ProcessMemoryLimit = 384 * 1024 * 1024;
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
        .map_err(|_| Error::Platform)?;
        AssignProcessToJobObject(job.0, GetCurrentProcess()).map_err(|_| Error::Platform)?;
        Ok(job)
    }
}
