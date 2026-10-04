//! Windows-only boundary; no credential enumeration, shell, or remote pipe.
use crate::Result;
use std::{
    fs::File,
    os::windows::io::{AsRawHandle, FromRawHandle},
    ptr,
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, Credentials::*, *},
    Storage::FileSystem::*,
    System::{JobObjects::*, Pipes::*, Threading::*},
};
use zeroize::{Zeroize, Zeroizing};
pub struct Handle(pub usize);
impl Handle {
    pub fn new(value: HANDLE) -> Result<Self> {
        if value.is_null() || value == INVALID_HANDLE_VALUE {
            Err("win32")
        } else {
            Ok(Self(value as usize))
        }
    }
    pub fn raw(&self) -> HANDLE {
        self.0 as HANDLE
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        /* SAFETY: a successful constructor uniquely owns this handle. */
        unsafe {
            CloseHandle(self.raw());
        }
    }
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
/// Node's Windows crypto initialization requires SystemRoot. Resolve that one
/// value from the OS; never inherit NODE_OPTIONS, OPENSSL_CONF or credentials.
pub fn node_environment(command: &mut std::process::Command) -> Result<()> {
    use std::os::windows::ffi::OsStringExt;
    let mut buffer = vec![0u16; 32768];
    // SAFETY: writable UTF-16 buffer with its exact bounded capacity.
    let length = unsafe {
        windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW(
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    } as usize;
    if length == 0 || length >= buffer.len() {
        return Err("system_directory");
    }
    let directory = std::path::PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length]));
    crate::package_reader::no_redirect(&directory)?;
    if !directory.is_dir() {
        return Err("system_directory");
    }
    command.env_clear().env("SystemRoot", directory);
    Ok(())
}

#[cfg(test)]
#[test]
fn node_environment_has_only_os_system_root() -> std::result::Result<(), Box<dyn std::error::Error>>
{
    let mut command = std::process::Command::new("unexecuted-fixture.exe");
    command.env("NODE_OPTIONS", "--require=unexecuted-fixture");
    command.env("OPENSSL_CONF", "unexecuted-fixture");
    command.env("SystemRoot", "unexecuted-fixture");
    node_environment(&mut command)?;
    let values: Vec<_> = command.get_envs().collect();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].0, "SystemRoot");
    let directory = values[0].1.ok_or("missing_system_root")?;
    assert!(std::path::Path::new(directory).is_absolute());
    assert!(std::path::Path::new(directory).is_dir());
    assert_ne!(directory, "unexecuted-fixture");
    Ok(())
}

