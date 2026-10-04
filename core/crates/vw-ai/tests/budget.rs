mod common;
use common::*;
use std::sync::{Arc, Barrier};
use vw_ai::{budget::*, config::Tokens, *};

#[test]
fn explicit_confirmation_and_one_durable_attempt_bind_exact_review() -> TestResult {
    let root = temp()?;
    let path = root.path().join("app-budget.sqlite");
    let p = prepared(1)?;
    let other = prepared(2)?;
    let mut ledger = SqliteBudgetLedger::create(&path)?;
    assert!(matches!(
        ledger.begin_attempt(p.review()),
        Err(Error::Confirmation)
    ));
    let wrong = Confirmation::explicit_send(other.review(), 20000, false)?;
    assert_eq!(
        ledger.reserve(p.review(), wrong, BudgetPolicy::default()),
        Err(Error::Confirmation)
    );
    ledger.reserve(
        p.review(),
        Confirmation::explicit_send(p.review(), 20000, false)?,
        BudgetPolicy::default(),
    )?;
    let permit = ledger.begin_attempt(p.review())?;
    assert!(matches!(
        other.authorize(permit, &NeverCancel),
        Err(Error::Confirmation)
    ));
    assert!(matches!(
        ledger.begin_attempt(p.review()),
        Err(Error::AttemptConsumed)
    ));
    assert_eq!(
        ledger.cancel_unattempted(p.review().request_id()),
        Err(Error::AttemptConsumed)
    );
    assert_eq!(
        ledger.entry(p.review().request_id())?.ok_or("entry")?.state,
        EntryState::Attempted
    );
    Ok(())
}

#[test]
fn absent_estimate_and_zero_estimate_cannot_authorize_spending() -> TestResult {
    for estimate in [
        None,
        Some(Tokens {
            text_input: 0,
            image_input: 0,
            image_output: 0,
        }),
    ] {
        let mut opt = options(1)?;
        opt.estimated_tokens = estimate;
        let source = png8(16, 16, |_, _| [1, 2, 3, 255])?;
        let p = Prepared::new(&source, &[region(16, 16, 1, 1, 4, 4)?], opt, &NeverCancel)?;
        assert!(matches!(
            Confirmation::explicit_send(p.review(), 20000, false),
            Err(Error::Confirmation)
        ));
    }
    Ok(())
}

#[test]
fn reservation_cancel_is_durable_and_same_request_never_replays() -> TestResult {
    let root = temp()?;
    let path = root.path().join("ledger.sqlite");
    let p = prepared(1)?;
    {
        let mut ledger = SqliteBudgetLedger::create(&path)?;
        ledger.reserve(
            p.review(),
            Confirmation::explicit_send(p.review(), 20000, false)?,
            BudgetPolicy::default(),
        )?;
    }
    let mut reopened = SqliteBudgetLedger::open(&path)?;
    assert_eq!(
        reopened
            .entry(p.review().request_id())?
            .ok_or("entry")?
            .state,
        EntryState::Reserved
    );
    reopened.cancel_unattempted(p.review().request_id())?;
    reopened.cancel_unattempted(p.review().request_id())?;
    assert_eq!(reopened.spent_on(20000)?, 0);
    drop(reopened);
    let mut reopened = SqliteBudgetLedger::open(&path)?;
    assert_eq!(
        reopened.reserve(
            p.review(),
            Confirmation::explicit_send(p.review(), 20000, false)?,
            BudgetPolicy::default()
        ),
        Err(Error::AttemptConsumed)
    );
    assert!(matches!(
        reopened.begin_attempt(p.review()),
        Err(Error::AttemptConsumed)
    ));
    Ok(())
}

#[test]
fn unknown_outcome_survives_restart_blocks_other_intents_and_cannot_refund() -> TestResult {
    let root = temp()?;
    let path = root.path().join("ledger.sqlite");
    let p = prepared(1)?;
    let other = prepared(2)?;
    {
        let mut ledger = SqliteBudgetLedger::create(&path)?;
        ledger.reserve(
            p.review(),
            Confirmation::explicit_send(p.review(), 20000, false)?,
            BudgetPolicy::default(),
        )?;
        drop(ledger.begin_attempt(p.review())?);
    }
    let mut ledger = SqliteBudgetLedger::open(&path)?;
    assert!(matches!(
        ledger.begin_attempt(p.review()),
        Err(Error::AttemptConsumed)
    ));
    assert_eq!(
        ledger.reserve(
            other.review(),
            Confirmation::explicit_send(other.review(), 20001, true)?,
            BudgetPolicy::default()
        ),
        Err(Error::Unresolved)
    );
    assert_eq!(
        ledger.cancel_unattempted(p.review().request_id()),
        Err(Error::AttemptConsumed)
    );
    assert_eq!(
        ledger.spent_on(20000)?,
        p.review().estimated_microusd().ok_or("quote")?
    );
    Ok(())
}

