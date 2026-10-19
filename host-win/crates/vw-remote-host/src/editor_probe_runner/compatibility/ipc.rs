//! Closed request/authorization transcript for the single Paint drawing child.
//! The only writable authorization is a fixed one-byte grant or denial.
use super::*;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    os::windows::io::AsRawHandle,
};
use vw_remote::wire::FiniteRetirement;
use windows_sys::Win32::System::Pipes::PeekNamedPipe;

/// Serializes the root's authorization byte with any parent-side cancellation
/// kill. Once closed, no callback can grant a later action.
pub(crate) struct AuthorizationGate(Mutex<bool>);
impl AuthorizationGate {
    pub(crate) fn new() -> Self {
        Self(Mutex::new(false))
    }
    pub(crate) fn authorize(
        &self,
        uncertain: &AtomicBool,
        callback: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let closed = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if *closed {
            return Err(Error::Cancelled);
        }
        uncertain.store(true, Ordering::Release);
        callback()
    }
    pub(crate) fn close_if_safe(
        &self,
        uncertain: &AtomicBool,
        unresolved: &AtomicBool,
        kill: impl FnOnce(),
    ) -> bool {
        let mut closed = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if *closed || uncertain.load(Ordering::Acquire) || unresolved.load(Ordering::Acquire) {
            return false;
        }
        *closed = true;
        kill();
        true
    }
}

