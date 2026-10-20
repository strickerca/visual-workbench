//! One retained tunnel actor. Cancellation never discards a late provider or
//! returns admission while an escaped response still owns its actual payload.
use super::{
    AndroidTunnelError as Error, AndroidTunnelMode, AndroidTunnelResult as Result,
    AndroidTunnelSource, AndroidTunnelState,
    contract::{OpenProvider, Provider, Response, Stop, Work},
};
use crate::{CoreError, worker::Worker as Actor};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, MutexGuard, Weak,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, ThreadId},
    time::Duration,
};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, oneshot};
use vw_stream::{MemoryCharge, MemoryKind, SessionBudget};

const OUTPUT_BYTES: u64 = 128 * 1024 * 1024;
const ACTOR_BYTES: u64 = 2 * 1024 * 1024 + 64 * 1024;
const OWNER_BYTES: u64 = 4096;
const CHECK_TIMEOUT: Duration = Duration::from_millis(200);
static CURRENT: Mutex<Option<Arc<Worker>>> = Mutex::new(None);
static EPOCH: AtomicU64 = AtomicU64::new(0);

pub(super) fn registry_epoch() -> u64 {
    EPOCH.load(Ordering::Acquire)
}
fn advance_epoch() -> Result<()> {
    EPOCH
        .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            value.checked_add(1)
        })
        .map(|_| ())
        .map_err(|_| Error::Limit)
}
fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    match value.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
struct Envelope {
    _charge: MemoryCharge,
}
struct Status {
    ready: Option<Result<()>>,
    revision: Option<u64>,
    closing: bool,
    close_generation: u64,
    close_inflight: bool,
    close_result: Option<(u64, Result<()>)>,
    provider_closed: bool,
    unknown: bool,
    joined: bool,
    thread: Option<ThreadId>,
}
struct Control {
    stop: Arc<Stop>,
    status: Mutex<Status>,
    changed: Notify,
}
impl Control {
    fn stop(&self) {
        // Stop stores its independent atomic flag before the platform signal.
        // A failing callback must not prevent retaining/retiring the actor.
        let _ = catch_unwind(AssertUnwindSafe(|| self.stop.cancel()));
        let mut status = lock(&self.status);
        status.closing = true;
        status.revision = None;
        drop(status);
        self.changed.notify_waiters();
    }
    fn begin_close(&self, retry: bool) -> (u64, bool) {
        let mut status = lock(&self.status);
        if status.provider_closed || status.joined || status.close_inflight {
            return (status.close_generation, false);
        }
        if status.close_result.is_some() && !retry {
            return (status.close_generation, false);
        }
        status.close_generation = status.close_generation.saturating_add(1);
        status.close_inflight = true;
        (status.close_generation, true)
    }
    fn on_actor(&self) -> bool {
        lock(&self.status).thread == Some(thread::current().id())
    }
    fn ready(&self, result: Result<()>) {
        lock(&self.status).ready = Some(result);
        self.changed.notify_waiters();
    }
    fn cache(&self, state: &AndroidTunnelState) -> Result<()> {
        let mut status = lock(&self.status);
        status.revision = None;
        if status.closing || self.stop.check().is_err() {
            return Err(Error::Cancelled);
        }
        if state.mirror_revision == 0
            || !matches!(
                state.mode,
                AndroidTunnelMode::ViewOnly | AndroidTunnelMode::OwnerControl
            )
        {
            return Err(Error::RetirementPending);
        }
        status.revision = Some(state.mirror_revision);
        Ok(())
    }
}
struct State {
    provider: Option<Box<dyn Provider>>,
    control: Arc<Control>,
    _envelope: Arc<Envelope>,
}

