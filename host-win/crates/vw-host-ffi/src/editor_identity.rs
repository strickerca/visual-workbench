//! Read-only selected-editor identity. No code is loaded from the target image.
//! A content digest and fixed PE version are facts, never proof of compatibility.
use super::*;

/// Read-only identity inspection for an already isolated native worker. This
/// ordinary Rust API grants no input/profile authority and invokes no UIA.
/// Caller retains its process, permit and IO owner through actual retirement;
/// synchronous filesystem/OS calls remain cooperatively cancellable between calls.
pub fn inspect_editor_identity_sync(
    target: CaptureTarget,
    cancel: Arc<vw_capture::Cancellation>,
) -> std::result::Result<RemoteEditorImageIdentity, CaptureError> {
    cancel.check().map_err(failure)?;
    let target: capture::WindowTarget = target.into();
    target.validate(owner()).map_err(failure)?;
    #[cfg(windows)]
    {
        native::inspect(target, &cancel).map_err(failure)
    }
    #[cfg(not(windows))]
    {
        let _ = (target, cancel);
        Err(failure(capture::Error::Unsupported))
    }
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct RemoteEditorImageIdentity {
    pub target: CaptureTarget,
    pub executable_name: String,
    pub executable_blake3: String,
    pub executable_bytes: u64,
    pub file_version: Option<RemoteEditorFileVersion>,
    pub package_full_name: Option<String>,
    pub package_version: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RemoteEditorFileVersion {
    pub major: u16,
    pub minor: u16,
    pub build: u16,
    pub revision: u16,
}
#[uniffi::export]
impl CaptureService {
    /// Inspection borrows the already selected HWND identity; it does not grant
    /// control, focus a window, inject input, capture pixels or invoke a provider.
    pub async fn inspect_editor_identity(
        &self,
        target: CaptureTarget,
        operation: Arc<CaptureOperation>,
    ) -> Result<RemoteEditorImageIdentity> {
        owned(operation.cancel.clone(), move || {
            let target: capture::WindowTarget = target.into();
            target.validate(owner())?;
            #[cfg(windows)]
            {
                native::inspect(target, &operation.cancel)
            }
            #[cfg(not(windows))]
            {
                let _ = (target, operation);
                Err(capture::Error::Unsupported)
            }
        })
        .await
    }
}

#[cfg(any(windows, test))]
#[path = "editor_identity/pe.rs"]
mod pe;

/// Shared data-only parser entry for the installed-package reader. This stays
/// crate-private and reads the caller's retained descriptor under its own check.
#[cfg(windows)]
pub(crate) fn read_file_version<R: std::io::Read + std::io::Seek>(
    file: &mut R,
    size: u64,
    check: &impl Fn() -> capture::Result<()>,
) -> capture::Result<Option<RemoteEditorFileVersion>> {
    pe::file_version(file, size, check)
}
#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        fs::File,
        io::{Read, Seek, SeekFrom},
        os::windows::{
            ffi::OsStringExt,
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        },
        path::{Path, PathBuf},
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::*,
        Storage::{FileSystem::*, Packaging::Appx::*},
        System::{Threading::*, WindowsProgramming::DRIVE_FIXED},
    };
    struct Process(HANDLE);
    impl Drop for Process {
        fn drop(&mut self) {
            /* SAFETY: unique successful OpenProcess handle. */
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    fn exact_file_path(file: &File, expected: &Path, directory: bool) -> capture::Result<()> {
        let metadata = file.metadata().map_err(|_| capture::Error::Platform)?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || metadata.is_dir() != directory
            || (!directory && !metadata.is_file())
        {
            return Err(capture::Error::Invalid);
        }
        let mut units = vec![0u16; 32768];
        // SAFETY: borrowed live file handle and initialized bounded UTF-16 output.
        let count = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                units.as_mut_ptr(),
                units.len() as u32,
                0,
            )
        };
        if count == 0 || count as usize >= units.len() {
            return Err(capture::Error::Platform);
        }
        let text =
            String::from_utf16(&units[..count as usize]).map_err(|_| capture::Error::Invalid)?;
        let text = text
            .strip_prefix("\\\\?\\")
            .ok_or(capture::Error::Invalid)?;
        if text.starts_with("UNC\\") || Path::new(text) != expected {
            return Err(capture::Error::Invalid);
        }
        Ok(())
    }
    fn pin_ancestors(
        path: &Path,
        check: &impl Fn() -> capture::Result<()>,
    ) -> capture::Result<Vec<File>> {
        if path.components().any(|c| {
            matches!(
                c,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        }) {
            return Err(capture::Error::Invalid);
        }
        let parent = path.parent().ok_or(capture::Error::Invalid)?;
        let ancestors = parent.ancestors().collect::<Vec<_>>();
        if ancestors.len() > 64 {
            return Err(capture::Error::Limit);
        }
        let mut leases = Vec::with_capacity(ancestors.len());
        for directory in ancestors.into_iter().rev() {
            check()?;
            // LIST_DIRECTORY participates in sharing checks; deny delete so
            // each verified ancestor cannot be replaced/renamed before the
            // next component is opened. OPEN_REPARSE_POINT opens a racing
            // junction itself for refusal instead of following it.
            let lease = File::options()
                .read(true)
                .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(directory)
                .map_err(|_| capture::Error::Platform)?;
            exact_file_path(&lease, directory, true)?;
            leases.push(lease);
        }
        Ok(leases)
    }
    fn created(process: HANDLE) -> capture::Result<u64> {
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut c = zero;
        let mut e = zero;
        let mut k = zero;
        let mut u = zero;
        // SAFETY: borrowed live process; all four outputs are initialized structs.
        if unsafe { GetProcessTimes(process, &mut c, &mut e, &mut k, &mut u) } == 0 {
            return Err(capture::Error::Platform);
        }
        Ok(u64::from(c.dwLowDateTime) | (u64::from(c.dwHighDateTime) << 32))
    }
    fn package(process: HANDLE) -> capture::Result<Option<String>> {
        let mut count = 0u32;
        // SAFETY: length-only read of this retained process; no buffer supplied.
        let status = unsafe { GetPackageFullName(process, &mut count, std::ptr::null_mut()) };
        if status == APPMODEL_ERROR_NO_PACKAGE {
            return Ok(None);
        }
        if status != ERROR_INSUFFICIENT_BUFFER || !(2..=1024).contains(&count) {
            return Err(capture::Error::Limit);
        }
        let mut units = vec![0u16; count as usize];
        // SAFETY: initialized count-unit buffer, bounded above, retained process.
        if unsafe { GetPackageFullName(process, &mut count, units.as_mut_ptr()) } != ERROR_SUCCESS {
            return Err(capture::Error::Platform);
        }
        if count as usize > units.len() || count < 2 || units[count as usize - 1] != 0 {
            return Err(capture::Error::Invalid);
        }
        let text = String::from_utf16(&units[..count as usize - 1])
            .map_err(|_| capture::Error::Invalid)?;
        if text.len() > 2048 || text.contains('\0') {
            return Err(capture::Error::Limit);
        }
        Ok(Some(text))
    }
    pub(super) fn inspect(
        target: capture::WindowTarget,
        cancel: &capture::Cancellation,
    ) -> capture::Result<RemoteEditorImageIdentity> {
        let start = Instant::now();
        let check = || {
            cancel.check()?;
            if start.elapsed() > Duration::from_secs(5) {
                Err(capture::Error::Timeout)
            } else {
                Ok(())
            }
        };
        check()?;
        target.unchanged(
            &capture::windows::fixed_target(target.window, owner())?,
            owner(),
        )?;
        // SAFETY: query-only access to owner-selected PID, validated above.
        let process = Process(unsafe {
            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, target.process_id)
        });
        if process.0.is_null() {
            return Err(capture::Error::Platform);
        }
        if created(process.0)? != target.process_created {
            return Err(capture::Error::Stale);
        }
        let mut path = vec![0u16; 32768];
        let mut count = path.len() as u32;
        // SAFETY: initialized bounded UTF-16 buffer, retained query-only process.
        if unsafe { QueryFullProcessImageNameW(process.0, 0, path.as_mut_ptr(), &mut count) } == 0
            || count == 0
            || count as usize >= path.len()
        {
            return Err(capture::Error::Platform);
        }
        path.truncate(count as usize);
        if path.contains(&0) {
            return Err(capture::Error::Invalid);
        }
        let path = PathBuf::from(std::ffi::OsString::from_wide(&path));
        if !path.is_absolute() {
            return Err(capture::Error::Invalid);
        }
        // Initial admission is an ordinary local drive image. UNC/device paths
        // would expose this data reader to a remote provider's unbounded I/O.
        let drive = match path.components().next() {
            Some(std::path::Component::Prefix(p)) => match p.kind() {
                std::path::Prefix::Disk(d) | std::path::Prefix::VerbatimDisk(d) => d,
                _ => return Err(capture::Error::Unsupported),
            },
            _ => return Err(capture::Error::Invalid),
        };
        let root = [u16::from(drive), u16::from(b':'), u16::from(b'\\'), 0];
        // SAFETY: initialized, NUL-terminated local drive root.
        if unsafe { GetDriveTypeW(root.as_ptr()) } != DRIVE_FIXED {
            return Err(capture::Error::Unsupported);
        }
        // Normalize only the supported extended DOS prefix, not any redirect.
        let path = if let Some(s) = path.to_str().and_then(|s| s.strip_prefix("\\\\?\\")) {
            PathBuf::from(s)
        } else {
            path
        };
        let _namespace_leases = pin_ancestors(&path, &check)?;
        let name = path
            .file_name()
            .and_then(|p| p.to_str())
            .filter(|p| p.len() <= 128)
            .ok_or(capture::Error::Invalid)?
            .to_owned();
        // Deny write/delete sharing while digest and PE data are read from THIS
        // handle. OPEN_REPARSE_POINT rejects a final-component redirect.
        let mut file = File::options()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)
            .map_err(|_| capture::Error::Platform)?;
        exact_file_path(&file, &path, false)?;
        let before = file.metadata().map_err(|_| capture::Error::Platform)?;
        if !before.is_file()
            || before.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || !(64..=256 * 1024 * 1024).contains(&before.len())
        {
            return Err(capture::Error::Limit);
        }
        let mut digest = blake3::Hasher::new();
        let mut buffer = [0u8; 65536];
        let mut total = 0u64;
        loop {
            check()?;
            let count = file
                .read(&mut buffer)
                .map_err(|_| capture::Error::Platform)?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(count as u64)
                .ok_or(capture::Error::Limit)?;
            if total > before.len() {
                return Err(capture::Error::Stale);
            }
            digest.update(&buffer[..count]);
        }
        if total != before.len() {
            return Err(capture::Error::Stale);
        }
        file.seek(SeekFrom::Start(0))
            .map_err(|_| capture::Error::Platform)?;
        let version = pe::file_version(&mut file, total, &check)?;
        let package_full_name = package(process.0)?;
        let package_version = package_full_name
            .as_deref()
            .map(package_version)
            .transpose()?;
        check()?;
        if file.metadata().map_err(|_| capture::Error::Platform)?.len() != before.len()
            || created(process.0)? != target.process_created
        {
            return Err(capture::Error::Stale);
        }
        target.unchanged(
            &capture::windows::fixed_target(target.window, owner())?,
            owner(),
        )?;
        Ok(RemoteEditorImageIdentity {
            target: target.into(),
            executable_name: name,
            executable_blake3: digest.finalize().to_hex().to_string(),
            executable_bytes: total,
            file_version: version,
            package_full_name,
            package_version,
        })
    }
    #[cfg(test)]
    mod namespace_tests {
        use super::*;
        type FixtureResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;
        fn root(temp: &tempfile::TempDir) -> FixtureResult<PathBuf> {
            let path = temp.path().canonicalize()?;
            let text = path.to_str().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "fixture path is not UTF-8")
            })?;
            Ok(PathBuf::from(text.strip_prefix("\\\\?\\").unwrap_or(text)))
        }
        #[test]
        fn held_parent_lease_refuses_ancestor_rename() -> FixtureResult<()> {
            let temp = tempfile::tempdir()?;
            let root = root(&temp)?;
            let directory = root.join("editor");
            std::fs::create_dir(&directory)?;
            let image = directory.join("image.exe");
            std::fs::write(&image, b"fixture")?;
            let leases = pin_ancestors(&image, &|| Ok(()))?;
            assert!(std::fs::rename(&directory, root.join("moved")).is_err());
            drop(leases);
            std::fs::rename(&directory, root.join("moved"))?;
            Ok(())
        }
        #[test]
        fn image_lease_refuses_replacement_write_and_hardlink_write() -> FixtureResult<()> {
            let temp = tempfile::tempdir()?;
            let root = root(&temp)?;
            let image = root.join("image.exe");
            std::fs::write(&image, b"fixture")?;
            let alias = root.join("alias.exe");
            std::fs::hard_link(&image, &alias)?;
            let leases = pin_ancestors(&image, &|| Ok(()))?;
            let file = File::options()
                .read(true)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(&image)?;
            exact_file_path(&file, &image, false)?;
            assert!(File::options().write(true).open(&image).is_err());
            assert!(std::fs::rename(&image, root.join("replaced.exe")).is_err());
            assert!(File::options().write(true).open(&alias).is_err());
            drop(file);
            drop(leases);
            File::options().write(true).open(&image)?;
            Ok(())
        }
        #[test]
        fn native_final_handle_refuses_a_different_expected_namespace() -> FixtureResult<()> {
            let temp = tempfile::tempdir()?;
            let root = root(&temp)?;
            let image = root.join("image.exe");
            std::fs::write(&image, b"fixture")?;
            let file = File::open(&image)?;
            assert!(exact_file_path(&file, &root.join("other.exe"), false).is_err());
            Ok(())
        }
    }
}

