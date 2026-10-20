//! Exact pinned Windows executables, suspended Job launch and native mirror census.
use super::{
    Error, Result,
    control::{self, InteractionMode},
    mirror::{self, CaptureSource, MirrorWindow, PhysicalRect, Snapshot, WindowProbe},
    session::{AdmittedRuntime, Cancellation, CommandOutput, OwnerAdb, Runner},
};
use crate::process::windows::Pins;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    mem::{size_of, size_of_val},
    os::windows::{ffi::OsStrExt, io::FromRawHandle},
    path::{Path, PathBuf},
    ptr,
    sync::{Arc, Mutex, OnceLock, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::*,
    System::{JobObjects::*, Pipes::CreatePipe, Threading::*},
    UI::WindowsAndMessaging::*,
};
use windows_sys::core::BOOL;

struct Handle(HANDLE);
// SAFETY: the handle is uniquely owned, kernel-managed and has no thread affinity.
unsafe impl Send for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        /* SAFETY: uniquely owned valid kernel handle. */
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct Pinned {
    path: PathBuf,
    pins: Pins,
    sha256: [u8; 32],
}
impl Pinned {
    fn open(path: &Path, expected: Option<[u8; 32]>) -> Result<Self> {
        let pins = Pins::open(path).map_err(|_| Error::RuntimeUnavailable)?;
        let mut file = pins
            .executable
            .file()
            .try_clone()
            .map_err(|_| Error::RuntimeUnavailable)?;
        file.seek(SeekFrom::Start(0))
            .map_err(|_| Error::RuntimeUnavailable)?;
        if file
            .metadata()
            .map_err(|_| Error::RuntimeUnavailable)?
            .len()
            > 128 * 1024 * 1024
        {
            return Err(Error::Limit);
        }
        let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
        let mut buffer = [0u8; 65536];
        let mut total = 0u64;
        loop {
            let n = file
                .read(&mut buffer)
                .map_err(|_| Error::RuntimeUnavailable)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > 128 * 1024 * 1024 {
                return Err(Error::Limit);
            }
            digest.update(&buffer[..n]);
        }
        let actual = digest.finish();
        if expected.is_some_and(|v| actual.as_ref() != v.as_slice()) {
            return Err(Error::RuntimeUnavailable);
        }
        let mut sha256 = [0u8; 32];
        sha256.copy_from_slice(actual.as_ref());
        pins.verify().map_err(|_| Error::RuntimeUnavailable)?;
        Ok(Self {
            path: path.to_owned(),
            pins,
            sha256,
        })
    }
    fn verify(&self) -> Result<()> {
        self.pins.verify().map_err(|_| Error::RuntimeUnavailable)
    }
}

fn wide(path: &Path) -> Result<Vec<u16>> {
    let chars: Vec<_> = path.as_os_str().encode_wide().collect();
    if chars.is_empty() || chars.len() > 32760 || chars.contains(&0) {
        return Err(Error::Invalid);
    }
    Ok(chars.into_iter().chain([0]).collect())
}

fn safe_arg(arg: &str) -> Result<()> {
    if arg.is_empty()
        || arg.len() > 256
        || !arg
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-./:=\\".contains(&b))
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

struct Attributes {
    _memory: Vec<usize>,
    ptr: LPPROC_THREAD_ATTRIBUTE_LIST,
}
impl Drop for Attributes {
    fn drop(&mut self) {
        /* SAFETY: initialized attribute list with live aligned storage. */
        unsafe {
            DeleteProcThreadAttributeList(self.ptr);
        }
    }
}

struct Child {
    process: Handle,
    _thread: Handle,
    job: Handle,
    stdout: Option<File>,
    assigned: bool,
}

fn unresolved_startups() -> &'static Mutex<Vec<Child>> {
    static CHILDREN: OnceLock<Mutex<Vec<Child>>> = OnceLock::new();
    CHILDREN.get_or_init(|| Mutex::new(Vec::new()))
}

