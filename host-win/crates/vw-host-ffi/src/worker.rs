use crate::{
    ClipboardImageFormat, ClipboardImageReceipt, ClipboardImport, ClipboardReadFormat,
    ClipboardReadOptions, ClipboardReceipt, DibOptions, DpiInfo, DragCleanupReceipt, DragFile,
    ExportBinding, HostError, HostOperation, HostOperationStatus, HostResult, WindowDpiInfo,
    operation::CancelOnDrop,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

pub(crate) const MAX_PNG_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;
const MAX_QUEUE: usize = 8;
const MAX_WORKERS: usize = 2;
const REQUEST_LIMIT: Duration = Duration::from_secs(30);
static LIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);

pub(crate) trait Backend {
    fn dpi_at_point(&mut self, x: i32, y: i32) -> HostResult<DpiInfo>;
    fn window_dpi_info(&mut self, window: u64) -> HostResult<WindowDpiInfo>;
    fn publish_png(&mut self, bytes: &[u8], context: &RequestContext) -> HostResult<u32>;
    fn publish_image(
        &mut self,
        png: &[u8],
        dib: &[u8],
        context: &RequestContext,
    ) -> HostResult<u32>;
    fn read_image(
        &mut self,
        format: ClipboardReadFormat,
        context: &RequestContext,
    ) -> HostResult<ClipboardSnapshot>;
    fn pump_messages(&mut self);
}

pub(crate) struct ClipboardSnapshot {
    pub bytes: Vec<u8>,
    pub format: ClipboardImageFormat,
    pub sequence: u32,
}

struct Shared {
    closed: AtomicBool,
    pending_bytes: AtomicUsize,
}
impl Shared {
    fn check_open(&self) -> HostResult<()> {
        if self.closed.load(Ordering::Acquire) {
            Err(HostError::Closed)
        } else {
            Ok(())
        }
    }
}

