//! Foreign-call ownership is independent of network and codec completion.
//! Dropping an unreturned view/draw seals that view; it never fabricates a
//! presentation receipt. The driver joins its actual decoder before completion.
use super::{
    api::{CaptureBackground, CaptureDraw, CaptureStream, StreamSignal},
    memory::MAX_RECEIVER_BYTES,
    publication_api::{CapturePublicationSignal, CapturePublishSource, CapturePublisher},
};
use crate::session::{SessionError, SessionResult};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use tokio::sync::{Notify, Semaphore, mpsc, oneshot, watch};
use vw_model::Id;

pub(in crate::session) struct Hub {
    current: Mutex<Option<Arc<Endpoint>>>,
    closed: AtomicBool,
}
pub(in crate::session) struct Endpoint {
    pub epoch: u64,
    pub commands: mpsc::Sender<Command>,
    pub stop: AtomicBool,
    pub wake: Notify,
    pub signal: watch::Sender<StreamSignal>,
    pub region: watch::Sender<super::region::CaptureRegionSignal>,
    pub done: watch::Sender<Option<SessionResult<()>>>,
    next: AtomicU64,
}
pub(in crate::session) struct Lease {
    pub id: u64,
    pub endpoint: Arc<Endpoint>,
    pub closing: AtomicBool,
    // Written only by the canonical durable Save path. One coalesced local
    // notice, never peer input or a claim based merely on a PNG response.
    pub saved: Mutex<Option<(Id, Option<vw_model::AssetId>, u64)>>,
    pub done: watch::Sender<Option<SessionResult<()>>>,
    pub stopped: watch::Sender<bool>,
    pub slots: Arc<Semaphore>,
}
pub(in crate::session) enum Command {
    Publication(super::publication::Command),
    RegionRequest {
        id: Id,
        reply: oneshot::Sender<SessionResult<()>>,
    },
    RegionResolve {
        id: Id,
        approve: bool,
        reply: oneshot::Sender<SessionResult<()>>,
    },
    Enable {
        lease: Weak<Lease>,
        budget: u64,
        reply: oneshot::Sender<SessionResult<()>>,
    },
    Draw {
        lease: u64,
        reply: oneshot::Sender<SessionResult<Option<Arc<CaptureDraw>>>>,
    },
    Background {
        lease: u64,
        reply: oneshot::Sender<SessionResult<Option<Arc<CaptureBackground>>>>,
    },
    Receipt {
        lease: u64,
        generation: u64,
        token: vw_stream::PresentationToken,
        shown: bool,
        settled: Arc<AtomicBool>,
        reply: watch::Sender<Option<SessionResult<()>>>,
    },
    Contact {
        lease: u64,
        draft: Id,
        reply: oneshot::Sender<SessionResult<Option<vw_stream::FreezeRequest>>>,
    },
    Resume {
        lease: u64,
        reply: oneshot::Sender<SessionResult<Option<Id>>>,
    },
}
impl Hub {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            current: Mutex::new(None),
            closed: AtomicBool::new(false),
        })
    }
    pub fn activate(&self, epoch: u64) -> SessionResult<(Arc<Endpoint>, mpsc::Receiver<Command>)> {
        if epoch & 1 == 0 || self.closed.load(Ordering::Acquire) {
            return Err(SessionError::Closed);
        }
        let mut current = self.current.lock().map_err(|_| SessionError::Worker)?;
        if let Some(prior) = current.as_ref() {
            if prior.done.borrow().is_none() {
                return Err(SessionError::Backpressure);
            }
            prior.stop();
        }
        let (commands, receive) = mpsc::channel(8);
        let (signal, _) = watch::channel(StreamSignal::empty(epoch));
        let (done, _) = watch::channel(None);
        let (region, _) = watch::channel(super::region::CaptureRegionSignal::empty(epoch));
        let endpoint = Arc::new(Endpoint {
            epoch,
            commands,
            stop: AtomicBool::new(false),
            wake: Notify::new(),
            signal,
            region,
            done,
            next: AtomicU64::new(1),
        });
        *current = Some(endpoint.clone());
        Ok((endpoint, receive))
    }
    /// `entered_epoch` is captured as the first statement of the native entry.
    pub async fn open(&self, entered_epoch: u64, budget: u64) -> SessionResult<Arc<CaptureStream>> {
        if budget == 0 || budget > MAX_RECEIVER_BYTES {
            return Err(SessionError::Invalid);
        }
        let endpoint = self.endpoint(entered_epoch)?;
        let id = endpoint
            .next
            .try_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_add(1))
            .map_err(|_| SessionError::Backpressure)?;
        let (done, _) = watch::channel(None);
        let (stopped, _) = watch::channel(false);
        let lease = Arc::new(Lease {
            id,
            endpoint: endpoint.clone(),
            closing: AtomicBool::new(false),
            saved: Mutex::new(None),
            done,
            stopped,
            slots: Arc::new(Semaphore::new(4)),
        });
        // The scope owns admission before any suspension. Cancellation during
        // enable delivery drops the lease and the driver refuses/joins it.
        let (receipt, receive) = oneshot::channel();
        endpoint.send(Command::Enable {
            lease: Arc::downgrade(&lease),
            budget,
            reply: receipt,
        })?;
        receive.await.map_err(|_| SessionError::Closed)??;
        lease.check()?;
        Ok(Arc::new(CaptureStream::new(lease)))
    }
    /// A separate explicit owner action admits a local producer. Neither the
    /// remote announcement nor a FreezeRequest can acquire this authority.
    pub async fn publish(
        &self,
        entered_epoch: u64,
        source: CapturePublishSource,
        budget: u64,
    ) -> SessionResult<Arc<CapturePublisher>> {
        if budget == 0 || budget > MAX_RECEIVER_BYTES {
            return Err(SessionError::Invalid);
        }
        let endpoint = self.endpoint(entered_epoch)?;
        let id = endpoint
            .next
            .try_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_add(1))
            .map_err(|_| SessionError::Backpressure)?;
        source.source(id)?;
        let (done, _) = watch::channel(None);
        let (stopped, _) = watch::channel(false);
        let lease = Arc::new(Lease {
            id,
            endpoint: endpoint.clone(),
            closing: AtomicBool::new(false),
            saved: Mutex::new(None),
            done,
            stopped,
            slots: Arc::new(Semaphore::new(4)),
        });
        let (signal, observe) = watch::channel(CapturePublicationSignal::empty(entered_epoch, id));
        let publisher = Arc::new(CapturePublisher::new(lease.clone(), observe));
        let (reply, receive) = oneshot::channel();
        endpoint.send(Command::Publication(super::publication::Command::Enable {
            lease: Arc::downgrade(&lease),
            source,
            budget,
            signal,
            reply,
        }))?;
        receive.await.map_err(|_| SessionError::Closed)??;
        lease.check()?;
        Ok(publisher)
    }
    pub fn endpoint(&self, epoch: u64) -> SessionResult<Arc<Endpoint>> {
        if self.closed.load(Ordering::Acquire) || epoch & 1 == 0 {
            return Err(SessionError::Closed);
        }
        let endpoint = self
            .current
            .lock()
            .map_err(|_| SessionError::Worker)?
            .clone()
            .ok_or(SessionError::Closed)?;
        if endpoint.epoch != epoch || endpoint.stop.load(Ordering::Acquire) {
            return Err(SessionError::Closed);
        }
        Ok(endpoint)
    }
    pub fn stop(&self) {
        self.closed.store(true, Ordering::Release);
        if let Ok(current) = self.current.lock()
            && let Some(endpoint) = current.as_ref()
        {
            endpoint.stop();
        }
    }
    pub async fn close(&self) -> SessionResult<()> {
        self.stop();
        let endpoint = self
            .current
            .lock()
            .map_err(|_| SessionError::Worker)?
            .clone();
        if let Some(endpoint) = endpoint {
            wait_done(endpoint.done.subscribe()).await?;
        }
        Ok(())
    }
}
impl Endpoint {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.wake.notify_one();
    }
    pub fn send(&self, command: Command) -> SessionResult<()> {
        if self.stop.load(Ordering::Acquire) {
            return Err(SessionError::Closed);
        }
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => SessionError::Backpressure,
                mpsc::error::TrySendError::Closed(_) => SessionError::Closed,
            })
    }
}
impl Lease {
    pub fn check(&self) -> SessionResult<()> {
        if self.closing.load(Ordering::Acquire) || self.endpoint.stop.load(Ordering::Acquire) {
            Err(SessionError::Closed)
        } else {
            Ok(())
        }
    }
    pub fn stop(&self) {
        self.closing.store(true, Ordering::Release);
        self.stopped.send_replace(true);
        self.endpoint.wake.notify_one();
    }
    pub async fn close(&self) -> SessionResult<()> {
        self.stop();
        let mut done = self.done.subscribe();
        let mut endpoint = self.endpoint.done.subscribe();
        loop {
            if let Some(result) = done.borrow().clone() {
                return result;
            }
            if let Some(result) = endpoint.borrow().clone() {
                return result.and(Err(SessionError::Worker));
            }
            tokio::select! {result=done.changed()=>result.map_err(|_|SessionError::Worker)?,result=endpoint.changed()=>result.map_err(|_|SessionError::Worker)?,}
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.stop();
    }
}
pub(super) async fn wait_done(
    mut done: watch::Receiver<Option<SessionResult<()>>>,
) -> SessionResult<()> {
    loop {
        if let Some(result) = done.borrow().clone() {
            return result;
        }
        done.changed().await.map_err(|_| SessionError::Worker)?;
    }
}