fn retain_unresolved(child: Child) {
    unresolved_startups()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(child);
}

pub(super) fn retry_unresolved() -> usize {
    static RETRYING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if RETRYING.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return 1;
    }
    struct RetryGuard(&'static std::sync::atomic::AtomicBool);
    impl Drop for RetryGuard {
        fn drop(&mut self) {
            self.0.store(false, std::sync::atomic::Ordering::Release);
        }
    }
    let _retry_guard = RetryGuard(&RETRYING);
    let mut pending = {
        let mut guard = unresolved_startups()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *guard)
    };
    pending.retain(|child| child.retire(Duration::from_millis(100)).is_err());
    let mut guard = unresolved_startups()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    guard.extend(pending);
    guard.len()
}

impl Child {
    fn launch(
        exe: &Pinned,
        argv: &[String],
        env: &[(&str, String)],
        capture: bool,
        active_limit: u32,
        cancel: &Cancellation,
    ) -> Result<Self> {
        cancel.check()?;
        exe.verify()?;
        argv.iter().try_for_each(|arg| safe_arg(arg))?;
        let file = wide(&exe.path)?;
        let directory = wide(exe.path.parent().ok_or(Error::Invalid)?)?;
        let mut command: Vec<u16> = format!("\"{}\"", exe.path.display())
            .encode_utf16()
            .collect();
        for arg in argv {
            command.push(' ' as u16);
            command.extend(arg.encode_utf16());
        }
        command.push(0);
        if command.len() > 32760 {
            return Err(Error::Limit);
        }
        let mut environment = Vec::<u16>::new();
        for (key, value) in env {
            if key.contains(['=', '\0']) || value.contains('\0') {
                return Err(Error::Invalid);
            }
            environment.extend(format!("{key}={value}").encode_utf16());
            environment.push(0);
        }
        environment.push(0);
        if environment.len() == 1 {
            environment.push(0);
        }
        let security = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: ptr::null_mut(),
            bInheritHandle: 1,
        };
        let mut pipe_read = ptr::null_mut();
        let mut pipe_write = ptr::null_mut();
        // SAFETY: initialized output handles; only the write end will be inherited.
        if unsafe { CreatePipe(&mut pipe_read, &mut pipe_write, &security, 65536) } == 0 {
            return Err(Error::Platform);
        }
        let pipe_read = Handle(pipe_read);
        let pipe_write = Handle(pipe_write);
        // SAFETY: removes inheritance from the parent's read endpoint.
        if unsafe { SetHandleInformation(pipe_read.0, HANDLE_FLAG_INHERIT, 0) } == 0 {
            return Err(Error::Platform);
        }
        let nul: Vec<u16> = "NUL\0".encode_utf16().collect();
        // SAFETY: Windows null device; no shell or user path involved.
        let null_handle = unsafe {
            CreateFileW(
                nul.as_ptr(),
                FILE_GENERIC_READ | FILE_GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                &security,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        };
        if null_handle == INVALID_HANDLE_VALUE {
            return Err(Error::Platform);
        }
        let null_handle = Handle(null_handle);
        let mut length = 0usize;
        // SAFETY: documented attribute-list sizing query.
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut length);
        }
        if length == 0 || length > 65536 {
            return Err(Error::Platform);
        }
        let mut memory = vec![0usize; length.div_ceil(size_of::<usize>())];
        let pointer = memory.as_mut_ptr().cast();
        // SAFETY: initialized aligned storage of the queried size.
        if unsafe { InitializeProcThreadAttributeList(pointer, 1, 0, &mut length) } == 0 {
            return Err(Error::Platform);
        }
        let attributes = Attributes {
            _memory: memory,
            ptr: pointer,
        };
        let inherited = if capture {
            vec![null_handle.0, pipe_write.0]
        } else {
            vec![null_handle.0]
        };
        // SAFETY: exactly these owned stdin/output handles may cross the launch.
        if unsafe {
            UpdateProcThreadAttribute(
                attributes.ptr,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                inherited.as_ptr().cast(),
                size_of_val(inherited.as_slice()),
                ptr::null_mut(),
                ptr::null(),
            )
        } == 0
        {
            return Err(Error::Platform);
        }
        // SAFETY: new unnamed Job, never a pre-existing adb server's Job.
        let raw_job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if raw_job.is_null() {
            return Err(Error::Platform);
        }
        let job = Handle(raw_job);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        limits.BasicLimitInformation.ActiveProcessLimit = active_limit;
        limits.ProcessMemoryLimit = 512 * 1024 * 1024;
        // SAFETY: owned Job and correctly sized initialized limits.
        if unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(Error::Platform);
        }
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = null_handle.0;
        startup.StartupInfo.hStdOutput = if capture { pipe_write.0 } else { null_handle.0 };
        startup.StartupInfo.hStdError = null_handle.0;
        startup.lpAttributeList = attributes.ptr;
        let mut info = PROCESS_INFORMATION::default();
        // SAFETY: exact pinned executable, closed argv, explicit environment and
        // inherited-handle whitelist; suspended before any child code executes.
        if unsafe {
            CreateProcessW(
                file.as_ptr(),
                command.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                1,
                CREATE_NO_WINDOW
                    | CREATE_SUSPENDED
                    | EXTENDED_STARTUPINFO_PRESENT
                    | CREATE_UNICODE_ENVIRONMENT,
                environment.as_ptr().cast(),
                directory.as_ptr(),
                &startup.StartupInfo,
                &mut info,
            )
        } == 0
        {
            return Err(Error::Platform);
        }
        let process = Handle(info.hProcess);
        let thread = Handle(info.hThread);
        let mut child = Self {
            process,
            _thread: thread,
            job,
            stdout: if capture {
                // SAFETY: sole transfer of parent read endpoint to File.
                let file = unsafe { File::from_raw_handle(pipe_read.0.cast()) };
                std::mem::forget(pipe_read);
                Some(file)
            } else {
                None
            },
            assigned: false,
        };
        // Every failure after CreateProcess targets this exact suspended child.
        let finish = (|| {
            cancel.check()?;
            // SAFETY: owned suspended process is assigned to owned kill-on-close Job.
            if unsafe { AssignProcessToJobObject(child.job.0, child.process.0) } == 0 {
                return Err(Error::Platform);
            }
            child.assigned = true;
            exe.verify()?;
            let mut actual = vec![0u16; 32768];
            let mut count = actual.len() as u32;
            // SAFETY: bounded read of the opened child process's image name.
            if unsafe {
                QueryFullProcessImageNameW(child.process.0, 0, actual.as_mut_ptr(), &mut count)
            } == 0
            {
                return Err(Error::Platform);
            }
            let name =
                String::from_utf16(&actual[..count as usize]).map_err(|_| Error::Platform)?;
            if !name
                .trim_start_matches("\\\\?\\")
                .eq_ignore_ascii_case(&exe.path.to_string_lossy())
            {
                return Err(Error::RuntimeUnavailable);
            }
            cancel.check()?;
            // SAFETY: Job membership and exact image identity were verified.
            if unsafe { ResumeThread(child._thread.0) } == u32::MAX {
                return Err(Error::Platform);
            }
            Ok(())
        })();
        if let Err(error) = finish {
            let _ = child.kill();
            // If retirement cannot be proved, the caller retains the child via
            // a later dedicated owner. This branch cannot release it silently.
            if child.retire(Duration::from_secs(2)).is_err() {
                retain_unresolved(child);
                return Err(Error::RetirementPending);
            }
            return Err(error);
        }
        Ok(child)
    }
    fn pid(&self) -> Result<u32> {
        // SAFETY: retained exact process handle.
        let pid = unsafe { GetProcessId(self.process.0) };
        if pid == 0 {
            Err(Error::Platform)
        } else {
            Ok(pid)
        }
    }
    fn process_alive(&self) -> Result<bool> {
        // SAFETY: query-only exact retained main process, distinct from the
        // Job-wide retirement test that may still observe descendants.
        match unsafe { WaitForSingleObject(self.process.0, 0) } {
            WAIT_TIMEOUT => Ok(true),
            WAIT_OBJECT_0 => Ok(false),
            _ => Err(Error::RetirementPending),
        }
    }
    fn exited(&self) -> Result<bool> {
        // SAFETY: nonblocking wait on owned child handle.
        match unsafe { WaitForSingleObject(self.process.0, 0) } {
            WAIT_TIMEOUT => Ok(false),
            WAIT_OBJECT_0 => {
                let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
                // SAFETY: query owned Job's actual active descendants.
                if self.assigned
                    && unsafe {
                        QueryInformationJobObject(
                            self.job.0,
                            JobObjectBasicAccountingInformation,
                            (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                            size_of_val(&info) as u32,
                            ptr::null_mut(),
                        )
                    } == 0
                {
                    return Err(Error::RetirementPending);
                }
                Ok(!self.assigned || info.ActiveProcesses == 0)
            }
            _ => Err(Error::RetirementPending),
        }
    }
    fn code(&self) -> Result<u32> {
        let mut code = 0;
        // SAFETY: retained process handle and initialized output.
        if unsafe { GetExitCodeProcess(self.process.0, &mut code) } == 0 {
            Err(Error::Platform)
        } else {
            Ok(code)
        }
    }
    fn kill(&self) -> Result<()> {
        // SAFETY: targets only this owned Job, or exact suspended child before assignment.
        let ok = unsafe {
            if self.assigned {
                TerminateJobObject(self.job.0, 2)
            } else {
                TerminateProcess(self.process.0, 2)
            }
        };
        if ok == 0 && !self.exited()? {
            Err(Error::RetirementPending)
        } else {
            Ok(())
        }
    }
    fn retire(&self, limit: Duration) -> Result<()> {
        let _ = self.kill();
        let deadline = Instant::now() + limit;
        loop {
            if self.exited()? {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(Error::RetirementPending);
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}

pub(super) struct WindowsJobRunner {
    adb: Pinned,
    client: Pinned,
    _server: Pinned,
    _dlls: Vec<Pinned>,
    active: Option<Child>,
    mirror: Option<Child>,
    io: Option<thread::JoinHandle<Result<Vec<u8>>>>,
    mirror_handle: Arc<Mutex<Option<Handle>>>,
    identity: [u8; 32],
}

fn millis() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Platform)?
        .as_millis()
        .try_into()
        .map_err(|_| Error::Limit)
}

impl Runner for WindowsJobRunner {
    fn run(
        &mut self,
        args: &[String],
        cap: usize,
        timeout: Duration,
        cancel: &Cancellation,
    ) -> Result<CommandOutput> {
        if self.active.is_some() || self.io.is_some() {
            return Err(Error::RetirementPending);
        }
        let started_ms = millis()?;
        let env = [(
            "PATH",
            self.adb
                .path
                .parent()
                .ok_or(Error::Invalid)?
                .display()
                .to_string(),
        )];
        let mut child = Child::launch(&self.adb, args, &env, true, 1, cancel)?;
        let mut stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                if child.retire(Duration::from_secs(2)).is_err() {
                    retain_unresolved(child);
                }
                return Err(Error::RetirementPending);
            }
        };
        let (tx, rx) = mpsc::sync_channel(1);
        let spawned = thread::Builder::new()
            .name("vw-android-adb-output".into())
            .spawn(move || {
                let read = (|| {
                    let mut result = Vec::new();
                    let mut buffer = [0u8; 8192];
                    loop {
                        let n = stdout.read(&mut buffer).map_err(|_| Error::Platform)?;
                        if n == 0 {
                            break;
                        }
                        if result.len().checked_add(n).is_none_or(|total| total > cap) {
                            return Err(Error::Limit);
                        }
                        result.extend_from_slice(&buffer[..n]);
                    }
                    Ok(result)
                })();
                let _ = tx.send(());
                read
            });
        let io = match spawned {
            Ok(io) => io,
            Err(_) => {
                if child.retire(Duration::from_secs(2)).is_err() {
                    retain_unresolved(child);
                }
                return Err(Error::RetirementPending);
            }
        };
        self.active = Some(child);
        self.io = Some(io);
        // A bounded single producer has no detached reader. On timeout/cancel,
        // close kills the exact Job and joins before release; uncertainty retains.
        let deadline = Instant::now() + timeout;
        let mut received: Option<Vec<u8>> = None;
        loop {
            if let Err(error) = cancel.check() {
                let _ = self.close_active();
                return Err(error);
            }
            if received.is_none()
                && self
                    .io
                    .as_ref()
                    .is_some_and(thread::JoinHandle::is_finished)
            {
                match self.io.take().ok_or(Error::Platform)?.join() {
                    Ok(Ok(bytes)) => received = Some(bytes),
                    Ok(Err(error)) => {
                        let _ = self.close_active();
                        return Err(error);
                    }
                    Err(_) => {
                        let _ = self.close_active();
                        return Err(Error::Platform);
                    }
                }
            }
            if received.is_some() {
                let child = self.active.as_ref().ok_or(Error::Platform)?;
                if !child.exited()? {
                    if Instant::now() >= deadline {
                        let _ = self.close_active();
                        return Err(Error::Timeout);
                    }
                } else {
                    let code = child.code()?;
                    self.active = None;
                    return Ok(CommandOutput {
                        code,
                        bytes: received.take().ok_or(Error::Platform)?,
                        started_ms,
                        finished_ms: millis()?,
                        finished_timestamp_ns: vw_capture::windows::clock_ns()
                            .map_err(|_| Error::Platform)?,
                    });
                }
            }
            if Instant::now() >= deadline {
                let _ = self.close_active();
                return Err(Error::Timeout);
            }
            let _ = rx.recv_timeout(Duration::from_millis(5));
        }
    }
    fn launch_mirror(
        &mut self,
        serial: &str,
        mode: InteractionMode,
        cancel: &Cancellation,
    ) -> Result<u32> {
        if self.mirror.is_some() {
            return Err(Error::RetirementPending);
        }
        safe_arg(serial)?;
        self.adb.verify()?;
        self.client.verify()?;
        self._server.verify()?;
        for dll in &self._dlls {
            dll.verify()?;
        }
        let env = [
            ("ADB", self.adb.path.display().to_string()),
            (
                "SCRCPY_SERVER_PATH",
                self._server.path.display().to_string(),
            ),
            (
                "PATH",
                self.adb
                    .path
                    .parent()
                    .ok_or(Error::Invalid)?
                    .display()
                    .to_string(),
            ),
        ];
        let argv = control::mirror_argv(serial, mode);
        let child = Child::launch(&self.client, &argv, &env, false, 2, cancel)?;
        let pid = match child.pid() {
            Ok(pid) => pid,
            Err(_) => {
                if child.retire(Duration::from_secs(2)).is_err() {
                    retain_unresolved(child);
                }
                return Err(Error::RetirementPending);
            }
        };
        let mut duplicate = ptr::null_mut();
        // SAFETY: duplicate only the retained exact owned process handle for
        // fresh liveness and PID checks during native window enumeration.
        if unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                child.process.0,
                GetCurrentProcess(),
                &mut duplicate,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        } == 0
        {
            if child.retire(Duration::from_secs(2)).is_err() {
                retain_unresolved(child);
            }
            return Err(Error::RetirementPending);
        }
        *self.mirror_handle.lock().unwrap_or_else(|e| e.into_inner()) = Some(Handle(duplicate));
        self.mirror = Some(child);
        Ok(pid)
    }
    fn retire_mirror(&mut self) -> Result<()> {
        let io = self.close_active();
        let mirror = self.mirror.as_ref().ok_or(Error::RetirementPending)?;
        let retirement = mirror.retire(Duration::from_secs(2));
        if retirement.is_ok() {
            self.mirror = None;
            *self.mirror_handle.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        if io.is_err() || retirement.is_err() {
            Err(Error::RetirementPending)
        } else {
            Ok(())
        }
    }
    fn mirror_alive(&self) -> Result<bool> {
        match &self.mirror {
            Some(mirror) => mirror.process_alive(),
            None => Ok(false),
        }
    }
    fn binary_identity(&self) -> Result<[u8; 32]> {
        self.adb.verify()?;
        self.client.verify()?;
        self._server.verify()?;
        for dll in &self._dlls {
            dll.verify()?;
        }
        Ok(self.identity)
    }
    fn settle_io(&mut self) -> Result<()> {
        self.close_active()
    }
    fn close(&mut self) -> Result<()> {
        if self.mirror.is_some() {
            self.retire_mirror()
        } else {
            self.close_active()
        }
    }
}

