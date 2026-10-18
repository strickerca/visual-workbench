//! Header admission never allocates proportional to compressed source or pixels.
use crate::{Cancellation, CoreError, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

pub(super) const MAX_ORIGINAL: u64 = 4 * 1024 * 1024 * 1024;
const MAX_HEADER: u64 = 16 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Metadata {
    pub width: u32,
    pub height: u32,
    pub orientation: u8,
    pub format: &'static str,
}
impl Metadata {
    pub fn dimensions(&self) -> (u32, u32) {
        if self.orientation >= 5 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }
    fn admit(self) -> Result<Self> {
        let (w, h) = self.dimensions();
        vw_assets::Pyramid::new(w, h).map_err(|_| CoreError::Invalid)?;
        Ok(self)
    }
}
pub(super) fn regular(path: &Path) -> Result<File> {
    let before = std::fs::symlink_metadata(path)?;
    if !before.is_file() || before.file_type().is_symlink() {
        return Err(CoreError::Invalid);
    }
    #[cfg(all(test, any(target_os = "linux", target_os = "android")))]
    open_race::run(path);
    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        if before.file_attributes() & 0x400 != 0 {
            return Err(CoreError::Invalid);
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x0020_0000)
            .open(path)?;
        if file.metadata()?.file_attributes() & 0x400 != 0 {
            return Err(CoreError::Invalid);
        }
        Ok(file)
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let fd = rustix::fs::open(
            path,
            // Metadata is only a pathname observation. A replacement FIFO must
            // not block before the descriptor's own regular-file check below.
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NONBLOCK,
            rustix::fs::Mode::empty(),
        )
        .map_err(std::io::Error::from)?;
        let file = File::from(fd);
        if !file.metadata()?.is_file() {
            return Err(CoreError::Invalid);
        }
        Ok(file)
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
    {
        Err(CoreError::Unsupported)
    }
}

/// Deterministic test-only interleaving at the real pathname/descriptor seam.
/// A thread-local, exact-path, one-shot hook cannot affect another test/worker.
#[cfg(all(test, any(target_os = "linux", target_os = "android")))]
pub(crate) mod open_race {
    use std::{
        cell::RefCell,
        path::{Path, PathBuf},
    };
    type Hook = Option<(PathBuf, Box<dyn FnOnce()>)>;
    thread_local! { static HOOK: RefCell<Hook> = const { RefCell::new(None) }; }
    pub(crate) struct Guard;
    pub(crate) fn install(path: PathBuf, callback: impl FnOnce() + 'static) -> Guard {
        HOOK.with(|slot| {
            assert!(slot.borrow().is_none(), "one scoped opener hook");
            *slot.borrow_mut() = Some((path, Box::new(callback)));
        });
        Guard
    }
    pub(super) fn run(path: &Path) {
        let hook = HOOK.with(|slot| {
            let mut value = slot.borrow_mut();
            if value.as_ref().is_some_and(|(expected, _)| expected == path) {
                value.take().map(|(_, callback)| callback)
            } else {
                None
            }
        });
        if let Some(callback) = hook {
            callback();
        }
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            HOOK.with(|slot| {
                slot.borrow_mut().take();
            });
        }
    }
}
#[cfg(test)]
pub(super) fn hash_copy(
    input: &mut File,
    output: Option<&mut File>,
    limit: u64,
    cancel: &Cancellation,
) -> Result<([u8; 32], String, u64)> {
    hash_copy_checked(input, output, limit, || cancel.check())
}
pub(super) fn hash_copy_checked(
    input: &mut File,
    mut output: Option<&mut File>,
    limit: u64,
    check: impl Fn() -> Result<()>,
) -> Result<([u8; 32], String, u64)> {
    input.seek(SeekFrom::Start(0))?;
    let mut blake = blake3::Hasher::new();
    let mut sha = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        check()?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes = bytes.checked_add(count as u64).ok_or(CoreError::Invalid)?;
        if bytes > limit {
            return Err(CoreError::Backpressure);
        }
        blake.update(&buffer[..count]);
        sha.update(&buffer[..count]);
        if let Some(out) = output.as_mut() {
            out.write_all(&buffer[..count])?
        }
    }
    if bytes == 0 {
        return Err(CoreError::Invalid);
    }
    let sha = sha.finalize();
    let hex = sha.iter().map(|value| format!("{value:02x}")).collect();
    Ok((*blake.finalize().as_bytes(), hex, bytes))
}
pub(super) fn inspect(input: &mut File, cancel: &Cancellation) -> Result<Metadata> {
    input.seek(SeekFrom::Start(0))?;
    let total = input.metadata()?.len();
    if !(33..=MAX_ORIGINAL).contains(&total) {
        return Err(CoreError::Invalid);
    }
    let mut magic = [0; 8];
    input.read_exact(&mut magic)?;
    input.seek(SeekFrom::Start(0))?;
    if magic == *b"\x89PNG\r\n\x1a\n" {
        png(input, total, cancel)
    } else if magic[..2] == [0xff, 0xd8] {
        jpeg(input, total, cancel)
    } else {
        Err(CoreError::Unsupported)
    }
}
fn png(input: &mut File, total: u64, cancel: &Cancellation) -> Result<Metadata> {
    input.seek(SeekFrom::Start(8))?;
    let mut result = None;
    let (mut header, mut has_data) = (0u64, false);
    loop {
        cancel.check()?;
        let mut chunk = [0; 8];
        input.read_exact(&mut chunk)?;
        let len = u32::from_be_bytes(chunk[..4].try_into().map_err(|_| CoreError::Invalid)?) as u64;
        let kind = &chunk[4..];
        let start = input.stream_position()?;
        let end = start
            .checked_add(len)
            .and_then(|v| v.checked_add(4))
            .ok_or(CoreError::Invalid)?;
        if end > total {
            return Err(CoreError::Invalid);
        }
        if result.is_none() {
            if kind != b"IHDR" || len != 13 {
                return Err(CoreError::Invalid);
            }
            let mut data = [0; 13];
            input.read_exact(&mut data)?;
            if data[8..] != [8, 2, 0, 0, 0] && data[8..] != [8, 2, 0, 0, 1] {
                return Err(CoreError::Unsupported);
            }
            result = Some(Metadata {
                width: u32::from_be_bytes(data[..4].try_into().map_err(|_| CoreError::Invalid)?),
                height: u32::from_be_bytes(data[4..8].try_into().map_err(|_| CoreError::Invalid)?),
                orientation: 1,
                format: "png",
            });
        } else if kind == b"IHDR" {
            return Err(CoreError::Invalid);
        } else if [
            b"iCCP", b"sRGB", b"eXIf", b"gAMA", b"cHRM", b"cICP", b"cLLI", b"mDCV", b"tRNS",
            b"acTL",
        ]
        .iter()
        .any(|value| kind == *value)
        {
            return Err(CoreError::Unsupported);
        } else if kind == b"IDAT" {
            has_data = true;
        } else if kind == b"IEND" {
            if len != 0 || !has_data || end != total {
                return Err(CoreError::Invalid);
            }
            return result.ok_or(CoreError::Invalid)?.admit();
        } else if kind[0] & 32 == 0 && kind != b"PLTE" {
            return Err(CoreError::Unsupported);
        }
        if kind != b"IDAT" {
            header = header.checked_add(len + 12).ok_or(CoreError::Invalid)?;
            if header > MAX_HEADER {
                return Err(CoreError::Backpressure);
            }
        }
        input.seek(SeekFrom::Start(end))?;
    }
}
fn jpeg(input: &mut File, total: u64, cancel: &Cancellation) -> Result<Metadata> {
    input.seek(SeekFrom::Start(2))?;
    let (mut result, mut orientation, mut exif) = (None, 1, false);
    loop {
        cancel.check()?;
        if input.stream_position()? > MAX_HEADER {
            return Err(CoreError::Backpressure);
        }
        let mut marker = [0; 2];
        input.read_exact(&mut marker)?;
        if marker[0] != 0xff {
            return Err(CoreError::Invalid);
        }
        while marker[1] == 0xff {
            input.read_exact(&mut marker[1..])?;
            if input.stream_position()? > MAX_HEADER {
                return Err(CoreError::Backpressure);
            }
        }
        if marker[1] == 0xda {
            return result
                .map(|mut value: Metadata| {
                    value.orientation = orientation;
                    value
                })
                .ok_or(CoreError::Invalid)?
                .admit();
        }
        if matches!(marker[1], 0 | 0xd8 | 0xd9 | 0x01 | 0xd0..=0xd7) {
            return Err(CoreError::Invalid);
        }
        let mut length = [0; 2];
        input.read_exact(&mut length)?;
        let len = usize::from(u16::from_be_bytes(length));
        if len < 2 {
            return Err(CoreError::Invalid);
        }
        let end = input
            .stream_position()?
            .checked_add((len - 2) as u64)
            .ok_or(CoreError::Invalid)?;
        if end > total || end > MAX_HEADER {
            return Err(CoreError::Backpressure);
        }
        let mut payload = vec![0; len - 2];
        input.read_exact(&mut payload)?;
        if marker[1] == 0xe2 && payload.starts_with(b"ICC_PROFILE\0") {
            return Err(CoreError::Unsupported);
        }
        // MPF multi-picture / gain-map and JUMBF sources are outside this still
        // SDR header subset. Full platform decode remains a separate producer
        // requirement; inspecting this header alone is never a codec proof.
        if (marker[1] == 0xe2 && payload.starts_with(b"MPF\0")) || marker[1] == 0xeb {
            return Err(CoreError::Unsupported);
        }
        if marker[1] == 0xe1 && payload.starts_with(b"Exif\0\0") {
            if exif {
                return Err(CoreError::Invalid);
            }
            exif = true;
            orientation = exif_orientation(&payload[6..])?;
        }
        if marker[1] == 0xee && payload.starts_with(b"Adobe") {
            return Err(CoreError::Unsupported);
        }
        if matches!(marker[1], 0xc0..=0xcf) && !matches!(marker[1], 0xc4 | 0xc8 | 0xcc) {
            if !matches!(marker[1], 0xc0..=0xc2)
                || payload.len() != 15
                || payload[0] != 8
                || payload[5] != 3
            {
                return Err(CoreError::Unsupported);
            }
            if result.is_some() {
                return Err(CoreError::Invalid);
            }
            result = Some(Metadata {
                height: u32::from(u16::from_be_bytes([payload[1], payload[2]])),
                width: u32::from(u16::from_be_bytes([payload[3], payload[4]])),
                orientation: 1,
                format: "jpeg",
            });
        }
    }
}
fn exif_orientation(bytes: &[u8]) -> Result<u8> {
    let little = match bytes.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return Err(CoreError::Invalid),
    };
    let word = |at: usize| -> Result<u16> {
        let end = at.checked_add(2).ok_or(CoreError::Invalid)?;
        let raw: [u8; 2] = bytes
            .get(at..end)
            .ok_or(CoreError::Invalid)?
            .try_into()
            .map_err(|_| CoreError::Invalid)?;
        Ok(if little {
            u16::from_le_bytes(raw)
        } else {
            u16::from_be_bytes(raw)
        })
    };
    let long = |at: usize| -> Result<u32> {
        let end = at.checked_add(4).ok_or(CoreError::Invalid)?;
        let raw: [u8; 4] = bytes
            .get(at..end)
            .ok_or(CoreError::Invalid)?
            .try_into()
            .map_err(|_| CoreError::Invalid)?;
        Ok(if little {
            u32::from_le_bytes(raw)
        } else {
            u32::from_be_bytes(raw)
        })
    };
    if word(2)? != 42 {
        return Err(CoreError::Invalid);
    }
    let at = long(4)? as usize;
    let count = word(at)? as usize;
    if count > 1024
        || at
            .checked_add(2 + count * 12 + 4)
            .filter(|end| *end <= bytes.len())
            .is_none()
    {
        return Err(CoreError::Invalid);
    }
    let mut orientation = None;
    for index in 0..count {
        let p = at + 2 + index * 12;
        if word(p)? == 0x112 {
            if orientation.is_some() || word(p + 2)? != 3 || long(p + 4)? != 1 {
                return Err(CoreError::Invalid);
            }
            let value = word(p + 8)?;
            if !(1..=8).contains(&value) {
                return Err(CoreError::Invalid);
            }
            orientation = Some(value as u8)
        }
    }
    Ok(orientation.unwrap_or(1))
}

