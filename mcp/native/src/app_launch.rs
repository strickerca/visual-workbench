//! Exact child ownership during minimized startup. A suspended process joins
//! its private kill-on-close Job before any application code can execute.
#![cfg(windows)]
use crate::{Result, windows::Handle};
use std::{
    path::Path,
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::System::{JobObjects::*, Threading::*};
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
pub struct AppLaunch {
    process: Handle,
    job: Handle,
    detached: bool,
}
impl AppLaunch {
    pub fn start(path: &Path) -> Result<Self> {
        crate::package_reader::no_redirect(path)?;
        let application = path.to_str().ok_or("app_path")?;
        if application.len() > 32760 || application.contains(['"', '\0']) {
            return Err("app_path");
        }
        let image = wide(application);
        let mut command = wide(&format!("\"{application}\" --mcp-minimized"));
        let cwd = wide(path.parent().and_then(Path::to_str).ok_or("app_path")?);
        // This explicit subset excludes JVM injection and provider credentials.
        let mut environment = Vec::<u16>::new();
        for key in [
            "APPDATA",
            "LOCALAPPDATA",
            "PATH",
            "SystemRoot",
            "TEMP",
            "TMP",
            "USERPROFILE",
            "WINDIR",
        ] {
            if let Some(value) = std::env::var_os(key) {
                let value = value.to_str().ok_or("app_environment")?;
                if value.len() > 32760 || value.contains('\0') {
                    return Err("app_environment");
                }
                environment.extend(format!("{key}={value}").encode_utf16());
                environment.push(0);
            }
        }
        if environment.is_empty() {
            environment.push(0);
        }
        environment.push(0);
        if environment.len() > 65536 {
            return Err("app_environment");
        }
        // SAFETY: unnamed noninherited Job, with initialized bounded limits.
        let job = Handle::new(unsafe { CreateJobObjectW(ptr::null(), ptr::null()) })?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        limits.BasicLimitInformation.ActiveProcessLimit = 16;
        // SAFETY: exact initialized structure size and owned live Job handle.
        if unsafe {
            SetInformationJobObject(
                job.raw(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err("app_job");
        }
        let startup = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        let mut process = PROCESS_INFORMATION::default();
        // SAFETY: explicit executable, terminated owned command/environment/CWD,
        // no inherited handles, initialized outputs, suspended before assignment.
        if unsafe {
            CreateProcessW(
                image.as_ptr(),
                command.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                0,
                CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
                environment.as_ptr().cast(),
                cwd.as_ptr(),
                &startup,
                &mut process,
            )
        } == 0
        {
            return Err("app_start");
        }
        let process_handle = Handle::new(process.hProcess)?;
        let thread = Handle::new(process.hThread)?;
        // SAFETY: only the exact newly created suspended process is assigned.
        if unsafe { AssignProcessToJobObject(job.raw(), process_handle.raw()) } == 0 {
            // SAFETY: the child has executed no user code and has no descendants.
            unsafe {
                TerminateProcess(process_handle.raw(), 1);
                WaitForSingleObject(process_handle.raw(), 5000);
            }
            return Err("app_job");
        }
        let result = Self {
            process: process_handle,
            job,
            detached: false,
        };
        // SAFETY: only this owned child's primary suspended thread is resumed.
        if unsafe { ResumeThread(thread.raw()) } == u32::MAX {
            return Err("app_start");
        }
        Ok(result)
    }
    /// Once the real authenticated local MCP pipe is ready the app becomes its
    /// own lifetime owner. Dropping this bridge must not terminate the user's app.
    pub fn detach(mut self) -> Result<()> {
        let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        // SAFETY: remove only this launch Job's temporary kill/process limits.
        if unsafe {
            SetInformationJobObject(
                self.job.raw(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err("app_detach");
        }
        self.detached = true;
        Ok(())
    }
    fn reap(&self) -> bool {
        // SAFETY: terminate only the private Job assigned before initial resume.
        unsafe {
            TerminateJobObject(self.job.raw(), 1);
        }
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(5) {
            let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            // SAFETY: initialized output structure and owned Job handle.
            if unsafe {
                QueryInformationJobObject(
                    self.job.raw(),
                    JobObjectBasicAccountingInformation,
                    (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                    std::mem::size_of_val(&info) as u32,
                    ptr::null_mut(),
                )
            } != 0
                && info.ActiveProcesses == 0
            {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }
    pub fn abort(mut self) -> Result<()> {
        let clean = self.reap();
        self.detached = true;
        if clean {
            Ok(())
        } else {
            Err("app_cleanup_uncertain")
        }
    }
}
impl Drop for AppLaunch {
    fn drop(&mut self) {
        if !self.detached {
            let _ = self.reap();
        }
        let _ = &self.process;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nonexistent_launcher_refuses_before_process_creation() {
        let temp = tempfile::tempdir().expect("fixture");
        assert!(AppLaunch::start(&temp.path().join("absent.exe")).is_err());
    }
    #[test]
    fn exact_fixture_child_is_reaped_after_failed_startup() {
        // Rust's test harness refuses --mcp-minimized. No test case or app starts;
        // the suspended exact test executable still exercises Job assignment.
        let image = std::env::current_exe().expect("test executable");
        let child = AppLaunch::start(&image).expect("owned suspended fixture");
        child.abort().expect("entire private Job empty");
    }
}