struct WorkerPermit;
impl WorkerPermit {
    fn acquire() -> HostResult<Self> {
        LIVE_WORKERS
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_WORKERS).then_some(count + 1)
            })
            .map(|_| Self)
            .map_err(|_| HostError::Busy)
    }
}
impl Drop for WorkerPermit {
    fn drop(&mut self) {
        LIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}

struct BytePermit {
    shared: Arc<Shared>,
    bytes: usize,
}
impl BytePermit {
    fn acquire(shared: &Arc<Shared>, bytes: usize) -> HostResult<Self> {
        shared
            .pending_bytes
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count
                    .checked_add(bytes)
                    .filter(|next| *next <= MAX_PENDING_BYTES)
            })
            .map_err(|_| HostError::Busy)?;
        Ok(Self {
            shared: shared.clone(),
            bytes,
        })
    }
}
impl Drop for BytePermit {
    fn drop(&mut self) {
        self.shared
            .pending_bytes
            .fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

// Completed bounded replies keep their reservation until the awaiting caller
// actually takes ownership. An unpolled or cancelled result remains accounted
// for; field order destroys a discarded result before releasing its permit.
struct BudgetedReply<T> {
    result: HostResult<T>,
    bytes: BytePermit,
}
impl<T> BudgetedReply<T> {
    fn take(self) -> HostResult<T> {
        let Self { result, bytes } = self;
        drop(bytes);
        result
    }
}

pub(crate) struct RequestContext {
    shared: Arc<Shared>,
    operation: Arc<HostOperation>,
    deadline: Instant,
}
impl RequestContext {
    pub(crate) fn checkpoint(&self) -> HostResult<()> {
        self.shared.check_open()?;
        if self.operation.status() == HostOperationStatus::Cancelled {
            return Err(HostError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(HostError::Timeout);
        }
        Ok(())
    }
    /// Called only after the platform owns all resources and has opened the
    /// clipboard. This is the cancellation commit point, before EmptyClipboard.
    pub(crate) fn begin_publication(&self) -> HostResult<()> {
        self.checkpoint()?;
        self.operation.advance(HostOperationStatus::Publishing)
    }
}

enum Command {
    Point {
        x: i32,
        y: i32,
        reply: oneshot::Sender<HostResult<DpiInfo>>,
    },
    Window {
        window: u64,
        reply: oneshot::Sender<HostResult<WindowDpiInfo>>,
    },
    Clipboard {
        png: Vec<u8>,
        context: RequestContext,
        reply: oneshot::Sender<BudgetedReply<ClipboardReceipt>>,
        _bytes: BytePermit,
    },
    ClipboardImage {
        png: Vec<u8>,
        binding: ExportBinding,
        options: DibOptions,
        context: RequestContext,
        reply: oneshot::Sender<BudgetedReply<ClipboardImageReceipt>>,
        _bytes: BytePermit,
    },
    ClipboardRead {
        options: ClipboardReadOptions,
        context: RequestContext,
        reply: oneshot::Sender<BudgetedReply<ClipboardImport>>,
        _bytes: BytePermit,
    },
    DragStage {
        source: PathBuf,
        binding: ExportBinding,
        context: RequestContext,
        reply: oneshot::Sender<HostResult<Arc<DragFile>>>,
    },
    DragRelease {
        id: String,
        reply: oneshot::Sender<HostResult<()>>,
    },
    DragCleanup {
        reply: oneshot::Sender<HostResult<DragCleanupReceipt>>,
    },
}
impl Command {
    fn reject(self, error: HostError) {
        match self {
            Self::Point { reply, .. } => {
                let _ = reply.send(Err(error));
            }
            Self::Window { reply, .. } => {
                let _ = reply.send(Err(error));
            }
            Self::Clipboard {
                png,
                context,
                reply,
                _bytes,
                ..
            } => {
                context.operation.finish(false);
                drop(png);
                let _ = reply.send(BudgetedReply {
                    result: Err(error),
                    bytes: _bytes,
                });
            }
            Self::ClipboardImage {
                png,
                context,
                reply,
                _bytes,
                ..
            } => {
                context.operation.finish(false);
                drop(png);
                let _ = reply.send(BudgetedReply {
                    result: Err(error),
                    bytes: _bytes,
                });
            }
            Self::ClipboardRead {
                context,
                reply,
                _bytes,
                ..
            } => {
                context.operation.finish(false);
                let _ = reply.send(BudgetedReply {
                    result: Err(error),
                    bytes: _bytes,
                });
            }
            Self::DragStage { context, reply, .. } => {
                context.operation.finish(false);
                let _ = reply.send(Err(error));
            }
            Self::DragRelease { reply, .. } => {
                let _ = reply.send(Err(error));
            }
            Self::DragCleanup { reply } => {
                let _ = reply.send(Err(error));
            }
        }
    }
    fn execute(
        self,
        backend: &mut impl Backend,
        drag: &mut Option<crate::drag::DragStore>,
        base: &Path,
    ) {
        match self {
            Self::Point { x, y, reply } => {
                if !reply.is_closed() {
                    let _ = reply.send(backend.dpi_at_point(x, y));
                }
            }
            Self::Window { window, reply } => {
                if !reply.is_closed() {
                    let _ = reply.send(backend.window_dpi_info(window));
                }
            }
            Self::Clipboard {
                png,
                context,
                reply,
                _bytes,
            } => {
                let result = prepare_and_publish(&png, &context, backend);
                context.operation.finish(result.is_ok());
                drop(png);
                let _ = reply.send(BudgetedReply {
                    result,
                    bytes: _bytes,
                });
            }
            Self::ClipboardImage {
                png,
                binding,
                options,
                context,
                reply,
                _bytes,
            } => {
                let result = prepare_image(&png, binding, options, &context, backend);
                context.operation.finish(result.is_ok());
                drop(png);
                let _ = reply.send(BudgetedReply {
                    result,
                    bytes: _bytes,
                });
            }
            Self::ClipboardRead {
                options,
                context,
                reply,
                _bytes,
            } => {
                let result = read_image(options, &context, backend);
                context.operation.finish(result.is_ok());
                let _ = reply.send(BudgetedReply {
                    result,
                    bytes: _bytes,
                });
            }
            Self::DragStage {
                source,
                binding,
                context,
                reply,
            } => {
                let result = (|| {
                    context.checkpoint()?;
                    context.operation.advance(HostOperationStatus::Validating)?;
                    drag_store(drag, base)?.stage(&source, binding, &context)
                })();
                context.operation.finish(result.is_ok());
                let _ = reply.send(result);
            }
            Self::DragRelease { id, reply } => {
                let result = drag
                    .as_mut()
                    .ok_or(HostError::UnknownDragLease)
                    .and_then(|store| store.release(&id));
                let _ = reply.send(result);
            }
            Self::DragCleanup { reply } => {
                let result =
                    drag_store(drag, base).and_then(|store| store.cleanup(crate::drag::unix_ms()?));
                let _ = reply.send(result);
            }
        }
    }
}

fn drag_store<'a>(
    store: &'a mut Option<crate::drag::DragStore>,
    base: &Path,
) -> HostResult<&'a mut crate::drag::DragStore> {
    if store.is_none() {
        *store = Some(crate::drag::DragStore::new(base)?);
    }
    store.as_mut().ok_or(HostError::HandoffStorage)
}

fn prepare_image(
    png: &[u8],
    binding: ExportBinding,
    options: DibOptions,
    context: &RequestContext,
    backend: &mut impl Backend,
) -> HostResult<ClipboardImageReceipt> {
    context.checkpoint()?;
    context.operation.advance(HostOperationStatus::Validating)?;
    if png.len() > MAX_PNG_BYTES {
        return Err(HostError::SizeLimit);
    }
    let declaration = crate::binding::bytes(png, &binding)?;
    let image = crate::dib::decode_png(png)?;
    if image.width != declaration.width
        || image.height != declaration.height
        || image.pixels.bit_depth() != declaration.bit_depth
    {
        return Err(HostError::ExportBindingMismatch);
    }
    let (width, height, bit_depth) = (image.width, image.height, image.pixels.bit_depth());
    let dib = crate::dib::prepare(&image, options)?;
    drop(image);
    context.checkpoint()?;
    context
        .operation
        .advance(HostOperationStatus::WaitingForClipboard)?;
    let sequence = backend.publish_image(png, &dib.bytes, context)?;
    Ok(ClipboardImageReceipt {
        image: ClipboardReceipt {
            width,
            height,
            bit_depth,
            png_bytes: png.len() as u64,
            clipboard_sequence: sequence,
            png_format_published: true,
        },
        dibv5_bytes: dib.bytes.len() as u64,
        dib_depth_reduced: dib.depth_reduced,
        dib_assumed_srgb: dib.assumed_srgb,
        binding,
    })
}

fn read_image(
    options: ClipboardReadOptions,
    context: &RequestContext,
    backend: &mut impl Backend,
) -> HostResult<ClipboardImport> {
    context.checkpoint()?;
    context.operation.advance(HostOperationStatus::Validating)?;
    context
        .operation
        .advance(HostOperationStatus::WaitingForClipboard)?;
    let snapshot = backend.read_image(options.format, context)?;
    context.checkpoint()?;
    let limit = if snapshot.format == ClipboardImageFormat::Png {
        MAX_PNG_BYTES
    } else {
        crate::dib::MAX_DIB_BYTES
    };
    if snapshot.bytes.len() > limit {
        return Err(HostError::SizeLimit);
    }
    let original_payload_blake3 = blake3::hash(&snapshot.bytes).to_hex().to_string();
    let (png, width, height, bit_depth, assumed_srgb) = match snapshot.format {
        ClipboardImageFormat::Png => {
            let image = crate::dib::decode_png(&snapshot.bytes)?;
            let dimensions = (image.width, image.height, image.pixels.bit_depth());
            drop(image);
            (
                snapshot.bytes,
                dimensions.0,
                dimensions.1,
                dimensions.2,
                false,
            )
        }
        ClipboardImageFormat::Dib | ClipboardImageFormat::DibV5 => {
            if snapshot.format == ClipboardImageFormat::DibV5
                && snapshot.bytes.get(..4) != Some(&124u32.to_le_bytes())
            {
                return Err(HostError::InvalidDib);
            }
            let image = crate::dib::import(&snapshot.bytes, options.assume_untagged_srgb)?;
            (image.png, image.width, image.height, 8, image.assumed_srgb)
        }
    };
    context.checkpoint()?;
    let png_blake3 = blake3::hash(&png).to_hex().to_string();
    Ok(ClipboardImport {
        png,
        format: snapshot.format,
        width,
        height,
        bit_depth,
        clipboard_sequence: snapshot.sequence,
        original_payload_blake3,
        png_blake3,
        assumed_srgb,
    })
}

fn prepare_and_publish(
    png: &[u8],
    context: &RequestContext,
    backend: &mut impl Backend,
) -> HostResult<ClipboardReceipt> {
    context.checkpoint()?;
    context.operation.advance(HostOperationStatus::Validating)?;
    // Real decoding detects truncated IDAT, invalid samples, ICC and EXIF before
    // the OS clipboard is touched. No arbitrary file or network access is used.
    let image = crate::dib::decode_png(png)?;
    let (width, height, bit_depth) = (image.width, image.height, image.pixels.bit_depth());
    drop(image);
    context.checkpoint()?;
    context
        .operation
        .advance(HostOperationStatus::WaitingForClipboard)?;
    let clipboard_sequence = backend.publish_png(png, context)?;
    Ok(ClipboardReceipt {
        width,
        height,
        bit_depth,
        png_bytes: png.len() as u64,
        clipboard_sequence,
        png_format_published: true,
    })
}

#[derive(uniffi::Object)]
pub struct HostService {
    sender: SyncSender<Command>,
    shared: Arc<Shared>,
    completion: Mutex<Option<oneshot::Receiver<()>>>,
}

#[uniffi::export]
impl HostService {
    /// Explicit user Copy only. Preserves exact revision-bound PNG bytes and
    /// prepares the DIBV5 companion before changing any clipboard contents.
    pub async fn set_clipboard_image(
        &self,
        png: Vec<u8>,
        binding: ExportBinding,
        options: DibOptions,
        operation: Arc<HostOperation>,
    ) -> HostResult<ClipboardImageReceipt> {
        operation.begin()?;
        let _cancel = CancelOnDrop(operation.clone());
        let result = async {
            self.shared.check_open()?;
            crate::binding::validate(&binding)?;
            if png.len() > MAX_PNG_BYTES {
                return Err(HostError::SizeLimit);
            }
            let bytes = BytePermit::acquire(&self.shared, png.len())?;
            let (reply, received) = oneshot::channel();
            self.submit(Command::ClipboardImage {
                png,
                binding,
                options,
                context: self.context(operation.clone()),
                reply,
                _bytes: bytes,
            })?;
            self.response_budgeted(received).await
        }
        .await;
        if result.is_err() {
            operation.finish(false);
        }
        result
    }

