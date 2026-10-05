//! Phone-native command barrier. A queue reservation precedes UI enqueue; an
//! actual rendered covering ACK settles prior pen samples. No timer releases it.
use super::*;
use vw_remote::wire::Phase;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Reservation {
    pub input_seq: u64,
    pub action: u32,
    pub nonce: u64,
    pub digest: String,
    pub injected: bool,
    pub refreshed: bool,
}
#[derive(Default)]
pub(super) struct Admission {
    queued: u32,
    contact: bool,
    last_pen: u64,
    covered: u64,
    pub command: Option<Reservation>,
}
impl Admission {
    pub fn busy(&self) -> bool {
        self.command.is_some()
    }
    pub fn queue(&mut self, samples: u32) -> SessionResult<bool> {
        if samples == 0 || samples > 8 {
            return Err(SessionError::Invalid);
        }
        if self.busy() {
            return Ok(false);
        }
        self.queued = self
            .queued
            .checked_add(samples)
            .filter(|v| *v <= 16)
            .ok_or(SessionError::Backpressure)?;
        Ok(true)
    }
    pub fn check_pen(&self, phase: Phase) -> SessionResult<()> {
        if self.busy() {
            return Err(SessionError::Backpressure);
        }
        if (phase == Phase::Down && self.contact) || (phase == Phase::Move && !self.contact) {
            return Err(SessionError::Invalid);
        }
        Ok(())
    }
    pub fn admit_pen(&mut self, phase: Phase, sequence: u64) {
        self.queued = self.queued.saturating_sub(1);
        if phase == Phase::Down {
            self.contact = true;
        }
        if matches!(phase, Phase::Up | Phase::Leave) {
            self.contact = false;
        }
        self.last_pen = sequence;
    }
    pub fn cover(&mut self, sequence: u64) {
        self.covered = self.covered.max(sequence);
    }
    pub fn can_reserve(&self) -> bool {
        !self.busy() && !self.contact && self.queued == 0 && self.covered >= self.last_pen
    }
    pub fn reserve(
        &mut self,
        input_seq: u64,
        action: u32,
        nonce: u64,
        digest: String,
    ) -> SessionResult<()> {
        if !self.can_reserve() {
            return Err(SessionError::Backpressure);
        }
        if input_seq == 0 || !(1..=16).contains(&action) || !vw_remote::profile::digest(&digest) {
            return Err(SessionError::Invalid);
        }
        self.command = Some(Reservation {
            input_seq,
            action,
            nonce,
            digest,
            injected: false,
            refreshed: false,
        });
        Ok(())
    }
    pub fn exact(
        &mut self,
        input_seq: u64,
        action: u32,
        nonce: u64,
    ) -> SessionResult<&mut Reservation> {
        self.command
            .as_mut()
            .filter(|c| c.input_seq == input_seq && c.action == action && c.nonce == nonce)
            .ok_or(SessionError::Authentication)
    }
    pub fn finish(&mut self) -> bool {
        if self
            .command
            .as_ref()
            .is_some_and(|c| c.injected && c.refreshed)
        {
            self.command = None;
            true
        } else {
            false
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn reserve(a: &mut Admission) -> SessionResult<()> {
        a.reserve(3, 1, 0, "a".repeat(64))
    }
    #[test]
    fn queued_down_prevents_command_before_async_pen() -> SessionResult<()> {
        let mut a = Admission::default();
        assert!(a.queue(1)?);
        assert!(reserve(&mut a).is_err());
        Ok(())
    }
    #[test]
    fn held_contact_and_uncovered_up_both_prevent_command() -> SessionResult<()> {
        let mut a = Admission::default();
        a.admit_pen(Phase::Down, 1);
        a.cover(1);
        assert!(reserve(&mut a).is_err());
        a.admit_pen(Phase::Up, 2);
        assert!(reserve(&mut a).is_err());
        a.cover(2);
        reserve(&mut a)?;
        Ok(())
    }
    #[test]
    fn reservation_refuses_new_batch_without_revoking_or_consuming() -> SessionResult<()> {
        let mut a = Admission::default();
        reserve(&mut a)?;
        assert!(!a.queue(1)?);
        assert!(a.check_pen(Phase::Down).is_err());
        assert!(a.busy());
        Ok(())
    }
    #[test]
    fn authority_before_injection_does_not_release() -> SessionResult<()> {
        let mut a = Admission::default();
        reserve(&mut a)?;
        a.exact(3, 1, 0)?.refreshed = true;
        assert!(!a.finish());
        a.exact(3, 1, 0)?.injected = true;
        assert!(a.finish());
        assert!(a.queue(1)?);
        Ok(())
    }
    #[test]
    fn injection_before_authority_does_not_release() -> SessionResult<()> {
        let mut a = Admission::default();
        reserve(&mut a)?;
        a.exact(3, 1, 0)?.injected = true;
        assert!(!a.finish());
        a.exact(3, 1, 0)?.refreshed = true;
        assert!(a.finish());
        Ok(())
    }
    #[test]
    fn wrong_sequence_action_nonce_cannot_settle() -> SessionResult<()> {
        let mut a = Admission::default();
        reserve(&mut a)?;
        assert!(a.exact(4, 1, 0).is_err());
        assert!(a.exact(3, 2, 0).is_err());
        assert!(a.exact(3, 1, 1).is_err());
        assert!(a.busy());
        Ok(())
    }
    #[test]
    fn later_old_coverage_cannot_regress_or_cover_future_up() -> SessionResult<()> {
        let mut a = Admission::default();
        a.admit_pen(Phase::Up, 2);
        a.cover(2);
        a.cover(1);
        assert!(a.can_reserve());
        a.admit_pen(Phase::Hover, 3);
        assert!(!a.can_reserve());
        Ok(())
    }
}
