//! Exact opened namespace leases and suspended launch into one parent-owned Job.
use crate::{Error, Result};
use std::{
    fs::{File, OpenOptions},
    mem::{size_of, size_of_val},
    os::windows::{
        ffi::OsStrExt,
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::*,
    Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::*,
    System::{JobObjects::*, Pipes::CreatePipe, Threading::*},
};
fn fail() -> Error {
    Error::Unavailable
}
#[derive(Clone, Copy)]
enum PinContext {
    Ancestor(usize),
    Executable,
}
#[derive(serde::Serialize)]
struct PinDiagnostic {
    schema: u32,
    phase: &'static str,
    object_kind: &'static str,
    ancestor_ordinal: Option<usize>,
    raw_os_error: Option<i32>,
    io_kind: Option<String>,
    error: Error,
    input_sent: bool,
    profile_authority: bool,
}
fn diagnostic(
    context: Option<PinContext>,
    phase: &'static str,
    io: Option<&std::io::Error>,
    error: Error,
) -> Error {
    if let Some(context) = context {
        let (object_kind, ancestor_ordinal) = match context {
            PinContext::Ancestor(index) => ("ancestor", Some(index)),
            PinContext::Executable => ("executable", None),
        };
        let value = PinDiagnostic {
            schema: 1,
            phase,
            object_kind,
            ancestor_ordinal,
            raw_os_error: io.and_then(std::io::Error::raw_os_error),
            io_kind: io.map(|v| format!("{:?}", v.kind())),
            error: error.clone(),
            input_sent: false,
            profile_authority: false,
        };
        // Probe-only, failure-only stderr. No private path/error message is
        // serialized; ordinary helpers never enable this diagnostic context.
        if let Ok(json) = serde_json::to_string(&value) {
            eprintln!("editor-probe-pin: {json}");
        }
    }
    error
}
struct Handle(HANDLE);
// SAFETY: exclusively owned kernel handle; no thread-affine API or alias escapes.
unsafe impl Send for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        /* SAFETY: one uniquely owned valid handle. */
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn wide(path: &Path) -> Result<Vec<u16>> {
    let v: Vec<u16> = path.as_os_str().encode_wide().collect();
    if v.len() > 32760 || v.contains(&0) {
        return Err(Error::Invalid);
    }
    Ok(v.into_iter().chain([0]).collect())
}
pub fn namespace(path: &Path) -> Result<()> {
    let value = path.to_str().ok_or(Error::Invalid)?;
    if value.len() < 3
        || value.len() > 32760
        || !value.as_bytes()[0].is_ascii_alphabetic()
        || !value.starts_with(&format!("{}:\\", &value[..1]))
        || value[2..].contains([':', '/', '\0'])
    {
        return Err(Error::Invalid);
    }
    if value.len() == 3 {
        return Ok(());
    }
    if path.components().count() > 256
        || path.components().any(|c| {
            matches!(
                c,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
        || value[3..]
            .split('\\')
            .any(|c| c.is_empty() || c.ends_with(['.', ' ']))
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
pub struct Pin {
    file: File,
    path: PathBuf,
    directory: bool,
}
impl Pin {
    pub fn open(path: &Path, directory: bool) -> Result<Self> {
        Self::open_with(path, directory, None)
    }
    fn open_with(path: &Path, directory: bool, context: Option<PinContext>) -> Result<Self> {
        namespace(path).map_err(|error| diagnostic(context, "namespace", None, error))?;
        let mut options = OpenOptions::new();
        options.read(true);
        // Directory leases allow child creation but deny rename/deletion, matching
        // the existing physical runtime anchor guard. Reparse is checked by handle.
        options.access_mode(if directory {
            FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY
        } else {
            FILE_GENERIC_READ
        });
        options.share_mode(if directory {
            FILE_SHARE_READ | FILE_SHARE_WRITE
        } else {
            FILE_SHARE_READ
        });
        options.custom_flags(
            FILE_FLAG_OPEN_REPARSE_POINT
                | if directory {
                    FILE_FLAG_BACKUP_SEMANTICS
                } else {
                    0
                },
        );
        let file = options
            .open(path)
            .map_err(|error| diagnostic(context, "open", Some(&error), Error::Io))?;
        let pin = Self {
            file,
            path: path.to_owned(),
            directory,
        };
        pin.verify_with(context)?;
        Ok(pin)
    }
    pub fn verify(&self) -> Result<()> {
        self.verify_with(None)
    }
    fn verify_with(&self, context: Option<PinContext>) -> Result<()> {
        let raw = self.file.as_raw_handle() as HANDLE;
        let mut attributes = FILE_ATTRIBUTE_TAG_INFO::default();
        // SAFETY: live opened handle and correctly sized initialized output.
        if unsafe {
            GetFileInformationByHandleEx(
                raw,
                FileAttributeTagInfo,
                (&mut attributes as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
                size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        } == 0
        {
            // Capture the failing call's raw Win32 error before other OS calls.
            let error = std::io::Error::last_os_error();
            return Err(diagnostic(
                context,
                "handle_attributes",
                Some(&error),
                fail(),
            ));
        }
        if attributes.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || (attributes.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) != self.directory
        {
            return Err(diagnostic(context, "reparse_or_type", None, Error::Invalid));
        }
        let mut result = vec![0u16; 32768]; /* SAFETY: bounded writable UTF16 output and live handle. */
        let n = unsafe {
            GetFinalPathNameByHandleW(
                raw,
                result.as_mut_ptr(),
                result.len() as u32,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        } as usize;
        if n == 0 {
            let error = std::io::Error::last_os_error();
            return Err(diagnostic(context, "final_path", Some(&error), fail()));
        }
        if n >= result.len() {
            return Err(diagnostic(context, "final_path_bound", None, fail()));
        }
        let value = String::from_utf16(&result[..n])
            .map_err(|_| diagnostic(context, "final_path_utf16", None, Error::Invalid))?;
        let dos = value
            .strip_prefix("\\\\?\\")
            .ok_or_else(|| diagnostic(context, "final_path_prefix", None, Error::Invalid))?;
        namespace(Path::new(dos))
            .map_err(|error| diagnostic(context, "final_path_namespace", None, error))?;
        if !dos.eq_ignore_ascii_case(self.path.to_str().ok_or(Error::Invalid)?) {
            return Err(diagnostic(
                context,
                "final_path_identity",
                None,
                Error::Invalid,
            ));
        }
        Ok(())
    }
    pub fn file(&self) -> &File {
        &self.file
    }
}
pub struct Pins {
    pub executable: Pin,
    _ancestors: Vec<Pin>,
}
impl Pins {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with(path, false)
    }
    /// Read-only probe diagnostics only. Success and ordinary helper protocol
    /// output remain silent; every access/lease/namespace check is identical.
    pub fn open_diagnostic(path: &Path) -> Result<Self> {
        Self::open_with(path, true)
    }
    fn open_with(path: &Path, report: bool) -> Result<Self> {
        namespace(path)?;
        let mut parents = path
            .parent()
            .ok_or(Error::Invalid)?
            .ancestors()
            .collect::<Vec<_>>();
        parents.reverse();
        let mut ancestors = Vec::new();
        // Root-drive namespace is proved by its handle too; root's trailing slash
        // is permitted by the dedicated canonical drive-root branch.
        for (index, parent) in parents.into_iter().enumerate() {
            ancestors.push(if report {
                Pin::open_with(parent, true, Some(PinContext::Ancestor(index)))?
            } else {
                Pin::open(parent, true)?
            })
        }
        let executable = if report {
            Pin::open_with(path, false, Some(PinContext::Executable))?
        } else {
            Pin::open(path, false)?
        };
        let owner = Self {
            executable,
            _ancestors: ancestors,
        };
        owner.verify_with(report)?;
        Ok(owner)
    }
    pub fn verify(&self) -> Result<()> {
        self.verify_with(false)
    }
    pub fn verify_diagnostic(&self) -> Result<()> {
        self.verify_with(true)
    }
    fn verify_with(&self, report: bool) -> Result<()> {
        for (index, pin) in self._ancestors.iter().enumerate() {
            if report {
                pin.verify_with(Some(PinContext::Ancestor(index)))?
            } else {
                pin.verify()?
            }
        }
        if report {
            self.executable.verify_with(Some(PinContext::Executable))
        } else {
            self.executable.verify()
        }
    }
}
struct Attributes {
    _storage: Vec<usize>,
    pointer: LPPROC_THREAD_ATTRIBUTE_LIST,
}
impl Drop for Attributes {
    fn drop(&mut self) {
        /* SAFETY: successfully initialized list with live aligned backing storage. */
        unsafe {
            DeleteProcThreadAttributeList(self.pointer);
        }
    }
}
pub struct Child {
    process: Handle,
    _thread: Handle,
    job: Handle,
    pub stdin: Option<File>,
    pub stdout: Option<File>,
    pub startup_error: Option<Error>,
    assigned: bool,
}
impl Child {
    pub fn launch(path: &Path, pins: &Pins, cancel: &super::Cancellation) -> Result<Self> {
        cancel.check()?;
        pins.verify()?;
        let file = wide(path)?;
        let directory = wide(path.parent().ok_or(Error::Invalid)?)?;
        let mut command = vec![b'"' as u16];
        command.extend(path.as_os_str().encode_wide());
        command.extend([b'"' as u16, 0]);
        let security = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: ptr::null_mut(),
            bInheritHandle: 1,
        };
        let (mut in_read, mut in_write, mut out_read, mut out_write) = (
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        );
        // SAFETY: initialized outputs for two anonymous pipes; only explicit endpoints inherit.
        if unsafe { CreatePipe(&mut in_read, &mut in_write, &security, 65536) } == 0 {
            return Err(fail());
        }
        let in_read = Handle(in_read);
        let in_write = Handle(in_write);
        if unsafe { CreatePipe(&mut out_read, &mut out_write, &security, 65536) } == 0 {
            return Err(fail());
        }
        let out_read = Handle(out_read);
        let out_write = Handle(out_write);
        // SAFETY: parent endpoints are valid owned handles, removing inheritance only.
        if unsafe { SetHandleInformation(in_write.0, HANDLE_FLAG_INHERIT, 0) } == 0
            || unsafe { SetHandleInformation(out_read.0, HANDLE_FLAG_INHERIT, 0) } == 0
        {
            return Err(fail());
        }
        let mut attribute_bytes = 0; /* SAFETY: documented length query. */
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut attribute_bytes);
        }
        if attribute_bytes == 0 || attribute_bytes > 65536 {
            return Err(fail());
        }
        let mut storage = vec![0usize; attribute_bytes.div_ceil(size_of::<usize>())];
        let pointer = storage.as_mut_ptr().cast();
        // SAFETY: initialized aligned allocation is at least attribute_bytes long.
        if unsafe { InitializeProcThreadAttributeList(pointer, 1, 0, &mut attribute_bytes) } == 0 {
            return Err(fail());
        }
        let attributes = Attributes {
            _storage: storage,
            pointer,
        };
        let inherited = [in_read.0, out_write.0];
        // SAFETY: whitelist only this child's input/output handles, never unrelated JVM handles.
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
            return Err(fail());
        }
        // SAFETY: new unnamed noninheritable Job, owned solely by this parent.
        let raw_job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if raw_job.is_null() {
            return Err(fail());
        }
        let job = Handle(raw_job);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        limits.BasicLimitInformation.ActiveProcessLimit = 1;
        limits.ProcessMemoryLimit = 768 * 1024 * 1024;
        // SAFETY: live Job and correctly sized structure; no foreign process/job is touched.
        if unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(fail());
        }
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = in_read.0;
        startup.StartupInfo.hStdOutput = out_write.0;
        startup.StartupInfo.hStdError = out_write.0;
        startup.lpAttributeList = attributes.pointer;
        let mut info = PROCESS_INFORMATION::default();
        let environment = [0u16, 0];
        // SAFETY: explicit exact-pinned executable, bounded strings, inherited
        // whitelist and empty environment. Suspended before any helper code runs.
        cancel.check()?;
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
            return Err(fail());
        }
        let process = Handle(info.hProcess);
        let thread = Handle(info.hThread);
        // SAFETY: ownership of parent pipe endpoints transfers exactly once to File.
        let stdin = unsafe { File::from_raw_handle(in_write.0.cast()) };
        std::mem::forget(in_write);
        let stdout = unsafe { File::from_raw_handle(out_read.0.cast()) };
        std::mem::forget(out_read);
        let mut child = Self {
            process,
            _thread: thread,
            job,
            stdin: Some(stdin),
            stdout: Some(stdout),
            startup_error: None,
            assigned: false,
        };
        // After CreateProcess, errors remain on the owned suspended child, not an
        // early return that could drop namespace leases/permits before reaping.
        let assigned = std::cell::Cell::new(false);
        let result = crate::startup::finish(
            || {
                cancel.check()?;
                // SAFETY: owned still-suspended child and owned constrained Job.
                if unsafe { AssignProcessToJobObject(child.job.0, child.process.0) } == 0 {
                    return Err(fail());
                }
                assigned.set(true);
                Ok(())
            },
            || {
                cancel.check()?;
                pins.verify()?;
                let mut name = vec![0u16; 32768];
                let mut len = name.len() as u32;
                /* SAFETY: query-only selected child image name and bounded output. */
                if unsafe {
                    QueryFullProcessImageNameW(child.process.0, 0, name.as_mut_ptr(), &mut len)
                } == 0
                {
                    return Err(fail());
                }
                let actual =
                    String::from_utf16(&name[..len as usize]).map_err(|_| Error::Invalid)?;
                let actual = actual.strip_prefix("\\\\?\\").unwrap_or(&actual);
                if !actual.eq_ignore_ascii_case(path.to_str().ok_or(Error::Invalid)?) {
                    return Err(Error::Invalid);
                }
                pins.verify()?;
                cancel.check()
            },
            || {
                cancel.check()?;
                // SAFETY: prior Job assignment and exact opened namespace/image gates succeeded.
                if unsafe { ResumeThread(child._thread.0) } == u32::MAX {
                    return Err(fail());
                }
                Ok(())
            },
        );
        child.assigned = assigned.get();
        child.startup_error = result.err();
        Ok(child)
    }
    /// Query-only receipt after the owned process is signaled and its Job has
    /// zero active descendants. Exit code is measured, never inferred from JSON.
    pub fn retired_receipt(&self) -> Result<(u32, u32)> {
        if !self.exited()? {
            return Err(Error::RetirementPending);
        }
        let mut code = 0u32;
        // SAFETY: retained queryable child process handle, initialized output.
        let pid = unsafe { GetProcessId(self.process.0) };
        // SAFETY: same retained process, already observed signaled; any actual
        // terminal DWORD exit code is retained (including a nonzero259).
        if pid == 0 || unsafe { GetExitCodeProcess(self.process.0, &mut code) } == 0 {
            return Err(Error::RetirementPending);
        }
        Ok((pid, code))
    }
    pub fn kill_tree(&self) -> Result<()> {
        // SAFETY: only the parent-created owned tree; startup-assignment failure
        // targets that exact still-suspended child, never a discovered PID.
        let ok = unsafe {
            if self.assigned {
                TerminateJobObject(self.job.0, 2)
            } else {
                TerminateProcess(self.process.0, 2)
            }
        };
        if ok == 0 && !self.exited()? {
            return Err(Error::RetirementPending);
        }
        Ok(())
    }
    pub fn exited(&self) -> Result<bool> {
        // SAFETY: nonblocking wait on uniquely retained child process handle.
        match unsafe { WaitForSingleObject(self.process.0, 0) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => return Ok(false),
            _ => return Err(Error::RetirementPending),
        }
        if !self.assigned {
            return Ok(true);
        }
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        // SAFETY: retained Job and initialized correctly sized accounting output.
        if unsafe {
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
        Ok(info.ActiveProcesses == 0)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_drive_root_is_allowed() {
        assert_eq!(namespace(Path::new("C:\\")), Ok(()));
    }
    #[test]
    fn ambiguous_namespace_is_refused() {
        for value in [
            "C:\\a\\..\\b",
            "C:\\a\\.\\b",
            "C:\\a\\",
            "C:\\a:stream",
            "C:/a",
            "\\\\server\\x",
            "\\\\?\\C:\\a",
            "C:\\a.\\b",
        ] {
            assert_eq!(namespace(Path::new(value)), Err(Error::Invalid));
        }
    }
    #[test]
    fn opened_pins_include_live_drive_root() -> Result<()> {
        let mut path = std::env::temp_dir();
        let name = format!(
            "vw-remote-pin-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| Error::Io)?
                .as_nanos()
        );
        path.push(name);
        std::fs::write(&path, b"owned pin fixture")?;
        let result = Pins::open(&path).and_then(|pins| pins.verify());
        // The fixture's file is closed before task-owned deletion.
        std::fs::remove_file(path)?;
        result
    }
    #[test]
    fn each_directory_lease_alone_blocks_rename_without_a_final_file_handle() -> Result<()> {
        fn fixture_path(base: &Path, parts: &[&str]) -> PathBuf {
            parts
                .iter()
                .fold(base.to_owned(), |path, part| path.join(part))
        }
        struct Fixture(PathBuf);
        impl Fixture {
            fn cleanup(&self) -> std::io::Result<()> {
                // Only these empty, task-created directories; no recursive delete.
                for parts in [
                    &["root", "parent", "leaf"][..],
                    &["root", "parent"][..],
                    &["root"][..],
                ] {
                    let path = fixture_path(&self.0, parts);
                    let moved = path.with_extension("moved");
                    for candidate in [path, moved] {
                        match std::fs::remove_dir(candidate) {
                            Ok(()) => {}
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            Err(error) => return Err(error),
                        }
                    }
                }
                std::fs::remove_dir(&self.0)
            }
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                // Fallible setup/assertion paths drop every Pin before this owner.
                // An explicit cleanup result below is required on a passing path.
                let _ = self.cleanup();
            }
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Error::Io)?
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "vw-remote-directory-only-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir(&base)?;
        let fixture = Fixture(base);
        let result = (|| -> Result<()> {
            std::fs::create_dir(fixture.0.join("root"))?;
            std::fs::create_dir(fixture_path(&fixture.0, &["root", "parent"]))?;
            std::fs::create_dir(fixture_path(&fixture.0, &["root", "parent", "leaf"]))?;
            for parts in [
                &["root"][..],
                &["root", "parent"][..],
                &["root", "parent", "leaf"][..],
            ] {
                let path = fixture_path(&fixture.0, parts);
                let moved = path.with_extension("moved");
                // Exactly one ancestor handle, no child handle or final image.
                let pin = Pin::open(&path, true)?;
                let blocked = std::fs::rename(&path, &moved);
                if blocked
                    .as_ref()
                    .err()
                    .and_then(std::io::Error::raw_os_error)
                    != Some(32)
                    || !path.is_dir()
                    || moved.exists()
                {
                    drop(pin);
                    if moved.exists() {
                        std::fs::rename(&moved, &path)?;
                    }
                    return Err(Error::Invalid);
                }
                pin.verify()?;
                drop(pin);
                // The same exact rename succeeds after that one lease closes.
                std::fs::rename(&path, &moved)?;
                std::fs::rename(&moved, &path)?;
            }
            Ok(())
        })();
        let cleanup = fixture.cleanup().map_err(Error::from);
        result.and(cleanup)
    }
}
