//! One coalesced Paint observer. The input loop only uses atomics/try_lock;
//! it never waits for UIA, a worker lock, a result, or a thread join.
use crate::{Error, Result};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};
use vw_remote::{Target, profile::Authority};

#[derive(Default)]
struct Shared {
    stopped: AtomicBool,
    revision: AtomicU64,
    pending: AtomicU64,
    result: Mutex<Option<(u64, Authority)>>,
}
pub(super) struct Observer(Arc<Shared>);
impl Observer {
    pub(super) fn open(target: Target, owner: u32, digest: String) -> Result<Self> {
        Self::spawn(move || {
            // The installed source token is retained. Never cancel it to abandon
            // one stroke; revision mismatch disposes that query's result.
            let _apartment = super::super::shortcuts::Apartment::initialize()?;
            super::super::native_guard::initialize_dpi().map_err(|_| Error::Unavailable)?;
            super::super::shortcuts::refresh_authority(&target, owner, &digest)
        })
    }
    fn spawn(mut observe: impl FnMut() -> Result<Authority> + Send + 'static) -> Result<Self> {
        let shared = Arc::new(Shared::default());
        let worker = shared.clone();
        let _ = thread::Builder::new()
            .name("vw-paint-authority".into())
            .spawn(move || {
                while !worker.stopped.load(Ordering::Acquire) {
                    let revision = worker.pending.swap(0, Ordering::AcqRel);
                    if revision == 0 || revision != worker.revision.load(Ordering::Acquire) {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    let answer = observe(); // existing absolute source/provider budget
                    if worker.stopped.load(Ordering::Acquire)
                        || worker.revision.load(Ordering::Acquire) != revision
                    {
                        continue;
                    }
                    if let Ok(authority) = answer
                        && let Ok(mut result) = worker.result.lock()
                    {
                        // A subsequent DOWN can invalidate while this short lock is
                        // held; take() checks the atomic revision again after taking.
                        *result = Some((revision, authority));
                    }
                }
            })
            .map_err(|_| Error::Io)?;
        Ok(Self(shared))
    }
    pub(super) fn invalidate(&self, sequence: u64) {
        self.0.revision.store(sequence, Ordering::Release);
        self.0.pending.store(0, Ordering::Release);
    }
    pub(super) fn completed_up(&self, sequence: u64) {
        self.0.revision.store(sequence, Ordering::Release);
        self.0.pending.store(sequence, Ordering::Release);
    }
    pub(super) fn take(&self) -> Option<(u64, Authority)> {
        let answer = self.0.result.try_lock().ok()?.take()?;
        (self.0.revision.load(Ordering::Acquire) == answer.0
            && !self.0.stopped.load(Ordering::Acquire))
        .then_some(answer)
    }
}
impl Drop for Observer {
    fn drop(&mut self) {
        self.0.stopped.store(true, Ordering::Release);
        self.0.pending.store(0, Ordering::Release);
        // No JoinHandle: the containing helper process/Job owns the worker until
        // actual whole-process exit. Stop releases input before process exit;
        // the parent's existing retained retirement joins its pipe IO afterward.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    #[test]
    fn blocked_observer_does_not_block_new_stroke_or_release() -> Result<()> {
        let (started, receive_started) = mpsc::channel();
        let (release, receive_release) = mpsc::channel();
        let (finished, receive_finished) = mpsc::channel();
        let observer = Observer::spawn(move || {
            let _ = started.send(());
            let _ = receive_release.recv();
            let _ = finished.send(());
            Err(Error::Unavailable)
        })?;
        observer.completed_up(2);
        receive_started
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| Error::Timeout)?;
        observer.invalidate(3); // next DOWN is not refused or held by observation
        observer.completed_up(4);
        observer.completed_up(6); // coalesces while the previous query is blocked
        assert_eq!(observer.0.pending.load(Ordering::Acquire), 6);
        assert!(observer.take().is_none());
        let shared = observer.0.clone();
        drop(observer); // must return before releasing the blocked provider
        assert!(shared.stopped.load(Ordering::Acquire));
        assert_eq!(shared.pending.load(Ordering::Acquire), 0);
        release.send(()).map_err(|_| Error::Io)?;
        receive_finished
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| Error::Timeout)?;
        Ok(())
    }

    fn authority() -> Authority {
        use vw_remote::profile::{AuthorityRoutes, CanvasProof, ImageIdentity};
        Authority {
            identity: ImageIdentity {
                executable_name: "mspaint.exe".into(),
                executable_blake3: "c".repeat(64),
                executable_bytes: 1024,
                file_version: Some("11.2605.81.0".into()),
                package_full_name: None,
                package_version: None,
            },
            profile_digest: "a".repeat(64),
            verified_actions: vec![1, 3],
            routes: AuthorityRoutes::Essential,
            focus: None,
            selected_canvas: Some(CanvasProof {
                runtime_id_hash: "b".repeat(64),
                process_id: 7,
                sampled_qpc_100ns: 100,
                class_name: "fixture".into(),
                control_type: 50026,
                canvas_rect: vw_remote::Rect {
                    x: 0,
                    y: 0,
                    width: 800,
                    height: 600,
                },
                profile_digest: "a".repeat(64),
            }),
        }
    }
    fn await_result(observer: &Observer) -> Result<(u64, Authority)> {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = observer.take() {
                return Ok(result);
            }
            if std::time::Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn successful_stale_query_is_discarded_and_latest_completed_up_coalesces() -> Result<()> {
        let (started, receive_started) = mpsc::channel();
        let (release, receive_release) = mpsc::channel();
        let observer = Observer::spawn(move || {
            started.send(()).map_err(|_| Error::Io)?;
            receive_release.recv().map_err(|_| Error::Io)?;
            Ok(authority())
        })?;
        observer.completed_up(2);
        receive_started
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| Error::Timeout)?;
        observer.invalidate(3);
        observer.completed_up(4);
        observer.invalidate(5);
        observer.completed_up(6);
        release.send(()).map_err(|_| Error::Io)?;
        receive_started
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| Error::Timeout)?;
        assert!(observer.take().is_none()); // successful revision 2 was discarded
        release.send(()).map_err(|_| Error::Io)?;
        let (sequence, actual) = await_result(&observer)?;
        assert_eq!(sequence, 6);
        assert_eq!(actual.verified_actions, authority().verified_actions);
        assert!(observer.take().is_none()); // exactly one publication
        Ok(())
    }
    #[test]
    fn completed_result_is_discarded_on_down_finite_or_shutdown_without_waiting_for_lock()
    -> Result<()> {
        let observer = Observer::spawn(|| Err(Error::Unavailable))?;
        for sequence in [3, 4] {
            // newer DOWN and finite share monotonic invalidation
            observer.0.revision.store(2, Ordering::Release);
            let mut slot = observer.0.result.lock().map_err(|_| Error::Io)?;
            *slot = Some((2, authority()));
            assert!(observer.take().is_none()); // try_lock must not wait
            observer.invalidate(sequence); // no worker-held-lock dependency
            drop(slot);
            assert!(observer.take().is_none());
        }
        let shared = observer.0.clone();
        let mut slot = shared.result.lock().map_err(|_| Error::Io)?;
        *slot = Some((2, authority()));
        drop(observer); // also cannot wait for the retained lock
        assert!(shared.stopped.load(Ordering::Acquire));
        Ok(())
    }
}
