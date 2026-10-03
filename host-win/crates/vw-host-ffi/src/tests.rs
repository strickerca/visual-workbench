use crate::{
    worker::{self, Backend, RequestContext},
    *,
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
pub(crate) static SERIAL: Mutex<()> = Mutex::new(());

#[derive(Default)]
struct FakeState {
    published: Mutex<Vec<Vec<u8>>>,
    threads: Mutex<Vec<std::thread::ThreadId>>,
    hold_point: AtomicBool,
    point_entered: AtomicBool,
    hold_publish: AtomicBool,
    publish_entered: AtomicBool,
    fail_publication: AtomicBool,
    dropped: AtomicUsize,
}
struct FakeBackend(Arc<FakeState>);
impl FakeBackend {
    fn record_thread(&self) -> HostResult<()> {
        self.0
            .threads
            .lock()
            .map_err(|_| HostError::WorkerUnavailable)?
            .push(std::thread::current().id());
        Ok(())
    }
}
impl Drop for FakeBackend {
    fn drop(&mut self) {
        self.0.dropped.fetch_add(1, Ordering::AcqRel);
    }
}
fn info() -> DpiInfo {
    DpiInfo {
        monitor_bounds: PhysicalRect {
            left: -1200,
            top: 0,
            right: 0,
            bottom: 800,
        },
        work_area: PhysicalRect {
            left: -1200,
            top: 0,
            right: 0,
            bottom: 760,
        },
        dpi: 144,
        scale_factor: 1.5,
        per_monitor_v2: true,
    }
}
impl Backend for FakeBackend {
    fn dpi_at_point(&mut self, _: i32, _: i32) -> HostResult<DpiInfo> {
        self.record_thread()?;
        self.0.point_entered.store(true, Ordering::Release);
        let limit = Instant::now() + Duration::from_secs(3);
        while self.0.hold_point.load(Ordering::Acquire) {
            if Instant::now() >= limit {
                return Err(HostError::Timeout);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(info())
    }
    fn window_dpi_info(&mut self, _: u64) -> HostResult<WindowDpiInfo> {
        self.record_thread()?;
        Ok(WindowDpiInfo {
            window_bounds: info().monitor_bounds,
            client_bounds: info().work_area,
            monitor: info(),
            window_dpi: 96,
            window_per_monitor_v2: false,
        })
    }
    fn publish_png(&mut self, bytes: &[u8], context: &RequestContext) -> HostResult<u32> {
        self.record_thread()?;
        self.0.publish_entered.store(true, Ordering::Release);
        while self.0.hold_publish.load(Ordering::Acquire) {
            context.checkpoint()?;
            std::thread::sleep(Duration::from_millis(1));
        }
        context.begin_publication()?;
        if self.0.fail_publication.load(Ordering::Acquire) {
            return Err(HostError::ClipboardPublicationFailed);
        }
        self.0
            .published
            .lock()
            .map_err(|_| HostError::WorkerUnavailable)?
            .push(bytes.to_vec());
        Ok(71)
    }
    fn publish_image(&mut self, _: &[u8], _: &[u8], _: &RequestContext) -> HostResult<u32> {
        Err(HostError::UnsupportedPlatform)
    }
    fn read_image(
        &mut self,
        _: ClipboardReadFormat,
        _: &RequestContext,
    ) -> HostResult<worker::ClipboardSnapshot> {
        Err(HostError::UnsupportedPlatform)
    }
    fn pump_messages(&mut self) {}
}

fn png_fixture(depth: png::BitDepth) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(depth);
    let data = if depth == png::BitDepth::Sixteen {
        vec![
            0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xff, 0xff, 0xde, 0xf0, 0x23, 0x45, 0x67, 0x89, 0,
            0,
        ]
    } else {
        vec![18, 52, 86, 255, 120, 154, 188, 0]
    };
    encoder.write_header()?.write_image_data(&data)?;
    Ok(bytes)
}
fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
}
async fn fake(state: &Arc<FakeState>) -> HostResult<Arc<HostService>> {
    let state = state.clone();
    worker::start_with(move || Ok(FakeBackend(state))).await
}
async fn wait_for(predicate: impl Fn() -> bool) -> HostResult<()> {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        if Instant::now() >= deadline {
            return Err(HostError::Timeout);
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    Ok(())
}
fn poll_once<T>(future: Pin<&mut impl Future<Output = T>>) -> Poll<T> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

#[test]
fn dpi_and_validated_png_execute_off_caller_and_release_worker() -> TestResult {
    let _serial = SERIAL.lock().map_err(|_| "test lock")?;
    runtime()?.block_on(async {
        let state = Arc::new(FakeState::default());
        let service = fake(&state).await?;
        assert_eq!(service.dpi_at_point(-200, 100).await?, info());
        let window = service.window_dpi_info(42).await?;
        assert_eq!(window.window_dpi, 96);
        assert_eq!(window.monitor.dpi, 144);
        assert!(!window.window_per_monitor_v2);
        for depth in [png::BitDepth::Eight, png::BitDepth::Sixteen] {
            let png = png_fixture(depth)?;
            let operation = HostOperation::new();
            let receipt = service
                .set_clipboard_png(png.clone(), operation.clone())
                .await?;
            assert_eq!((receipt.width, receipt.height), (2, 1));
            assert_eq!(
                receipt.bit_depth,
                if depth == png::BitDepth::Eight { 8 } else { 16 }
            );
            assert_eq!(receipt.png_bytes, png.len() as u64);
            assert!(receipt.png_format_published);
            assert_eq!(receipt.clipboard_sequence, 71);
            assert_eq!(operation.status(), HostOperationStatus::Succeeded);
            assert!(!operation.cancel());
            assert_eq!(
                state.published.lock().map_err(|_| "publications")?.last(),
                Some(&png)
            );
        }
        assert!(
            state
                .threads
                .lock()
                .map_err(|_| "threads")?
                .iter()
                .all(|id| *id != std::thread::current().id())
        );
        service.shutdown().await?;
        assert_eq!(state.dropped.load(Ordering::Acquire), 1);
        assert_eq!(worker::live_workers(), 0);
        assert!(matches!(
            service.dpi_at_point(0, 0).await,
            Err(HostError::Closed)
        ));
        Ok(())
    })
}

#[test]
fn invalid_png_limits_and_reused_tokens_never_reach_clipboard() -> TestResult {
    let _serial = SERIAL.lock().map_err(|_| "test lock")?;
    runtime()?.block_on(async {
        let state = Arc::new(FakeState::default());
        let service = fake(&state).await?;
        let good = png_fixture(png::BitDepth::Eight)?;
        for bad in [vec![1, 2, 3], good[..33].to_vec()] {
            let operation = HostOperation::new();
            assert!(matches!(
                service.set_clipboard_png(bad, operation.clone()).await,
                Err(HostError::InvalidPng)
            ));
            assert_eq!(operation.status(), HostOperationStatus::Failed);
            assert!(matches!(
                service.set_clipboard_png(good.clone(), operation).await,
                Err(HostError::AlreadyUsed)
            ));
        }
        let mut huge_header = good.clone();
        huge_header[16..20].copy_from_slice(&16_000_001u32.to_be_bytes());
        assert!(matches!(
            service
                .set_clipboard_png(huge_header, HostOperation::new())
                .await,
            Err(HostError::SizeLimit)
        ));
        assert!(matches!(
            service
                .set_clipboard_png(vec![0; worker::MAX_PNG_BYTES + 1], HostOperation::new())
                .await,
            Err(HostError::SizeLimit)
        ));
        assert!(matches!(
            service.window_dpi_info(0).await,
            Err(HostError::InvalidWindow)
        ));
        assert!(
            state
                .published
                .lock()
                .map_err(|_| "publications")?
                .is_empty()
        );
        service.shutdown().await?;
        Ok(())
    })
}

#[test]
fn cancellation_before_enqueue_and_during_wait_prevents_publication() -> TestResult {
    let _serial = SERIAL.lock().map_err(|_| "test lock")?;
    runtime()?.block_on(async {
        let state = Arc::new(FakeState::default());
        let service = fake(&state).await?;
        let png = png_fixture(png::BitDepth::Eight)?;
        let cancelled = HostOperation::new();
        assert!(cancelled.cancel());
        assert!(matches!(
            service.set_clipboard_png(png.clone(), cancelled).await,
            Err(HostError::Cancelled)
        ));
        state.hold_publish.store(true, Ordering::Release);
        let operation = HostOperation::new();
        let mut pending = Box::pin(service.set_clipboard_png(png, operation.clone()));
        assert!(poll_once(pending.as_mut()).is_pending());
        wait_for(|| state.publish_entered.load(Ordering::Acquire)).await?;
        assert_eq!(operation.status(), HostOperationStatus::WaitingForClipboard);
        assert!(operation.cancel());
        assert!(matches!(pending.await, Err(HostError::Cancelled)));
        assert!(
            state
                .published
                .lock()
                .map_err(|_| "publications")?
                .is_empty()
        );
        service.shutdown().await?;
        Ok(())
    })
}

#[test]
fn dropping_a_pending_future_cancels_queued_native_work() -> TestResult {
    let _serial = SERIAL.lock().map_err(|_| "test lock")?;
    runtime()?.block_on(async {
        let state = Arc::new(FakeState::default());
        state.hold_publish.store(true, Ordering::Release);
        let service = fake(&state).await?;
        let operation = HostOperation::new();
        let mut pending = Box::pin(
            service.set_clipboard_png(png_fixture(png::BitDepth::Eight)?, operation.clone()),
        );
        assert!(poll_once(pending.as_mut()).is_pending());
        wait_for(|| state.publish_entered.load(Ordering::Acquire)).await?;
        drop(pending);
        assert_eq!(operation.status(), HostOperationStatus::Cancelled);
        state.hold_publish.store(false, Ordering::Release);
        service.shutdown().await?;
        assert!(
            state
                .published
                .lock()
                .map_err(|_| "publications")?
                .is_empty()
        );
        assert_eq!(worker::live_workers(), 0);
        Ok(())
    })
}

#[test]
fn queue_full_is_backpressure_and_all_accepted_queries_complete() -> TestResult {
    let _serial = SERIAL.lock().map_err(|_| "test lock")?;
    runtime()?.block_on(async {
        let state = Arc::new(FakeState::default());
        state.hold_point.store(true, Ordering::Release);
        let service = fake(&state).await?;
        let mut first = Box::pin(service.dpi_at_point(0, 0));
        assert!(poll_once(first.as_mut()).is_pending());
        wait_for(|| state.point_entered.load(Ordering::Acquire)).await?;
        let mut pending = (0..8)
            .map(|x| Box::pin(service.dpi_at_point(x, 0)))
            .collect::<Vec<_>>();
        for future in &mut pending {
            assert!(poll_once(future.as_mut()).is_pending());
        }
        assert!(matches!(
            service.dpi_at_point(0, 0).await,
            Err(HostError::Busy)
        ));
        state.hold_point.store(false, Ordering::Release);
        assert_eq!(first.await?, info());
        for future in pending {
            assert_eq!(future.await?, info());
        }
        service.shutdown().await?;
        Ok(())
    })
}

#[test]
fn service_count_is_bounded_and_drop_releases_native_ownership() -> TestResult {
    let _serial = SERIAL.lock().map_err(|_| "test lock")?;
    runtime()?.block_on(async {
        let a = Arc::new(FakeState::default());
        let b = Arc::new(FakeState::default());
        let service_a = fake(&a).await?;
        let service_b = fake(&b).await?;
        assert!(matches!(
            fake(&Arc::new(FakeState::default())).await,
            Err(HostError::Busy)
        ));
        drop(service_a);
        wait_for(|| a.dropped.load(Ordering::Acquire) == 1).await?;
        service_b.shutdown().await?;
        wait_for(|| worker::live_workers() == 0).await?;
        Ok(())
    })
}

#[test]
fn publication_failure_is_not_reported_as_success_or_cancelled() -> TestResult {
    let _serial = SERIAL.lock().map_err(|_| "test lock")?;
    runtime()?.block_on(async {
        let state = Arc::new(FakeState::default());
        state.fail_publication.store(true, Ordering::Release);
        let service = fake(&state).await?;
        let operation = HostOperation::new();
        assert!(matches!(
            service
                .set_clipboard_png(png_fixture(png::BitDepth::Eight)?, operation.clone())
                .await,
            Err(HostError::ClipboardPublicationFailed)
        ));
        assert_eq!(operation.status(), HostOperationStatus::Failed);
        assert!(!operation.cancel());
        service.shutdown().await?;
        Ok(())
    })
}

#[cfg(windows)]
#[test]
fn native_dpi_read_is_physical_and_never_writes_clipboard() -> TestResult {
    let _serial = SERIAL.lock().map_err(|_| "test lock")?;
    runtime()?.block_on(async {
        let service = create_host_service().await?;
        let dpi = service.dpi_at_point(0, 0).await?;
        assert!(dpi.dpi > 0);
        assert_eq!(dpi.scale_factor, f64::from(dpi.dpi) / 96.0);
        assert!(dpi.per_monitor_v2);
        assert!(dpi.monitor_bounds.right > dpi.monitor_bounds.left);
        assert!(dpi.monitor_bounds.bottom > dpi.monitor_bounds.top);
        assert!(matches!(
            service.window_dpi_info(u64::MAX).await,
            Err(HostError::InvalidWindow)
        ));
        service.shutdown().await?;
        Ok(())
    })
}

#[cfg(not(windows))]
#[test]
fn other_platforms_return_explicit_unavailable() -> TestResult {
    let _serial = SERIAL.lock().map_err(|_| "test lock")?;
    runtime()?.block_on(async {
        assert!(matches!(
            create_host_service().await,
            Err(HostError::UnsupportedPlatform)
        ));
        wait_for(|| worker::live_workers() == 0).await?;
        Ok(())
    })
}
