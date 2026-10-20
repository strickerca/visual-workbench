#![allow(clippy::expect_used, clippy::panic)]
use super::*;
use std::{
    io::Cursor,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Instant,
};
use vw_capture::{
    Rect,
    picker::{MonitorTarget, PickerSource},
    stream_wire::{self, Header},
};

// The shared fixture gate also serializes native pump tests against this owner.
const WAIT: Duration = Duration::from_secs(3);

#[derive(Default)]
struct Gate {
    entered: AtomicBool,
    released: Mutex<bool>,
    wake: Condvar,
    changed: Notify,
}
impl Gate {
    fn block(&self) -> Result<()> {
        self.entered.store(true, Ordering::Release);
        self.changed.notify_waiters();
        let start = Instant::now();
        let mut released = lock(&self.released);
        while !*released {
            let remaining = WAIT.checked_sub(start.elapsed()).ok_or(Error::Timeout)?;
            released = self
                .wake
                .wait_timeout(released, remaining)
                .expect("fixture gate lock")
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
    open_gate: Option<Arc<Gate>>,
    io_gate: Option<Arc<Gate>>,
    close_gate: Option<Arc<Gate>>,
    opens: AtomicUsize,
    exchanges: AtomicUsize,
    closes: AtomicUsize,
    drops: AtomicUsize,
    unsafe_drops: AtomicUsize,
    fail_close: AtomicBool,
    wrong_epoch: AtomicBool,
    cancel_seen: AtomicBool,
    payload_bytes: usize,
    commands: Mutex<Vec<(u64, (u64, u64))>>,
}
struct Synthetic {
    probe: Arc<Probe>,
    budget: SessionBudget,
    retired: bool,
}
impl Provider for Synthetic {
    fn exchange(&mut self, command: Command, cancel: &Cancellation) -> Result<Packet> {
        self.probe.exchanges.fetch_add(1, Ordering::AcqRel);
        let sequence = command.sequence().ok_or(Error::Invalid)?;
        let (epoch, generation) = command.binding().ok_or(Error::Invalid)?;
        lock(&self.probe.commands).push((sequence, (epoch, generation)));
        if let Some(gate) = &self.probe.io_gate {
            gate.block()?;
        }
        self.probe
            .cancel_seen
            .store(cancel.check().is_err(), Ordering::Release);
        // Intentionally produce a late charged reply even after cancellation.
        let epoch = epoch + u64::from(self.probe.wrong_epoch.load(Ordering::Acquire));
        let len = self.probe.payload_bytes;
        let header = if len == 0 {
            Header::Idle {
                sequence,
                connection_epoch: epoch,
                source_generation: generation,
                retained_frames: 0,
                retained_bytes: 0,
            }
        } else {
            Header::Tiles {
                sequence,
                connection_epoch: epoch,
                source_generation: generation,
                frame_id: sequence,
                timestamp_ns: sequence + 1,
                captured_at_ms: 1000,
                keyframe: true,
                payload_bytes: len as u32,
            }
        };
        let mut bytes = Vec::new();
        let fixture_cancel = Cancellation::default();
        stream_wire::write_packet(
            &mut bytes,
            &header,
            &vec![7; len],
            &self.budget,
            &fixture_cancel,
        )?;
        stream_wire::read_packet(&mut Cursor::new(bytes), &self.budget, &fixture_cancel)
    }
    fn close(&mut self) -> Result<()> {
        self.probe.closes.fetch_add(1, Ordering::AcqRel);
        if let Some(gate) = &self.probe.close_gate {
            gate.block()?;
        }
        if self.probe.fail_close.load(Ordering::Acquire) {
            return Err(Error::Platform);
        }
        self.retired = true;
        Ok(())
    }
}
impl Drop for Synthetic {
    fn drop(&mut self) {
        self.probe.drops.fetch_add(1, Ordering::AcqRel);
        if !self.retired {
            self.probe.unsafe_drops.fetch_add(1, Ordering::AcqRel);
        }
    }
}
fn request() -> StreamRequest {
    StreamRequest {
        explicit_owner_action: true,
        owner_process_id: 1,
        capture_session_id: "019a5d54-0000-7000-8000-000000000001".into(),
        connection_epoch: 17,
        source_generation: 29,
        source: PickerSource::Monitor {
            target: MonitorTarget {
                monitor: 1,
                device: "synthetic-monitor".into(),
                physical: Rect {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 2,
                },
                dpi: 96,
                rotation: 0,
                primary: true,
                observed_ns: 1,
            },
        },
        geometry_revision: 1,
        target_fps: 15,
        endpoint_bytes: 64 * 1024 * 1024,
        ring_bytes: 1024 * 1024,
    }
}
fn pool() -> SessionBudget {
    SessionBudget::new(128 * 1024 * 1024)
        .expect("process pool")
        .child(64 * 1024 * 1024)
        .expect("endpoint pool")
}
fn opener(probe: Arc<Probe>) -> OpenProvider {
    Box::new(move |_, budget, _cancel| {
        probe.opens.fetch_add(1, Ordering::AcqRel);
        if let Some(gate) = &probe.open_gate {
            gate.block()?;
        }
        Ok(Box::new(Synthetic {
            probe,
            budget: budget.clone(),
            retired: false,
        }))
    })
}
fn start(probe: Arc<Probe>, budget: &SessionBudget, current: CurrentCheck) -> Arc<Worker> {
    Worker::start(request(), budget.clone(), current, opener(probe)).expect("worker")
}
fn current() -> CurrentCheck {
    Arc::new(|| Ok(()))
}
fn poll() -> Work {
    Work::Poll {
        feedback: None,
        repair: false,
    }
}
async fn close(worker: &Worker) {
    tokio::time::timeout(WAIT, worker.close())
        .await
        .expect("close deadline")
        .expect("closed");
}
async fn until(predicate: impl Fn() -> bool) {
    tokio::time::timeout(WAIT, async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("fixture condition deadline");
}

#[tokio::test]
async fn admission_precedes_open_and_uses_exact_injected_pool() {
    let _serial = TESTS.lock().await;
    let budget = pool();
    let probe = Arc::new(Probe::default());
    let tiny = SessionBudget::new(OWNER_BYTES - 1).expect("small budget");
    assert!(matches!(
        Worker::start(request(), tiny.clone(), current(), opener(probe.clone())),
        Err(Error::Limit)
    ));
    assert_eq!(probe.opens.load(Ordering::Acquire), 0);
    assert_eq!(tiny.used_bytes(), 0);
    let worker = start(probe.clone(), &budget, current());
    worker.ready().await.expect("ready");
    assert!(worker.pool().same_pool(&budget));
    assert_eq!(budget.used_bytes(), OWNER_BYTES);
    let forbidden = Arc::new(Probe::default());
    assert!(matches!(
        Worker::start(
            request(),
            budget.clone(),
            current(),
            opener(forbidden.clone())
        ),
        Err(Error::Busy)
    ));
    assert_eq!(forbidden.opens.load(Ordering::Acquire), 0);
    close(&worker).await;
    assert!(lock(&worker.control.state).joined);
    assert_eq!(probe.unsafe_drops.load(Ordering::Acquire), 0);
    drop(worker);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn cancelled_late_open_is_owned_until_actual_retirement_and_join() {
    let _serial = TESTS.lock().await;
    let open_gate = Arc::new(Gate::default());
    let close_gate = Arc::new(Gate::default());
    let probe = Arc::new(Probe {
        open_gate: Some(open_gate.clone()),
        close_gate: Some(close_gate.clone()),
        ..Probe::default()
    });
    let budget = pool();
    let worker = start(probe.clone(), &budget, current());
    open_gate.entered().await;
    worker.cancel();
    let closing_owner = worker.clone();
    let closing = tokio::spawn(async move { closing_owner.close().await });
    open_gate.release();
    close_gate.entered().await;
    assert!(!closing.is_finished());
    assert_eq!(budget.used_bytes(), OWNER_BYTES);
    assert_eq!(probe.drops.load(Ordering::Acquire), 0);
    assert!(matches!(worker.ready().await, Err(Error::Cancelled)));
    close_gate.release();
    closing
        .await
        .expect("close task")
        .expect("late owner retired");
    assert!(lock(&worker.control.state).joined);
    assert_eq!(probe.drops.load(Ordering::Acquire), 1);
    assert_eq!(probe.unsafe_drops.load(Ordering::Acquire), 0);
    drop(worker);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn abandoned_ready_future_cancels_the_independent_open_token() {
    let _serial = TESTS.lock().await;
    let gate = Arc::new(Gate::default());
    let probe = Arc::new(Probe {
        open_gate: Some(gate.clone()),
        ..Probe::default()
    });
    let budget = pool();
    let worker = start(probe.clone(), &budget, current());
    let waiting_owner = worker.clone();
    let waiting = tokio::spawn(async move { waiting_owner.ready().await });
    gate.entered().await;
    tokio::task::yield_now().await;
    waiting.abort();
    assert!(waiting.await.expect_err("aborted ready").is_cancelled());
    assert!(worker.control.cancel.check().is_err());
    gate.release();
    close(&worker).await;
    assert_eq!(probe.drops.load(Ordering::Acquire), 1);
    drop(worker);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn one_inflight_exchange_preserves_monotonic_sequence_and_binding() {
    let _serial = TESTS.lock().await;
    let gate = Arc::new(Gate::default());
    let probe = Arc::new(Probe {
        io_gate: Some(gate.clone()),
        ..Probe::default()
    });
    let budget = pool();
    let worker = start(probe.clone(), &budget, current());
    worker.ready().await.expect("ready");
    let first_owner = worker.clone();
    let first = tokio::spawn(async move { first_owner.exchange(poll()).await });
    gate.entered().await;
    assert!(matches!(
        worker.exchange(Work::InspectSource).await,
        Err(Error::Busy)
    ));
    assert_eq!(probe.exchanges.load(Ordering::Acquire), 1);
    gate.release();
    drop(first.await.expect("exchange task").expect("first packet"));
    drop(
        worker
            .exchange(Work::InspectSource)
            .await
            .expect("heartbeat"),
    );
    assert_eq!(*lock(&probe.commands), vec![(1, (17, 29)), (2, (17, 29))]);
    close(&worker).await;
    drop(worker);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn stale_binding_after_io_is_never_published() {
    let _serial = TESTS.lock().await;
    let probe = Arc::new(Probe {
        wrong_epoch: AtomicBool::new(true),
        ..Probe::default()
    });
    let budget = pool();
    let worker = start(probe.clone(), &budget, current());
    worker.ready().await.expect("ready");
    assert!(matches!(worker.exchange(poll()).await, Err(Error::Stale)));
    close(&worker).await;
    assert_eq!(probe.drops.load(Ordering::Acquire), 1);
    drop(worker);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn revoked_current_after_io_refuses_late_packet() {
    let _serial = TESTS.lock().await;
    let gate = Arc::new(Gate::default());
    let revoked = Arc::new(AtomicBool::new(false));
    let lease = revoked.clone();
    let current: CurrentCheck = Arc::new(move || {
        if lease.load(Ordering::Acquire) {
            Err(Error::Stale)
        } else {
            Ok(())
        }
    });
    let probe = Arc::new(Probe {
        io_gate: Some(gate.clone()),
        payload_bytes: 4096,
        ..Probe::default()
    });
    let budget = pool();
    let worker = start(probe.clone(), &budget, current);
    worker.ready().await.expect("ready");
    let exchange_owner = worker.clone();
    let exchange = tokio::spawn(async move { exchange_owner.exchange(poll()).await });
    gate.entered().await;
    revoked.store(true, Ordering::Release);
    gate.release();
    assert!(matches!(
        exchange.await.expect("exchange task"),
        Err(Error::Stale)
    ));
    close(&worker).await;
    drop(worker);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn cancel_before_publication_discards_actual_charged_packet() {
    let _serial = TESTS.lock().await;
    let gate = Arc::new(Gate::default());
    let probe = Arc::new(Probe {
        io_gate: Some(gate.clone()),
        payload_bytes: 4096,
        ..Probe::default()
    });
    let budget = pool();
    let worker = start(probe.clone(), &budget, current());
    worker.ready().await.expect("ready");
    let exchange_owner = worker.clone();
    let exchange = tokio::spawn(async move { exchange_owner.exchange(poll()).await });
    gate.entered().await;
    worker.cancel();
    gate.release();
    assert!(matches!(
        exchange.await.expect("exchange task"),
        Err(Error::Cancelled)
    ));
    assert!(probe.cancel_seen.load(Ordering::Acquire));
    close(&worker).await;
    drop(worker);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn abandoned_exchange_waits_for_real_late_work_before_close_completes() {
    let _serial = TESTS.lock().await;
    let gate = Arc::new(Gate::default());
    let probe = Arc::new(Probe {
        io_gate: Some(gate.clone()),
        payload_bytes: 4096,
        ..Probe::default()
    });
    let budget = pool();
    let worker = start(probe.clone(), &budget, current());
    worker.ready().await.expect("ready");
    let exchange_owner = worker.clone();
    let exchange = tokio::spawn(async move { exchange_owner.exchange(poll()).await });
    gate.entered().await;
    exchange.abort();
    assert!(matches!(exchange.await, Err(error) if error.is_cancelled()));
    let close_owner = worker.clone();
    let closing = tokio::spawn(async move { close_owner.close().await });
    tokio::task::yield_now().await;
    assert!(!closing.is_finished());
    assert!(lock(&worker.control.state).busy);
    assert_eq!(budget.used_bytes(), OWNER_BYTES + SLOT_BYTES);
    gate.release();
    closing
        .await
        .expect("close task")
        .expect("close after actual IO");
    assert!(!lock(&worker.control.state).busy);
    assert!(probe.cancel_seen.load(Ordering::Acquire));
    assert_eq!(probe.unsafe_drops.load(Ordering::Acquire), 0);
    drop(worker);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn failed_retirement_retains_provider_credit_and_slot_for_exact_retry() {
    let _serial = TESTS.lock().await;
    let probe = Arc::new(Probe {
        fail_close: AtomicBool::new(true),
        ..Probe::default()
    });
    let budget = pool();
    let worker = start(probe.clone(), &budget, current());
    worker.ready().await.expect("ready");
    assert!(matches!(worker.close().await, Err(Error::Platform)));
    assert_eq!(probe.closes.load(Ordering::Acquire), 1);
    assert_eq!(probe.drops.load(Ordering::Acquire), 0);
    assert_eq!(budget.used_bytes(), OWNER_BYTES);
    assert!(!lock(&worker.control.state).joined);
    let successor = Arc::new(Probe::default());
    assert!(matches!(
        Worker::start(
            request(),
            budget.clone(),
            current(),
            opener(successor.clone())
        ),
        Err(Error::Busy)
    ));
    assert_eq!(successor.opens.load(Ordering::Acquire), 0);
    probe.fail_close.store(false, Ordering::Release);
    close(&worker).await;
    assert_eq!(probe.closes.load(Ordering::Acquire), 2);
    assert_eq!(probe.drops.load(Ordering::Acquire), 1);
    assert_eq!(probe.unsafe_drops.load(Ordering::Acquire), 0);
    drop(worker);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn concurrent_close_waits_for_the_same_provider_retirement_and_thread_join() {
    let _serial = TESTS.lock().await;
    let gate = Arc::new(Gate::default());
    let probe = Arc::new(Probe {
        close_gate: Some(gate.clone()),
        ..Probe::default()
    });
    let budget = pool();
    let worker = start(probe.clone(), &budget, current());
    worker.ready().await.expect("ready");
    let first_owner = worker.clone();
    let first = tokio::spawn(async move { first_owner.close().await });
    gate.entered().await;
    let second_owner = worker.clone();
    let second = tokio::spawn(async move { second_owner.close().await });
    tokio::task::yield_now().await;
    assert!(!first.is_finished());
    assert!(!second.is_finished());
    assert_eq!(probe.closes.load(Ordering::Acquire), 1);
    assert_eq!(budget.used_bytes(), OWNER_BYTES);
    gate.release();
    first.await.expect("first close").expect("first retired");
    second.await.expect("second close").expect("second retired");
    assert!(lock(&worker.control.state).joined);
    assert!(lock(&worker.control.thread).is_none());
    close(&worker).await;
    assert_eq!(probe.closes.load(Ordering::Acquire), 1);
    drop(worker);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn idle_revocation_retires_provider_without_any_frame_exchange() {
    let _serial = TESTS.lock().await;
    let revoked = Arc::new(AtomicBool::new(false));
    let lease = revoked.clone();
    let current: CurrentCheck = Arc::new(move || {
        if lease.load(Ordering::Acquire) {
            Err(Error::Stale)
        } else {
            Ok(())
        }
    });
    let probe = Arc::new(Probe::default());
    let budget = pool();
    let worker = start(probe.clone(), &budget, current);
    worker.ready().await.expect("ready");
    revoked.store(true, Ordering::Release);
    until(|| probe.drops.load(Ordering::Acquire) == 1).await;
    assert_eq!(probe.exchanges.load(Ordering::Acquire), 0);
    close(&worker).await;
    drop(worker);
    assert_eq!(budget.used_bytes(), 0);
}

#[tokio::test]
async fn charged_packet_outlives_endpoint_owner_and_its_actual_thread() {
    let _serial = TESTS.lock().await;
    let process = SessionBudget::new(128 * 1024 * 1024).expect("process pool");
    let budget = process.child(64 * 1024 * 1024).expect("endpoint pool");
    let probe = Arc::new(Probe {
        payload_bytes: 4096,
        ..Probe::default()
    });
    let worker = start(probe.clone(), &budget, current());
    worker.ready().await.expect("ready");
    let packet = worker.exchange(poll()).await.expect("charged packet");
    let payload_charge = packet.payload.charge().bytes();
    assert!(payload_charge >= 4096);
    assert!(packet.belongs_to(&budget));
    close(&worker).await;
    drop(worker);
    let retained = budget.used_bytes();
    assert!(retained >= payload_charge);
    assert_eq!(process.used_bytes(), retained);
    drop(budget);
    assert_eq!(packet.payload.bytes(), &[7; 4096]);
    assert_eq!(process.used_bytes(), retained);
    drop(packet);
    assert_eq!(process.used_bytes(), 0);
}

#[tokio::test]
async fn orphaned_failed_close_is_retained_retried_and_joined_before_successor() {
    let _serial = TESTS.lock().await;
    let probe = Arc::new(Probe {
        fail_close: AtomicBool::new(true),
        ..Probe::default()
    });
    let budget = pool();
    let worker = start(probe.clone(), &budget, current());
    worker.ready().await.expect("ready");
    drop(worker);
    until(|| probe.closes.load(Ordering::Acquire) >= 1).await;
    assert_eq!(probe.drops.load(Ordering::Acquire), 0);
    assert_eq!(budget.used_bytes(), OWNER_BYTES);
    let successor = Arc::new(Probe::default());
    assert!(matches!(
        Worker::start(
            request(),
            budget.clone(),
            current(),
            opener(successor.clone())
        ),
        Err(Error::Busy)
    ));
    probe.fail_close.store(false, Ordering::Release);
    until(|| probe.drops.load(Ordering::Acquire) == 1).await;
    // Starting a successor performs the actual old-thread join, then releases
    // its reservation before allocating the successor actor.
    let worker = start(successor.clone(), &budget, current());
    worker.ready().await.expect("successor ready");
    assert_eq!(budget.used_bytes(), OWNER_BYTES);
    assert_eq!(probe.unsafe_drops.load(Ordering::Acquire), 0);
    close(&worker).await;
    drop(worker);
    assert_eq!(budget.used_bytes(), 0);
}
