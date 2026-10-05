//! Query-only retained process and strong final-image descriptor. Namespace
//! trust is OS installed-package policy, not ordinary ancestor-handle equivalence.
use super::*;
use crate::{CaptureTarget, RemoteEditorImageIdentity};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    mem::size_of,
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use windows::{
    Management::Deployment::PackageManager,
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
    core::HSTRING,
};
use windows_sys::Win32::{
    Foundation::*,
    Security::*,
    Storage::{FileSystem::*, Packaging::Appx::*},
    System::{Threading::*, WindowsProgramming::DRIVE_FIXED},
};
const MAX_IMAGE: u64 = 256 * 1024 * 1024;
const MAX_OPERATION: Duration = Duration::from_secs(5);
fn bracket<T>(
    stage: &'static str,
    check: impl Fn() -> Result<()>,
    call: impl FnOnce() -> windows::core::Result<T>,
) -> Result<T> {
    check()?;
    let result = call().map_err(|error| PaintPackageError::Platform {
        stage,
        code: error.code().0,
    });
    check()?;
    result
}
struct Budget<'a> {
    start: Instant,
    limit: Duration,
    cancel: &'a capture::Cancellation,
}
impl<'a> Budget<'a> {
    fn new(cancel: &'a capture::Cancellation, limit: Duration) -> Result<Self> {
        if limit.is_zero() || limit > MAX_OPERATION {
            return Err(PaintPackageError::Limit);
        }
        let value = Self {
            start: Instant::now(),
            limit,
            cancel,
        };
        value.check()?;
        Ok(value)
    }
    fn check(&self) -> Result<()> {
        self.cancel.check().map_err(capture_error)?;
        if self.start.elapsed() >= self.limit {
            return Err(PaintPackageError::Timeout);
        }
        Ok(())
    }
    fn call<T>(
        &self,
        stage: &'static str,
        call: impl FnOnce() -> windows::core::Result<T>,
    ) -> Result<T> {
        bracket(stage, || self.check(), call)
    }
}
struct Apartment(std::marker::PhantomData<std::rc::Rc<()>>);
impl Apartment {
    fn enter(budget: &Budget<'_>) -> Result<Self> {
        // SAFETY: paired WinRT initialization on this query worker thread;
        // existing STA/unknown initialization refuses instead of being changed.
        budget.check()?;
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(|error| {
            PaintPackageError::Platform {
                stage: "winrt_initialize",
                code: error.code().0,
            }
        })?;
        // Own the successful initialization before a post-call deadline can
        // fail. A late return drops this paired guard instead of leaking it.
        let apartment = Self(std::marker::PhantomData);
        budget.check()?;
        Ok(apartment)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: successful paired RoInitialize on the same !Send thread guard.
        unsafe { RoUninitialize() };
    }
}
struct Handle(HANDLE);
// SAFETY: uniquely retained query-only kernel handle; query calls are thread-safe.
// No borrowed buffers, COM apartment or mutation rights accompany this owner.
unsafe impl Send for Handle {}
// SAFETY: shared operations only query this retained HANDLE; Drop needs ownership.
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: uniquely owned successful OpenProcess/OpenProcessToken handle.
        unsafe { CloseHandle(self.0) };
    }
}
fn os_error(stage: &'static str) -> PaintPackageError {
    // SAFETY: read thread-local error immediately after failed synchronous API.
    PaintPackageError::Platform {
        stage,
        code: unsafe { GetLastError() } as i32,
    }
}
fn utf16(units: &[u16]) -> Result<String> {
    if units.len() > 32767 || units.contains(&0) {
        return Err(PaintPackageError::Limit);
    }
    let text = String::from_utf16(units).map_err(|_| PaintPackageError::Policy)?;
    if text.len() > 128 * 1024 || text.chars().any(char::is_control) {
        return Err(PaintPackageError::Limit);
    }
    Ok(text)
}
fn hstring(value: HSTRING) -> Result<String> {
    let units: &[u16] = &value;
    utf16(units)
}
fn sized_string(
    stage: &'static str,
    budget: &Budget<'_>,
    call: impl Fn(*mut u32, *mut u16) -> u32,
) -> Result<String> {
    budget.check()?;
    let mut count = 0;
    let status = call(&mut count, std::ptr::null_mut());
    budget.check()?;
    if status != ERROR_INSUFFICIENT_BUFFER {
        return Err(PaintPackageError::Platform {
            stage,
            code: status as i32,
        });
    }
    if !(2..=32768).contains(&count) {
        return Err(PaintPackageError::Limit);
    }
    let mut units = vec![0u16; count as usize];
    let status = call(&mut count, units.as_mut_ptr());
    budget.check()?;
    if status != ERROR_SUCCESS {
        return Err(PaintPackageError::Platform {
            stage,
            code: status as i32,
        });
    }
    if count < 2 || count as usize > units.len() || units[count as usize - 1] != 0 {
        return Err(PaintPackageError::Policy);
    }
    utf16(&units[..count as usize - 1])
}
fn process_image(process: HANDLE, budget: &Budget<'_>) -> Result<String> {
    budget.check()?;
    let mut units = vec![0u16; 32768];
    let mut count = units.len() as u32;
    // SAFETY: retained query process and initialized bounded output/length.
    if unsafe { QueryFullProcessImageNameW(process, 0, units.as_mut_ptr(), &mut count) } == 0 {
        return Err(os_error("process_image"));
    }
    budget.check()?;
    if count == 0 || count as usize >= units.len() {
        return Err(PaintPackageError::Limit);
    }
    utf16(&units[..count as usize])
}
fn process_created(process: HANDLE, budget: &Budget<'_>) -> Result<u64> {
    budget.check()?;
    let mut c = FILETIME::default();
    let mut e = c;
    let mut k = c;
    let mut u = c;
    // SAFETY: retained query process and four initialized FILETIME outputs.
    if unsafe { GetProcessTimes(process, &mut c, &mut e, &mut k, &mut u) } == 0 {
        return Err(os_error("process_times"));
    }
    budget.check()?;
    if e.dwLowDateTime != 0 || e.dwHighDateTime != 0 {
        return Err(PaintPackageError::TargetChanged);
    }
    Ok(u64::from(c.dwLowDateTime) | (u64::from(c.dwHighDateTime) << 32))
}
fn user_sid(process: HANDLE, budget: &Budget<'_>) -> Result<Vec<u8>> {
    budget.check()?;
    let mut token = std::ptr::null_mut();
    // SAFETY: borrowed live process; output initialized; TOKEN_QUERY only.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(os_error("process_token"));
    }
    let token = Handle(token);
    let mut count = 0;
    // SAFETY: size-only query; null output is intentional.
    let status =
        unsafe { GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut count) };
    if status != 0 || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
        return Err(os_error("token_user_size"));
    }
    if !(size_of::<TOKEN_USER>() as u32..=65536).contains(&count) {
        return Err(PaintPackageError::Limit);
    }
    let mut storage = vec![0usize; (count as usize).div_ceil(size_of::<usize>())];
    // SAFETY: aligned initialized storage at least count bytes, live token owner.
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            storage.as_mut_ptr().cast(),
            count,
            &mut count,
        )
    } == 0
    {
        return Err(os_error("token_user"));
    }
    budget.check()?;
    if count as usize > storage.len() * size_of::<usize>()
        || (count as usize) < size_of::<TOKEN_USER>()
    {
        return Err(PaintPackageError::Policy);
    }
    // SAFETY: successful API output has TOKEN_USER alignment and validated size.
    let sid = unsafe { (*storage.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let base = storage.as_ptr() as usize;
    let at = sid as usize;
    let offset = at.checked_sub(base).ok_or(PaintPackageError::Policy)?;
    if offset.checked_add(8).is_none_or(|end| end > count as usize) {
        return Err(PaintPackageError::Policy);
    }
    // SAFETY: bounded SID header inside initialized successful token output.
    let head = unsafe { std::slice::from_raw_parts(sid.cast::<u8>(), 8) };
    if head[0] != 1 || head[1] > 15 {
        return Err(PaintPackageError::Policy);
    }
    let length = 8 + 4 * usize::from(head[1]);
    if offset
        .checked_add(length)
        .is_none_or(|end| end > count as usize)
    {
        return Err(PaintPackageError::Policy);
    }
    // SAFETY: complete bounded SID lies inside token buffer; no external pointer.
    if unsafe { IsValidSid(sid) } == 0 || unsafe { GetLengthSid(sid) } as usize != length {
        return Err(PaintPackageError::Policy);
    }
    // SAFETY: same validated contiguous SID bytes; copied before token storage drops.
    Ok(unsafe { std::slice::from_raw_parts(sid.cast::<u8>(), length) }.to_vec())
}
fn same_user(process: HANDLE, budget: &Budget<'_>) -> Result<()> {
    budget.check()?;
    let mut thread_token = std::ptr::null_mut();
    // SAFETY: query-only current thread token; no impersonation changes.
    let has_token =
        unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut thread_token) };
    if has_token != 0 {
        let _token = Handle(thread_token);
        return Err(PaintPackageError::Policy);
    }
    // SAFETY: read the failed thread-token query's error immediately.
    if unsafe { GetLastError() } != ERROR_NO_TOKEN {
        return Err(os_error("thread_token"));
    }
    // SAFETY: borrowed current-process pseudo handle; user_sid never closes it.
    if user_sid(process, budget)? != user_sid(unsafe { GetCurrentProcess() }, budget)? {
        return Err(PaintPackageError::Policy);
    }
    Ok(())
}
fn snapshot(process: HANDLE, budget: &Budget<'_>) -> Result<PaintPackageEvidence> {
    same_user(process, budget)?;
    let _apartment = Apartment::enter(budget)?;
    // SAFETY: callbacks receive only the same retained process and initialized
    // bounded output provided by sized_string. APIs return Win32 status values.
    let full = unsafe {
        sized_string("package_full_name", budget, |count, out| {
            GetPackageFullName(process, count, out)
        })
    }?;
    let family = unsafe {
        sized_string("package_family", budget, |count, out| {
            GetPackageFamilyName(process, count, out)
        })
    }?;
    if full != PAINT_FULL_NAME || family != PAINT_FAMILY {
        return Err(PaintPackageError::Policy);
    }
    let full_units: Vec<u16> = full.encode_utf16().chain([0]).collect();
    let path = |kind, stage| unsafe {
        sized_string(stage, budget, |count, out| {
            GetPackagePathByFullName2(full_units.as_ptr(), kind, count, out)
        })
    };
    let original_path = path(PackagePathType_Install, "package_original_path")?;
    let effective_path = path(PackagePathType_Effective, "package_effective_path")?;
    let manager = budget.call("package_manager", PackageManager::new)?;
    // Empty SID requests only the calling user's registration. Same-user and
    // no-impersonation proof above binds this to the retained selected process.
    let package = budget.call("current_user_package", || {
        manager.FindPackageByUserSecurityIdPackageFullName(
            &HSTRING::new(),
            &HSTRING::from(full.as_str()),
        )
    })?;
    let id = budget.call("current_package_id", || package.Id())?;
    if hstring(budget.call("current_full_name", || id.FullName())?)? != full
        || hstring(budget.call("current_family", || id.FamilyName())?)? != family
    {
        return Err(PaintPackageError::TargetChanged);
    }
    let version = budget.call("package_version", || id.Version())?;
    // Reacquire Status on every snapshot; old PackageStatus is never authority.
    let status = budget.call("fresh_package_status", || package.Status())?;
    let status_ok = budget.call("package_status_ok", || status.VerifyIsOK())?;
    let flags = [
        budget.call("status_not_available", || status.NotAvailable())?,
        budget.call("status_package_offline", || status.PackageOffline())?,
        budget.call("status_data_offline", || status.DataOffline())?,
        budget.call("status_disabled", || status.Disabled())?,
        budget.call("status_remediation", || status.NeedsRemediation())?,
        budget.call("status_license", || status.LicenseIssue())?,
        budget.call("status_modified", || status.Modified())?,
        budget.call("status_tampered", || status.Tampered())?,
        budget.call("status_dependency", || status.DependencyIssue())?,
        budget.call("status_servicing", || status.Servicing())?,
        budget.call("status_deployment", || status.DeploymentInProgress())?,
        budget.call("status_partly_staged", || status.IsPartiallyStaged())?,
    ];
    let status_flags = flags
        .iter()
        .enumerate()
        .fold(0u32, |out, (bit, bad)| out | (u32::from(*bad) << bit));
    let result = PaintPackageEvidence {
        full_name: full,
        family,
        publisher: hstring(budget.call("package_publisher", || id.Publisher())?)?,
        version: format!(
            "{}.{}.{}.{}",
            version.Major, version.Minor, version.Build, version.Revision
        ),
        signature_kind: budget
            .call("package_signature", || package.SignatureKind())?
            .0,
        development: budget.call("package_development", || package.IsDevelopmentMode())?,
        framework_or_resource_or_bundle_or_optional: budget
            .call("package_framework", || package.IsFramework())?
            || budget.call("package_resource", || package.IsResourcePackage())?
            || budget.call("package_bundle", || package.IsBundle())?
            || budget.call("package_optional", || package.IsOptional())?,
        status_ok,
        status_flags,
        original_path,
        effective_path,
        winrt_installed_path: hstring(
            budget.call("winrt_installed_path", || package.InstalledPath())?,
        )?,
        winrt_effective_path: hstring(
            budget.call("winrt_effective_path", || package.EffectivePath())?,
        )?,
        mutable_path: hstring(budget.call("package_mutable_path", || package.MutablePath())?)?,
        machine_external_path: hstring(
            budget.call("package_machine_external", || package.MachineExternalPath())?,
        )?,
        user_external_path: hstring(
            budget.call("package_user_external", || package.UserExternalPath())?,
        )?,
        effective_external_path: hstring(budget.call("package_effective_external", || {
            package.EffectiveExternalPath()
        })?)?,
        process_image_path: process_image(process, budget)?,
    };
    result.validate()?;
    budget.check()?;
    Ok(result)
}
fn final_path(file: &File, flags: u32, budget: &Budget<'_>) -> Result<String> {
    budget.check()?;
    let mut units = vec![0u16; 32768];
    // SAFETY: retained strong final file and initialized bounded UTF-16 buffer.
    let count = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            units.as_mut_ptr(),
            units.len() as u32,
            flags,
        )
    };
    if count == 0 {
        return Err(os_error("final_image_path"));
    }
    budget.check()?;
    if count as usize >= units.len() {
        return Err(PaintPackageError::Limit);
    }
    utf16(&units[..count as usize])
}
fn local_final(file: &File, expected: &Path, budget: &Budget<'_>) -> Result<()> {
    let text = expected.to_str().ok_or(PaintPackageError::Policy)?;
    canonical_dos(text)?;
    budget.check()?;
    let metadata = file.metadata().map_err(|e| PaintPackageError::Platform {
        stage: "image_metadata",
        code: e.raw_os_error().unwrap_or(0),
    })?;
    // SAFETY: query only of the retained strong final executable descriptor.
    if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK
        || !metadata.is_file()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(PaintPackageError::Policy);
    }
    let root: Vec<u16> = text[..3].encode_utf16().chain([0]).collect();
    // SAFETY: validated drive root and initialized NUL-terminated buffer.
    if unsafe { GetDriveTypeW(root.as_ptr()) } != DRIVE_FIXED {
        return Err(PaintPackageError::Policy);
    }
    if final_path(file, 0, budget)? != format!("\\\\?\\{text}") {
        return Err(PaintPackageError::TargetChanged);
    }
    let mut volume = vec![0u16; 128];
    // SAFETY: validated local root and initialized bounded volume-name output.
    if unsafe {
        GetVolumeNameForVolumeMountPointW(root.as_ptr(), volume.as_mut_ptr(), volume.len() as u32)
    } == 0
    {
        return Err(os_error("root_volume_guid"));
    }
    let end = volume
        .iter()
        .position(|v| *v == 0)
        .ok_or(PaintPackageError::Limit)?;
    let volume = utf16(&volume[..end])?;
    if !volume.starts_with("\\\\?\\Volume{")
        || !volume.ends_with("}\\")
        || final_path(file, VOLUME_NAME_GUID, budget)? != format!("{volume}{}", &text[3..])
    {
        return Err(PaintPackageError::Policy);
    }
    budget.check()
}
fn selected(
    target: &capture::WindowTarget,
    owner: u32,
    process: HANDLE,
    budget: &Budget<'_>,
) -> Result<()> {
    budget.check()?;
    target.validate(owner).map_err(capture_error)?;
    target
        .unchanged(
            &capture::windows::fixed_target(target.window, owner).map_err(capture_error)?,
            owner,
        )
        .map_err(capture_error)?;
    if process_created(process, budget)? != target.process_created {
        return Err(PaintPackageError::TargetChanged);
    }
    budget.check()
}