#[test]
fn soft_budget_acknowledgement_actual_cost_and_clock_rollback_are_explicit() -> TestResult {
    let root = temp()?;
    let path = root.path().join("ledger.sqlite");
    let p = prepared(1)?;
    let next = prepared(2)?;
    let mut ledger = SqliteBudgetLedger::create(&path)?;
    let policy = BudgetPolicy {
        daily_soft_limit_microusd: 1,
    };
    assert_eq!(
        ledger.reserve(
            p.review(),
            Confirmation::explicit_send(p.review(), 20000, false)?,
            policy
        ),
        Err(Error::SoftBudget)
    );
    assert!(ledger.entry(p.review().request_id())?.is_none());
    ledger.reserve(
        p.review(),
        Confirmation::explicit_send(p.review(), 20000, true)?,
        policy,
    )?;
    drop(ledger.begin_attempt(p.review())?);
    let tokens = Tokens {
        text_input: 100,
        image_input: 200,
        image_output: 300,
    };
    let actual = ledger.settle(p.review().request_id(), p.provider(), tokens)?;
    assert_eq!(actual, 1400);
    assert!(actual > p.review().estimated_microusd().ok_or("quote")?);
    assert_eq!(
        ledger.settle(p.review().request_id(), p.provider(), tokens)?,
        actual
    );
    assert_eq!(ledger.spent_on(20000)?, actual);
    assert_eq!(
        ledger.reserve(
            next.review(),
            Confirmation::explicit_send(next.review(), 19999, true)?,
            policy
        ),
        Err(Error::ClockRegression)
    );
    ledger.reserve(
        next.review(),
        Confirmation::explicit_send(next.review(), 20001, true)?,
        policy,
    )?;
    Ok(())
}

#[test]
fn settlement_rejects_wrong_provider_or_different_second_charge() -> TestResult {
    let root = temp()?;
    let p = prepared(1)?;
    let mut ledger = SqliteBudgetLedger::create(&root.path().join("ledger.sqlite"))?;
    ledger.reserve(
        p.review(),
        Confirmation::explicit_send(p.review(), 20000, false)?,
        BudgetPolicy::default(),
    )?;
    drop(ledger.begin_attempt(p.review())?);
    let tokens = Tokens {
        text_input: 1,
        image_input: 2,
        image_output: 3,
    };
    let mut altered = p.provider().clone();
    altered.model.push_str("-other");
    assert_eq!(
        ledger.settle(p.review().request_id(), &altered, tokens),
        Err(Error::Confirmation)
    );
    ledger.settle(p.review().request_id(), p.provider(), tokens)?;
    assert_eq!(
        ledger.settle(
            p.review().request_id(),
            p.provider(),
            Tokens {
                image_output: 4,
                ..tokens
            }
        ),
        Err(Error::AttemptConsumed)
    );
    Ok(())
}

#[test]
fn independent_connections_cannot_both_obtain_the_same_attempt_permit() -> TestResult {
    let root = temp()?;
    let path = root.path().join("ledger.sqlite");
    let p = prepared(1)?;
    let mut ledger = SqliteBudgetLedger::create(&path)?;
    ledger.reserve(
        p.review(),
        Confirmation::explicit_send(p.review(), 20000, false)?,
        BudgetPolicy::default(),
    )?;
    drop(ledger);
    let first = SqliteBudgetLedger::open(&path)?;
    let second = SqliteBudgetLedger::open(&path)?;
    let barrier = Arc::new(Barrier::new(2));
    let review = p.review().clone();
    let handles = [first, second]
        .into_iter()
        .map(|mut ledger| {
            let barrier = barrier.clone();
            let review = review.clone();
            std::thread::spawn(move || {
                barrier.wait();
                ledger.begin_attempt(&review).is_ok()
            })
        })
        .collect::<Vec<_>>();
    let mut successes = 0;
    for handle in handles {
        successes += usize::from(handle.join().map_err(|_| "thread failed")?);
    }
    assert_eq!(successes, 1);
    assert_eq!(
        SqliteBudgetLedger::open(&path)?
            .entry(p.review().request_id())?
            .ok_or("entry")?
            .state,
        EntryState::Attempted
    );
    Ok(())
}