// Field order is intentional: Response (including PNG/elements/string storage)
// is destroyed before its operation permit and conservative allocation owner.
pub(super) struct OwnedResponse {
    pub(super) value: Response,
    lease: Lease,
}
impl OwnedResponse {
    /// The consumer has finished using this exact response. Captured owners
    /// retain this entire value until every native reader/import has settled.
    pub(super) fn close(mut self) {
        self.lease.received = true;
    }
}
impl Drop for OwnedResponse {
    fn drop(&mut self) {
        // Keeping Drop prevents moving value out without its linear owner.
        // Lease::drop seals abandoned delivery before returning its permit.
    }
}
struct Lease {
    owner: Weak<Worker>,
    received: bool,
    _envelope: Arc<Envelope>,
    _permit: OwnedSemaphorePermit,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if !self.received
            && let Some(owner) = self.owner.upgrade()
        {
            owner.cancel();
        }
    }
}
struct Reply {
    result: Result<Response>,
    lease: Lease,
}
struct CheckReply {
    result: Result<()>,
    lease: Lease,
}
struct Abandoned<'a> {
    owner: &'a Worker,
    settled: bool,
}
impl Drop for Abandoned<'_> {
    fn drop(&mut self) {
        if !self.settled {
            self.owner.cancel();
        }
    }
}

pub(super) struct Worker {
    actor: Actor<State>,
    control: Arc<Control>,
    operation: Arc<Semaphore>,
    retirement: tokio::sync::Mutex<()>,
    envelope: Mutex<Option<Arc<Envelope>>>,
    actor_charge: Mutex<Option<MemoryCharge>>,
    _owner_charge: MemoryCharge,
    owner: Weak<Self>,
}
impl Worker {
    pub(super) fn start(
        open: OpenProvider,
        stop: Arc<Stop>,
        budget: SessionBudget,
    ) -> Result<Arc<Self>> {
        let mut registered = lock(&CURRENT);
        if registered.is_some() {
            return Err(Error::RetirementPending);
        }
        stop.check()?;
        // Reserve the explicit base worker stack, all fixed queues/owners and
        // the one bounded capture/PNG/XML/parse/output envelope before allocation.
        let owner_charge = budget
            .reserve(MemoryKind::Foreign, OWNER_BYTES)
            .map_err(map_budget)?;
        let actor_charge = budget
            .reserve(MemoryKind::Foreign, ACTOR_BYTES)
            .map_err(map_budget)?;
        let output = budget
            .reserve(MemoryKind::Foreign, OUTPUT_BYTES)
            .map_err(map_budget)?;
        let envelope = Arc::new(Envelope { _charge: output });
        let control = Arc::new(Control {
            stop,
            status: Mutex::new(Status {
                ready: None,
                revision: None,
                closing: false,
                close_generation: 0,
                close_inflight: false,
                close_result: None,
                provider_closed: false,
                unknown: false,
                joined: false,
                thread: None,
            }),
            changed: Notify::new(),
        });
        let actor = Actor::new(
            "vw-android-tunnel",
            State {
                provider: None,
                control: control.clone(),
                _envelope: envelope.clone(),
            },
            1,
        )
        .map_err(map_actor)?;
        let worker = Arc::new_cyclic(|owner| Self {
            actor,
            control,
            operation: Arc::new(Semaphore::new(1)),
            retirement: tokio::sync::Mutex::new(()),
            envelope: Mutex::new(Some(envelope)),
            actor_charge: Mutex::new(Some(actor_charge)),
            _owner_charge: owner_charge,
            owner: owner.clone(),
        });
        // Installation precedes the only job that can invoke OS/provider open.
        // At epoch exhaustion retain this unstarted actor for explicit close.
        *registered = Some(worker.clone());
        if advance_epoch().is_err() {
            lock(&worker.control.status).provider_closed = true;
            worker.control.ready(Err(Error::Limit));
            drop(registered);
            worker.cancel();
            return Err(Error::RetirementPending);
        }
        let queued = worker
            .actor
            .enqueue(Box::new(move |state| opening(state, open, &budget)));
        drop(registered);
        if let Err(error) = queued {
            lock(&worker.control.status).provider_closed = true;
            worker.control.ready(Err(map_actor(error)));
            worker.cancel();
            return Err(Error::RetirementPending);
        }
        Ok(worker)
    }

