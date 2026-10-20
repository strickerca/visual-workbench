//! One admitted provider owner. Failed retirement retains the exact owner and
//! thread in a bounded registry; a successor cannot overlap an unresolved owner.
use super::contract::{CurrentCheck, OpenProvider, Provider, Work};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Condvar, Mutex, MutexGuard, Weak},
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::sync::{Notify, oneshot};
use vw_capture::{
    Error, Result,
    stream::{StreamRequest, stream_error},
    stream_wire::{Command, MAX_HEADER_BYTES, Packet},
};
use vw_stream::{Cancellation, MemoryCharge, MemoryKind, SessionBudget};

const STACK_BYTES: usize = 2 * 1024 * 1024;
// Admit the actual explicit thread stack, fixed actor/channel/allocator overhead,
// and bounded request/command strings before creating the actor or its queue.
const OWNER_BYTES: u64 = STACK_BYTES as u64 + 64 * 1024 + 4 * MAX_HEADER_BYTES as u64;
const CONTROL_BYTES: u64 = 4096;
const SLOT_BYTES: u64 = 2 * MAX_HEADER_BYTES as u64 + 4096;
const INSPECT_INTERVAL: Duration = Duration::from_millis(25);
static OWNER: Mutex<Option<Arc<Control>>> = Mutex::new(None);
#[cfg(test)]
pub(super) static TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    match value.lock() {
        Ok(guard) => guard,
        // No callback runs under our locks. Retain ownership even if a caller
        // unwinds while manipulating its own future or test fixture.
        Err(poisoned) => poisoned.into_inner(),
    }
}

struct Job {
    work: Work,
    send: oneshot::Sender<Reply>,
    slot: Slot,
}
struct Reply {
    result: Result<Packet>,
    _slot: Slot,
}
struct Slot {
    control: Weak<Control>,
    _charge: MemoryCharge,
    received: bool,
}
impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(control) = self.control.upgrade() {
            // A receiver may be dropped before its surrounding future's guard.
            // Seal the actor before releasing busy admission, so that drop
            // ordering cannot admit a successor after a lost offer/feedback.
            if !self.received {
                control.cancel(false);
            }
            lock(&control.state).busy = false;
            control.wake.notify_all();
        }
    }
}
struct State {
    ready: Option<Result<()>>,
    pending: Option<Job>,
    busy: bool,
    closing: bool,
    orphaned: bool,
    close_generation: u64,
    close_inflight: bool,
    close_result: Option<(u64, Result<()>)>,
    stopped: bool,
    joined: bool,
    unknown: bool,
}
struct Control {
    state: Mutex<State>,
    wake: Condvar,
    changed: Notify,
    cancel: Cancellation,
    current: CurrentCheck,
    pool: SessionBudget,
    // Closed foreign handles still own their actual Control/Worker allocation.
    _owner_charge: MemoryCharge,
    charge: Mutex<Option<MemoryCharge>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}
