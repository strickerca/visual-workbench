//! Local root policy and confirmation boundary for the private Android tunnel.
//!
//! Prepared runtime bytes are not runtime/license admission. This build has no
//! approved catalog, so availability is false and open refuses before file,
//! process, adb, mirror, or device work. No caller-supplied receipt can change it.

use crate::android_tunnel::{
    AdmittedRuntime, AndroidTunnelOwner, ControlChallenge, OwnerControlPermit, RuntimeAdmission,
};
pub use crate::android_tunnel::{
    Cancellation, CapturePair, CaptureSource, DeviceToken, Error, Hierarchy, InteractionMode,
    InteractionState, OwnerAdb, OwnerTargetDescriptor, PhysicalRect, PngImage, Result,
    SemanticElement, UsbSelection, retry_retirements,
};
use std::{
    fs::{self, File, Metadata},
    io::Read,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

const CONTROL_CONFIRMATION_LIFETIME: Duration = Duration::from_secs(30);
const MAX_RUNTIME_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_RUNTIME_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const DLL_NAMES: [&str; 6] = [
    "SDL3.dll",
    "avcodec-62.dll",
    "avformat-62.dll",
    "avutil-60.dll",
    "libusb-1.0.dll",
    "swresample-6.dll",
];

/// Only reviewed source may install a value here after actual-byte DLL license,
/// configuration, aggregate, replaceability and notices approval. The prepared
/// inventory is intentionally not imported as authority.
const COMPILED_APPROVED_CATALOG: Option<&ApprovedCatalog> = None;

#[derive(Clone, Copy)]
struct ApprovedFile {
    relative: &'static str,
    bytes: u64,
    sha256: [u8; 32],
}

/// Private, fixed-shape approval data: no JSON/file/report parser or public
/// constructor may turn an arbitrary runtime inventory into this value.
struct ApprovedCatalog {
    client: ApprovedFile,
    server: ApprovedFile,
    dlls: [ApprovedFile; 6],
}

impl ApprovedCatalog {
    fn files(&self) -> impl Iterator<Item = &ApprovedFile> {
        std::iter::once(&self.client)
            .chain(std::iter::once(&self.server))
            .chain(self.dlls.iter())
    }

    fn validate(&self) -> Result<()> {
        if self.client.relative != "scrcpy.exe"
            || self.server.relative != "scrcpy-server"
            || self
                .dlls
                .iter()
                .zip(DLL_NAMES)
                .any(|(file, expected)| file.relative != expected)
        {
            return Err(Error::RuntimeUnavailable);
        }
        let mut total = 0u64;
        for file in self.files() {
            if file.bytes == 0 || file.bytes > MAX_RUNTIME_FILE_BYTES || file.sha256 == [0; 32] {
                return Err(Error::RuntimeUnavailable);
            }
            total = total
                .checked_add(file.bytes)
                .ok_or(Error::RuntimeUnavailable)?;
            if total > MAX_RUNTIME_TOTAL_BYTES {
                return Err(Error::RuntimeUnavailable);
            }
        }
        Ok(())
    }

    fn verify(&self, directory: &Path) -> Result<AdmittedRuntime> {
        self.validate()?;
        plain_directory(directory)?;
        for file in self.files() {
            file.verify(directory)?;
        }
        // Windows F reopens and retains its exact namespace/file pins and
        // repeats these hashes before launch; this read-only policy preflight
        // does not replace those native lifetime/TOCTOU checks.
        AdmittedRuntime::from_root_admission(
            directory.join(self.client.relative),
            directory.join(self.server.relative),
            self.client.sha256,
            self.server.sha256,
            self.dlls
                .iter()
                .map(|file| (directory.join(file.relative), file.sha256))
                .collect(),
        )
    }
}

fn plain(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return false;
        }
    }
    true
}

fn plain_directory(directory: &Path) -> Result<()> {
    if !directory.is_absolute()
        || directory.as_os_str().len() > 32_760
        || directory.components().count() > 256
        || directory
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(Error::RuntimeUnavailable);
    }
    #[cfg(windows)]
    crate::process::windows::namespace(directory).map_err(|_| Error::RuntimeUnavailable)?;
    for ancestor in directory.ancestors() {
        let metadata = fs::symlink_metadata(ancestor).map_err(|_| Error::RuntimeUnavailable)?;
        if !metadata.is_dir() || !plain(&metadata) {
            return Err(Error::RuntimeUnavailable);
        }
    }
    Ok(())
}

impl ApprovedFile {
    fn verify(&self, directory: &Path) -> Result<()> {
        let path = directory.join(self.relative);
        let metadata = fs::symlink_metadata(&path).map_err(|_| Error::RuntimeUnavailable)?;
        if !metadata.is_file() || !plain(&metadata) || metadata.len() != self.bytes {
            return Err(Error::RuntimeUnavailable);
        }
        let mut file = File::open(path).map_err(|_| Error::RuntimeUnavailable)?;
        let opened = file.metadata().map_err(|_| Error::RuntimeUnavailable)?;
        if !opened.is_file() || !plain(&opened) || opened.len() != self.bytes {
            return Err(Error::RuntimeUnavailable);
        }
        let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
        let mut buffer = [0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|_| Error::RuntimeUnavailable)?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(count as u64)
                .ok_or(Error::RuntimeUnavailable)?;
            if total > self.bytes {
                return Err(Error::RuntimeUnavailable);
            }
            digest.update(&buffer[..count]);
        }
        if total != self.bytes || digest.finish().as_ref() != self.sha256 {
            return Err(Error::RuntimeUnavailable);
        }
        Ok(())
    }
}