    pub(super) fn current() -> Option<Arc<Self>> {
        lock(&CURRENT).clone()
    }
    pub(super) async fn retry_retirement() -> Result<()> {
        match Self::current() {
            Some(owner) if owner.control.stop.check().is_err() => owner.close().await,
            Some(_) => Err(Error::RetirementPending),
            None => Ok(()),
        }
    }
    pub(super) async fn ready(&self) -> Result<()> {
        if self.control.on_actor() {
            return Err(Error::RetirementPending);
        }
        let mut abandoned = Abandoned {
            owner: self,
            settled: false,
        };
        loop {
            let changed = self.control.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let result = lock(&self.control.status).ready.clone();
            if let Some(result) = result {
                result?;
                self.control.stop.check()?;
                abandoned.settled = true;
                return Ok(());
            }
            changed.await;
        }
    }
    pub(super) fn mirror_revision(&self) -> Result<u64> {
        if self.control.stop.check().is_err() {
            return Err(Error::RetirementPending);
        }
        let status = lock(&self.control.status);
        if status.closing || !matches!(status.ready, Some(Ok(()))) {
            return Err(Error::RetirementPending);
        }
        status.revision.ok_or(Error::RetirementPending)
    }
    fn lease(&self) -> Result<Lease> {
        self.control.stop.check()?;
        if self.control.on_actor() {
            return Err(Error::RetirementPending);
        }
        {
            let status = lock(&self.control.status);
            if status.closing || !matches!(status.ready, Some(Ok(()))) {
                return Err(Error::RetirementPending);
            }
        }
        let permit = self
            .operation
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::RetirementPending)?;
        let envelope = lock(&self.envelope).clone().ok_or(Error::Closed)?;
        Ok(Lease {
            owner: self.owner.clone(),
            received: false,
            _envelope: envelope,
            _permit: permit,
        })
    }
    pub(super) async fn exchange(&self, work: Work) -> Result<OwnedResponse> {
        let lease = self.lease()?;
        let mut abandoned = Abandoned {
            owner: self,
            settled: false,
        };
        let (send, receive) = oneshot::channel();
        self.actor
            .enqueue(Box::new(move |state| {
                let result = perform(state, work);
                let _ = send.send(Reply { result, lease });
                retire_requested(state);
            }))
            .map_err(map_actor)?;
        let reply = receive.await.map_err(|_| Error::RetirementPending)?;
        match reply.result {
            Ok(value) => {
                self.control.stop.check()?;
                abandoned.settled = true;
                Ok(OwnedResponse {
                    value,
                    lease: reply.lease,
                })
            }
            Err(error) => {
                let mut lease = reply.lease;
                lease.received = true;
                abandoned.settled = true;
                Err(error)
            }
        }
    }
    pub(super) fn check_source(&self, source: AndroidTunnelSource, revision: u64) -> Result<()> {
        if self.mirror_revision()? != revision {
            return Err(Error::Stale);
        }
        let lease = self.lease().map_err(|_| Error::RetirementPending)?;
        let (send, receive) = mpsc::sync_channel(1);
        if let Err(error) = self.actor.enqueue(Box::new(move |state| {
            let result = invoke(state, |provider| provider.check_source(source, revision));
            let _ = send.try_send(CheckReply { result, lease });
            retire_requested(state);
        })) {
            self.cancel();
            return Err(map_actor(error));
        }
        match receive.recv_timeout(CHECK_TIMEOUT) {
            Ok(mut reply) => {
                self.control.stop.check()?;
                reply.lease.received = true;
                reply.result
            }
            Err(_) => {
                self.cancel();
                Err(Error::RetirementPending)
            }
        }
    }
    pub(super) fn cancel(&self) {
        self.control.stop();
        self.request_close(false);
    }
    fn request_close(&self, retry: bool) -> u64 {
        let (generation, enqueue) = self.control.begin_close(retry);
        if enqueue {
            let result = self.actor.enqueue(Box::new(retire_requested));
            if let Err(error) = result
                && !matches!(error, CoreError::Backpressure)
            {
                let mut status = lock(&self.control.status);
                status.unknown = true;
                status.close_inflight = false;
                status.close_result = Some((generation, Err(Error::RetirementPending)));
                drop(status);
                self.control.changed.notify_waiters();
            }
            // A full FIFO already owns the open/operation/check job. Every
            // such job observes this retained request before returning to idle.
        }
        generation
    }
    pub(super) async fn close(&self) -> Result<()> {
        self.control.stop();
        let generation = self.request_close(true);
        if self.control.on_actor() {
            return Err(Error::RetirementPending);
        }
        let _retirement = self.retirement.lock().await;
        loop {
            let changed = self.control.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let (closed, result) = {
                let status = lock(&self.control.status);
                (
                    status.provider_closed && !status.unknown,
                    status.close_result.clone(),
                )
            };
            if closed {
                break;
            }
            if let Some((finished, Err(error))) = result
                && finished >= generation
            {
                return Err(error);
            }
            changed.await;
        }
        self.actor
            .shutdown(|state| {
                if state.provider.is_some() || lock(&state.control.status).unknown {
                    Err(CoreError::Worker)
                } else {
                    Ok(())
                }
            })
            .await
            .map_err(|_| Error::RetirementPending)?;
        lock(&self.control.status).joined = true;
        // Actual joined thread retirement precedes credit and registry release.
        lock(&self.actor_charge).take();
        lock(&self.envelope).take();
        let mut registered = lock(&CURRENT);
        if registered
            .as_ref()
            .is_some_and(|owner| std::ptr::eq(owner.as_ref(), self))
        {
            advance_epoch().map_err(|_| Error::RetirementPending)?;
            *registered = None;
        }
        Ok(())
    }
}

