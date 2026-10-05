//! Ordered actual finite attempts. Only the current command's first receipt
//! can authorize its second producer; no fake retirement or replacement owner.
use crate::{Error, Result};
use vw_remote::wire::{FiniteAttempt, FiniteRetirement};
pub(super) trait Stages {
    fn first(&mut self) -> FiniteAttempt;
    fn second(&mut self, first: &FiniteAttempt) -> Result<Option<FiniteAttempt>>;
}
pub(super) fn provider_ready(contact: bool, attempt: Option<&FiniteAttempt>) -> Result<()> {
    if contact {
        return Err(Error::Unavailable);
    }
    if attempt.is_some_and(|v| !matches!(v.retirement, FiniteRetirement::Complete)) {
        return Err(Error::RetirementPending);
    }
    Ok(())
}
fn complete(attempt: &FiniteAttempt) -> Result<()> {
    if !matches!(attempt.retirement, FiniteRetirement::Complete) {
        return Err(Error::RetirementPending);
    }
    if let Some(error) = &attempt.error {
        return Err(error.clone());
    }
    if !(1..=16).contains(&attempt.expected_count)
        || attempt.accepted_count != attempt.expected_count
        || !attempt.accepted_qpc_100ns.is_some_and(|v| v > 0)
    {
        return Err(Error::PartialInput);
    }
    Ok(())
}
/// Report partial command effects without discarding the precise failing stage
/// cause, accepted counts/time, or actual current retirement/unknown ownership.
pub(super) fn failure(receipt: &mut Option<FiniteAttempt>, error: Error) -> Error {
    if let Some(attempt) = receipt {
        if attempt.error.is_none() {
            attempt.error = Some(error.clone());
        }
        if attempt.accepted_count > 0 {
            return Error::PartialInput;
        }
    }
    error
}
fn combined(first: &FiniteAttempt, second: FiniteAttempt) -> FiniteAttempt {
    // First was proven Complete. The same live owner's latest stage therefore
    // supplies retirement; counts preserve all actual attempts in this command.
    FiniteAttempt {
        expected_count: first.expected_count.saturating_add(second.expected_count),
        accepted_count: first.accepted_count.saturating_add(second.accepted_count),
        accepted_qpc_100ns: if second.accepted_count > 0 {
            second.accepted_qpc_100ns
        } else {
            first.accepted_qpc_100ns
        },
        error: second.error,
        retirement: second.retirement,
    }
}
pub(super) fn execute(port: &mut impl Stages, receipt: &mut Option<FiniteAttempt>) -> Result<()> {
    let first = port.first();
    *receipt = Some(first.clone());
    if let Err(error) = complete(&first) {
        return Err(failure(receipt, error));
    }
    let second = match port.second(&first) {
        Ok(value) => value,
        Err(error) => return Err(failure(receipt, error)),
    };
    if let Some(second) = second {
        let result = complete(&second);
        *receipt = Some(combined(&first, second));
        if let Err(error) = result {
            return Err(failure(receipt, error));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Port {
        first: FiniteAttempt,
        second: Result<Option<FiniteAttempt>>,
        calls: usize,
        first_clock: Option<u64>,
    }
    impl Stages for Port {
        fn first(&mut self) -> FiniteAttempt {
            self.first.clone()
        }
        fn second(&mut self, first: &FiniteAttempt) -> Result<Option<FiniteAttempt>> {
            self.calls += 1;
            self.first_clock = first.accepted_qpc_100ns;
            self.second.clone()
        }
    }
    fn attempt(expected: u32, accepted: u32) -> FiniteAttempt {
        FiniteAttempt {
            expected_count: expected,
            accepted_count: accepted,
            accepted_qpc_100ns: (accepted > 0).then_some(10),
            error: None,
            retirement: FiniteRetirement::Complete,
        }
    }
    fn port() -> Port {
        Port {
            first: attempt(3, 3),
            second: Ok(Some(attempt(2, 2))),
            calls: 0,
            first_clock: None,
        }
    }
    fn pending() -> FiniteRetirement {
        FiniteRetirement::Pending {
            held_count: 1,
            uncertain: false,
            error: Error::TargetChanged,
        }
    }
    #[test]
    fn first_pending_never_prepares_second_and_retains_owner_receipt() -> Result<()> {
        let mut p = port();
        p.first.retirement = pending();
        let mut receipt = None;
        assert_eq!(execute(&mut p, &mut receipt), Err(Error::PartialInput));
        assert_eq!(p.calls, 0);
        let r = receipt.ok_or(Error::Invalid)?;
        assert_eq!(r.accepted_count, 3);
        assert!(matches!(
            r.retirement,
            FiniteRetirement::Pending { held_count: 1, .. }
        ));

        Ok(())
    }
    #[test]
    fn first_complete_with_error_never_prepares_second() -> Result<()> {
        let mut p = port();
        p.first.error = Some(Error::TargetChanged);
        let mut receipt = None;
        assert_eq!(execute(&mut p, &mut receipt), Err(Error::PartialInput));
        assert_eq!(p.calls, 0);
        assert_eq!(
            receipt.ok_or(Error::Invalid)?.error,
            Some(Error::TargetChanged)
        );

        Ok(())
    }
    #[test]
    fn first_partial_or_unknown_clock_cannot_authorize_second() -> Result<()> {
        for mode in 0..3 {
            let mut p = port();
            match mode {
                0 => p.first.accepted_count = 2,
                1 => p.first.accepted_qpc_100ns = None,
                _ => p.first.accepted_qpc_100ns = Some(0),
            };
            assert_eq!(execute(&mut p, &mut None), Err(Error::PartialInput));
            assert_eq!(p.calls, 0);
        }

        Ok(())
    }
    #[test]
    fn second_preparation_failure_preserves_first_effect_and_precise_error() -> Result<()> {
        let mut p = port();
        p.second = Err(Error::Timeout);
        let mut receipt = None;
        assert_eq!(execute(&mut p, &mut receipt), Err(Error::PartialInput));
        assert_eq!(p.first_clock, Some(10));
        let r = receipt.ok_or(Error::Invalid)?;
        assert_eq!(
            (r.expected_count, r.accepted_count, r.accepted_qpc_100ns),
            (3, 3, Some(10))
        );
        assert_eq!(r.error, Some(Error::Timeout));

        Ok(())
    }
    #[test]
    fn second_pending_preserves_cumulative_counts_and_latest_release_owner() -> Result<()> {
        let mut p = port();
        let mut second = attempt(2, 1);
        second.retirement = pending();
        second.error = Some(Error::PartialInput);
        second.accepted_qpc_100ns = Some(20);
        p.second = Ok(Some(second));
        let mut receipt = None;
        assert_eq!(execute(&mut p, &mut receipt), Err(Error::PartialInput));
        let r = receipt.ok_or(Error::Invalid)?;
        assert_eq!(
            (r.expected_count, r.accepted_count, r.accepted_qpc_100ns),
            (5, 4, Some(20))
        );
        assert!(matches!(
            r.retirement,
            FiniteRetirement::Pending { held_count: 1, .. }
        ));

        Ok(())
    }
    #[test]
    fn second_guard_refusal_does_not_erase_prior_accepted_input() -> Result<()> {
        let mut p = port();
        let mut second = attempt(2, 0);
        second.error = Some(Error::TargetChanged);
        p.second = Ok(Some(second));
        let mut receipt = None;
        assert_eq!(execute(&mut p, &mut receipt), Err(Error::PartialInput));
        let r = receipt.ok_or(Error::Invalid)?;
        assert_eq!(
            (r.expected_count, r.accepted_count, r.accepted_qpc_100ns),
            (5, 3, Some(10))
        );
        assert_eq!(r.error, Some(Error::TargetChanged));

        Ok(())
    }
    #[test]
    fn complete_two_stage_command_keeps_actual_last_clock() -> Result<()> {
        let mut p = port();
        let mut second = attempt(2, 2);
        second.accepted_qpc_100ns = Some(20);
        p.second = Ok(Some(second));
        let mut receipt = None;
        assert_eq!(execute(&mut p, &mut receipt), Ok(()));
        let r = receipt.ok_or(Error::Invalid)?;
        assert_eq!(
            (r.expected_count, r.accepted_count, r.accepted_qpc_100ns),
            (5, 5, Some(20))
        );
        assert!(matches!(r.retirement, FiniteRetirement::Complete));
        assert_eq!(r.error, None);

        Ok(())
    }
    #[test]
    fn one_stage_command_has_no_manufactured_second_attempt() -> Result<()> {
        let mut p = port();
        p.second = Ok(None);
        let mut receipt = None;
        assert_eq!(execute(&mut p, &mut receipt), Ok(()));
        let r = receipt.ok_or(Error::Invalid)?;
        assert_eq!((r.expected_count, r.accepted_count), (3, 3));

        Ok(())
    }
    #[test]
    fn no_input_refusal_remains_original_typed_error() -> Result<()> {
        let mut p = port();
        p.first = attempt(3, 0);
        p.first.error = Some(Error::Ungranted);
        let mut receipt = None;
        assert_eq!(execute(&mut p, &mut receipt), Err(Error::Ungranted));
        assert_eq!(p.calls, 0);

        Ok(())
    }
    #[test]
    fn refresh_requires_no_contact_and_actual_finite_release() -> Result<()> {
        assert_eq!(provider_ready(true, None), Err(Error::Unavailable));
        let mut a = attempt(3, 2);
        a.retirement = pending();
        assert_eq!(
            provider_ready(false, Some(&a)),
            Err(Error::RetirementPending)
        );
        assert_eq!(provider_ready(false, Some(&attempt(3, 3))), Ok(()));
        assert_eq!(provider_ready(false, None), Ok(()));

        Ok(())
    }
    #[test]
    fn post_send_clock_failure_preserves_receipt_effect() -> Result<()> {
        let mut receipt = Some(attempt(3, 3));
        assert_eq!(
            failure(&mut receipt, Error::Unavailable),
            Error::PartialInput
        );
        let r = receipt.ok_or(Error::Invalid)?;
        assert_eq!(r.accepted_count, 3);
        assert_eq!(r.error, Some(Error::Unavailable));

        Ok(())
    }
}
