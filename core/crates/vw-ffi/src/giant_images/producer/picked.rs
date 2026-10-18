//! Private pre-open input ownership, admitted from frozen c0d1e602.
//!
//! Root integration in producer.rs (not applied by this owner):
//! ```ignore
//! mod picked;
//! pub(crate) use picked::{PendingInputAdmission, PickedOriginal, reserve_picked_input};
//! ```
//! Root must reserve before synchronously opening its File/TempDir, then attach.
//! No foreign FD/path/result factory or producer approval is introduced. Existing
//! prepare_original callers share input::reserve_admission and its SAME counter.
//!
//! Picked owners create no threads. Preparation uses existing bounded startup
//! ownership. A completed source stays in shared control, never in an unpolled
//! oneshot reply: close/drop can retire it without cooperation from that future.
//! Actual cleanup precedes completion, or quarantines the same permit/4MiB.
//! The independent prepared worker is constructed only in the receiving future's
//! final synchronous poll; no await follows construction/delivery.
use super::{
    Error, GiantInputLimits, PreparedOriginal, PreparedState, Result, RootOwnedPickedOriginal,
    input, source,
};
use crate::{Cancellation, worker::Worker};
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::watch;
use vw_store::FreeSpaceQuery;
use vw_stream::SessionBudget;

