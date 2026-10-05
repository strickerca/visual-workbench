//! Harness-only outer WGC owner. Production capture and effect children unchanged.
use super::{Collection, collect_latest, locked_image};
use crate::{
    Error, Result,
    editor_source::{SourceBudget, SourceLease, SourcePolicy},
    platform,
    process::{
        Cancellation,
        windows::{Child, Pin, Pins},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use vw_remote::{
    Rect, Target,
    profile::{ImageIdentity, live::ToolSettings},
};
use windows::{
    Win32::System::{JobObjects::IsProcessInJob, Threading::GetCurrentProcess},
    core::BOOL,
};
#[path = "frame/metrics.rs"]
mod metrics;
#[path = "frame/paint.rs"]
mod paint;
const MAX_REQUEST: usize = 16 * 1024;
const MAX_OUTPUT: usize = 4 * 1024 * 1024;
static ACTIVE: AtomicBool = AtomicBool::new(false);
static RETAINED: OnceLock<Mutex<Vec<Pending>>> = OnceLock::new();
struct Permit;
impl Permit {
    fn acquire() -> Result<Self> {
        ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::Limit)?;
        Ok(Self)
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum SamplingScope {
    #[default]
    FullClient,
    VerifiedEditorCanvas,
}
impl SamplingScope {
    fn is_full(&self) -> bool {
        *self == Self::FullClient
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    schema: u32,
    owner_pid: u32,
    target: Target,
    image: ImageIdentity,
    source_policy: SourcePolicy,
    explicit_owner_capture: bool,
    owned_blank_fixture: bool,
    capture_helper_path: PathBuf,
    capture_helper_sha256: String,
    capture_session_id: String,
    private_temp_root: PathBuf,
    marker_nonce: String,
    sample_id: String,
    canvas_rect_host: Rect,
    #[serde(default)]
    sampling_scope: SamplingScope,
    #[serde(default)]
    expected_tool_settings: Option<ToolSettings>,
    #[serde(default)]
    expected_paint_frame: Option<platform::paint_controls::PaintFrameWitness>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Marker {
    schema: u32,
    owner_pid: u32,
    nonce: String,
}
struct TempLease {
    root: PathBuf,
    sample: PathBuf,
    ancestors: Vec<Pin>,
    marker: Pin,
    sample_pin: Pin,
}
fn sha(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}
/// Only the actual OS TEMP root may have one conventional terminal separator.
/// No general path canonicalization or component/dot/redirect normalization.
fn os_temp_root(path: &Path) -> Result<PathBuf> {
    let text = path.to_str().ok_or(Error::Invalid)?;
    if text.ends_with("\\\\") {
        return Err(Error::Invalid);
    }
    let text = if text.len() > 3 {
        text.strip_suffix('\\').unwrap_or(text)
    } else {
        text
    };
    let root = PathBuf::from(text);
    crate::process::windows::namespace(&root)?;
    Ok(root)
}
fn same_path(left: &Path, right: &Path) -> bool {
    left.to_str().zip(right.to_str()).is_some_and(|(a, b)| {
        a.trim_end_matches('\\')
            .eq_ignore_ascii_case(b.trim_end_matches('\\'))
    })
}
fn within(path: &Path, root: &Path) -> bool {
    path.to_str()
        .zip(root.to_str())
        .is_some_and(|(path, root)| {
            let path = path.to_ascii_lowercase();
            let root = root.to_ascii_lowercase();
            path == root
                || path
                    .strip_prefix(&root)
                    .is_some_and(|v| v.starts_with('\\'))
        })
}
fn temp_path(path: &Path, temp: &Path, workspace: &Path, nonce: &str) -> Result<()> {
    crate::process::windows::namespace(path)?;
    vw_remote::id(nonce)?;
    if !same_path(path.parent().ok_or(Error::Invalid)?, temp)
        || within(path, workspace)
        || path.file_name().and_then(|v| v.to_str())
            != Some(format!("VisualWorkbench-editor-metrics-{nonce}").as_str())
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
impl TempLease {
    fn open(r: &Request) -> Result<Self> {
        let temp = os_temp_root(&std::env::temp_dir())?;
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(4)
            .ok_or(Error::Invalid)?;
        temp_path(&r.private_temp_root, &temp, workspace, &r.marker_nonce)?;
        super::local(&r.private_temp_root)?;
        vw_remote::id(&r.sample_id)?;
        let mut paths = r.private_temp_root.ancestors().collect::<Vec<_>>();
        paths.reverse();
        let mut ancestors = Vec::new();
        for path in paths {
            ancestors.push(Pin::open(path, true)?);
        }
        let marker = Pin::open(&r.private_temp_root.join(".vw-editor-metrics-owner"), false)?;
        let mut bytes = Vec::new();
        marker
            .file()
            .try_clone()?
            .take(1025)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 1024 {
            return Err(Error::Limit);
        }
        let value: Marker = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
        if value.schema != 1 || value.owner_pid != r.owner_pid || value.nonce != r.marker_nonce {
            return Err(Error::Invalid);
        }
        let sample = r.private_temp_root.join(&r.sample_id);
        // create_dir refuses an existing sample; never recursively creates a caller path.
        fs::create_dir(&sample)?;
        let sample_pin = Pin::open(&sample, true)?;
        if fs::read_dir(&sample)?.next().is_some() {
            return Err(Error::Invalid);
        }
        let value = Self {
            root: r.private_temp_root.clone(),
            sample,
            ancestors,
            marker,
            sample_pin,
        };
        value.verify()?;
        Ok(value)
    }
    fn verify(&self) -> Result<()> {
        for pin in &self.ancestors {
            pin.verify()?;
        }
        self.marker.verify()?;
        self.sample_pin.verify()?;
        if self.sample.parent() != Some(self.root.as_path()) {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
enum Witness {
    Ordinary(Box<platform::Measurement>),
    InstalledPaint(platform::paint_controls::PaintFrameWitness),
}
impl Witness {
    fn json(&self) -> Result<serde_json::Value> {
        match self {
            Self::Ordinary(value) => serde_json::to_value(value),
            Self::InstalledPaint(value) => serde_json::to_value(value),
        }
        .map_err(|_| Error::Invalid)
    }
    fn start_qpc_100ns(&self) -> u64 {
        match self {
            Self::Ordinary(value) => value.canvas.sampled_qpc_100ns,
            Self::InstalledPaint(value) => value.observed_start_qpc_100ns,
        }
    }
    fn end_qpc_100ns(&self) -> u64 {
        match self {
            Self::Ordinary(value) => value.canvas.sampled_qpc_100ns,
            Self::InstalledPaint(value) => value.observed_end_qpc_100ns,
        }
    }
}
struct Context {
    request: Request,
    cancel: Arc<vw_capture::Cancellation>,
    source: SourceLease,
    temp: TempLease,
    before: Witness,
    capture_target: vw_capture::WindowTarget,
}
struct Pipes {
    input: Option<File>,
    output: Option<File>,
}
struct Outcome {
    _output: Option<File>,
    bytes: Result<Vec<u8>>,
}
struct Owner {
    child: Child,
    pins: Arc<Pins>,
    context: Context,
    pipes: Arc<Mutex<Pipes>>,
    io: Option<JoinHandle<Outcome>>,
    io_started: bool,
    _permit: Permit,
}
#[derive(Clone, Serialize)]
struct Receipt {
    schema: u32,
    #[serde(skip_serializing_if = "SamplingScope::is_full")]
    sampling_scope: SamplingScope,
    operation: &'static str,
    status: &'static str,
    helper_sha256: String,
    helper_pid: Option<u32>,
    helper_exit_code: Option<u32>,
    suspended_before_job_assignment: bool,
    exact_image_verified_before_resume: bool,
    process_and_job_retired: bool,
    io_worker_started: bool,
    io_join_completed: bool,
    error: Option<Error>,
    retirement_error: Option<Error>,
    io_error: Option<Error>,
    capture_refusal: Option<vw_capture::Error>,
    capture_color_depth: Option<vw_capture::ColorDepthDiagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    capture_alpha_summary: Option<vw_capture::AlphaSummary>,
    target: Target,
    image: ImageIdentity,
    source_proof: crate::editor_source::SourceProof,
    #[serde(skip_serializing_if = "Option::is_none")]
    installed_package_evidence: Option<vw_host::editor_paint_package::PaintPackageEvidence>,
    capture_session_id: String,
    sample_id: String,
    before: serde_json::Value,
    after: Option<serde_json::Value>,
    frame: Option<vw_capture::FrameReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    canvas_frame: Option<vw_capture::CanvasFrameReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    canvas_metrics: Option<metrics::CanvasMetrics>,
    metrics: Option<metrics::Metrics>,
    diagnostic_sha256: Option<String>,
    input_sent: bool,
    profile_authority: bool,
    numeric_semantics_proved: bool,
    image_file_presence: Option<bool>,
    root_image_disposal_required: bool,
}
struct Pending {
    owner: Owner,
    receipt: Receipt,
}
fn emit(receipt: &Receipt) -> Result<()> {
    let bytes = serde_json::to_vec(receipt).map_err(|_| Error::Invalid)?;
    if bytes.len() > MAX_OUTPUT {
        return Err(Error::Limit);
    }
    let mut out = std::io::stdout().lock();
    out.write_all(&bytes)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}
fn request() -> Result<Request> {
    let mut args = std::env::args().skip(1);
    let bytes = if let Some(json) = args.next() {
        if args.next().is_some() {
            return Err(Error::Invalid);
        }
        json.into_bytes()
    } else {
        let mut bytes = Vec::new();
        std::io::stdin()
            .lock()
            .take(MAX_REQUEST as u64 + 1)
            .read_to_end(&mut bytes)?;
        bytes
    };
    if bytes.len() > MAX_REQUEST {
        return Err(Error::Limit);
    }
    let r: Request = serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
    r.target.validate()?;
    r.canvas_rect_host.validate()?;
    vw_remote::id(&r.capture_session_id)?;
    vw_remote::id(&r.sample_id)?;
    vw_remote::id(&r.marker_nonce)?;
    if r.schema != 1
        || r.owner_pid == 0
        || !r.explicit_owner_capture
        || !r.owned_blank_fixture
        || !r.canvas_rect_host.inside(r.target.client_rect)
    {
        return Err(Error::Invalid);
    }
    paint::policy(
        r.source_policy,
        &r.image,
        r.expected_tool_settings.is_some(),
        r.expected_paint_frame.as_ref(),
    )?;
    Ok(r)
}
fn parent_job() -> Result<()> {
    // An outer source-owned runner must retain this process before foreign work.
    // A short handshake permits its actual Job assignment; no pre-resume outer claim.
    let start = Instant::now();
    loop {
        let mut present = BOOL::default();
        // SAFETY: query-only current-process pseudo handle and initialized BOOL.
        unsafe {
            platform::api(
                "frame parent Job",
                IsProcessInJob(GetCurrentProcess(), None, &mut present),
            )?;
        }
        if present.as_bool() {
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(1) {
            return Err(Error::Ungranted);
        }
        thread::sleep(Duration::from_millis(10));
    }
}
fn budget(cancel: &Arc<vw_capture::Cancellation>) -> Result<SourceBudget> {
    SourceBudget::new(cancel.clone(), Duration::from_secs(5))
}
fn ordinary_matches(
    before: &platform::Measurement,
    after: &platform::Measurement,
    r: &Request,
) -> Result<()> {
    canvas_proofs_match(
        &before.settings,
        &after.settings,
        &before.canvas,
        &after.canvas,
        r.expected_tool_settings.as_ref().ok_or(Error::Invalid)?,
        r.target.process_id,
        r.canvas_rect_host,
    )
}
fn canvas_proofs_match(
    before_settings: &ToolSettings,
    after_settings: &ToolSettings,
    before: &vw_remote::profile::CanvasProof,
    after: &vw_remote::profile::CanvasProof,
    expected: &ToolSettings,
    pid: u32,
    canvas: Rect,
) -> Result<()> {
    if before_settings != expected
        || after_settings != before_settings
        || before.process_id != pid
        || after.process_id != pid
        || before.canvas_rect != canvas
        || after.canvas_rect != before.canvas_rect
        || before.runtime_id_hash != after.runtime_id_hash
        || !vw_remote::profile::digest(&before.runtime_id_hash)
        || before.class_name != "KisOpenGLCanvas2"
        || before.control_type != 50026
        || before.class_name != after.class_name
        || before.control_type != after.control_type
        || before.sampled_qpc_100ns == 0
        || after.sampled_qpc_100ns < before.sampled_qpc_100ns
    {
        return Err(Error::TargetChanged);
    }
    Ok(())
}
fn retain_refusal_summary<T>(
    summary: &mut Option<vw_capture::AlphaSummary>,
    check: impl FnOnce() -> Result<T>,
) -> Option<T> {
    summary.as_ref()?;
    match check() {
        Ok(proof) => Some(proof),
        Err(_) => {
            *summary = None;
            None
        }
    }
}
fn measure_witness(r: &Request, b: &SourceBudget) -> Result<Witness> {
    match r.source_policy {
        SourcePolicy::Ordinary => b
            .call(|| platform::measure(&r.target, r.owner_pid, 3))
            .map(|value| Witness::Ordinary(Box::new(value))),
        SourcePolicy::InstalledPaint => b
            .call(|| platform::paint_controls::observe(&r.target, b)?.frame_witness())
            .map(Witness::InstalledPaint),
    }
}
fn admit_witness(value: &Witness, r: &Request) -> Result<()> {
    match (r.source_policy, value) {
        (SourcePolicy::Ordinary, Witness::Ordinary(before)) => ordinary_matches(before, before, r),
        (SourcePolicy::InstalledPaint, Witness::InstalledPaint(before)) => paint::admit(
            r.expected_paint_frame.as_ref().ok_or(Error::Invalid)?,
            before,
            r.canvas_rect_host,
            r.target.client_rect,
        ),
        _ => Err(Error::Invalid),
    }
}
fn canvas_matches(before: &Witness, after: &Witness, r: &Request) -> Result<()> {
    admit_witness(before, r)?;
    admit_witness(after, r)?;
    match (before, after) {
        (Witness::Ordinary(before), Witness::Ordinary(after)) => ordinary_matches(before, after, r),
        (Witness::InstalledPaint(before), Witness::InstalledPaint(after)) => {
            paint::unchanged(before, after)
        }
        _ => Err(Error::TargetChanged),
    }
}
fn initial(r: Request) -> Result<Context> {
    let cancel = Arc::new(vw_capture::Cancellation::default());
    let b = budget(&cancel)?;
    let source = SourceLease::open(&r.target, r.owner_pid, &r.image, r.source_policy, &b)?;
    let temp = b.call(|| TempLease::open(&r))?;
    let before = measure_witness(&r, &b)?;
    admit_witness(&before, &r)?;
    source.verify(&r.target, r.owner_pid, &b)?;
    let capture_target = b.call(|| {
        vw_capture::windows::fixed_target(r.target.window, r.owner_pid)
            .map_err(|_| Error::TargetChanged)
    })?;
    if rect(capture_target.client) != r.target.client_rect
        || rect(capture_target.frame) != r.target.frame_rect
    {
        return Err(Error::TargetChanged);
    }
    Ok(Context {
        request: r,
        cancel,
        source,
        temp,
        before,
        capture_target,
    })
}
fn rect(r: vw_capture::Rect) -> Rect {
    Rect {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    }
}
fn alpha_region(ctx: &Context) -> Result<vw_capture::AlphaRegion> {
    relative_alpha_region(ctx.request.canvas_rect_host, ctx.capture_target.client)
}
fn relative_alpha_region(
    canvas: Rect,
    client: vw_capture::Rect,
) -> Result<vw_capture::AlphaRegion> {
    let (x, y) = vw_capture::Rect {
        x: canvas.x,
        y: canvas.y,
        width: canvas.width,
        height: canvas.height,
    }
    .offset_inside(client)
    .map_err(|_| Error::TargetChanged)?;
    let region = vw_capture::AlphaRegion {
        x,
        y,
        width: canvas.width,
        height: canvas.height,
    };
    region
        .validate(client.width, client.height)
        .map_err(|_| Error::Invalid)?;
    Ok(region)
}
fn capture_request(ctx: &Context) -> Result<Vec<u8>> {
    let limits = vw_capture::Limits {
        memory_bytes: 128 * 1024 * 1024,
        png_bytes: metrics::MAX_PNG as u64,
        max_pixels: 4 * 1024 * 1024,
        capture_ms: 3000,
        ..vw_capture::Limits::default()
    };
    limits
        .image(
            ctx.capture_target.frame.width,
            ctx.capture_target.frame.height,
        )
        .map_err(|_| Error::Limit)?;
    let capture = vw_capture::CaptureRequest {
        explicit_owner_action: true,
        owner_process_id: ctx.request.owner_pid,
        target: ctx.capture_target.clone(),
        capture_session_id: ctx.request.capture_session_id.clone(),
        output_directory: ctx.temp.sample.to_str().ok_or(Error::Invalid)?.into(),
        limits,
        diagnostic_color_stage: true,
        diagnostic_alpha_region: if ctx.request.sampling_scope == SamplingScope::FullClient {
            Some(alpha_region(ctx)?)
        } else {
            None
        },
    };
    let value = match ctx.request.sampling_scope {
        SamplingScope::FullClient => vw_capture::Request::Capture(capture),
        SamplingScope::VerifiedEditorCanvas => {
            vw_capture::Request::CanvasFixture(vw_capture::CanvasCaptureRequest {
                capture,
                explicit_owned_blank_fixture: ctx.request.owned_blank_fixture,
                canvas_rect_host: vw_capture::Rect {
                    x: ctx.request.canvas_rect_host.x,
                    y: ctx.request.canvas_rect_host.y,
                    width: ctx.request.canvas_rect_host.width,
                    height: ctx.request.canvas_rect_host.height,
                },
            })
        }
    };
    let bytes = serde_json::to_vec(&value).map_err(|_| Error::Invalid)?;
    if bytes.len() > MAX_REQUEST {
        return Err(Error::Limit);
    }
    Ok(bytes)
}
fn io(pipes: Arc<Mutex<Pipes>>, bytes: Vec<u8>, _pins: Arc<Pins>) -> Outcome {
    let (input, output) = {
        let mut p = pipes.lock().unwrap_or_else(|e| e.into_inner());
        (p.input.take(), p.output.take())
    };
    let Some(mut output) = output else {
        return Outcome {
            _output: None,
            bytes: Err(Error::Io),
        };
    };
    let result = match input {
        Some(mut input) => {
            let sent = input.write_all(&bytes).map_err(Error::from);
            drop(input);
            sent.and_then(|()| {
                let mut bytes = Vec::new();
                Read::by_ref(&mut output)
                    .take(MAX_OUTPUT as u64 + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() > MAX_OUTPUT {
                    return Err(Error::Limit);
                }
                Ok(bytes)
            })
        }
        None => Err(Error::Io),
    };
    Outcome {
        _output: Some(output),
        bytes: result,
    }
}
impl Owner {
    fn collect(&mut self) -> Collection<Outcome> {
        let child = &self.child;
        let worker = &mut self.io;
        collect_latest(
            || child.retired_receipt(),
            worker.as_ref().is_none_or(JoinHandle::is_finished),
            || {
                worker
                    .take()
                    .map(|v| v.join().map_err(|_| Error::Io))
                    .transpose()
            },
        )
    }
}
fn pending(owner: Owner, mut receipt: Receipt, cause: Error) -> Result<()> {
    receipt.retirement_error = Some(cause.clone());
    receipt.error.get_or_insert(cause);
    let output = receipt.clone();
    RETAINED
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(Pending { owner, receipt });
    // Registration precedes output; failed stdout cannot drop a live ownership tree.
    emit(&output)?;
    Err(Error::RetirementPending)
}
fn frame_valid(
    frame: &vw_capture::FrameReceipt,
    expected: &vw_capture::WindowTarget,
    owner: u32,
    session: &str,
    before_qpc_100ns: u64,
) -> Result<()> {
    source_identity_valid(
        &frame.identity,
        &frame.target,
        expected,
        owner,
        session,
        before_qpc_100ns,
    )?;
    if frame.width != expected.client.width
        || frame.height != expected.client.height
        || frame.bit_depth != 8
        || !frame.border_visible
        || !frame.lossless
        || frame.filename != "capture.png"
        || frame.png_bytes == 0
        || frame.png_bytes > metrics::MAX_PNG as u64
        || !vw_remote::profile::digest(&frame.source_asset_id)
    {
        return Err(Error::TargetChanged);
    }
    Ok(())
}
fn source_identity_valid(
    i: &vw_capture::FrameIdentity,
    target: &vw_capture::WindowTarget,
    expected: &vw_capture::WindowTarget,
    owner: u32,
    session: &str,
    before_qpc_100ns: u64,
) -> Result<()> {
    expected
        .unchanged(target, owner)
        .map_err(|_| Error::TargetChanged)?;
    i.validate().map_err(|_| Error::Invalid)?;
    if i.capture_session_id != session
        || i.frame_id != 1
        || i.geometry_revision != 1
        || i.source_kind != "window"
        || i.platform != "windows"
        || !i.monitor_id.is_empty()
        || i.window_handle != expected.window
        || i.client_rect != expected.client
        || i.dpi_scale != f64::from(expected.dpi) / 96.0
        || i.monotonic_timestamp_ns < expected.observed_ns
        || i.monotonic_timestamp_ns < before_qpc_100ns.checked_mul(100).ok_or(Error::Limit)?
    {
        return Err(Error::TargetChanged);
    }
    Ok(())
}
fn canvas_frame_valid(
    frame: &vw_capture::CanvasFrameReceipt,
    expected: &vw_capture::WindowTarget,
    canvas: Rect,
    owner: u32,
    session: &str,
    before: u64,
) -> Result<()> {
    source_identity_valid(
        &frame.source_identity,
        &frame.target,
        expected,
        owner,
        session,
        before,
    )?;
    let wanted = vw_capture::Rect {
        x: canvas.x,
        y: canvas.y,
        width: canvas.width,
        height: canvas.height,
    };
    wanted
        .offset_inside(expected.client)
        .map_err(|_| Error::TargetChanged)?;
    let (x, y) = wanted
        .offset_inside(expected.frame)
        .map_err(|_| Error::TargetChanged)?;
    if frame.canvas_rect_host != wanted
        || frame.crop_in_frame
            != (vw_capture::AlphaRegion {
                x,
                y,
                width: canvas.width,
                height: canvas.height,
            })
        || frame.width != canvas.width
        || frame.height != canvas.height
        || frame.bit_depth != 8
        || !frame.border_visible
        || !frame.lossless
        || frame.filename != "capture-canvas.png"
        || frame.png_bytes == 0
        || frame.png_bytes > metrics::MAX_PNG as u64
        || !vw_remote::profile::digest(&frame.source_asset_id)
    {
        return Err(Error::TargetChanged);
    }
    Ok(())
}
enum Sample {
    FullClient(vw_capture::FrameReceipt),
    Canvas(vw_capture::CanvasFrameReceipt),
}
impl Sample {
    fn identity(&self) -> &vw_capture::FrameIdentity {
        match self {
            Self::FullClient(v) => &v.identity,
            Self::Canvas(v) => &v.source_identity,
        }
    }
    fn filename(&self) -> &str {
        match self {
            Self::FullClient(v) => &v.filename,
            Self::Canvas(v) => &v.filename,
        }
    }
    fn validate(&self, ctx: &Context) -> Result<()> {
        match self {
            Self::FullClient(v) => frame_valid(
                v,
                &ctx.capture_target,
                ctx.request.owner_pid,
                &ctx.request.capture_session_id,
                ctx.before.end_qpc_100ns(),
            ),
            Self::Canvas(v) => canvas_frame_valid(
                v,
                &ctx.capture_target,
                ctx.request.canvas_rect_host,
                ctx.request.owner_pid,
                &ctx.request.capture_session_id,
                ctx.before.end_qpc_100ns(),
            ),
        }
    }
}
fn canvas_response(
    bytes: &[u8],
    refusal: &mut Option<vw_capture::Error>,
    color_depth: &mut Option<vw_capture::ColorDepthDiagnostic>,
    alpha_summary: &mut Option<vw_capture::AlphaSummary>,
) -> Result<vw_capture::CanvasFrameReceipt> {
    match serde_json::from_slice::<vw_capture::Response>(bytes).map_err(|_| Error::Invalid)? {
        vw_capture::Response::CanvasFixture(frame) => Ok(frame),
        vw_capture::Response::Refused {
            error,
            color_depth: facts,
            alpha_summary: alpha,
        } => {
            *refusal = Some(error);
            *color_depth = facts;
            *alpha_summary = alpha;
            Err(Error::Unavailable)
        }
        _ => Err(Error::Unavailable),
    }
}
fn capture_response(
    bytes: &[u8],
    refusal: &mut Option<vw_capture::Error>,
    color_depth: &mut Option<vw_capture::ColorDepthDiagnostic>,
    alpha_summary: &mut Option<vw_capture::AlphaSummary>,
) -> Result<vw_capture::FrameReceipt> {
    match serde_json::from_slice::<vw_capture::Response>(bytes).map_err(|_| Error::Invalid)? {
        vw_capture::Response::Frame(frame) => Ok(frame),
        vw_capture::Response::Refused {
            error,
            color_depth: facts,
            alpha_summary: alpha,
        } => {
            *refusal = Some(error);
            *color_depth = facts;
            *alpha_summary = alpha;
            Err(Error::Unavailable)
        }
        _ => Err(Error::Unavailable),
    }
}
fn validate_alpha_summary(
    summary: &mut Option<vw_capture::AlphaSummary>,
    refusal: Option<vw_capture::Error>,
    stage: Option<vw_capture::ColorDepthDiagnostic>,
    client: vw_capture::Rect,
    canvas: vw_capture::AlphaRegion,
) -> Result<()> {
    if let Some(value) = *summary {
        let valid_stage = matches!(stage,Some(vw_capture::ColorDepthDiagnostic::NonopaquePixel {alpha}) if (value.min_alpha..=value.max_alpha).contains(&alpha));
        if refusal != Some(vw_capture::Error::ColorDepth)
            || !valid_stage
            || value.validate(client.width, client.height, canvas).is_err()
        {
            *summary = None;
            return Err(Error::Invalid);
        }
    }
    Ok(())
}
fn measurement(owner: &Owner, bytes: &[u8], receipt: &mut Receipt) -> Result<()> {
    let ctx = &owner.context;
    let r = &ctx.request;
    let captured = match r.sampling_scope {
        SamplingScope::FullClient => capture_response(
            bytes,
            &mut receipt.capture_refusal,
            &mut receipt.capture_color_depth,
            &mut receipt.capture_alpha_summary,
        )
        .map(Sample::FullClient),
        SamplingScope::VerifiedEditorCanvas => canvas_response(
            bytes,
            &mut receipt.capture_refusal,
            &mut receipt.capture_color_depth,
            &mut receipt.capture_alpha_summary,
        )
        .map(Sample::Canvas),
    };
    if r.sampling_scope == SamplingScope::VerifiedEditorCanvas
        && receipt.capture_alpha_summary.is_some()
    {
        receipt.capture_alpha_summary = None;
        return Err(Error::Invalid);
    }
    validate_alpha_summary(
        &mut receipt.capture_alpha_summary,
        receipt.capture_refusal,
        receipt.capture_color_depth,
        ctx.capture_target.client,
        alpha_region(ctx)?,
    )?;
    if captured.is_err() {
        let after = retain_refusal_summary(&mut receipt.capture_alpha_summary, || {
            let b = budget(&ctx.cancel)?;
            ctx.source.verify(&r.target, r.owner_pid, &b)?;
            ctx.temp.verify()?;
            owner.pins.verify()?;
            let after = measure_witness(r, &b)?;
            canvas_matches(&ctx.before, &after, r)?;
            ctx.source.verify(&r.target, r.owner_pid, &b)?;
            b.call(|| after.json())
        });
        if let Some(after) = after {
            receipt.after = Some(after);
        }
    }
    let sample = captured?;
    sample.validate(ctx)?;
    let b = budget(&ctx.cancel)?;
    ctx.source.verify(&r.target, r.owner_pid, &b)?;
    ctx.temp.verify()?;
    owner.pins.verify()?;
    let after = measure_witness(r, &b)?;
    canvas_matches(&ctx.before, &after, r)?;
    if after
        .start_qpc_100ns()
        .checked_mul(100)
        .ok_or(Error::Limit)?
        < sample.identity().monotonic_timestamp_ns
    {
        return Err(Error::TargetChanged);
    }
    let png = Pin::open(&ctx.temp.sample.join(sample.filename()), false)?;
    let (metrics, canvas_metrics) = match &sample {
        Sample::FullClient(v) => (
            Some(b.call(|| metrics::measure(&png, v, r.canvas_rect_host, rect(v.target.client)))?),
            None,
        ),
        Sample::Canvas(v) => (
            None,
            Some(b.call(|| {
                metrics::measure_canvas(&png, v, r.canvas_rect_host, rect(v.target.client))
            })?),
        ),
    };
    ctx.source.verify(&r.target, r.owner_pid, &b)?;
    let final_proof = measure_witness(r, &b)?;
    canvas_matches(&after, &final_proof, r)?;
    ctx.source.verify(&r.target, r.owner_pid, &b)?;
    ctx.temp.verify()?;
    png.verify()?;
    receipt.after = Some(final_proof.json()?);
    match sample {
        Sample::FullClient(v) => receipt.frame = Some(v),
        Sample::Canvas(v) => receipt.canvas_frame = Some(v),
    }
    receipt.metrics = metrics;
    receipt.canvas_metrics = canvas_metrics;
    receipt.image_file_presence = Some(true);
    Ok(())
}
fn completed(
    owner: &Owner,
    receipt: &mut Receipt,
    pid: u32,
    code: u32,
    outcome: Result<Option<Outcome>>,
) {
    receipt.helper_pid = Some(pid);
    receipt.helper_exit_code = Some(code);
    receipt.process_and_job_retired = true;
    receipt.io_join_completed = owner.io_started;
    match outcome {
        Ok(Some(outcome)) => match outcome.bytes {
            Ok(bytes) => {
                if code != 0 {
                    receipt.error.get_or_insert(Error::Unavailable);
                    receipt.diagnostic_sha256 = Some(sha(&bytes));
                } else if receipt.error.is_none()
                    && let Err(error) = measurement(owner, &bytes, receipt)
                {
                    receipt.error = Some(error);
                    receipt.diagnostic_sha256 = Some(sha(&bytes));
                }
            }
            Err(error) => {
                receipt.io_error = Some(error.clone());
                receipt.error.get_or_insert(error);
            }
        },
        Ok(None) => {
            receipt.error.get_or_insert(Error::Io);
        }
        Err(error) => {
            receipt.io_error = Some(error.clone());
            receipt.error.get_or_insert(error);
        }
    }
    receipt.status = if receipt.error.is_none() {
        "Measured"
    } else {
        "Failed"
    };
}
pub(super) fn run() -> Result<()> {
    parent_job()?;
    let permit = Permit::acquire()?;
    let r = request()?;
    let _apartment = platform::EffectApartment::initialize()?;
    platform::initialize_hil_dpi()?;
    let pins = locked_image(
        &r.capture_helper_path,
        &r.capture_helper_sha256,
        "vw-capture-helper.exe",
    )?;
    let ctx = initial(r)?;
    let bytes = capture_request(&ctx)?;
    // Complete every fallible receipt allocation before a child exists.
    let before = ctx.before.json()?;
    let installed_package_evidence = ctx.source.installed_package_evidence().cloned();
    let cancel = Cancellation::default();
    let mut child = Child::launch(&ctx.request.capture_helper_path, &pins, &cancel)?;
    let startup = child.startup_error.clone();
    let pipes = Arc::new(Mutex::new(Pipes {
        input: child.stdin.take(),
        output: child.stdout.take(),
    }));
    let mut owner = Owner {
        child,
        pins,
        context: ctx,
        pipes,
        io: None,
        io_started: false,
        _permit: permit,
    };
    let mut receipt = Receipt {
        schema: 1,
        sampling_scope: owner.context.request.sampling_scope,
        operation: "owned-blank-editor-wgc-metrics",
        status: "RetirementPending",
        helper_sha256: owner.context.request.capture_helper_sha256.clone(),
        helper_pid: None,
        helper_exit_code: None,
        suspended_before_job_assignment: true,
        exact_image_verified_before_resume: startup.is_none(),
        process_and_job_retired: false,
        io_worker_started: false,
        io_join_completed: false,
        error: startup,
        retirement_error: None,
        io_error: None,
        capture_refusal: None,
        capture_color_depth: None,
        capture_alpha_summary: None,
        target: owner.context.request.target.clone(),
        image: owner.context.request.image.clone(),
        source_proof: owner.context.source.proof(),
        installed_package_evidence,
        capture_session_id: owner.context.request.capture_session_id.clone(),
        sample_id: owner.context.request.sample_id.clone(),
        before,
        after: None,
        frame: None,
        canvas_frame: None,
        canvas_metrics: None,
        metrics: None,
        diagnostic_sha256: None,
        input_sent: false,
        profile_authority: false,
        numeric_semantics_proved: false,
        image_file_presence: None,
        root_image_disposal_required: true,
    };
    let pipe_owner = owner.pipes.clone();
    let pin_owner = owner.pins.clone();
    match thread::Builder::new()
        .name("editor-frame-io".into())
        .spawn(move || io(pipe_owner, bytes, pin_owner))
    {
        Ok(worker) => {
            owner.io = Some(worker);
            owner.io_started = true;
            receipt.io_worker_started = true;
        }
        Err(_) => {
            receipt.error.get_or_insert(Error::Io);
        }
    }
    let start = Instant::now();
    let mut progress = Instant::now();
    let deadline = if receipt.error.is_some() {
        Duration::ZERO
    } else {
        Duration::from_secs(8)
    };
    while start.elapsed() < deadline {
        match owner.collect() {
            Collection::Joined { pid, code, outcome } => {
                completed(&owner, &mut receipt, pid, code, outcome);
                emit(&receipt)?;
                return receipt.error.map_or(Ok(()), Err);
            }
            Collection::Pending(_) => {}
        }
        if progress.elapsed() >= Duration::from_secs(1) {
            eprintln!("editor-frame: capture active; retained child/Job/IO/source");
            progress = Instant::now();
        }
        thread::sleep(Duration::from_millis(10));
    }
    receipt.error.get_or_insert(Error::Timeout);
    owner.context.cancel.cancel();
    if let Err(error) = owner.child.kill_tree() {
        receipt.retirement_error = Some(error);
    }
    let retired = Instant::now();
    while retired.elapsed() < Duration::from_secs(5) {
        match owner.collect() {
            Collection::Joined { pid, code, outcome } => {
                completed(&owner, &mut receipt, pid, code, outcome);
                emit(&receipt)?;
                return receipt.error.map_or(Ok(()), Err);
            }
            Collection::Pending(_) => {}
        }
        if progress.elapsed() >= Duration::from_secs(1) {
            eprintln!("editor-frame: retirement active; all owners retained");
            progress = Instant::now();
        }
        thread::sleep(Duration::from_millis(10));
    }
    match owner.collect() {
        Collection::Joined { pid, code, outcome } => {
            completed(&owner, &mut receipt, pid, code, outcome);
            emit(&receipt)?;
            receipt.error.map_or(Ok(()), Err)
        }
        Collection::Pending(error) => pending(owner, receipt, error),
    }
}
pub(super) fn wait_retained() {
    let mut progress = Instant::now();
    loop {
        let mut owners = RETAINED
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if owners.is_empty() {
            return;
        }
        let mut at = 0;
        while at < owners.len() {
            let _kill = owners[at].owner.child.kill_tree();
            match owners[at].owner.collect() {
                Collection::Joined { pid, code, outcome } => {
                    let mut entry = owners.remove(at);
                    completed(&entry.owner, &mut entry.receipt, pid, code, outcome);
                    let _output = emit(&entry.receipt);
                }
                Collection::Pending(_) => {
                    at += 1;
                }
            }
        }
        drop(owners);
        if progress.elapsed() >= Duration::from_secs(1) {
            eprintln!(
                "editor-frame: Pending owner retained until actual process/Job/IO retirement"
            );
            progress = Instant::now();
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_os_temp_separator_preserves_drive_root_and_exact_spelling() -> Result<()> {
        assert_eq!(os_temp_root(Path::new(r"C:\Temp\"))?, Path::new(r"C:\Temp"));
        assert_eq!(os_temp_root(Path::new(r"C:\Temp"))?, Path::new(r"C:\Temp"));
        assert_eq!(os_temp_root(Path::new(r"C:\"))?, Path::new(r"C:\"));
        Ok(())
    }
    #[test]
    fn os_temp_shape_repair_does_not_admit_ambiguous_namespaces() {
        for path in [
            r"C:\Temp\\",
            r"C:\\",
            r"C:\Temp\..\",
            r"C:\Temp.\",
            r"C:\Temp \",
            r"\\server\share\",
            r"C:\Temp:stream\",
            r"C:/Temp/",
            r"C:relative",
        ] {
            assert_eq!(os_temp_root(Path::new(path)), Err(Error::Invalid));
        }
    }
    fn fixture() -> vw_capture::FrameReceipt {
        let client = vw_capture::Rect {
            x: -90,
            y: -40,
            width: 80,
            height: 80,
        };
        let target = vw_capture::WindowTarget {
            window: 1,
            process_id: 7,
            process_created: 1,
            client,
            frame: vw_capture::Rect {
                x: -100,
                y: -50,
                width: 100,
                height: 100,
            },
            dpi: 96,
            observed_ns: 1000,
        };
        vw_capture::FrameReceipt {
            identity: vw_capture::FrameIdentity {
                capture_session_id: "00000000-0000-7000-8000-000000000001".into(),
                frame_id: 1,
                geometry_revision: 1,
                source_kind: "window".into(),
                client_rect: client,
                dpi_scale: 1.0,
                monotonic_timestamp_ns: 1100,
                captured_at_ms: 1,
                platform: "windows".into(),
                window_handle: 1,
                monitor_id: String::new(),
            },
            target,
            source_asset_id: "a".repeat(64),
            png_bytes: 100,
            width: 80,
            height: 80,
            bit_depth: 8,
            border_visible: true,
            lossless: true,
            filename: "capture.png".into(),
        }
    }
    #[test]
    fn receipt_cannot_replace_client_crop_or_selected_process() -> Result<()> {
        let mut frame = fixture();
        let expected = frame.target.clone();
        let session = frame.identity.capture_session_id.clone();
        frame_valid(&frame, &expected, 999, &session, 10)?;
        frame.width = 81;
        assert_eq!(
            frame_valid(&frame, &expected, 999, &session, 10),
            Err(Error::TargetChanged)
        );
        frame.width = 80;
        frame.target.process_created = 2;
        assert_eq!(
            frame_valid(&frame, &expected, 999, &session, 10),
            Err(Error::TargetChanged)
        );
        Ok(())
    }
    #[test]
    fn source_session_and_fresh_frame_clock_cannot_be_substituted() -> Result<()> {
        let mut frame = fixture();
        let expected = frame.target.clone();
        let session = frame.identity.capture_session_id.clone();
        assert_eq!(
            frame_valid(&frame, &expected, 999, &session, 12),
            Err(Error::TargetChanged)
        );
        frame.identity.capture_session_id = "00000000-0000-7000-8000-000000000002".into();
        assert_eq!(
            frame_valid(&frame, &expected, 999, &session, 10),
            Err(Error::TargetChanged)
        );
        frame.identity.capture_session_id = session.clone();
        frame.filename = "alternate.png".into();
        assert_eq!(
            frame_valid(&frame, &expected, 999, &session, 10),
            Err(Error::TargetChanged)
        );
        Ok(())
    }
    #[test]
    fn temp_must_be_direct_owned_child_outside_workspace() -> Result<()> {
        let id = "00000000-0000-7000-8000-000000000001";
        let temp = Path::new("C:\\Temp");
        let workspace = Path::new("C:\\Repo");
        let good = temp.join(format!("VisualWorkbench-editor-metrics-{id}"));
        temp_path(&good, temp, workspace, id)?;
        assert_eq!(
            temp_path(&good, Path::new("C:\\Other"), workspace, id),
            Err(Error::Invalid)
        );
        assert_eq!(temp_path(&good, temp, temp, id), Err(Error::Invalid));
        assert_eq!(
            temp_path(&good.join("child"), temp, workspace, id),
            Err(Error::Invalid)
        );
        Ok(())
    }
    #[test]
    fn uncertain_retirement_never_consumes_finished_worker() {
        let joined = std::cell::Cell::new(false);
        let result = collect_latest(
            || Err(Error::Io),
            true,
            || {
                joined.set(true);
                Ok(Some(7))
            },
        );
        assert!(matches!(result, Collection::Pending(Error::Io)));
        assert!(!joined.get());
        let result = collect_latest(
            || Ok((1, 0)),
            false,
            || {
                joined.set(true);
                Ok(Some(7))
            },
        );
        assert!(matches!(
            result,
            Collection::Pending(Error::RetirementPending)
        ));
        assert!(!joined.get());
    }
    #[test]
    fn completed_failed_join_is_distinct_from_pending_owner() {
        let joined = std::cell::Cell::new(false);
        let result: Collection<u32> = collect_latest(
            || Ok((1, 2)),
            true,
            || {
                joined.set(true);
                Err(Error::Io)
            },
        );
        assert!(matches!(
            result,
            Collection::Joined {
                pid: 1,
                code: 2,
                outcome: Err(Error::Io)
            }
        ));
        assert!(joined.get());
    }
    #[test]
    fn exact_capture_color_depth_refusal_keeps_closed_stage_and_original_error() -> Result<()> {
        let stage = vw_capture::ColorDepthDiagnostic::NonopaquePixel { alpha: 0 };
        let bytes = serde_json::to_vec(&vw_capture::Response::Refused {
            error: vw_capture::Error::ColorDepth,
            color_depth: Some(stage),
            alpha_summary: None,
        })
        .map_err(|_| Error::Invalid)?;
        let (mut refusal, mut facts, mut alpha) = (None, None, None);
        assert!(matches!(
            capture_response(&bytes, &mut refusal, &mut facts, &mut alpha),
            Err(Error::Unavailable)
        ));
        assert_eq!(refusal, Some(vw_capture::Error::ColorDepth));
        assert_eq!(facts, Some(stage));
        assert_eq!(alpha, None);
        Ok(())
    }
    #[test]
    fn legacy_and_malformed_capture_response_cannot_invent_numeric_color_facts() {
        let (mut refusal, mut facts, mut alpha) = (None, None, None);
        assert!(matches!(
            capture_response(
                br#"{"result":"refused","error":"color_depth"}"#,
                &mut refusal,
                &mut facts,
                &mut alpha
            ),
            Err(Error::Unavailable)
        ));
        assert_eq!(refusal, Some(vw_capture::Error::ColorDepth));
        assert_eq!(facts, None);
        assert_eq!(alpha, None);
        refusal = None;
        assert!(matches!(
            capture_response(
                br#"{"result":"refused","error":"unknown","text":"private"}"#,
                &mut refusal,
                &mut facts,
                &mut alpha
            ),
            Err(Error::Invalid)
        ));
        assert_eq!(refusal, None);
        assert_eq!(facts, None);
        assert_eq!(alpha, None);
    }
    #[test]
    fn alpha_canvas_is_relative_to_exact_client_and_never_guessed() -> Result<()> {
        let client = vw_capture::Rect {
            x: -100,
            y: 30,
            width: 80,
            height: 60,
        };
        let canvas = Rect {
            x: -98,
            y: 33,
            width: 40,
            height: 20,
        };
        assert_eq!(
            relative_alpha_region(canvas, client)?,
            vw_capture::AlphaRegion {
                x: 2,
                y: 3,
                width: 40,
                height: 20
            }
        );
        assert!(relative_alpha_region(Rect { x: -101, ..canvas }, client).is_err());
        Ok(())
    }
    #[test]
    fn alpha_summary_keeps_original_refusal_and_rejects_wrong_stage_or_scope() -> Result<()> {
        let summary = vw_capture::AlphaSummary {
            client_width: 5,
            client_height: 5,
            bad_pixel_count: 1,
            first_x: 0,
            first_y: 0,
            min_x: 0,
            min_y: 0,
            max_x: 0,
            max_y: 0,
            min_alpha: 228,
            max_alpha: 228,
            edge_pixel_count: 1,
            interior_pixel_count: 0,
            min_edge_distance: 0,
            max_edge_distance: 0,
            canvas_bad_pixel_count: 0,
        };
        let bytes = serde_json::to_vec(&vw_capture::Response::Refused {
            error: vw_capture::Error::ColorDepth,
            color_depth: Some(vw_capture::ColorDepthDiagnostic::NonopaquePixel { alpha: 228 }),
            alpha_summary: Some(summary),
        })
        .map_err(|_| Error::Invalid)?;
        let (mut refusal, mut stage, mut alpha) = (None, None, None);
        assert!(matches!(
            capture_response(&bytes, &mut refusal, &mut stage, &mut alpha),
            Err(Error::Unavailable)
        ));
        let client = vw_capture::Rect {
            x: 100,
            y: 200,
            width: 5,
            height: 5,
        };
        let canvas = vw_capture::AlphaRegion {
            x: 1,
            y: 1,
            width: 3,
            height: 3,
        };
        validate_alpha_summary(&mut alpha, refusal, stage, client, canvas)?;
        assert_eq!(alpha, Some(summary));
        assert_eq!(refusal, Some(vw_capture::Error::ColorDepth));
        for bad_stage in [
            None,
            Some(vw_capture::ColorDepthDiagnostic::NonopaquePixel { alpha: 1 }),
            Some(vw_capture::ColorDepthDiagnostic::MonitorColorSpace { color_space: 0 }),
        ] {
            let mut alpha = Some(summary);
            assert!(
                validate_alpha_summary(&mut alpha, refusal, bad_stage, client, canvas).is_err()
            );
            assert_eq!(alpha, None);
        }
        let mut alpha = Some(summary);
        assert!(
            validate_alpha_summary(
                &mut alpha,
                refusal,
                stage,
                vw_capture::Rect { width: 6, ..client },
                canvas
            )
            .is_err()
        );
        assert_eq!(alpha, None);
        Ok(())
    }
    fn post_summary() -> vw_capture::AlphaSummary {
        vw_capture::AlphaSummary {
            client_width: 5,
            client_height: 5,
            bad_pixel_count: 1,
            first_x: 0,
            first_y: 0,
            min_x: 0,
            min_y: 0,
            max_x: 0,
            max_y: 0,
            min_alpha: 228,
            max_alpha: 228,
            edge_pixel_count: 1,
            interior_pixel_count: 0,
            min_edge_distance: 0,
            max_edge_distance: 0,
            canvas_bad_pixel_count: 0,
        }
    }
    fn post_canvas() -> vw_remote::profile::CanvasProof {
        vw_remote::profile::CanvasProof {
            runtime_id_hash: "a".repeat(64),
            process_id: 7,
            sampled_qpc_100ns: 1,
            class_name: "KisOpenGLCanvas2".into(),
            control_type: 50026,
            canvas_rect: Rect {
                x: 1,
                y: 1,
                width: 3,
                height: 3,
            },
            profile_digest: String::new(),
        }
    }
    fn post_settings() -> ToolSettings {
        ToolSettings {
            freehand_selected: true,
            selected_tool_id: "KisToolBrush".into(),
            eraser_mode: false,
            preset: "fixture".into(),
            blending_mode: "Normal".into(),
            preserve_alpha: false,
        }
    }
    #[test]
    fn valid_refusal_summary_requires_unchanged_actual_post_canvas_and_settings() {
        let before = post_canvas();
        let mut after = before.clone();
        after.sampled_qpc_100ns = 2;
        let settings = post_settings();
        let mut summary = Some(post_summary());
        let proof = retain_refusal_summary(&mut summary, || {
            canvas_proofs_match(
                &settings,
                &settings,
                &before,
                &after,
                &settings,
                7,
                before.canvas_rect,
            )
        });
        assert_eq!(proof, Some(()));
        assert_eq!(summary, Some(post_summary()));
    }
    #[test]
    fn changed_post_canvas_or_target_suppresses_summary_keeps_color_depth() {
        let before = post_canvas();
        let settings = post_settings();
        let original = vw_capture::Error::ColorDepth;
        for field in 0..3 {
            let mut after = before.clone();
            after.sampled_qpc_100ns = 2;
            match field {
                0 => after.canvas_rect.x += 1,
                1 => after.runtime_id_hash = "b".repeat(64),
                _ => after.process_id += 1,
            }
            let mut summary = Some(post_summary());
            assert_eq!(
                retain_refusal_summary(&mut summary, || canvas_proofs_match(
                    &settings,
                    &settings,
                    &before,
                    &after,
                    &settings,
                    7,
                    before.canvas_rect
                )),
                None
            );
            assert_eq!(summary, None);
            assert_eq!(original, vw_capture::Error::ColorDepth);
        }
    }
    #[test]
    fn post_source_failure_or_cancel_suppresses_summary_and_absence_skips_query() {
        for error in [
            Error::TargetChanged,
            Error::Cancelled,
            Error::Timeout,
            Error::Io,
        ] {
            let mut summary = Some(post_summary());
            assert_eq!(
                retain_refusal_summary(&mut summary, || Err::<(), _>(error)),
                None
            );
            assert_eq!(summary, None);
        }
        let called = std::cell::Cell::new(false);
        let mut summary = None;
        assert_eq!(
            retain_refusal_summary(&mut summary, || {
                called.set(true);
                Ok(())
            }),
            None
        );
        assert!(!called.get());
    }

    fn canvas_fixture() -> vw_capture::CanvasFrameReceipt {
        let full = fixture();
        vw_capture::CanvasFrameReceipt {
            source_identity: full.identity,
            target: full.target,
            canvas_rect_host: vw_capture::Rect {
                x: -80,
                y: -30,
                width: 40,
                height: 20,
            },
            crop_in_frame: vw_capture::AlphaRegion {
                x: 20,
                y: 20,
                width: 40,
                height: 20,
            },
            source_asset_id: full.source_asset_id,
            png_bytes: full.png_bytes,
            width: 40,
            height: 20,
            bit_depth: 8,
            border_visible: true,
            lossless: true,
            filename: "capture-canvas.png".into(),
        }
    }
    fn fixture_canvas() -> Rect {
        Rect {
            x: -80,
            y: -30,
            width: 40,
            height: 20,
        }
    }
    #[test]
    fn canvas_receipt_retains_original_target_and_exact_distinct_crop() -> Result<()> {
        let frame = canvas_fixture();
        let expected = frame.target.clone();
        canvas_frame_valid(
            &frame,
            &expected,
            fixture_canvas(),
            999,
            &frame.source_identity.capture_session_id,
            10,
        )?;
        assert_eq!(frame.source_identity.client_rect, expected.client);
        assert_ne!(frame.width, expected.client.width);
        assert_eq!(frame.crop_in_frame.x, 20);
        Ok(())
    }
    #[test]
    fn canvas_receipt_rejects_substituted_crop_dimensions_source_or_clock() -> Result<()> {
        let original = canvas_fixture();
        let expected = original.target.clone();
        let session = original.source_identity.capture_session_id.clone();
        for kind in 0..7 {
            let mut frame = original.clone();
            match kind {
                0 => frame.crop_in_frame.x += 1,
                1 => frame.canvas_rect_host.x += 1,
                2 => frame.width += 1,
                3 => frame.target.process_created += 1,
                4 => frame.source_identity.monotonic_timestamp_ns = 999,
                5 => {
                    frame.source_identity.capture_session_id =
                        "00000000-0000-7000-8000-000000000002".into()
                }
                _ => frame.filename = "capture.png".into(),
            }
            assert!(
                canvas_frame_valid(&frame, &expected, fixture_canvas(), 999, &session, 10).is_err()
            );
        }
        Ok(())
    }
    #[test]
    fn full_client_and_canvas_responses_cannot_cross_adopt() -> Result<()> {
        let full = serde_json::to_vec(&vw_capture::Response::Frame(fixture()))
            .map_err(|_| Error::Invalid)?;
        let canvas = serde_json::to_vec(&vw_capture::Response::CanvasFixture(canvas_fixture()))
            .map_err(|_| Error::Invalid)?;
        let (mut refusal, mut color, mut alpha) = (None, None, None);
        assert!(canvas_response(&full, &mut refusal, &mut color, &mut alpha).is_err());
        assert!(capture_response(&canvas, &mut refusal, &mut color, &mut alpha).is_err());
        assert!(canvas_response(&canvas, &mut refusal, &mut color, &mut alpha).is_ok());
        assert!(capture_response(&full, &mut refusal, &mut color, &mut alpha).is_ok());
        assert!(refusal.is_none() && color.is_none() && alpha.is_none());
        Ok(())
    }
    #[test]
    fn roi_refusal_keeps_exact_color_gate_without_canvas_summary_authority() -> Result<()> {
        let bytes = serde_json::to_vec(&vw_capture::Response::Refused {
            error: vw_capture::Error::ColorDepth,
            color_depth: Some(vw_capture::ColorDepthDiagnostic::NonopaquePixel { alpha: 228 }),
            alpha_summary: None,
        })
        .map_err(|_| Error::Invalid)?;
        let (mut refusal, mut color, mut alpha) = (None, None, None);
        assert!(canvas_response(&bytes, &mut refusal, &mut color, &mut alpha).is_err());
        assert_eq!(refusal, Some(vw_capture::Error::ColorDepth));
        assert_eq!(
            color,
            Some(vw_capture::ColorDepthDiagnostic::NonopaquePixel { alpha: 228 })
        );
        assert!(alpha.is_none());
        Ok(())
    }
    #[test]
    fn canvas_sampling_is_explicit_and_ordinary_default_stays_full_client() -> Result<()> {
        assert_eq!(SamplingScope::default(), SamplingScope::FullClient);
        assert_eq!(
            serde_json::from_str::<SamplingScope>("\"verified_editor_canvas\"")
                .map_err(|_| Error::Invalid)?,
            SamplingScope::VerifiedEditorCanvas
        );
        assert!(serde_json::from_str::<SamplingScope>("\"canvas\"").is_err());
        Ok(())
    }
}
