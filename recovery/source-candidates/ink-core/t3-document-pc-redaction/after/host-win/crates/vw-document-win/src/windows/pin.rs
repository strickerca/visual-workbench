use super::{Handle, last_error, wide};
use crate::{ArtifactEntry, Error, Result, RuntimeManifest};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, ffi::OsStr, fs::File, io::{Read, Write}, mem::{size_of, size_of_val}, os::windows::io::{AsRawHandle, FromRawHandle}, path::{Path, PathBuf}, ptr};
use windows_sys::Win32::{
    Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, ERROR_FILE_EXISTS, ERROR_ALREADY_EXISTS, LocalFree},
    Security::{Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1}, SECURITY_ATTRIBUTES},
    Storage::FileSystem::{CreateFileW, GetFileInformationByHandleEx, GetFinalPathNameByHandleW, SetFileInformationByHandle,
        CREATE_NEW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
        FILE_ATTRIBUTE_TEMPORARY, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_DELETE_ON_CLOSE, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FILE_STANDARD_INFO, FILE_DISPOSITION_INFO, FileAttributeTagInfo, FileDispositionInfo, FileStandardInfo, OPEN_EXISTING, READ_CONTROL, WRITE_DAC, DELETE},
    System::Threading::GetCurrentProcess,
};

pub(super) fn absolute(path: &Path) -> Result<PathBuf> {
    let text = path.to_str().ok_or(Error::Invalid)?.replace('/', "\\");
    let bytes = text.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || &bytes[1..3] != b":\\" || text.len() > 30_000
        || text.chars().any(|c| c.is_control()) || text[3..].contains(':')
    { return Err(Error::Invalid); }
    if text.len() > 3 && text[3..].split('\\').any(|p| p.is_empty() || p == "." || p == ".." || p.ends_with('.') || p.ends_with(' ')) { return Err(Error::Invalid); }
    Ok(PathBuf::from(text))
}

pub(super) struct DirectoryChain { pub(super) handles: Vec<Handle>, pub(super) path: PathBuf }
impl DirectoryChain {
    pub(super) fn open(path: &Path) -> Result<Self> { Self::open_with_root_access(path, 0) }
    fn open_with_root_access(path: &Path, extra: u32) -> Result<Self> {
        let path = absolute(path)?;
        let text = path.to_str().ok_or(Error::Invalid)?;
        let mut names = vec![PathBuf::from(&text[..3])];
        let mut current = names[0].clone();
        if text.len() > 3 { for part in text[3..].split('\\') { current.push(part); names.push(current.clone()); } }
        if names.len() > 128 { return Err(Error::Limit); }
        let mut handles = Vec::new(); handles.try_reserve_exact(names.len()).map_err(|_| Error::Limit)?;
        for (index, name) in names.iter().enumerate() {
            // FILE_LIST_DIRECTORY (0x1) is intentional: READ_ATTRIBUTES alone
            // does not prevent Windows directory rename despite deny-delete.
            handles.push(pin(name, true, if index + 1 == names.len() { extra } else { 0 }, None)?);
        }
        Ok(Self { handles, path })
    }
}

