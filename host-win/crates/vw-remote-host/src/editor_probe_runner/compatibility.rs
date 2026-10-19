//! Source-held, joined Paint dot/undo/redo/undo pixel producer. The public
//! preparation and completed result are opaque; neither accepts receipt bytes.
use super::*;
use crate::editor_source::SourceLease;
use crate::platform::paint_controls::{PaintFrameWitness, experiment::Action};
use crate::process::windows::Pin;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    sync::atomic::AtomicBool,
};
use vw_remote::{Target, profile::ImageIdentity};

#[path = "compatibility/evidence.rs"]
mod evidence;
mod helper;
mod ipc;
#[cfg(test)]
mod tests;
pub(super) use ipc::AuthorizationGate;

pub type PaintDrawingAuthorize =
    Arc<dyn Fn(PaintDrawingInputStep, &mut dyn FnMut() -> Result<()>) -> Result<()> + Send + Sync>;
pub type Current = Arc<dyn Fn() -> Result<()> + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaintDrawingInputStep {
    CanvasDot,
    Undo,
    Redo,
}
pub struct PaintDrawingPaths {
    pub observation_helper: PathBuf,
    pub observation_sha256: String,
    pub drawing_helper: PathBuf,
    pub drawing_sha256: String,
}
pub struct PaintDrawingPreparation {
    target: Target,
    source: Arc<SourceLease>,
    context_sha256: String,
    capture_session_id: String,
    marker_id: String,
    sample_id: String,
    settings_digest: String,
    drawing_path: PathBuf,
    drawing_pins: Arc<Pins>,
    _observation: PaintObservationHelper,
    cancel: Arc<Cancellation>,
    current: Current,
    authorize: PaintDrawingAuthorize,
}
pub struct CompletedPaintDrawingObservation {
    target: Target,
    image: ImageIdentity,
    tool_id: &'static str,
    settings_digest: String,
    evidence: evidence::DrawingEvidence,
    disposed_private_files: u32,
}
impl CompletedPaintDrawingObservation {
    pub fn target(&self) -> &Target {
        &self.target
    }
    pub fn image(&self) -> &ImageIdentity {
        &self.image
    }
    pub fn tool_id(&self) -> &str {
        self.tool_id
    }
    pub fn settings_digest(&self) -> &str {
        &self.settings_digest
    }
    pub fn evidence_sha256(&self) -> &str {
        &self.evidence.evidence_sha256
    }
    pub fn changed_pixels(&self) -> u64 {
        self.evidence.changed_pixels
    }
    pub fn changed_bounds(&self) -> [u32; 4] {
        self.evidence.changed_bounds
    }
    pub fn disposed_private_files(&self) -> u32 {
        self.disposed_private_files
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: u32,
    owner_pid: u32,
    target: Target,
    image: ImageIdentity,
    context_sha256: String,
    settings_digest: String,
    capture_session_id: String,
    private_root: PathBuf,
    sample_id: String,
}
impl Request {
    fn sample_directory(&self, index: usize) -> Result<PathBuf> {
        if index >= 10 {
            return Err(Error::Limit);
        }
        Ok(self
            .private_root
            .join(format!("{}-{index:02}", self.sample_id)))
    }
    fn validate(&self) -> Result<()> {
        if self.schema != 1
            || self.owner_pid == 0
            || self.image.executable_name != "mspaint.exe"
            || self.target.validate().is_err()
            || !vw_remote::profile::digest(&self.context_sha256)
            || !vw_remote::profile::digest(&self.settings_digest)
            || vw_remote::id(&self.capture_session_id).is_err()
            || vw_remote::id(&self.sample_id).is_err()
        {
            return Err(Error::Invalid);
        }
        let temp = frame::os_temp_root(&std::env::temp_dir())?;
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(4)
            .ok_or(Error::Invalid)?;
        let marker = self
            .private_root
            .file_name()
            .and_then(|v| v.to_str())
            .and_then(|v| v.strip_prefix("VisualWorkbench-editor-metrics-"))
            .ok_or(Error::Invalid)?;
        frame::temp_path(&self.private_root, &temp, workspace, marker)?;
        Ok(())
    }
}

/// Owns the exact private temp directory and its ten output slots. Cleanup is
/// deliberately retryable; a failed delete keeps this owner in RETAINED.
pub(super) struct PrivateTempLease {
    root: PathBuf,
    sample_id: String,
    ancestor: Pin,
    root_pin: Option<Pin>,
    marker_pin: Option<Pin>,
    dirs: Vec<Option<Pin>>,
    removed: u32,
}
static RETAINED_PRIVATE: OnceLock<Mutex<Vec<PrivateTempLease>>> = OnceLock::new();
fn retain_private(lease: PrivateTempLease) {
    RETAINED_PRIVATE
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push(lease);
}
pub(super) fn retry_private_disposal() -> usize {
    let Some(registry) = RETAINED_PRIVATE.get() else {
        return 0;
    };
    let mut leases = registry.lock().unwrap_or_else(|error| error.into_inner());
    let mut index = 0;
    while index < leases.len() {
        if leases[index].dispose().is_ok() {
            leases.remove(index);
        } else {
            index += 1;
        }
    }
    leases.len()
}
impl PrivateTempLease {
    fn create(marker_id: &str, sample_id: &str, owner_pid: u32) -> Result<Self> {
        vw_remote::id(marker_id)?;
        vw_remote::id(sample_id)?;
        let temp = frame::os_temp_root(&std::env::temp_dir())?;
        let ancestor = Pin::open(&temp, true)?;
        let root = temp.join(format!("VisualWorkbench-editor-metrics-{marker_id}"));
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(4)
            .ok_or(Error::Invalid)?;
        frame::temp_path(&root, &temp, workspace, marker_id)?;
        local(&root)?;
        fs::create_dir(&root)?;
        let root_pin = match Pin::open(&root, true) {
            Ok(pin) => pin,
            Err(error) => {
                let _ = fs::remove_dir(&root);
                return Err(error);
            }
        };
        let mut lease = Self {
            root,
            sample_id: sample_id.into(),
            ancestor,
            root_pin: Some(root_pin),
            marker_pin: None,
            dirs: (0..10).map(|_| None).collect(),
            removed: 0,
        };
        if let Err(error) = lease.populate(marker_id, owner_pid) {
            if lease.dispose().is_err() {
                retain_private(lease);
            }
            return Err(error);
        }
        Ok(lease)
    }
    fn populate(&mut self, marker_id: &str, owner_pid: u32) -> Result<()> {
        let marker = self.root.join(".vw-editor-metrics-owner");
        let bytes = serde_json::to_vec(&serde_json::json!({"schema":1,"owner_pid":owner_pid,
            "nonce":marker_id}))
        .map_err(|_| Error::Invalid)?;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)?
            .write_all(&bytes)?;
        self.marker_pin = Some(Pin::open(&marker, false)?);
        for index in 0..10 {
            let path = self.root.join(format!("{}-{index:02}", self.sample_id));
            fs::create_dir(&path)?;
            self.dirs[index] = Some(Pin::open(&path, true)?);
        }
        self.verify()
    }
    fn verify(&self) -> Result<()> {
        self.ancestor.verify()?;
        self.root_pin.as_ref().ok_or(Error::Invalid)?.verify()?;
        self.marker_pin.as_ref().ok_or(Error::Invalid)?.verify()?;
        for dir in &self.dirs {
            if let Some(pin) = dir {
                pin.verify()?;
            }
        }
        Ok(())
    }
    fn png(&self, index: usize) -> Result<Pin> {
        self.verify()?;
        self.dirs
            .get(index)
            .and_then(Option::as_ref)
            .ok_or(Error::Invalid)?
            .verify()?;
        Pin::open(
            &self
                .root
                .join(format!("{}-{index:02}\\capture-canvas.png", self.sample_id)),
            false,
        )
    }
    pub(super) fn dispose(&mut self) -> Result<u32> {
        self.ancestor.verify()?;
        if let Some(pin) = &self.root_pin {
            pin.verify()?;
        }
        for index in 0..self.dirs.len() {
            if let Some(pin) = &self.dirs[index] {
                pin.verify()?;
            }
            let path = self.root.join(format!("{}-{index:02}", self.sample_id));
            let png = path.join("capture-canvas.png");
            if png.exists() {
                let file = Pin::open(&png, false)?;
                file.verify()?;
                drop(file);
                fs::remove_file(&png)?;
                self.removed += 1;
            }
            drop(self.dirs[index].take());
            if path.exists() {
                fs::remove_dir(&path)?;
            }
        }
        if let Some(pin) = &self.marker_pin {
            pin.verify()?;
        }
        drop(self.marker_pin.take());
        let marker = self.root.join(".vw-editor-metrics-owner");
        if marker.exists() {
            fs::remove_file(marker)?;
        }
        if let Some(pin) = &self.root_pin {
            pin.verify()?;
        }
        drop(self.root_pin.take());
        if self.root.exists() {
            fs::remove_dir(&self.root)?;
        }
        Ok(self.removed)
    }
}