#[test]
fn missing_corrupt_or_incompatible_ledgers_are_not_silently_reinitialized() -> TestResult {
    let root = temp()?;
    let path = root.path().join("ledger.sqlite");
    assert!(matches!(
        SqliteBudgetLedger::open(&path),
        Err(Error::Storage)
    ));
    assert!(!path.exists());
    std::fs::write(&path, b"not a ledger")?;
    assert!(matches!(
        SqliteBudgetLedger::open(&path),
        Err(Error::Storage)
    ));
    assert_eq!(std::fs::read(&path)?, b"not a ledger");
    let other = root.path().join("other.sqlite");
    drop(SqliteBudgetLedger::create(&other)?);
    let connection = rusqlite::Connection::open(&other)?;
    connection.execute_batch("PRAGMA user_version=2")?;
    drop(connection);
    assert!(matches!(
        SqliteBudgetLedger::open(&other),
        Err(Error::Storage)
    ));
    let invalid = root.path().join("invalid.sqlite");
    drop(SqliteBudgetLedger::create(&invalid)?);
    let connection = rusqlite::Connection::open(&invalid)?;
    connection.execute("UPDATE meta SET sequence=1 WHERE id=1", [])?;
    drop(connection);
    assert!(matches!(
        SqliteBudgetLedger::open(&invalid),
        Err(Error::Storage)
    ));
    Ok(())
}

#[test]
fn unexpected_schema_refuses_reservation_before_mutating() -> TestResult {
    let root = temp()?;
    let path = root.path().join("ledger.sqlite");
    let mut ledger = SqliteBudgetLedger::create(&path)?;
    let p = prepared(1)?;
    let connection = rusqlite::Connection::open(&path)?;
    connection.execute_batch("CREATE TRIGGER refuse_meta BEFORE UPDATE ON meta BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;")?;
    assert_eq!(
        ledger.reserve(
            p.review(),
            Confirmation::explicit_send(p.review(), 20000, false)?,
            BudgetPolicy::default()
        ),
        Err(Error::Storage)
    );
    assert_eq!(
        connection.query_row("SELECT count(*) FROM entries", [], |row| row
            .get::<_, i64>(0))?,
        0
    );
    connection.execute_batch("DROP TRIGGER refuse_meta")?;
    assert!(ledger.entry(p.review().request_id())?.is_none());
    drop(ledger);
    drop(connection);
    assert_eq!(SqliteBudgetLedger::open(&path)?.spent_on(20000)?, 0);
    Ok(())
}