use super::{GiantImageError as Error, GiantImageResult as ImageResult};
use crate::{WorkflowBinding, project::ProjectState};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};
use vw_assets::Source;
use vw_model::{AssetId, Id};
use vw_stream::{MemoryCharge, MemoryKind, SessionBudget};

/// Captured only from the serialized canonical document binding. There is no
/// caller-selected source or producer directory in this read capability.
#[derive(Clone)]
pub(super) struct CapturedSource {
    pub root: PathBuf,
    pub path: PathBuf,
    pub source: Source,
    pub format: &'static str,
}

pub(super) fn live(closed: &AtomicBool, cancel: &Cancellation) -> ImageResult<()> {
    if closed.load(Ordering::Acquire) {
        return Err(Error::Closed);
    }
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    Ok(())
}
pub(super) fn reserve(
    budget: &SessionBudget,
    kind: MemoryKind,
    bytes: u64,
) -> ImageResult<MemoryCharge> {
    budget.reserve(kind, bytes).map_err(|_| Error::Memory {
        estimated: budget.used_bytes().saturating_add(bytes),
        budget: budget.limit_bytes(),
    })
}
pub(super) fn store_error(error: vw_store::StoreError) -> Error {
    match error {
        vw_store::StoreError::Busy => Error::Backpressure,
        vw_store::StoreError::LowSpace => Error::Limit,
        vw_store::StoreError::Invalid(
            "cache read limit" | "cache manifest size" | "cache allocation" | "cache address space",
        ) => Error::Limit,
        vw_store::StoreError::Invalid(_) => Error::Invalid,
        vw_store::StoreError::Io(e) if e.kind() == std::io::ErrorKind::NotFound => Error::Missing,
        vw_store::StoreError::Corrupt(_) => Error::Invalid,
        _ => Error::Storage,
    }
}
fn workflow_error(error: crate::WorkflowError) -> Error {
    match error {
        crate::WorkflowError::Stale | crate::WorkflowError::Missing => Error::Stale,
        crate::WorkflowError::Closed => Error::Closed,
        crate::WorkflowError::Cancelled => Error::Cancelled,
        crate::WorkflowError::Limit => Error::Limit,
        crate::WorkflowError::Memory { estimated, budget } => Error::Memory { estimated, budget },
        crate::WorkflowError::Backpressure => Error::Backpressure,
        crate::WorkflowError::Storage => Error::Storage,
        _ => Error::Invalid,
    }
}
pub(super) fn capture(
    state: &ProjectState,
    binding: &WorkflowBinding,
    closed: &AtomicBool,
    cancel: &Cancellation,
    budget: &SessionBudget,
) -> ImageResult<CapturedSource> {
    live(closed, cancel)?;
    let estimated = crate::workflow::canonical_workspace(
        state,
        budget.limit_bytes().min(crate::workflow::MAX_MEMORY),
        false,
    )
    .map_err(workflow_error)?;
    let _canonical = reserve(budget, MemoryKind::CodecWorkspace, estimated)?;
    crate::workflow::check_binding(state, binding).map_err(workflow_error)?;
    let project = state.project().map_err(Error::from)?;
    let document = project
        .documents
        .get(&Id::try_from(binding.document_id.clone()).map_err(|_| Error::Invalid)?)
        .ok_or(Error::Stale)?;
    // No page selector exists in this capability. A paged/PDF document must
    // never silently borrow its primary asset as though it were the image page.
    if document.definition.kind != vw_proto::v1::DocumentKind::Image as i32
        || !document.pages.is_empty()
    {
        return Err(Error::Unsupported);
    }
    let id = AssetId::try_from(document.definition.primary_asset_id.clone())
        .map_err(|_| Error::Invalid)?;
    let asset = project.assets.get(&id).ok_or(Error::Stale)?;
    let format = match asset.format.as_str() {
        "jpeg" | "jpg" => "jpeg",
        "png" => "png",
        _ => return Err(Error::Unsupported),
    };
    if asset.has_alpha
        || asset.bit_depth != 8
        || !asset.icc_profile.is_empty()
        || asset.color_space != "untagged"
    {
        return Err(Error::Unsupported);
    }
    let (width, height) = if asset.orientation >= 5 {
        (asset.height, asset.width)
    } else {
        (asset.width, asset.height)
    };
    let source = Source {
        original: id.bytes(),
        original_bytes: asset.byte_size,
        width,
        height,
        orientation: u8::try_from(asset.orientation).map_err(|_| Error::Invalid)?,
        bit_depth: 8,
        icc: None,
        display_conversion: false,
    };
    source.validate().map_err(|_| Error::Limit)?;
    let root = state
        .store()
        .map_err(Error::from)?
        .root_path()
        .to_path_buf();
    let path = state.blobs.path(&id).map_err(store_error)?;
    live(closed, cancel)?;
    Ok(CapturedSource {
        root,
        path,
        source,
        format,
    })
}
impl CapturedSource {
    /// Streaming verification retains no full original Vec. Repeat before each
    /// read so same-length damage/replacement cannot bless a derived result.
    /// This is bounded I/O, not a claimed giant-image latency measurement.
    pub fn verify(
        &self,
        closed: &AtomicBool,
        cancel: &Cancellation,
        budget: &SessionBudget,
    ) -> ImageResult<()> {
        self.verify_guarded(closed, None, cancel, budget)
    }
    pub fn verify_owned(
        &self,
        project_closed: &AtomicBool,
        owner_closed: &AtomicBool,
        cancel: &Cancellation,
        budget: &SessionBudget,
    ) -> ImageResult<()> {
        self.verify_guarded(project_closed, Some(owner_closed), cancel, budget)
    }
    fn verify_guarded(
        &self,
        closed: &AtomicBool,
        owner_closed: Option<&AtomicBool>,
        cancel: &Cancellation,
        budget: &SessionBudget,
    ) -> ImageResult<()> {
        live(closed, cancel)?;
        if let Some(owner_closed) = owner_closed {
            live(owner_closed, cancel)?;
        }
        let _workspace = reserve(budget, MemoryKind::Transfer, 1024 * 1024)?;
        if let Err(error) = std::fs::symlink_metadata(&self.path) {
            return Err(if error.kind() == std::io::ErrorKind::NotFound {
                Error::Missing
            } else {
                Error::Storage
            });
        }
        let mut file = regular(&self.path).map_err(Error::from)?;
        if file.metadata().map_err(|_| Error::Storage)?.len() != self.source.original_bytes {
            return Err(Error::Stale);
        }
        let (hash, _, size) =
            hash_copy_checked(&mut file, None, self.source.original_bytes, || {
                if closed.load(Ordering::Acquire)
                    || owner_closed.is_some_and(|owner| owner.load(Ordering::Acquire))
                {
                    return Err(CoreError::Closed);
                }
                cancel.check()
            })
            .map_err(Error::from)?;
        if hash != self.source.original || size != self.source.original_bytes {
            return Err(Error::Stale);
        }
        let metadata = inspect(&mut file, cancel).map_err(Error::from)?;
        if metadata.dimensions() != (self.source.width, self.source.height)
            || metadata.orientation != self.source.orientation
            || metadata.format != self.format
        {
            return Err(Error::Stale);
        }
        live(closed, cancel)?;
        if let Some(owner_closed) = owner_closed {
            live(owner_closed, cancel)?;
        }
        Ok(())
    }
    pub fn same_source(&self, other: &Self) -> bool {
        self.root == other.root
            && self.path == other.path
            && self.source == other.source
            && self.format == other.format
    }
}
