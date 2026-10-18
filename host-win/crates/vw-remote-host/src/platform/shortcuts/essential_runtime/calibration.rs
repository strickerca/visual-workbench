//! Isolated root-owned calibration. No static install, catalog or peer input API.
//! Startup opens retained source and observes facts. A distinct ready/execute
//! rendezvous then measures the same production preparation and finite stages.
use super::*;
use crate::{
    editor_effect::terminal,
    editor_source::SourcePolicy,
    platform::{
        finite_input::{FiniteAttempt, FiniteInputOwner},
        input::stages,
    },
};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Read, Write};

pub(crate) const SETUP: &[u8] = b"M4_ESSENTIAL_CALIBRATION_SETUP_V1\n";
pub(crate) const READY: &[u8] = b"M4_ESSENTIAL_CALIBRATION_READY_V1\n";
pub(crate) const EXECUTE: &[u8] = b"M4_ESSENTIAL_CALIBRATION_EXECUTE_V1\n";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invocation {
    mode: String,
    request: Request,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: u32,
    owner_pid: u32,
    target: Target,
    image: profile::ImageIdentity,
    editor: Editor,
    source_policy: SourcePolicy,
    candidate: ActionSpec,
    mutable_size: Option<RangeShape>,
    owned_blank_fixture: bool,
    consent_to_finite_input: bool,
    expected_observation_digest: Option<String>,
}
#[derive(Serialize)]
struct Observation {
    context_digest: String,
    candidate: ActionSpec,
    mutable_size: Option<RangeShape>,
    canvas_runtime_hash: String,
    watched_settings_digest: String,
    actual: Proof,
}
impl Observation {
    fn capture(
        context_digest: &str,
        mutable_size: Option<&RangeShape>,
        target: &Target,
        action: &ActionSpec,
        proof: &Proof,
    ) -> Result<Self> {
        let mut actual = proof.clone();
        // Sampling time is carried by live native proofs but cannot make equal
        // rooted facts hash differently between observation and execution.
        if let Proof::Krita(value) = &mut actual {
            value.canvas.sampled_qpc_100ns = 0;
        }
        let watched = match proof {
            Proof::Krita(value) => &value.watched_digest,
            Proof::Paint(value) => &value.watched_settings_digest,
        };
        Ok(Self {
            context_digest: context_digest.into(),
            candidate: action.clone(),
            mutable_size: mutable_size.cloned(),
            canvas_runtime_hash: proof.canvas(target, context_digest)?.runtime_id_hash,
            watched_settings_digest: watched.clone(),
            actual,
        })
    }
    fn digest(&self) -> Result<String> {
        Ok(sha256(
            &serde_json::to_vec(self).map_err(|_| Error::Invalid)?,
        ))
    }
}
// Observational calibration-only timing. These marks cannot renew a provider
// budget or authorize input. There are at most twelve marks in a two-stage run;
// the fixed cap also bounds diagnostics after an unexpected future extension.
#[derive(Serialize)]
struct StageTiming {
    phase: &'static str,
    boundary: &'static str,
    elapsed_us: u64,
    remaining_us: Option<u64>,
    error: Option<Error>,
    attempt: Option<FiniteAttempt>,
}
#[derive(Serialize)]
struct CalibrationTiming {
    schema: u32,
    clock_origin: &'static str,
    provider_budget_ms: u32,
    queue_time_measured: bool,
    send_input_dispatch_measured: bool,
    max_rows: usize,
    truncated: bool,
    rows: Vec<StageTiming>,
    #[serde(skip)]
    started: Instant,
    #[serde(skip)]
    deadline: Option<Instant>,
}
impl CalibrationTiming {
    fn new(started: Instant, provider_budget_ms: u32) -> Self {
        Self {
            schema: 1,
            clock_origin: "execute-token-read",
            provider_budget_ms,
            queue_time_measured: false,
            send_input_dispatch_measured: false,
            max_rows: 16,
            truncated: false,
            rows: Vec::with_capacity(16),
            started,
            deadline: started.checked_add(Duration::from_millis(u64::from(provider_budget_ms))),
        }
    }
    fn mark(
        &mut self,
        phase: &'static str,
        boundary: &'static str,
        error: Option<Error>,
        attempt: Option<&FiniteAttempt>,
    ) {
        if self.rows.len() == self.max_rows {
            self.truncated = true;
            return;
        }
        let now = Instant::now();
        let micros = |duration: Duration| duration.as_micros().min(u128::from(u64::MAX)) as u64;
        self.rows.push(StageTiming {
            phase,
            boundary,
            elapsed_us: micros(now.saturating_duration_since(self.started)),
            remaining_us: self
                .deadline
                .map(|end| micros(end.saturating_duration_since(now))),
            error,
            attempt: attempt.cloned(),
        });
    }
}
struct MeasuredStages<'a> {
    target: &'a Target,
    owner_pid: u32,
    owner: &'a mut FiniteInputOwner<Context>,
    command: &'a Command,
    timing: &'a mut CalibrationTiming,
}
impl stages::Stages for MeasuredStages<'_> {
    fn first(&mut self) -> FiniteAttempt {
        let stage = self.command.first_stage();
        let timing = &mut *self.timing;
        timing.mark("stage-one-owner-send", "enter", None, None);
        let attempt = self
            .owner
            .send(stage.inputs(), stage.destination(), |context| {
                timing.mark("stage-one-validation", "enter", None, None);
                let result =
                    validate_with(context, self.target, self.owner_pid, self.command, stage);
                timing.mark(
                    "stage-one-validation",
                    "return",
                    result.as_ref().err().cloned(),
                    None,
                );
                result
            });
        timing.mark(
            "stage-one-owner-send",
            "return",
            attempt.error.clone(),
            Some(&attempt),
        );
        attempt
    }
    fn second(&mut self, first: &FiniteAttempt) -> Result<Option<FiniteAttempt>> {
        // stages::execute admits this call only after this owner's actual
        // complete/error-free/count-exact first receipt. No fresh deadline.
        self.timing
            .mark("stage-two-preparation", "enter", None, None);
        let result = second_with(
            self.owner.source(),
            self.target,
            self.owner_pid,
            self.command,
        );
        self.timing.mark(
            "stage-two-preparation",
            "return",
            result.as_ref().err().cloned(),
            None,
        );
        let Some(stage) = result? else {
            return Ok(None);
        };
        if stage
            .inputs()
            .len()
            .checked_add(first.expected_count as usize)
            .is_none_or(|n| n > 16)
        {
            return Err(Error::Limit);
        }
        let timing = &mut *self.timing;
        timing.mark("stage-two-owner-send", "enter", None, None);
        let attempt = self
            .owner
            .send(stage.inputs(), stage.destination(), |context| {
                timing.mark("stage-two-validation", "enter", None, None);
                let result =
                    validate_with(context, self.target, self.owner_pid, self.command, &stage);
                timing.mark(
                    "stage-two-validation",
                    "return",
                    result.as_ref().err().cloned(),
                    None,
                );
                result
            });
        timing.mark(
            "stage-two-owner-send",
            "return",
            attempt.error.clone(),
            Some(&attempt),
        );
        Ok(Some(attempt))
    }
}
fn request_valid(r: &Request, execute: bool) -> Result<()> {
    if r.schema != 1
        || r.owner_pid == 0
        || !r.owned_blank_fixture
        || execute != r.consent_to_finite_input
        || execute
            != r.expected_observation_digest
                .as_ref()
                .is_some_and(|v| profile::digest(v))
    {
        return Err(Error::Ungranted);
    }
    if !execute && r.expected_observation_digest.is_some() {
        return Err(Error::Invalid);
    }
    r.candidate.validate_candidate(r.editor)?;
    if !matches!(
        (r.editor, r.source_policy),
        (Editor::Krita, SourcePolicy::Ordinary) | (Editor::Paint, SourcePolicy::InstalledPaint)
    ) {
        return Err(Error::Unavailable);
    }
    let exact_image = match r.editor {
        Editor::Krita => {
            r.image.executable_name == "krita.exe"
                && r.image.executable_blake3
                    == "8e7e6999745d3f84cf402796717f86c05e39ba2398af0e3758888be333d50720"
                && r.image.executable_bytes == 273624
                && r.image.file_version.as_deref() == Some("5.3.4.0")
                && r.image.package_full_name.is_none()
                && r.image.package_version.is_none()
        }
        Editor::Paint => {
            r.image.executable_name == "mspaint.exe"
                && r.image.executable_blake3
                    == "b2a7f30ffa5f4864fbe63b49c64153f1d75a278f5771f14e59ffb6be4f65792f"
                && r.image.executable_bytes == 4_272_128
                && r.image.file_version.as_deref() == Some("11.2605.81.0")
                && r.image.package_full_name.as_deref()
                    == Some(vw_remote::profile::essential::PAINT_PACKAGE)
                && r.image.package_version.as_deref() == Some("11.2605.81.0")
        }
    };
    if !exact_image {
        return Err(Error::Unavailable);
    }
    if let Some(shape) = &r.mutable_size {
        // This is an explicitly nominated candidate size shape, not a semantic
        // claim. Its uniqueness and every non-size value remain observed.
        if r.editor != Editor::Krita {
            return Err(Error::Invalid);
        }
        shape.validate_wheel()?;
    }
    r.target.validate()
}
fn execute_mode(mode: &str) -> Result<bool> {
    Ok(match mode {
        "--essential-calibration-observe" => false,
        "--essential-calibration-experiment" => true,
        _ => return Err(Error::Invalid),
    })
}
fn context_digest(r: &Request) -> Result<String> {
    Ok(sha256(
        &serde_json::to_vec(&(
            "M4-Calibration-Context-v1",
            &r.target,
            &r.image,
            r.editor,
            &r.candidate,
            &r.mutable_size,
        ))
        .map_err(|_| Error::Invalid)?,
    ))
}
pub(crate) fn run(
    bytes: &[u8],
    input: &mut impl BufRead,
    producer: &mut terminal::Producer,
) -> Result<()> {
    let invocation: Invocation = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
    let execute = execute_mode(&invocation.mode)?;
    let r = invocation.request;
    request_valid(&r, execute)?;
    let cancel = Arc::new(vw_capture::Cancellation::default());
    let setup_budget = SourceBudget::new(cancel.clone(), Duration::from_secs(5))?;
    let source = SourceLease::open(
        &r.target,
        r.owner_pid,
        &r.image,
        r.source_policy,
        &setup_budget,
    )?;
    // This digest is explicitly a calibration context binding, never an admitted
    // catalog digest. It includes the exact selected target/image and candidate.
    let digest = context_digest(&r)?;
    let mut context = Context {
        editor: r.editor,
        identity: r.image.clone(),
        watched_settings_digest: None,
        mutable_size: r.mutable_size.clone(),
        source,
        digest,
        cancel,
    };
    let observed = read(
        &context,
        &r.target,
        r.owner_pid,
        &r.candidate,
        &context.budget(deadline(context.editor, Instant::now())?)?,
    )?;
    let before = Observation::capture(
        &context.digest,
        context.mutable_size.as_ref(),
        &r.target,
        &r.candidate,
        &observed,
    )?;
    let observation_digest = before.digest()?;
    context.watched_settings_digest = Some(before.watched_settings_digest.clone());
    if execute && r.expected_observation_digest.as_deref() != Some(observation_digest.as_str()) {
        return Err(Error::TargetChanged);
    }
    let mut owner = FiniteInputOwner::new(r.target.clone(), r.owner_pid, context)?;
    let mut attempt = None;
    let mut command_elapsed_us = None;
    let mut stage_timing = None;
    let result = (|| -> Result<()> {
        if !execute {
            return Ok(());
        }
        // Retained source/target owner is ready before the measured exchange.
        {
            let mut out = std::io::stdout().lock();
            out.write_all(READY)?;
            out.flush()?;
        }
        let mut token = Vec::new();
        Read::by_ref(input)
            .take(EXECUTE.len() as u64 + 1)
            .read_to_end(&mut token)?;
        if token != EXECUTE {
            return Err(Error::Invalid);
        }
        let started = Instant::now();
        let mut timing = CalibrationTiming::new(started, r.editor.provider_budget_ms());
        let command_result = (|| -> Result<()> {
            timing.mark("command-preparation", "enter", None, None);
            let prepared = prepare_with(
                owner.source(),
                &r.target,
                r.owner_pid,
                &owner.source().digest,
                r.candidate.clone(),
                &before.canvas_runtime_hash,
                started,
            );
            timing.mark(
                "command-preparation",
                "return",
                prepared.as_ref().err().cloned(),
                None,
            );
            let command = prepared?;
            if !command.proof.stable_eq(&observed) {
                return Err(Error::TargetChanged);
            }
            producer.arm_attempt()?;
            stages::execute(
                &mut MeasuredStages {
                    target: &r.target,
                    owner_pid: r.owner_pid,
                    owner: &mut owner,
                    command: &command,
                    timing: &mut timing,
                },
                &mut attempt,
            )?;
            owner
                .source()
                .budget(deadline(owner.source().editor, started)?)?
                .check()
        })();
        command_elapsed_us = Some(started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64);
        stage_timing = Some(timing);
        command_result
    })();
    // Even pending/partial/late commands settle only through the common owner.
    // No post-input provider, inverse attempt or new budget hides that result.
    producer.finish(&mut owner)?;
    let input_sent = attempt.as_ref().is_some_and(|v| v.accepted_count > 0);
    let receipt = serde_json::json!({
        "schema":1,"operation":"owned-essential-production-route-calibration",
        "target":r.target,"image":r.image,"parent_job_present":true,
        "owned_blank_fixture_asserted":true,"source_proof":owner.source().source.proof(),
        "before":before,"observation_digest":observation_digest,"attempt":attempt,
        "error":result.err(),"input_sent":input_sent,"input_retirement":"Complete",
        "common_finite_owner":true,"shared_production_preparation":true,
        "shared_production_stage_engine":true,"provider_budget_ms":r.editor.provider_budget_ms(),
        "command_elapsed_us":command_elapsed_us,"stage_timing":stage_timing,
        "setup_excluded_from_parent_exchange":true,
        "resident_setup":"process/Job/COM/source-open/initial-read/finite-owner",
        "measured_shared_route_resident_exchange":execute,"production_parent_exchange_proven":false,
        "catalog_installed":false,"profile_authority":false,"semantic_role_proven":false,
        "restoration_proven":false,"owned_fixture_control_cleanup_outstanding":input_sent,
        "physical_or_editor_pixel_effect_proven":false
    });
    let bytes = serde_json::to_vec(&receipt).map_err(|_| Error::Invalid)?;
    if bytes.len() > 1024 * 1024 {
        return Err(Error::Limit);
    }
    let mut out = std::io::stdout().lock();
    out.write_all(&bytes)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Result<Request> {
        let rect = vw_remote::Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };
        Ok(Request {
            schema: 1,
            owner_pid: 11,
            target: Target {
                token: "018f5a10-7b00-7000-8000-000000000001".into(),
                window: 3,
                process_id: 4,
                thread_id: 5,
                process_created: 6,
                window_rect: rect,
                frame_rect: rect,
                client_rect: rect,
                dpi: 96,
                integrity: 8192,
            },
            image: profile::ImageIdentity {
                executable_name: "krita.exe".into(),
                executable_blake3:
                    "8e7e6999745d3f84cf402796717f86c05e39ba2398af0e3758888be333d50720".into(),
                executable_bytes: 273624,
                file_version: Some("5.3.4.0".into()),
                package_full_name: None,
                package_version: None,
            },
            editor: Editor::Krita,
            source_policy: SourcePolicy::Ordinary,
            candidate: ActionSpec {
                action: 3,
                route: RouteSpec::Toolbar {
                    route: live::route_for(3)?,
                },
            },
            mutable_size: None,
            owned_blank_fixture: true,
            consent_to_finite_input: false,
            expected_observation_digest: None,
        })
    }
    fn proof(r: &Request, digest: &str) -> Proof {
        Proof::Krita(Box::new(KritaProof {
            settings: ToolSettings {
                freehand_selected: true,
                selected_tool_id: live::BRUSH_ID.into(),
                eraser_mode: false,
                preset: "basic".into(),
                blending_mode: "normal".into(),
                preserve_alpha: false,
            },
            canvas: CanvasProof {
                runtime_id_hash: "a".repeat(64),
                process_id: r.target.process_id,
                sampled_qpc_100ns: 10,
                class_name: "KisOpenGLCanvas2".into(),
                control_type: 50026,
                canvas_rect: r.target.client_rect,
                profile_digest: digest.into(),
            },
            element: ElementProof {
                runtime_id_hash: "b".repeat(64),
                physical_rect: r.target.client_rect,
                enabled: true,
                offscreen: false,
            },
            numeric: Some(NumericProof {
                element: ElementProof {
                    runtime_id_hash: "c".repeat(64),
                    physical_rect: r.target.client_rect,
                    enabled: true,
                    offscreen: false,
                },
                range: RangeShape {
                    minimum: 1.0,
                    maximum: 100.0,
                    small_change: 1.0,
                    large_change: 10.0,
                },
                current: 40.0,
                value_text: "40".into(),
                read_only: false,
            }),
            watched: vec![],
            watched_digest: "d".repeat(64),
        }))
    }
    #[test]
    fn observation_normalizes_only_clock_and_binds_value_settings_target_image() -> Result<()> {
        let r = request()?;
        let binding = context_digest(&r)?;
        let original = proof(&r, &binding);
        let hash = |r: &Request, p: &Proof| {
            Observation::capture(&context_digest(r)?, None, &r.target, &r.candidate, p)?.digest()
        };
        let expected = hash(&r, &original)?;
        let mut changed = original.clone();
        if let Proof::Krita(p) = &mut changed {
            p.canvas.sampled_qpc_100ns += 100;
        }
        assert_eq!(hash(&r, &changed)?, expected);
        if let Proof::Krita(p) = &mut changed {
            p.numeric.as_mut().ok_or(Error::Invalid)?.current += 1.0;
        }
        assert_ne!(hash(&r, &changed)?, expected);
        let mut changed = original.clone();
        if let Proof::Krita(p) = &mut changed {
            p.settings.preset = "changed".into();
        }
        assert_ne!(hash(&r, &changed)?, expected);
        let mut moved = r.clone();
        moved.target.process_created += 1;
        assert_ne!(hash(&moved, &original)?, expected);
        moved = r.clone();
        moved.image.executable_blake3 = "f".repeat(64);
        assert_ne!(hash(&moved, &original)?, expected);
        Ok(())
    }
    #[test]
    fn modes_consent_owned_fixture_and_expected_hash_are_closed_before_input() -> Result<()> {
        assert!(!execute_mode("--essential-calibration-observe")?);
        assert!(execute_mode("--essential-calibration-experiment")?);
        assert_eq!(execute_mode("--click"), Err(Error::Invalid));
        let mut r = request()?;
        request_valid(&r, false)?;
        assert_eq!(request_valid(&r, true), Err(Error::Ungranted));
        r.consent_to_finite_input = true;
        assert_eq!(request_valid(&r, true), Err(Error::Ungranted));
        r.expected_observation_digest = Some("a".repeat(64));
        request_valid(&r, true)?;
        assert_eq!(request_valid(&r, false), Err(Error::Ungranted));
        r.owned_blank_fixture = false;
        assert_eq!(request_valid(&r, true), Err(Error::Ungranted));
        Ok(())
    }
}
