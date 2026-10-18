//! Synthetic local ownership tests, not producer/device approval. Run the full
//! giant_images suite with --test-threads=1: legacy producer tests have their own
//! serialization guard around the SAME process-global two-input admission.
//! The quarantine probe runs in a bounded child test process because it correctly
//! retains its input permit for process life; it never resets that real counter.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use std::{
    future::{Future, poll_fn},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::AtomicUsize,
        mpsc::{self, Receiver, Sender},
    },
    task::Poll,
    time::{Duration, Instant},
};
use vw_store::{DiskSpace, StoreError};

const OWNER: u64 = 4 * 1024 * 1024;
fn budget() -> SessionBudget {
    SessionBudget::new(vw_stream::PHONE_SESSION_BYTES).unwrap()
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
}
async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(8), future)
        .await
        .expect("bounded ownership completion")
}
async fn pending<T>(mut future: std::pin::Pin<&mut impl Future<Output = T>>) {
    assert!(poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx).is_pending())).await);
}
fn png() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[21, 42, 84])
            .unwrap();
    }
    out
}
fn opened() -> (
    RootOwnedPickedOriginal,
    GiantInputLimits,
    PathBuf,
    tempfile::NamedTempFile,
) {
    let mut selected = tempfile::NamedTempFile::new().unwrap();
    selected.write_all(&png()).unwrap();
    let work = tempfile::tempdir().unwrap();
    let path = work.path().to_path_buf();
    (
        RootOwnedPickedOriginal {
            file: std::fs::File::open(selected.path()).unwrap(),
            work,
        },
        GiantInputLimits {
            max_original_bytes: 4 * 1024 * 1024,
            max_work_disk_bytes: 512 * 1024 * 1024,
            assume_untagged_srgb: true,
        },
        path,
        selected,
    )
}
fn picked(budget: &SessionBudget) -> (Arc<PickedOriginal>, PathBuf, tempfile::NamedTempFile) {
    let pending = reserve_picked_input(budget.clone()).unwrap();
    let (original, limits, path, selected) = opened();
    (pending.attach(original, limits).unwrap(), path, selected)
}
struct Plenty;
impl FreeSpaceQuery for Plenty {
    fn query(&self, _: &Path) -> std::result::Result<DiskSpace, StoreError> {
        Ok(DiskSpace {
            total_bytes: 200_000_000_000,
            available_bytes: 150_000_000_000,
        })
    }
}
struct Blocked {
    first: Mutex<Option<(tokio::sync::oneshot::Sender<()>, Receiver<()>)>>,
    settled: Option<Sender<()>>,
}
impl FreeSpaceQuery for Blocked {
    fn query(&self, path: &Path) -> std::result::Result<DiskSpace, StoreError> {
        let first = self.first.lock().unwrap().take();
        if let Some((started, release)) = first {
            let _ = started.send(());
            release
                .recv_timeout(Duration::from_secs(8))
                .map_err(|_| StoreError::Invalid("bounded fixture release"))?;
        }
        Plenty.query(path)
    }
}
impl Drop for Blocked {
    fn drop(&mut self) {
        if let Some(settled) = self.settled.take() {
            let _ = settled.send(());
        }
    }
}
fn blocked() -> (
    Blocked,
    tokio::sync::oneshot::Receiver<()>,
    Sender<()>,
    Receiver<()>,
) {
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = mpsc::channel();
    let (settled, retired_query) = mpsc::channel();
    (
        Blocked {
            first: Mutex::new(Some((started, wait))),
            settled: Some(settled),
        },
        ready,
        release,
        retired_query,
    )
}