pub(super) struct PinnedFile { pub(super) handle: Handle }
pub struct PinnedRuntime {
    chain: DirectoryChain,
    pub(super) files: Vec<PinnedFile>,
    pub(super) executable: PathBuf,
    pc_redaction_available: bool,
}
impl PinnedRuntime {
    /// The root must be a dedicated already-extracted runtime with exactly the
    /// trusted manifest files. Hashes are checked through retained deny-write /
    /// deny-delete handles; every physical ancestor remains deny-delete pinned.
    pub fn pin(root: &Path, manifest: &RuntimeManifest) -> Result<Self> {
        manifest.validate()?;
        let chain = DirectoryChain::open_with_root_access(root, READ_CONTROL | WRITE_DAC)?;
        let expected: BTreeSet<&str> = manifest.entries.iter().map(|row| row.file_name.as_str()).collect();
        let mut seen = BTreeSet::new();
        for entry in std::fs::read_dir(&chain.path)? {
            let entry = entry?;
            let name = entry.file_name().into_string().map_err(|_| Error::Integrity)?;
            if seen.len() >= expected.len() || !expected.contains(name.as_str()) || !seen.insert(name) { return Err(Error::Integrity); }
        }
        if seen.len() != expected.len() { return Err(Error::Integrity); }
        let mut files = Vec::new(); files.try_reserve_exact(manifest.entries.len()).map_err(|_| Error::Limit)?;
        for entry in &manifest.entries {
            let handle = pin(&chain.path.join(&entry.file_name), false, READ_CONTROL | WRITE_DAC, Some(entry))?;
            files.push(PinnedFile { handle });
        }
        let executable = chain.path.join("vw-document-worker.exe");
        let pc_redaction_available = expected.contains("qpdf30.dll");
        Ok(Self { chain, files, executable, pc_redaction_available })
    }
    pub(super) fn root_handle(&self) -> Result<&Handle> { self.chain.handles.last().ok_or(Error::Integrity) }
    pub(super) fn root(&self) -> &Path { &self.chain.path }
    /// True only when qpdf30.dll is among the source-bound retained file pins.
    pub fn pc_redaction_available(&self) -> bool { self.pc_redaction_available }
    /// Stable private sibling shared by all versions/instances of this runtime
    /// cache. The application creates it explicitly before invoking run.
    pub fn private_workspace_path(&self) -> Result<PathBuf> {
        Ok(self.root().parent().ok_or(Error::Invalid)?.join("document-worker-private"))
    }
    /// Includes a crash fence from a previous process. Unknown existing marker
    /// bytes are conservatively pending; this method never deletes or adopts it.
    pub fn workspace_cleanup_pending(&self) -> Result<bool> {
        let path = self.private_workspace_path()?;
        let _chain = DirectoryChain::open(&path)?;
        match std::fs::symlink_metadata(path.join("vw-document-active-v1.json")) {
            Ok(_) => Ok(true), Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false), Err(error) => Err(error.into()),
        }
    }
}

fn pin(path: &Path, directory: bool, extra: u32, artifact: Option<&ArtifactEntry>) -> Result<Handle> {
    let path = absolute(path)?; let encoded = wide(path.as_os_str())?;
    let desired = if directory { FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | extra } else { FILE_GENERIC_READ | extra };
    let share = if directory { FILE_SHARE_READ | FILE_SHARE_WRITE } else { FILE_SHARE_READ };
    // SAFETY: terminated absolute path, no creation, open-reparse-point and a
    // deny-delete handle; content files also deny writes while hashed/loaded.
    let handle = Handle::checked(unsafe { CreateFileW(encoded.as_ptr(), desired, share, ptr::null(), OPEN_EXISTING,
        FILE_FLAG_OPEN_REPARSE_POINT | if directory { FILE_FLAG_BACKUP_SEMANTICS } else { 0 }, ptr::null_mut()) })?;
    let mut tag = FILE_ATTRIBUTE_TAG_INFO::default();
    let mut info = FILE_STANDARD_INFO::default();
    // SAFETY: correctly sized writable structures and retained file handle.
    if unsafe { GetFileInformationByHandleEx(handle.0, FileAttributeTagInfo, (&raw mut tag).cast(), size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32) } == 0
        || unsafe { GetFileInformationByHandleEx(handle.0, FileStandardInfo, (&raw mut info).cast(), size_of::<FILE_STANDARD_INFO>() as u32) } == 0
    { return Err(last_error()); }
    if tag.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 || (tag.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) != directory
        || info.DeletePending != 0 || (!directory && info.NumberOfLinks != 1)
    { return Err(Error::Integrity); }
    let mut final_path = vec![0_u16; 32_768];
    // SAFETY: bounded writable buffer and default normalized DOS naming.
    let count = unsafe { GetFinalPathNameByHandleW(handle.0, final_path.as_mut_ptr(), final_path.len() as u32, 0) };
    if count == 0 || count as usize >= final_path.len() { return Err(Error::Integrity); }
    let actual = String::from_utf16(&final_path[..count as usize]).map_err(|_| Error::Integrity)?;
    let actual = actual.strip_prefix("\\\\?\\").ok_or(Error::Integrity)?;
    if !actual.eq_ignore_ascii_case(path.to_str().ok_or(Error::Integrity)?) { return Err(Error::Integrity); }
    if let Some(entry) = artifact {
        if info.EndOfFile < 0 || info.EndOfFile as u64 != entry.bytes { return Err(Error::Integrity); }
        let mut file = duplicate_file(&handle)?;
        let mut hasher = Sha256::new(); let mut buffer = [0_u8; 65_536]; let mut total = 0_u64;
        loop { let count = file.read(&mut buffer)?; if count == 0 { break; }
            total = total.checked_add(count as u64).ok_or(Error::Limit)?;
            if total > entry.bytes { return Err(Error::Integrity); } hasher.update(&buffer[..count]);
        }
        if total != entry.bytes || hasher.finalize().as_slice() != entry.sha256 { return Err(Error::Integrity); }
    }
    Ok(handle)
}
fn duplicate_file(handle: &Handle) -> Result<File> {
    let mut copied = ptr::null_mut();
    // SAFETY: duplicate one owned handle in this process, not inheritable.
    if unsafe { DuplicateHandle(GetCurrentProcess(), handle.0, GetCurrentProcess(), &mut copied, 0, 0, DUPLICATE_SAME_ACCESS) } == 0 { return Err(last_error()); }
    // SAFETY: transfers this newly created duplicate, not the retained pin.
    Ok(unsafe { File::from_raw_handle(copied) })
}