#[test]
fn multipart_and_response_mapping_are_bounded_and_bound_to_request() -> TestResult {
    let root = temp()?;
    let p = prepared(1)?;
    let other = prepared(2)?;
    let mut ledger = SqliteBudgetLedger::create(&root.path().join("ledger.sqlite"))?;
    ledger.reserve(
        p.review(),
        Confirmation::explicit_send(p.review(), 20000, false)?,
        BudgetPolicy::default(),
    )?;
    let request = p.authorize(ledger.begin_attempt(p.review())?, &NeverCancel)?;
    let text = String::from_utf8_lossy(request.body());
    assert!(text.contains("name=\"model\"\r\n\r\noffline-fixture-model"));
    assert!(text.contains("filename=\"source.png\""));
    assert!(!text.contains("Authorization:"));
    assert_eq!(request.endpoint(), "https://api.openai.com/v1/images/edits");
    assert_eq!(request.request_id(), p.review().request_id());
    use base64::Engine;
    let png = png8(
        p.review().crop().model_width,
        p.review().crop().model_height,
        |_, _| [220, 30, 100, 255],
    )?;
    let bytes = serde_json::to_vec(
        &serde_json::json!({"data":[{"b64_json":base64::engine::general_purpose::STANDARD.encode(&png)}],"usage":{"input_tokens":3,"input_tokens_details":{"text_tokens":1,"image_tokens":2},"output_tokens":3,"total_tokens":6}}),
    )?;
    let response = request.parse_response(&bytes)?;
    assert_eq!(response.origin(), ResponseOrigin::ProviderPayload);
    assert_eq!(
        response.tokens(),
        Some(Tokens {
            text_input: 1,
            image_input: 2,
            image_output: 3
        })
    );
    assert!(matches!(
        other.finish(response, &NeverCancel),
        Err(Error::Confirmation)
    ));
    let complete = p.finish(request.parse_response(&bytes)?, &NeverCancel)?;
    assert_eq!(complete.actual_microusd(), Some(14));
    for bad in [
        br#"{"data":[]}"#.as_slice(),
        br#"{"data":[{"b64_json":"???"}]}"#.as_slice(),
    ] {
        assert!(request.parse_response(bad).is_err());
    }
    let mut invalid = serde_json::from_slice::<serde_json::Value>(&bytes)?;
    invalid["usage"]["total_tokens"] = 999.into();
    assert!(
        request
            .parse_response(&serde_json::to_vec(&invalid)?)
            .is_err()
    );
    let mut invalid = serde_json::from_slice::<serde_json::Value>(&bytes)?;
    let duplicate = invalid["data"][0].clone();
    invalid["data"]
        .as_array_mut()
        .ok_or("data array")?
        .push(duplicate);
    assert!(
        request
            .parse_response(&serde_json::to_vec(&invalid)?)
            .is_err()
    );
    assert!(
        request
            .parse_response(&vec![0; request.max_response_bytes() + 1])
            .is_err()
    );
    let wrong = png8(1, 1, |_, _| [0, 0, 0, 255])?;
    let wrong = serde_json::to_vec(
        &serde_json::json!({"data":[{"b64_json":base64::engine::general_purpose::STANDARD.encode(wrong)}]}),
    )?;
    assert!(
        p.finish(request.parse_response(&wrong)?, &NeverCancel)
            .is_err()
    );
    Ok(())
}

#[test]
fn killed_process_keeps_reservation_or_attempt_committed() -> TestResult {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    for state in ["reserved", "attempted"] {
        let root = temp()?;
        let path = root.path().join("ledger.sqlite");
        let marker = root.path().join("ready");
        let child = Command::new(std::env::current_exe()?)
            .args(["--ignored", "--exact", "crash_worker"])
            .env("VW_AI_LEDGER_WORKER", root.path())
            .env("VW_AI_LEDGER_STATE", state)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let mut child = ChildGuard(child);
        let started = Instant::now();
        while !marker.exists() {
            if child.0.try_wait()?.is_some() {
                return Err("crash worker ended before durable marker".into());
            }
            if started.elapsed() > Duration::from_secs(10) {
                return Err("crash worker startup exceeded 10s".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        child.0.kill()?;
        child.0.wait()?;
        drop(child);
        let p = prepared(1)?;
        let mut ledger = SqliteBudgetLedger::open(&path)?;
        assert_eq!(
            ledger.entry(p.review().request_id())?.ok_or("entry")?.state,
            if state == "reserved" {
                EntryState::Reserved
            } else {
                EntryState::Attempted
            }
        );
        if state == "reserved" {
            drop(ledger.begin_attempt(p.review())?);
        } else {
            assert!(matches!(
                ledger.begin_attempt(p.review()),
                Err(Error::AttemptConsumed)
            ));
        }
        assert_eq!(
            ledger.cancel_unattempted(p.review().request_id()),
            Err(Error::AttemptConsumed)
        );
    }
    Ok(())
}

#[test]
#[ignore = "subprocess fixture; parent runs it with a private owned directory"]
fn crash_worker() -> TestResult {
    use std::io::Write;
    let Some(directory) = std::env::var_os("VW_AI_LEDGER_WORKER") else {
        return Ok(());
    };
    let directory = std::path::PathBuf::from(directory);
    let p = prepared(1)?;
    let mut ledger = SqliteBudgetLedger::create(&directory.join("ledger.sqlite"))?;
    ledger.reserve(
        p.review(),
        Confirmation::explicit_send(p.review(), 20000, false)?,
        BudgetPolicy::default(),
    )?;
    if std::env::var("VW_AI_LEDGER_STATE")?.as_str() == "attempted" {
        drop(ledger.begin_attempt(p.review())?);
    }
    let mut marker = std::fs::File::create(directory.join("ready"))?;
    marker.write_all(b"durable")?;
    marker.sync_all()?;
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(15) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Err("parent failed to kill fixture".into())
}
