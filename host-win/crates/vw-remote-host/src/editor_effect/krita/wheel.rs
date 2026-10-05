//! Root-owned one-detent numeric calibration. No catalog/shortcut authority.
use super::*;
use platform::krita_controls::wheel::{self, Batch, State};
use vw_remote::profile::essential::RangeShape;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: u32,
    owner_pid: u32,
    target: Target,
    image: ImageIdentity,
    range: RangeShape,
    wheel_delta: i32,
    owned_blank_fixture: bool,
    consent_to_finite_input: bool,
    expected_observation_digest: Option<String>,
    expected_destination_runtime_hash: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invocation {
    mode: String,
    request: Request,
}
fn mode_valid(
    mode: &str,
    consent: bool,
    digest: Option<&str>,
    destination: Option<&str>,
) -> Result<bool> {
    let execute = match mode {
        "--krita-wheel-observe" => false,
        "--krita-wheel-experiment" => true,
        _ => return Err(Error::Invalid),
    };
    if consent != execute
        || if execute {
            digest.is_none_or(|v| !vw_remote::profile::digest(v))
                || destination.is_none_or(|v| !vw_remote::profile::digest(v))
        } else {
            digest.is_some() || destination.is_some()
        }
    {
        return Err(Error::Ungranted);
    }
    Ok(execute)
}
fn phase(clock: &Clock) -> Result<SourceBudget> {
    clock.cancel.check().map_err(|_| Error::Cancelled)?;
    let now = Instant::now();
    let remaining = clock
        .deadline
        .checked_duration_since(now)
        .filter(|v| !v.is_zero())
        .ok_or(Error::Timeout)?;
    SourceBudget::until(
        clock.cancel.clone(),
        now.checked_add(remaining.min(Duration::from_millis(180)))
            .ok_or(Error::Limit)?,
    )
}
fn observe(source: &SourceLease, r: &Request, clock: &Clock) -> Result<State> {
    let budget = phase(clock)?;
    source.verify(&r.target, r.owner_pid, &budget)?;
    let state = wheel::observe(&r.target, &budget, &r.range)?;
    source.verify(&r.target, r.owner_pid, &budget)?;
    budget.check()?;
    Ok(state)
}
fn batch(
    owner: &mut InputOwner,
    producer: &mut super::super::terminal::Producer,
    r: &Request,
    clock: &Clock,
    delta: i32,
    state: &State,
    receipts: &mut Vec<Batch>,
) -> Result<()> {
    let budget = phase(clock)?;
    producer.arm_attempt()?;
    let value = wheel::inject(owner, &r.target, r.owner_pid, &budget, delta, state)?;
    let result = value.complete();
    let elapsed = value.local_preflight_input_micros;
    receipts.push(value);
    result?;
    if elapsed >= 250_000 {
        return Err(Error::Timeout);
    }
    Ok(())
}
#[derive(Serialize)]
struct Receipt {
    schema: u32,
    operation: &'static str,
    target: Target,
    image: ImageIdentity,
    source_proof: crate::editor_source::SourceProof,
    parent_job_present: bool,
    owned_blank_fixture_asserted: bool,
    provider_budget_ms: u32,
    local_preflight_input_budget_ms: u32,
    observation_digest: String,
    before: State,
    batches: Vec<Batch>,
    after: Option<State>,
    observed_numeric_effect: Option<bool>,
    restored_state: Option<State>,
    restoration_exact: bool,
    error: Option<Error>,
    input_retirement: FiniteRetirement,
    common_finite_owner: bool,
    input_sent: bool,
    owned_fixture_control_cleanup_outstanding: bool,
    semantic_role_proven: bool,
    production_parent_exchange_proven: bool,
    profile_authority: bool,
    physical_or_editor_pixel_effect_proven: bool,
}
pub(super) fn run(bytes: &[u8], producer: &mut super::super::terminal::Producer) -> Result<()> {
    let invocation: Invocation = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
    let r = invocation.request;
    if r.schema != 1 || r.owner_pid == 0 || !r.owned_blank_fixture {
        return Err(Error::Invalid);
    }
    let execute = mode_valid(
        &invocation.mode,
        r.consent_to_finite_input,
        r.expected_observation_digest.as_deref(),
        r.expected_destination_runtime_hash.as_deref(),
    )?;
    wheel::delta_valid(r.wheel_delta)?;
    r.range.validate_wheel()?;
    r.target.validate()?;
    exact_image(&r.image)?;
    let clock = Clock::new()?;
    let source = SourceLease::open(
        &r.target,
        r.owner_pid,
        &r.image,
        SourcePolicy::Ordinary,
        &phase(&clock)?,
    )?;
    let before = observe(&source, &r, &clock)?;
    let observation_digest = before.digest()?;
    let mut owner = FiniteInputOwner::new(r.target.clone(), r.owner_pid, source)?;
    let mut batches = vec![];
    let mut after = None;
    let mut restored_state = None;
    let mut observed_numeric_effect = None;
    let mut restoration_exact = false;
    // After a send attempt every error is captured and reaches actual terminal
    // retirement. Unknown/partial/late effects never start another wheel batch.
    let result = (|| -> Result<()> {
        if !execute {
            return Ok(());
        }
        if r.expected_observation_digest.as_deref() != Some(observation_digest.as_str())
            || r.expected_destination_runtime_hash.as_deref()
                != Some(before.numeric.element.runtime_id_hash.as_str())
        {
            return Err(Error::TargetChanged);
        }
        batch(
            &mut owner,
            producer,
            &r,
            &clock,
            r.wheel_delta,
            &before,
            &mut batches,
        )?;
        let current = observe(owner.source(), &r, &clock)?;
        after = Some(current.clone());
        let changed = before.numeric_effect(&current)?;
        observed_numeric_effect = Some(changed);
        if changed {
            // Exactly one inverse detent; no guessed range/pixel mapping, loops,
            // retries, or writable UIA action restores an anonymous candidate.
            batch(
                &mut owner,
                producer,
                &r,
                &clock,
                -r.wheel_delta,
                &current,
                &mut batches,
            )?;
        }
        let restored = observe(owner.source(), &r, &clock)?;
        restoration_exact = restored == before;
        restored_state = Some(restored);
        if !restoration_exact {
            return Err(Error::TargetChanged);
        }
        Ok(())
    })();
    let error = result.err();
    let input_sent = batches.iter().any(|b| b.native_accepted_count > 0);
    producer.finish(&mut owner)?;
    let receipt = Receipt {
        schema: 1,
        operation: "owned-krita-numeric-wheel-calibration",
        target: r.target,
        image: r.image,
        source_proof: owner.source().proof(),
        parent_job_present: true,
        owned_blank_fixture_asserted: true,
        provider_budget_ms: 180,
        local_preflight_input_budget_ms: 250,
        observation_digest,
        before,
        batches,
        after,
        observed_numeric_effect,
        restored_state,
        restoration_exact,
        error,
        input_retirement: FiniteRetirement::Complete,
        common_finite_owner: true,
        input_sent,
        owned_fixture_control_cleanup_outstanding: input_sent && !restoration_exact,
        semantic_role_proven: false,
        production_parent_exchange_proven: false,
        profile_authority: false,
        physical_or_editor_pixel_effect_proven: false,
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
    #[test]
    fn observe_never_accepts_consent_or_imported_expectations() -> Result<()> {
        assert!(!mode_valid("--krita-wheel-observe", false, None, None)?);
        assert_eq!(
            mode_valid("--krita-wheel-observe", true, None, None),
            Err(Error::Ungranted)
        );
        assert_eq!(
            mode_valid("--krita-wheel-observe", false, Some(&"a".repeat(64)), None),
            Err(Error::Ungranted)
        );
        Ok(())
    }
    #[test]
    fn input_needs_exact_closed_mode_consent_and_both_hashes() -> Result<()> {
        let hash = "a".repeat(64);
        assert!(mode_valid(
            "--krita-wheel-experiment",
            true,
            Some(&hash),
            Some(&hash)
        )?);
        assert_eq!(
            mode_valid("--krita-wheel-experiment", false, Some(&hash), Some(&hash)),
            Err(Error::Ungranted)
        );
        assert_eq!(
            mode_valid("--krita-wheel-experiment", true, Some(&hash), None),
            Err(Error::Ungranted)
        );
        assert_eq!(
            mode_valid("--krita-experiment", true, Some(&hash), Some(&hash)),
            Err(Error::Invalid)
        );
        Ok(())
    }
    #[test]
    fn expired_outer_clock_never_opens_fresh_provider_budget() -> Result<()> {
        let c = Clock {
            deadline: Instant::now()
                .checked_sub(Duration::from_millis(1))
                .ok_or(Error::Limit)?,
            cancel: Arc::new(vw_capture::Cancellation::default()),
        };
        assert!(matches!(phase(&c), Err(Error::Timeout)));
        Ok(())
    }
}
