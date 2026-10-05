//! Common finite-owner Krita blank-fixture effects. No packaged authority.
mod wheel;
use crate::{
    Error, Result,
    editor_source::{SourceBudget, SourceLease, SourcePolicy},
    platform,
};
use platform::{
    finite_input::{FiniteInputOwner, FiniteRetirement},
    krita_controls::{self, Action, BatchReceipt, Snapshot},
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
fn exact_image(image: &ImageIdentity) -> Result<()> {
    if image.executable_name == "krita.exe"
        && image.executable_blake3
            == "8e7e6999745d3f84cf402796717f86c05e39ba2398af0e3758888be333d50720"
        && image.executable_bytes == 273624
        && image.file_version.as_deref() == Some("5.3.4.0")
        && image.package_full_name.is_none()
        && image.package_version.is_none()
    {
        Ok(())
    } else {
        Err(Error::Unavailable)
    }
}
fn request_valid(invocation: &Invocation) -> Result<bool> {
    let r = &invocation.request;
    if r.schema != 1
        || r.owner_pid == 0
        || !r.owned_blank_fixture
        || !matches!(
            invocation.mode.as_str(),
            "--krita-observe" | "--krita-experiment"
        )
    {
        return Err(Error::Invalid);
    }
    let execute = invocation.mode == "--krita-experiment";
    if r.consent_to_finite_input != execute
        || (execute
            && (r
                .expected_settings_digest
                .as_deref()
                .is_none_or(|s| !vw_remote::profile::digest(s))
                || r.expected_destination_runtime_hash
                    .as_deref()
                    .is_none_or(|s| !vw_remote::profile::digest(s))))
        || (!execute
            && (r.expected_settings_digest.is_some()
                || r.expected_destination_runtime_hash.is_some()
                || matches!(r.history_policy, HistoryPolicy::LeaveForOwnedFrameFixture)))
    {
        return Err(Error::Ungranted);
    }
    exact_image(&r.image)?;
    r.target.validate()?;
    Ok(execute)
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
        self.cancel.check().map_err(|_| Error::Cancelled)?;
        let now = Instant::now();
        let remaining = self
            .deadline
            .checked_duration_since(now)
            .filter(|d| !d.is_zero())
            .ok_or(Error::Timeout)?;
        SourceBudget::until(
            self.cancel.clone(),
            now.checked_add(remaining.min(Duration::from_secs(5)))
                .ok_or(Error::Limit)?,
        )
    }
}
fn observe(source: &SourceLease, r: &Request, clock: &Clock, action: Action) -> Result<Snapshot> {
    let budget = clock.phase()?;
    source.verify(&r.target, r.owner_pid, &budget)?;
    let result = krita_controls::observe(&r.target, &budget, action)?;
    source.verify(&r.target, r.owner_pid, &budget)?;
    Ok(result)
}
// Post-state does not require the action's button to remain actionable. Undo
// and Redo may legitimately disable themselves after their last history entry.
// Pre-send observe(action) and the common owner still require enabled admission.
fn observe_post<T>(query: impl FnOnce(Action) -> Result<T>) -> Result<T> {
    query(Action::Brush)
}
fn restoration_after<T>(
    error: Option<&Error>,
    restore: impl FnOnce() -> T,
    failed: impl FnOnce() -> T,
) -> T {
    if error.is_none() { restore() } else { failed() }
}
fn batch(
    owner: &mut InputOwner,
    producer: &mut super::terminal::Producer,
    r: &Request,
    clock: &Clock,
    action: Action,
    expected: &Snapshot,
    receipts: &mut Vec<BatchReceipt>,
) -> Result<()> {
    let budget = clock.phase()?;
    producer.arm_attempt()?;
    let result = krita_controls::inject(owner, &r.target, r.owner_pid, &budget, action, expected)?;
    let completed = result.completed();
    // Record every actual accepted-prefix count before fallible post-query work.
    receipts.push(result);
    completed
}
#[derive(Serialize)]
struct Restoration {
    status: &'static str,
    batches: Vec<BatchReceipt>,
    final_observed_state: Option<Snapshot>,
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
            error: Some(error),
            status: "failed",
            ..Self::inactive()
        }
    }
}
fn restore(
    owner: &mut InputOwner,
    producer: &mut super::terminal::Producer,
    r: &Request,
    clock: &Clock,
    baseline: &Snapshot,
) -> Restoration {
    if matches!(r.history_policy, HistoryPolicy::LeaveForOwnedFrameFixture) {
        return Restoration {
            status: "deferred_to_owned_frame_fixture",
            ..Restoration::inactive()
        };
    }
    let mut batches = vec![];
    let mut last_state = None;
    let result = (|| -> Result<Snapshot> {
        if let Some(inverse) = r.action.inverse_history() {
            let before = observe(owner.source(), r, clock, inverse)?;
            batch(owner, producer, r, clock, inverse, &before, &mut batches)?;
        }
        let mut current = observe(owner.source(), r, clock, Action::Brush)?;
        last_state = Some(current.clone());
        if current.settings.selected_tool_id != baseline.settings.selected_tool_id {
            let original = Action::baseline_tool(&baseline.settings)?;
            current = observe(owner.source(), r, clock, original)?;
            batch(owner, producer, r, clock, original, &current, &mut batches)?;
            current = observe(owner.source(), r, clock, Action::Brush)?;
            last_state = Some(current.clone());
        }
        if current.settings.eraser_mode != baseline.settings.eraser_mode {
            current = observe(owner.source(), r, clock, Action::EraserToggle)?;
            batch(
                owner,
                producer,
                r,
                clock,
                Action::EraserToggle,
                &current,
                &mut batches,
            )?;
            current = observe(owner.source(), r, clock, Action::Brush)?;
            last_state = Some(current.clone());
        }
        // Both anonymously labelled ranges remain watched by current runtime
        // identity/value. No range ordinal becomes size/opacity or restore input.
        if !baseline.same_state(&current) {
            return Err(Error::TargetChanged);
        }
        let budget = clock.phase()?;
        owner.source().verify(&r.target, r.owner_pid, &budget)?;
        Ok(current)
    })();
    match result {
        Ok(state) => Restoration {
            status: "observed_controls_restored",
            batches,
            final_observed_state: Some(state),
            error: None,
            editor_pixels_or_history_restored_proven: false,
        },
        Err(error) => Restoration {
            status: "failed",
            batches,
            final_observed_state: last_state,
            error: Some(error),
            editor_pixels_or_history_restored_proven: false,
        },
    }
}
#[derive(Serialize)]
struct Receipt {
    schema: u32,
    operation: &'static str,
    target: Target,
    image: ImageIdentity,
    parent_job_present: bool,
    held_process_and_image_namespace: bool,
    source_proof: crate::editor_source::SourceProof,
    owned_blank_fixture_asserted: bool,
    before: Snapshot,
    batches: Vec<BatchReceipt>,
    after: Option<Snapshot>,
    error: Option<Error>,
    restoration: Restoration,
    common_finite_owner: bool,
    input_retirement: FiniteRetirement,
    input_sent: bool,
    profile_authority: bool,
    physical_or_editor_pixel_effect_proven: bool,
    owned_fixture_control_or_history_cleanup_outstanding: bool,
}
pub(super) fn run(bytes: &[u8], producer: &mut super::terminal::Producer) -> Result<()> {
    // Caller owns the actual parent Job, one-process confinement, MTA and PMv2.
    let mode: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
    if matches!(
        mode.get("mode").and_then(serde_json::Value::as_str),
        Some("--krita-wheel-observe" | "--krita-wheel-experiment")
    ) {
        return wheel::run(bytes, producer);
    }
    let invocation: Invocation = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
    let execute = request_valid(&invocation)?;
    let r = invocation.request;
    let clock = Clock::new()?;
    let source = SourceLease::open(
        &r.target,
        r.owner_pid,
        &r.image,
        SourcePolicy::Ordinary,
        &clock.phase()?,
    )?;
    let before = observe(&source, &r, &clock, r.action)?;
    let mut input_owner = FiniteInputOwner::new(r.target.clone(), r.owner_pid, source)?;
    let mut batches = vec![];
    let mut after = None;
    let mut restoration = Restoration::inactive();
    let mut error = None;
    if execute {
        let result = (|| -> Result<()> {
            if r.expected_settings_digest.as_deref() != Some(before.settings_digest.as_str())
                || r.expected_destination_runtime_hash.as_deref()
                    != Some(before.element.runtime_id_hash.as_str())
            {
                return Err(Error::TargetChanged);
            }
            batch(
                &mut input_owner,
                producer,
                &r,
                &clock,
                r.action,
                &before,
                &mut batches,
            )?;
            let current = observe_post(|state_action| {
                observe(input_owner.source(), &r, &clock, state_action)
            })?;
            let compatible = before.compatible_change(&current, r.action);
            after = Some(current);
            compatible
        })();
        error = result.err();
        restoration = restoration_after(
            error.as_ref(),
            || restore(&mut input_owner, producer, &r, &clock, &before),
            || {
                // Failed/unknown/partial input does not start another control batch.
                // The common owner retains/retire-releases its accepted downs.
                if batches.is_empty() {
                    Restoration::inactive()
                } else {
                    Restoration::failed(error.clone().unwrap_or(Error::Unavailable))
                }
            },
        );
    }
    let input_sent = batches
        .iter()
        .chain(restoration.batches.iter())
        .any(|b| b.native_accepted_count > 0);
    let cleanup_outstanding = input_sent
        && (r.action.inverse_history().is_some()
            || matches!(r.history_policy, HistoryPolicy::LeaveForOwnedFrameFixture)
            || restoration.error.is_some());
    // Optional Pending is flushed before actual release-only wait. The same
    // moved ordinary SourceLease lives through zero-held Complete; all failures
    // still reach this owner path. Parent EOF/process death never proves release.
    producer.finish(&mut input_owner)?;
    let receipt = Receipt {
        schema: 1,
        operation: "owned-blank-editor-effect-hil",
        target: r.target,
        image: r.image,
        parent_job_present: true,
        held_process_and_image_namespace: true,
        source_proof: input_owner.source().proof(),
        owned_blank_fixture_asserted: true,
        before,
        batches,
        after,
        error,
        restoration,
        common_finite_owner: true,
        input_retirement: FiniteRetirement::Complete,
        input_sent,
        profile_authority: false,
        physical_or_editor_pixel_effect_proven: false,
        owned_fixture_control_or_history_cleanup_outstanding: cleanup_outstanding,
    };
    let output = serde_json::to_vec(&receipt).map_err(|_| Error::Invalid)?;
    if output.len() > 1024 * 1024 {
        return Err(Error::Limit);
    }
    let mut out = std::io::stdout().lock();
    out.write_all(&output)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request(mode: &str) -> Invocation {
        let rect = vw_remote::Rect {
            x: 10,
            y: 20,
            width: 800,
            height: 600,
        };
        Invocation {
            mode: mode.into(),
            request: Request {
                schema: 1,
                owner_pid: 7,
                target: Target {
                    token: "01900000-0000-7000-8000-000000000001".into(),
                    window: 8,
                    process_id: 9,
                    thread_id: 10,
                    process_created: 11,
                    window_rect: rect,
                    frame_rect: rect,
                    client_rect: rect,
                    dpi: 96,
                    integrity: 0,
                },
                image: ImageIdentity {
                    executable_name: "krita.exe".into(),
                    executable_blake3:
                        "8e7e6999745d3f84cf402796717f86c05e39ba2398af0e3758888be333d50720".into(),
                    executable_bytes: 273624,
                    file_version: Some("5.3.4.0".into()),
                    package_full_name: None,
                    package_version: None,
                },
                action: Action::Brush,
                owned_blank_fixture: true,
                consent_to_finite_input: false,
                expected_settings_digest: None,
                expected_destination_runtime_hash: None,
                history_policy: HistoryPolicy::Restore,
            },
        }
    }
    #[test]
    fn observation_and_input_consent_cannot_cross_modes_or_import_stale_expectations() -> Result<()>
    {
        let mut value = request("--krita-observe");
        assert!(!request_valid(&value)?);
        value.request.consent_to_finite_input = true;
        assert_eq!(request_valid(&value), Err(Error::Ungranted));
        value.request.consent_to_finite_input = false;
        value.request.expected_settings_digest = Some("a".repeat(64));
        assert_eq!(request_valid(&value), Err(Error::Ungranted));
        value.mode = "--krita-experiment".into();
        assert_eq!(request_valid(&value), Err(Error::Ungranted));
        value.request.consent_to_finite_input = true;
        value.request.expected_destination_runtime_hash = Some("b".repeat(64));
        assert!(request_valid(&value)?);
        value.mode = "--paint-experiment".into();
        assert_eq!(request_valid(&value), Err(Error::Invalid));
        Ok(())
    }
    #[test]
    fn measured_image_policy_does_not_admit_name_or_version_alone() -> Result<()> {
        let mut value = ImageIdentity {
            executable_name: "krita.exe".into(),
            executable_blake3: "8e7e6999745d3f84cf402796717f86c05e39ba2398af0e3758888be333d50720"
                .into(),
            executable_bytes: 273624,
            file_version: Some("5.3.4.0".into()),
            package_full_name: None,
            package_version: None,
        };
        exact_image(&value)?;
        value.executable_blake3 = "a".repeat(64);
        assert_eq!(exact_image(&value), Err(Error::Unavailable));
        value.executable_blake3 =
            "8e7e6999745d3f84cf402796717f86c05e39ba2398af0e3758888be333d50720".into();
        value.package_full_name = Some("foreign".into());
        assert_eq!(exact_image(&value), Err(Error::Unavailable));
        Ok(())
    }
    #[test]
    fn expired_experiment_cannot_start_a_new_restoration_phase() -> Result<()> {
        let clock = Clock {
            deadline: Instant::now()
                .checked_sub(Duration::from_secs(1))
                .ok_or(Error::Limit)?,
            cancel: Arc::new(vw_capture::Cancellation::default()),
        };
        assert!(matches!(clock.phase(), Err(Error::Timeout)));
        Ok(())
    }
    #[test]
    fn final_undo_or_redo_disabled_still_records_state_and_reaches_requested_inverse() -> Result<()>
    {
        use std::cell::Cell;
        for requested in [Action::Undo, Action::Redo] {
            let disabled_address_queried = Cell::new(false);
            let state = observe_post(|route| {
                if route == requested {
                    disabled_address_queried.set(true);
                    Err(Error::Unavailable)
                } else {
                    Ok("last history action disabled; watched state still known")
                }
            })?;
            assert!(!disabled_address_queried.get());
            let inverse = restoration_after(None, || requested.inverse_history(), || None);
            assert_eq!(
                inverse,
                Some(if requested == Action::Undo {
                    Action::Redo
                } else {
                    Action::Undo
                })
            );
            assert_eq!(
                state,
                "last history action disabled; watched state still known"
            );
        }
        Ok(())
    }
    #[test]
    fn unknown_post_state_or_failed_injection_never_reaches_inverse_callback() {
        use std::cell::Cell;
        let inverse_called = Cell::new(false);
        let unknown: Result<()> = observe_post(|_| Err(Error::TargetChanged));
        let error = unknown.err();
        let restored = restoration_after(
            error.as_ref(),
            || {
                inverse_called.set(true);
                true
            },
            || false,
        );
        assert!(!restored && !inverse_called.get());
        let partial = Error::PartialInput;
        restoration_after(Some(&partial), || inverse_called.set(true), || ());
        assert!(!inverse_called.get());
    }
}
