mod fixtures;
use super::*;
use fixtures::*;
use reqwest::header::{self, HeaderMap, HeaderValue};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use vw_ai::{NeverCancel, budget::*};
use zeroize::Zeroizing;

// Only tests acquiring process-wide worker slots use this lock. Real independent
// API callers remain protected by the production atomic capacity check.
static WORKER_TESTS: Mutex<()> = Mutex::new(());

struct WorkerTest(std::sync::MutexGuard<'static, ()>);
impl Drop for WorkerTest {
    fn drop(&mut self) {
        // Keep the serialization lock until detached workers release capacity,
        // including when a test returns early with an error.
        let _held = &self.0;
        assert!(
            worker::wait_idle(Duration::from_secs(5)),
            "owned worker did not retire"
        );
    }
}
fn workers() -> std::result::Result<WorkerTest, Box<dyn std::error::Error>> {
    Ok(WorkerTest(WORKER_TESTS.lock().map_err(|_| "test lock")?))
}

#[test]
fn secret_is_bounded_redacted_and_header_injection_is_refused() -> TestResult {
    for bytes in [
        Vec::new(),
        vec![b'x'; MAX_SECRET_BYTES + 1],
        b"bad\r\nheader".to_vec(),
        vec![0xff],
    ] {
        assert!(matches!(
            Secret::from_protected_bytes(Zeroizing::new(bytes)),
            Err(Error::Credential(CredentialFailure::Invalid))
        ));
    }
    let secret = Secret::from_protected_bytes(Zeroizing::new(b"synthetic-fixture-only".to_vec()))?;
    assert_eq!(format!("{secret:?}"), "Secret([REDACTED])");
    assert!(secret.authorization()?.is_sensitive());
    Ok(())
}

#[test]
fn response_headers_refuse_compression_duplicates_large_lengths_and_error_bodies() -> TestResult {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("6"));
    assert_eq!(http::admit_headers(200, &headers, 6)?, Some(6));
    assert_eq!(
        http::admit_headers(200, &headers, 5),
        Err(Error::ResponseLimit)
    );
    headers.append(header::CONTENT_LENGTH, HeaderValue::from_static("6"));
    assert_eq!(
        http::admit_headers(200, &headers, 6),
        Err(Error::Representation)
    );
    headers.remove(header::CONTENT_LENGTH);
    headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    assert_eq!(
        http::admit_headers(200, &headers, 6),
        Err(Error::Representation)
    );
    headers.insert(header::RETRY_AFTER, HeaderValue::from_static("12"));
    assert_eq!(
        http::admit_headers(429, &headers, 6),
        Err(Error::Http {
            status: 429,
            retry_after_seconds: Some(12)
        })
    );
    headers.insert(
        header::RETRY_AFTER,
        HeaderValue::from_static("private-untrusted-text"),
    );
    assert_eq!(
        http::admit_headers(503, &headers, 6),
        Err(Error::Http {
            status: 503,
            retry_after_seconds: None
        })
    );
    Ok(())
}

#[test]
fn chunk_limit_refuses_before_append_preserving_existing_bytes() -> TestResult {
    let mut buffer = http::BoundedBytes::new(5)?;
    buffer.append(b"123")?;
    assert_eq!(buffer.append(b"456"), Err(Error::ResponseLimit));
    assert_eq!(buffer.as_slice(), b"123");
    buffer.append(b"45")?;
    assert_eq!(buffer.as_slice(), b"12345");
    assert_eq!(buffer.append(b"6"), Err(Error::ResponseLimit));
    Ok(())
}