impl WindowsJobRunner {
    fn close_active(&mut self) -> Result<()> {
        if let Some(active) = &self.active {
            active.retire(Duration::from_secs(2))?;
        }
        if let Some(io) = self.io.take() {
            let _ = io.join().map_err(|_| Error::RetirementPending)?;
        }
        self.active = None;
        Ok(())
    }
}

pub(super) fn open(
    runtime: AdmittedRuntime,
    adb: OwnerAdb,
) -> Result<(Box<dyn Runner>, Box<dyn WindowProbe>)> {
    if retry_unresolved() != 0 {
        return Err(Error::RetirementPending);
    }
    if runtime.client.parent() == adb.path.parent() {
        return Err(Error::RuntimeUnavailable);
    }
    let adb = Pinned::open(&adb.path, None)?;
    const BUNDLED_ADB: [u8; 32] = [
        0x95, 0x7e, 0x46, 0xb8, 0x61, 0x5f, 0x7a, 0xf5, 0xb7, 0x29, 0x2a, 0x2d, 0xda, 0xbe, 0x98,
        0xd2, 0xe6, 0x19, 0x40, 0xc3, 0xfb, 0x2b, 0x05, 0x45, 0x75, 0x65, 0x07, 0xf0, 0x80, 0x61,
        0x3e, 0x71,
    ];
    if adb.sha256 == BUNDLED_ADB {
        return Err(Error::RuntimeUnavailable);
    }
    let client = Pinned::open(&runtime.client, Some(runtime.client_sha256))?;
    let server = Pinned::open(&runtime.server, Some(runtime.server_sha256))?;
    let names = [
        "SDL3.dll",
        "avcodec-62.dll",
        "avformat-62.dll",
        "avutil-60.dll",
        "libusb-1.0.dll",
        "swresample-6.dll",
    ];
    let mut dlls = Vec::new();
    for name in names {
        let (path, digest) = runtime
            .dlls
            .iter()
            .find(|(path, _)| {
                path.file_name()
                    .and_then(|actual| actual.to_str())
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(name))
            })
            .ok_or(Error::RuntimeUnavailable)?;
        dlls.push(Pinned::open(path, Some(*digest))?);
    }
    fn identity_part(hasher: &mut blake3::Hasher, pin: &Pinned) {
        let wide: Vec<u16> = pin.path.as_os_str().encode_wide().collect();
        hasher.update(&(wide.len() as u32).to_le_bytes());
        for code in wide {
            hasher.update(&code.to_le_bytes());
        }
        hasher.update(&pin.sha256);
    }
    let mut digest = blake3::Hasher::new();
    for pin in [&adb, &client, &server] {
        identity_part(&mut digest, pin);
    }
    for pin in &dlls {
        identity_part(&mut digest, pin);
    }
    let identity = *digest.finalize().as_bytes();
    let mirror_handle = Arc::new(Mutex::new(None));
    Ok((
        Box::new(WindowsJobRunner {
            adb,
            client,
            _server: server,
            _dlls: dlls,
            active: None,
            mirror: None,
            io: None,
            mirror_handle: mirror_handle.clone(),
            identity,
        }),
        Box::new(NativeWindowProbe {
            owned: mirror_handle,
        }),
    ))
}