    /// Reads only after explicit Paste. Does not enumerate text, monitor changes,
    /// restore old clipboard data, or replace the clipboard during normalization.
    pub async fn read_clipboard_image(
        &self,
        options: ClipboardReadOptions,
        operation: Arc<HostOperation>,
    ) -> HostResult<ClipboardImport> {
        operation.begin()?;
        let _cancel = CancelOnDrop(operation.clone());
        let result = async {
            self.shared.check_open()?;
            let bytes = BytePermit::acquire(&self.shared, crate::dib::MAX_DIB_BYTES)?;
            let (reply, received) = oneshot::channel();
            self.submit(Command::ClipboardRead {
                options,
                context: self.context(operation.clone()),
                reply,
                _bytes: bytes,
            })?;
            self.response_budgeted(received).await
        }
        .await;
        if result.is_err() {
            operation.finish(false);
        }
        result
    }

    /// Only completed private core exports are accepted. The native file is held
    /// by a lease until release_drag_file or service shutdown; neither deletes it
    /// before its retention deadline. Desktop initiates the actual drag gesture.
    pub async fn stage_drag_png(
        &self,
        source_path: String,
        binding: ExportBinding,
        operation: Arc<HostOperation>,
    ) -> HostResult<Arc<DragFile>> {
        operation.begin()?;
        let _cancel = CancelOnDrop(operation.clone());
        let result = async {
            self.shared.check_open()?;
            crate::binding::validate(&binding)?;
            if source_path.len() > 32767
                || source_path.contains('\0')
                || !Path::new(&source_path).is_absolute()
            {
                return Err(HostError::HandoffStorage);
            }
            let (reply, received) = oneshot::channel();
            self.submit(Command::DragStage {
                source: PathBuf::from(source_path),
                binding,
                context: self.context(operation.clone()),
                reply,
            })?;
            self.response(received).await
        }
        .await;
        if result.is_err() {
            operation.finish(false);
        }
        result
    }

