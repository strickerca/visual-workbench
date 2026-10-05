//! Shared real retirement predicate, with injected fault paths in unit fixtures.
use crate::{Error, Result};
pub(crate) trait Owner {
    fn tree_exited(&self) -> Result<bool>;
    fn io_finished(&self) -> bool;
    fn join_io(&mut self) -> Result<()>;
}
pub(crate) fn settle_once(owner: &mut impl Owner) -> Result<bool> {
    // A wait/query error is unknown ownership. Never convert it to closed.
    if !owner.tree_exited().map_err(|_| Error::RetirementPending)? || !owner.io_finished() {
        return Ok(false);
    }
    owner.join_io()?;
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Fault {
        exit: Result<bool>,
        io: bool,
        join: Result<()>,
        joined: bool,
    }
    impl Owner for Fault {
        fn tree_exited(&self) -> Result<bool> {
            self.exit.clone()
        }
        fn io_finished(&self) -> bool {
            self.io
        }
        fn join_io(&mut self) -> Result<()> {
            self.joined = true;
            self.join.clone()
        }
    }
    fn owner() -> Fault {
        Fault {
            exit: Ok(true),
            io: true,
            join: Ok(()),
            joined: false,
        }
    }
    #[test]
    fn wait_and_job_query_errors_preserve_io_ownership() {
        for error in [Error::Io, Error::Unavailable] {
            let mut o = owner();
            o.exit = Err(error);
            assert_eq!(settle_once(&mut o), Err(Error::RetirementPending));
            assert!(!o.joined)
        }
    }
    #[test]
    fn exited_tree_does_not_close_with_blocked_io() {
        let mut o = owner();
        o.io = false;
        assert_eq!(settle_once(&mut o), Ok(false));
        assert!(!o.joined);
        o.io = true;
        assert_eq!(settle_once(&mut o), Ok(true));
        assert!(o.joined)
    }
    #[test]
    fn finished_io_does_not_close_live_tree() {
        let mut o = owner();
        o.exit = Ok(false);
        assert_eq!(settle_once(&mut o), Ok(false));
        assert!(!o.joined)
    }
    #[test]
    fn worker_panic_join_failure_is_not_success() {
        let mut o = owner();
        o.join = Err(Error::Io);
        assert_eq!(settle_once(&mut o), Err(Error::Io));
        assert!(o.joined)
    }
    #[test]
    fn suspended_child_retirement_requires_affirmative_exit() {
        let mut o = owner();
        o.exit = Ok(false);
        assert_eq!(settle_once(&mut o), Ok(false));
        o.exit = Ok(true);
        assert_eq!(settle_once(&mut o), Ok(true));
    }
}