pub(super) const MAX_LINE: usize = 16 * 1024;
pub(super) const MAX_TRANSCRIPT: usize = 256 * 1024;
pub(super) const GRANT: u8 = b'G';
pub(super) const DENY: u8 = b'D';

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Step {
    CanvasDot,
    Undo,
    Redo,
    FinalUndo,
}
impl Step {
    pub(super) const ORDER: [Self; 4] = [Self::CanvasDot, Self::Undo, Self::Redo, Self::FinalUndo];
    pub(super) fn authorize(self) -> PaintDrawingInputStep {
        match self {
            Self::CanvasDot => PaintDrawingInputStep::CanvasDot,
            Self::Undo | Self::FinalUndo => PaintDrawingInputStep::Undo,
            Self::Redo => PaintDrawingInputStep::Redo,
        }
    }
    pub(super) fn action(self) -> Action {
        match self {
            Self::CanvasDot => Action::CanvasDot,
            Self::Undo | Self::FinalUndo => Action::Undo,
            Self::Redo => Action::Redo,
        }
    }
    pub(super) fn samples_before(self) -> usize {
        match self {
            Self::CanvasDot => 2,
            Self::Undo => 4,
            Self::Redo => 6,
            Self::FinalUndo => 8,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Sample {
    pub index: usize,
    pub target: Target,
    pub image: ImageIdentity,
    pub context_sha256: String,
    pub before_witness: PaintFrameWitness,
    pub witness: PaintFrameWitness,
    pub pointer_before: [i64; 2],
    pub pointer_after: [i64; 2],
    pub os_cursor_capture_disabled: bool,
    pub frame: vw_capture::CanvasFrameReceipt,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Click {
    pub step: Step,
    pub action: Action,
    pub target: Target,
    pub image: ImageIdentity,
    pub context_sha256: String,
    pub before: PaintFrameWitness,
    pub after: PaintFrameWitness,
    pub destination: [i32; 2],
    pub native_accepted_count: u32,
    pub expected_count: u32,
    pub accepted_qpc_100ns: Option<u64>,
    pub input_retirement_complete: bool,
    pub held_count: u32,
    pub had_error: bool,
    pub pointer_after: [i64; 2],
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Line {
    Sample {
        value: Sample,
    },
    Ready {
        step: Step,
        target: Target,
        image: ImageIdentity,
        context_sha256: String,
        witness: PaintFrameWitness,
    },
    Terminal {
        value: Click,
    },
    Finished {
        schema: u32,
        prior_history_restoration_unverified: bool,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ObservedClick {
    pub report: Click,
    pub receipt_sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Transcript {
    pub samples: Vec<Sample>,
    pub clicks: Vec<ObservedClick>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceHeldTerminal {
    schema: u32,
    operation: String,
    retirement: FiniteRetirement,
}
pub(super) fn read_source_held_terminal(output: &mut impl Read) -> Result<()> {
    let first = line(&mut *output)?;
    let record: SourceHeldTerminal =
        serde_json::from_slice(&first).map_err(|_| Error::RetirementPending)?;
    if record.schema != 1 || record.operation != "finite-retirement" {
        return Err(Error::RetirementPending);
    }
    match record.retirement {
        FiniteRetirement::Complete => Ok(()),
        FiniteRetirement::Pending { .. } => {
            let second = line(output)?;
            let complete: SourceHeldTerminal =
                serde_json::from_slice(&second).map_err(|_| Error::RetirementPending)?;
            if complete.schema == 1
                && complete.operation == "finite-retirement"
                && matches!(complete.retirement, FiniteRetirement::Complete)
            {
                Ok(())
            } else {
                Err(Error::RetirementPending)
            }
        }
    }
}

pub(super) fn line(mut reader: impl Read) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut one = [0u8; 1];
    while reader.read(&mut one)? != 0 {
        if bytes.len() >= MAX_LINE {
            return Err(Error::Limit);
        }
        if one[0] == b'\n' {
            return Ok(bytes);
        }
        bytes.push(one[0]);
    }
    Err(Error::Io)
}
pub(super) fn send(mut writer: impl Write, value: &Line) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| Error::Invalid)?;
    if bytes.len() > MAX_LINE {
        return Err(Error::Limit);
    }
    writer.write_all(&bytes)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}
fn sha(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}
fn output_empty(output: &File) -> Result<()> {
    let mut available = 0u32;
    // SAFETY: this is the sole live reader of the dedicated anonymous child pipe.
    let ok = unsafe {
        PeekNamedPipe(
            output.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut available,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 || available != 0 {
        return Err(Error::Ungranted);
    }
    Ok(())
}
pub(super) fn blank_white(first: &[u8], second: &[u8]) -> Result<()> {
    if first != second
        || first.is_empty()
        || !first.len().is_multiple_of(4)
        || first
            .chunks_exact(4)
            .any(|pixel| pixel != [255, 255, 255, 255])
    {
        Err(Error::Unavailable)
    } else {
        Ok(())
    }
}
pub(super) fn admitted_dot_delta(
    before: &[u8],
    after: &[u8],
    width: u32,
    dot: [i64; 2],
) -> Result<()> {
    let width = usize::try_from(width).map_err(|_| Error::Limit)?;
    if width == 0 || before.len() != after.len() || before.len() % 4 != 0 {
        return Err(Error::Invalid);
    }
    let mut center_hit = false;
    for (index, (previous, current)) in before
        .chunks_exact(4)
        .zip(after.chunks_exact(4))
        .enumerate()
    {
        if previous == current {
            continue;
        }
        let x = i64::try_from(index % width).map_err(|_| Error::Limit)?;
        let y = i64::try_from(index / width).map_err(|_| Error::Limit)?;
        let dx = (x - dot[0]).abs();
        let dy = (y - dot[1]).abs();
        if dx > 64 || dy > 64 {
            return Err(Error::Unavailable);
        }
        center_hit |= dx <= 1 && dy <= 1;
    }
    if center_hit {
        Ok(())
    } else {
        Err(Error::Unavailable)
    }
}
/// A refusal-only guard while the child waits at Ready. The same files are
/// independently decoded again after process/Job/IO retirement for proof.
fn preflight_pixels(
    step: Step,
    transcript: &Transcript,
    pixels: &mut Vec<Vec<u8>>,
    expected: &Request,
    capture_target: &vw_capture::WindowTarget,
    source: &SourceLease,
    cancel: &Cancellation,
    current: &Current,
) -> Result<()> {
    let baseline = transcript.samples.first().ok_or(Error::Invalid)?;
    let canvas = baseline.witness.canvas_rect_host;
    let parked = baseline.pointer_before;
    if !helper::outside(parked, canvas) {
        return Err(Error::Unavailable);
    }
    for index in pixels.len()..step.samples_before() {
        cancel.check()?;
        current()?;
        source.verify_observation(&expected.target, expected.owner_pid)?;
        let sample = &transcript.samples[index];
        let timestamp = sample.frame.source_identity.monotonic_timestamp_ns;
        if sample.witness.canvas_rect_host != canvas
            || sample.before_witness.canvas_rect_host != canvas
            || sample.witness.canvas_runtime_id_hash != baseline.witness.canvas_runtime_id_hash
            || sample.before_witness.canvas_runtime_id_hash
                != baseline.witness.canvas_runtime_id_hash
            || sample.witness.settings_digest != expected.settings_digest
            || sample.before_witness.settings_digest != expected.settings_digest
            || sample.witness.canvas_tool_name != "Using Brush tool on Canvas"
            || sample.pointer_before != parked
            || sample.pointer_after != parked
            || !sample.os_cursor_capture_disabled
            || sample
                .before_witness
                .observed_end_qpc_100ns
                .checked_mul(100)
                .is_none_or(|v| v > timestamp)
            || sample
                .witness
                .observed_start_qpc_100ns
                .checked_mul(100)
                .is_none_or(|v| v < timestamp)
        {
            return Err(Error::TargetChanged);
        }
        let path = expected.sample_directory(index)?.join("capture-canvas.png");
        let pin = Pin::open(&path, false)?;
        pixels.push(frame::joined_canvas_rgba(
            &pin,
            &sample.frame,
            capture_target,
            canvas,
            expected.owner_pid,
            &expected.capture_session_id,
            capture_target.observed_ns / 100,
            evidence::MAX_PIXELS,
        )?);
        source.verify_observation(&expected.target, expected.owner_pid)?;
        cancel.check()?;
        current()?;
    }
    let pair = step.samples_before() - 2;
    if pixels[pair] != pixels[pair + 1] {
        return Err(Error::Unavailable);
    }
    match step {
        Step::CanvasDot => {
            blank_white(&pixels[0], &pixels[1])?;
        }
        Step::Undo => {
            if pixels[2] == pixels[0] {
                return Err(Error::Unavailable);
            }
            let dot = &transcript.clicks.first().ok_or(Error::Invalid)?.report;
            let cx = i64::from(dot.destination[0]) - i64::from(canvas.x);
            let cy = i64::from(dot.destination[1]) - i64::from(canvas.y);
            if cx < 64
                || cy < 64
                || cx + 64 >= i64::from(canvas.width)
                || cy + 64 >= i64::from(canvas.height)
            {
                return Err(Error::Unavailable);
            }
            admitted_dot_delta(&pixels[0], &pixels[2], canvas.width, [cx, cy])?;
        }
        Step::Redo => {
            if pixels[4] != pixels[0] {
                return Err(Error::Unavailable);
            }
        }
        Step::FinalUndo => {
            if pixels[6] != pixels[2] {
                return Err(Error::Unavailable);
            }
        }
    }
    if pair > 0 {
        let receipt = &transcript.clicks[(pair / 2) - 1].report;
        let first = transcript.samples[pair]
            .frame
            .source_identity
            .monotonic_timestamp_ns
            / 100;
        let second = transcript.samples[pair + 1]
            .frame
            .source_identity
            .monotonic_timestamp_ns
            / 100;
        if first.saturating_sub(receipt.accepted_qpc_100ns.ok_or(Error::Invalid)?) < 1_000_000
            || second.saturating_sub(first) < 1_000_000
        {
            return Err(Error::Unavailable);
        }
    } else {
        let first = transcript.samples[0]
            .frame
            .source_identity
            .monotonic_timestamp_ns
            / 100;
        let second = transcript.samples[1]
            .frame
            .source_identity
            .monotonic_timestamp_ns
            / 100;
        if second.saturating_sub(first) < 1_000_000 {
            return Err(Error::Unavailable);
        }
    }
    Ok(())
}

/// The IO worker holds both pipe ends and pins while this state machine runs.
/// Every Ready is phase-checked; uncertainty precedes the root guard callback.
pub(super) fn exchange(
    mut input: File,
    mut output: File,
    request: &[u8],
    expected: &Request,
    uncertain: &AtomicBool,
    current: &Current,
    authorize: &PaintDrawingAuthorize,
    cancel: &Cancellation,
    canvas_unresolved: &AtomicBool,
    gate: &AuthorizationGate,
    capture_target: &vw_capture::WindowTarget,
    source: &SourceLease,
) -> Result<Vec<u8>> {
    if request.len() > MAX_LINE {
        return Err(Error::Limit);
    }
    input.write_all(request)?;
    input.write_all(b"\n")?;
    input.flush()?;
    let mut result = Transcript {
        samples: Vec::with_capacity(10),
        clicks: Vec::with_capacity(4),
    };
    let mut phase_pixels = Vec::with_capacity(8);
    for step in Step::ORDER {
        while result.samples.len() < step.samples_before() {
            current()?;
            cancel.check()?;
            let raw = line(&mut output)?;
            let Line::Sample { value } =
                serde_json::from_slice(&raw).map_err(|_| Error::Invalid)?
            else {
                return Err(Error::Invalid);
            };
            value.before_witness.validate()?;
            value.witness.validate()?;
            if value.index != result.samples.len()
                || value.target != expected.target
                || value.image != expected.image
                || value.context_sha256 != expected.context_sha256
            {
                return Err(Error::TargetChanged);
            }
            result.samples.push(value);
            current()?;
        }
        current()?;
        cancel.check()?;
        let raw = line(&mut output)?;
        let Line::Ready {
            step: actual,
            target,
            image,
            context_sha256,
            witness,
        } = serde_json::from_slice(&raw).map_err(|_| Error::Invalid)?
        else {
            return Err(Error::Invalid);
        };
        witness.validate()?;
        if actual != step
            || target != expected.target
            || image != expected.image
            || context_sha256 != expected.context_sha256
            || witness.settings_digest != expected.settings_digest
            || witness.canvas_tool_name != "Using Brush tool on Canvas"
            || result.samples.first().is_some_and(|sample| {
                sample.witness.canvas_runtime_id_hash != witness.canvas_runtime_id_hash
                    || sample.witness.canvas_rect_host != witness.canvas_rect_host
            })
        {
            return Err(Error::TargetChanged);
        }
        preflight_pixels(
            step,
            &result,
            &mut phase_pixels,
            expected,
            capture_target,
            source,
            cancel,
            current,
        )?;
        output_empty(&output)?;
        // No other code may clear this before the exact source-held terminal.
        let mut sent = false;
        let mut grant = || -> Result<()> {
            if sent {
                return Err(Error::Invalid);
            }
            if step == Step::CanvasDot {
                canvas_unresolved.store(true, Ordering::Release);
            }
            input.write_all(&[GRANT])?;
            input.flush()?;
            sent = true;
            Ok(())
        };
        let permission = gate.authorize(uncertain, || authorize(step.authorize(), &mut grant));
        if !sent {
            input.write_all(&[DENY])?;
            input.flush()?;
        }
        read_source_held_terminal(&mut output)?;
        let raw = line(&mut output)?;
        let Line::Terminal { value } = serde_json::from_slice(&raw).map_err(|_| Error::Invalid)?
        else {
            return Err(Error::Invalid);
        };
        value.before.validate()?;
        value.after.validate()?;
        if value.step != step
            || value.action != step.action()
            || value.target != expected.target
            || value.image != expected.image
            || value.context_sha256 != expected.context_sha256
            || value.before.settings_digest != expected.settings_digest
            || value.after.settings_digest != expected.settings_digest
            || value.before.canvas_runtime_id_hash != witness.canvas_runtime_id_hash
            || value.after.canvas_runtime_id_hash != witness.canvas_runtime_id_hash
            || value.before.canvas_rect_host != witness.canvas_rect_host
            || value.after.canvas_rect_host != witness.canvas_rect_host
            || !value.input_retirement_complete
            || value.held_count != 0
        {
            return Err(Error::RetirementPending);
        }
        let admitted = permission.is_ok()
            && sent
            && value.expected_count == 4
            && value.native_accepted_count == 4
            && !value.had_error
            && value.accepted_qpc_100ns.is_some_and(|v| v != 0);
        let denied = permission.is_err()
            && !sent
            && value.expected_count == 0
            && value.native_accepted_count == 0
            && !value.had_error;
        if !admitted && !denied {
            return Err(Error::PartialInput);
        }
        uncertain.store(false, Ordering::Release);
        if !admitted {
            return Err(permission.err().unwrap_or(Error::Ungranted));
        }
        result.clicks.push(ObservedClick {
            report: value,
            receipt_sha256: sha(&raw),
        });
        current()?;
        cancel.check()?;
    }
    while result.samples.len() < 10 {
        current()?;
        cancel.check()?;
        let raw = line(&mut output)?;
        let Line::Sample { value } = serde_json::from_slice(&raw).map_err(|_| Error::Invalid)?
        else {
            return Err(Error::Invalid);
        };
        value.before_witness.validate()?;
        value.witness.validate()?;
        if value.index != result.samples.len()
            || value.target != expected.target
            || value.image != expected.image
            || value.context_sha256 != expected.context_sha256
        {
            return Err(Error::TargetChanged);
        }
        result.samples.push(value);
        current()?;
    }
    let raw = line(&mut output)?;
    if !matches!(
        serde_json::from_slice::<Line>(&raw),
        Ok(Line::Finished {
            schema: 1,
            prior_history_restoration_unverified: true
        })
    ) {
        return Err(Error::Unavailable);
    }
    drop(input);
    let mut extra = [0u8; 1];
    if output.read(&mut extra)? != 0 {
        return Err(Error::Invalid);
    }
    let bytes = serde_json::to_vec(&result).map_err(|_| Error::Invalid)?;
    if bytes.len() > MAX_TRANSCRIPT {
        return Err(Error::Limit);
    }
    Ok(bytes)
}
