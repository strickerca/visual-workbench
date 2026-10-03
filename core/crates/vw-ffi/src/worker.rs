use crate::{CoreError, Result};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, SyncSender, TrySendError},
};
use tokio::sync::{oneshot, watch};

#[derive(Default, uniffi::Object)]
pub struct Cancellation {
    flag: AtomicBool,
}
#[uniffi::export]
impl Cancellation {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}
impl Cancellation {
    pub(crate) fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(CoreError::Cancelled)
        } else {
            Ok(())
        }
    }
}

type Job<T> = Box<dyn FnOnce(&mut T) + Send>;
type ShutdownJob<T> = Box<dyn FnOnce(&mut T) -> Result<()> + Send>;
enum Message<T> {
    Call(Job<T>),
    Shutdown(ShutdownJob<T>),
}
struct Admission {
    queued: usize,
    capacity: usize,
    closing: bool,
}
pub(crate) struct Worker<T> {
    sender: SyncSender<Message<T>>,
    admission: Arc<Mutex<Admission>>,
    closed: watch::Sender<Option<Result<()>>>,
}
impl<T> Clone for Worker<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            admission: self.admission.clone(),
            closed: self.closed.clone(),
        }
    }
}
struct Completion(watch::Sender<Option<Result<()>>>);
impl Drop for Completion {
    fn drop(&mut self) {
        let unfinished = self.0.borrow().is_none();
        if unfinished {
            let _ = self.0.send_replace(Some(Err(CoreError::Worker)));
        }
    }
}
impl<T: Send + 'static> Worker<T> {
    pub fn new(name: &str, state: T, capacity: usize) -> Result<Self> {
        let (sender, receiver) =
            mpsc::sync_channel::<Message<T>>(capacity.checked_add(1).ok_or(CoreError::Invalid)?);
        let admission = Arc::new(Mutex::new(Admission {
            queued: 0,
            capacity,
            closing: false,
        }));
        let gate = admission.clone();
        let (closed, _) = watch::channel(None);
        let completion = closed.clone();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let completion = Completion(completion);
                let mut state = state;
                while let Ok(message) = receiver.recv() {
                    match message {
                        Message::Call(job) => {
                            let Ok(mut admission) = gate.lock() else {
                                break;
                            };
                            admission.queued -= 1;
                            drop(admission);
                            job(&mut state);
                        }
                        Message::Shutdown(job) => {
                            let result = job(&mut state);
                            // Completion includes destruction of state-owned permits,
                            // file handles and other resources, not only the last job.
                            drop(state);
                            let _ = completion.0.send_replace(Some(result));
                            return;
                        }
                    }
                }
            })
            .map_err(|_| CoreError::Worker)?;
        Ok(Self {
            sender,
            admission,
            closed,
        })
    }
    pub(crate) fn enqueue(&self, job: Job<T>) -> Result<()> {
        let mut admission = self.admission.lock().map_err(|_| CoreError::Worker)?;
        if admission.closing {
            return Err(CoreError::Closed);
        }
        if admission.queued >= admission.capacity {
            return Err(CoreError::Backpressure);
        }
        admission.queued += 1;
        if let Err(error) = self.sender.try_send(Message::Call(job)) {
            admission.queued -= 1;
            return Err(match error {
                TrySendError::Full(_) => CoreError::Backpressure,
                TrySendError::Disconnected(_) => CoreError::Closed,
            });
        }
        Ok(())
    }
    pub async fn call<R: Send + 'static>(
        &self,
        job: impl FnOnce(&mut T) -> Result<R> + Send + 'static,
    ) -> Result<R> {
        let (send, receive) = oneshot::channel();
        self.enqueue(Box::new(move |state| {
            if !send.is_closed() {
                let _ = send.send(job(state));
            }
        }))?;
        receive.await.map_err(|_| CoreError::Worker)?
    }
    /// One reserved FIFO slot survives ordinary queue saturation. Accepted work
    /// drains first; shutdown runs even if its awaiting foreign future is dropped.
    /// Repeated callers observe the same completion and never enqueue duplicates.
    /// Completion is published only after the worker state has been destroyed.
    pub async fn shutdown(
        &self,
        job: impl FnOnce(&mut T) -> Result<()> + Send + 'static,
    ) -> Result<()> {
        let mut receive = self.closed.subscribe();
        {
            let mut admission = self.admission.lock().map_err(|_| CoreError::Worker)?;
            if !admission.closing {
                admission.closing = true;
                if self
                    .sender
                    .try_send(Message::Shutdown(Box::new(job)))
                    .is_err()
                {
                    let _ = self.closed.send_replace(Some(Err(CoreError::Worker)));
                }
            }
        }
        loop {
            if let Some(result) = receive.borrow().clone() {
                return result;
            }
            receive.changed().await.map_err(|_| CoreError::Worker)?;
        }
    }
}
static STARTUPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct Permit;
impl Drop for Permit {
    fn drop(&mut self) {
        STARTUPS.fetch_sub(1, Ordering::AcqRel);
    }
}
pub(crate) async fn startup<R: Send + 'static>(
    job: impl FnOnce() -> Result<R> + Send + 'static,
) -> Result<R> {
    STARTUPS
        .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < 4).then_some(n + 1)
        })
        .map_err(|_| CoreError::Backpressure)?;
    let permit = Permit;
    let worker = Worker::new("vw-core-open", (), 1)?;
    worker
        .call(move |_| {
            let _permit = permit;
            job()
        })
        .await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn shutdown_waits_for_state_destruction_before_publishing_completion() {
        use std::future::{Future, poll_fn};
        use std::task::Poll;
        struct State {
            started: Option<oneshot::Sender<()>>,
            release: mpsc::Receiver<()>,
        }
        impl Drop for State {
            fn drop(&mut self) {
                self.started.take().unwrap().send(()).unwrap();
                self.release.recv().unwrap();
            }
        }
        let (started, ready) = oneshot::channel();
        let (release, wait) = mpsc::channel();
        let worker = Worker::new(
            "ffi-test-shutdown-drop",
            State {
                started: Some(started),
                release: wait,
            },
            1,
        )
        .unwrap();
        let mut shutdown = Box::pin(worker.shutdown(|_| Ok(())));
        let initially_pending =
            poll_fn(|cx| Poll::Ready(shutdown.as_mut().poll(cx).is_pending())).await;
        ready.await.unwrap();
        let pending = initially_pending
            && poll_fn(|cx| Poll::Ready(shutdown.as_mut().poll(cx).is_pending())).await;
        // Release even on a failed assertion so the owned native thread can exit.
        release.send(()).unwrap();
        assert!(
            pending,
            "shutdown reported completion while state was still alive"
        );
        shutdown.await.unwrap();
        worker.shutdown(|_| Err(CoreError::Invalid)).await.unwrap();
    }
    #[tokio::test]
    async fn saturated_worker_returns_backpressure_without_running_or_dropping_a_job() {
        let worker = Worker::new("ffi-test-backpressure", 0u64, 1).unwrap();
        let (started, ready) = oneshot::channel();
        let (release, wait) = mpsc::channel();
        worker
            .enqueue(Box::new(move |state| {
                let _ = started.send(());
                wait.recv().unwrap();
                *state += 1;
            }))
            .unwrap();
        ready.await.unwrap();
        let (done, finished) = oneshot::channel();
        worker
            .enqueue(Box::new(move |state| {
                *state += 1;
                let _ = done.send(*state);
            }))
            .unwrap();
        assert!(matches!(
            worker
                .call(|state| {
                    *state += 100;
                    Ok(())
                })
                .await,
            Err(CoreError::Backpressure)
        ));
        release.send(()).unwrap();
        assert_eq!(finished.await.unwrap(), 2);
        assert_eq!(worker.call(|state| Ok(*state)).await.unwrap(), 2);
    }
    #[tokio::test]
    async fn shutdown_has_a_reserved_slot_and_survives_cancelled_waiters() {
        use std::future::{Future, poll_fn};
        use std::task::Poll;
        let worker = Worker::new("ffi-test-shutdown", 0u64, 1).unwrap();
        let (started, ready) = oneshot::channel();
        let (release, wait) = mpsc::channel();
        worker
            .enqueue(Box::new(move |state| {
                let _ = started.send(());
                wait.recv().unwrap();
                *state += 1;
            }))
            .unwrap();
        ready.await.unwrap();
        worker.enqueue(Box::new(|state| *state += 1)).unwrap();
        let (value, result) = oneshot::channel();
        let mut close = Box::pin(worker.shutdown(move |state| {
            value.send(*state).unwrap();
            Ok(())
        }));
        assert!(poll_fn(|cx| Poll::Ready(close.as_mut().poll(cx).is_pending())).await);
        drop(close);
        assert!(matches!(
            worker.call(|_| Ok(())).await,
            Err(CoreError::Closed)
        ));
        release.send(()).unwrap();
        worker.shutdown(|_| Err(CoreError::Invalid)).await.unwrap();
        assert_eq!(result.await.unwrap(), 2);
    }
}