/// Query-only preparation. The exact packaged observation and drawing helpers
/// are pinned before any state can be returned to a confirming owner.
pub fn prepare_paint_drawing(
    paths: PaintDrawingPaths,
    target: Target,
    source: Arc<SourceLease>,
    context_sha256: String,
    capture_session_id: String,
    marker_id: String,
    sample_id: String,
    cancel: Arc<Cancellation>,
    current: Current,
    authorize: PaintDrawingAuthorize,
) -> Result<PaintDrawingPreparation> {
    cancel.check()?;
    current()?;
    if source.installed_package_evidence().is_none()
        || !vw_remote::profile::digest(&context_sha256)
        || vw_remote::id(&capture_session_id).is_err()
        || vw_remote::id(&marker_id).is_err()
        || vw_remote::id(&sample_id).is_err()
    {
        return Err(Error::Invalid);
    }
    source.verify_observation(&target, std::process::id())?;
    let observation =
        PaintObservationHelper::open(&paths.observation_helper, &paths.observation_sha256)?;
    let drawing_pins = locked_image(
        &paths.drawing_helper,
        &paths.drawing_sha256,
        "vw-paint-drawing-helper.exe",
    )?;
    let prepared = observation.observe(&target, source.clone(), &cancel)?;
    if prepared.tool_id != "paint.brush" || !vw_remote::profile::digest(&prepared.settings_digest) {
        return Err(Error::Unavailable);
    }
    cancel.check()?;
    current()?;
    source.verify_observation(&target, std::process::id())?;
    Ok(PaintDrawingPreparation {
        target,
        source,
        context_sha256,
        capture_session_id,
        marker_id,
        sample_id,
        settings_digest: prepared.settings_digest,
        drawing_path: paths.drawing_helper,
        drawing_pins,
        cancel,
        current,
        authorize,
        _observation: observation,
    })
}

