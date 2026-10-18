use super::{Handle, PinnedRuntime, Profile, last_error, wide};
use crate::{Cancellation, Error, Result, WorkerLimits};
use std::{
    fs::File,
    io::{Read, Write},
    mem::{size_of, size_of_val},
    os::windows::io::AsRawHandle,
    ptr,
    time::{Duration, Instant},
};
use vw_document_protocol::{Completion, JobBinding, MAX_WIRE_OVERHEAD, Progress, ProtocolResult};
use windows_sys::Win32::{
    Foundation::{
        DuplicateHandle, ERROR_BROKEN_PIPE, GetLastError, HANDLE, HANDLE_FLAG_INHERIT,
        SetHandleInformation, WAIT_OBJECT_0,
    },
    Security::{
        EqualSid, GetTokenInformation, PSID, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES,
        TOKEN_APPCONTAINER_INFORMATION, TOKEN_GROUPS, TOKEN_INFORMATION_CLASS, TOKEN_QUERY,
        TokenAppContainerSid, TokenCapabilities, TokenIsAppContainer,
        TokenIsLessPrivilegedAppContainer,
    },
    Storage::FileSystem::{FILE_GENERIC_READ, ReadFile},
    System::{
        JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOB_OBJECT_LIMIT_PROCESS_TIME,
            JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS,
            JOB_OBJECT_UILIMIT_EXITWINDOWS, JOB_OBJECT_UILIMIT_GLOBALATOMS,
            JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
            JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
            JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation,
            QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
        },
        LibraryLoader::{
            LOAD_LIBRARY_SEARCH_APPLICATION_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
            SetDefaultDllDirectories, SetDllDirectoryW,
        },
        Pipes::{CreatePipe, PeekNamedPipe},
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
            DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
            GetExitCodeProcess, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
            OpenProcessToken, PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY,
            PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            PROC_THREAD_ATTRIBUTE_JOB_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
            PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW,
            UpdateProcThreadAttribute, WaitForSingleObject,
        },
        WindowsProgramming::{
            PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT,
            PROCESS_CREATION_CHILD_PROCESS_RESTRICTED,
        },
    },
};

const STDERR_LIMIT: u64 = 65_536;
const CLEANUP_MS: u32 = 5000;
struct Attributes {
    _storage: Vec<usize>,
    pointer: LPPROC_THREAD_ATTRIBUTE_LIST,
}
impl Attributes {
    fn new() -> Result<Self> {
        let mut bytes = 0;
        // SAFETY: documented sizing call; no existing list is supplied.
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), 5, 0, &mut bytes);
        }
        if bytes == 0 || bytes > 65_536 {
            return Err(Error::Isolation);
        }
        let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        let pointer = storage.as_mut_ptr().cast();
        // SAFETY: correctly aligned backing allocation at least bytes long.
        if unsafe { InitializeProcThreadAttributeList(pointer, 5, 0, &mut bytes) } == 0 {
            return Err(last_error());
        }
        Ok(Self {
            _storage: storage,
            pointer,
        })
    }
    fn set<T>(&self, key: u32, value: &T) -> Result<()> {
        // SAFETY: each caller keeps value and pointed-to arrays alive until
        // CreateProcess returns; list and its backing allocation remain live.
        if unsafe {
            UpdateProcThreadAttribute(
                self.pointer,
                0,
                key as usize,
                (value as *const T).cast(),
                size_of_val(value),
                ptr::null_mut(),
                ptr::null(),
            )
        } == 0
        {
            return Err(last_error());
        }
        Ok(())
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        /* SAFETY: initialized list and live storage. */
        unsafe {
            DeleteProcThreadAttributeList(self.pointer);
        }
    }
}

fn pipe() -> Result<(Handle, Handle)> {
    let security = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: 1,
    };
    let (mut reader, mut writer) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: fresh anonymous owned pipe, child inherits only writing endpoint.
    if unsafe { CreatePipe(&mut reader, &mut writer, &security, 4096) } == 0 {
        return Err(last_error());
    }
    let reader = Handle::checked(reader)?;
    let writer = Handle::checked(writer)?;
    // SAFETY: remove parent endpoint inheritance before any process is created.
    if unsafe { SetHandleInformation(reader.0, HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(last_error());
    }
    Ok((reader, writer))
}

