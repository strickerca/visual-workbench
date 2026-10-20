use std::{cell::Cell, time::Duration};
use vw_remote_host::{Error, Result};

#[path = "../src/hil/receiver_readiness.rs"]
mod receiver_readiness;

#[test]
fn early_refusal_keeps_polling_until_ordinary_focus_succeeds() -> Result<()> {
    let elapsed = Cell::new(Duration::ZERO);
    let polls = Cell::new(0);
    receiver_readiness::wait_for_focus(
        || elapsed.get(),
        || {
            polls.set(polls.get() + 1);
            receiver_readiness::acquire_focus(
                10,
                || Ok(()),
                || 99,
                || {
                    if polls.get() < 3 {
                        Err(Error::Ungranted)
                    } else {
                        Ok(())
                    }
                },
            )
        },
        |duration| elapsed.set(elapsed.get() + duration),
    )?;
    assert_eq!(polls.get(), 3);
    assert_eq!(elapsed.get(), Duration::from_millis(40));
    Ok(())
}

#[test]
fn permanent_refusal_expires_without_readiness_or_busy_polling() {
    let elapsed = Cell::new(Duration::ZERO);
    let polls = Cell::new(0);
    let mut pauses = Vec::new();
    let result = receiver_readiness::wait_for_focus(
        || elapsed.get(),
        || {
            polls.set(polls.get() + 1);
            Err(Error::Ungranted)
        },
        |duration| {
            pauses.push(duration);
            elapsed.set(elapsed.get() + duration);
        },
    );
    assert!(matches!(result, Err(Error::Ungranted)));
    assert_eq!(elapsed.get(), Duration::from_secs(10));
    assert_eq!(polls.get(), 500);
    assert_eq!(pauses, vec![Duration::from_millis(20); 500]);
}

#[test]
fn identity_quit_and_paint_errors_stop_immediately() {
    for error in [Error::TargetChanged, Error::Unavailable, Error::Io] {
        let polls = Cell::new(0);
        let pauses = Cell::new(0);
        let result = receiver_readiness::wait_for_focus(
            || Duration::ZERO,
            || {
                polls.set(polls.get() + 1);
                Err(error.clone())
            },
            |_| pauses.set(pauses.get() + 1),
        );
        assert_eq!(result, Err(error));
        assert_eq!(polls.get(), 1);
        assert_eq!(pauses.get(), 0);
    }
}

#[test]
fn success_at_or_after_deadline_is_not_readiness() {
    for finished in [Duration::from_secs(10), Duration::from_secs(11)] {
        let elapsed = Cell::new(Duration::ZERO);
        let pauses = Cell::new(0);
        let result = receiver_readiness::wait_for_focus(
            || elapsed.get(),
            || {
                elapsed.set(finished);
                Ok(())
            },
            |_| pauses.set(pauses.get() + 1),
        );
        assert!(matches!(result, Err(Error::Ungranted)));
        assert_eq!(pauses.get(), 0);
    }
}

#[test]
fn sustained_work_cannot_renew_deadline_and_sleep_is_capped() {
    let elapsed = Cell::new(Duration::from_millis(9990));
    let polls = Cell::new(0);
    let mut pauses = Vec::new();
    let result = receiver_readiness::wait_for_focus(
        || elapsed.get(),
        || {
            polls.set(polls.get() + 1);
            elapsed.set(elapsed.get() + Duration::from_millis(7));
            Err(Error::Ungranted)
        },
        |duration| {
            pauses.push(duration);
            elapsed.set(elapsed.get() + duration);
        },
    );
    assert!(matches!(result, Err(Error::Ungranted)));
    assert_eq!(polls.get(), 1);
    assert_eq!(pauses, [Duration::from_millis(3)]);
    assert_eq!(elapsed.get(), Duration::from_secs(10));
}

#[test]
fn ordinary_click_is_confirmed_without_redundant_focus_request() -> Result<()> {
    let validations = Cell::new(0);
    let observations = Cell::new(0);
    let requests = Cell::new(0);
    receiver_readiness::acquire_focus(
        10,
        || {
            validations.set(validations.get() + 1);
            Ok(())
        },
        || {
            observations.set(observations.get() + 1);
            10
        },
        || {
            requests.set(requests.get() + 1);
            Err(Error::Ungranted)
        },
    )?;
    assert_eq!(validations.get(), 2);
    assert_eq!(observations.get(), 2);
    assert_eq!(requests.get(), 0);
    Ok(())
}

#[test]
fn confirmation_rechecks_foreground_after_identity_validation() {
    let foreground = Cell::new(10);
    let validations = Cell::new(0);
    let result = receiver_readiness::acquire_focus(
        10,
        || {
            validations.set(validations.get() + 1);
            if validations.get() == 2 {
                foreground.set(99);
            }
            Ok(())
        },
        || foreground.get(),
        || Err(Error::Invalid),
    );
    assert!(matches!(result, Err(Error::Ungranted)));
    assert_eq!(validations.get(), 2);
}

#[test]
fn changed_original_identity_is_fatal_without_refresh_or_focus() {
    for changed_on in [1, 2] {
        let validations = Cell::new(0);
        let requests = Cell::new(0);
        let result = receiver_readiness::acquire_focus(
            10,
            || {
                validations.set(validations.get() + 1);
                if validations.get() == changed_on {
                    Err(Error::TargetChanged)
                } else {
                    Ok(())
                }
            },
            || 10,
            || {
                requests.set(requests.get() + 1);
                Ok(())
            },
        );
        assert!(matches!(result, Err(Error::TargetChanged)));
        assert_eq!(validations.get(), changed_on);
        assert_eq!(requests.get(), 0);
    }
}
