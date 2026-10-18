//! Fixed one-job transport adapters. The Windows entry never loads PDFium into
//! the app. Android native parsing requires the OS-assigned isolated UID.
#[cfg(windows)]
use crate::CoreError;
#[cfg(windows)]
use crate::worker::startup;
#[cfg(windows)]
use std::io::{Seek, SeekFrom};
use std::sync::Arc;
#[cfg(any(windows, target_os = "android"))]
use std::{io::Read, path::Path};
use vw_document_protocol::{Operation, Request};

#[derive(Clone, uniffi::Record)]
pub struct DocumentJobResult {
    pub source_blake3: String,
    pub metadata_json: String,
    pub bytes: Vec<u8>,
    pub output_blake3: String,
}
#[derive(Clone, uniffi::Record)]
pub struct DocumentRuntimeArtifact {
    pub file_name: String,
    pub bytes: u64,
    pub sha256: Vec<u8>,
}
#[derive(Clone, uniffi::Record)]
pub struct DocumentRuntimeOptions {
    pub directory: String,
    pub artifacts: Vec<DocumentRuntimeArtifact>,
}
#[derive(Clone, Debug, thiserror::Error, uniffi::Error)]
pub enum DocumentJobError {
    #[error("document runtime is not admitted or unavailable")]
    Unavailable,
    #[error("invalid bounded document request")]
    Invalid,
    #[error("document request exceeded its resource limit")]
    Limit,
    #[error("document worker is busy")]
    Busy,
    #[error("document worker cancelled")]
    Cancelled,
    #[error("document worker timed out")]
    Timeout,
    #[error("document worker isolation or cleanup failed")]
    Isolation,
    #[error("document source/output identity failed")]
    Verification,
    #[error("document password is required or incorrect")]
    Password,
    #[error("unsupported document content; choose another export explicitly")]
    Unsupported,
    #[error("document is malformed")]
    Malformed,
    #[error("document worker failed")]
    Worker,
}
type JobResult<T> = std::result::Result<T, DocumentJobError>;
#[uniffi::export]
pub fn document_source_hash(source: Vec<u8>) -> JobResult<String> {
    if source.len() as u64 > 128 * 1024 * 1024 {
        return Err(DocumentJobError::Limit);
    }
    Ok(blake3::hash(&source).to_hex().to_string())
}
#[uniffi::export]
pub fn validate_document_job_result(
    request_json: String,
    expected_source_blake3: String,
    result: DocumentJobResult,
) -> JobResult<DocumentJobResult> {
    if request_json.len() > vw_document_protocol::MAX_METADATA
        || result.metadata_json.len() > vw_document_protocol::MAX_REPLY_METADATA
        || result.bytes.len() > 128 * 1024 * 1024
        || expected_source_blake3.len() != 64
        || result.source_blake3 != expected_source_blake3
        || blake3::hash(&result.bytes).to_hex().as_str() != result.output_blake3
    {
        return Err(DocumentJobError::Verification);
    }
    let request: Request =
        serde_json::from_str(&request_json).map_err(|_| DocumentJobError::Invalid)?;
    request.validate().map_err(|_| DocumentJobError::Invalid)?;
    if request.is_copy() {
        request
            .require_copy_identity()
            .map_err(|_| DocumentJobError::Invalid)?;
    }
    if request
        .export_identity
        .as_ref()
        .is_some_and(|identity| identity.source_asset != expected_source_blake3)
    {
        return Err(DocumentJobError::Verification);
    }
    let metadata: vw_document_win::ResultMetadata =
        serde_json::from_str(&result.metadata_json).map_err(|_| DocumentJobError::Verification)?;
    metadata
        .validate_for(&request)
        .map_err(|_| DocumentJobError::Verification)?;
    if metadata.data_bytes() != result.bytes.len() as u64 {
        return Err(DocumentJobError::Verification);
    }
    Ok(result)
}
fn request(text: &str, source: &[u8]) -> JobResult<Request> {
    if text.len() > vw_document_protocol::MAX_METADATA
        || source.is_empty()
        || source.len() as u64 > super::DOCUMENT_SOURCE_LIMIT
    {
        return Err(DocumentJobError::Limit);
    }
    let request: Request = serde_json::from_str(text).map_err(|_| DocumentJobError::Invalid)?;
    request.validate().map_err(|_| DocumentJobError::Invalid)?;
    if request.is_copy() {
        request
            .require_copy_identity()
            .map_err(|_| DocumentJobError::Invalid)?;
    }
    request
        .validate_source_identity(blake3::hash(source).as_bytes())
        .map_err(|_| DocumentJobError::Verification)?;
    if request.limits.input_bytes > super::DOCUMENT_SOURCE_LIMIT
        || request.limits.output_bytes > 128 * 1024 * 1024
        || request.limits.memory_bytes > 512 * 1024 * 1024
        || source.len() as u64 > request.limits.input_bytes
        || matches!(
            request.operation,
            Operation::SvgInfo | Operation::SvgTile { .. } | Operation::SvgCopy { .. }
        ) && source.len() > 4 * 1024 * 1024
    {
        return Err(DocumentJobError::Limit);
    }
    Ok(request)
}
#[derive(Default, uniffi::Object)]
pub struct DocumentJobCancellation {
    #[cfg(windows)]
    cancel: vw_document_win::Cancellation,
    flag: std::sync::atomic::AtomicBool,
}
#[uniffi::export]
impl DocumentJobCancellation {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self {
            #[cfg(windows)]
            cancel: vw_document_win::Cancellation::default(),
            flag: std::sync::atomic::AtomicBool::new(false),
        }
    }
    pub fn cancel(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::Release);
        #[cfg(windows)]
        self.cancel.cancel();
    }
}
#[derive(uniffi::Object)]
pub struct DesktopDocumentRuntime {
    #[cfg(windows)]
    runtime: Arc<vw_document_win::PinnedRuntime>,
    #[cfg(windows)]
    scratch: std::path::PathBuf,
}
#[uniffi::export]
pub async fn open_desktop_document_runtime(
    options: DocumentRuntimeOptions,
) -> JobResult<Arc<DesktopDocumentRuntime>> {
    if options.directory.len() > 32767
        || options.directory.contains('\0')
        || options.artifacts.is_empty()
        || options.artifacts.len() > 32
    {
        return Err(DocumentJobError::Invalid);
    }
    #[cfg(windows)]
    {
        startup(move || {
            let manifest = vw_document_win::RuntimeManifest {
                entries: options
                    .artifacts
                    .into_iter()
                    .map(|entry| {
                        Ok(vw_document_win::ArtifactEntry {
                            file_name: entry.file_name,
                            bytes: entry.bytes,
                            sha256: entry.sha256.try_into().map_err(|_| CoreError::Invalid)?,
                        })
                    })
                    .collect::<crate::Result<_>>()?,
            };
            let runtime = Arc::new(
                vw_document_win::PinnedRuntime::pin(Path::new(&options.directory), &manifest)
                    .map_err(|_| CoreError::Unsupported)?,
            );
            let scratch = runtime
                .private_workspace_path()
                .map_err(|_| CoreError::Unsupported)?;
            // Packaging creates the dedicated sibling. Never create/adopt it here.
            if runtime
                .workspace_cleanup_pending()
                .map_err(|_| CoreError::Unsupported)?
            {
                return Err(CoreError::Unsupported);
            }
            Ok(Arc::new(DesktopDocumentRuntime { runtime, scratch }))
        })
        .await
        .map_err(|_| DocumentJobError::Unavailable)
    }
    #[cfg(not(windows))]
    {
        let _ = options;
        Err(DocumentJobError::Unavailable)
    }
}
#[uniffi::export]
impl DesktopDocumentRuntime {
    /// Retry the process-owned quarantine; success also proves that this
    /// runtime has no retained crash marker. Never delete an orphan marker.
    pub async fn retry_retirement(&self) -> JobResult<()> {
        #[cfg(windows)]
        {
            let runtime = self.runtime.clone();
            startup(move || {
                Ok((|| -> JobResult<()> {
                    vw_document_win::retry_runtime_cleanup(&runtime).map_err(host_error)?;
                    if runtime.workspace_cleanup_pending().map_err(host_error)? {
                        return Err(DocumentJobError::Isolation);
                    }
                    Ok(())
                })())
            })
            .await
            .map_err(|_| DocumentJobError::Isolation)?
        }
        #[cfg(not(windows))]
        {
            Err(DocumentJobError::Unavailable)
        }
    }
    pub async fn run(
        &self,
        request_json: String,
        source: Vec<u8>,
        cancellation: Arc<DocumentJobCancellation>,
    ) -> JobResult<DocumentJobResult> {
        let request = request(&request_json, &source)?;
        #[cfg(windows)]
        {
            let runtime = self.runtime.clone();
            let scratch = self.scratch.clone();
            // startup admission is bounded globally; host also admits one worker.
            let job_cancel = cancellation.clone();
            startup(move || {
                Ok((|| -> JobResult<DocumentJobResult> {
                    let source_blake3 = blake3::hash(&source).to_hex().to_string();
                    let mut reply = vw_document_win::run(
                        runtime,
                        &scratch,
                        &request,
                        &source,
                        vw_document_win::WorkerLimits {
                            wall_time: std::time::Duration::from_secs(120),
                            process_memory_bytes: request.limits.memory_bytes,
                        },
                        &job_cancel.cancel,
                        &mut |_| Ok(()),
                        &mut |_| {},
                    )
                    .map_err(host_error)?;
                    let metadata_json = serde_json::to_string(&reply.metadata)
                        .map_err(|_| DocumentJobError::Worker)?;
                    let length = reply.metadata.data_bytes();
                    if length > request.limits.output_bytes {
                        return Err(DocumentJobError::Verification);
                    }
                    let mut bytes = Vec::new();
                    bytes
                        .try_reserve_exact(length as usize)
                        .map_err(|_| DocumentJobError::Limit)?;
                    reply
                        .private_file()
                        .seek(SeekFrom::Start(0))
                        .map_err(|_| DocumentJobError::Worker)?;
                    reply
                        .private_file()
                        .take(length + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|_| DocumentJobError::Worker)?;
                    if bytes.len() as u64 != length {
                        return Err(DocumentJobError::Verification);
                    }
                    let output_blake3 = blake3::hash(&bytes).to_hex().to_string();
                    Ok(DocumentJobResult {
                        source_blake3,
                        metadata_json,
                        bytes,
                        output_blake3,
                    })
                })())
            })
            .await
            .map_err(|error| {
                if matches!(error, CoreError::Backpressure) {
                    DocumentJobError::Busy
                } else if cancellation.flag.load(std::sync::atomic::Ordering::Acquire) {
                    DocumentJobError::Cancelled
                } else {
                    DocumentJobError::Worker
                }
            })?
        }
        #[cfg(not(windows))]
        {
            let _ = (request, cancellation);
            Err(DocumentJobError::Unavailable)
        }
    }
}

