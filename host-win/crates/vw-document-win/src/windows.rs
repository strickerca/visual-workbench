//! This is the only OS boundary. Profile/ACL/process mutations happen only when
//! the application explicitly invokes run; merely loading the crate does none.
mod acl;
mod pin;
mod process;
use crate::{Cancellation, Error, Result, ResultMetadata, WorkerLimits};
pub use pin::PinnedRuntime;
pub use process::verify_worker_token;
use std::{
    fs::File,
    io::{Seek, SeekFrom},
    path::Path,
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use vw_document_protocol::{Completion, JobBinding, Progress, ProtocolResult, Request};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE},
    Security::{
        Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom},
        FreeSid,
        Isolation::{CreateAppContainerProfile, DeleteAppContainerProfile},
        PSID,
    },
};

pub(super) struct Handle(pub HANDLE);
// SAFETY: owned Windows kernel handles may be transferred/shared between host
// threads; closing only occurs after all Rust owners/references have retired.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Handle {
    pub(super) fn checked(value: HANDLE) -> Result<Self> {
        if value.is_null() || value == INVALID_HANDLE_VALUE {
            Err(last_error())
        } else {
            Ok(Self(value))
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        /* SAFETY: one owned valid handle. */
        unsafe {
            CloseHandle(self.0);
        }
    }
}
pub(super) fn last_error() -> Error {
    /* SAFETY: immediate thread-local Win32 error. */
    Error::Windows(unsafe { GetLastError() })
}
pub(super) fn wide(value: &std::ffi::OsStr) -> Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let mut result: Vec<u16> = value.encode_wide().collect();
    if result.is_empty() || result.len() > 30_000 || result.contains(&0) {
        return Err(Error::Invalid);
    }
    result.push(0);
    Ok(result)
}
fn nonce() -> Result<[u8; 16]> {
    let mut value = [0_u8; 16];
    // SAFETY: fixed writable output and OS CSPRNG; no external provider handle.
    if unsafe {
        BCryptGenRandom(
            ptr::null_mut(),
            value.as_mut_ptr(),
            16,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    } < 0
        || value == [0; 16]
    {
        return Err(Error::Isolation);
    }
    Ok(value)
}
fn hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct Profile {
    name: Vec<u16>,
    sid: PSID,
    deleted: bool,
}
// SAFETY: SID memory is immutable until exclusive profile cleanup/Drop.
unsafe impl Send for Profile {}
impl Profile {
    fn create(id: [u8; 16]) -> Result<Self> {
        let name = wide(std::ffi::OsStr::new(&format!(
            "VisualWorkbench.Document.{}",
            hex(&id)
        )))?;
        let mut sid = ptr::null_mut();
        // SAFETY: bounded terminated profile name, zero capabilities. A name
        // collision fails; no existing profile is adopted, changed or deleted.
        let result = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                name.as_ptr(),
                name.as_ptr(),
                ptr::null(),
                0,
                &mut sid,
            )
        };
        if result < 0 || sid.is_null() {
            return Err(Error::Isolation);
        }
        Ok(Self {
            name,
            sid,
            deleted: false,
        })
    }
    fn cleanup(&mut self) -> Result<()> {
        if !self.deleted {
            // SAFETY: exact profile successfully created by this owner. The
            // process has joined and every runtime grant has been restored.
            if unsafe { DeleteAppContainerProfile(self.name.as_ptr()) } < 0 {
                return Err(Error::CleanupPending);
            }
            self.deleted = true;
        }
        Ok(())
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        /* SAFETY: CreateAppContainerProfile allocated this SID. */
        unsafe {
            FreeSid(self.sid);
        }
    }
}