struct NativeWindowProbe {
    owned: Arc<Mutex<Option<Handle>>>,
}
struct Census {
    pid: u32,
    selected_hwnd: HWND,
    mirrors: Vec<MirrorWindow>,
    visited: usize,
    failed: bool,
}

unsafe extern "system" fn record_window(hwnd: HWND, value: LPARAM) -> BOOL {
    // SAFETY: EnumWindows invokes synchronously with this call's live Census.
    let census = unsafe { &mut *(value as *mut Census) };
    census.visited += 1;
    if census.visited > 4096 {
        census.failed = true;
        return 0;
    }
    let mut pid = 0;
    // SAFETY: query-only real window owner identity.
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    if pid != census.pid {
        return 1;
    }
    let mut rect = RECT::default();
    // SAFETY: query-only live HWND and initialized RECT.
    if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
        census.failed = true;
        return 0;
    }
    let physical = PhysicalRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    };
    // SAFETY: query-only current window tree/visibility.
    census.mirrors.push(MirrorWindow {
        hwnd: hwnd as isize,
        rect: physical,
        visible: unsafe { IsWindowVisible(hwnd) } != 0,
        child_of_source: !census.selected_hwnd.is_null()
            && unsafe { IsChild(census.selected_hwnd, hwnd) } != 0,
        source_child_of_mirror: !census.selected_hwnd.is_null()
            && unsafe { IsChild(hwnd, census.selected_hwnd) } != 0,
    });
    1
}

