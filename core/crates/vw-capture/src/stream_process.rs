//! One pinned helper and one bounded in-flight exchange. Windows owns a
//! suspended, kill-on-close Job before the helper's first instruction runs.
use crate::stream::stream_error;
use crate::{
    Error, FrameIdentity, Result,
    stream::StreamRequest,
    stream_wire::{Command, Header, Packet},
};
use vw_proto::Message;
use vw_stream::{Cancellation, EncodedTiles, MemoryKind, SessionBudget};

/// Wire values can confirm exact correlation, never create a capture grant.
/// The root facade separately checks authenticated CONTROL/MEDIA/BLOB owners.
pub fn validate_reply(
    request: &StreamRequest,
    command: &Command,
    packet: &Packet,
    budget: &SessionBudget,
) -> Result<()> {
    let sequence = command.sequence().ok_or(Error::Invalid)?;
    if !packet.belongs_to(budget)
        || sequence == 0
        || packet.header.sequence() != Some(sequence)
        || command.binding() != Some((request.connection_epoch, request.source_generation))
        || packet.header.binding() != (request.connection_epoch, request.source_generation)
        || packet.header.payload_len()? != packet.payload.bytes().len()
    {
        return Err(Error::Invalid);
    }
    match (&packet.header, command) {
        (Header::Refused { .. }, _) if packet.payload.bytes().is_empty() => Ok(()),
        (Header::Stopped { .. }, Command::Stop { .. }) if packet.payload.bytes().is_empty() => {
            Ok(())
        }
        (
            Header::Idle {
                retained_frames,
                retained_bytes,
                ..
            },
            Command::Poll { .. } | Command::InspectSource { .. },
        ) if packet.payload.bytes().is_empty()
            && *retained_frames <= crate::stream::STREAM_RING_FRAMES
            && *retained_bytes <= request.ring_bytes =>
        {
            Ok(())
        }
        (
            Header::Tiles {
                frame_id,
                timestamp_ns,
                captured_at_ms,
                keyframe,
                ..
            },
            Command::Poll { .. },
        ) => {
            if *frame_id == 0
                || *timestamp_ns < request.source.observed_ns()
                || !(1..=253_402_300_799_999).contains(captured_at_ms)
                || packet.payload.bytes().len() > vw_stream::MAX_TILE_BYTES + 64 * 1024
            {
                return Err(Error::Invalid);
            }
            let cancel = Cancellation::default();
            let _parse_workspace = budget
                .reserve(
                    MemoryKind::CodecWorkspace,
                    (vw_stream::MAX_TILE_BYTES + 64 * 1024) as u64,
                )
                .map_err(stream_error)?;
            let decoded_owner = EncodedTiles::reserve(budget, &cancel).map_err(stream_error)?;
            preflight_frame_tiles(packet.payload.bytes())?;
            let tiles = vw_proto::v1::FrameTiles::decode(packet.payload.bytes())
                .map_err(|_| Error::Invalid)?;
            let tiles = decoded_owner.adopt(tiles, &cancel).map_err(stream_error)?;
            let tiles = tiles.packet();
            let expected = request.stream_geometry(*timestamp_ns)?;
            let actual = vw_stream::StreamGeometry::new(
                tiles.geometry.as_ref().ok_or(Error::Invalid)?,
                tiles.width,
                tiles.height,
            )
            .map_err(stream_error)?;
            if !actual.same_source(&expected)
                || tiles.timestamp_ns != *timestamp_ns as i64
                || actual.wire().timestamp_ns != *timestamp_ns as i64
                || tiles.frame_id != *frame_id
                || tiles.keyframe != *keyframe
                || tiles.last_input_seq_applied != 0
                || tiles.delivery_sequence != 0
                || tiles.source_generation != 0
                || tiles.captured_at_ms != *captured_at_ms
                || tiles.tiles.iter().any(|tile| tile.format != "jpeg")
                || vw_model::Id::from_proto(tiles.capture_session_id.as_ref())
                    .map_err(|_| Error::Invalid)?
                    .as_str()
                    != request.capture_session_id
            {
                return Err(Error::Invalid);
            }
            Ok(())
        }
        (
            Header::Original {
                identity, asset_id, ..
            },
            Command::Original {
                identity: expected, ..
            },
        ) => {
            identity.validate()?;
            if !same_identity(identity, expected)
                || identity.capture_session_id != request.capture_session_id
                || identity.geometry_revision != request.geometry_revision
                || identity.client_rect != request.source.rect()
                || identity.monotonic_timestamp_ns < request.source.observed_ns()
                || identity.platform != "windows"
                || packet.payload.bytes().is_empty()
                || vw_model::AssetId::hash(packet.payload.bytes()).as_str() != asset_id
            {
                return Err(Error::Invalid);
            }
            let geometry = request.stream_geometry(identity.monotonic_timestamp_ns)?;
            let wire = geometry.wire();
            if identity.source_kind != wire.source_kind
                || identity.window_handle != wire.window_handle
                || identity.monitor_id != wire.monitor_id
                || identity.dpi_scale != wire.dpi_scale
            {
                return Err(Error::Invalid);
            }
            Ok(())
        }
        _ => Err(Error::Invalid),
    }
}
fn same_identity(a: &FrameIdentity, b: &FrameIdentity) -> bool {
    a.capture_session_id == b.capture_session_id
        && a.frame_id == b.frame_id
        && a.geometry_revision == b.geometry_revision
        && a.source_kind == b.source_kind
        && a.client_rect == b.client_rect
        && a.dpi_scale == b.dpi_scale
        && a.monotonic_timestamp_ns == b.monotonic_timestamp_ns
        && a.captured_at_ms == b.captured_at_ms
        && a.platform == b.platform
        && a.window_handle == b.window_handle
        && a.monitor_id == b.monitor_id
}