pub(super) fn private_file(root: &Path, id: &[u8; 16], suffix: &str) -> Result<File> {
    owned_file(root, &format!("vw-document-{}-{suffix}.tmp", super::hex(id)), true)
}
fn owned_file(root: &Path, name: &str, delete_on_close: bool) -> Result<File> {
    let root = absolute(root)?;
    let name = root.join(name);
    let path = wide(name.as_os_str())?;
    let sddl = wide(OsStr::new("D:P(A;;FA;;;SY)(A;;FA;;;OW)"))?;
    let mut descriptor = ptr::null_mut();
    // SAFETY: fixed owner/system-only protected descriptor for a NEW file.
    if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut descriptor, ptr::null_mut()) } == 0 { return Err(last_error()); }
    let security = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor, bInheritHandle: 0 };
    // SAFETY: CREATE_NEW refuses existing files/symlinks. Kernel delete-on-close
    // removes exactly this file object after all parent/child handles retire.
    let raw = unsafe { CreateFileW(path.as_ptr(), FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE, 0, &security,
        CREATE_NEW, FILE_ATTRIBUTE_TEMPORARY | if delete_on_close { FILE_FLAG_DELETE_ON_CLOSE } else { 0 } | FILE_FLAG_OPEN_REPARSE_POINT, ptr::null_mut()) };
    let checked = Handle::checked(raw);
    // SAFETY: descriptor was allocated by ConvertStringSecurityDescriptor.
    unsafe { LocalFree(descriptor); }
    let handle = checked?;
    let raw = handle.0; std::mem::forget(handle);
    // SAFETY: transfer exactly the newly created owned file handle.
    let file = unsafe { File::from_raw_handle(raw) };
    if file.as_raw_handle().is_null() { return Err(Error::Integrity); }
    Ok(file)
}

/// Persistent crash fence. A restarted process never guesses whether a prior
/// profile/ACL mutation completed and never deletes a profile by prefix scan.
pub(super) struct Marker { file: File, removed: bool }
impl Marker {
    pub(super) fn create(root: &Path, id: &[u8; 16]) -> Result<Self> {
        let mut file = match owned_file(root, "vw-document-active-v1.json", false) {
            Err(Error::Windows(code)) if code == ERROR_FILE_EXISTS || code == ERROR_ALREADY_EXISTS => return Err(Error::CleanupPending),
            other => other?,
        };
        let contents = format!("{{\"schema\":1,\"profile\":\"VisualWorkbench.Document.{}\",\"state\":\"cleanup-required-until-owned-handle-removal\"}}\n", super::hex(id));
        file.write_all(contents.as_bytes())?; file.sync_all()?;
        Ok(Self { file, removed: false })
    }
    pub(super) fn cleanup(&mut self) -> Result<()> {
        if self.removed { return Ok(()); }
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: 1 };
        // SAFETY: deletes this exact originally created file by retained handle,
        // after joined child, restored ACLs and owned profile deletion only.
        if unsafe { SetFileInformationByHandle(self.file.as_raw_handle(), FileDispositionInfo, (&raw const disposition).cast(), size_of_val(&disposition) as u32) } == 0 { return Err(Error::CleanupPending); }
        self.removed = true; Ok(())
    }
}
