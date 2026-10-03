use super::{SessionError, SessionResult};
use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::{Semaphore, oneshot, watch};

static RUNTIMES: AtomicUsize = AtomicUsize::new(0);
struct Permit;
impl Drop for Permit {
    fn drop(&mut self) {
        RUNTIMES.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Inner {
    handle: tokio::runtime::Handle,
    stop: watch::Sender<bool>,
    done: watch::Receiver<bool>,
    closed: AtomicBool,
    slots: Arc<Semaphore>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}
#[derive(Clone)]
pub(crate) struct NetworkRuntime(Arc<Inner>);
struct Abort(tokio::task::AbortHandle);
impl Drop for Abort {
    fn drop(&mut self) {
        self.0.abort();
    }
}
impl NetworkRuntime {
    /// Called from a core startup worker, never a foreign UI thread.
    pub fn new() -> SessionResult<Self> {
        RUNTIMES
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 8).then_some(count + 1)
            })
            .map_err(|_| SessionError::Backpressure)?;
        let permit = Permit;
        let (ready, started) = std::sync::mpsc::sync_channel(1);
        let (stop, mut stopping) = watch::channel(false);
        let (done, finished) = watch::channel(false);
        std::thread::Builder::new()
            .name("vw-app-network".into())
            .spawn(move || {
                let _permit = permit;
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .max_blocking_threads(2)
                    .build();
                match runtime {
                    Ok(runtime) => {
                        if ready.send(Ok(runtime.handle().clone())).is_ok() {
                            runtime.block_on(async {
                                while !*stopping.borrow() {
                                    if stopping.changed().await.is_err() {
                                        break;
                                    }
                                }
                            });
                        }
                        // Platform reads have a five-second bound. Shutdown is owned
                        // by this worker rather than a foreign object destructor.
                        runtime.shutdown_timeout(std::time::Duration::from_secs(6));
                    }
                    Err(_) => {
                        let _ = ready.send(Err(SessionError::Worker));
                    }
                }
                let _ = done.send(true);
            })
            .map_err(|_| SessionError::Worker)?;
        let handle = started.recv().map_err(|_| SessionError::Worker)??;
        Ok(Self(Arc::new(Inner {
            handle,
            stop,
            done: finished,
            closed: AtomicBool::new(false),
            slots: Arc::new(Semaphore::new(8)),
        })))
    }
    pub async fn call<R: Send + 'static>(
        &self,
        future: impl Future<Output = SessionResult<R>> + Send + 'static,
    ) -> SessionResult<R> {
        if self.0.closed.load(Ordering::Acquire) {
            return Err(SessionError::Closed);
        }
        let permit = self
            .0
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| SessionError::Backpressure)?;
        let (send, receive) = oneshot::channel();
        let task = self.0.handle.spawn(async move {
            let _permit = permit;
            let result = future.await;
            let _ = send.send(result);
        });
        let _abort = Abort(task.abort_handle());
        drop(task);
        receive.await.map_err(|_| SessionError::Closed)?
    }
    pub async fn cancellable<R: Send + 'static>(
        &self,
        cancellation: Arc<crate::Cancellation>,
        future: impl Future<Output = SessionResult<R>> + Send + 'static,
    ) -> SessionResult<R> {
        self.call(async move {
            let cancelled=async move { loop { if cancellation.is_cancelled(){break;} tokio::time::sleep(std::time::Duration::from_millis(20)).await; } };
            tokio::select! { biased; _=cancelled=>Err(SessionError::Cancelled), result=future=>result }
        }).await
    }
    pub fn stop(&self) {
        self.0.closed.store(true, Ordering::Release);
        let _ = self.0.stop.send(true);
    }
    pub async fn close(&self) -> SessionResult<()> {
        self.stop();
        let mut done = self.0.done.clone();
        while !*done.borrow() {
            done.changed().await.map_err(|_| SessionError::Worker)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::{future::poll_fn, task::Poll};
    #[tokio::test]
    async fn cancelled_calls_release_admission_and_close_ignores_queue_pressure() {
        let runtime = NetworkRuntime::new().unwrap();
        let token = Arc::new(crate::Cancellation::new());
        token.cancel();
        assert!(matches!(
            runtime
                .cancellable(token, std::future::pending::<SessionResult<()>>())
                .await,
            Err(SessionError::Cancelled)
        ));
        let mut calls = Vec::new();
        for _ in 0..8 {
            let mut call = Box::pin(runtime.call(std::future::pending::<SessionResult<()>>()));
            assert!(poll_fn(|cx| Poll::Ready(call.as_mut().poll(cx).is_pending())).await);
            calls.push(call);
        }
        assert!(matches!(
            runtime.call(async { Ok(()) }).await,
            Err(SessionError::Backpressure)
        ));
        drop(calls);
        runtime.close().await.unwrap();
        assert!(matches!(
            runtime.call(async { Ok(()) }).await,
            Err(SessionError::Closed)
        ));
        runtime.close().await.unwrap();
    }
}
