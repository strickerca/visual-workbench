//! Completion checks never hide a blocked call behind a fresh timer.
use crate::{Error, Result};
use std::time::{Duration, Instant};
pub(crate) fn next(
    previous: Option<Instant>,
    started: Instant,
    completed: Instant,
    continues: bool,
) -> Result<Option<Instant>> {
    let deadline = Duration::from_millis(50);
    if completed.saturating_duration_since(started) >= deadline
        || previous.is_some_and(|p| completed.saturating_duration_since(p) >= deadline)
    {
        return Err(Error::Timeout);
    }
    Ok(continues.then_some(started))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blocked_down_is_not_acknowledged() {
        let t = Instant::now();
        assert_eq!(
            next(None, t, t + Duration::from_millis(50), true),
            Err(Error::Timeout)
        );
    }
    #[test]
    fn slow_final_refresh_does_not_reset_previous_deadline() {
        let t = Instant::now();
        assert_eq!(
            next(
                Some(t),
                t + Duration::from_millis(40),
                t + Duration::from_millis(51),
                true
            ),
            Err(Error::Timeout)
        );
    }
    #[test]
    fn slow_up_is_typed_failure_even_if_os_returned_success() {
        let t = Instant::now();
        assert_eq!(
            next(
                Some(t),
                t + Duration::from_millis(49),
                t + Duration::from_millis(51),
                false
            ),
            Err(Error::Timeout)
        );
    }
    #[test]
    fn refresh_anchor_is_attempt_start() {
        let t = Instant::now();
        assert_eq!(
            next(
                Some(t),
                t + Duration::from_millis(20),
                t + Duration::from_millis(25),
                true
            ),
            Ok(Some(t + Duration::from_millis(20)))
        );
    }
}
