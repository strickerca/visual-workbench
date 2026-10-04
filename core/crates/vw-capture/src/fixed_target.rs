//! Refresh one already owner-selected identity, never a new foreground window.
use crate::{Error, Result, WindowTarget};
pub fn refreshed_target(
    expected: &WindowTarget,
    current: WindowTarget,
    owner: u32,
) -> Result<WindowTarget> {
    expected.validate(owner)?;
    current.validate(owner)?;
    if expected.window != current.window
        || expected.process_id != current.process_id
        || expected.process_created != current.process_created
        || current.observed_ns < expected.observed_ns
    {
        return Err(Error::Stale);
    }
    // A grant binds the window/process, not obsolete geometry. The capture
    // request subsequently binds this fresh full rectangle/DPI and rechecks it.
    Ok(current)
}