fn joined_click(
    report: &ipc::ObservedClick,
    pid: u32,
    code: u32,
    expected: &Request,
    canvas: &PaintFrameWitness,
) -> Result<evidence::JoinedClick> {
    let click = &report.report;
    if click.target != expected.target
        || click.image != expected.image
        || click.context_sha256 != expected.context_sha256
        || click.action != click.step.action()
        || click.before.settings_digest != expected.settings_digest
        || click.after.settings_digest != expected.settings_digest
        || click.before.canvas_runtime_id_hash != canvas.canvas_runtime_id_hash
        || click.after.canvas_runtime_id_hash != canvas.canvas_runtime_id_hash
        || click.before.canvas_rect_host != canvas.canvas_rect_host
        || click.after.canvas_rect_host != canvas.canvas_rect_host
        || click.accepted_qpc_100ns.is_none_or(|accepted| {
            accepted < click.before.observed_end_qpc_100ns
                || accepted > click.after.observed_start_qpc_100ns
        })
    {
        return Err(Error::TargetChanged);
    }
    Ok(evidence::JoinedClick {
        action: match click.step {
            ipc::Step::CanvasDot => evidence::ClickAction::CanvasDot,
            ipc::Step::Undo | ipc::Step::FinalUndo => evidence::ClickAction::Undo,
            ipc::Step::Redo => evidence::ClickAction::Redo,
        },
        context_sha256: click.context_sha256.clone(),
        canvas_runtime_sha256: canvas.canvas_runtime_id_hash.clone(),
        settings_sha256: click.before.settings_digest.clone(),
        process_id: pid,
        process_exit_code: code,
        process_and_job_retired: true,
        io_joined: true,
        native_accepted_count: click.native_accepted_count,
        expected_count: click.expected_count,
        input_retirement_complete: click.input_retirement_complete,
        held_count: click.held_count,
        had_error: click.had_error,
        accepted_qpc_100ns: click.accepted_qpc_100ns.ok_or(Error::Invalid)?,
        receipt_sha256: report.receipt_sha256.clone(),
    })
}