#[cfg(any(windows, test))]
fn package_version(full: &str) -> capture::Result<String> {
    // PackageFullName is Name_Version_Architecture_ResourceId_PublisherId.
    // Work from the right so no guessed filename/version becomes authority.
    let fields = full.rsplitn(5, '_').collect::<Vec<_>>();
    if fields.len() != 5 || fields[0].is_empty() || fields[2].is_empty() || fields[4].is_empty() {
        return Err(capture::Error::Invalid);
    }
    let version = fields[3];
    let parts = version.split('.').collect::<Vec<_>>();
    if parts.len() != 4
        || parts.iter().any(|p| {
            p.is_empty()
                || p.len() > 5
                || !p.bytes().all(|b| b.is_ascii_digit())
                || p.parse::<u16>().is_err()
        })
    {
        return Err(capture::Error::Invalid);
    }
    Ok(version.to_owned())
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    #[test]
    fn package_version_is_separate_from_executable_version() -> capture::Result<()> {
        assert_eq!(
            package_version("Microsoft.Paint_11.2605.81.0_x64__publisher")?,
            "11.2605.81.0"
        );
        Ok(())
    }
    #[test]
    fn malformed_or_overflow_package_versions_refuse() {
        for full in [
            "Paint_11.2_x64__pub",
            "Paint_11.2.3.65536_x64__pub",
            "Paint_11.2.x.4_x64__pub",
            "Paint_11.2.3.4__pub",
            "_11.2.3.4_x64__pub",
        ] {
            assert!(package_version(full).is_err());
        }
    }
}

#[cfg(test)]
mod sync_contract_tests {
    use super::*;
    fn absent_target() -> CaptureTarget {
        CaptureTarget {
            window: 0,
            process_id: 0,
            process_created: 0,
            client_x: 0,
            client_y: 0,
            client_width: 0,
            client_height: 0,
            frame_x: 0,
            frame_y: 0,
            frame_width: 0,
            frame_height: 0,
            dpi: 0,
            observed_ns: 0,
        }
    }
    #[test]
    fn cancellation_refuses_before_target_or_platform_inspection() {
        let cancel = Arc::new(capture::Cancellation::default());
        cancel.cancel();
        assert!(
            matches!(inspect_editor_identity_sync(absent_target(), cancel),
            Err(CaptureError::Refused { kind }) if kind == "Cancelled")
        );
    }
    #[test]
    fn absent_target_never_reaches_native_inspection() {
        assert!(matches!(inspect_editor_identity_sync(absent_target(),
            Arc::new(capture::Cancellation::default())),
            Err(CaptureError::Refused { kind }) if kind == "Invalid"));
    }
}