impl Control {
    fn on_worker_thread(&self) -> bool {
        lock(&self.thread)
            .as_ref()
            .is_some_and(|value| value.thread().id() == thread::current().id())
    }
    fn check(&self) -> Result<()> {
        self.cancel.check().map_err(stream_error)?;
        catch_unwind(AssertUnwindSafe(|| (self.current)())).unwrap_or(Err(Error::Platform))?;
        self.cancel.check().map_err(stream_error)
    }
    fn cancel(&self, orphaned: bool) {
        self.retire(orphaned, orphaned);
    }
    fn retire(&self, orphaned: bool, retry: bool) -> u64 {
        // Independent of the actor lock and blocking provider IO.
        self.cancel.cancel();
        let mut state = lock(&self.state);
        let first = !state.closing;
        state.closing = true;
        state.orphaned |= orphaned;
        if !state.stopped && !state.close_inflight && (first || retry) {
            state.close_generation = state.close_generation.saturating_add(1);
            state.close_inflight = true;
        }
        let generation = state.close_generation;
        drop(state);
        self.wake.notify_all();
        self.changed.notify_waiters();
        generation
    }
    fn ready(&self, result: Result<()>) {
        lock(&self.state).ready = Some(result);
        self.changed.notify_waiters();
    }
    fn stopped(&self, unknown: bool) {
        let mut state = lock(&self.state);
        state.unknown |= unknown;
        state.stopped = true;
        state.close_inflight = false;
        state.close_result = Some((
            state.close_generation,
            if state.unknown {
                Err(Error::Platform)
            } else {
                Ok(())
            },
        ));
        drop(state);
        self.changed.notify_waiters();
    }
    fn join(&self) -> Result<()> {
        let mut handle = lock(&self.thread);
        {
            let state = lock(&self.state);
            if state.joined {
                return Ok(());
            }
            if !state.stopped {
                return Err(Error::Busy);
            }
        }
        if handle
            .as_ref()
            .is_some_and(|value| value.thread().id() == thread::current().id())
        {
            return Err(Error::Busy);
        }
        if let Some(handle) = handle.take()
            && handle.join().is_err()
        {
            lock(&self.state).unknown = true;
        }
        let mut state = lock(&self.state);
        if state.unknown {
            // An unknown owner stays registered and charged, even after its
            // thread exited. Never interpret a missing handle as safe close.
            return Err(Error::Platform);
        }
        state.joined = true;
        drop(state);
        lock(&self.charge).take();
        self.changed.notify_waiters();
        Ok(())
    }
}

pub(in crate::session) struct Worker {
    control: Arc<Control>,
}
impl Worker {
    pub(in crate::session) fn start(
        request: StreamRequest,
        budget: SessionBudget,
        current: CurrentCheck,
        open: OpenProvider,
    ) -> Result<Arc<Self>> {
        let mut registered = lock(&OWNER);
        if let Some(previous) = registered.as_ref() {
            // This joins orphaned successful retirement too. Failure retains
            // the exact entry and prevents a new process/actor allocation.
            previous.join().map_err(|_| Error::Busy)?;
            *registered = None;
        }
        let owner_charge = budget
            .reserve(MemoryKind::Foreign, CONTROL_BYTES)
            .map_err(stream_error)?;
        let charge = budget
            .reserve(MemoryKind::Foreign, OWNER_BYTES - CONTROL_BYTES)
            .map_err(stream_error)?;
        let source_bytes = match &request.source {
            vw_capture::picker::PickerSource::Window { .. } => 0,
            vw_capture::picker::PickerSource::Monitor { target }
            | vw_capture::picker::PickerSource::Region { target, .. } => target.device.capacity(),
        };
        if request
            .capture_session_id
            .capacity()
            .saturating_add(source_bytes)
            > MAX_HEADER_BYTES
            || budget.limit_bytes() > request.endpoint_bytes
        {
            return Err(Error::Limit);
        }
        request.validate()?;
        let control = Arc::new(Control {
            state: Mutex::new(State {
                ready: None,
                pending: None,
                busy: false,
                closing: false,
                orphaned: false,
                close_generation: 0,
                close_inflight: false,
                close_result: None,
                stopped: false,
                joined: false,
                unknown: false,
            }),
            wake: Condvar::new(),
            changed: Notify::new(),
            cancel: Cancellation::default(),
            current,
            pool: budget,
            _owner_charge: owner_charge,
            charge: Mutex::new(Some(charge)),
            thread: Mutex::new(None),
        });
        let actor = control.clone();
        let handle = thread::Builder::new()
            .name("vw-local-producer".into())
            .stack_size(STACK_BYTES)
            .spawn(move || run(actor, request, open))
            .map_err(|_| Error::Platform)?;
        *lock(&control.thread) = Some(handle);
        *registered = Some(control.clone());
        Ok(Arc::new(Self { control }))
    }

