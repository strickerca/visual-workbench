//! Current-user DPAPI app trust, separate from project storage. The directory has
//! an owner/SYSTEM-only DACL; ciphertext is atomically replaced under an OS file
//! lock, after file fsync and with MOVEFILE_WRITE_THROUGH. There is no plaintext
//! disk fallback and no CRYPTPROTECT_LOCAL_MACHINE flag.
use fs2::FileExt;
use std::{
    ffi::{OsStr, OsString},
    fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    mem::size_of,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
    ptr, thread,
    time::{Duration, Instant},
};
use vw_net::pairing::{
    DeviceIdentity, MAX_TRUST_STATE_BYTES, PairingError, QrPayload, Result, SecretBytes,
    TrustState, TrustStore,
};
use windows_sys::Win32::{
    Foundation::{ERROR_ALREADY_EXISTS, GetLastError, LocalFree},
    Security::{
        Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SE_FILE_OBJECT, SetSecurityInfo,
        },
        Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
        DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION,
        SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::{
        CreateDirectoryW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW, READ_CONTROL, WRITE_DAC,
    },
    System::Com::CoTaskMemFree,
    UI::Shell::{FOLDERID_LocalAppData, SHGetKnownFolderPath},
};
use zeroize::{Zeroize, Zeroizing};

const MAX_CIPHERTEXT: usize = MAX_TRUST_STATE_BYTES + 64 * 1024;
const ENTROPY: &[u8] = b"Visual Workbench/app-trust/current-user/1";
const STATE_FILE: &str = "trust-state.dpapi";

fn wide(text: &OsStr) -> Result<Vec<u16>> {
    let mut bytes: Vec<u16> = text.encode_wide().collect();
    if bytes.is_empty() || bytes.len() > 32760 || bytes.contains(&0) {
        return Err(PairingError::Storage);
    }
    bytes.push(0);
    Ok(bytes)
}
struct DpapiBuffer(CRYPT_INTEGER_BLOB);
impl Drop for DpapiBuffer {
    fn drop(&mut self) {
        if !self.0.pbData.is_null() {
            // SAFETY: DPAPI owns this allocation and reports its initialized
            // byte length. Clear it before freeing with its required allocator.
            unsafe {
                std::slice::from_raw_parts_mut(self.0.pbData, self.0.cbData as usize).zeroize();
                LocalFree(self.0.pbData.cast());
            }
        }
    }
}
fn crypt(bytes: &[u8], protect: bool, entropy: &[u8]) -> Result<SecretBytes> {
    if bytes.is_empty() || bytes.len() > MAX_CIPHERTEXT {
        return Err(PairingError::Storage);
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(bytes.len()).map_err(|_| PairingError::Storage)?,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let additional = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(entropy.len()).map_err(|_| PairingError::Storage)?,
        pbData: entropy.as_ptr().cast_mut(),
    };
    let mut output = DpapiBuffer(CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    });
    // SAFETY: input/entropy slices remain live and are read-only despite the
    // legacy mutable pointer ABI. DPAPI initializes output or returns failure.
    let success = unsafe {
        if protect {
            CryptProtectData(
                &input,
                ptr::null(),
                &additional,
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output.0,
            )
        } else {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                &additional,
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output.0,
            )
        }
    };
    if success == 0 || output.0.pbData.is_null() || output.0.cbData as usize > MAX_CIPHERTEXT {
        return Err(PairingError::Storage);
    }
    // SAFETY: successful DPAPI result supplies this initialized allocation;
    // DpapiBuffer retains and securely frees it after this bounded copy.
    let result =
        unsafe { std::slice::from_raw_parts(output.0.pbData, output.0.cbData as usize).to_vec() };
    Ok(SecretBytes::new(result))
}

