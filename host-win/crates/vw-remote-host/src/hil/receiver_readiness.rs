//! Bounded HIL receiver startup policy. No input or control-grant authority.
use crate::{Error, Result};
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

pub(super) fn acquire_focus(
    expected_window: u64,
    mut unchanged: impl FnMut() -> Result<()>,
    mut foreground: impl FnMut() -> u64,
    ordinary_focus: impl FnOnce() -> Result<()>,
) -> Result<()> {
    unchanged()?;
    if foreground() == expected_window {
        // An ordinary click can establish the same postcondition as the focus
        // API. Recheck identity and actual foreground; this grants no control.
        unchanged()?;
        return if foreground() == expected_window {
            Ok(())
        } else {
            Err(Error::Ungranted)
        };
    }
    ordinary_focus()
}

pub(super) fn wait_for_focus(
    mut elapsed: impl FnMut() -> Duration,
    mut poll: impl FnMut() -> Result<()>,
    mut pause: impl FnMut(Duration),
) -> Result<()> {
    loop {
        if elapsed() >= DEADLINE {
            return Err(Error::Ungranted);
        }
        let ready = match poll() {
            Ok(()) => true,
            Err(Error::Ungranted) => false,
            Err(error) => return Err(error),
        };
        // Message traffic or a slow native call cannot renew the deadline or
        // publish a late success. Only pending ordinary focus is retryable.
        let elapsed = elapsed();
        if elapsed >= DEADLINE {
            return Err(Error::Ungranted);
        }
        if ready {
            return Ok(());
        }
        pause(POLL_INTERVAL.min(DEADLINE - elapsed));
    }
}