    pub(in crate::session) async fn ready(&self) -> Result<()> {
        if self.control.on_worker_thread() {
            return Err(Error::Busy);
        }
        let mut abandoned = Abandoned {
            control: &self.control,
            armed: true,
        };
        loop {
            let changed = self.control.changed.notified();
            // Register before reading state so notifications cannot be lost.
            tokio::pin!(changed);
            changed.as_mut().enable();
            let ready = lock(&self.control.state).ready;
            if let Some(result) = ready {
                result?;
                self.control.check()?;
                abandoned.armed = false;
                return Ok(());
            }
            changed.await;
        }
    }

    pub(in crate::session) async fn exchange(&self, work: Work) -> Result<Packet> {
        if self.control.on_worker_thread() {
            return Err(Error::Busy);
        }
        self.control.check()?;
        if let Work::Original { identity } = &work {
            let string_bytes = identity
                .capture_session_id
                .capacity()
                .saturating_add(identity.source_kind.capacity())
                .saturating_add(identity.platform.capacity())
                .saturating_add(identity.monitor_id.capacity());
            if string_bytes > MAX_HEADER_BYTES {
                return Err(Error::Limit);
            }
        }
        let receive = {
            let mut state = lock(&self.control.state);
            if state.closing || state.stopped {
                return Err(Error::Cancelled);
            }
            match state.ready {
                Some(Ok(())) => (),
                Some(Err(error)) => return Err(error),
                None => return Err(Error::Busy),
            }
            if state.busy {
                return Err(Error::Busy);
            }
            let charge = self
                .control
                .pool
                .reserve(MemoryKind::Foreign, SLOT_BYTES)
                .map_err(stream_error)?;
            if let Work::Original { identity } = &work {
                // Validation's temporary ID copy is admitted by this sole
                // command slot, rather than by every contending caller.
                identity.validate()?;
            }
            state.busy = true;
            let (send, receive) = oneshot::channel();
            state.pending = Some(Job {
                work,
                send,
                slot: Slot {
                    control: Arc::downgrade(&self.control),
                    _charge: charge,
                    received: false,
                },
            });
            receive
        };
        let mut abandoned = Abandoned {
            control: &self.control,
            armed: true,
        };
        self.control.wake.notify_all();
        let mut reply = receive.await.map_err(|_| Error::Platform)?;
        // The actual packet and busy slot remain owned together through this
        // final check and caller receipt, including abandoned/late replies.
        let packet = reply.result?;
        self.control.check()?;
        reply._slot.received = true;
        abandoned.armed = false;
        Ok(packet)
    }

    pub(in crate::session) fn cancel(&self) {
        self.control.cancel(false);
    }

    pub(in crate::session) async fn close(&self) -> Result<()> {
        let generation = self.control.retire(false, true);
        if self.control.on_worker_thread() {
            // Provider/current callbacks may request cancellation, but waiting
            // for this same actor from its callback would deadlock retirement.
            return Err(Error::Busy);
        }
        loop {
            let changed = self.control.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let (stopped, result) = {
                let state = lock(&self.control.state);
                (state.stopped, state.close_result)
            };
            if stopped {
                self.control.join()?;
                let mut registered = lock(&OWNER);
                if registered
                    .as_ref()
                    .is_some_and(|owner| Arc::ptr_eq(owner, &self.control))
                {
                    *registered = None;
                }
                return Ok(());
            }
            if let Some((completed, Err(error))) = result
                && completed >= generation
            {
                return Err(error);
            }
            changed.await;
        }
    }

    pub(in crate::session) fn pool(&self) -> SessionBudget {
        self.control.pool.clone()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        // The registry still owns Control and its JoinHandle. A late open or
        // failed close cannot lose its owner when the foreign endpoint vanishes.
        self.control.cancel(true);
    }
}
struct Abandoned<'a> {
    control: &'a Control,
    armed: bool,
}
impl Drop for Abandoned<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.control.cancel(false);
        }
    }
}