fn local_app_data() -> Result<PathBuf> {
    let mut raw = ptr::null_mut();
    // SAFETY: the known-folder GUID and output slot are valid; null token selects
    // the current user. This avoids an environment-supplied project directory.
    let status =
        unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppData, 0, ptr::null_mut(), &mut raw) };
    if status < 0 || raw.is_null() {
        return Err(PairingError::Storage);
    }
    struct TaskString(*mut u16);
    impl Drop for TaskString {
        fn drop(&mut self) {
            /* SAFETY: shell allocates with CoTaskMemAlloc. */
            unsafe {
                CoTaskMemFree(self.0.cast());
            }
        }
    }
    let owned = TaskString(raw);
    let mut length = 0;
    // SAFETY: SHGetKnownFolderPath returns a NUL-terminated UTF-16 allocation.
    // The Windows path maximum bounds the scan of the documented result.
    while length < 32768 && unsafe { *owned.0.add(length) } != 0 {
        length += 1;
    }
    if length == 0 || length == 32768 {
        return Err(PairingError::Storage);
    }
    // SAFETY: length is the terminator position inside the shell-owned string.
    let path = PathBuf::from(OsString::from_wide(unsafe {
        std::slice::from_raw_parts(owned.0, length)
    }));
    fs::canonicalize(path).map_err(|_| PairingError::Storage)
}
fn private_directory(path: &Path) -> Result<File> {
    let name = wide(path.as_os_str())?;
    let sddl = wide(OsStr::new("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;OW)"))?;
    let mut descriptor = ptr::null_mut();
    // SAFETY: constant valid SDDL is NUL terminated; API allocates descriptor.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(PairingError::Storage);
    }
    struct Descriptor(*mut std::ffi::c_void);
    impl Drop for Descriptor {
        fn drop(&mut self) {
            /* SAFETY: conversion API uses LocalAlloc. */
            unsafe {
                LocalFree(self.0);
            }
        }
    }
    let descriptor = Descriptor(descriptor);
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    // SAFETY: path/descriptor remain valid for this synchronous call.
    let created = unsafe { CreateDirectoryW(name.as_ptr(), &attributes) };
    // SAFETY: read immediately after failed Win32 call on this thread.
    if created == 0 && unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
        return Err(PairingError::Storage);
    }
    // Holding a handle without FILE_SHARE_DELETE prevents swapping the directory
    // after validation. OPEN_REPARSE_POINT observes the junction itself.
    let directory = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | READ_CONTROL | WRITE_DAC)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| PairingError::Storage)?;
    let metadata = directory.metadata().map_err(|_| PairingError::Storage)?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(PairingError::Storage);
    }
    let mut dacl = ptr::null_mut();
    let mut present = 0;
    let mut defaulted = 0;
    // SAFETY: descriptor owns the valid ACL; all output slots are initialized.
    if unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut dacl, &mut defaulted) }
        == 0
        || present == 0
        || dacl.is_null()
    {
        return Err(PairingError::Storage);
    }
    // SAFETY: validated non-reparse directory handle remains live. Reapply only
    // this app directory's owner/SYSTEM DACL so pre-existing broad inheritance
    // cannot expose a QR transfer file. No machine-wide security policy changes.
    if unsafe {
        SetSecurityInfo(
            directory.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            dacl,
            ptr::null(),
        )
    } != 0
    {
        return Err(PairingError::Storage);
    }
    Ok(directory)
}

pub struct DpapiTrustStore {
    root: PathBuf,
    _directories: Vec<File>,
}
impl DpapiTrustStore {
    /// Fixed current-user app storage. The API accepts no project path.
    pub fn open_current_user() -> Result<Self> {
        Self::open_current_user_for(None)
    }
    pub fn open_current_user_for(device: Option<vw_model::DeviceId>) -> Result<Self> {
        let base = local_app_data()?;
        let app = base.join("Visual Workbench");
        let app_guard = private_directory(&app)?;
        let root = app.join("trust");
        let trust_guard = private_directory(&root)?;
        Self::initialize_for(root, vec![app_guard, trust_guard], device)
    }
    /// Explicit CLI-only QR transfer credential, inside the protected app
    /// directory. Never call from logging/export code. The file is removed when
    /// the returned owner is dropped; the receiving device must also remove its
    /// task-owned copy. Existing paths are never replaced.
    pub fn create_qr_transfer(&self, qr: &QrPayload, file_name: &str) -> Result<PrivateQrTransfer> {
        if !(1..=80).contains(&file_name.len())
            || !file_name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(PairingError::Invalid("QR transfer name"));
        }
        let path = self.root.join(format!("{file_name}.qr"));
        let guard = private_directory(&self.root)?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)
            .map_err(|_| PairingError::Storage)?;
        // Mark ownership only after create_new succeeds; errors never delete an
        // unrelated pre-existing path.
        let mut owner = PrivateQrTransfer {
            path,
            _directory: guard,
            file: Some(file),
        };
        let file = owner.file.as_mut().ok_or(PairingError::Storage)?;
        let bytes = qr.encode_for_qr()?;
        file.write_all(bytes.expose())
            .map_err(|_| PairingError::Storage)?;
        file.sync_all().map_err(|_| PairingError::Storage)?;
        Ok(owner)
    }
    #[cfg(test)]
    fn initialize(root: PathBuf, guards: Vec<File>) -> Result<Self> {
        Self::initialize_for(root, guards, None)
    }
    fn initialize_for(
        root: PathBuf,
        guards: Vec<File>,
        device: Option<vw_model::DeviceId>,
    ) -> Result<Self> {
        let store = Self {
            root,
            _directories: guards,
        };
        let _lock = store.lock()?;
        match store.read_state()? {
            Some(state) => {
                if device
                    .as_ref()
                    .is_some_and(|id| id != state.identity().device())
                {
                    return Err(PairingError::Authentication);
                }
            }
            None => store.write_state(&TrustState::new(match device {
                Some(value) => DeviceIdentity::generate_for(value)?,
                None => DeviceIdentity::generate()?,
            })?)?,
        }
        Ok(store)
    }
    fn lock(&self) -> Result<File> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.root.join("trust.lock"))
            .map_err(|_| PairingError::Storage)?;
        let metadata = file.metadata().map_err(|_| PairingError::Storage)?;
        if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(PairingError::Storage);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(file),
                Err(error)
                    if (error.kind() == std::io::ErrorKind::WouldBlock
                        || error.raw_os_error() == fs2::lock_contended_error().raw_os_error())
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(_) => return Err(PairingError::Storage),
            }
        }
    }
    fn read_state(&self) -> Result<Option<TrustState>> {
        let file = match OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.root.join(STATE_FILE))
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(PairingError::Storage),
        };
        let metadata = file.metadata().map_err(|_| PairingError::Storage)?;
        if !metadata.is_file()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || metadata.len() > MAX_CIPHERTEXT as u64
        {
            return Err(PairingError::Storage);
        }
        let mut bytes = Zeroizing::new(Vec::new());
        file.take(MAX_CIPHERTEXT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| PairingError::Storage)?;
        if bytes.len() > MAX_CIPHERTEXT {
            return Err(PairingError::Storage);
        }
        let plaintext = crypt(&bytes, false, ENTROPY)?;
        Ok(Some(TrustState::decode_unprotected(plaintext.expose())?))
    }
    fn write_state(&self, state: &TrustState) -> Result<()> {
        let plaintext = state.encode_for_protection()?;
        let ciphertext = crypt(plaintext.expose(), true, ENTROPY)?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(&self.root).map_err(|_| PairingError::Storage)?;
        temporary
            .write_all(ciphertext.expose())
            .map_err(|_| PairingError::Storage)?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| PairingError::Storage)?;
        let temporary = temporary.into_temp_path();
        let source = wide(temporary.as_os_str())?;
        let destination = wide(self.root.join(STATE_FILE).as_os_str())?;
        // SAFETY: same validated/held directory, NUL-terminated owned paths.
        // Only encrypted bytes are on disk. Write-through waits for the move.
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(PairingError::Storage);
        }
        Ok(())
    }
    #[cfg(test)]
    fn fixture(parent: &Path) -> Result<Self> {
        let root = parent.join("owned-trust");
        let guard = private_directory(&root)?;
        Self::initialize(root, vec![guard])
    }
}
impl TrustStore for DpapiTrustStore {
    fn load(&self) -> Result<TrustState> {
        let _lock = self.lock()?;
        self.read_state()?.ok_or(PairingError::Storage)
    }
    fn compare_exchange(&self, expected: u64, replacement: &TrustState) -> Result<()> {
        let _lock = self.lock()?;
        let state = self.read_state()?.ok_or(PairingError::Storage)?;
        if state.revision() != expected {
            return Err(PairingError::Conflict);
        }
        let previous_identity = state.identity();
        let next_identity = replacement.identity();
        if replacement.revision() != expected.checked_add(1).ok_or(PairingError::Capacity)?
            || previous_identity.device() != next_identity.device()
            || previous_identity.certificate() != next_identity.certificate()
        {
            return Err(PairingError::Storage);
        }
        self.write_state(replacement)
    }
}
impl fmt::Debug for DpapiTrustStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DpapiTrustStore([REDACTED])")
    }
}

