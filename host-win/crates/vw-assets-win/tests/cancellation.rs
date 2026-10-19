use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use vw_assets_win::{Cancellation, Error};

#[test]
fn local_cancellation_seals_default_and_probed_tokens() {
    for token in [
        Cancellation::default(),
        Cancellation::from_probe(Arc::new(|| false)),
    ] {
        assert_eq!(token.check(), Ok(()));
        token.cancel();
        assert_eq!(token.check(), Err(Error::Cancelled));
    }
}

#[test]
fn caller_cancellation_or_owner_close_is_observed_without_a_watcher() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let closed = Arc::new(AtomicBool::new(false));
    let token = Cancellation::from_probe({
        let cancelled = cancelled.clone();
        let closed = closed.clone();
        Arc::new(move || cancelled.load(Ordering::Acquire) || closed.load(Ordering::Acquire))
    });
    assert_eq!(token.check(), Ok(()));
    closed.store(true, Ordering::Release);
    assert_eq!(token.check(), Err(Error::Cancelled));
    closed.store(false, Ordering::Release);
    cancelled.store(true, Ordering::Release);
    assert_eq!(token.check(), Err(Error::Cancelled));
}

#[test]
fn probe_retains_its_actual_caller_state_until_token_retirement() {
    let owner = Arc::new(AtomicBool::new(false));
    let weak = Arc::downgrade(&owner);
    let token = Cancellation::from_probe(Arc::new(move || owner.load(Ordering::Acquire)));
    assert!(weak.upgrade().is_some());
    assert_eq!(token.check(), Ok(()));
    drop(token);
    assert!(weak.upgrade().is_none());
}