    pub async fn release_drag_file(&self, lease_id: String) -> HostResult<()> {
        if lease_id.len() != 32 {
            return Err(HostError::UnknownDragLease);
        }
        let (reply, received) = oneshot::channel();
        self.submit(Command::DragRelease {
            id: lease_id,
            reply,
        })?;
        self.response(received).await
    }

    pub async fn cleanup_drag_files(&self) -> HostResult<DragCleanupReceipt> {
        let (reply, received) = oneshot::channel();
        self.submit(Command::DragCleanup { reply })?;
        self.response(received).await
    }

    pub async fn dpi_at_point(&self, x: i32, y: i32) -> HostResult<DpiInfo> {
        let (reply, received) = oneshot::channel();
        self.submit(Command::Point { x, y, reply })?;
        self.response(received).await
    }

    pub async fn window_dpi_info(&self, window_handle: u64) -> HostResult<WindowDpiInfo> {
        if window_handle == 0 {
            return Err(HostError::InvalidWindow);
        }
        let (reply, received) = oneshot::channel();
        self.submit(Command::Window {
            window: window_handle,
            reply,
        })?;
        self.response(received).await
    }

    /// Publishes exactly these PNG bytes in the Windows registered PNG format.
    /// Use set_clipboard_image for the separately prepared DIBV5 companion.
    pub async fn set_clipboard_png(
        &self,
        png: Vec<u8>,
        operation: Arc<HostOperation>,
    ) -> HostResult<ClipboardReceipt> {
        operation.begin()?;
        let _cancel = CancelOnDrop(operation.clone());
        let result = async {
            self.shared.check_open()?;
            if png.len() > MAX_PNG_BYTES {
                return Err(HostError::SizeLimit);
            }
            let bytes = BytePermit::acquire(&self.shared, png.len())?;
            let (reply, received) = oneshot::channel();
            let context = RequestContext {
                shared: self.shared.clone(),
                operation: operation.clone(),
                deadline: Instant::now() + REQUEST_LIMIT,
            };
            self.submit(Command::Clipboard {
                png,
                context,
                reply,
                _bytes: bytes,
            })?;
            self.response_budgeted(received).await
        }
        .await;
        if result.is_err() {
            operation.finish(false);
        }
        result
    }

