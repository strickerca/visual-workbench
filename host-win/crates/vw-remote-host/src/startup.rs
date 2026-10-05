//! Used after CreateProcess returned ownership: failed gates never run helper code.
use crate::Result;
pub(crate) fn finish(
    assign: impl FnOnce() -> Result<()>,
    proof: impl FnOnce() -> Result<()>,
    resume: impl FnOnce() -> Result<()>,
) -> Result<()> {
    assign()?;
    proof()?;
    resume()
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use std::cell::Cell;
    #[test]
    fn assignment_failure_never_proves_or_resumes() {
        let proof = Cell::new(false);
        let resume = Cell::new(false);
        assert_eq!(
            finish(
                || Err(Error::Unavailable),
                || {
                    proof.set(true);
                    Ok(())
                },
                || {
                    resume.set(true);
                    Ok(())
                }
            ),
            Err(Error::Unavailable)
        );
        assert!(!proof.get() && !resume.get());
    }
    #[test]
    fn namespace_failure_never_resumes_assigned_child() {
        let assigned = Cell::new(false);
        let resumed = Cell::new(false);
        assert_eq!(
            finish(
                || {
                    assigned.set(true);
                    Ok(())
                },
                || Err(Error::Invalid),
                || {
                    resumed.set(true);
                    Ok(())
                }
            ),
            Err(Error::Invalid)
        );
        assert!(assigned.get() && !resumed.get());
    }
    #[test]
    fn resume_error_remains_typed_after_successful_gates() {
        let assigned = Cell::new(false);
        let proved = Cell::new(false);
        assert_eq!(
            finish(
                || {
                    assigned.set(true);
                    Ok(())
                },
                || {
                    proved.set(true);
                    Ok(())
                },
                || Err(Error::Unavailable)
            ),
            Err(Error::Unavailable)
        );
        assert!(assigned.get() && proved.get());
    }
}