static ADMISSION: AtomicBool = AtomicBool::new(false);
struct Admission;
impl Admission {
    fn acquire() -> Result<Self> {
        ADMISSION
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::Busy)?;
        Ok(Self)
    }
}
impl Drop for Admission {
    fn drop(&mut self) {
        ADMISSION.store(false, Ordering::Release);
    }
}
static QUARANTINE: Mutex<Option<Resources>> = Mutex::new(None);
struct Resources {
    runtime: Arc<PinnedRuntime>,
    profile: Option<Profile>,
    marker: pin::Marker,
    grants: Vec<acl::Grant>,
    child: Option<process::Child>,
}
impl Resources {
    fn cleanup(&mut self) -> Result<()> {
        if let Some(child) = &mut self.child {
            child.terminate_and_join()?;
        }
        self.child = None;
        // Retain each saved ACL until its exact restore succeeds. Never restore
        // over a changed third-party ACL or release binary pins before cleanup.
        while let Some(grant) = self.grants.last_mut() {
            grant.restore()?;
            let _ = self.grants.pop();
        }
        if let Some(profile) = &mut self.profile {
            profile.cleanup()?;
        }
        self.profile = None;
        self.marker.cleanup()
    }
}
struct RunGuard(Option<Resources>);
impl RunGuard {
    fn settle(&mut self) -> Result<()> {
        if let Some(mut resources) = self.0.take() {
            if resources.cleanup().is_err() {
                // Exactly one admitted owner can reach this slot. Retain all
                // resources once; do not repeat a blocking join in Drop.
                let mut slot = QUARANTINE
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                *slot = Some(resources);
                return Err(Error::CleanupPending);
            }
        }
        Ok(())
    }
}
impl Drop for RunGuard {
    fn drop(&mut self) {
        if self.0.is_none() {
            return;
        }
        let _ = self.settle();
    }
}
/// In-process quarantine only. Check runtime.workspace_cleanup_pending() too
/// before declaring a workspace clean after application restart.
pub fn cleanup_pending() -> bool {
    QUARANTINE.lock().map_or(true, |slot| slot.is_some())
}
/// Retry only an owner retained by this running process. This never adopts or
/// deletes an orphan marker/profile from an earlier process.
pub fn retry_cleanup() -> Result<()> {
    let _admission = Admission::acquire()?;
    let mut slot = QUARANTINE.lock().map_err(|_| Error::CleanupPending)?;
    if let Some(resources) = slot.as_mut() {
        resources.cleanup()?;
    }
    *slot = None;
    Ok(())
}

/// Retry only the quarantine retained by this exact pinned runtime owner.
/// A caller holding another cache version cannot settle or release these pins.
pub fn retry_runtime_cleanup(runtime: &Arc<PinnedRuntime>) -> Result<()> {
    let _admission = Admission::acquire()?;
    let mut slot = QUARANTINE.lock().map_err(|_| Error::CleanupPending)?;
    if let Some(resources) = slot.as_mut() {
        if !Arc::ptr_eq(&resources.runtime, runtime) {
            return Err(Error::CleanupPending);
        }
        resources.cleanup()?;
    }
    *slot = None;
    Ok(())
}

/// A complete private artifact. The caller still validates its captured project
/// revision, flushes/publishes atomically, and never exposes failed staging.
pub struct WorkerReply {
    pub completion: Completion,
    pub metadata: ResultMetadata,
    pub stderr_bytes_disposed: u64,
    file: File,
}
impl WorkerReply {
    pub fn private_file(&mut self) -> &mut File {
        &mut self.file
    }
    pub fn into_private_file(self) -> File {
        self.file
    }
}