    /// Requests shutdown, then awaits release of worker-owned windows and queued
    /// PNG buffers. Call once before releasing the generated Kotlin object.
    pub async fn shutdown(&self) -> HostResult<()> {
        let completion = self
            .completion
            .try_lock()
            .map_err(|_| HostError::Busy)?
            .take()
            .ok_or(HostError::Closed)?;
        self.shared.closed.store(true, Ordering::Release);
        completion.await.map_err(|_| HostError::WorkerUnavailable)
    }
}
impl HostService {
    fn context(&self, operation: Arc<HostOperation>) -> RequestContext {
        RequestContext {
            shared: self.shared.clone(),
            operation,
            deadline: Instant::now() + REQUEST_LIMIT,
        }
    }
    fn submit(&self, command: Command) -> HostResult<()> {
        self.shared.check_open()?;
        self.sender.try_send(command).map_err(|error| match error {
            TrySendError::Full(_) => HostError::Busy,
            TrySendError::Disconnected(_) => HostError::WorkerUnavailable,
        })
    }
    async fn response_budgeted<T>(
        &self,
        receiver: oneshot::Receiver<BudgetedReply<T>>,
    ) -> HostResult<T> {
        let reply = receiver.await.map_err(|_| {
            if self.shared.closed.load(Ordering::Acquire) {
                HostError::Closed
            } else {
                HostError::WorkerUnavailable
            }
        })?;
        reply.take()
    }
    async fn response<T>(&self, receiver: oneshot::Receiver<HostResult<T>>) -> HostResult<T> {
        receiver.await.map_err(|_| {
            if self.shared.closed.load(Ordering::Acquire) {
                HostError::Closed
            } else {
                HostError::WorkerUnavailable
            }
        })?
    }
}
impl Drop for HostService {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
    }
}