#[cfg(any(test, target_os = "android"))]
#[path = "engine.rs"]
mod engine;
/// Called only by the one-job isolated Android service. UID check is native and
/// precedes library loading, so an ordinary app-process caller is refused.
#[uniffi::export]
pub fn run_isolated_android_document_job(
    library: String,
    library_sha256: Vec<u8>,
    request_json: String,
    source: Vec<u8>,
) -> JobResult<DocumentJobResult> {
    #[cfg(target_os = "android")]
    {
        unsafe extern "C" {
            fn getuid() -> u32;
        }
        // SAFETY: libc getuid has no arguments/pointers; checks the actual process.
        let uid = unsafe { getuid() } % 100_000;
        if !(90_000..=99_999).contains(&uid) {
            return Err(DocumentJobError::Isolation);
        }
        let request = request(&request_json, &source)?;
        if matches!(request.operation, Operation::PdfPcRedaction { .. }) {
            return Err(DocumentJobError::Unsupported);
        }
        let svg = matches!(
            request.operation,
            Operation::SvgInfo | Operation::SvgTile { .. } | Operation::SvgCopy { .. }
        );
        // The OS-enforced process limit is installed only in this admitted
        // isolated UID, before any parser/library call. It can make a runtime
        // unable to allocate; that is a refusal, never a limit relaxation.
        use rustix::process::{Resource, Rlimit, setrlimit};
        for (resource, bound) in [
            (Resource::As, request.limits.memory_bytes),
            (Resource::Cpu, 120),
        ] {
            setrlimit(
                resource,
                Rlimit {
                    current: Some(bound),
                    maximum: Some(bound),
                },
            )
            .map_err(|_| DocumentJobError::Isolation)?;
        }
        if !svg {
            if library_sha256.len() != 32
                || !Path::new(&library).is_absolute()
                || Path::new(&library)
                    .file_name()
                    .and_then(|name| name.to_str())
                    != Some("libpdfium.so")
            {
                return Err(DocumentJobError::Unavailable);
            }
            use sha2::Digest;
            let file = std::fs::File::open(&library).map_err(|_| DocumentJobError::Unavailable)?;
            let size = file
                .metadata()
                .map_err(|_| DocumentJobError::Unavailable)?
                .len();
            if size == 0 || size > 128 * 1024 * 1024 {
                return Err(DocumentJobError::Limit);
            }
            let mut hash = sha2::Sha256::new();
            let mut reader = file.take(size + 1);
            let mut buffer = [0; 65536];
            let mut length = 0u64;
            loop {
                let count = reader
                    .read(&mut buffer)
                    .map_err(|_| DocumentJobError::Unavailable)?;
                if count == 0 {
                    break;
                }
                length += count as u64;
                if length > size {
                    return Err(DocumentJobError::Verification);
                }
                hash.update(&buffer[..count]);
            }
            if length != size || hash.finalize().as_slice() != library_sha256 {
                return Err(DocumentJobError::Verification);
            }
        } else if !library.is_empty() || !library_sha256.is_empty() {
            return Err(DocumentJobError::Invalid);
        }
        let mut nonce = [0u8; 16];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut random| random.read_exact(&mut nonce))
            .map_err(|_| DocumentJobError::Isolation)?;
        if nonce == [0; 16] {
            return Err(DocumentJobError::Isolation);
        }
        let binding = vw_document_protocol::JobBinding {
            nonce,
            source_blake3: *blake3::hash(&source).as_bytes(),
        };
        let source_blake3 = blake3::hash(&source).to_hex().to_string();
        let output = engine::execute(
            vw_document_protocol::Job {
                binding,
                request,
                source,
            },
            Path::new(&library),
        )
        .map_err(fault)?;
        let metadata_json =
            serde_json::to_string(&output.metadata).map_err(|_| DocumentJobError::Worker)?;
        if metadata_json.len() > vw_document_protocol::MAX_REPLY_METADATA {
            return Err(DocumentJobError::Limit);
        }
        let output_blake3 = blake3::hash(&output.bytes).to_hex().to_string();
        Ok(DocumentJobResult {
            source_blake3,
            metadata_json,
            bytes: output.bytes,
            output_blake3,
        })
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (library, library_sha256, request_json, source);
        Err(DocumentJobError::Unavailable)
    }
}
#[cfg(any(windows, target_os = "android"))]
fn fault(code: vw_document_protocol::FaultCode) -> DocumentJobError {
    use vw_document_protocol::FaultCode as F;
    match code {
        F::Invalid => DocumentJobError::Invalid,
        F::Limit | F::Memory => DocumentJobError::Limit,
        F::Cancelled => DocumentJobError::Cancelled,
        F::Password => DocumentJobError::Password,
        F::Unsupported => DocumentJobError::Unsupported,
        F::Malformed => DocumentJobError::Malformed,
        F::Verification => DocumentJobError::Verification,
        _ => DocumentJobError::Worker,
    }
}
#[cfg(windows)]
fn host_error(error: vw_document_win::Error) -> DocumentJobError {
    use vw_document_win::Error as E;
    match error {
        E::Invalid => DocumentJobError::Invalid,
        E::Limit | E::Output => DocumentJobError::Limit,
        E::Busy => DocumentJobError::Busy,
        E::Cancelled => DocumentJobError::Cancelled,
        E::Timeout => DocumentJobError::Timeout,
        E::Integrity => DocumentJobError::Verification,
        E::Isolation | E::CleanupPending => DocumentJobError::Isolation,
        E::Protocol(vw_document_protocol::ProtocolError::Worker(code)) => fault(code),
        _ => DocumentJobError::Worker,
    }
}
#[cfg(test)]
#[path = "jobs_tests.rs"]
mod tests;