fn run(control: Arc<Control>, request: StreamRequest, open: OpenProvider) {
    if let Err(error) = control.check() {
        control.ready(Err(error));
        control.stopped(false);
        return;
    }
    let opened = catch_unwind(AssertUnwindSafe(|| {
        open(&request, &control.pool, &control.cancel)
    }));
    let mut provider = match opened {
        Ok(Ok(provider)) => provider,
        Ok(Err(error)) => {
            control.ready(Err(error));
            control.stopped(false);
            return;
        }
        Err(_) => {
            // A panicking factory cannot establish whether its own resources
            // retired. Preserve its admission rather than fabricating closure.
            control.ready(Err(Error::Platform));
            control.stopped(true);
            return;
        }
    };
    match control.check() {
        Ok(()) => control.ready(Ok(())),
        Err(error) => {
            control.ready(Err(error));
            control.cancel(false);
        }
    }
    let mut sequence = 0_u64;
    loop {
        // Check lease/connection revocation while idle, without frame readback.
        if control.check().is_err() {
            control.cancel(false);
        }
        let (job, closing, attempt) = {
            let mut state = lock(&control.state);
            let job = state.pending.take();
            (
                job,
                state.closing,
                state.close_inflight.then_some(state.close_generation),
            )
        };
        if let Some(job) = job {
            let result = if closing {
                Err(Error::Cancelled)
            } else {
                exchange(
                    &control,
                    &request,
                    provider.as_mut(),
                    &mut sequence,
                    job.work,
                )
            };
            let result = result.and_then(|packet| control.check().map(|()| packet));
            let failed = result.is_err();
            let _ = job.send.send(Reply {
                result,
                _slot: job.slot,
            });
            if failed {
                control.cancel(false);
            }
        }
        if closing && let Some(generation) = attempt {
            let result =
                catch_unwind(AssertUnwindSafe(|| provider.close())).unwrap_or(Err(Error::Platform));
            if result.is_ok() {
                let unknown = catch_unwind(AssertUnwindSafe(|| drop(provider))).is_err();
                control.stopped(unknown);
                return;
            }
            let mut state = lock(&control.state);
            state.close_result = Some((generation, result));
            state.close_inflight = false;
            drop(state);
            control.changed.notify_waiters();
        }
        let mut state = lock(&control.state);
        if state.pending.is_none() && !(state.closing && state.close_inflight) {
            state = match control.wake.wait_timeout(state, INSPECT_INTERVAL) {
                Ok((state, _)) => state,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
        if state.orphaned && state.closing && !state.close_inflight {
            state.close_generation = state.close_generation.saturating_add(1);
            state.close_inflight = true;
        }
    }
}

fn exchange(
    control: &Control,
    request: &StreamRequest,
    provider: &mut dyn Provider,
    sequence: &mut u64,
    work: Work,
) -> Result<Packet> {
    control.check()?;
    *sequence = sequence.checked_add(1).ok_or(Error::Limit)?;
    let connection_epoch = request.connection_epoch;
    let source_generation = request.source_generation;
    let sequence = *sequence;
    let command = match work {
        Work::Poll { feedback, repair } => Command::Poll {
            sequence,
            connection_epoch,
            source_generation,
            feedback,
            repair_keyframe: repair,
        },
        Work::Original { identity } => Command::Original {
            sequence,
            connection_epoch,
            source_generation,
            identity,
        },
        Work::InspectSource => Command::InspectSource {
            sequence,
            connection_epoch,
            source_generation,
        },
    };
    let packet = catch_unwind(AssertUnwindSafe(|| {
        provider.exchange(command, &control.cancel)
    }))
    .unwrap_or(Err(Error::Platform))?;
    control.check()?;
    if packet.header.sequence() != Some(sequence)
        || packet.header.binding() != (connection_epoch, source_generation)
        || !packet.belongs_to(&control.pool)
        || packet.header.payload_len()? != packet.payload.bytes().len()
    {
        return Err(Error::Stale);
    }
    // Publication has its own check; there is no callback under a worker lock.
    control.check()?;
    Ok(packet)
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