fn sid_of(process: HANDLE) -> Result<String> {
    let mut token = ptr::null_mut();
    // SAFETY: process is live, output initialized; successful token is owned.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err("token");
    }
    let token = Handle::new(token)?;
    let mut length = 0;
    // SAFETY: size-only query with a null buffer.
    unsafe {
        GetTokenInformation(token.raw(), TokenUser, ptr::null_mut(), 0, &mut length);
    }
    if !(std::mem::size_of::<TOKEN_USER>() as u32..=65536).contains(&length) {
        return Err("token_size");
    }
    let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
    // SAFETY: u64 buffer aligns TOKEN_USER, length was bounded, pointers remain live.
    if unsafe {
        GetTokenInformation(
            token.raw(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            length,
            &mut length,
        )
    } == 0
    {
        return Err("token");
    }
    let mut text = ptr::null_mut();
    // SAFETY: the OS initialized TOKEN_USER and its SID inside the live buffer.
    if unsafe {
        ConvertSidToStringSidW(
            (*(buffer.as_ptr().cast::<TOKEN_USER>())).User.Sid,
            &mut text,
        )
    } == 0
    {
        return Err("sid");
    }
    if text.is_null() {
        return Err("sid");
    }
    let result = (|| {
        let mut n = 0; /* SAFETY: API returns a terminated allocated SID string. SID maximum is bounded by Win32. */
        unsafe {
            while n < 256 && *text.add(n) != 0 {
                n += 1;
            }
            if n == 256 {
                return Err("sid_size");
            }
            String::from_utf16(std::slice::from_raw_parts(text, n)).map_err(|_| "sid")
        }
    })();
    // SAFETY: matching allocator for ConvertSidToStringSidW.
    unsafe {
        LocalFree(text.cast());
    }
    result
}
pub fn own_sid() -> Result<String> {
    /* SAFETY: pseudo handle is valid and not closed. */
    sid_of(unsafe { GetCurrentProcess() })
}
pub fn pid_matches(pid: u32, sid: &str) -> Result<()> {
    // SAFETY: only query rights, no process mutation.
    let process = Handle::new(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) })?;
    if sid_of(process.raw())? != sid {
        return Err("foreign_user");
    }
    Ok(())
}
pub fn pipe_name(sid: &str) -> Result<String> {
    if !sid.starts_with("S-1-")
        || sid.len() > 184
        || !sid
            .bytes()
            .all(|b| b == b'S' || b == b'-' || b.is_ascii_digit())
    {
        return Err("sid");
    }
    Ok(format!(r"\\.\pipe\VisualWorkbench.Mcp.v1.{sid}"))
}
pub fn server_pipe(sid: &str, first: bool) -> Result<File> {
    let name = wide(&pipe_name(sid)?);
    let sddl = wide(&format!("D:P(A;;GA;;;{sid})"));
    let mut descriptor = ptr::null_mut();
    // SAFETY: fixed SDDL template with validated SID, initialized output.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err("acl");
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    // SAFETY: bounded buffers and valid security descriptor outlive creation.
    let raw = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX
                | if first {
                    FILE_FLAG_FIRST_PIPE_INSTANCE
                } else {
                    0
                },
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            6,
            65536,
            65536,
            1000,
            &attributes,
        )
    };
    // SAFETY: descriptor ownership belongs to LocalAlloc.
    unsafe {
        LocalFree(descriptor);
    }
    let handle = Handle::new(raw)?;
    let raw = handle.raw();
    std::mem::forget(handle);
    // SAFETY: unique valid pipe handle transfers into File RAII.
    Ok(unsafe { File::from_raw_handle(raw) })
}
pub fn connect_server(pipe: &File, sid: &str) -> Result<bool> {
    // SAFETY: live nonblocking pipe, no OVERLAPPED because PIPE_NOWAIT.
    if unsafe { ConnectNamedPipe(pipe.as_raw_handle(), ptr::null_mut()) } == 0 {
        // SAFETY: immediately reads current-thread last error.
        match unsafe { GetLastError() } {
            ERROR_PIPE_LISTENING => return Ok(false),
            ERROR_PIPE_CONNECTED => {}
            _ => return Err("pipe_connect"),
        }
    }
    let pid = client_pid(pipe)?;
    pid_matches(pid, sid)?;
    Ok(true)
}
pub fn client_pid(pipe: &File) -> Result<u32> {
    let mut pid = 0;
    // SAFETY: live connected server pipe and writable pid output.
    if unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle(), &mut pid) } == 0 {
        return Err("client_pid");
    }
    Ok(pid)
}
pub fn client_pipe(sid: &str) -> Result<File> {
    let name = wide(&pipe_name(sid)?);
    // SAFETY: fixed local name, no inheritance, open existing only.
    let raw = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    let handle = Handle::new(raw)?;
    let mut pid = 0;
    // SAFETY: live client pipe, writable output.
    if unsafe { GetNamedPipeServerProcessId(handle.raw(), &mut pid) } == 0 {
        return Err("server_pid");
    }
    pid_matches(pid, sid)?;
    let mode = PIPE_READMODE_BYTE | PIPE_NOWAIT;
    // SAFETY: valid mode pointer and live pipe.
    if unsafe { SetNamedPipeHandleState(handle.raw(), &mode, ptr::null(), ptr::null()) } == 0 {
        return Err("pipe_mode");
    }
    let raw = handle.raw();
    std::mem::forget(handle);
    // SAFETY: transfer unique owned handle to File.
    Ok(unsafe { File::from_raw_handle(raw) })
}
pub fn token() -> Result<Zeroizing<String>> {
    let mut target = wide("VisualWorkbench/MCP/LoopbackBearer/v1");
    let mut record = ptr::null_mut();
    // SAFETY: fixed terminated credential target and writable pointer.
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut record) } != 0 {
        if record.is_null() {
            return Err("credential");
        }
        // SAFETY: successful CredRead allocation is live until cleanup below.
        let result = unsafe {
            let r = &*record;
            if r.Type != CRED_TYPE_GENERIC
                || r.Persist != CRED_PERSIST_LOCAL_MACHINE
                || r.CredentialBlobSize != 64
                || r.CredentialBlob.is_null()
            {
                Err("credential")
            } else {
                let b = std::slice::from_raw_parts(r.CredentialBlob, 64);
                if b.iter()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
                {
                    String::from_utf8(b.to_vec())
                        .map(Zeroizing::new)
                        .map_err(|_| "credential")
                } else {
                    Err("credential")
                }
            }
        };
        // SAFETY: only wipe the bounded blob returned by the OS, then free.
        unsafe {
            if !(*record).CredentialBlob.is_null()
                && (*record).CredentialBlobSize <= CRED_MAX_CREDENTIAL_BLOB_SIZE
            {
                std::slice::from_raw_parts_mut(
                    (*record).CredentialBlob,
                    (*record).CredentialBlobSize as usize,
                )
                .zeroize();
            }
            CredFree(record.cast());
        }
        return result;
    }
    // SAFETY: read current-thread error only; no fallback on failure.
    if unsafe { GetLastError() } != ERROR_NOT_FOUND {
        return Err("credential");
    }
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|_| "entropy")?;
    let mut value = Zeroizing::new(
        random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
    );
    random.zeroize();
    // SAFETY: the only API consumer borrows these ASCII bytes for a synchronous
    // read; it cannot alter the String's UTF-8 invariant.
    let record = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        CredentialBlobSize: 64,
        CredentialBlob: unsafe { value.as_bytes_mut() }.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    // SAFETY: fixed target and valid bounded owned blob outlive synchronous call.
    if unsafe { CredWriteW(&record, 0) } == 0 {
        return Err("credential");
    }
    Ok(value)
}
/// Called before spawning Node. The supervisor and all descendants are in one
/// kill-on-close Job; parent death/pipe EOF cannot leave verifier grandchildren.
pub fn own_job() -> Result<Handle> {
    // SAFETY: unnamed Job, no inherited handle.
    let job = Handle::new(unsafe { CreateJobObjectW(ptr::null(), ptr::null()) })?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
    limits.BasicLimitInformation.ActiveProcessLimit = 4; // supervisor + SDK + 2 verifiers
    // SAFETY: initialized structure, exact byte size and live handles.
    if unsafe {
        SetInformationJobObject(
            job.raw(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
    } == 0
        || unsafe { AssignProcessToJobObject(job.raw(), GetCurrentProcess()) } == 0
    {
        return Err("job");
    }
    Ok(job)
}