unsafe extern "system" fn top_window_callback(hwnd: HWND, value: LPARAM) -> BOOL {
    // SAFETY: same synchronous live census passed to both native enumerators.
    if unsafe { record_window(hwnd, value) } == 0 {
        return 0;
    }
    // SAFETY: enumerate all actual descendants, including an owned mirror
    // child of a foreign top-level window. Callback enforces the census cap.
    unsafe {
        EnumChildWindows(hwnd, Some(record_window), value);
    }
    // SAFETY: live context, still within synchronous EnumWindows callback.
    if unsafe { (*(value as *const Census)).failed } {
        0
    } else {
        1
    }
}

impl WindowProbe for NativeWindowProbe {
    fn current(&mut self, source: CaptureSource, owned_scrcpy_pid: u32) -> Result<Snapshot> {
        if owned_scrcpy_pid == 0 {
            return Err(Error::Platform);
        }
        let live = || -> Result<()> {
            let guard = self.owned.lock().unwrap_or_else(|e| e.into_inner());
            let handle = guard.as_ref().ok_or(Error::Platform)?;
            // SAFETY: query-only exact retained process handle; a reused PID
            // cannot authorize an unrelated window after owner exit.
            if unsafe { WaitForSingleObject(handle.0, 0) } != WAIT_TIMEOUT
                || unsafe { GetProcessId(handle.0) } != owned_scrcpy_pid
            {
                return Err(Error::Platform);
            }
            Ok(())
        };
        live()?;
        let (selected, selected_hwnd, monitor_bounds, selected_pid) = match source {
            CaptureSource::Window { hwnd } => {
                let native = hwnd as HWND;
                // SAFETY: validates current native HWND, then reads live bounds.
                if native.is_null()
                    || unsafe { IsWindow(native) } == 0
                    || unsafe { IsWindowVisible(native) } == 0
                {
                    return Err(Error::Platform);
                }
                let mut rect = RECT::default();
                if unsafe { GetWindowRect(native, &mut rect) } == 0 {
                    return Err(Error::Platform);
                }
                let mut pid = 0;
                // SAFETY: query-only selected HWND owner PID for second-sample identity.
                unsafe {
                    GetWindowThreadProcessId(native, &mut pid);
                }
                if pid == 0 {
                    return Err(Error::Platform);
                }
                (
                    PhysicalRect {
                        left: rect.left,
                        top: rect.top,
                        right: rect.right,
                        bottom: rect.bottom,
                    },
                    native,
                    None,
                    Some(pid),
                )
            }
            CaptureSource::Monitor { monitor } | CaptureSource::Region { monitor, .. } => {
                let mut info = MONITORINFO {
                    cbSize: size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                // SAFETY: current monitor handle and initialized size-tagged output.
                if monitor == 0 || unsafe { GetMonitorInfoW(monitor as HMONITOR, &mut info) } == 0 {
                    return Err(Error::Platform);
                }
                let whole = PhysicalRect {
                    left: info.rcMonitor.left,
                    top: info.rcMonitor.top,
                    right: info.rcMonitor.right,
                    bottom: info.rcMonitor.bottom,
                };
                let selected = if let CaptureSource::Region { rect, .. } = source {
                    if !rect.valid()
                        || rect.left < whole.left
                        || rect.top < whole.top
                        || rect.right > whole.right
                        || rect.bottom > whole.bottom
                    {
                        return Err(Error::Invalid);
                    }
                    rect
                } else {
                    whole
                };
                (selected, ptr::null_mut(), Some(whole), None)
            }
        };
        let mut census = Census {
            pid: owned_scrcpy_pid,
            selected_hwnd,
            mirrors: Vec::new(),
            visited: 0,
            failed: false,
        };
        // SAFETY: synchronous enumeration; callback only records current HWNDs.
        if unsafe {
            EnumWindows(
                Some(top_window_callback),
                (&mut census as *mut Census) as LPARAM,
            )
        } == 0
            || census.failed
        {
            return Err(Error::Platform);
        }
        live()?;
        if let CaptureSource::Window { hwnd } = source {
            let mut fresh = RECT::default();
            let mut fresh_pid = 0;
            // SAFETY: query-only same native HWND identity after enumeration.
            unsafe {
                GetWindowThreadProcessId(hwnd as HWND, &mut fresh_pid);
            }
            // SAFETY: requery same actual HWND after enumeration for currentness.
            if unsafe { IsWindow(hwnd as HWND) } == 0
                || unsafe { GetWindowRect(hwnd as HWND, &mut fresh) } == 0
                || Some(fresh_pid) != selected_pid
                || selected
                    != (PhysicalRect {
                        left: fresh.left,
                        top: fresh.top,
                        right: fresh.right,
                        bottom: fresh.bottom,
                    })
            {
                return Err(Error::Stale);
            }
        }
        if let (
            CaptureSource::Monitor { monitor } | CaptureSource::Region { monitor, .. },
            Some(was),
        ) = (source, monitor_bounds)
        {
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            // SAFETY: requery the same actual monitor after enumeration.
            if unsafe { GetMonitorInfoW(monitor as HMONITOR, &mut info) } == 0
                || was
                    != (PhysicalRect {
                        left: info.rcMonitor.left,
                        top: info.rcMonitor.top,
                        right: info.rcMonitor.right,
                        bottom: info.rcMonitor.bottom,
                    })
            {
                return Err(Error::Stale);
            }
        }
        let snapshot = Snapshot {
            selected,
            mirrors: census.mirrors,
            complete: true,
        };
        mirror::check(&snapshot)?;
        Ok(snapshot)
    }
}