pub(super) struct Child {
    process: Handle,
    thread: Handle,
    job: Handle,
    stdout: Handle,
    stderr: Handle,
    joined: bool,
}
impl Child {
    /// Returns a SUSPENDED already-Job-owned child. The enclosing Resources
    /// stores it before start performs any additional fallible operation.
    pub(super) fn launch(
        runtime: &PinnedRuntime,
        profile: &Profile,
        input: &File,
        limits: WorkerLimits,
    ) -> Result<Self> {
        let executable = wide(runtime.executable.as_os_str())?;
        let directory = wide(runtime.root().as_os_str())?;
        let mut command = quote(&executable)?;
        command.extend(" --document-worker-v1\0".encode_utf16());
        let (stdout, stdout_child) = pipe()?;
        let (stderr, stderr_child) = pipe()?;
        let mut input_child = ptr::null_mut();
        // SAFETY: reduced read-only access; duplicate shares a private, rewound
        // file cursor whose host owner does not read/write during child lifetime.
        if unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                input.as_raw_handle(),
                GetCurrentProcess(),
                &mut input_child,
                FILE_GENERIC_READ,
                1,
                0,
            )
        } == 0
        {
            return Err(last_error());
        }
        let input_child = Handle::checked(input_child)?;
        // SAFETY: new unnamed noninheritable Job, never an external process/job.
        let job = Handle::checked(unsafe { CreateJobObjectW(ptr::null(), ptr::null()) })?;
        let mut restrictions = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        restrictions.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_PROCESS_MEMORY
            | JOB_OBJECT_LIMIT_PROCESS_TIME;
        restrictions.BasicLimitInformation.ActiveProcessLimit = 1;
        restrictions.BasicLimitInformation.PerProcessUserTimeLimit = i64::try_from(
            limits
                .wall_time
                .as_secs()
                .checked_mul(10_000_000)
                .ok_or(Error::Limit)?,
        )
        .map_err(|_| Error::Limit)?;
        restrictions.ProcessMemoryLimit =
            usize::try_from(limits.process_memory_bytes).map_err(|_| Error::Limit)?;
        // SAFETY: owned Job and initialized bounded memory/CPU/process limits.
        if unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&raw const restrictions).cast(),
                size_of_val(&restrictions) as u32,
            )
        } == 0
        {
            return Err(last_error());
        }
        // All UI handles/clipboard/global atoms/desktop/display/exit access is
        // refused. This is additive to LPAC and is not the file/network policy.
        let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
            UIRestrictionsClass: JOB_OBJECT_UILIMIT_DESKTOP
                | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
                | JOB_OBJECT_UILIMIT_EXITWINDOWS
                | JOB_OBJECT_UILIMIT_GLOBALATOMS
                | JOB_OBJECT_UILIMIT_HANDLES
                | JOB_OBJECT_UILIMIT_READCLIPBOARD
                | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
                | JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
        };
        // SAFETY: matching documented UI restrictions structure/class.
        if unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectBasicUIRestrictions,
                (&raw const ui).cast(),
                size_of_val(&ui) as u32,
            )
        } == 0
        {
            return Err(last_error());
        }
        let attributes = Attributes::new()?;
        let inherited = [input_child.0, stdout_child.0, stderr_child.0];
        let capabilities = SECURITY_CAPABILITIES {
            AppContainerSid: profile.sid,
            Capabilities: ptr::null_mut(),
            CapabilityCount: 0,
            Reserved: 0,
        };
        let low = PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT;
        let children = PROCESS_CREATION_CHILD_PROCESS_RESTRICTED;
        let jobs = [job.0];
        attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &inherited)?;
        attributes.set(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &capabilities)?;
        attributes.set(PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY, &low)?;
        attributes.set(PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, &children)?;
        attributes.set(PROC_THREAD_ATTRIBUTE_JOB_LIST, &jobs)?;
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = input_child.0;
        startup.StartupInfo.hStdOutput = stdout_child.0;
        startup.StartupInfo.hStdError = stderr_child.0;
        startup.lpAttributeList = attributes.pointer;
        let mut process = PROCESS_INFORMATION::default();
        // SAFETY: explicit pinned executable, no shell/PATH, empty environment,
        // exact handle list, LPAC with no capabilities, mandatory atomic Job
        // assignment and suspended initial thread before any parser code runs.
        if unsafe {
            CreateProcessW(
                executable.as_ptr(),
                command.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                1,
                CREATE_NO_WINDOW
                    | CREATE_SUSPENDED
                    | CREATE_UNICODE_ENVIRONMENT
                    | EXTENDED_STARTUPINFO_PRESENT,
                [0_u16, 0].as_ptr().cast(),
                directory.as_ptr(),
                &startup.StartupInfo,
                &mut process,
            )
        } == 0
        {
            return Err(last_error());
        }
        // SAFETY invariant: successful CreateProcess supplies both owned handles.
        Ok(Self {
            process: Handle(process.hProcess),
            thread: Handle(process.hThread),
            job,
            stdout,
            stderr,
            joined: false,
        })
    }
    pub(super) fn start(&mut self, profile: &Profile) -> Result<()> {
        verify_token(self.process.0, Some(profile.sid))?;
        // SAFETY: initial owned suspended thread; Job ownership is already atomic.
        if unsafe { ResumeThread(self.thread.0) } == u32::MAX {
            return Err(last_error());
        }
        Ok(())
    }
    pub(super) fn terminate_and_join(&mut self) -> Result<()> {
        if self.joined {
            return Ok(());
        }
        // SAFETY: exact owned Job/process only. Job child creation is prohibited.
        unsafe {
            TerminateJobObject(self.job.0, 71);
        }
        // A finite wait; failed termination/join retains the whole owner in the
        // single quarantine slot. No pins/profile are released prematurely.
        if unsafe { WaitForSingleObject(self.process.0, CLEANUP_MS) } != WAIT_OBJECT_0 {
            return Err(Error::CleanupPending);
        }
        self.joined = true;
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn receive(
        &mut self,
        output: &mut dyn Write,
        binding: JobBinding,
        output_limit: u64,
        start: Instant,
        wall: Duration,
        cancel: &Cancellation,
        metadata: &mut dyn FnMut(&[u8]) -> ProtocolResult<()>,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<(Completion, u64)> {
        let mut input = Reader {
            stdout: &self.stdout,
            stderr: &self.stderr,
            start,
            wall,
            cancel,
            stdout_count: 0,
            stdout_limit: output_limit
                .checked_add(MAX_WIRE_OVERHEAD + 28)
                .ok_or(Error::Limit)?,
            stderr_count: 0,
            error: None,
        };
        let result = vw_document_protocol::receive_reply(
            &mut input,
            output,
            binding,
            output_limit,
            metadata,
            progress,
        );
        if let Some(error) = input.error.take() {
            return Err(error);
        }
        let complete = result?;
        loop {
            input.check()?;
            input.drain_stderr()?;
            // SAFETY: read-only nonblocking wait for our own process.
            if unsafe { WaitForSingleObject(self.process.0, 0) } == WAIT_OBJECT_0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        input.drain_stderr()?;
        let mut code = 0;
        // SAFETY: joined process handle and valid writable exit-code output.
        if unsafe { GetExitCodeProcess(self.process.0, &mut code) } == 0 {
            return Err(last_error());
        }
        if code != 0 {
            return Err(Error::WorkerExit);
        }
        self.joined = true;
        Ok((complete, input.stderr_count))
    }
}

struct Reader<'a> {
    stdout: &'a Handle,
    stderr: &'a Handle,
    start: Instant,
    wall: Duration,
    cancel: &'a Cancellation,
    stdout_count: u64,
    stdout_limit: u64,
    stderr_count: u64,
    error: Option<Error>,
}
impl Reader<'_> {
    fn check(&self) -> Result<()> {
        self.cancel.check()?;
        if self.start.elapsed() >= self.wall {
            Err(Error::Timeout)
        } else {
            Ok(())
        }
    }
    fn drain_stderr(&mut self) -> Result<()> {
        let mut buffer = [0_u8; 4096];
        for _ in 0..17 {
            self.check()?;
            let Some(available) = available(self.stderr)? else {
                return Ok(());
            };
            if available == 0 {
                return Ok(());
            }
            if self
                .stderr_count
                .checked_add(u64::from(available))
                .is_none_or(|v| v > STDERR_LIMIT)
            {
                return Err(Error::Output);
            }
            let read = read_available(self.stderr, &mut buffer[..(available as usize).min(4096)])?;
            self.stderr_count += read as u64;
            // Opaque bytes are discarded immediately, never logged or retained.
        }
        Err(Error::Output)
    }
    fn read_inner(&mut self, output: &mut [u8]) -> Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        loop {
            self.check()?;
            self.drain_stderr()?;
            let Some(count) = available(self.stdout)? else {
                return Ok(0);
            };
            if count == 0 {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            let length = output.len().min(count as usize);
            let next = self
                .stdout_count
                .checked_add(length as u64)
                .ok_or(Error::Output)?;
            if next > self.stdout_limit {
                return Err(Error::Output);
            }
            let read = read_available(self.stdout, &mut output[..length])?;
            self.stdout_count += read as u64;
            return Ok(read);
        }
    }
}
impl Read for Reader<'_> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        match self.read_inner(output) {
            Ok(count) => Ok(count),
            Err(error) => {
                self.error = Some(error);
                Err(std::io::ErrorKind::Other.into())
            }
        }
    }
}
fn available(handle: &Handle) -> Result<Option<u32>> {
    let mut count = 0;
    // SAFETY: private sole-reader anonymous pipe; no blocking read is issued.
    if unsafe {
        PeekNamedPipe(
            handle.0,
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            &mut count,
            ptr::null_mut(),
        )
    } == 0
    {
        // SAFETY: immediate last-error read for the failed pipe call.
        return if unsafe { GetLastError() } == ERROR_BROKEN_PIPE {
            Ok(None)
        } else {
            Err(last_error())
        };
    }
    Ok(Some(count))
}
fn read_available(handle: &Handle, output: &mut [u8]) -> Result<usize> {
    let mut read = 0;
    // SAFETY: caller obtained at least this many bytes from PeekNamedPipe and
    // owns the only reader, so this bounded synchronous read cannot wait.
    if unsafe {
        ReadFile(
            handle.0,
            output.as_mut_ptr(),
            output.len() as u32,
            &mut read,
            ptr::null_mut(),
        )
    } == 0
        || read == 0
    {
        return Err(last_error());
    }
    Ok(read as usize)
}
fn quote(terminated: &[u16]) -> Result<Vec<u16>> {
    if terminated.last() != Some(&0) {
        return Err(Error::Invalid);
    }
    let mut result = vec![b'"' as u16];
    let mut slashes = 0;
    for &unit in &terminated[..terminated.len() - 1] {
        if unit == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        result.extend(std::iter::repeat_n(
            b'\\' as u16,
            if unit == b'"' as u16 {
                slashes * 2 + 1
            } else {
                slashes
            },
        ));
        slashes = 0;
        result.push(unit);
    }
    result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    result.push(b'"' as u16);
    Ok(result)
}