pub(crate) struct PendingInputAdmission {
    budget: SessionBudget,
    admission: input::InputAdmission,
}
pub(crate) fn reserve_picked_input(budget: SessionBudget) -> Result<PendingInputAdmission> {
    let admission = input::reserve_admission(&budget)?;
    Ok(PendingInputAdmission { budget, admission })
}
impl PendingInputAdmission {
    pub(crate) fn attach(
        self,
        original: RootOwnedPickedOriginal,
        limits: GiantInputLimits,
    ) -> Result<Arc<PickedOriginal>> {
        // Aggregate File, TempDir, permit and charge before fallible admission.
        let input = input::AdmittedInput::attach(original, self.admission);
        if let Err(error) = input::validate_limits(&limits) {
            input.release()?;
            return Err(error);
        }
        let (done, _) = watch::channel(false);
        Ok(Arc::new(PickedOriginal {
            control: Arc::new(Control {
                status: Mutex::new(Status {
                    state: State::Ready { input, limits },
                    retirement: Ok(()),
                }),
                closed: Arc::new(AtomicBool::new(false)),
                done,
            }),
            budget: self.budget,
        }))
    }
}
enum State {
    Ready {
        input: input::AdmittedInput,
        limits: GiantInputLimits,
    },
    Working,
    Available(Arc<input::PreparedSource>),
    Consumed,
}
struct Status {
    state: State,
    retirement: Result<()>,
}
struct Control {
    status: Mutex<Status>,
    closed: Arc<AtomicBool>,
    done: watch::Sender<bool>,
}
enum Retiring {
    Input(input::AdmittedInput),
    Source(Arc<input::PreparedSource>),
}
impl Control {
    fn status(&self) -> MutexGuard<'_, Status> {
        // No caller callback runs under this lock; its enum remains valid after
        // an unwind. Recovering poison does not release any actual resource.
        match self.status.lock() {
            Ok(status) => status,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
    fn complete(&self, result: Result<()>) {
        let mut status = self.status();
        if status.retirement.is_ok() {
            status.retirement = result;
        }
        self.done.send_replace(true);
    }
    fn request_close(&self) {
        let retiring = {
            let mut status = self.status();
            self.closed.store(true, Ordering::Release);
            let retiring = match std::mem::replace(&mut status.state, State::Consumed) {
                State::Ready { input, .. } => Some(Retiring::Input(input)),
                State::Available(source) => Some(Retiring::Source(source)),
                State::Working | State::Consumed => None,
            };
            if retiring.is_some() {
                // Concurrent close callers must join this actual cleanup, even
                // if the copy previously completed and published done=true.
                self.done.send_replace(false);
            }
            retiring
        };
        if let Some(retiring) = retiring {
            let result = match retiring {
                Retiring::Input(input) => input.release(),
                Retiring::Source(source) => input::release(source),
            };
            self.complete(result);
        }
    }
    fn publish_source(&self, source: Arc<input::PreparedSource>) -> Result<()> {
        let mut status = self.status();
        if !self.closed.load(Ordering::Acquire) && matches!(status.state, State::Working) {
            status.state = State::Available(source);
            self.done.send_replace(true);
            return Ok(());
        }
        drop(status);
        let retirement = input::release(source);
        let error = retirement.as_ref().err().cloned().unwrap_or(Error::Closed);
        self.complete(retirement);
        Err(error)
    }
}
pub(crate) struct PickedOriginal {
    control: Arc<Control>,
    budget: SessionBudget,
}
impl PickedOriginal {
    pub(crate) async fn prepare(&self, cancel: Arc<Cancellation>) -> Result<Arc<PreparedOriginal>> {
        self.prepare_with(cancel, vw_store::RealFreeSpace, 2).await
    }
    async fn prepare_with<Q: FreeSpaceQuery + Send + 'static>(
        &self,
        cancel: Arc<Cancellation>,
        query: Q,
        prepared_capacity: usize,
    ) -> Result<Arc<PreparedOriginal>> {
        let (input, limits) = {
            let mut status = self.control.status();
            if self.control.closed.load(Ordering::Acquire) {
                return Err(Error::Closed);
            }
            match std::mem::replace(&mut status.state, State::Working) {
                State::Ready { input, limits } => (input, limits),
                previous => {
                    status.state = previous;
                    return Err(Error::Backpressure);
                }
            }
        };
        let mut abandoned = Abandoned {
            control: self.control.clone(),
            armed: true,
        };
        let job = Preparation {
            input: Some(input),
            limits,
            control: self.control.clone(),
            finished: false,
        };
        // An already-cancelled attempt never invokes the actual worker body.
        source::live(&self.control.closed, &cancel)?;
        let work_cancel = cancel.clone();
        crate::worker::startup(move || Ok(job.run(&work_cancel, &query)))
            .await
            .map_err(Error::from)??;

        // Linearize consumption against close. No await/caller callback follows
        // this fence. A stalled receiving future cannot own the available source.
        let mut status = self.control.status();
        source::live(&self.control.closed, &cancel)?;
        let source = match std::mem::replace(&mut status.state, State::Consumed) {
            State::Available(source) => source,
            previous => {
                status.state = previous;
                return Err(Error::Closed);
            }
        };
        let info = source.info.clone();
        // Keep this fallback lease through the fallible constructor. It shares
        // the same allocation/admission; it never acquires a second permit.
        let worker = match Worker::new(
            "vw-giant-input",
            Some(PreparedState {
                source: source.clone(),
                build_started: false,
            }),
            prepared_capacity,
        ) {
            Ok(worker) => worker,
            Err(error) => {
                let retirement = input::release(source);
                if status.retirement.is_ok() {
                    status.retirement = retirement;
                }
                return Err(Error::from(error));
            }
        };
        let prepared = Arc::new(PreparedOriginal {
            worker,
            // Picked close/drop must never seal the independently accepted Arc.
            closed: Arc::new(AtomicBool::new(false)),
            budget: self.budget.clone(),
            info,
        });
        drop(source);
        abandoned.armed = false;
        Ok(prepared)
    }
    pub(crate) async fn close_worker(&self) -> Result<()> {
        let mut done = self.control.done.subscribe();
        self.control.request_close();
        loop {
            if *done.borrow() {
                return self.control.status().retirement.clone();
            }
            done.changed().await.map_err(|_| Error::Storage)?;
        }
    }
}
impl Drop for PickedOriginal {
    fn drop(&mut self) {
        self.control.request_close();
    }
}
struct Abandoned {
    control: Arc<Control>,
    armed: bool,
}
impl Drop for Abandoned {
    fn drop(&mut self) {
        if self.armed {
            self.control.request_close();
        }
    }
}
struct Preparation {
    input: Option<input::AdmittedInput>,
    limits: GiantInputLimits,
    control: Arc<Control>,
    finished: bool,
}
impl Preparation {
    fn run(mut self, cancel: &Cancellation, query: &impl FreeSpaceQuery) -> Result<()> {
        let input = self.input.take().ok_or(Error::Closed)?;
        let limits = GiantInputLimits {
            max_original_bytes: self.limits.max_original_bytes,
            max_work_disk_bytes: self.limits.max_work_disk_bytes,
            assume_untagged_srgb: self.limits.assume_untagged_srgb,
        };
        let result = match input::prepare_admitted_report(
            input,
            limits,
            &self.control.closed,
            cancel,
            query,
        ) {
            Ok(source) => self.control.publish_source(source),
            Err(failure) => {
                self.control.complete(failure.retirement);
                Err(failure.error)
            }
        };
        self.finished = true;
        result
    }
}
impl Drop for Preparation {
    fn drop(&mut self) {
        if !self.finished {
            // Covers startup refusal, a skipped abandoned receiver, and unwind.
            // Physical input cleanup or quarantine precedes completion.
            let result = match self.input.take() {
                Some(input) => input.release(),
                None => Err(Error::Storage),
            };
            self.control.complete(result);
        }
    }
}

#[cfg(test)]
#[path = "picked_tests.rs"]
mod tests;