fn evaluate(
    owner: &Owner,
    expected: &Request,
    target: &vw_capture::WindowTarget,
    transcript: &ipc::Transcript,
    cancel: &Cancellation,
    current: &Current,
    pid: u32,
    code: u32,
) -> Result<evidence::DrawingEvidence> {
    if code != 0
        || transcript.samples.len() != 10
        || transcript.clicks.len() != 4
        || owner.input_uncertain.load(Ordering::Acquire)
    {
        return Err(Error::RetirementPending);
    }
    let temp = owner.private_temp.as_ref().ok_or(Error::Invalid)?;
    temp.verify()?;
    let first = &transcript.samples[0].witness;
    let canvas = first.canvas_rect_host;
    let parked = transcript.samples[0].pointer_before;
    let relative_pointer = [
        parked[0]
            .checked_sub(i64::from(canvas.x))
            .ok_or(Error::Limit)?,
        parked[1]
            .checked_sub(i64::from(canvas.y))
            .ok_or(Error::Limit)?,
    ];
    if !helper::outside(parked, canvas)
        || transcript
            .clicks
            .iter()
            .any(|v| v.report.pointer_after != parked)
    {
        return Err(Error::TargetChanged);
    }
    let mut pixels = Vec::with_capacity(10);
    for (index, sample) in transcript.samples.iter().enumerate() {
        cancel.check()?;
        current()?;
        (owner
            ._source
            .as_ref()
            .ok_or(Error::Invalid)?
            .verify_observation(&expected.target, expected.owner_pid))?;
        if sample.index != index
            || sample.target != expected.target
            || sample.image != expected.image
            || sample.context_sha256 != expected.context_sha256
            || sample.witness.settings_digest != expected.settings_digest
            || sample.before_witness.settings_digest != expected.settings_digest
            || sample.before_witness.canvas_runtime_id_hash != first.canvas_runtime_id_hash
            || sample.before_witness.canvas_rect_host != canvas
            || sample.witness.canvas_runtime_id_hash != first.canvas_runtime_id_hash
            || sample.witness.canvas_rect_host != canvas
            || sample.witness.canvas_tool_name != "Using Brush tool on Canvas"
            || sample.pointer_before != parked
            || sample.pointer_after != parked
            || sample
                .before_witness
                .observed_end_qpc_100ns
                .checked_mul(100)
                .is_none_or(|v| v > sample.frame.source_identity.monotonic_timestamp_ns)
            || sample
                .witness
                .observed_start_qpc_100ns
                .checked_mul(100)
                .is_none_or(|v| v < sample.frame.source_identity.monotonic_timestamp_ns)
        {
            return Err(Error::TargetChanged);
        }
        let pin = temp.png(index)?;
        pixels.push(frame::joined_canvas_rgba(
            &pin,
            &sample.frame,
            target,
            canvas,
            expected.owner_pid,
            &expected.capture_session_id,
            target.observed_ns / 100,
            evidence::MAX_PIXELS,
        )?);
        cancel.check()?;
        current()?;
    }
    let frames = transcript
        .samples
        .iter()
        .zip(&pixels)
        .map(|(sample, rgba)| evidence::Canvas {
            context_sha256: &sample.context_sha256,
            canvas_runtime_sha256: &sample.witness.canvas_runtime_id_hash,
            settings_sha256: &sample.witness.settings_digest,
            tool_id: "paint.brush",
            width: sample.frame.width,
            height: sample.frame.height,
            captured_qpc_100ns: sample.frame.source_identity.monotonic_timestamp_ns / 100,
            os_cursor_capture_disabled: sample.os_cursor_capture_disabled,
            pointer_before: relative_pointer,
            pointer_after: relative_pointer,
            rgba,
        })
        .collect::<Vec<_>>();
    let clicks = transcript
        .clicks
        .iter()
        .map(|v| joined_click(v, pid, code, expected, first))
        .collect::<Result<Vec<_>>>()?;
    let dot = &transcript.clicks[0].report;
    let x = i64::from(dot.destination[0]) - i64::from(canvas.x);
    let y = i64::from(dot.destination[1]) - i64::from(canvas.y);
    let dot_in_canvas = [
        u32::try_from(x).map_err(|_| Error::Invalid)?,
        u32::try_from(y).map_err(|_| Error::Invalid)?,
    ];
    evidence::evaluate(evidence::DrawingCheck {
        before: [frames[0], frames[1]],
        after: [frames[2], frames[3]],
        restored: [frames[4], frames[5]],
        redone: [frames[6], frames[7]],
        final_restored: [frames[8], frames[9]],
        dot: &clicks[0],
        undo: &clicks[1],
        redo: clicks[2].clone(),
        final_undo: clicks[3].clone(),
        dot_in_canvas,
        radius_pixels: 64,
    })
}