struct RootAdmission<'a> {
    directory: PathBuf,
    catalog: &'a ApprovedCatalog,
}

impl RuntimeAdmission for RootAdmission<'_> {
    fn acquire(&self) -> Result<AdmittedRuntime> {
        self.catalog.verify(&self.directory)
    }
}

/// Reports compiled policy availability only, without probing an installation.
pub fn package_available() -> bool {
    COMPILED_APPROVED_CATALOG.is_some_and(|catalog| catalog.validate().is_ok())
}

struct PendingConfirmation<T> {
    challenge: T,
    model: String,
    transport_id: u32,
    expires_at: Instant,
}

impl<T> PendingConfirmation<T> {
    fn new(challenge: T, model: &str, transport_id: u32, now: Instant) -> Result<Self> {
        let expires_at = now
            .checked_add(CONTROL_CONFIRMATION_LIFETIME)
            .ok_or(Error::Limit)?;
        Ok(Self {
            challenge,
            model: model.to_owned(),
            transport_id,
            expires_at,
        })
    }

    fn consume(
        pending: &mut Option<Self>,
        displayed_model: &str,
        displayed_transport_id: u32,
        cancel: &Cancellation,
        now: Instant,
    ) -> Result<(T, Duration)> {
        // Take before every refusal, including cancellation: retries need a new
        // displayed local confirmation, never the same pending challenge.
        let pending = pending.take().ok_or(Error::Consent)?;
        cancel.check()?;
        let lifetime = pending
            .expires_at
            .checked_duration_since(now)
            .ok_or(Error::Consent)?;
        if lifetime.is_zero()
            || lifetime > CONTROL_CONFIRMATION_LIFETIME
            || displayed_model != pending.model
            || displayed_transport_id != pending.transport_id
        {
            return Err(Error::Consent);
        }
        Ok((pending.challenge, lifetime))
    }
}

/// One raw owner and one pending challenge. Root retains this whole value while
/// close/revoke fails, and runs its bounded blocking methods off the UI thread.
pub struct LocalTunnelOwner {
    owner: AndroidTunnelOwner,
    pending: Option<PendingConfirmation<ControlChallenge>>,
}

impl LocalTunnelOwner {
    pub fn open(
        runtime_directory: PathBuf,
        adb: OwnerAdb,
        selection: UsbSelection,
        session: [u8; 16],
        generation: u64,
        cancel: &Cancellation,
    ) -> Result<Self> {
        // This check precedes all file, OS, process, and device operations.
        let catalog = COMPILED_APPROVED_CATALOG.ok_or(Error::RuntimeUnavailable)?;
        let admission = RootAdmission {
            directory: runtime_directory,
            catalog,
        };
        let owner = AndroidTunnelOwner::open(
            Some(&admission),
            adb,
            selection,
            session,
            generation,
            cancel,
        )?;
        Ok(Self {
            owner,
            pending: None,
        })
    }

    pub fn interaction_state(&self) -> InteractionState {
        self.owner.interaction_state()
    }

    pub fn mirror_revision(&self) -> u64 {
        self.owner.mirror_revision()
    }

    pub fn begin_control_confirmation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<OwnerTargetDescriptor> {
        self.pending = None;
        let challenge = self.owner.control_challenge(cancel)?;
        let target = challenge.target().clone();
        let pending = PendingConfirmation::new(
            challenge,
            target.model(),
            target.transport_id(),
            Instant::now(),
        )?;
        cancel.check()?;
        self.pending = Some(pending);
        Ok(target)
    }

    /// Only root's fresh explicit local confirmation handler calls this with
    /// the model/transport it actually displayed. Peer/project data must never
    /// be routed to this entry; no arbitrary receipt or approval bool is used.
    pub fn confirm_control(
        &mut self,
        displayed_model: &str,
        displayed_transport_id: u32,
        cancel: &Cancellation,
    ) -> Result<()> {
        let (challenge, lifetime) = PendingConfirmation::consume(
            &mut self.pending,
            displayed_model,
            displayed_transport_id,
            cancel,
            Instant::now(),
        )?;
        let mut permit = OwnerControlPermit::mint_from_root_confirmation(
            &challenge,
            challenge.target(),
            lifetime,
        )?;
        self.owner.enable_control(Some(&mut permit), cancel)
    }

    pub fn cancel_control_confirmation(&mut self) {
        self.pending = None;
    }

    pub fn revoke_control(&mut self) -> Result<()> {
        self.pending = None;
        self.owner.revoke_control()
    }

    pub fn check_source(&mut self, source: CaptureSource, expected_revision: u64) -> Result<()> {
        self.owner.check_source(source, expected_revision)
    }

    pub fn capture_pair(&mut self, cancel: &Cancellation) -> Result<CapturePair> {
        self.owner.capture_pair(cancel)
    }

    pub fn close(&mut self) -> Result<()> {
        self.pending = None;
        self.owner.close()
    }
}

#[cfg(test)]
#[path = "android_tunnel_root_tests.rs"]
mod tests;