#[test]
fn memory_and_slot_refusals_never_invoke_the_opening_hook() {
    fn root_open(budget: SessionBudget, hook: impl FnOnce(PendingInputAdmission)) -> Result<()> {
        let admitted = reserve_picked_input(budget)?;
        hook(admitted);
        Ok(())
    }
    let calls = AtomicUsize::new(0);
    let tiny = SessionBudget::new(OWNER - 1).unwrap();
    assert!(matches!(
        root_open(tiny.clone(), |_| {
            calls.fetch_add(1, Ordering::AcqRel);
        }),
        Err(Error::Memory { .. })
    ));
    assert_eq!(tiny.used_bytes(), 0);
    let budget = budget();
    let a = reserve_picked_input(budget.clone()).unwrap();
    let b = reserve_picked_input(budget.clone()).unwrap();
    assert_eq!(budget.used_bytes(), 2 * OWNER);
    assert!(matches!(
        root_open(budget.clone(), |_| {
            calls.fetch_add(1, Ordering::AcqRel);
        }),
        Err(Error::Backpressure)
    ));
    assert_eq!(calls.load(Ordering::Acquire), 0);
    assert_eq!(budget.used_bytes(), 2 * OWNER);
    drop(a);
    let replacement = reserve_picked_input(budget.clone()).unwrap();
    assert_eq!(budget.used_bytes(), 2 * OWNER);
    drop(b);
    drop(replacement);
    assert_eq!(budget.used_bytes(), 0);
}
#[test]
fn two_selected_inputs_prepare_without_a_second_permit_or_charge() {
    let budget = budget();
    let (a, a_path, a_selected) = picked(&budget);
    let (b, b_path, b_selected) = picked(&budget);
    assert_eq!(budget.used_bytes(), 2 * OWNER);
    assert!(matches!(
        reserve_picked_input(budget.clone()),
        Err(Error::Backpressure)
    ));
    runtime().block_on(async {
        // Native root callers can carry this future across executor threads.
        // Dropping it before its first poll must leave the picked owner ready.
        fn require_send<T: Send>(_: T) {}
        require_send(a.prepare(Arc::new(Cancellation::new())));
        let first = bounded(a.prepare(Arc::new(Cancellation::new())))
            .await
            .unwrap();
        let second = bounded(b.prepare(Arc::new(Cancellation::new())))
            .await
            .unwrap();
        assert_eq!(budget.used_bytes(), 2 * OWNER);
        assert_eq!(first.describe().asset_id, *blake3::hash(&png()).as_bytes());
        assert_eq!(std::fs::read(first.describe().source_path).unwrap(), png());
        bounded(a.close_worker()).await.unwrap();
        bounded(a.close_worker()).await.unwrap();
        bounded(b.close_worker()).await.unwrap();
        drop(a);
        drop(b);
        assert!(!first.closed.load(Ordering::Acquire));
        assert!(!second.closed.load(Ordering::Acquire));
        assert!(a_path.exists() && b_path.exists());
        assert!(matches!(
            reserve_picked_input(budget.clone()),
            Err(Error::Backpressure)
        ));
        bounded(first.close_worker()).await.unwrap();
        assert!(!a_path.exists());
        assert_eq!(budget.used_bytes(), OWNER);
        bounded(second.close_worker()).await.unwrap();
        assert!(!b_path.exists());
        assert_eq!(budget.used_bytes(), 0);
    });
    assert_eq!(std::fs::read(a_selected.path()).unwrap(), png());
    assert_eq!(std::fs::read(b_selected.path()).unwrap(), png());
}
#[test]
fn close_ready_input_cleans_before_return_and_forbids_preparation() {
    let budget = budget();
    let (picked, path, selected) = picked(&budget);
    runtime().block_on(async {
        bounded(picked.close_worker()).await.unwrap();
        assert!(!path.exists());
        assert_eq!(budget.used_bytes(), 0);
        assert!(matches!(
            picked.prepare(Arc::new(Cancellation::new())).await,
            Err(Error::Closed)
        ));
        bounded(picked.close_worker()).await.unwrap();
    });
    assert_eq!(std::fs::read(selected.path()).unwrap(), png());
}
#[test]
fn dropping_an_unprepared_picker_retires_its_actual_input() {
    let budget = budget();
    let (picked, path, selected) = picked(&budget);
    drop(picked);
    assert_eq!(budget.used_bytes(), 0);
    assert!(!path.exists());
    assert_eq!(std::fs::read(selected.path()).unwrap(), png());
}
#[test]
fn cancelled_before_start_consumes_one_attempt_and_never_runs_the_body() {
    let budget = budget();
    let (picked, path, selected) = picked(&budget);
    let cancel = Arc::new(Cancellation::new());
    cancel.cancel();
    let (query, ready, _release, retired) = blocked();
    runtime().block_on(async {
        assert!(matches!(
            bounded(picked.prepare_with(cancel, query, 2)).await,
            Err(Error::Cancelled)
        ));
        assert!(
            bounded(ready).await.is_err(),
            "body invoked its free-space query"
        );
        retired.recv_timeout(Duration::from_secs(2)).unwrap();
        bounded(picked.close_worker()).await.unwrap();
        assert!(matches!(
            picked.prepare(Arc::new(Cancellation::new())).await,
            Err(Error::Closed)
        ));
    });
    assert!(!path.exists());
    assert_eq!(budget.used_bytes(), 0);
    assert_eq!(std::fs::read(selected.path()).unwrap(), png());
}
#[test]
fn close_joins_blocked_preparation_and_keeps_the_same_charge_until_release() {
    let budget = budget();
    let (picked, path, selected) = picked(&budget);
    let (query, ready, release, _retired) = blocked();
    runtime().block_on(async {
        let mut preparing = Box::pin(picked.prepare_with(Arc::new(Cancellation::new()), query, 2));
        pending(preparing.as_mut()).await;
        bounded(ready).await.unwrap();
        let mut closing = Box::pin(picked.close_worker());
        pending(closing.as_mut()).await;
        assert_eq!(budget.used_bytes(), OWNER);
        assert!(path.exists());
        assert!(!path.join("original.input").exists());
        release.send(()).unwrap();
        bounded(closing).await.unwrap();
        assert!(matches!(bounded(preparing).await, Err(Error::Closed)));
        assert!(!path.exists());
        assert_eq!(budget.used_bytes(), 0);
    });
    assert_eq!(std::fs::read(selected.path()).unwrap(), png());
}
#[test]
fn abandoned_running_reply_cancels_actual_work_and_close_joins_cleanup() {
    let budget = budget();
    let (picked, path, selected) = picked(&budget);
    let (query, ready, release, _retired) = blocked();
    runtime().block_on(async {
        let mut preparing = Box::pin(picked.prepare_with(Arc::new(Cancellation::new()), query, 2));
        pending(preparing.as_mut()).await;
        bounded(ready).await.unwrap();
        drop(preparing);
        assert_eq!(budget.used_bytes(), OWNER);
        let mut closing = Box::pin(picked.close_worker());
        pending(closing.as_mut()).await;
        release.send(()).unwrap();
        bounded(closing).await.unwrap();
        assert_eq!(budget.used_bytes(), 0);
        assert!(!path.exists());
    });
    assert_eq!(std::fs::read(selected.path()).unwrap(), png());
}
#[test]
fn abandoned_completed_reply_retires_the_undelivered_source_without_a_worker() {
    let budget = budget();
    let (picked, path, selected) = picked(&budget);
    let (query, ready, release, retired) = blocked();
    runtime().block_on(async {
        let mut preparing = Box::pin(picked.prepare_with(Arc::new(Cancellation::new()), query, 2));
        pending(preparing.as_mut()).await;
        bounded(ready).await.unwrap();
        release.send(()).unwrap();
        retired.recv_timeout(Duration::from_secs(8)).unwrap();
        assert!(path.join("original.input").exists());
        assert_eq!(budget.used_bytes(), OWNER);
        drop(preparing);
        bounded(picked.close_worker()).await.unwrap();
        assert!(!path.exists());
        assert_eq!(budget.used_bytes(), 0);
    });
    assert_eq!(std::fs::read(selected.path()).unwrap(), png());
}
#[test]
fn close_retires_completed_source_while_reply_future_is_alive_and_unpolled() {
    let budget = budget();
    let (picked, path, selected) = picked(&budget);
    let (query, ready, release, retired) = blocked();
    runtime().block_on(async {
        let mut preparing = Box::pin(picked.prepare_with(Arc::new(Cancellation::new()), query, 2));
        pending(preparing.as_mut()).await;
        bounded(ready).await.unwrap();
        release.send(()).unwrap();
        retired.recv_timeout(Duration::from_secs(8)).unwrap();
        assert!(path.join("original.input").exists());
        assert_eq!(budget.used_bytes(), OWNER);
        // Keep preparing alive and unpolled: the shared control owns its source.
        bounded(picked.close_worker()).await.unwrap();
        assert!(!path.exists());
        assert_eq!(budget.used_bytes(), 0);
        assert!(matches!(bounded(preparing).await, Err(Error::Closed)));
    });
    assert_eq!(std::fs::read(selected.path()).unwrap(), png());
}
#[test]
fn failed_attach_retires_file_directory_permit_and_charge() {
    let budget = budget();
    let admitted = reserve_picked_input(budget.clone()).unwrap();
    let (original, mut limits, path, selected) = opened();
    limits.assume_untagged_srgb = false;
    assert!(matches!(
        admitted.attach(original, limits),
        Err(Error::Unsupported)
    ));
    assert!(!path.exists());
    assert_eq!(budget.used_bytes(), 0);
    let a = reserve_picked_input(budget.clone()).unwrap();
    let b = reserve_picked_input(budget.clone()).unwrap();
    drop((a, b));
    assert_eq!(std::fs::read(selected.path()).unwrap(), png());
}
#[test]
fn failed_prepared_worker_constructor_retires_its_actual_state() {
    let budget = budget();
    let (picked, path, selected) = picked(&budget);
    runtime().block_on(async {
        assert!(matches!(
            bounded(picked.prepare_with(Arc::new(Cancellation::new()), Plenty, usize::MAX)).await,
            Err(Error::Invalid)
        ));
        bounded(picked.close_worker()).await.unwrap();
        assert!(!path.exists());
        assert_eq!(budget.used_bytes(), 0);
    });
    assert_eq!(std::fs::read(selected.path()).unwrap(), png());
}
#[test]
fn prepared_close_keeps_admission_with_an_actual_retained_source() {
    let budget = budget();
    let (picked, path, _selected) = picked(&budget);
    runtime().block_on(async {
        let prepared = bounded(picked.prepare_with(Arc::new(Cancellation::new()), Plenty, 2))
            .await
            .unwrap();
        let source = bounded(
            prepared
                .worker
                .call(|state| Ok(state.as_ref().unwrap().source.clone())),
        )
        .await
        .unwrap();
        bounded(picked.close_worker()).await.unwrap();
        bounded(prepared.close_worker()).await.unwrap();
        assert!(path.exists());
        assert_eq!(budget.used_bytes(), OWNER);
        input::release(source).unwrap();
        assert!(!path.exists());
        assert_eq!(budget.used_bytes(), 0);
    });
}
#[test]
fn abandoned_queued_body_is_skipped_and_retains_admission_until_actual_drop() {
    let budget = budget();
    let (picked, path, selected) = picked(&budget);
    runtime().block_on(async {
        // Reproduce startup's exact Worker::call receiver-abandonment branch
        // behind a deterministic running blocker, without a production hook.
        let worker = Worker::new("picked-queued-fixture", (), 1).unwrap();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, wait) = mpsc::channel();
        worker
            .enqueue(Box::new(move |_| {
                let _ = started.send(());
                wait.recv_timeout(Duration::from_secs(8)).unwrap();
            }))
            .unwrap();
        bounded(ready).await.unwrap();
        let (input, limits) =
            match std::mem::replace(&mut picked.control.status().state, State::Working) {
                State::Ready { input, limits } => (input, limits),
                _ => panic!("fresh fixture was consumed"),
            };
        let preparation = Preparation {
            input: Some(input),
            limits,
            control: picked.control.clone(),
            finished: false,
        };
        let ran = Arc::new(AtomicBool::new(false));
        let observed = ran.clone();

        let mut queued = Box::pin(worker.call(move |_| {
            observed.store(true, Ordering::Release);
            Ok(preparation.run(&Cancellation::new(), &Plenty))
        }));
        pending(queued.as_mut()).await;
        drop(queued);
        let mut closing = Box::pin(picked.close_worker());
        pending(closing.as_mut()).await;
        assert_eq!(budget.used_bytes(), OWNER);
        assert!(path.exists());
        release.send(()).unwrap();
        bounded(closing).await.unwrap();
        bounded(worker.shutdown(|_| Ok(()))).await.unwrap();
        assert!(!ran.load(Ordering::Acquire));
        assert!(!path.exists());
        assert_eq!(budget.used_bytes(), 0);
    });
    assert_eq!(std::fs::read(selected.path()).unwrap(), png());
}
#[test]
fn storage_operation_failure_does_not_invent_a_cleanup_failure() {
    struct FailedQuery;
    impl FreeSpaceQuery for FailedQuery {
        fn query(&self, _: &Path) -> std::result::Result<DiskSpace, StoreError> {
            Err(StoreError::Io(std::io::Error::other(
                "synthetic space failure",
            )))
        }
    }
    let budget = budget();
    let (picked, path, selected) = picked(&budget);
    runtime().block_on(async {
        assert!(matches!(
            bounded(picked.prepare_with(Arc::new(Cancellation::new()), FailedQuery, 2)).await,
            Err(Error::Storage)
        ));
        bounded(picked.close_worker()).await.unwrap();
        assert_eq!(budget.used_bytes(), 0);
        assert!(!path.exists());
    });
    assert_eq!(std::fs::read(selected.path()).unwrap(), png());
}
#[test]
fn startup_saturation_retires_the_input_without_starting_its_body() {
    let budget = budget();
    let (picked, path, selected) = picked(&budget);
    runtime().block_on(async {
        let mut running: Vec<std::pin::Pin<Box<dyn Future<Output = crate::Result<()>>>>> =
            Vec::new();
        let mut releases = Vec::new();
        for _ in 0..4 {
            let (started, ready) = tokio::sync::oneshot::channel();
            let (release, wait) = mpsc::channel();
            let mut job = Box::pin(crate::worker::startup(move || {
                let _ = started.send(());
                wait.recv_timeout(Duration::from_secs(8))
                    .map_err(|_| crate::CoreError::Worker)?;
                Ok(())
            }));
            pending(job.as_mut()).await;
            bounded(ready).await.unwrap();
            running.push(job);
            releases.push(release);
        }
        assert!(matches!(
            bounded(picked.prepare_with(Arc::new(Cancellation::new()), Plenty, 2)).await,
            Err(Error::Backpressure)
        ));
        bounded(picked.close_worker()).await.unwrap();
        assert_eq!(budget.used_bytes(), 0);
        assert!(!path.exists());
        for release in releases {
            release.send(()).unwrap();
        }
        for job in running {
            bounded(job).await.unwrap();
        }
    });
    assert_eq!(std::fs::read(selected.path()).unwrap(), png());
}
#[test]
fn cleanup_failure_retains_the_real_admission_in_an_isolated_process() {
    // libtest omits the crate prefix carried by module_path!. Require one
    // executed case below so a misspelled --exact filter cannot silently pass.
    let module = module_path!().split_once("::").unwrap().1;
    let name = format!("{module}::quarantine_probe");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--ignored", "--test-threads=1"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        if child.try_wait().unwrap().is_some() {
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(
                stdout.contains("test result: ok. 1 passed; 0 failed"),
                "{stdout}"
            );
            assert!(stdout.contains(&name), "{stdout}");
            println!(
                "quarantine probe executed: 1 passed; same4MiB retained and third input refused"
            );
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("quarantine child exceeded its bounded lifetime");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
#[ignore = "Executed by the bounded quarantine wrapper; do not leak this process-global permit into unrelated tests"]
fn quarantine_probe() {
    let budget = budget();
    let (picked, path, selected) = picked(&budget);
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, b"owned cleanup-failure fixture").unwrap();
    runtime().block_on(async {
        assert!(matches!(
            bounded(picked.prepare_with(Arc::new(Cancellation::new()), Plenty, 2)).await,
            Err(Error::Storage)
        ));
        assert!(matches!(
            bounded(picked.close_worker()).await,
            Err(Error::Storage)
        ));
        assert_eq!(budget.used_bytes(), OWNER);
        let second = reserve_picked_input(budget.clone()).unwrap();
        assert!(matches!(
            reserve_picked_input(budget.clone()),
            Err(Error::Backpressure)
        ));
        assert_eq!(budget.used_bytes(), 2 * OWNER);
        drop(second);
        assert_eq!(budget.used_bytes(), OWNER);
    });
    assert_eq!(std::fs::read(selected.path()).unwrap(), png());
    std::fs::remove_file(path).unwrap();
    // No counter reset or fake credit. The exact quarantine owns permit+charge
    // until this isolated process exits; no recovery/approval claim is made.
}