pub(crate) async fn start() -> HostResult<Arc<HostService>> {
    start_with(crate::platform::Native::new).await
}

pub(crate) async fn start_with<B: Backend + 'static>(
    factory: impl FnOnce() -> HostResult<B> + Send + 'static,
) -> HostResult<Arc<HostService>> {
    start_with_root(factory, std::env::temp_dir()).await
}

pub(crate) async fn start_with_root<B: Backend + 'static>(
    factory: impl FnOnce() -> HostResult<B> + Send + 'static,
    drag_base: PathBuf,
) -> HostResult<Arc<HostService>> {
    let permit = WorkerPermit::acquire()?;
    let shared = Arc::new(Shared {
        closed: AtomicBool::new(false),
        pending_bytes: AtomicUsize::new(0),
    });
    let (sender, receiver) = mpsc::sync_channel(MAX_QUEUE);
    let (ready, initialized) = oneshot::channel();
    let (done, completion) = oneshot::channel();
    let service = Arc::new(HostService {
        sender,
        shared: shared.clone(),
        completion: Mutex::new(Some(completion)),
    });
    std::thread::Builder::new()
        .name("vw-host-services".into())
        .spawn(move || {
            match factory() {
                Ok(mut backend) => {
                    if ready.send(Ok(())).is_ok() {
                        let mut drag = None;
                        run(&receiver, &shared, &mut backend, &mut drag, &drag_base);
                    }
                    // Window handles are destroyed on their creating thread, before
                    // notifying shutdown. No JoinHandle::join blocks a Kotlin poll.
                    drop(backend);
                }
                Err(error) => {
                    let _ = ready.send(Err(error));
                }
            }
            for command in receiver.try_iter() {
                command.reject(HostError::Closed);
            }
            drop(receiver);
            drop(permit);
            let _ = done.send(());
        })
        .map_err(|_| HostError::WorkerUnavailable)?;
    initialized
        .await
        .map_err(|_| HostError::WorkerUnavailable)??;
    Ok(service)
}

