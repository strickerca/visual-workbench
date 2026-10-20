#![allow(clippy::expect_used, clippy::panic)]
use super::*;
use crate::android_tunnel::{AndroidTunnelHierarchy, AndroidTunnelTarget, contract::Captured};
use std::{
    cell::RefCell,
    sync::atomic::{AtomicBool, AtomicUsize},
    time::Instant,
};

static TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const WAIT: Duration = Duration::from_secs(4);

#[derive(Default)]
struct Gate {
    entered: AtomicBool,
    released: Mutex<bool>,
    wake: std::sync::Condvar,
    changed: Notify,
}
impl Gate {
    fn block(&self) -> Result<()> {
        self.entered.store(true, Ordering::Release);
        self.changed.notify_waiters();
        let started = Instant::now();
        let mut released = lock(&self.released);
        while !*released {
            let remaining = WAIT.checked_sub(started.elapsed()).ok_or(Error::Timeout)?;
            released = self
                .wake
                .wait_timeout(released, remaining)
                .expect("fixture gate")
                .0;
        }
        Ok(())
    }
    fn release(&self) {
        *lock(&self.released) = true;
        self.wake.notify_all();
    }
    async fn entered(&self) {
        tokio::time::timeout(WAIT, async {
            loop {
                let changed = self.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if self.entered.load(Ordering::Acquire) {
                    return;
                }
                changed.await;
            }
        })
        .await
        .expect("fixture entered deadline");
    }
}
#[derive(Default)]
struct Probe {
    open: Option<Arc<Gate>>,
    capture: Option<Arc<Gate>>,
    check: Option<Arc<Gate>>,
    close: Option<Arc<Gate>>,
    opens: AtomicUsize,
    states: AtomicUsize,
    captures: AtomicUsize,
    checks: AtomicUsize,
    closes: AtomicUsize,
    drops: AtomicUsize,
    unsafe_drops: AtomicUsize,
    signals: AtomicUsize,
    revision: AtomicU64,
    control: AtomicBool,
    fail_close: AtomicBool,
    fail_revoke: AtomicBool,
    close_on_revoke: AtomicBool,
    revoked_closed: AtomicBool,
    png: Option<Vec<u8>>,
}
impl Probe {
    fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire) + 1
    }
    fn state(&self) -> AndroidTunnelState {
        if self.revoked_closed.load(Ordering::Acquire) {
            return AndroidTunnelState {
                mode: AndroidTunnelMode::Closed,
                mirror_revision: 0,
            };
        }
        AndroidTunnelState {
            mode: if self.control.load(Ordering::Acquire) {
                AndroidTunnelMode::OwnerControl
            } else {
                AndroidTunnelMode::ViewOnly
            },
            mirror_revision: self.revision(),
        }
    }
}
struct Synthetic {
    probe: Arc<Probe>,
    retired: bool,
}
impl Provider for Synthetic {
    fn state(&mut self) -> Result<AndroidTunnelState> {
        self.probe.states.fetch_add(1, Ordering::AcqRel);
        Ok(self.probe.state())
    }
    fn begin_confirmation(&mut self) -> Result<AndroidTunnelTarget> {
        Ok(AndroidTunnelTarget {
            model: "synthetic".into(),
            transport_id: 1,
            mirror_revision: self.probe.revision(),
        })
    }
    fn confirm(&mut self, target: AndroidTunnelTarget) -> Result<AndroidTunnelState> {
        if target.model != "synthetic"
            || target.transport_id != 1
            || target.mirror_revision != self.probe.revision()
        {
            return Err(Error::Stale);
        }
        self.probe.control.store(true, Ordering::Release);
        self.probe.revision.fetch_add(1, Ordering::AcqRel);
        Ok(self.probe.state())
    }
    fn cancel_confirmation(&mut self) -> Result<()> {
        Ok(())
    }
    fn revoke_control(&mut self) -> Result<AndroidTunnelState> {
        self.probe.control.store(false, Ordering::Release);
        self.probe.revision.fetch_add(1, Ordering::AcqRel);
        if self.probe.fail_revoke.load(Ordering::Acquire) {
            return Err(Error::Platform);
        }
        if self.probe.close_on_revoke.load(Ordering::Acquire) {
            self.probe.revoked_closed.store(true, Ordering::Release);
        }
        Ok(self.probe.state())
    }
    fn capture(&mut self) -> Result<Captured> {
        self.probe.captures.fetch_add(1, Ordering::AcqRel);
        if let Some(gate) = &self.probe.capture {
            gate.block()?;
        }
        Ok(Captured {
            png: self.probe.png.clone().unwrap_or_else(|| vec![0x6b; 1024]),
            width: if self.probe.png.is_some() { 2 } else { 1 },
            height: if self.probe.png.is_some() { 2 } else { 1 },
            image_started_ms: 999,
            image_finished_ms: 1000,
            image_finished_timestamp_ns: 1_000_000,
            hierarchy: AndroidTunnelHierarchy::Missing,
            hierarchy_age_ms: None,
            hierarchy_started_ms: None,
            hierarchy_finished_ms: None,
            elements: Vec::new(),
        })
    }
    fn check_source(&mut self, _source: AndroidTunnelSource, revision: u64) -> Result<()> {
        self.probe.checks.fetch_add(1, Ordering::AcqRel);
        if let Some(gate) = &self.probe.check {
            gate.block()?;
        }
        if revision != self.probe.revision() {
            return Err(Error::Stale);
        }
        Ok(())
    }
    fn close(&mut self) -> Result<()> {
        self.probe.closes.fetch_add(1, Ordering::AcqRel);
        if let Some(gate) = &self.probe.close {
            gate.block()?;
        }
        if self.probe.fail_close.load(Ordering::Acquire) {
            return Err(Error::Platform);
        }
        self.retired = true;
        Ok(())
    }
}