fn token_buffer(token: &Handle, class: TOKEN_INFORMATION_CLASS) -> Result<Vec<usize>> {
    let mut buffer = vec![0usize; 4096 / size_of::<usize>()];
    let mut returned = 0;
    // SAFETY: aligned bounded buffer; any larger token metadata fails closed.
    if unsafe {
        GetTokenInformation(
            token.0,
            class,
            buffer.as_mut_ptr().cast(),
            4096,
            &mut returned,
        )
    } == 0
        || returned == 0
        || returned > 4096
    {
        return Err(Error::Isolation);
    }
    Ok(buffer)
}
fn verify_token(process: HANDLE, expected: Option<PSID>) -> Result<()> {
    let mut raw = ptr::null_mut();
    // SAFETY: read-only token query for our exact process or current worker.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw) } == 0 {
        return Err(Error::Isolation);
    }
    let token = Handle::checked(raw)?;
    for class in [TokenIsAppContainer, TokenIsLessPrivilegedAppContainer] {
        let buffer = token_buffer(&token, class)?;
        // SAFETY: aligned buffer and these two classes return one DWORD.
        if unsafe { *buffer.as_ptr().cast::<u32>() } != 1 {
            return Err(Error::Isolation);
        }
    }
    let capabilities = token_buffer(&token, TokenCapabilities)?;
    // SAFETY: the API returns a TOKEN_GROUPS at the aligned start of this buffer.
    if unsafe { (*capabilities.as_ptr().cast::<TOKEN_GROUPS>()).GroupCount } != 0 {
        return Err(Error::Isolation);
    }
    if let Some(expected) = expected {
        let actual = token_buffer(&token, TokenAppContainerSid)?;
        // SAFETY: both live OS-returned SIDs remain allocated through EqualSid.
        let sid = unsafe {
            (*actual.as_ptr().cast::<TOKEN_APPCONTAINER_INFORMATION>()).TokenAppContainer
        };
        if sid.is_null() || unsafe { EqualSid(sid, expected) } == 0 {
            return Err(Error::Isolation);
        }
    }
    Ok(())
}
/// Worker self-check before any source is decoded or native library is loaded.
pub fn verify_worker_token() -> Result<u64> {
    // SAFETY: pseudo-handle is read-only; never closed/retained by this API.
    verify_token(unsafe { GetCurrentProcess() }, None)?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    // SAFETY: null selects this process's containing Job, initialized output.
    if unsafe {
        QueryInformationJobObject(
            ptr::null_mut(),
            JobObjectExtendedLimitInformation,
            (&raw mut limits).cast(),
            size_of_val(&limits) as u32,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(Error::Isolation);
    }
    let required = JOB_OBJECT_LIMIT_ACTIVE_PROCESS
        | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY
        | JOB_OBJECT_LIMIT_PROCESS_TIME;
    if limits.BasicLimitInformation.LimitFlags & required != required
        || limits.BasicLimitInformation.ActiveProcessLimit != 1
        || !(64 * 1024 * 1024..=2 * 1024 * 1024 * 1024).contains(&limits.ProcessMemoryLimit)
        || limits.BasicLimitInformation.PerProcessUserTimeLimit <= 0
    {
        return Err(Error::Isolation);
    }
    // SAFETY: process-local DLL policy before loading PDFium. Only the pinned
    // executable directory and trusted Windows System32 remain search roots.
    if unsafe { SetDllDirectoryW([0_u16].as_ptr()) } == 0
        || unsafe {
            SetDefaultDllDirectories(
                LOAD_LIBRARY_SEARCH_APPLICATION_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        } == 0
    {
        return Err(Error::Isolation);
    }
    Ok(limits.ProcessMemoryLimit as u64)
}