#[derive(Clone, Copy)]
enum ProtoKind {
    Frame,
    Uuid,
    Geometry,
    Tile,
    Rect,
}
#[derive(Default)]
struct ProtoCounts {
    fields: usize,
    tiles: usize,
    tile_bytes: usize,
}
fn varint(bytes: &[u8], cursor: &mut usize) -> Result<u64> {
    let mut value = 0u64;
    for shift in (0..=63).step_by(7) {
        let byte = *bytes.get(*cursor).ok_or(Error::Invalid)?;
        *cursor += 1;
        if shift == 63 && byte > 1 {
            return Err(Error::Invalid);
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(Error::Invalid)
}
fn scan_proto(bytes: &[u8], kind: ProtoKind, counts: &mut ProtoCounts) -> Result<()> {
    let mut cursor = 0usize;
    let mut seen = 0u16;
    while cursor < bytes.len() {
        counts.fields = counts.fields.checked_add(1).ok_or(Error::Limit)?;
        // One fully populated tile contributes eight wire fields (the outer
        // repeated field, three tile fields and four rectangle fields). Leave
        // room for all 128 tiles plus frame/geometry metadata.
        if counts.fields > 1200 {
            return Err(Error::Limit);
        }
        let key = varint(bytes, &mut cursor)?;
        let field = key >> 3;
        let wire = (key & 7) as u8;
        let (expected, max_len, nested, repeat) = match (kind, field) {
            (ProtoKind::Frame, 1) => (2, 32, Some(ProtoKind::Uuid), false),
            (ProtoKind::Frame, 2 | 4 | 5 | 6 | 8 | 9 | 10 | 11 | 12) => (0, 0, None, false),
            (ProtoKind::Frame, 3) => (2, 512, Some(ProtoKind::Geometry), false),
            (ProtoKind::Frame, 7) => (
                2,
                vw_stream::MAX_TILE_BYTES + 128,
                Some(ProtoKind::Tile),
                true,
            ),
            (ProtoKind::Uuid, 1) => (2, 16, None, false),
            (ProtoKind::Geometry, 1) => (2, 16, None, false),
            (ProtoKind::Geometry, 2 | 6 | 7) => (0, 0, None, false),
            (ProtoKind::Geometry, 3) => (2, 128, None, false),
            (ProtoKind::Geometry, 4) => (2, 64, Some(ProtoKind::Rect), false),
            (ProtoKind::Geometry, 5) => (1, 0, None, false),
            (ProtoKind::Tile, 1) => (2, 64, Some(ProtoKind::Rect), false),
            (ProtoKind::Tile, 2) => (2, 16, None, false),
            (ProtoKind::Tile, 3) => (2, vw_stream::MAX_TILE_BYTES, None, false),
            (ProtoKind::Rect, 1..=4) => (0, 0, None, false),
            _ => return Err(Error::Invalid),
        };
        if wire != expected {
            return Err(Error::Invalid);
        }
        if !repeat {
            let bit = 1u16
                .checked_shl(u32::try_from(field).map_err(|_| Error::Invalid)?)
                .ok_or(Error::Invalid)?;
            if seen & bit != 0 {
                return Err(Error::Invalid);
            }
            seen |= bit;
        }
        match wire {
            0 => {
                varint(bytes, &mut cursor)?;
            }
            1 => {
                cursor = cursor
                    .checked_add(8)
                    .filter(|end| *end <= bytes.len())
                    .ok_or(Error::Invalid)?;
            }
            2 => {
                let len = usize::try_from(varint(bytes, &mut cursor)?).map_err(|_| Error::Limit)?;
                if len > max_len {
                    return Err(Error::Limit);
                }
                let end = cursor
                    .checked_add(len)
                    .filter(|end| *end <= bytes.len())
                    .ok_or(Error::Invalid)?;
                if matches!(kind, ProtoKind::Frame) && field == 7 {
                    counts.tiles += 1;
                    if counts.tiles > vw_stream::MAX_TILES {
                        return Err(Error::Limit);
                    }
                }
                if matches!(kind, ProtoKind::Tile) && field == 3 {
                    counts.tile_bytes = counts.tile_bytes.checked_add(len).ok_or(Error::Limit)?;
                    if counts.tile_bytes > vw_stream::MAX_TILE_BYTES {
                        return Err(Error::Limit);
                    }
                }
                if matches!(kind, ProtoKind::Uuid) && field == 1 && len != 16 {
                    return Err(Error::Invalid);
                }
                if let Some(child) = nested {
                    scan_proto(&bytes[cursor..end], child, counts)?;
                }
                cursor = end;
            }
            _ => return Err(Error::Invalid),
        }
    }
    Ok(())
}
fn preflight_frame_tiles(bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() || bytes.len() > vw_stream::MAX_TILE_BYTES + 64 * 1024 {
        return Err(Error::Limit);
    }
    scan_proto(bytes, ProtoKind::Frame, &mut ProtoCounts::default())
}
#[cfg(windows)]
mod owned {
    use super::*;
    use crate::stream::{PROCESS_MEMORY_BYTES, STREAM_RING_FRAMES};
    use crate::stream_wire::{self, MAX_HEADER_BYTES};
    use ::windows::Win32::{
        Foundation::{CloseHandle, ERROR_NO_MORE_FILES, GetLastError, HANDLE},
        Storage::FileSystem::{
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_SHARE_READ, FILE_SHARE_WRITE,
        },
        System::{Diagnostics::ToolHelp::*, JobObjects::*, Threading::*},
    };
    use std::{
        fs::{File, OpenOptions},
        io::Read,
        os::windows::{
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        },
        path::{Path, PathBuf},
        process::{Child, Command as OsCommand, Stdio},
        sync::{
            Arc, Mutex, OnceLock,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, Instant},
    };
    use vw_stream::{Cancellation, MemoryCharge, MemoryKind, SessionBudget};

    const CREATE_SUSPENDED_FLAG: u32 = 0x0000_0004;
    const CREATE_NO_WINDOW_FLAG: u32 = 0x0800_0000;
    static ACTIVE: AtomicBool = AtomicBool::new(false);
    static UNRESOLVED: OnceLock<Mutex<Vec<Retained>>> = OnceLock::new();
    fn unresolved() -> &'static Mutex<Vec<Retained>> {
        UNRESOLVED.get_or_init(|| Mutex::new(Vec::new()))
    }
    struct Slot;
    impl Drop for Slot {
        fn drop(&mut self) {
            ACTIVE.store(false, Ordering::Release);
        }
    }
    struct Handle(usize);
    impl Handle {
        fn raw(&self) -> HANDLE {
            HANDLE(self.0 as *mut _)
        }
    }
    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: this wrapper uniquely owns a successful Win32 handle.
            unsafe {
                let _ = CloseHandle(self.raw());
            }
        }
    }
    pub struct PinnedStreamHelper {
        path: PathBuf,
        _file: File,
        _ancestors: Vec<File>,
    }
    impl PinnedStreamHelper {
        /// `trusted_sha256` comes from root's packaged runtime inventory, not
        /// the file, stream request, or helper's own response.
        pub fn open(path: &Path, trusted_sha256: &str) -> Result<Self> {
            crate::plain(path, false)?;
            if trusted_sha256.len() != 64 || !trusted_sha256.bytes().all(|c| c.is_ascii_hexdigit())
            {
                return Err(Error::Invalid);
            }
            let mut ancestors = Vec::new();
            let parent = path.parent().ok_or(Error::Invalid)?;
            for part in parent.ancestors().collect::<Vec<_>>().into_iter().rev() {
                let directory = OpenOptions::new()
                    .read(true)
                    .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
                    .open(part)
                    .map_err(|_| Error::Storage)?;
                let meta = directory.metadata().map_err(|_| Error::Storage)?;
                if !meta.is_dir() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
                    return Err(Error::Storage);
                }
                ancestors.push(directory);
            }
            let mut file = OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ.0)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
                .open(path)
                .map_err(|_| Error::Storage)?;
            let meta = file.metadata().map_err(|_| Error::Storage)?;
            if !meta.is_file()
                || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
                || meta.len() > 128 * 1024 * 1024
            {
                return Err(Error::Storage);
            }
            let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
            let mut chunk = [0u8; 65_536];
            let mut total = 0u64;
            loop {
                let n = file.read(&mut chunk).map_err(|_| Error::Storage)?;
                if n == 0 {
                    break;
                }
                total = total.checked_add(n as u64).ok_or(Error::Limit)?;
                if total > 128 * 1024 * 1024 {
                    return Err(Error::Limit);
                }
                digest.update(&chunk[..n]);
            }
            let actual = digest
                .finish()
                .as_ref()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            if actual != trusted_sha256.to_ascii_lowercase() {
                return Err(Error::Invalid);
            }
            Ok(Self {
                path: path.to_owned(),
                _file: file,
                _ancestors: ancestors,
            })
        }
    }
    struct Job(Handle);
    impl Job {
        fn new(kill_on_close: bool) -> Result<Self> {
            // SAFETY: unnamed Job; handle is immediately owned and configured.
            unsafe {
                let job = Self(Handle(
                    CreateJobObjectW(None, None).map_err(|_| Error::Platform)?.0 as usize,
                ));
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags =
                    JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
                if kill_on_close {
                    limits.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                }
                limits.BasicLimitInformation.ActiveProcessLimit = 1;
                limits.ProcessMemoryLimit = PROCESS_MEMORY_BYTES as usize;
                SetInformationJobObject(
                    job.0.raw(),
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of_val(&limits) as u32,
                )
                .map_err(|_| Error::Platform)?;
                Ok(job)
            }
        }
        fn assign(&self, child: &Child) -> Result<()> {
            // SAFETY: child handle names only the newly suspended helper.
            unsafe {
                AssignProcessToJobObject(self.0.raw(), HANDLE(child.as_raw_handle() as *mut _))
                    .map_err(|_| Error::Platform)
            }
        }
        fn terminate(&self) -> Result<()> {
            // SAFETY: this owned Job contains only the helper and at most one process.
            unsafe { TerminateJobObject(self.0.raw(), 2).map_err(|_| Error::Platform) }
        }
    }
    fn resume_primary(process_id: u32) -> Result<()> {
        // SAFETY: CREATE_SUSPENDED keeps the helper's sole primary thread from
        // running until after Job assignment. Refuse ambiguous thread identity.
        unsafe {
            let snapshot = Handle(
                CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)
                    .map_err(|_| Error::Platform)?
                    .0 as usize,
            );
            let mut entry = THREADENTRY32 {
                dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                ..Default::default()
            };
            Thread32First(snapshot.raw(), &mut entry).map_err(|_| Error::Platform)?;
            let mut found = None;
            loop {
                if entry.th32OwnerProcessID == process_id
                    && found.replace(entry.th32ThreadID).is_some()
                {
                    return Err(Error::Platform);
                }
                if Thread32Next(snapshot.raw(), &mut entry).is_err() {
                    if GetLastError() != ERROR_NO_MORE_FILES {
                        return Err(Error::Platform);
                    }
                    break;
                }
            }
            let thread = Handle(
                OpenThread(THREAD_SUSPEND_RESUME, false, found.ok_or(Error::Platform)?)
                    .map_err(|_| Error::Platform)?
                    .0 as usize,
            );
            if ResumeThread(thread.raw()) != 1 {
                return Err(Error::Platform);
            }
            Ok(())
        }
    }
    struct Queued {
        command: Command,
        _charge: MemoryCharge,
    }
    struct Owned {
        child: Child,
        job: Job,
        assigned: bool,
        sender: Option<mpsc::SyncSender<Queued>>,
        receiver: Option<mpsc::Receiver<Result<Packet>>>,
        worker: Option<thread::JoinHandle<()>>,
    }
    impl Owned {
        fn settle(&mut self) -> Result<()> {
            self.sender.take();
            // A stopped helper gets one short chance to flush and close WGC.
            // Cancellation still signals the Job within a fixed bound.
            let grace = Instant::now() + Duration::from_millis(100);
            let exited = loop {
                let observed = self
                    .child
                    .try_wait()
                    .map_err(|_| Error::Platform)?
                    .is_some();
                if observed || Instant::now() >= grace {
                    break observed;
                }
                thread::sleep(Duration::from_millis(5));
            };
            if !exited {
                if self.assigned {
                    self.job.terminate()?;
                } else {
                    self.child.kill().map_err(|_| Error::Platform)?;
                }
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if self
                    .child
                    .try_wait()
                    .map_err(|_| Error::Platform)?
                    .is_some()
                {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(Error::Busy);
                }
                thread::sleep(Duration::from_millis(5));
            }
            // A bounded late reply may be waiting in the one-slot channel.
            // Once the Job is affirmatively dead it can no longer publish;
            // dropping the receiver releases that charged queue owner.
            self.receiver.take();
            if let Some(worker) = self.worker.as_ref() {
                while !worker.is_finished() {
                    if Instant::now() >= deadline {
                        return Err(Error::Busy);
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            }
            if let Some(worker) = self.worker.take() {
                worker.join().map_err(|_| Error::Platform)?;
            }
            Ok(())
        }
    }
    struct Retained {
        owned: Owned,
        _slot: Slot,
        _helper: Arc<PinnedStreamHelper>,
    }
    pub fn retry_unresolved_retirement() -> Result<()> {
        let pending = {
            let mut guard = unresolved().lock().map_err(|_| Error::Busy)?;
            std::mem::take(&mut *guard)
        };
        let mut failures = Vec::new();
        for mut retained in pending {
            if retained.owned.settle().is_err() {
                failures.push(retained);
            }
        }
        let failed = !failures.is_empty();
        match unresolved().lock() {
            Ok(mut guard) => guard.extend(failures),
            Err(_) => {
                // Never let an unknown process/Job/slot fall out of ownership
                // because the quarantine lock itself became unusable.
                for retained in failures {
                    let _ = Box::leak(Box::new(retained));
                }
                return Err(Error::Busy);
            }
        }
        if failed { Err(Error::Busy) } else { Ok(()) }
    }

    pub struct StreamProcess {
        owned: Option<Owned>,
        slot: Option<Slot>,
        helper: Arc<PinnedStreamHelper>,
        budget: SessionBudget,
        request: StreamRequest,
        _request_charge: MemoryCharge,
        sequence: u64,
        last_frame: u64,
        last_timestamp: u64,
        closing: bool,
    }
    impl StreamProcess {
        /// Clone of the exact endpoint pool that owns inbound pipe buffers.
        /// Downstream decode/display/transport copies must reserve from it.
        pub fn budget(&self) -> SessionBudget {
            self.budget.clone()
        }
        pub fn open(
            helper: Arc<PinnedStreamHelper>,
            request: StreamRequest,
            cancel: &Cancellation,
        ) -> Result<Self> {
            let budget = SessionBudget::new(request.endpoint_bytes).map_err(stream_error)?;
            Self::open_budgeted(helper, request, budget, cancel)
        }
        /// Root supplies the exact actual parented endpoint pool. The helper
        /// has its own process limit from request.endpoint_bytes; host pipe,
        /// parser and retained packet allocations all use this injected pool.
        pub fn open_budgeted(
            helper: Arc<PinnedStreamHelper>,
            request: StreamRequest,
            budget: SessionBudget,
            cancel: &Cancellation,
        ) -> Result<Self> {
            request.validate()?;
            if budget.limit_bytes() > request.endpoint_bytes {
                return Err(Error::Invalid);
            }
            cancel.check().map_err(stream_error)?;
            ACTIVE
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| Error::Busy)?;
            let slot = Slot;
            let request_charge = budget
                .reserve(MemoryKind::Foreign, MAX_HEADER_BYTES as u64)
                .map_err(stream_error)?;
            let job = Job::new(true)?;
            let mut launch = OsCommand::new(&helper.path);
            launch
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .env_clear();
            use std::os::windows::process::CommandExt;
            launch.creation_flags(CREATE_SUSPENDED_FLAG | CREATE_NO_WINDOW_FLAG);
            let child = launch.spawn().map_err(|_| Error::Platform)?;
            let mut owner = Self {
                owned: Some(Owned {
                    child,
                    job,
                    assigned: false,
                    sender: None,
                    receiver: None,
                    worker: None,
                }),
                slot: Some(slot),
                helper,
                budget,
                request,
                _request_charge: request_charge,
                sequence: 0,
                last_frame: 0,
                last_timestamp: 0,
                closing: false,
            };
            let setup = (|| -> Result<()> {
                let active = owner.owned.as_mut().ok_or(Error::Platform)?;
                active.job.assign(&active.child)?;
                active.assigned = true;
                resume_primary(active.child.id())?;
                let mut input = active.child.stdin.take().ok_or(Error::Platform)?;
                let mut output = active.child.stdout.take().ok_or(Error::Platform)?;
                let (sender, commands) = mpsc::sync_channel::<Queued>(1);
                let (replies, receiver) = mpsc::sync_channel::<Result<Packet>>(1);
                let io_budget = owner.budget.clone();
                let worker = thread::Builder::new()
                    .name("vw-stream-pipe".into())
                    .spawn(move || {
                        let wire_cancel = Cancellation::default();
                        while let Ok(queued) = commands.recv() {
                            let result = stream_wire::write_command(
                                &mut input,
                                &queued.command,
                                &io_budget,
                                &wire_cancel,
                            )
                            .and_then(|()| {
                                stream_wire::read_packet(&mut output, &io_budget, &wire_cancel)
                            });
                            let failed = result.is_err();
                            if replies.send(result).is_err() || failed {
                                break;
                            }
                        }
                    })
                    .map_err(|_| Error::Platform)?;
                active.sender = Some(sender);
                active.receiver = Some(receiver);
                active.worker = Some(worker);
                Ok(())
            })();
            if let Err(error) = setup {
                let _ = owner.close();
                return Err(error);
            }
            let opening = Command::Open {
                request: owner.request.clone(),
            };
            let expected = stream_wire::command_hash(&opening, &owner.budget, cancel)?;
            let ready = owner.exchange_inner(opening, cancel, 5_000)?;
            match ready.header {
                Header::Ready {
                    request_hash,
                    connection_epoch,
                    source_generation,
                    ring_frames,
                    ring_bytes,
                    endpoint_bytes,
                    border_visible,
                } if request_hash == expected
                    && connection_epoch == owner.request.connection_epoch
                    && source_generation == owner.request.source_generation
                    && ring_frames == STREAM_RING_FRAMES
                    && ring_bytes == owner.request.ring_bytes
                    && endpoint_bytes == owner.request.endpoint_bytes
                    && border_visible
                    && ready.payload.bytes().is_empty() =>
                {
                    Ok(owner)
                }
                Header::Refused { error, .. } => {
                    let _ = owner.close();
                    Err(error)
                }
                _ => {
                    let _ = owner.close();
                    Err(Error::Invalid)
                }
            }
        }
        fn exchange_inner(
            &mut self,
            command: Command,
            cancel: &Cancellation,
            timeout_ms: u64,
        ) -> Result<Packet> {
            if self.closing {
                return Err(Error::Unavailable);
            }
            if cancel.check().is_err() {
                let _ = self.close();
                return Err(Error::Cancelled);
            }
            let charge = self
                .budget
                .reserve(MemoryKind::Transfer, MAX_HEADER_BYTES as u64)
                .map_err(stream_error)?;
            self.owned
                .as_ref()
                .ok_or(Error::Unavailable)?
                .sender
                .as_ref()
                .ok_or(Error::Unavailable)?
                .try_send(Queued {
                    command,
                    _charge: charge,
                })
                .map_err(|_| Error::Busy)?;
            let started = Instant::now();
            loop {
                if cancel.check().is_err() {
                    let _ = self.close();
                    return Err(Error::Cancelled);
                }
                if started.elapsed() >= Duration::from_millis(timeout_ms) {
                    let _ = self.close();
                    return Err(Error::Timeout);
                }
                let received = self
                    .owned
                    .as_ref()
                    .ok_or(Error::Unavailable)?
                    .receiver
                    .as_ref()
                    .ok_or(Error::Unavailable)?
                    .recv_timeout(Duration::from_millis(5));
                match received {
                    Ok(Ok(packet)) => return Ok(packet),
                    Ok(Err(error)) => {
                        let _ = self.close();
                        return Err(error);
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        let _ = self.close();
                        return Err(Error::Platform);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => (),
                }
            }
        }
        pub fn exchange(&mut self, command: Command, cancel: &Cancellation) -> Result<Packet> {
            let sequence = command.sequence().ok_or(Error::Invalid)?;
            if sequence == 0 || sequence <= self.sequence {
                return Err(Error::Invalid);
            }
            if command.binding()
                != Some((
                    self.request.connection_epoch,
                    self.request.source_generation,
                ))
            {
                return Err(Error::Stale);
            }
            self.sequence = sequence;
            let timeout = if matches!(command, Command::Original { .. }) {
                10_000
            } else {
                5_000
            };
            let packet = self.exchange_inner(command.clone(), cancel, timeout)?;
            if let Err(error) =
                super::validate_reply(&self.request, &command, &packet, &self.budget)
            {
                let _ = self.close();
                return Err(error);
            }
            if let Header::Tiles {
                frame_id,
                timestamp_ns,
                ..
            } = &packet.header
            {
                if *frame_id != self.last_frame.saturating_add(1)
                    || *timestamp_ns <= self.last_timestamp
                {
                    let _ = self.close();
                    return Err(Error::Stale);
                }
                self.last_frame = *frame_id;
                self.last_timestamp = *timestamp_ns;
            }
            if let Header::Refused { error, .. } = &packet.header
                && (*error != Error::Unavailable || !matches!(command, Command::Original { .. }))
            {
                let _ = self.close();
                return Err(*error);
            }
            if matches!(&packet.header, Header::Stopped { .. }) {
                self.close()?;
            }
            Ok(packet)
        }
        pub fn close(&mut self) -> Result<()> {
            self.closing = true;
            let Some(active) = self.owned.as_mut() else {
                return Ok(());
            };
            active.settle()?;
            self.owned.take();
            self.slot.take();
            Ok(())
        }
        pub fn retirement_pending(&self) -> bool {
            self.closing && self.owned.is_some()
        }
    }
    impl Drop for StreamProcess {
        fn drop(&mut self) {
            if self.close().is_ok() {
                return;
            }
            if let (Some(owned), Some(slot)) = (self.owned.take(), self.slot.take()) {
                let retained = Retained {
                    owned,
                    _slot: slot,
                    _helper: self.helper.clone(),
                };
                match unresolved().lock() {
                    Ok(mut guard) => guard.push(retained),
                    Err(_) => {
                        let _ = Box::leak(Box::new(retained));
                    }
                }
            }
        }
    }

    /// The helper applies a second process-memory cap; the parent Job is the
    /// authoritative kill-on-close owner and never includes the editor itself.
    pub fn confine_stream_helper() -> Result<impl Drop> {
        let job = Job::new(false)?;
        // SAFETY: this private helper assigns only its own current process.
        unsafe {
            AssignProcessToJobObject(job.0.raw(), GetCurrentProcess())
                .map_err(|_| Error::Platform)?;
        }
        Ok(job.0)
    }
}
#[cfg(windows)]
pub use owned::{
    PinnedStreamHelper, StreamProcess, confine_stream_helper, retry_unresolved_retirement,
};

#[cfg(not(windows))]
pub struct PinnedStreamHelper;
#[cfg(not(windows))]
impl PinnedStreamHelper {
    pub fn open(_path: &std::path::Path, _trusted_sha256: &str) -> Result<Self> {
        Err(Error::Unsupported)
    }
}
#[cfg(not(windows))]
pub struct StreamProcess;
#[cfg(not(windows))]
pub fn retry_unresolved_retirement() -> Result<()> {
    Err(Error::Unsupported)
}

#[cfg(test)]
mod preflight_tests {
    use super::*;
    #[test]
    fn repeated_or_inflated_protobuf_fields_refuse_before_decode() {
        assert!(matches!(
            preflight_frame_tiles(&[0x10, 1, 0x10, 2]),
            Err(Error::Invalid)
        ));
        let mut too_many_tiles = Vec::new();
        for _ in 0..=vw_stream::MAX_TILES {
            too_many_tiles.extend_from_slice(&[0x3a, 0]);
        }
        assert!(matches!(
            preflight_frame_tiles(&too_many_tiles),
            Err(Error::Limit)
        ));
        assert!(matches!(
            preflight_frame_tiles(&[0x1a, 2, 0x40, 1]),
            Err(Error::Invalid)
        ));
    }
    #[test]
    fn full_tile_grid_does_not_trip_the_wire_field_ceiling() {
        let tile = [
            0x0a, 8, 0x08, 1, 0x10, 1, 0x18, 1, 0x20, 1, 0x12, 4, b'j', b'p', b'e', b'g', 0x1a, 0,
        ];
        let mut frame = vec![0x10, 1]; // frame ID plus 128 fully populated tiles
        for _ in 0..vw_stream::MAX_TILES {
            frame.extend_from_slice(&[0x3a, tile.len() as u8]);
            frame.extend_from_slice(&tile);
        }
        assert!(preflight_frame_tiles(&frame).is_ok());
    }
}