fn opening(state: &mut State, open: OpenProvider, budget: &SessionBudget) {
    lock(&state.control.status).thread = Some(thread::current().id());
    let result = (|| {
        state.control.stop.check()?;
        match catch_unwind(AssertUnwindSafe(|| open(budget))) {
            Ok(Ok(provider)) => state.provider = Some(provider),
            Ok(Err(error)) => {
                let mut status = lock(&state.control.status);
                if matches!(&error, Error::RetirementPending) {
                    // The factory reported unresolved native ownership without
                    // returning its concrete provider. Keep actor, admission and
                    // registry fenced; no callback can prove safe retirement.
                    status.unknown = true;
                } else {
                    status.provider_closed = true;
                }
                drop(status);
                return Err(error);
            }
            Err(_) => {
                lock(&state.control.status).unknown = true;
                return Err(Error::Platform);
            }
        }
        // Adopt the late owner first; cancellation can only retire that exact
        // provider, never cause a returned owner to fall out of the actor.
        state.control.stop.check()?;
        let actual = invoke(state, |provider| provider.state())?;
        state.control.cache(&actual)
    })();
    state.control.ready(result.clone());
    if result.is_err() {
        state.control.stop();
        state.control.begin_close(false);
    }
    retire_requested(state);
}
fn invoke<T>(
    state: &mut State,
    operation: impl FnOnce(&mut dyn Provider) -> Result<T>,
) -> Result<T> {
    state.control.stop.check()?;
    let provider = state.provider.as_mut().ok_or(Error::Closed)?;
    let result = catch_unwind(AssertUnwindSafe(|| operation(provider.as_mut())));
    let value = match result {
        Ok(result) => result?,
        Err(_) => {
            state.control.stop();
            state.control.begin_close(false);
            return Err(Error::Platform);
        }
    };
    state.control.stop.check()?;
    Ok(value)
}
fn actual_state(state: &mut State) -> Result<AndroidTunnelState> {
    let actual = invoke(state, |provider| provider.state())?;
    state.control.cache(&actual)?;
    Ok(actual)
}
fn perform(state: &mut State, work: Work) -> Result<Response> {
    let result = match work {
        Work::State => actual_state(state).map(Response::State),
        Work::BeginConfirmation => {
            lock(&state.control.status).revision = None;
            invoke(state, |provider| provider.begin_confirmation()).map(Response::Target)
        }
        Work::Confirm(target) => {
            lock(&state.control.status).revision = None;
            let result = (|| {
                invoke(state, |provider| provider.confirm(target))?;
                actual_state(state).map(Response::State)
            })();
            if result.is_err() {
                state.control.stop();
                state.control.begin_close(false);
            }
            result
        }
        Work::RevokeControl => {
            lock(&state.control.status).revision = None;
            let result = (|| {
                invoke(state, |provider| provider.revoke_control())?;
                let actual = invoke(state, |provider| provider.state())?;
                // Actual Windows revocation closes its mirror. Return that
                // terminal truth; the exported caller joins retirement before
                // delivery. Closed never becomes a cached live revision.
                if actual.mode == AndroidTunnelMode::Closed && actual.mirror_revision == 0 {
                    Ok(Response::State(actual))
                } else {
                    state.control.cache(&actual)?;
                    Ok(Response::State(actual))
                }
            })();
            if result.is_err() {
                state.control.stop();
                state.control.begin_close(false);
            }
            result
        }
        Work::CancelConfirmation => {
            lock(&state.control.status).revision = None;
            (|| {
                invoke(state, |provider| provider.cancel_confirmation())?;
                actual_state(state)?;
                Ok(Response::Done)
            })()
        }
        Work::Capture => invoke(state, |provider| provider.capture()).and_then(|captured| {
            let mut bytes = captured.png.capacity() as u64;
            bytes = bytes
                .checked_add(
                    (captured.elements.capacity() as u64)
                        .checked_mul(std::mem::size_of::<super::AndroidTunnelElement>() as u64)
                        .ok_or(Error::Limit)?,
                )
                .ok_or(Error::Limit)?;
            for element in &captured.elements {
                for value in [
                    &element.class_name,
                    &element.resource_id,
                    &element.description,
                    &element.text,
                ] {
                    bytes = bytes
                        .checked_add(value.capacity() as u64)
                        .ok_or(Error::Limit)?;
                }
            }
            if bytes > OUTPUT_BYTES
                || captured.png.is_empty()
                || captured.width == 0
                || captured.height == 0
            {
                return Err(Error::Limit);
            }
            Ok(Response::Captured(captured))
        }),
    };
    if result.is_err() {
        lock(&state.control.status).revision = None;
    }
    result
}
fn retire_requested(state: &mut State) {
    let generation = {
        let status = lock(&state.control.status);
        if !status.close_inflight {
            return;
        }
        status.close_generation
    };
    let unknown = lock(&state.control.status).unknown;
    let result = if unknown {
        Err(Error::RetirementPending)
    } else if let Some(provider) = &mut state.provider {
        catch_unwind(AssertUnwindSafe(|| provider.close())).unwrap_or(Err(Error::RetirementPending))
    } else {
        Ok(())
    };
    let closed = if result.is_ok() {
        let provider = state.provider.take();
        catch_unwind(AssertUnwindSafe(|| drop(provider))).is_ok()
    } else {
        false
    };
    let mut status = lock(&state.control.status);
    status.unknown |= result.is_ok() && !closed;
    status.provider_closed = closed;
    status.close_inflight = false;
    status.close_result = Some((
        generation,
        if closed {
            Ok(())
        } else {
            Err(Error::RetirementPending)
        },
    ));
    drop(status);
    state.control.changed.notify_waiters();
}
fn map_actor(error: CoreError) -> Error {
    match error {
        CoreError::Backpressure => Error::Limit,
        CoreError::Cancelled => Error::Cancelled,
        CoreError::Invalid => Error::Invalid,
        CoreError::Closed => Error::Closed,
        _ => Error::RetirementPending,
    }
}
fn map_budget(_: vw_stream::StreamError) -> Error {
    Error::Limit
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
