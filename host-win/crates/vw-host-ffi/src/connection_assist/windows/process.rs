//! The only ordinary subprocesses are the selected adb's local `version`
//! command and our fixed helper. Neither receives arbitrary command strings.
use super::{ConnectionRequest, Error, Handle, Result, wide};
use std::{
    ffi::OsStr,
    mem::{size_of, size_of_val},
    path::Path,
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_BROKEN_PIPE, GetLastError, HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation,
        WAIT_OBJECT_0,
    },
    Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_SHARE_READ, OPEN_EXISTING,
        ReadFile,
    },
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Pipes::{CreatePipe, PeekNamedPipe},
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, CreateProcessW, DeleteProcThreadAttributeList,
            EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, InitializeProcThreadAttributeList,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, ResumeThread,
            STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute,
            WaitForSingleObject,
        },
    },
};

const MAX_OUTPUT: usize = 65_535;
pub(super) struct Output {
    pub code: u32,
    pub output: Vec<u8>,
}
struct Attributes {
    _storage: Vec<usize>,
    pointer: windows_sys::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST,
}
impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: one successful initialized list, backing storage still lives.
        unsafe {
            DeleteProcThreadAttributeList(self.pointer);
        }
    }
}
struct Child {
    process: Handle,
    _thread: Handle,
    job: Handle,
}
impl Drop for Child {
    fn drop(&mut self) {
        // SAFETY: only this successfully created task child/job is targeted. A
        // suspended child is never resumed before job assignment succeeds.
        unsafe {
            TerminateJobObject(self.job.0, 30);
            TerminateProcess(self.process.0, 30);
            WaitForSingleObject(self.process.0, 1000);
        }
    }
}

pub(super) fn quote(arg: &OsStr) -> Result<Vec<u16>> {
    let arg = wide(arg)?;
    let mut result = vec![b'"' as u16];
    let mut slashes = 0;
    for &unit in &arg[..arg.len() - 1] {
        if unit == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        if unit == b'"' as u16 {
            result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
        } else {
            result.extend(std::iter::repeat_n(b'\\' as u16, slashes));
        }
        slashes = 0;
        result.push(unit);
    }
    result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    result.push(b'"' as u16);
    Ok(result)
}

