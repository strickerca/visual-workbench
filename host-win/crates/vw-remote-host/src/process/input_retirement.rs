//! Synthetic devices are process scoped; finite SendInput has a separate fence.
use crate::{Error, Result};
use vw_remote::{
    Binding,
    wire::{Header, PenReleaseWitness},
};

pub(super) fn already_gone(
    owner: &mut impl crate::retirement::Owner,
    finite_pending: bool,
) -> Result<bool> {
    if finite_pending {
        return Ok(false);
    }
    crate::retirement::settle_once(owner)
}

pub(super) struct Fence {
    binding: Binding,
    stop: Option<u64>,
    stopped: bool,
    witness: Option<PenReleaseWitness>,
}
impl Fence {
    pub fn new(binding: Binding) -> Self {
        Self {
            binding,
            stop: None,
            stopped: false,
            witness: None,
        }
    }
    pub fn pending(&self) -> bool {
        !self.stopped
    }
    pub fn stop_sequence(&self) -> Option<u64> {
        self.stop
    }
    pub fn sent_stop(&mut self, sequence: u64) -> Result<()> {
        if sequence == 0 || self.stop.is_some() {
            return Err(Error::Invalid);
        }
        self.stop = Some(sequence);
        Ok(())
    }
    pub fn observe(&mut self, header: &Header) {
        if let Header::Stopped {
            sequence,
            pen_release,
        } = header
            && !self.stopped
            && self.stop == Some(*sequence)
        {
            self.stopped = true;
            // Diagnostic failure never falsifies the release-only terminal or
            // alters cleanup. It simply cannot establish fixture acceptance.
            self.witness = pen_release
                .as_ref()
                .filter(|v| {
                    v.binding == self.binding
                        && v.binding.validate().is_ok()
                        && v.release_started_qpc > 0
                        && v.release_completed_qpc >= v.release_started_qpc
                        && v.qpc_frequency > 0
                })
                .cloned();
        }
    }
    pub fn witness(&self) -> Option<&PenReleaseWitness> {
        self.witness.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn stopped(sequence: u64) -> Header {
        Header::Stopped {
            sequence,
            pen_release: None,
        }
    }
    fn proof() -> PenReleaseWitness {
        PenReleaseWitness {
            binding: binding(),
            held_before_release: true,
            release_started_qpc: 100,
            release_completed_qpc: 101,
            qpc_frequency: 10_000_000,
        }
    }
    #[test]
    fn queued_open_requires_release_without_any_pen_receipt() {
        let f = Fence::new(binding());
        assert!(f.pending());
        assert_eq!(f.stop_sequence(), None);
    }
    #[test]
    fn no_reply_or_pipe_eof_cannot_infer_stopped() -> Result<()> {
        let mut f = Fence::new(binding());
        f.sent_stop(2)?;
        // Neither elapsed time nor endpoint disconnection changes this fence.
        assert!(f.pending());
        assert!(f.witness().is_none());
        Ok(())
    }
    #[test]
    fn stale_or_unsolicited_stopped_cannot_retire_current_owner() -> Result<()> {
        let mut f = Fence::new(binding());
        f.observe(&stopped(2));
        assert!(f.pending());
        f.sent_stop(3)?;
        f.observe(&stopped(2));
        assert!(f.pending());
        f.observe(&stopped(3));
        assert!(!f.pending());
        Ok(())
    }
    #[test]
    fn late_matching_stop_settles_same_fence_without_new_request() -> Result<()> {
        let mut f = Fence::new(binding());
        f.sent_stop(3)?;
        assert!(f.pending());
        assert_eq!(f.stop_sequence(), Some(3));
        f.observe(&stopped(3));
        assert!(!f.pending());
        assert_eq!(f.sent_stop(4), Err(Error::Invalid));
        Ok(())
    }
    #[test]
    fn duplicate_terminal_cannot_replace_first_release_witness() -> Result<()> {
        let mut f = Fence::new(binding());
        f.sent_stop(3)?;
        f.observe(&Header::Stopped {
            sequence: 3,
            pen_release: Some(proof()),
        });
        let mut later = proof();
        later.release_started_qpc = 200;
        later.release_completed_qpc = 201;
        f.observe(&Header::Stopped {
            sequence: 3,
            pen_release: Some(later),
        });
        assert_eq!(f.witness().map(|v| v.release_started_qpc), Some(100));
        Ok(())
    }
    #[test]
    fn wrong_binding_or_bad_clock_refuses_only_diagnostic_claim() -> Result<()> {
        for mode in 0..4 {
            let mut f = Fence::new(binding());
            f.sent_stop(3)?;
            let mut p = proof();
            match mode {
                0 => p.binding.scope.connection_epoch = 2,
                1 => p.release_started_qpc = 0,
                2 => p.release_completed_qpc = 99,
                _ => p.qpc_frequency = 0,
            }
            f.observe(&Header::Stopped {
                sequence: 3,
                pen_release: Some(p),
            });
            assert!(!f.pending());
            assert!(f.witness().is_none());
        }
        Ok(())
    }
    struct Owner {
        exited: Result<bool>,
        io: bool,
        joined: bool,
    }
    impl crate::retirement::Owner for Owner {
        fn tree_exited(&self) -> Result<bool> {
            self.exited.clone()
        }
        fn io_finished(&self) -> bool {
            self.io
        }
        fn join_io(&mut self) -> Result<()> {
            self.joined = true;
            Ok(())
        }
    }
    #[test]
    fn already_gone_pen_requires_whole_tree_and_reader_completion() -> Result<()> {
        let mut o = Owner {
            exited: Ok(false),
            io: true,
            joined: false,
        };
        assert!(!already_gone(&mut o, false)?);
        assert!(!o.joined);
        o.exited = Ok(true);
        o.io = false;
        assert!(!already_gone(&mut o, false)?);
        assert!(!o.joined);
        o.io = true;
        assert!(already_gone(&mut o, false)?);
        assert!(o.joined);
        Ok(())
    }
    #[test]
    fn pipe_failure_or_exit_query_error_never_substitutes_for_os_settlement() {
        let mut o = Owner {
            exited: Err(Error::Io),
            io: true,
            joined: false,
        };
        assert_eq!(already_gone(&mut o, false), Err(Error::RetirementPending));
        assert!(!o.joined);
    }
    #[test]
    fn actual_process_exit_cannot_clear_finite_sendinput_uncertainty() -> Result<()> {
        let mut o = Owner {
            exited: Ok(true),
            io: true,
            joined: false,
        };
        assert!(!already_gone(&mut o, true)?);
        assert!(!o.joined);
        Ok(())
    }
}