impl PaintDrawingPreparation {
    /// Consuming this object is the only execution entry. The root confirmation
    /// owner has already acquired its exclusive input/idle barrier.
    pub fn run_confirmed(self) -> Result<CompletedPaintDrawingObservation> {
        self.cancel.check()?;
        (self.current)()?;
        self.source
            .verify_observation(&self.target, std::process::id())?;
        self.drawing_pins.verify()?;
        if retry_private_disposal() != 0 {
            return Err(Error::RetirementPending);
        }
        let permit = Permit::acquire()?;
        let private_root = frame::os_temp_root(&std::env::temp_dir())?
            .join(format!("VisualWorkbench-editor-metrics-{}", self.marker_id));
        let request = Request {
            schema: 1,
            owner_pid: std::process::id(),
            target: self.target.clone(),
            image: self.source.identity().clone(),
            context_sha256: self.context_sha256.clone(),
            settings_digest: self.settings_digest.clone(),
            capture_session_id: self.capture_session_id,
            private_root,
            sample_id: self.sample_id,
        };
        request.validate()?;
        let expected_capture =
            vw_capture::windows::fixed_target(request.target.window, request.owner_pid)
                .map_err(|_| Error::TargetChanged)?;
        let frame_pixels = u64::from(expected_capture.frame.width)
            .checked_mul(u64::from(expected_capture.frame.height))
            .ok_or(Error::Limit)?;
        if frame_pixels > 4 * 1024 * 1024 {
            return Err(Error::Limit);
        }
        frame::drawing_capacity(evidence::MAX_PIXELS, frame_pixels)?;
        let bytes = serde_json::to_vec(&request).map_err(|_| Error::Invalid)?;
        if bytes.len() > ipc::MAX_LINE {
            return Err(Error::Limit);
        }
        let temp =
            PrivateTempLease::create(&self.marker_id, &request.sample_id, request.owner_pid)?;
        if let Err(error) = self.cancel.check().and_then(|()| (self.current)()) {
            return fail_prelaunch(temp, error);
        }
        let mut child = match Child::launch(&self.drawing_path, &self.drawing_pins, &self.cancel) {
            Ok(child) => child,
            Err(error) => return fail_prelaunch(temp, error),
        };
        let startup = child.startup_error.clone();
        let pipes = Arc::new(Mutex::new(Pipes {
            input: child.stdin.take(),
            output: child.stdout.take(),
        }));
        let gate = Arc::new(AuthorizationGate::new());
        let preview_source = self.source.clone();
        let mut owner = Owner {
            child,
            pins: self.drawing_pins,
            pipes: pipes.clone(),
            io: None,
            parked_outcome: None,
            _permit: permit,
            input_uncertain: Arc::new(AtomicBool::new(false)),
            _source: Some(self.source),
            private_temp: Some(temp),
            canvas_unresolved: Arc::new(AtomicBool::new(false)),
            drawing_gate: Some(gate.clone()),
        };
        let uncertain = owner.input_uncertain.clone();
        let canvas_unresolved = owner.canvas_unresolved.clone();
        let current = self.current.clone();
        let authorize = self.authorize.clone();
        let cancel = self.cancel.clone();
        let expected = request.clone();
        let preview_target = expected_capture.clone();
        let pins = owner.pins.clone();
        let thread = thread::Builder::new()
            .name("paint-drawing-io".into())
            .spawn(move || {
                let (input, output) = {
                    let mut p = pipes.lock().unwrap_or_else(|e| e.into_inner());
                    (p.input.take(), p.output.take())
                };
                let Some(output) = output else {
                    return Outcome {
                        _input: input,
                        _output: None,
                        bytes: Err(Error::Io),
                        parent_exchange: None,
                    };
                };
                let result = match input {
                    Some(input) => output.try_clone().map_err(Error::from).and_then(|reader| {
                        ipc::exchange(
                            input,
                            reader,
                            &bytes,
                            &expected,
                            &uncertain,
                            &current,
                            &authorize,
                            &cancel,
                            &canvas_unresolved,
                            &gate,
                            &preview_target,
                            &preview_source,
                        )
                    }),
                    None => Err(Error::Io),
                };
                let _ = pins.verify();
                Outcome {
                    _input: None,
                    _output: Some(output),
                    bytes: result,
                    parent_exchange: None,
                }
            });
        match thread {
            Ok(worker) => owner.io = Some(worker),
            Err(_) => return retain(owner, Error::Io),
        }
        if let Some(error) = startup {
            return retain(owner, error);
        }
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(90) {
            if let Err(error) = self.cancel.check().and_then(|()| (self.current)()) {
                owner.kill_if_released();
                return retain(owner, error);
            }
            if owner.retired().unwrap_or(false) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        if !owner.retired().unwrap_or(false) {
            owner.kill_if_released();
            return retain(owner, Error::Timeout);
        }
        let Collection::Joined { pid, code, outcome } = owner.collect() else {
            return retain(owner, Error::RetirementPending);
        };
        let transcript = match outcome
            .and_then(|v| v.ok_or(Error::Io))
            .and_then(|v| v.bytes)
            .and_then(|raw| {
                serde_json::from_slice::<ipc::Transcript>(&raw).map_err(|_| Error::Invalid)
            }) {
            Ok(v) => v,
            Err(error) => return retain(owner, error),
        };
        if let Err(error) = (self.current)() {
            return retain(owner, error);
        }
        let evidence = match evaluate(
            &owner,
            &request,
            &expected_capture,
            &transcript,
            &self.cancel,
            &self.current,
            pid,
            code,
        ) {
            Ok(v) => v,
            Err(error) => return retain(owner, error),
        };
        owner.canvas_unresolved.store(false, Ordering::Release);
        if let Err(error) = (self.current)() {
            return retain(owner, error);
        }
        let disposal = match owner.private_temp.as_mut() {
            Some(temp) => temp.dispose(),
            None => Err(Error::Invalid),
        };
        let disposed = match disposal {
            Ok(v) => v,
            Err(error) => return retain(owner, error),
        };
        owner.private_temp = None;
        if disposed != 10 {
            return Err(Error::Invalid);
        }
        Ok(CompletedPaintDrawingObservation {
            target: request.target,
            image: request.image,
            tool_id: "paint.brush",
            settings_digest: request.settings_digest,
            evidence,
            disposed_private_files: disposed,
        })
    }
}
fn retain(owner: Owner, error: Error) -> Result<CompletedPaintDrawingObservation> {
    let mut receipt = receipt(&owner, "", Some(error));
    receipt.retirement_error = Some(Error::RetirementPending);
    RETAINED
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(PendingOwner {
            owner,
            receipt,
            quiet: true,
        });
    Err(Error::RetirementPending)
}
fn fail_prelaunch(
    mut temp: PrivateTempLease,
    error: Error,
) -> Result<CompletedPaintDrawingObservation> {
    if temp.dispose().is_err() {
        retain_private(temp);
        return Err(Error::RetirementPending);
    }
    Err(error)
}
pub fn run_paint_drawing_helper() -> Result<()> {
    helper::run()
}
