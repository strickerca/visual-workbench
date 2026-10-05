//! Parent uncertainty fence. Child death or a missing pipe receipt proves no UP.
use crate::{Error, Result};
use vw_remote::{
    Binding,
    wire::{FiniteRetirement, Header},
};
#[derive(Default)]
pub(super) struct Fence {
    pending: bool,
    binding: Option<Binding>,
    input_seq: Option<u64>,
    finite_sequence: Option<u64>,
    stop_sequence: Option<u64>,
}
impl Fence {
    pub fn with_binding(binding: Option<Binding>) -> Self {
        Self {
            binding,
            ..Self::default()
        }
    }
    pub fn begin(&mut self, sequence: u64, input_seq: u64) -> Result<()> {
        if self.pending || sequence == 0 || input_seq == 0 {
            return Err(Error::RetirementPending);
        }
        self.pending = true;
        self.input_seq = Some(input_seq);
        self.finite_sequence = Some(sequence);
        self.stop_sequence = None;
        Ok(())
    }
    pub fn not_queued(&mut self, sequence: u64) {
        if self.finite_sequence == Some(sequence) && self.stop_sequence.is_none() {
            self.pending = false;
            self.finite_sequence = None
        }
    }
    pub fn pending(&self) -> bool {
        self.pending
    }
    pub fn stop_sequence(&self) -> Option<u64> {
        self.stop_sequence
    }
    pub fn sent_stop(&mut self, sequence: u64) -> Result<()> {
        if !self.pending || sequence == 0 || self.stop_sequence.is_some() {
            return Err(Error::Invalid);
        }
        self.stop_sequence = Some(sequence);
        Ok(())
    }
    pub fn observe(&mut self, header: &Header) -> Result<()> {
        match header {
            Header::Injected {
                sequence,
                input_seq,
                accepted_qpc_100ns,
                binding,
                ..
            } if self.finite_sequence == Some(*sequence) => {
                if self.input_seq != Some(*input_seq)
                    || self.binding.as_ref() != Some(binding)
                    || binding.validate().is_err()
                    || *accepted_qpc_100ns == 0
                {
                    return Err(Error::Invalid);
                }
                self.pending = false;
            }
            Header::Refused {
                sequence,
                finite_attempt: Some(attempt),
                ..
            } if self.finite_sequence == Some(*sequence) => {
                if attempt.expected_count > 16
                    || attempt.accepted_count > attempt.expected_count
                    || attempt.error.is_none()
                    || attempt.accepted_qpc_100ns == Some(0)
                {
                    return Err(Error::Invalid);
                }
                match &attempt.retirement {
                    FiniteRetirement::Complete => self.pending = false,
                    FiniteRetirement::Pending { held_count, .. } => {
                        if *held_count > 16 {
                            return Err(Error::Invalid);
                        }
                        self.pending = true;
                    }
                }
            }
            Header::Stopped { sequence, .. } if self.stop_sequence == Some(*sequence) => {
                self.pending = false
            }
            _ => {}
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use vw_remote::wire::FiniteAttempt;
    fn refused(sequence: u64, retirement: FiniteRetirement) -> Header {
        Header::Refused {
            sequence,
            error: Error::PartialInput,
            finite_attempt: Some(FiniteAttempt {
                expected_count: 3,
                accepted_count: 2,
                accepted_qpc_100ns: Some(99),
                error: Some(Error::PartialInput),
                retirement,
            }),
        }
    }
    #[test]
    fn finite_queued_is_uncertain_before_any_receipt() -> Result<()> {
        let mut f = Fence::default();
        f.begin(2, 1)?;
        assert!(f.pending());
        Ok(())
    }
    #[test]
    fn failed_queue_has_no_producer_and_clears_only_that_sequence() -> Result<()> {
        let mut f = Fence::default();
        f.begin(2, 1)?;
        f.not_queued(3);
        assert!(f.pending());
        f.not_queued(2);
        assert!(!f.pending());
        Ok(())
    }
    #[test]
    fn exact_complete_refusal_releases_parent_uncertainty() -> Result<()> {
        let mut f = Fence::default();
        f.begin(2, 1)?;
        f.observe(&refused(2, FiniteRetirement::Complete))?;
        assert!(!f.pending());
        Ok(())
    }
    #[test]
    fn pending_refusal_keeps_actual_job_and_io_fence() -> Result<()> {
        let mut f = Fence::default();
        f.begin(2, 1)?;
        f.observe(&refused(
            2,
            FiniteRetirement::Pending {
                held_count: 1,
                uncertain: false,
                error: Error::RetirementPending,
            },
        ))?;
        assert!(f.pending());
        Ok(())
    }
    #[test]
    fn unrelated_receipt_cannot_clear_owned_down() -> Result<()> {
        let mut f = Fence::default();
        f.begin(2, 1)?;
        f.observe(&refused(3, FiniteRetirement::Complete))?;
        f.observe(&Header::Stopped {
            sequence: 4,
            pen_release: None,
        })?;
        assert!(f.pending());
        Ok(())
    }
    #[test]
    fn malformed_prefix_count_keeps_parent_retained() -> Result<()> {
        let mut f = Fence::default();
        f.begin(2, 1)?;
        let mut h = refused(2, FiniteRetirement::Complete);
        if let Header::Refused {
            finite_attempt: Some(a),
            ..
        } = &mut h
        {
            a.accepted_count = 4
        }
        assert_eq!(f.observe(&h), Err(Error::Invalid));
        assert!(f.pending());
        Ok(())
    }
    #[test]
    fn only_exact_requested_stop_can_prove_zero_held() -> Result<()> {
        let mut f = Fence::default();
        f.begin(2, 1)?;
        f.sent_stop(3)?;
        f.observe(&Header::Stopped {
            sequence: 2,
            pen_release: None,
        })?;
        assert!(f.pending());
        f.observe(&Header::Stopped {
            sequence: 3,
            pen_release: None,
        })?;
        assert!(!f.pending());
        Ok(())
    }
    #[test]
    fn cancellation_or_child_death_has_no_implicit_clear() -> Result<()> {
        let mut f = Fence::default();
        f.begin(2, 1)?;
        assert!(f.pending());
        assert_eq!(f.begin(3, 1), Err(Error::RetirementPending));
        assert!(f.pending());
        Ok(())
    }
    #[test]
    fn legacy_refusal_without_retirement_is_unknown_not_complete() -> Result<()> {
        let mut f = Fence::default();
        f.begin(2, 1)?;
        f.observe(&Header::Refused {
            sequence: 2,
            error: Error::PartialInput,
            finite_attempt: None,
        })?;
        assert!(f.pending());
        Ok(())
    }
    fn binding() -> Binding {
        Binding {
            scope: vw_remote::Scope {
                connection_epoch: 1,
                capture_session_id: "00000000-0000-7000-8000-000000000001".into(),
                source_generation: 1,
                target_token: "00000000-0000-7000-8000-000000000002".into(),
                geometry_revision: 1,
            },
            input_session_id: "00000000-0000-7000-8000-000000000003".into(),
        }
    }
    #[test]
    fn successful_exact_injection_clears_only_current_input_session() -> Result<()> {
        let b = binding();
        let mut f = Fence::with_binding(Some(b.clone()));
        f.begin(2, 10)?;
        f.observe(&Header::Injected {
            sequence: 2,
            binding: b,
            input_seq: 10,
            accepted_qpc_100ns: 100,
        })?;
        assert!(!f.pending());
        Ok(())
    }
    #[test]
    fn malformed_injection_scope_sequence_or_zero_clock_keeps_owner() -> Result<()> {
        for mode in 0..3 {
            let b = binding();
            let mut actual = b.clone();
            if mode == 0 {
                actual.input_session_id = "00000000-0000-7000-8000-000000000004".into()
            };
            let mut f = Fence::with_binding(Some(b));
            f.begin(2, 10)?;
            assert_eq!(
                f.observe(&Header::Injected {
                    sequence: 2,
                    binding: actual,
                    input_seq: if mode == 1 { 11 } else { 10 },
                    accepted_qpc_100ns: if mode == 2 { 0 } else { 100 }
                }),
                Err(Error::Invalid)
            );
            assert!(f.pending())
        }
        Ok(())
    }
}
