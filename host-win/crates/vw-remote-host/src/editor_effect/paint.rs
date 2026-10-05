//! Exact installed Paint blank-fixture experiment. No production profile.
use crate::{
    Error, Result,
    editor_source::{SourceBudget, SourceLease, SourcePolicy},
    platform,
};
use platform::finite_input::{FiniteInputOwner, FiniteRetirement};
use platform::paint_controls::{
    Snapshot,
    experiment::{self, Action, Address, BatchReceipt, State},
};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    sync::Arc,
    time::{Duration, Instant},
};
use vw_remote::{Target, profile::ImageIdentity};

type InputOwner = FiniteInputOwner<SourceLease>;

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum HistoryPolicy {
    #[default]
    Restore,
    LeaveForOwnedFrameFixture,
}
impl HistoryPolicy {
    fn validate(self, action: Action, execute: bool) -> Result<()> {
        if matches!(self, Self::LeaveForOwnedFrameFixture)
            && (!execute || !matches!(action, Action::CanvasDot | Action::Undo | Action::Redo))
        {
            return Err(Error::Ungranted);
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: u32,
    owner_pid: u32,
    target: Target,
    image: ImageIdentity,
    action: Action,
    owned_blank_fixture: bool,
    consent_to_finite_input: bool,
    expected_settings_digest: Option<String>,
    expected_destination_runtime_hash: Option<String>,
    #[serde(default)]
    history_policy: HistoryPolicy,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invocation {
    mode: String,
    request: Request,
}
struct Clock {
    deadline: Instant,
    cancel: Arc<vw_capture::Cancellation>,
}
impl Clock {
    fn new() -> Result<Self> {
        Ok(Self {
            deadline: Instant::now()
                .checked_add(Duration::from_secs(12))
                .ok_or(Error::Limit)?,
            cancel: Arc::new(vw_capture::Cancellation::default()),
        })
    }
    fn phase(&self) -> Result<SourceBudget> {
        // The absolute 12-second experiment deadline never moves. Individual
        // completed phases also obey the source adapter's <=5-second cap.
        // A slow/late provider cannot create a new phase after that deadline.
        self.cancel.check().map_err(|_| Error::Cancelled)?;
        let now = Instant::now();
        let remaining = self
            .deadline
            .checked_duration_since(now)
            .filter(|v| !v.is_zero())
            .ok_or(Error::Timeout)?;
        SourceBudget::until(
            self.cancel.clone(),
            now.checked_add(remaining.min(Duration::from_secs(5)))
                .ok_or(Error::Limit)?,
        )
    }
}
#[derive(Serialize)]
struct Restoration {
    status: &'static str,
    batches: Vec<BatchReceipt>,
    final_observed_state: Option<State>,
    error: Option<Error>,
    editor_pixels_or_history_restored_proven: bool,
}
impl Restoration {
    fn inactive() -> Self {
        Self {
            status: "not_needed",
            batches: vec![],
            final_observed_state: None,
            error: None,
            editor_pixels_or_history_restored_proven: false,
        }
    }
    fn failed(error: Error) -> Self {
        Self {
            status: "failed",
            batches: vec![],
            final_observed_state: None,
            error: Some(error),
            editor_pixels_or_history_restored_proven: false,
        }
    }
}
#[derive(Serialize)]
struct Receipt {
    schema: u32,
    operation: &'static str,
    target: Target,
    image: ImageIdentity,
    parent_job_present: bool,
    source_namespace_policy: &'static str,
    held_process_and_image_namespace: bool,
    held_process_and_final_image: bool,
    held_ancestor_namespace: bool,
    namespace_lease_equivalence_claim: bool,
    os_package_namespace_validated: bool,
    package_evidence: vw_host::editor_paint_package::PaintPackageEvidence,
    source_proof: crate::editor_source::SourceProof,
    owned_blank_fixture_asserted: bool,
    before: Snapshot,
    address: Address,
    batches: Vec<BatchReceipt>,
    observed_states_after_batches: Vec<State>,
    after: Option<Snapshot>,
    error: Option<Error>,
    restoration: Restoration,
    input_sent: bool,
    profile_authority: bool,
    physical_or_editor_pixel_effect_proven: bool,
    input_retirement: FiniteRetirement,
    owned_fixture_history_cleanup_outstanding: bool,
}
fn observe(source: &SourceLease, r: &Request, clock: &Clock) -> Result<(Snapshot, State)> {
    let budget = clock.phase()?;
    source.verify(&r.target, r.owner_pid, &budget)?;
    let snapshot = platform::paint_controls::observe(&r.target, &budget)?;
    let state = State::from_snapshot(&snapshot)?;
    source.verify(&r.target, r.owner_pid, &budget)?;
    Ok((snapshot, state))
}
fn address(source: &SourceLease, r: &Request, clock: &Clock, action: Action) -> Result<Address> {
    let budget = clock.phase()?;
    source.verify(&r.target, r.owner_pid, &budget)?;
    let result = experiment::address(&r.target, &budget, action)?;
    source.verify(&r.target, r.owner_pid, &budget)?;
    Ok(result)
}
fn batch(
    input_owner: &mut InputOwner,
    producer: &mut super::terminal::Producer,
    r: &Request,
    clock: &Clock,
    step: experiment::Step<'_>,
    receipts: &mut Vec<BatchReceipt>,
) -> Result<()> {
    let budget = clock.phase()?;
    producer.arm_attempt()?;
    let receipt = experiment::inject(input_owner, &r.target, r.owner_pid, &budget, step)?;
    let result = receipt.completed();
    // Preserve this actual SendInput count before any fallible post-observation.
    receipts.push(receipt);
    result
}
fn opposite_history(action: Action) -> Option<Action> {
    match action {
        Action::Undo => Some(Action::Redo),
        Action::Redo => Some(Action::Undo),
        Action::CanvasDot => Some(Action::Undo),
        _ => None,
    }
}
fn restore(
    input_owner: &mut InputOwner,
    producer: &mut super::terminal::Producer,
    r: &Request,
    clock: &Clock,
    baseline: &State,
    current: &State,
) -> Restoration {
    if matches!(r.history_policy, HistoryPolicy::LeaveForOwnedFrameFixture) {
        return Restoration {
            status: "deferred_to_owned_frame_fixture",
            batches: vec![],
            final_observed_state: Some(current.clone()),
            error: None,
            editor_pixels_or_history_restored_proven: false,
        };
    }
    let mut receipts = vec![];
    let mut state = current.clone();
    let result = (|| -> Result<State> {
        if let Some(inverse) = opposite_history(r.action) {
            // Paired guarded history input is recorded, never claimed to restore
            // pixels/history without independent owned-frame effect evidence.
            let route = address(input_owner.source(), r, clock, inverse)?;
            batch(
                input_owner,
                producer,
                r,
                clock,
                experiment::Step {
                    action: inverse,
                    expected: &state,
                    expected_address: &route,
                    arrow: None,
                },
                &mut receipts,
            )?;
            state = observe(input_owner.source(), r, clock)?.1;
        } else if matches!(r.action, Action::Brush | Action::Eraser) && !baseline.same(&state) {
            let original = baseline.baseline_tool()?;
            let route = address(input_owner.source(), r, clock, original)?;
            batch(
                input_owner,
                producer,
                r,
                clock,
                experiment::Step {
                    action: original,
                    expected: &state,
                    expected_address: &route,
                    arrow: None,
                },
                &mut receipts,
            )?;
            state = observe(input_owner.source(), r, clock)?.1;
        } else if r.action.slider() {
            for _ in 0..32 {
                let wanted = baseline.numeric_current(r.action)?;
                let previous = state.numeric_current(r.action)?;
                if previous == wanted {
                    break;
                }
                let route = address(input_owner.source(), r, clock, r.action)?;
                batch(
                    input_owner,
                    producer,
                    r,
                    clock,
                    experiment::Step {
                        action: r.action,
                        expected: &state,
                        expected_address: &route,
                        arrow: Some(wanted > previous),
                    },
                    &mut receipts,
                )?;
                let next = observe(input_owner.source(), r, clock)?.1;
                baseline.compatible_change(&next, r.action)?;
                experiment::progress_toward(previous, next.numeric_current(r.action)?, wanted)?;
                state = next;
            }
        } else if matches!(
            r.action,
            Action::ZoomIn | Action::ZoomOut | Action::FitToWindow
        ) {
            for _ in 0..32 {
                let wanted = baseline.numeric_current(r.action)?;
                let previous = state.numeric_current(r.action)?;
                if previous == wanted {
                    break;
                }
                let inverse = if wanted > previous {
                    Action::ZoomIn
                } else {
                    Action::ZoomOut
                };
                let route = address(input_owner.source(), r, clock, inverse)?;
                batch(
                    input_owner,
                    producer,
                    r,
                    clock,
                    experiment::Step {
                        action: inverse,
                        expected: &state,
                        expected_address: &route,
                        arrow: None,
                    },
                    &mut receipts,
                )?;
                let next = observe(input_owner.source(), r, clock)?.1;
                baseline.compatible_change(&next, r.action)?;
                experiment::progress_toward(previous, next.numeric_current(r.action)?, wanted)?;
                state = next;
            }
        }
        if !baseline.same(&state) {
            return Err(Error::TargetChanged);
        }
        let budget = clock.phase()?;
        input_owner
            .source()
            .verify(&r.target, r.owner_pid, &budget)?;
        Ok(state.clone())
    })();
    match result {
        Ok(state) => Restoration {
            status: "observed_controls_restored",
            batches: receipts,
            final_observed_state: Some(state),
            error: None,
            editor_pixels_or_history_restored_proven: false,
        },
        Err(error) => Restoration {
            status: "failed",
            batches: receipts,
            final_observed_state: Some(state),
            error: Some(error),
            editor_pixels_or_history_restored_proven: false,
        },
    }
}
pub(super) fn run(bytes: &[u8], producer: &mut super::terminal::Producer) -> Result<()> {
    // Caller has already proved actual parent Job, confined one process, MTA
    // and PMv2. The exact effect image is held by its suspended-launch runner.
    let invocation: Invocation = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
    let r = invocation.request;
    if r.schema != 1
        || r.owner_pid == 0
        || !r.owned_blank_fixture
        || !matches!(
            invocation.mode.as_str(),
            "--paint-observe" | "--paint-experiment"
        )
    {
        return Err(Error::Invalid);
    }
    let execute = invocation.mode == "--paint-experiment";
    r.history_policy.validate(r.action, execute)?;
    if execute
        && (!r.consent_to_finite_input
            || r.expected_settings_digest
                .as_deref()
                .is_none_or(|s| !vw_remote::profile::digest(s))
            || r.expected_destination_runtime_hash
                .as_deref()
                .is_none_or(|s| !vw_remote::profile::digest(s)))
    {
        return Err(Error::Ungranted);
    }
    if !execute && r.consent_to_finite_input {
        return Err(Error::Ungranted);
    }
    r.target.validate()?;
    let clock = Clock::new()?;
    let budget = clock.phase()?;
    let source = SourceLease::open(
        &r.target,
        r.owner_pid,
        &r.image,
        SourcePolicy::InstalledPaint,
        &budget,
    )?;
    let package_evidence = source
        .installed_package_evidence()
        .ok_or(Error::Unavailable)?
        .clone();
    // This owner receives the actual SourceLease, never a borrowed receipt.
    // Its accepted-down ledger and process/image/package leases share lifetime.
    let mut input_owner = InputOwner::new(r.target.clone(), r.owner_pid, source)?;
    let (before, baseline) = observe(input_owner.source(), &r, &clock)?;
    // Before any input, require a legitimate currently recognized baseline tool.
    // This also excludes conflicting toggles and any guessed eraser canvas name.
    baseline.baseline_tool()?;
    let route = address(input_owner.source(), &r, &clock, r.action)?;
    let mut batches = vec![];
    let mut observed_states_after_batches = vec![];
    let mut after = None;
    let mut current = None;
    let mut error = None;
    let mut restoration = Restoration::inactive();
    if execute {
        // Caller's source-bound acquisition is an equality fence only, never
        // action authority. Re-observation and final native guard remain required.
        let result = (|| -> Result<()> {
            if experiment::settings_digest(&before) != r.expected_settings_digest.as_deref()
                || r.expected_destination_runtime_hash.as_deref() != Some(route.runtime_hash())
            {
                return Err(Error::TargetChanged);
            }
            if r.action.slider() {
                baseline.slider_preflight(r.action)?;
            }
            batch(
                &mut input_owner,
                producer,
                &r,
                &clock,
                experiment::Step {
                    action: r.action,
                    expected: &baseline,
                    expected_address: &route,
                    arrow: None,
                },
                &mut batches,
            )?;
            let (snapshot, state) = observe(input_owner.source(), &r, &clock)?;
            observed_states_after_batches.push(state.clone());
            after = Some(snapshot);
            current = Some(state.clone());
            baseline.compatible_change(&state, r.action)?;
            if r.action.slider() {
                // Clicking the actual Thumb must preserve every baseline
                // control fact. Do not infer safety from its advertised span.
                if !baseline.same(&state) {
                    return Err(Error::TargetChanged);
                }
                // A click is not keyboard-focus proof. inject performs fresh
                // GetFocusedElement/PID-first/CompareElements on this exact slider.
                batch(
                    &mut input_owner,
                    producer,
                    &r,
                    &clock,
                    experiment::Step {
                        action: r.action,
                        expected: &state,
                        expected_address: &route,
                        arrow: Some(r.action.right()),
                    },
                    &mut batches,
                )?;
                let (snapshot, state) = observe(input_owner.source(), &r, &clock)?;
                observed_states_after_batches.push(state.clone());
                after = Some(snapshot);
                current = Some(state.clone());
                baseline.compatible_change(&state, r.action)?;
            }
            Ok(())
        })();
        error = result.err();
        if error.is_none() {
            restoration = match current.as_ref() {
                Some(state) => restore(&mut input_owner, producer, &r, &clock, &baseline, state),
                None => Restoration::failed(Error::Unavailable),
            };
        } else if !batches.is_empty() {
            // Partial input, unknown state or changed evidence seals this attempt.
            // No new tool/control batch follows. The actual shared owner still
            // retires its own accepted downs through dedicated native UP guards.
            restoration = Restoration::failed(error.clone().unwrap_or(Error::Unavailable));
        }
    }
    let input_sent = batches
        .iter()
        .chain(restoration.batches.iter())
        .any(|b| b.native_accepted_count > 0);
    producer.finish(&mut input_owner)?;
    let input_retirement = FiniteRetirement::Complete;
    let source_proof = input_owner.source().proof();
    let owned_fixture_history_cleanup_outstanding =
        input_sent && matches!(r.history_policy, HistoryPolicy::LeaveForOwnedFrameFixture);
    let receipt = Receipt {
        schema: 1,
        operation: "owned-blank-paint-controls-hil",
        target: r.target,
        image: r.image,
        parent_job_present: true,
        source_namespace_policy: "installed-paint-v1",
        held_process_and_image_namespace: false,
        held_process_and_final_image: true,
        held_ancestor_namespace: false,
        namespace_lease_equivalence_claim: false,
        os_package_namespace_validated: true,
        package_evidence,
        source_proof,
        owned_blank_fixture_asserted: true,
        before,
        address: route,
        batches,
        observed_states_after_batches,
        after,
        error,
        restoration,
        input_sent,
        profile_authority: false,
        physical_or_editor_pixel_effect_proven: false,
        input_retirement,
        owned_fixture_history_cleanup_outstanding,
    };
    let output = serde_json::to_vec(&receipt).map_err(|_| Error::Invalid)?;
    if output.len() > 1024 * 1024 {
        return Err(Error::Limit);
    }
    let mut out = std::io::stdout().lock();
    out.write_all(&output)?;
    out.write_all(b"\n")?;
    out.flush()?;
    // Returning Ok means receipt output, not successful input/effect/restoration.
    // Root must inspect receipt.error AND restoration.error plus actual retirement.
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_inverse_is_explicit_and_never_invents_a_tool_inverse() {
        assert_eq!(opposite_history(Action::Undo), Some(Action::Redo));
        assert_eq!(opposite_history(Action::Redo), Some(Action::Undo));
        assert_eq!(opposite_history(Action::Brush), None);
        assert_eq!(opposite_history(Action::Eraser), None);
        assert_eq!(opposite_history(Action::CanvasDot), Some(Action::Undo));
    }
    #[test]
    fn deferred_pixel_history_is_only_for_owned_draw_undo_redo_measurement() -> Result<()> {
        for action in [Action::CanvasDot, Action::Undo, Action::Redo] {
            HistoryPolicy::LeaveForOwnedFrameFixture.validate(action, true)?;
        }
        for action in [Action::Brush, Action::SizeUp, Action::ZoomIn] {
            assert_eq!(
                HistoryPolicy::LeaveForOwnedFrameFixture.validate(action, true),
                Err(Error::Ungranted)
            );
        }
        assert_eq!(
            HistoryPolicy::LeaveForOwnedFrameFixture.validate(Action::CanvasDot, false),
            Err(Error::Ungranted)
        );
        Ok(())
    }
    #[test]
    fn finite_actions_cannot_import_keys_coordinates_or_unsupported_opacity() -> Result<()> {
        let action: Action = serde_json::from_str("\"size_up\"").map_err(|_| Error::Invalid)?;
        assert!(action.slider());
        for value in [
            "\"opacity_up\"",
            "\"ctrl_z\"",
            "{\"key\":39}",
            "{\"x\":100,\"y\":100}",
        ] {
            assert!(serde_json::from_str::<Action>(value).is_err());
        }
        Ok(())
    }
    #[test]
    fn an_overdue_experiment_cannot_renew_a_restoration_phase() -> Result<()> {
        let deadline = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .ok_or(Error::Limit)?;
        let clock = Clock {
            deadline,
            cancel: Arc::new(vw_capture::Cancellation::default()),
        };
        assert!(matches!(clock.phase(), Err(Error::Timeout)));
        assert_eq!(clock.deadline, deadline);
        Ok(())
    }
}