#[tokio::test]
async fn exported_revoke_preserves_actual_closed_state_and_joins_its_owner() {
    let _serial = TESTS.lock().await;
    let probe = Arc::new(Probe {
        close_on_revoke: AtomicBool::new(true),
        ..Probe::default()
    });
    let budget = budget();
    let owner = start(probe.clone(), &budget);
    owner.ready().await.expect("ready");
    let session = crate::android_tunnel::api::synthetic_session(owner.clone());
    let state = session
        .revoke_control()
        .await
        .expect("successful closed revoke");
    assert_eq!(
        state,
        AndroidTunnelState {
            mode: AndroidTunnelMode::Closed,
            mirror_revision: 0
        }
    );
    assert!(Worker::current().is_none());
    assert_eq!(probe.closes.load(Ordering::Acquire), 1);
    assert!(matches!(
        owner.mirror_revision(),
        Err(Error::RetirementPending)
    ));
    drop(session);
    drop(owner);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn failed_raw_open_has_a_concrete_retained_retry_provider() {
    let _serial = TESTS.lock().await;
    let probe = Arc::new(Probe::default());
    let budget = budget();
    let can_retire = Arc::new(AtomicBool::new(false));
    let retried = Arc::new(AtomicUsize::new(0));
    let allowed = can_retire.clone();
    let attempts = retried.clone();
    let open: OpenProvider = Box::new(move |_| {
        Ok(Box::new(
            crate::android_tunnel::platform::RetiringProvider::new(Box::new(move || {
                attempts.fetch_add(1, Ordering::AcqRel);
                if allowed.load(Ordering::Acquire) {
                    Ok(())
                } else {
                    Err(Error::RetirementPending)
                }
            })),
        ))
    });
    let owner = Worker::start(open, stop(&probe), budget.clone()).expect("retained actor");
    assert!(matches!(owner.ready().await, Err(Error::RetirementPending)));
    assert!(matches!(owner.close().await, Err(Error::RetirementPending)));
    assert!(Worker::current().is_some());
    assert!(budget.used_bytes() > OUTPUT_BYTES);
    can_retire.store(true, Ordering::Release);
    Worker::retry_retirement()
        .await
        .expect("actual callback retry");
    assert!(retried.load(Ordering::Acquire) >= 2);
    assert!(Worker::current().is_none());
    drop(owner);
    assert_eq!(budget.used_bytes(), 0);
}
impl Drop for Synthetic {
    fn drop(&mut self) {
        self.probe.drops.fetch_add(1, Ordering::AcqRel);
        if !self.retired {
            self.probe.unsafe_drops.fetch_add(1, Ordering::AcqRel);
        }
    }
}
fn budget() -> SessionBudget {
    SessionBudget::new(vw_stream::DESKTOP_SESSION_BYTES)
        .expect("test process pool")
        .child(256 * 1024 * 1024)
        .expect("injected endpoint pool")
}
fn stop(probe: &Arc<Probe>) -> Arc<Stop> {
    let probe = probe.clone();
    Stop::new(Arc::new(move || {
        probe.signals.fetch_add(1, Ordering::AcqRel);
    }))
}
fn opener(probe: Arc<Probe>) -> OpenProvider {
    Box::new(move |_budget| {
        probe.opens.fetch_add(1, Ordering::AcqRel);
        if let Some(gate) = &probe.open {
            gate.block()?;
        }
        Ok(Box::new(Synthetic {
            probe,
            retired: false,
        }))
    })
}
fn start(probe: Arc<Probe>, budget: &SessionBudget) -> Arc<Worker> {
    Worker::start(opener(probe.clone()), stop(&probe), budget.clone()).expect("tunnel worker")
}
async fn close(owner: &Worker) {
    tokio::time::timeout(WAIT, owner.close())
        .await
        .expect("close deadline")
        .expect("actual retirement");
}
fn source() -> AndroidTunnelSource {
    AndroidTunnelSource::Window { window: 7 }
}

#[tokio::test]
async fn admission_and_registry_precede_provider_open_and_healthy_owner_cannot_be_replaced() {
    let _serial = TESTS.lock().await;
    let epoch = registry_epoch();
    let probe = Arc::new(Probe::default());
    let small =
        SessionBudget::new(OUTPUT_BYTES + ACTOR_BYTES + OWNER_BYTES - 1).expect("small budget");
    assert!(matches!(
        Worker::start(opener(probe.clone()), stop(&probe), small.clone()),
        Err(Error::Limit)
    ));
    assert_eq!(small.used_bytes(), 0);
    assert_eq!(probe.opens.load(Ordering::Acquire), 0);
    assert_eq!(registry_epoch(), epoch);
    let budget = budget();
    let owner = start(probe.clone(), &budget);
    assert!(Worker::current().is_some_and(|current| Arc::ptr_eq(&current, &owner)));
    assert_eq!(registry_epoch(), epoch + 1);
    owner.ready().await.expect("actual provider ready");
    assert_eq!(owner.mirror_revision().expect("actual cached revision"), 1);
    assert_eq!(
        budget.used_bytes(),
        OUTPUT_BYTES + ACTOR_BYTES + OWNER_BYTES
    );
    assert!(matches!(
        Worker::retry_retirement().await,
        Err(Error::RetirementPending)
    ));
    let other = Arc::new(Probe::default());
    assert!(matches!(
        Worker::start(opener(other.clone()), stop(&other), budget.clone()),
        Err(Error::RetirementPending)
    ));
    assert_eq!(other.opens.load(Ordering::Acquire), 0);
    close(&owner).await;
    assert!(Worker::current().is_none());
    assert_eq!(registry_epoch(), epoch + 2);
    drop(owner);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn late_cancelled_open_adopts_exact_provider_and_waits_for_actual_close() {
    let _serial = TESTS.lock().await;
    let opening = Arc::new(Gate::default());
    let closing = Arc::new(Gate::default());
    let probe = Arc::new(Probe {
        open: Some(opening.clone()),
        close: Some(closing.clone()),
        ..Probe::default()
    });
    let budget = budget();
    let owner = start(probe.clone(), &budget);
    let epoch = registry_epoch();
    let waiting_owner = owner.clone();
    let waiting = tokio::spawn(async move { waiting_owner.ready().await });
    opening.entered().await;
    tokio::task::yield_now().await;
    waiting.abort();
    assert!(waiting.await.expect_err("aborted ready").is_cancelled());
    assert!(owner.control.stop.check().is_err());
    let closer = owner.clone();
    let close_task = tokio::spawn(async move { closer.close().await });
    opening.release();
    closing.entered().await;
    assert!(!close_task.is_finished());
    assert_eq!(probe.states.load(Ordering::Acquire), 0);
    assert_eq!(probe.drops.load(Ordering::Acquire), 0);
    assert_eq!(registry_epoch(), epoch);
    assert_eq!(
        budget.used_bytes(),
        OUTPUT_BYTES + ACTOR_BYTES + OWNER_BYTES
    );
    closing.release();
    close_task
        .await
        .expect("close task")
        .expect("late provider retired");
    assert_eq!(probe.drops.load(Ordering::Acquire), 1);
    assert_eq!(probe.unsafe_drops.load(Ordering::Acquire), 0);
    drop(owner);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn escaped_response_keeps_operation_and_actual_envelope_after_provider_thread_join() {
    let _serial = TESTS.lock().await;
    let probe = Arc::new(Probe::default());
    let budget = budget();
    let owner = start(probe.clone(), &budget);
    owner.ready().await.expect("ready");
    let response = owner.exchange(Work::Capture).await.expect("captured owner");
    assert!(matches!(
        owner.exchange(Work::State).await,
        Err(Error::RetirementPending)
    ));
    assert!(matches!(
        owner.check_source(source(), 1),
        Err(Error::RetirementPending)
    ));
    assert_eq!(probe.checks.load(Ordering::Acquire), 0);
    let escaped = Arc::new(response);
    close(&owner).await;
    assert!(Worker::current().is_none());
    assert_eq!(budget.used_bytes(), OUTPUT_BYTES + OWNER_BYTES);
    drop(owner);
    assert_eq!(budget.used_bytes(), OUTPUT_BYTES);
    let Response::Captured(captured) = &escaped.value else {
        panic!("capture response")
    };
    assert_eq!(captured.png, vec![0x6b; 1024]);
    drop(escaped);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn explicit_response_close_releases_slot_and_abandoned_response_seals_successor() {
    let _serial = TESTS.lock().await;
    let probe = Arc::new(Probe::default());
    let budget = budget();
    let owner = start(probe.clone(), &budget);
    owner.ready().await.expect("ready");
    owner
        .exchange(Work::Capture)
        .await
        .expect("first capture")
        .close();
    let response = owner
        .exchange(Work::State)
        .await
        .expect("state after explicit receipt");
    drop(response);
    assert!(matches!(
        owner.exchange(Work::Capture).await,
        Err(Error::Cancelled)
    ));
    assert_eq!(probe.captures.load(Ordering::Acquire), 1);
    close(&owner).await;
    drop(owner);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn cancelled_capture_retains_real_work_until_late_output_and_close_settle() {
    let _serial = TESTS.lock().await;
    let gate = Arc::new(Gate::default());
    let probe = Arc::new(Probe {
        capture: Some(gate.clone()),
        ..Probe::default()
    });
    let budget = budget();
    let owner = start(probe.clone(), &budget);
    owner.ready().await.expect("ready");
    let capturing = owner.clone();
    let task = tokio::spawn(async move { capturing.exchange(Work::Capture).await });
    gate.entered().await;
    assert!(matches!(
        owner.check_source(source(), 1),
        Err(Error::RetirementPending)
    ));
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    assert!(owner.control.stop.check().is_err());
    let closing = owner.clone();
    let closed = tokio::spawn(async move { closing.close().await });
    tokio::task::yield_now().await;
    assert!(!closed.is_finished());
    assert_eq!(probe.drops.load(Ordering::Acquire), 0);
    assert_eq!(
        budget.used_bytes(),
        OUTPUT_BYTES + ACTOR_BYTES + OWNER_BYTES
    );
    gate.release();
    closed
        .await
        .expect("close task")
        .expect("late capture retired");
    assert_eq!(probe.unsafe_drops.load(Ordering::Acquire), 0);
    drop(owner);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn failed_close_retains_exact_provider_credit_and_registry_for_explicit_retry() {
    let _serial = TESTS.lock().await;
    let probe = Arc::new(Probe {
        fail_close: AtomicBool::new(true),
        ..Probe::default()
    });
    let budget = budget();
    let owner = start(probe.clone(), &budget);
    owner.ready().await.expect("ready");
    let epoch = registry_epoch();
    assert!(matches!(owner.close().await, Err(Error::RetirementPending)));
    assert_eq!(probe.closes.load(Ordering::Acquire), 1);
    assert_eq!(probe.drops.load(Ordering::Acquire), 0);
    assert_eq!(
        budget.used_bytes(),
        OUTPUT_BYTES + ACTOR_BYTES + OWNER_BYTES
    );
    assert_eq!(registry_epoch(), epoch);
    drop(owner);
    assert!(Worker::current().is_some());
    probe.fail_close.store(false, Ordering::Release);
    Worker::retry_retirement()
        .await
        .expect("retained exact-owner retry");
    assert_eq!(probe.closes.load(Ordering::Acquire), 2);
    assert_eq!(probe.drops.load(Ordering::Acquire), 1);
    assert_eq!(probe.unsafe_drops.load(Ordering::Acquire), 0);
    assert!(Worker::current().is_none());
    assert_eq!(registry_epoch(), epoch + 1);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn concurrent_close_observes_same_actual_provider_retirement() {
    let _serial = TESTS.lock().await;
    let gate = Arc::new(Gate::default());
    let probe = Arc::new(Probe {
        close: Some(gate.clone()),
        ..Probe::default()
    });
    let budget = budget();
    let owner = start(probe.clone(), &budget);
    owner.ready().await.expect("ready");
    let first = owner.clone();
    let first = tokio::spawn(async move { first.close().await });
    gate.entered().await;
    let second = owner.clone();
    let second = tokio::spawn(async move { second.close().await });
    tokio::task::yield_now().await;
    assert!(!first.is_finished());
    assert!(!second.is_finished());
    assert_eq!(probe.closes.load(Ordering::Acquire), 1);
    gate.release();
    first.await.expect("first task").expect("first receipt");
    second.await.expect("second task").expect("same receipt");
    assert_eq!(probe.closes.load(Ordering::Acquire), 1);
    assert!(Worker::current().is_none());
    drop(owner);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn bounded_source_check_timeout_cancels_without_admitting_another_pending_check() {
    let _serial = TESTS.lock().await;
    let gate = Arc::new(Gate::default());
    let probe = Arc::new(Probe {
        check: Some(gate.clone()),
        ..Probe::default()
    });
    let budget = budget();
    let owner = start(probe.clone(), &budget);
    owner.ready().await.expect("ready");
    let started = Instant::now();
    assert!(matches!(
        owner.check_source(source(), 1),
        Err(Error::RetirementPending)
    ));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(probe.checks.load(Ordering::Acquire), 1);
    assert!(owner.control.stop.check().is_err());
    assert!(matches!(
        owner.check_source(source(), 1),
        Err(Error::RetirementPending)
    ));
    assert_eq!(probe.checks.load(Ordering::Acquire), 1);
    assert!(probe.signals.load(Ordering::Acquire) > 0);
    gate.release();
    close(&owner).await;
    drop(owner);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn cached_revision_comes_from_actual_state_and_ambiguous_confirmation_stays_fenced() {
    let _serial = TESTS.lock().await;
    let probe = Arc::new(Probe::default());
    let budget = budget();
    let owner = start(probe.clone(), &budget);
    owner.ready().await.expect("ready");
    assert_eq!(probe.states.load(Ordering::Acquire), 1);
    owner
        .check_source(source(), 1)
        .expect("actual bounded source check");
    let target = owner
        .exchange(Work::BeginConfirmation)
        .await
        .expect("confirmation ticket");
    let Response::Target(ticket) = &target.value else {
        panic!("target response")
    };
    let ticket = ticket.clone();
    target.close();
    assert!(matches!(
        owner.mirror_revision(),
        Err(Error::RetirementPending)
    ));
    let confirmed = owner
        .exchange(Work::Confirm(ticket))
        .await
        .expect("confirmed");
    assert!(
        matches!(&confirmed.value, Response::State(state) if state.mode == AndroidTunnelMode::OwnerControl && state.mirror_revision == 2)
    );
    confirmed.close();
    assert_eq!(
        owner.mirror_revision().expect("actual post-confirm state"),
        2
    );
    assert_eq!(probe.states.load(Ordering::Acquire), 2);
    assert!(matches!(owner.check_source(source(), 1), Err(Error::Stale)));
    owner.check_source(source(), 2).expect("new exact revision");
    owner
        .exchange(Work::RevokeControl)
        .await
        .expect("revoked")
        .close();
    assert_eq!(
        owner.mirror_revision().expect("actual post-revoke state"),
        3
    );
    assert_eq!(probe.states.load(Ordering::Acquire), 3);
    owner
        .exchange(Work::BeginConfirmation)
        .await
        .expect("new ticket")
        .close();
    owner
        .exchange(Work::CancelConfirmation)
        .await
        .expect("cancel confirmation")
        .close();
    assert_eq!(
        owner.mirror_revision().expect("actual state after cancel"),
        3
    );
    probe.fail_revoke.store(true, Ordering::Release);
    assert!(matches!(
        owner.exchange(Work::RevokeControl).await,
        Err(Error::Platform)
    ));
    assert!(matches!(
        owner.mirror_revision(),
        Err(Error::RetirementPending)
    ));
    assert!(matches!(
        owner.exchange(Work::Capture).await,
        Err(Error::Cancelled)
    ));
    close(&owner).await;
    drop(owner);
    assert_eq!(budget.used_bytes(), 0);
}

struct ThreadExit(Arc<Gate>);
impl Drop for ThreadExit {
    fn drop(&mut self) {
        let _ = self.0.block();
    }
}
thread_local! { static EXIT: RefCell<Option<ThreadExit>> = const { RefCell::new(None) }; }
#[tokio::test]
async fn registry_epoch_and_credit_wait_for_actual_thread_exit_after_provider_close() {
    let _serial = TESTS.lock().await;
    let probe = Arc::new(Probe::default());
    let budget = budget();
    let owner = start(probe.clone(), &budget);
    owner.ready().await.expect("ready");
    let gate = Arc::new(Gate::default());
    let exit = gate.clone();
    owner
        .actor
        .call(move |_| {
            EXIT.with(|slot| *slot.borrow_mut() = Some(ThreadExit(exit)));
            Ok(())
        })
        .await
        .expect("actual TLS owner");
    let epoch = registry_epoch();
    let closing = owner.clone();
    let (send, receive) = mpsc::channel();
    let thread = thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("fixture runtime");
        let _ = send.send(runtime.block_on(closing.close()));
    });
    gate.entered().await;
    assert!(receive.try_recv().is_err());
    assert_eq!(probe.drops.load(Ordering::Acquire), 1);
    assert!(Worker::current().is_some());
    assert_eq!(registry_epoch(), epoch);
    assert_eq!(
        budget.used_bytes(),
        OUTPUT_BYTES + ACTOR_BYTES + OWNER_BYTES
    );
    gate.release();
    thread.join().expect("closing thread");
    receive
        .recv_timeout(WAIT)
        .expect("close receipt")
        .expect("actual thread joined");
    assert!(Worker::current().is_none());
    assert_eq!(registry_epoch(), epoch + 1);
    drop(owner);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn exported_guard_detects_transient_registry_owner_and_confirmed_revision_change() {
    let _serial = TESTS.lock().await;
    let guard = crate::android_tunnel::begin_android_tunnel_capture_guard(source())
        .expect("guard without tunnel owner");
    guard.check().expect("initial empty registry");
    let probe = Arc::new(Probe::default());
    let budget = budget();
    let owner = start(probe, &budget);
    owner.ready().await.expect("ready");
    close(&owner).await;
    drop(owner);
    assert!(Worker::current().is_none());
    assert!(matches!(guard.check(), Err(Error::Stale)));
    guard.close_guard();

    let probe = Arc::new(Probe::default());
    let owner = start(probe.clone(), &budget);
    owner.ready().await.expect("next actual owner ready");
    let guard = crate::android_tunnel::begin_android_tunnel_capture_guard(source())
        .expect("exact active tunnel guard");
    assert_eq!(probe.checks.load(Ordering::Acquire), 1);
    let target = owner
        .exchange(Work::BeginConfirmation)
        .await
        .expect("target");
    let Response::Target(ticket) = &target.value else {
        panic!("target response")
    };
    let ticket = ticket.clone();
    target.close();
    owner
        .exchange(Work::Confirm(ticket))
        .await
        .expect("confirmed control")
        .close();
    assert_eq!(owner.mirror_revision().expect("actual new revision"), 2);
    assert!(matches!(guard.check(), Err(Error::Stale)));
    guard.close_guard();
    close(&owner).await;
    drop(owner);
    assert_eq!(budget.used_bytes(), 0);
}

fn real_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder.write_header().expect("fixture PNG header");
        writer
            .write_image_data(&[
                40, 80, 120, 255, 80, 120, 160, 255, 120, 160, 200, 255, 160, 200, 240, 255,
            ])
            .expect("fixture opaque pixels");
    }
    bytes
}
#[tokio::test]
async fn native_capture_import_uses_owned_png_and_releases_operation_only_after_capture_close() {
    let _serial = TESTS.lock().await;
    let png = real_png();
    let probe = Arc::new(Probe {
        png: Some(png.clone()),
        ..Probe::default()
    });
    let budget = crate::session_resources::shared_budget()
        .expect("common parent")
        .child(256 * 1024 * 1024)
        .expect("actual tunnel endpoint");
    let owner = start(probe.clone(), &budget);
    owner.ready().await.expect("ready");
    let session = crate::android_tunnel::api::synthetic_session(owner.clone());
    let capture = session.capture().await.expect("actual exported capture");
    let info = capture.info().expect("captured info");
    assert_eq!((info.width, info.height), (2, 2));
    assert_eq!(info.frame_id, 1);
    assert_eq!(info.image_finished_ms, 1000);
    assert_eq!(info.image_finished_timestamp_ns, 1_000_000);
    assert_eq!(info.encoded_bytes, png.len() as u64);
    assert_eq!(info.asset_id, vw_model::AssetId::hash(&png).to_string());
    assert!(capture.elements().expect("hierarchy absent").is_empty());
    assert!(matches!(
        session.capture().await,
        Err(Error::RetirementPending)
    ));

    let directory = tempfile::tempdir().expect("test-owned project directory");
    let id = |value| {
        vw_model::Id::from_parts(1_700_000_000_000, [value; 10])
            .expect("fixture id")
            .to_string()
    };
    let project_id = id(21);
    let document_id = id(22);
    let options = crate::CreateFileImageProject {
        path: directory
            .path()
            .join("captured-project")
            .to_string_lossy()
            .into_owned(),
        project_id: project_id.clone(),
        document_id: document_id.clone(),
        layer_id: id(23),
        device_id: vw_model::DeviceId::from_bytes([24; 16]).to_string(),
        title: "Synthetic Android capture".into(),
        source_path: String::new(),
        work_directory: directory.path().to_string_lossy().into_owned(),
        now_ms: 1000,
        memory_budget_bytes: 64 * 1024 * 1024,
        max_encoded_bytes: 1024 * 1024,
        max_scratch_bytes: 4 * 1024 * 1024,
    };
    // The sRGB PNG carries a real ICC profile. The existing importer's72MiB
    // profile envelope must refuse64MiB even for a tiny image, without losing
    // the original capture or publishing a partial canonical project.
    assert!(matches!(
        capture
            .create_project(options.clone(), Arc::new(crate::Cancellation::default()))
            .await,
        Err(crate::CoreError::Backpressure)
    ));
    assert!(!directory.path().join("captured-project").exists());
    assert_eq!(
        std::fs::read_dir(directory.path())
            .expect("owned work")
            .count(),
        0
    );
    let options = crate::CreateFileImageProject {
        memory_budget_bytes: 128 * 1024 * 1024,
        ..options
    };
    let mut importing =
        Box::pin(capture.create_project(options, Arc::new(crate::Cancellation::default())));
    use std::{
        future::{Future, poll_fn},
        task::Poll,
    };
    let first = poll_fn(|cx| Poll::Ready(importing.as_mut().poll(cx))).await;
    assert!(matches!(
        session.capture().await,
        Err(Error::RetirementPending)
    ));
    let project = match first {
        Poll::Ready(result) => result.expect("actual native capture import"),
        Poll::Pending => importing
            .as_mut()
            .await
            .expect("actual native capture import"),
    };
    drop(importing);
    let created = project.info().await.expect("actual project actor");
    assert_eq!(created.project_id, project_id);
    assert_eq!(created.document_ids, vec![document_id]);
    assert!(directory.path().join("captured-project").exists());
    assert!(matches!(
        session.capture().await,
        Err(Error::RetirementPending)
    ));
    capture
        .close_capture()
        .expect("explicit capture retirement");
    assert!(matches!(capture.info(), Err(Error::Closed)));
    let next = session
        .capture()
        .await
        .expect("next actual capture after retirement");
    assert_eq!(next.info().expect("next info").frame_id, 2);
    next.close_capture().expect("next capture retired");
    project.close().await.expect("project actor closed");
    session
        .close_session()
        .await
        .expect("actual tunnel provider/thread joined");
    drop(session);
    drop(owner);
    assert!(Worker::current().is_none());
    assert_eq!(probe.unsafe_drops.load(Ordering::Acquire), 0);
    assert_eq!(budget.used_bytes(), 0);
}
