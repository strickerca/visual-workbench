//! Recovery eligibility only. The actual WGC frame retains its compositor QPC.
use crate::{Error, Result};
use std::time::Duration;
#[derive(Default)]
pub(crate) struct CaptureRecovery {
    attempts: u8,
}
impl CaptureRecovery {
    pub(crate) fn restart(&mut self, pending: bool, elapsed: Duration) -> Result<bool> {
        if !pending || elapsed < Duration::from_millis(250) {
            return Ok(false);
        }
        if self.attempts >= 3 {
            return Err(Error::Timeout);
        }
        self.attempts += 1;
        Ok(true)
    }
    pub(crate) fn captured(&mut self) {
        self.attempts = 0;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_restart_without_request_or_before_cooldown() -> Result<()> {
        let mut recovery = CaptureRecovery::default();
        assert!(!recovery.restart(false, Duration::from_secs(1))?);
        assert!(!recovery.restart(true, Duration::from_millis(249))?);
        assert!(recovery.restart(true, Duration::from_millis(250))?);
        Ok(())
    }
    #[test]
    fn no_progress_fails_after_three_owned_restarts() -> Result<()> {
        let mut recovery = CaptureRecovery::default();
        for _ in 0..3 {
            assert!(recovery.restart(true, Duration::from_millis(250))?);
        }
        assert!(matches!(
            recovery.restart(true, Duration::from_secs(1)),
            Err(Error::Timeout)
        ));
        Ok(())
    }
    #[test]
    fn actual_new_frame_resets_recovery_budget() -> Result<()> {
        let mut recovery = CaptureRecovery::default();
        for _ in 0..3 {
            assert!(recovery.restart(true, Duration::from_millis(250))?);
        }
        recovery.captured();
        assert!(recovery.restart(true, Duration::from_millis(250))?);
        Ok(())
    }
}
