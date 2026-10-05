//! Read-only selected-editor observation, executed only in a parent-owned Job.
//! Receipts are private measurements, never shortcut/profile/input authority.
#[path = "editor_probe/paint.rs"]
mod paint;
#[path = "editor_probe/uia.rs"]
mod uia;

use crate::{Error, Result, platform, process::windows::Pins};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::Arc,
};
use vw_remote::{Target, profile::ImageIdentity};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HWND},
        Storage::FileSystem::GetDriveTypeW,
        System::{JobObjects::IsProcessInJob, Threading::*, WindowsProgramming::DRIVE_FIXED},
        UI::WindowsAndMessaging::GetForegroundWindow,
    },
    core::{BOOL, PWSTR},
};

const MAX_REQUEST: usize = 16 * 1024;
pub(crate) const MAX_OUTPUT: usize = 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    schema: u32,
    window: u64,
    process_id: u32,
    owner_pid: u32,
    token: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: u32,
    owner_pid: u32,
    target: Target,
    image: ImageIdentity,
    max_nodes: usize,
}
#[derive(Serialize)]
struct Receipt<T: Serialize> {
    schema: u32,
    target: Target,
    image: ImageIdentity,
    parent_job_present: bool,
    held_process_and_image_namespace: bool,
    observed_start_qpc_100ns: u64,
    observed_end_qpc_100ns: u64,
    foreground_same_selected_window_before: bool,
    foreground_same_selected_window_after: bool,
    observation: T,
    input_sent: bool,
    profile_authority: bool,
}
struct Process(HANDLE);
// SAFETY: uniquely owned query-only process handle; no thread-affine state,
// VM/input access, mutation or borrowed handle escapes. Every use is a query.
unsafe impl Send for Process {}
// SAFETY: concurrent retained-handle identity/lifetime queries do not mutate
// the process or owner. Drop closes the unique handle after ownership releases.
unsafe impl Sync for Process {}
impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: uniquely owned query-only process handle.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
pub(crate) struct Source {
    process: Process,
    pins: Pins,
    diagnostics: bool,
}
impl Source {
    pub(crate) fn open(target: &Target, owner_pid: u32) -> Result<Self> {
        Self::open_with(target, owner_pid, true)
    }
    pub(crate) fn open_quiet(target: &Target, owner_pid: u32) -> Result<Self> {
        Self::open_with(target, owner_pid, false)
    }
    fn open_with(target: &Target, owner_pid: u32, diagnostics: bool) -> Result<Self> {
        platform::unchanged(target, owner_pid)?;
        // SAFETY: exact revalidated selected PID; query-only, no VM/input access.
        let process = Process(unsafe {
            platform::api(
                "probe process",
                OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, target.process_id),
            )?
        });
        let mut path = [0u16; 32768];
        let mut length = path.len() as u32;
        // SAFETY: retained process and bounded initialized UTF-16 output.
        unsafe {
            platform::api(
                "probe process image",
                QueryFullProcessImageNameW(
                    process.0,
                    PROCESS_NAME_WIN32,
                    PWSTR(path.as_mut_ptr()),
                    &mut length,
                ),
            )?;
        }
        if length == 0 || length as usize >= path.len() {
            return Err(Error::Invalid);
        }
        let text = String::from_utf16(&path[..length as usize]).map_err(|_| Error::Invalid)?;
        let text = text.strip_prefix("\\\\?\\").unwrap_or(&text);
        let path = PathBuf::from(text);
        crate::process::windows::namespace(&path)?;
        let units: Vec<u16> = text[..3].encode_utf16().chain([0]).collect();
        // SAFETY: namespace proved a drive-letter root; initialized NUL terminator.
        if unsafe { GetDriveTypeW(windows::core::PCWSTR(units.as_ptr())) } != DRIVE_FIXED {
            return Err(Error::Unavailable);
        }
        // Ancestor/final handle leases reject reparse redirects and deny rename,
        // deletion and image writes through both inspections and UIA observation.
        let source = Self {
            process,
            pins: if diagnostics {
                Pins::open_diagnostic(&path)?
            } else {
                Pins::open(&path)?
            },
            diagnostics,
        };
        source.verify(target, owner_pid)?;
        Ok(source)
    }
    pub(crate) fn verify(&self, target: &Target, owner_pid: u32) -> Result<()> {
        if self.diagnostics {
            self.pins.verify_diagnostic()?;
        } else {
            self.pins.verify()?;
        }
        let mut created = windows::Win32::Foundation::FILETIME::default();
        let mut exited = created;
        let mut kernel = created;
        let mut user = created;
        // SAFETY: retained process handle and initialized, correctly typed outputs.
        unsafe {
            platform::api(
                "probe process lifetime",
                GetProcessTimes(
                    self.process.0,
                    &mut created,
                    &mut exited,
                    &mut kernel,
                    &mut user,
                ),
            )?;
        }
        if ((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
            != target.process_created
        {
            return Err(Error::TargetChanged);
        }
        platform::unchanged(target, owner_pid)
    }
}
pub(crate) fn image(target: &Target, owner_pid: u32) -> Result<ImageIdentity> {
    let fixed = vw_capture::windows::fixed_target(target.window, owner_pid)
        .map_err(|_| Error::TargetChanged)?;
    let v = vw_host::inspect_editor_identity_sync(
        fixed.into(),
        Arc::new(vw_capture::Cancellation::default()),
    )
    .map_err(|_| Error::Unavailable)?;
    Ok(ImageIdentity {
        executable_name: v.executable_name,
        executable_blake3: v.executable_blake3,
        executable_bytes: v.executable_bytes,
        file_version: v
            .file_version
            .map(|v| format!("{}.{}.{}.{}", v.major, v.minor, v.build, v.revision)),
        package_full_name: v.package_full_name,
        package_version: v.package_version,
    })
}
fn foreground(target: &Target) -> bool {
    // SAFETY: query only; this probe never changes focus or foreground state.
    unsafe { GetForegroundWindow() == HWND(target.window as usize as *mut std::ffi::c_void) }
}
fn emit<T: Serialize>(value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| Error::Invalid)?;
    if bytes.len() > MAX_OUTPUT {
        return Err(Error::Limit);
    }
    let mut output = std::io::stdout().lock();
    output.write_all(&bytes)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invocation {
    mode: String,
    request: serde_json::Value,
}
fn arguments() -> Result<Vec<String>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        // Child::launch supplies only explicit pipe handles and no command args.
        // Root-owned runner writes one bounded JSON request then closes stdin.
        let mut bytes = Vec::new();
        std::io::stdin()
            .lock()
            .take((MAX_REQUEST + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_REQUEST {
            return Err(Error::Limit);
        }
        let input: Invocation = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
        if !input.request.is_object() {
            return Err(Error::Invalid);
        }
        args = vec![
            input.mode,
            serde_json::to_string(&input.request).map_err(|_| Error::Invalid)?,
        ];
    }
    if args.len() != 2
        || args[1].len() > MAX_REQUEST
        || !matches!(
            args[0].as_str(),
            "--bind"
                | "--observe"
                | "--bind-paint"
                | "--observe-paint"
                | "--observe-controls"
                | "--observe-paint-controls"
        )
    {
        return Err(Error::Invalid);
    }
    Ok(args)
}
/// Root invokes under its bounded process/Job/stream owner. Cooperative checks
/// cannot interrupt blocked foreign providers, file IO or COM object release;
/// root must terminate the exact owned Job and observe actual process retirement.
pub fn run() -> Result<()> {
    let mut parent_job = BOOL::default();
    // SAFETY: current-process pseudo-handle; query-only Job membership output.
    unsafe {
        platform::api(
            "probe parent Job",
            IsProcessInJob(GetCurrentProcess(), None, &mut parent_job),
        )?;
    }
    if !parent_job.as_bool() {
        return Err(Error::Ungranted);
    }
    // Parent Job is already active before any foreign source/provider call.
    let _confinement = platform::confine()?;
    platform::initialize_hil_dpi()?;
    let args = arguments()?;
    match args[0].as_str() {
        "--observe-controls" => {
            let request: Request = serde_json::from_str(&args[1]).map_err(|_| Error::Invalid)?;
            request.target.validate()?;
            if request.schema != 1
                || request.owner_pid == 0
                || !(1..=uia::MAX_NODES).contains(&request.max_nodes)
            {
                return Err(Error::Invalid);
            }
            let source = Source::open(&request.target, request.owner_pid)?;
            if image(&request.target, request.owner_pid)? != request.image {
                return Err(Error::TargetChanged);
            }
            source.verify(&request.target, request.owner_pid)?;
            let start = platform::clock_100ns()?;
            let before_focus = foreground(&request.target);
            let observation = uia::observe_controls(&request.target)?;
            source.verify(&request.target, request.owner_pid)?;
            if image(&request.target, request.owner_pid)? != request.image {
                return Err(Error::TargetChanged);
            }
            source.verify(&request.target, request.owner_pid)?;
            let after_focus = foreground(&request.target);
            let end = platform::clock_100ns()?;
            emit(&Receipt {
                schema: 1,
                target: request.target,
                image: request.image,
                parent_job_present: true,
                held_process_and_image_namespace: true,
                observed_start_qpc_100ns: start,
                observed_end_qpc_100ns: end,
                foreground_same_selected_window_before: before_focus,
                foreground_same_selected_window_after: after_focus,
                observation,
                input_sent: false,
                profile_authority: false,
            })
        }
        "--bind-paint" | "--observe-paint" | "--observe-paint-controls" => {
            paint::run(&args[0], &args[1])
        }
        "--bind" => {
            let selected: Selection = serde_json::from_str(&args[1]).map_err(|_| Error::Invalid)?;
            if selected.schema != 1 || selected.owner_pid == 0 {
                return Err(Error::Invalid);
            }
            let target =
                platform::query_target(selected.window, selected.owner_pid, selected.token)?;
            if target.process_id != selected.process_id {
                return Err(Error::TargetChanged);
            }
            let source = Source::open(&target, selected.owner_pid)?;
            let start = platform::clock_100ns()?;
            let before_focus = foreground(&target);
            let first = image(&target, selected.owner_pid)?;
            source.verify(&target, selected.owner_pid)?;
            if image(&target, selected.owner_pid)? != first {
                return Err(Error::TargetChanged);
            }
            source.verify(&target, selected.owner_pid)?;
            let after_focus = foreground(&target);
            let end = platform::clock_100ns()?;
            emit(&Receipt {
                schema: 1,
                target,
                image: first,
                parent_job_present: true,
                held_process_and_image_namespace: true,
                observed_start_qpc_100ns: start,
                observed_end_qpc_100ns: end,
                foreground_same_selected_window_before: before_focus,
                foreground_same_selected_window_after: after_focus,
                observation: "native binding only; no UIA",
                input_sent: false,
                profile_authority: false,
            })
        }
        "--observe" => {
            let request: Request = serde_json::from_str(&args[1]).map_err(|_| Error::Invalid)?;
            request.target.validate()?;
            if request.schema != 1
                || request.owner_pid == 0
                || !(1..=uia::MAX_NODES).contains(&request.max_nodes)
            {
                return Err(Error::Invalid);
            }
            let source = Source::open(&request.target, request.owner_pid)?;
            if image(&request.target, request.owner_pid)? != request.image {
                return Err(Error::TargetChanged);
            }
            let start = platform::clock_100ns()?;
            let before_focus = foreground(&request.target);
            let observation = uia::observe(&request.target, request.max_nodes)?;
            source.verify(&request.target, request.owner_pid)?;
            if image(&request.target, request.owner_pid)? != request.image {
                return Err(Error::TargetChanged);
            }
            source.verify(&request.target, request.owner_pid)?;
            let after_focus = foreground(&request.target);
            let end = platform::clock_100ns()?;
            emit(&Receipt {
                schema: 1,
                target: request.target,
                image: request.image,
                parent_job_present: true,
                held_process_and_image_namespace: true,
                observed_start_qpc_100ns: start,
                observed_end_qpc_100ns: end,
                foreground_same_selected_window_before: before_focus,
                foreground_same_selected_window_after: after_focus,
                observation,
                input_sent: false,
                profile_authority: false,
            })
        }
        _ => Err(Error::Invalid),
    }
}