pub struct PrivateQrTransfer {
    path: PathBuf,
    _directory: File,
    file: Option<File>,
}
impl PrivateQrTransfer {
    /// Explicit transfer access only. The returned path may identify the user;
    /// do not include it in retained logs/evidence.
    pub fn path_for_transfer(&self) -> &Path {
        &self.path
    }
}
impl Drop for PrivateQrTransfer {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            drop(file);
            let _ = fs::remove_file(&self.path);
        }
    }
}
impl fmt::Debug for PrivateQrTransfer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PrivateQrTransfer([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use vw_net::pairing::PairingManager;
    #[test]
    fn dpapi_rejects_corruption_and_wrong_application_entropy() -> Result<()> {
        let plaintext = b"synthetic-app-secret-only";
        let encrypted = crypt(plaintext, true, ENTROPY)?;
        assert_ne!(encrypted.expose(), plaintext);
        assert_eq!(
            crypt(encrypted.expose(), false, ENTROPY)?.expose(),
            plaintext
        );
        assert!(crypt(encrypted.expose(), false, b"different-app").is_err());
        let mut corrupt = encrypted.expose().to_vec();
        corrupt[0] ^= 1;
        assert!(crypt(&corrupt, false, ENTROPY).is_err());
        Ok(())
    }
    #[test]
    fn trust_reopens_same_identity_and_never_writes_plaintext() -> Result<()> {
        let fixture = tempfile::tempdir().map_err(|_| PairingError::Storage)?;
        let store = Arc::new(DpapiTrustStore::fixture(fixture.path())?);
        let manager = PairingManager::new(store.clone())?;
        let identity = manager.identity()?;
        let code = manager.issue_code()?;
        let on_disk = fs::read(store.root.join(STATE_FILE)).map_err(|_| PairingError::Storage)?;
        let digits = code.digits_for_display()?.as_bytes();
        assert!(!on_disk.windows(digits.len()).any(|bytes| bytes == digits));
        assert!(
            !on_disk
                .windows(identity.certificate().len())
                .any(|bytes| bytes == identity.certificate())
        );
        drop(manager);
        drop(store);
        let reopened = DpapiTrustStore::fixture(fixture.path())?;
        assert_eq!(reopened.load()?.identity().device(), identity.device());
        Ok(())
    }
}