pub(super) fn run(
    executable: &Path,
    args: &[String],
    request: &ConnectionRequest,
    limit: Duration,
) -> Result<Output> {
    if args.len() > 2 {
        return Err(Error::InvalidSelection);
    }
    request.check()?;
    let file = wide(executable.as_os_str())?;
    let directory = wide(
        executable
            .parent()
            .ok_or(Error::InvalidSelection)?
            .as_os_str(),
    )?;
    let mut command = quote(executable.as_os_str())?;
    for arg in args {
        command.push(b' ' as u16);
        command.extend(quote(OsStr::new(arg))?);
    }
    if command.len() > 30_000 {
        return Err(Error::InvalidSelection);
    }
    command.push(0);
    let security = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: 1,
    };
    let (mut reader, mut writer): (HANDLE, HANDLE) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: valid outputs and explicit inheritable pipe attributes.
    if unsafe { CreatePipe(&mut reader, &mut writer, &security, 4096) } == 0 {
        return Err(Error::WorkerUnavailable);
    }
    let reader = Handle::checked(reader)?;
    let writer = Handle::checked(writer)?;
    // SAFETY: remove inheritance from the parent's reading endpoint.
    if unsafe { SetHandleInformation(reader.0, HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(Error::WorkerUnavailable);
    }
    let null_name: Vec<u16> = "NUL\0".encode_utf16().collect();
    // SAFETY: opens the Windows null input device only; no filesystem mutation.
    let input = Handle::checked(unsafe {
        CreateFileW(
            null_name.as_ptr(),
            FILE_GENERIC_READ,
            FILE_SHARE_READ,
            &security,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    })?;
    let mut attribute_bytes = 0;
    // SAFETY: the documented sizing call fills the required byte count.
    unsafe {
        InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut attribute_bytes);
    }
    if attribute_bytes == 0 || attribute_bytes > 65_536 {
        return Err(Error::WorkerUnavailable);
    }
    let mut storage = vec![0usize; attribute_bytes.div_ceil(size_of::<usize>())];
    let pointer = storage.as_mut_ptr().cast();
    // SAFETY: properly aligned backing allocation is at least the required size.
    if unsafe { InitializeProcThreadAttributeList(pointer, 1, 0, &mut attribute_bytes) } == 0 {
        return Err(Error::WorkerUnavailable);
    }
    let attributes = Attributes {
        _storage: storage,
        pointer,
    };
    let inherited = [input.0, writer.0];
    // SAFETY: whitelist exactly two owned handles; no unrelated app handles can
    // be inherited even if another thread creates inheritable handles at once.
    if unsafe {
        UpdateProcThreadAttribute(
            attributes.pointer,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            inherited.as_ptr().cast(),
            size_of_val(&inherited),
            ptr::null_mut(),
            ptr::null(),
        )
    } == 0
    {
        return Err(Error::WorkerUnavailable);
    }
    // SAFETY: unnamed noninheritable job, with no external job opened or reused.
    let job = Handle::checked(unsafe { CreateJobObjectW(ptr::null(), ptr::null()) })?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: owned job and correctly-sized initialized limits structure.
    if unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            size_of_val(&limits) as u32,
        )
    } == 0
    {
        return Err(Error::WorkerUnavailable);
    }
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = input.0;
    startup.StartupInfo.hStdOutput = writer.0;
    startup.StartupInfo.hStdError = writer.0;
    startup.lpAttributeList = attributes.pointer;
    let mut process = PROCESS_INFORMATION::default();
    // SAFETY: explicit executable (no PATH or shell), bounded mutable command,
    // NUL-terminated strings, initialized STARTUPINFOEX and writable output.
    if unsafe {
        CreateProcessW(
            file.as_ptr(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            CREATE_NO_WINDOW | CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT,
            ptr::null(),
            directory.as_ptr(),
            &startup.StartupInfo,
            &mut process,
        )
    } == 0
    {
        return Err(Error::WorkerUnavailable);
    }
    let child = Child {
        process: Handle::checked(process.hProcess)?,
        _thread: Handle::checked(process.hThread)?,
        job,
    };
    // SAFETY: this child is still suspended, so no untracked descendant can be
    // created before mandatory job ownership succeeds.
    if unsafe { AssignProcessToJobObject(child.job.0, child.process.0) } == 0 {
        return Err(Error::WorkerUnavailable);
    }
    request.check()?;
    // SAFETY: owned initial thread from successful CreateProcessW.
    if unsafe { ResumeThread(child._thread.0) } == u32::MAX {
        return Err(Error::WorkerUnavailable);
    }
    drop(writer);
    drop(input);
    drop(attributes);
    let deadline = Instant::now() + limit;
    let mut output = Vec::new();
    loop {
        request.check()?;
        if Instant::now() >= deadline {
            return Err(Error::Timeout);
        }
        drain(reader.0, &mut output)?;
        // SAFETY: owned process handle, nonblocking wait and valid exit output.
        if unsafe { WaitForSingleObject(child.process.0, 0) } == WAIT_OBJECT_0 {
            drain(reader.0, &mut output)?;
            let mut code = 0;
            if unsafe { GetExitCodeProcess(child.process.0, &mut code) } == 0 {
                return Err(Error::WorkerUnavailable);
            }
            return Ok(Output { code, output });
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn drain(pipe: HANDLE, output: &mut Vec<u8>) -> Result<()> {
    // Do not wait for EOF: an untrusted tool might leave a descendant holding
    // stdout. The enclosing job kills such descendants on every return path.
    for _ in 0..17 {
        let mut available = 0;
        // SAFETY: this is the reading end of our live anonymous pipe.
        if unsafe {
            PeekNamedPipe(
                pipe,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                &mut available,
                ptr::null_mut(),
            )
        } == 0
        {
            // SAFETY: immediately reads the previous failing API's error.
            return if unsafe { GetLastError() } == ERROR_BROKEN_PIPE {
                Ok(())
            } else {
                Err(Error::WorkerUnavailable)
            };
        }
        if available == 0 {
            return Ok(());
        }
        if output.len().saturating_add(available as usize) > MAX_OUTPUT {
            return Err(Error::WorkerUnavailable);
        }
        let mut buffer = [0u8; 4096];
        let count = available.min(buffer.len() as u32);
        let mut read = 0;
        // SAFETY: available bytes mean this synchronous pipe read cannot wait
        // for the producer; only this owner reads the pipe.
        if unsafe { ReadFile(pipe, buffer.as_mut_ptr(), count, &mut read, ptr::null_mut()) } == 0
            || read == 0
        {
            return Err(Error::WorkerUnavailable);
        }
        output.extend_from_slice(&buffer[..read as usize]);
    }
    Err(Error::WorkerUnavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn executable_arguments_are_windows_argv_quoted_without_a_shell() -> Result<()> {
        let expected = |s: &str| s.encode_utf16().collect::<Vec<_>>();
        assert_eq!(
            quote(OsStr::new("C:\\Program Files\\adb.exe"))?,
            expected("\"C:\\Program Files\\adb.exe\"")
        );
        assert_eq!(quote(OsStr::new("end\\"))?, expected("\"end\\\\\""));
        assert_eq!(quote(OsStr::new("a\"b"))?, expected("\"a\\\"b\""));
        assert!(quote(OsStr::new("x\0y")).is_err());
        Ok(())
    }
}