#[test]
fn new_usage_categories_cannot_be_silently_priced_as_image() -> TestResult {
    http::admit_usage(br#"{"usage":{"output_tokens":2,"output_tokens_details":{"image_tokens":2,"text_tokens":0}}}"#)?;
    for bytes in [
        br#"{"usage":{"output_tokens":2,"output_tokens_details":{"image_tokens":1,"text_tokens":1}}}"#.as_slice(),
        br#"{"usage":{"output_tokens":2,"output_tokens_details":{"image_tokens":1,"text_tokens":0}}}"#.as_slice(),
    ] { assert_eq!(http::admit_usage(bytes), Err(Error::UsageUnsupported)); }
    assert_eq!(http::admit_usage(b"not JSON"), Err(Error::Representation));
    Ok(())
}

#[test]
fn full_handshake_uploads_exact_multipart_once_then_settles_reported_usage() -> TestResult {
    let _guard = workers()?;
    let root = temp()?;
    let prepared = prepared(1, 1024 * 1024)?;
    let reply = response(&prepared, true)?;
    let fixture = Server::start(reply, ReplyMode::Chunks)?;
    let secrets = Arc::new(FakeSecret::default());
    let mut adapter = adapter(&fixture, secrets.clone())?;
    let mut ledger = SqliteBudgetLedger::create(&root.path().join("ledger.sqlite"))?;
    let expected = authorized_bytes(&prepared, &root.path().join("independent-ledger.sqlite"))?;
    let outcome = adapter.send_confirmed(
        &prepared,
        &mut ledger,
        Confirmation::explicit_send(prepared.review(), 20000, false)?,
        BudgetPolicy::default(),
        &NeverCancel,
    )?;
    assert!(matches!(
        outcome.settlement,
        Settlement::UsagePriced { microusd: 140 }
    ));
    assert_eq!(
        outcome.response.request_id(),
        prepared.review().request_id()
    );
    assert_eq!(secrets.calls.load(Ordering::Acquire), 1);
    assert_eq!(
        ledger
            .entry(prepared.review().request_id())?
            .ok_or("entry")?
            .state,
        EntryState::Settled
    );
    let observed = fixture.finish()?;
    assert_eq!(observed.requests, 1);
    assert_eq!(observed.body, expected);
    assert!(observed.head.starts_with("POST /fixture HTTP/1.1\r\n"));
    assert!(
        observed
            .head
            .to_ascii_lowercase()
            .contains("accept-encoding: identity")
    );
    let repeat = adapter.send_confirmed(
        &prepared,
        &mut ledger,
        Confirmation::explicit_send(prepared.review(), 20000, false)?,
        BudgetPolicy::default(),
        &NeverCancel,
    );
    assert!(matches!(
        repeat,
        Err(Error::Core(vw_ai::Error::AttemptConsumed))
    ));
    assert_eq!(secrets.calls.load(Ordering::Acquire), 1);
    Ok(())
}

#[test]
fn no_usage_retains_image_but_does_not_fabricate_a_settlement() -> TestResult {
    let _guard = workers()?;
    let root = temp()?;
    let p = prepared(2, 1024 * 1024)?;
    let fixture = Server::start(response(&p, false)?, ReplyMode::Normal)?;
    let mut adapter = adapter(&fixture, Arc::new(FakeSecret::default()))?;
    let mut ledger = SqliteBudgetLedger::create(&root.path().join("ledger.sqlite"))?;
    let outcome = adapter.send_confirmed(
        &p,
        &mut ledger,
        Confirmation::explicit_send(p.review(), 20000, false)?,
        BudgetPolicy::default(),
        &NeverCancel,
    )?;
    assert_eq!(outcome.settlement, Settlement::UsageMissing);
    assert_eq!(
        ledger.entry(p.review().request_id())?.ok_or("entry")?.state,
        EntryState::Attempted
    );
    assert_eq!(fixture.finish()?.requests, 1);
    Ok(())
}

#[test]
fn wrong_confirmation_and_soft_budget_precede_any_secret_access() -> TestResult {
    let _guard = workers()?;
    let root = temp()?;
    let p = prepared(3, 1024 * 1024)?;
    let other = prepared(4, 1024 * 1024)?;
    let secrets = Arc::new(FakeSecret::default());
    let mut adapter = OpenAiAdapter {
        client: http::test_client()?,
        secrets: secrets.clone(),
        fixture_target: None,
    };
    let mut ledger = SqliteBudgetLedger::create(&root.path().join("ledger.sqlite"))?;
    assert!(matches!(
        adapter.send_confirmed(
            &p,
            &mut ledger,
            Confirmation::explicit_send(other.review(), 20000, false)?,
            BudgetPolicy::default(),
            &NeverCancel
        ),
        Err(Error::Core(vw_ai::Error::Confirmation))
    ));
    assert!(matches!(
        adapter.send_confirmed(
            &p,
            &mut ledger,
            Confirmation::explicit_send(p.review(), 20000, false)?,
            BudgetPolicy {
                daily_soft_limit_microusd: 1
            },
            &NeverCancel
        ),
        Err(Error::Core(vw_ai::Error::SoftBudget))
    ));
    assert!(ledger.entry(p.review().request_id())?.is_none());
    assert_eq!(secrets.calls.load(Ordering::Acquire), 0);
    Ok(())
}

#[test]
fn credential_failure_consumes_attempt_and_cannot_be_replayed_after_reopen() -> TestResult {
    let _guard = workers()?;
    let root = temp()?;
    let path = root.path().join("ledger.sqlite");
    let p = prepared(5, 1024 * 1024)?;
    let secrets = Arc::new(FakeSecret {
        calls: AtomicUsize::new(0),
        missing: true,
    });
    let mut adapter = OpenAiAdapter {
        client: http::test_client()?,
        secrets: secrets.clone(),
        fixture_target: None,
    };
    let mut ledger = SqliteBudgetLedger::create(&path)?;
    assert!(matches!(
        adapter.send_confirmed(
            &p,
            &mut ledger,
            Confirmation::explicit_send(p.review(), 20000, false)?,
            BudgetPolicy::default(),
            &NeverCancel
        ),
        Err(Error::Credential(CredentialFailure::Missing))
    ));
    drop(ledger);
    let mut ledger = SqliteBudgetLedger::open(&path)?;
    assert_eq!(
        ledger.entry(p.review().request_id())?.ok_or("entry")?.state,
        EntryState::Attempted
    );
    assert!(matches!(
        ledger.begin_attempt(p.review()),
        Err(vw_ai::Error::AttemptConsumed)
    ));
    assert_eq!(secrets.calls.load(Ordering::Acquire), 1);
    Ok(())
}

#[test]
fn redirects_errors_truncation_and_malformed_payload_never_retry_or_settle() -> TestResult {
    let _guard = workers()?;
    for (id, mode) in [
        (10, ReplyMode::Redirect),
        (11, ReplyMode::RateLimit),
        (12, ReplyMode::Truncated),
        (13, ReplyMode::Normal),
    ] {
        let root = temp()?;
        let p = prepared(id, 1024 * 1024)?;
        let fixture = Server::start(b"{malformed-private-payload".to_vec(), mode)?;
        let secrets = Arc::new(FakeSecret::default());
        let mut adapter = adapter(&fixture, secrets.clone())?;
        let mut ledger = SqliteBudgetLedger::create(&root.path().join("ledger.sqlite"))?;
        let result = adapter.send_confirmed(
            &p,
            &mut ledger,
            Confirmation::explicit_send(p.review(), 20000, false)?,
            BudgetPolicy::default(),
            &NeverCancel,
        );
        assert!(result.is_err());
        assert_eq!(
            ledger.entry(p.review().request_id())?.ok_or("entry")?.state,
            EntryState::Attempted
        );
        assert_eq!(fixture.finish()?.requests, 1);
        assert_eq!(secrets.calls.load(Ordering::Acquire), 1);
    }
    Ok(())
}

#[test]
fn declared_and_streamed_response_limits_leave_attempt_unresolved() -> TestResult {
    let _guard = workers()?;
    for (id, mode) in [(20, ReplyMode::Normal), (21, ReplyMode::Chunks)] {
        let root = temp()?;
        let p = prepared(id, 64)?;
        let fixture = Server::start(vec![b' '; 4096], mode)?;
        let mut adapter = adapter(&fixture, Arc::new(FakeSecret::default()))?;
        let mut ledger = SqliteBudgetLedger::create(&root.path().join("ledger.sqlite"))?;
        assert!(matches!(
            adapter.send_confirmed(
                &p,
                &mut ledger,
                Confirmation::explicit_send(p.review(), 20000, false)?,
                BudgetPolicy::default(),
                &NeverCancel
            ),
            Err(Error::ResponseLimit)
        ));
        assert_eq!(
            ledger.entry(p.review().request_id())?.ok_or("entry")?.state,
            EntryState::Attempted
        );
        assert_eq!(fixture.finish()?.requests, 1);
    }
    Ok(())
}

#[test]
fn cancellation_returns_while_blocked_owned_worker_keeps_its_slot_and_drops_late_result()
-> TestResult {
    let _guard = workers()?;
    let cancel = Arc::new(AtomicBool::new(false));
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let (dropped_tx, dropped_rx) = mpsc::sync_channel(1);
    struct Late(mpsc::SyncSender<()>);
    impl Drop for Late {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    let caller_cancel = cancel.clone();
    let slot = worker::Slot::acquire()?;
    let caller = std::thread::spawn(move || {
        worker::run(
            slot,
            Duration::from_secs(5),
            caller_cancel.as_ref(),
            move |_, _| {
                let _ = entered_tx.send(());
                release_rx
                    .recv_timeout(Duration::from_secs(3))
                    .map_err(|_| Error::Worker)?;
                Ok(Late(dropped_tx))
            },
        )
    });
    entered_rx.recv_timeout(Duration::from_secs(2))?;
    cancel.store(true, Ordering::Release);
    let began = Instant::now();
    let result = caller.join().map_err(|_| "caller panicked")?;
    assert!(matches!(result, Err(Error::Cancelled)));
    assert!(began.elapsed() < Duration::from_secs(1));
    let second = worker::Slot::acquire()?;
    assert!(matches!(worker::Slot::acquire(), Err(Error::Busy)));
    release_tx.send(())?;
    dropped_rx.recv_timeout(Duration::from_secs(2))?;
    drop(second);
    Ok(())
}

#[test]
fn deadline_and_pre_cancel_do_not_wait_for_uncooperative_calls() -> TestResult {
    let _guard = workers()?;
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let result = worker::run(
        worker::Slot::acquire()?,
        Duration::from_secs(1),
        &AtomicBool::new(true),
        move |_, _| {
            counted.fetch_add(1, Ordering::AcqRel);
            Ok(())
        },
    );
    assert_eq!(result, Err(Error::Cancelled));
    assert_eq!(calls.load(Ordering::Acquire), 0);
    let (release, blocked) = mpsc::sync_channel(1);
    let (finished, done) = mpsc::sync_channel(1);
    let start = Instant::now();
    let result = worker::run(
        worker::Slot::acquire()?,
        Duration::from_millis(60),
        &NeverCancel,
        move |_, _| {
            let _ = blocked.recv_timeout(Duration::from_secs(2));
            let _ = finished.send(());
            Ok(())
        },
    );
    assert_eq!(result, Err(Error::Deadline));
    assert!(start.elapsed() < Duration::from_secs(1));
    release.send(())?;
    done.recv_timeout(Duration::from_secs(2))?;
    Ok(())
}

#[test]
fn cancellation_interrupts_a_real_stalled_http_response_and_preserves_attempt() -> TestResult {
    let _guard = workers()?;
    let root = temp()?;
    let p = prepared(30, 1024 * 1024)?;
    let request_id = p.review().request_id().to_owned();
    let path = root.path().join("ledger.sqlite");
    let mut ledger = SqliteBudgetLedger::create(&path)?;
    let fixture = Server::start(vec![], ReplyMode::Stall)?;
    let mut adapter = adapter(&fixture, Arc::new(FakeSecret::default()))?;
    let cancel = Arc::new(AtomicBool::new(false));
    let caller_cancel = cancel.clone();
    let caller = std::thread::spawn(move || {
        let confirm = Confirmation::explicit_send(p.review(), 20000, false)?;
        adapter.send_confirmed(
            &p,
            &mut ledger,
            confirm,
            BudgetPolicy::default(),
            caller_cancel.as_ref(),
        )
    });
    fixture.wait_received()?; // Actual POST received; response intentionally absent.
    let start = Instant::now();
    cancel.store(true, Ordering::Release);
    assert!(matches!(
        caller.join().map_err(|_| "caller panicked")?,
        Err(Error::Cancelled)
    ));
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(fixture.finish()?.requests, 1);
    let ledger = SqliteBudgetLedger::open(&path)?;
    assert_eq!(
        ledger.entry(&request_id)?.ok_or("entry")?.state,
        EntryState::Attempted
    );
    Ok(())
}