fn run(
    receiver: &Receiver<Command>,
    shared: &Shared,
    backend: &mut impl Backend,
    drag: &mut Option<crate::drag::DragStore>,
    base: &Path,
) {
    while !shared.closed.load(Ordering::Acquire) {
        if let Some(store) = drag {
            store.reap();
        }
        backend.pump_messages();
        match receiver.recv_timeout(Duration::from_millis(10)) {
            Ok(command) => {
                if shared.closed.load(Ordering::Acquire) {
                    command.reject(HostError::Closed);
                } else {
                    command.execute(backend, drag, base);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

#[cfg(test)]
pub(crate) fn live_workers() -> usize {
    LIVE_WORKERS.load(Ordering::Acquire)
}

#[cfg(test)]
pub(crate) fn handoff_test_context() -> HostResult<RequestContext> {
    let operation = HostOperation::new();
    operation.begin()?;
    operation.advance(HostOperationStatus::Validating)?;
    Ok(RequestContext {
        shared: Arc::new(Shared {
            closed: AtomicBool::new(false),
            pending_bytes: AtomicUsize::new(0),
        }),
        operation,
        deadline: Instant::now() + Duration::from_secs(5),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_byte_budget_releases_every_reservation() -> HostResult<()> {
        let state = Arc::new(Shared {
            closed: AtomicBool::new(false),
            pending_bytes: AtomicUsize::new(0),
        });
        let a = BytePermit::acquire(&state, MAX_PENDING_BYTES - 1)?;
        let b = BytePermit::acquire(&state, 1)?;
        assert!(matches!(
            BytePermit::acquire(&state, 1),
            Err(HostError::Busy)
        ));
        assert!(matches!(
            BytePermit::acquire(&state, usize::MAX),
            Err(HostError::Busy)
        ));
        drop(a);
        let c = BytePermit::acquire(&state, MAX_PENDING_BYTES - 1)?;
        drop(b);
        drop(c);
        assert_eq!(state.pending_bytes.load(Ordering::Acquire), 0);
        Ok(())
    }
    fn isolated_service() -> HostService {
        let (sender, _) = mpsc::sync_channel(MAX_QUEUE);
        HostService {
            sender,
            shared: Arc::new(Shared {
                closed: AtomicBool::new(false),
                pending_bytes: AtomicUsize::new(0),
            }),
            completion: Mutex::new(None),
        }
    }
    #[test]
    fn completed_bounded_read_releases_before_immediate_successor_admission() -> HostResult<()> {
        use std::{
            future::Future,
            task::{Context, Poll, Waker},
        };
        let host = isolated_service();
        let bytes = BytePermit::acquire(&host.shared, MAX_PENDING_BYTES)?;
        let (sender, receiver) = oneshot::channel();
        let mut response = Box::pin(host.response_budgeted(receiver));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(response.as_mut().poll(&mut cx), Poll::Pending));
        assert!(
            sender
                .send(BudgetedReply {
                    result: Err::<(), _>(HostError::InvalidPng),
                    bytes
                })
                .is_ok()
        );
        // A queued reply remains charged until the consumer actually polls it.
        assert!(matches!(
            BytePermit::acquire(&host.shared, MAX_PENDING_BYTES),
            Err(HostError::Busy)
        ));
        assert_eq!(
            response.as_mut().poll(&mut cx),
            Poll::Ready(Err(HostError::InvalidPng))
        );
        // No worker thread is allowed a scheduling turn between completion and reuse.
        let next = BytePermit::acquire(&host.shared, MAX_PENDING_BYTES)?;
        drop(next);
        assert_eq!(host.shared.pending_bytes.load(Ordering::Acquire), 0);
        Ok(())
    }
    struct ChargedResult(Arc<Shared>);
    impl Drop for ChargedResult {
        fn drop(&mut self) {
            assert_eq!(
                self.0.pending_bytes.load(Ordering::Acquire),
                MAX_PENDING_BYTES
            );
        }
    }
    #[test]
    fn abandoned_completed_result_drops_payload_before_byte_reservation() -> HostResult<()> {
        let host = isolated_service();
        let bytes = BytePermit::acquire(&host.shared, MAX_PENDING_BYTES)?;
        let (sender, receiver) = oneshot::channel();
        assert!(
            sender
                .send(BudgetedReply {
                    result: Ok(ChargedResult(host.shared.clone())),
                    bytes,
                })
                .is_ok()
        );
        assert!(matches!(
            BytePermit::acquire(&host.shared, 1),
            Err(HostError::Busy)
        ));
        drop(receiver);
        assert_eq!(host.shared.pending_bytes.load(Ordering::Acquire), 0);
        Ok(())
    }
    #[test]
    fn cancelled_receiver_keeps_payload_charged_until_failed_send_is_disposed() -> HostResult<()> {
        let host = isolated_service();
        let bytes = BytePermit::acquire(&host.shared, MAX_PENDING_BYTES)?;
        let (sender, receiver) = oneshot::channel();
        drop(receiver);
        let unsent = sender.send(BudgetedReply {
            result: Ok(ChargedResult(host.shared.clone())),
            bytes,
        });
        assert!(unsent.is_err());
        assert!(matches!(
            BytePermit::acquire(&host.shared, 1),
            Err(HostError::Busy)
        ));
        drop(unsent);
        assert_eq!(host.shared.pending_bytes.load(Ordering::Acquire), 0);
        Ok(())
    }
}