/// Retained read-only ownership. Caller keeps this lease, its actual worker,
/// permit and Job/IO alive through cancellation and actual joined retirement.
/// Dropping a future is not permission to drop a still-running worker's lease.
pub struct PaintPackageLease {
    process: Handle,
    file: File,
    target: capture::WindowTarget,
    owner: u32,
    cancel: Arc<capture::Cancellation>,
    evidence: PaintPackageEvidence,
    identity: RemoteEditorImageIdentity,
}
impl PaintPackageLease {
    pub fn open(
        target: CaptureTarget,
        owner: u32,
        cancel: Arc<capture::Cancellation>,
    ) -> Result<Self> {
        Self::open_with_budget(target, owner, cancel, MAX_OPERATION)
    }
    /// Use the caller's remaining invocation budget and same cancellation owner.
    /// This is cooperative only; blocked calls remain under its actual Job owner.
    pub fn open_with_budget(
        target: CaptureTarget,
        owner: u32,
        cancel: Arc<capture::Cancellation>,
        remaining: Duration,
    ) -> Result<Self> {
        let target: capture::WindowTarget = target.into();
        let budget = Budget::new(&cancel, remaining)?;
        target.validate(owner).map_err(capture_error)?;
        budget.check()?;
        // SAFETY: explicitly selected validated PID, query-only access.
        let process =
            Handle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, target.process_id) });
        if process.0.is_null() {
            return Err(os_error("open_selected_process"));
        }
        selected(&target, owner, process.0, &budget)?;
        let evidence = snapshot(process.0, &budget)?;
        let path = PathBuf::from(&evidence.process_image_path);
        // OS-current trusted immutable-package namespace is already validated.
        // This is NOT ordinary ancestor lease equivalence or an access fallback.
        let mut file = File::options()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)
            .map_err(|e| PaintPackageError::Platform {
                stage: "open_final_image",
                code: e.raw_os_error().unwrap_or(0),
            })?;
        local_final(&file, &path, &budget)?;
        let size = file
            .metadata()
            .map_err(|e| PaintPackageError::Platform {
                stage: "image_size",
                code: e.raw_os_error().unwrap_or(0),
            })?
            .len();
        if !(64..=MAX_IMAGE).contains(&size) {
            return Err(PaintPackageError::Limit);
        }
        let mut digest = blake3::Hasher::new();
        let mut bytes = [0u8; 65536];
        let mut total = 0u64;
        loop {
            budget.check()?;
            let count = file
                .read(&mut bytes)
                .map_err(|e| PaintPackageError::Platform {
                    stage: "read_final_image",
                    code: e.raw_os_error().unwrap_or(0),
                })?;
            budget.check()?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(count as u64)
                .ok_or(PaintPackageError::Limit)?;
            if total > size {
                return Err(PaintPackageError::TargetChanged);
            }
            digest.update(&bytes[..count]);
        }
        if total != size {
            return Err(PaintPackageError::TargetChanged);
        }
        file.seek(SeekFrom::Start(0))
            .map_err(|e| PaintPackageError::Platform {
                stage: "image_seek",
                code: e.raw_os_error().unwrap_or(0),
            })?;
        // Same source-bound strict parser used by ordinary identity; reads this
        // descriptor only, never the version API by a racing path or loaded DLL.
        let version = crate::capture::editor_identity::read_file_version(&mut file, size, &|| {
            budget.check().map_err(|error| match error {
                PaintPackageError::Cancelled => capture::Error::Cancelled,
                PaintPackageError::Timeout => capture::Error::Timeout,
                _ => capture::Error::Platform,
            })
        })
        .map_err(capture_error)?;
        selected(&target, owner, process.0, &budget)?;
        evidence.unchanged(&snapshot(process.0, &budget)?)?;
        local_final(&file, &path, &budget)?;
        let identity = RemoteEditorImageIdentity {
            target: target.clone().into(),
            executable_name: "mspaint.exe".into(),
            executable_blake3: digest.finalize().to_hex().to_string(),
            executable_bytes: size,
            file_version: version,
            package_full_name: Some(evidence.full_name.clone()),
            package_version: Some(evidence.version.clone()),
        };
        budget.check()?;
        Ok(Self {
            process,
            file,
            target,
            owner,
            cancel,
            evidence,
            identity,
        })
    }
    pub fn identity(&self) -> &RemoteEditorImageIdentity {
        &self.identity
    }
    pub fn evidence(&self) -> &PaintPackageEvidence {
        &self.evidence
    }
    pub fn verify(&self, target: CaptureTarget, owner: u32) -> Result<()> {
        self.verify_with_budget(target, owner, MAX_OPERATION)
    }
    pub fn verify_with_budget(
        &self,
        target: CaptureTarget,
        owner: u32,
        remaining: Duration,
    ) -> Result<()> {
        let budget = Budget::new(&self.cancel, remaining)?;
        if owner != self.owner {
            return Err(PaintPackageError::TargetChanged);
        }
        let supplied: capture::WindowTarget = target.into();
        self.target
            .unchanged(&supplied, owner)
            .map_err(capture_error)?;
        selected(&self.target, owner, self.process.0, &budget)?;
        self.evidence
            .unchanged(&snapshot(self.process.0, &budget)?)?;
        let path = Path::new(&self.evidence.process_image_path);
        local_final(&self.file, path, &budget)?;
        if self
            .file
            .metadata()
            .map_err(|e| PaintPackageError::Platform {
                stage: "held_image_size",
                code: e.raw_os_error().unwrap_or(0),
            })?
            .len()
            != self.identity.executable_bytes
        {
            return Err(PaintPackageError::TargetChanged);
        }
        selected(&self.target, owner, self.process.0, &budget)?;
        budget.check()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type FixtureResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;
    #[test]
    fn strong_final_image_alone_refuses_write_hardlink_write_and_replacement() -> FixtureResult<()>
    {
        let temp = tempfile::tempdir()?;
        let image = temp.path().join("image.exe");
        let alias = temp.path().join("alias.exe");
        std::fs::write(&image, b"retained image fixture")?;
        std::fs::hard_link(&image, &alias)?;
        let lease = File::options()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&image)?;
        assert_eq!(
            File::options()
                .write(true)
                .open(&image)
                .err()
                .and_then(|e| e.raw_os_error()),
            Some(32)
        );
        assert_eq!(
            File::options()
                .write(true)
                .open(&alias)
                .err()
                .and_then(|e| e.raw_os_error()),
            Some(32)
        );
        assert_eq!(
            std::fs::rename(&image, temp.path().join("moved.exe"))
                .err()
                .and_then(|e| e.raw_os_error()),
            Some(32)
        );
        drop(lease);
        std::fs::rename(&image, temp.path().join("moved.exe"))?;
        std::fs::write(&alias, b"after actual lease drop")?;
        Ok(())
    }
    #[test]
    fn cancelled_or_expired_reader_never_enters_winrt_provider() {
        let cancel = capture::Cancellation::default();
        cancel.cancel();
        assert!(matches!(
            Budget::new(&cancel, MAX_OPERATION),
            Err(PaintPackageError::Cancelled)
        ));
        let cancel = capture::Cancellation::default();
        let budget = Budget {
            start: Instant::now(),
            limit: Duration::ZERO,
            cancel: &cancel,
        };
        let entered = std::cell::Cell::new(false);
        let result = budget.call("fixture", || {
            entered.set(true);
            Ok(())
        });
        assert_eq!(result, Err(PaintPackageError::Timeout));
        assert!(!entered.get());
    }
    #[test]
    fn completed_provider_after_deadline_cannot_refresh_budget() {
        let expired = std::cell::Cell::new(false);
        let calls = std::cell::Cell::new(0);
        let check = || {
            if expired.get() {
                Err(PaintPackageError::Timeout)
            } else {
                Ok(())
            }
        };
        let late = bracket("fixture", check, || {
            calls.set(calls.get() + 1);
            expired.set(true);
            Ok(17)
        });
        assert_eq!(late, Err(PaintPackageError::Timeout));
        let repeated = bracket("fixture", check, || {
            calls.set(calls.get() + 1);
            Ok(19)
        });
        assert_eq!(repeated, Err(PaintPackageError::Timeout));
        assert_eq!(calls.get(), 1);
    }
}