/// Blocking bounded worker operation; call from an application-owned bounded
/// background executor. No arbitrary command/path is accepted from Request.
pub fn run(
    runtime: Arc<PinnedRuntime>,
    scratch_root: &Path,
    request: &Request,
    source: &[u8],
    limits: WorkerLimits,
    cancel: &Cancellation,
    validate_metadata: &mut dyn FnMut(&[u8]) -> ProtocolResult<()>,
    progress: &mut dyn FnMut(Progress),
) -> Result<WorkerReply> {
    let _admission = Admission::acquire()?;
    if cleanup_pending() {
        return Err(Error::CleanupPending);
    }
    limits.validate(request)?;
    cancel.check()?;
    let svg = matches!(
        request.operation,
        vw_document_protocol::Operation::SvgInfo
            | vw_document_protocol::Operation::SvgTile { .. }
            | vw_document_protocol::Operation::SvgCopy { .. }
    );
    if !svg && !runtime.pdfium_available() {
        return Err(Error::Integrity);
    }
    if matches!(
        request.operation,
        vw_document_protocol::Operation::PdfPcRedaction { .. }
    ) && !runtime.pc_redaction_available()
    {
        return Err(Error::Integrity);
    }
    let scratch_root = pin::absolute(scratch_root)?;
    if !scratch_root
        .to_str()
        .ok_or(Error::Invalid)?
        .eq_ignore_ascii_case(
            runtime
                .private_workspace_path()?
                .to_str()
                .ok_or(Error::Invalid)?,
        )
    {
        return Err(Error::Invalid);
    }
    if source.len() as u64 > request.limits.input_bytes
        || source.len() as u64 > vw_document_protocol::MAX_INPUT
        || (matches!(
            request.operation,
            vw_document_protocol::Operation::SvgInfo
                | vw_document_protocol::Operation::SvgTile { .. }
                | vw_document_protocol::Operation::SvgCopy { .. }
        ) && source.len() > 4 * 1024 * 1024)
    {
        return Err(Error::Limit);
    }
    let started = Instant::now();
    let binding = JobBinding {
        nonce: nonce()?,
        source_blake3: *blake3::hash(source).as_bytes(),
    };
    let scratch = pin::DirectoryChain::open(&scratch_root)?;
    let mut input = pin::private_file(&scratch_root, &binding.nonce, "request")?;
    vw_document_protocol::write_job(&mut input, binding, request, source)?;
    input.sync_all()?;
    input.seek(SeekFrom::Start(0))?;
    let mut output = pin::private_file(&scratch_root, &binding.nonce, "reply")?;
    cancel.check()?;
    let marker = pin::Marker::create(&scratch_root, &binding.nonce)?;
    let mut owner = RunGuard(Some(Resources {
        runtime,
        profile: None,
        marker,
        grants: Vec::new(),
        child: None,
    }));
    let result = (|| {
        let resources = owner.0.as_mut().ok_or(Error::Isolation)?;
        if started.elapsed() >= limits.wall_time {
            return Err(Error::Timeout);
        }
        resources.profile = Some(Profile::create(binding.nonce)?);
        let profile = resources.profile.as_ref().ok_or(Error::Isolation)?;
        resources
            .grants
            .try_reserve_exact(resources.runtime.files.len() + 1)
            .map_err(|_| Error::Limit)?;
        // All ACL targets are exact pinned files or the single dedicated runtime
        // root. Ancestor pins are read-only; no parent-directory ACL is changed.
        resources.grants.push(acl::Grant::prepare(
            resources.runtime.root_handle()?,
            profile.sid,
        )?);
        resources
            .grants
            .last_mut()
            .ok_or(Error::Isolation)?
            .apply()?;
        for file in &resources.runtime.files {
            resources
                .grants
                .push(acl::Grant::prepare(&file.handle, profile.sid)?);
            resources
                .grants
                .last_mut()
                .ok_or(Error::Isolation)?
                .apply()?;
        }
        cancel.check()?;
        if started.elapsed() >= limits.wall_time {
            return Err(Error::Timeout);
        }
        resources.child = Some(process::Child::launch(
            &resources.runtime,
            profile,
            &input,
            limits,
        )?);
        let child = resources.child.as_mut().ok_or(Error::Isolation)?;
        child.start(profile)?;
        let mut typed_metadata = None;
        let mut validate = |bytes: &[u8]| {
            let typed = ResultMetadata::parse_for(bytes, request)?;
            validate_metadata(bytes)?;
            typed_metadata = Some(typed);
            Ok(())
        };
        let (completion, stderr_bytes_disposed) = child.receive(
            &mut output,
            binding,
            request.limits.output_bytes,
            started,
            limits.wall_time,
            cancel,
            &mut validate,
            progress,
        )?;
        let metadata = typed_metadata.ok_or(Error::Integrity)?;
        if metadata.data_bytes() != completion.bytes {
            return Err(Error::Integrity);
        }
        output.sync_all()?;
        output.seek(SeekFrom::Start(0))?;
        cancel.check()?;
        Ok((completion, metadata, stderr_bytes_disposed))
    })();
    // Do this on success AND failure before returning any artifact or error.
    // RunGuard quarantines a failed join/restore/profile cleanup on unwind too.
    if owner.settle().is_err() {
        return Err(Error::CleanupPending);
    }
    drop(input);
    drop(scratch);
    let (completion, metadata, stderr_bytes_disposed) = result?;
    Ok(WorkerReply {
        completion,
        metadata,
        stderr_bytes_disposed,
        file: output,
    })
}
